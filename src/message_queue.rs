use crate::{Attempt, Store, provider::Message};
use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, params};

impl Store {
    /// Keep an in-flight direction until its provider writes or acknowledges it.
    /// A fresh process can then report an interrupted delivery without replaying it.
    pub(crate) fn stage_direction(&self, chat: i64, mut message: Message) -> Result<Message> {
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO queued_messages(attempt,body,files,sending,steering) VALUES(?1,?2,?3,1,1)",
            params![chat, message.text, serde_json::to_string(&message.files)?],
        )?;
        message.queued_id = Some(db.last_insert_rowid());
        Ok(message)
    }

    pub(crate) fn direction_unavailable(
        &self,
        chat: i64,
        message: &Message,
        reason: &str,
    ) -> Result<()> {
        self.event_with_image(
            chat,
            "steer/unavailable",
            Some(&format!("{reason}\n{}", message.text)),
            self.attached_chat_image(&message.files),
            Some(message),
        )
    }

    pub(crate) fn finish_pending_directions(&self, chat: Option<i64>) -> Result<bool> {
        let pending = {
            let db = self.db.lock().unwrap();
            let mut query = db.prepare(
                "SELECT attempt,id,body,files FROM queued_messages WHERE steering=1 AND (?1 IS NULL OR attempt=?1) ORDER BY id")?;
            query
                .query_map([chat], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let changed = !pending.is_empty();
        for (attempt, id, text, files) in pending {
            let message = Message {
                text,
                files: serde_json::from_str(&files)?,
                queued_id: Some(id),
            };
            self.direction_unavailable(
                attempt,
                &message,
                "The run ended before delivery could be confirmed.",
            )?;
        }
        Ok(changed)
    }

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
             SELECT id,?2,?3 FROM attempts WHERE id=?1 AND (state='running' OR (?4 AND state IN ('review','cancelled','failed','interrupted')))",
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
        self.claim_queued(chat, true)
    }

    pub(crate) fn claim_queued(
        &self,
        chat: i64,
        reopen_session: bool,
    ) -> Result<Option<(Attempt, Message)>> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let next = tx
            .query_row(
                "SELECT q.id, q.body, q.files, a.session<>'', a.base<>'' FROM queued_messages q
                 JOIN attempts a ON a.id=q.attempt WHERE q.attempt=?1 AND q.sending=0
                 ORDER BY q.id LIMIT 1",
                [chat],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, bool>(3)?,
                        r.get::<_, bool>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, text, files, has_session, has_copy)) = next else {
            return Ok(None);
        };
        // Automatic draining waits for the provider to establish a conversation.
        // Only an explicit Send next may replay the original request.
        if !has_session && !reopen_session {
            return Ok(None);
        }
        let followup = Message {
            text,
            files: serde_json::from_str(&files)?,
            queued_id: Some(id),
        };
        for file in &followup.files {
            ensure!(
                file.exists(),
                "Queued attachment is unavailable: {}",
                file.display()
            );
        }
        if has_session {
            crate::run_state::Resume::QueuedMessage.claim(&tx, chat)?;
            tx.execute("UPDATE queued_messages SET sending=1 WHERE id=?1", [id])?;
        } else {
            ensure!(
                has_copy && self.work(chat).is_dir(),
                "The original working copy is unavailable. Retry this chat to send its queued messages."
            );
            crate::run_state::Resume::ReopenForQueue.claim(&tx, chat)?;
        }
        tx.commit()?;
        drop(db);
        let attempt = self.attempt(chat)?;
        let message = if has_session {
            followup
        } else {
            Message::from(self.retry_prompt(&attempt)?.as_str())
        };
        Ok(Some((attempt, message)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_sessionless_queue_only_reopens_the_original_on_request() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = Store::open(&tmp.path().join("project"), &tmp.path().join("home"))?;
        let original = store.prepare("Create the lobby", "", None)?;
        let draft = store.work(original.id).join("lobby.txt");
        fs::write(&draft, "in progress")?;
        store.queue_message(original.id, &"Review it".into())?;
        store.queue_message(original.id, &"Summarize it".into())?;
        let pending = store.queued_messages(original.id)?;
        store.finish(original.id, "cancelled", "Stopped before initialization")?;

        let (same, message) = store.resume_queued(original.id)?.unwrap();
        assert_eq!(same.id, original.id);
        assert_eq!(message.text, original.prompt);
        assert!(message.queued_id.is_none());
        assert_eq!(fs::read_to_string(draft)?, "in progress");
        store.finish(same.id, "review", "Provider finished without a session ID")?;
        assert!(store.claim_queued(same.id, false)?.is_none());
        assert_eq!(store.queued_messages(same.id)?, pending);
        assert_eq!(store.attempt(same.id)?.state, "review");

        let (retry, _) = store.retry_chat(same.id)?;
        assert!(store.queued_messages(same.id)?.is_empty());
        assert_eq!(store.queued_messages(retry.id)?, pending);
        Ok(())
    }

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
