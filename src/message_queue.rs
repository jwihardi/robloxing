use crate::{Attempt, Store, provider::Message};
use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, params};

impl Store {
    pub fn queue_message(&self, chat: i64, message: &Message) -> Result<()> {
        self.save_queued(chat, message, false)
    }

    pub(crate) fn save_queued(
        &self,
        chat: i64,
        message: &Message,
        between_turns: bool,
    ) -> Result<()> {
        ensure!(
            !message.text.trim().is_empty() || !message.files.is_empty(),
            "Enter a message to queue"
        );
        let written = self.db.lock().unwrap().execute(
            "INSERT INTO queued_messages(attempt,body,files)
             SELECT id,?2,?3 FROM attempts WHERE id=?1 AND (state='running' OR (?4 AND state='review'))",
            params![chat, message.text, serde_json::to_string(&message.files)?, between_turns],
        )?;
        ensure!(
            written == 1,
            "The run has ended. Send your draft as a follow-up."
        );
        Ok(())
    }

    pub fn queued_messages(&self, chat: i64) -> Result<Vec<(i64, String)>> {
        let db = self.db.lock().unwrap();
        let mut query = db.prepare(
            "SELECT id,body FROM queued_messages WHERE attempt=?1 AND sending=0 ORDER BY id",
        )?;
        Ok(query
            .query_map([chat], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?)
    }

    pub fn remove_queued(&self, chat: i64, id: i64) -> Result<()> {
        let removed = self.db.lock().unwrap().execute(
            "DELETE FROM queued_messages WHERE attempt=?1 AND id=?2 AND sending=0",
            params![chat, id],
        )?;
        ensure!(removed == 1, "That message has already started.");
        Ok(())
    }

    pub fn resume_queued(&self, chat: i64) -> Result<Option<(Attempt, Message)>> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let next = tx.query_row(
            "SELECT id,body,files FROM queued_messages WHERE attempt=?1 AND sending=0 ORDER BY id LIMIT 1",
            [chat], |r| Ok((r.get::<_,i64>(0)?,r.get(1)?,r.get::<_,String>(2)?))
        ).optional()?;
        let Some((id, text, files)) = next else {
            return Ok(None);
        };
        let message = Message {
            text,
            files: serde_json::from_str(&files)?,
            queued_id: Some(id),
        };
        for file in &message.files {
            ensure!(
                file.exists(),
                "Queued attachment is unavailable: {}",
                file.display()
            );
        }
        let started = tx.execute(
            "UPDATE attempts SET state='running',action='Sending queued message'
             WHERE id=?1 AND session!='' AND state IN ('review','interrupted','failed','cancelled')
             AND NOT EXISTS (SELECT 1 FROM attempts WHERE state IN ('running','preparing','applying'))", [chat])?;
        ensure!(
            started == 1,
            "This chat cannot start a queued message right now."
        );
        tx.execute("UPDATE queued_messages SET sending=1 WHERE id=?1", [id])?;
        tx.commit()?;
        drop(db);
        Ok(Some((self.attempt(chat)?, message)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn pending_files_and_inflight_claims_survive_reopening_the_chat() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        let home = tmp.path().join("data");
        let file = tmp.path().join("reference.txt");
        fs::write(&file, "reference")?;
        let store = Store::open(&project, &home)?;
        let a = store.prepare("first chat", "", None)?;
        store.session(a.id, "fixture")?;
        let message = Message::compose("Compare this", vec![file.clone()]);
        store.queue_message(a.id, &message)?;
        store.queue_message(a.id, &"removed".into())?;
        let pending = store.queued_messages(a.id)?;
        assert!(store.remove_queued(a.id + 1, pending[1].0).is_err());
        store.remove_queued(a.id, pending[1].0)?;
        store.finish(a.id, "cancelled", "Stopped")?;
        assert!(store.resume_queued(a.id + 1)?.is_none());
        fs::remove_file(&file)?;
        assert!(store.resume_queued(a.id).is_err());
        assert_eq!(store.queued_messages(a.id)?.len(), 1);
        fs::write(&file, "reference")?;
        let (_, first) = store.resume_queued(a.id)?.unwrap();
        assert_eq!(first.files.as_slice(), std::slice::from_ref(&file));
        assert!(store.remove_queued(a.id, pending[0].0).is_err());
        drop(store);
        let store = Store::open(&project, &home)?;
        let (_, retry) = store.resume_queued(a.id)?.unwrap();
        assert_eq!(retry.text, message.text);
        assert_eq!(retry.files, [file]);
        store.message_event(a.id, "user/followup", &retry)?;
        assert!(store.queued_messages(a.id)?.is_empty());
        assert!(store.message_event(a.id, "user/followup", &retry).is_err());
        assert_eq!(
            store
                .messages_after(a.id, 0)?
                .iter()
                .filter(|m| m.text == message.text)
                .count(),
            1
        );
        Ok(())
    }
}
