use crate::{Result, Store};
use rusqlite::{Connection, params};
use std::path::Path;

fn ensure_schema(db: &Connection, table: &str) -> Result<()> {
    db.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {table} (
    key TEXT PRIMARY KEY, project TEXT NOT NULL, attempt INTEGER, preceding_prompt INTEGER NOT NULL,
    created TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP)"
    ))?;
    Ok(())
}

pub struct AlbumEntry {
    pub key: String,
    pub project: String,
    pub created: String,
}

impl AlbumEntry {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            key: row.get(0)?,
            project: row.get(1)?,
            created: row.get(2)?,
        })
    }
}

pub fn album(home: &Path) -> Result<Vec<AlbumEntry>> {
    std::fs::create_dir_all(home)?;
    let db = Connection::open(home.join("preferences.sqlite"))?;
    ensure_schema(&db, "screenshots")?;
    let mut q = db.prepare("SELECT key,project,datetime(created,'localtime') FROM screenshots ORDER BY created DESC,rowid DESC")?;
    Ok(q.query_map([], AlbumEntry::from_row)?
        .collect::<rusqlite::Result<_>>()?)
}

impl Store {
    pub(crate) fn init_screenshots(db: &Connection) -> Result<()> {
        ensure_schema(db, "preferences.screenshots")
    }
    pub fn post_screenshot(&self, attempt: Option<i64>, key: &str) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute("INSERT INTO preferences.screenshots(key,project,attempt,preceding_prompt) VALUES(?1,?2,?3,?4)",
            params![key,self.project.to_string_lossy(),attempt,attempt.is_none()])?;
        if let Some(id) = attempt {
            tx.execute(
                "INSERT INTO events(attempt,kind,summary) VALUES(?1,'workshop/screenshot',?2)",
                params![id, key],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn bind_draft_screenshots(&self, attempt: i64) -> Result<()> {
        self.db.lock().unwrap().execute(
            "UPDATE preferences.screenshots SET attempt=?1 WHERE project=?2 AND attempt IS NULL",
            params![attempt, self.project.to_string_lossy()],
        )?;
        Ok(())
    }
    pub fn chat_screenshots(&self, attempt: Option<i64>) -> Result<Vec<AlbumEntry>> {
        let db = self.db.lock().unwrap();
        let mut q = db.prepare(
            "SELECT key,project,
                    (CASE date(created,'localtime') WHEN date('now','localtime') THEN 'Today'
                     ELSE date(created,'localtime') END) || ' · ' || strftime('%H:%M',created,'localtime')
             FROM preferences.screenshots
             WHERE project=?1 AND attempt IS ?2 ORDER BY created DESC,rowid DESC",
        )?;
        Ok(q.query_map(
            params![self.project.to_string_lossy(), attempt],
            AlbumEntry::from_row,
        )?
        .collect::<rusqlite::Result<_>>()?)
    }
    pub fn preceding_screenshots(&self, attempt: Option<i64>) -> Result<Vec<String>> {
        let db = self.db.lock().unwrap();
        let mut q=db.prepare("SELECT key FROM preferences.screenshots WHERE project=?1 AND attempt IS ?2 AND preceding_prompt=1 ORDER BY rowid")?;
        Ok(
            q.query_map(params![self.project.to_string_lossy(), attempt], |r| {
                r.get(0)
            })?
            .collect::<rusqlite::Result<_>>()?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn album_spans_projects_and_receipts_stay_with_their_chat() -> Result<()> {
        let t = tempfile::tempdir()?;
        let home = t.path().join("data");
        let a = Store::open(&t.path().join("first"), &home)?;
        let b = Store::open(&t.path().join("second"), &home)?;
        a.post_screenshot(None, "shot-first")?;
        b.post_screenshot(None, "shot-second")?;
        assert_eq!(a.chat_screenshots(None)?.len(), 1);
        let chat = a.prepare("Review the project", "", None)?;
        a.bind_draft_screenshots(chat.id)?;
        a.post_screenshot(Some(chat.id), "shot-later")?;
        a.finish(chat.id, "review", "")?;
        let other_chat = a.prepare("Second chat", "", None)?;
        a.post_screenshot(Some(other_chat.id), "shot-other")?;
        a.post_screenshot(None, "shot-draft")?;
        let keys = |items: Vec<AlbumEntry>| items.into_iter().map(|i| i.key).collect::<Vec<_>>();
        assert_eq!(
            keys(a.chat_screenshots(Some(chat.id))?),
            ["shot-later", "shot-first"]
        );
        assert_eq!(
            keys(a.chat_screenshots(Some(other_chat.id))?),
            ["shot-other"]
        );
        assert_eq!(keys(a.chat_screenshots(None)?), ["shot-draft"]);
        assert_eq!(keys(b.chat_screenshots(None)?), ["shot-second"]);
        assert_eq!(a.preceding_screenshots(Some(chat.id))?, ["shot-first"]);
        assert_eq!(a.preceding_screenshots(None)?, ["shot-draft"]);
        assert_eq!(b.preceding_screenshots(None)?, ["shot-second"]);
        let events = a.messages_after(chat.id, 0)?;
        assert!(
            events
                .iter()
                .any(|m| m.screenshot && m.text == "shot-later")
        );
        assert!(a.task_attachments(chat.task)?.is_empty());
        assert_eq!(album(&home)?.len(), 5);
        Ok(())
    }
}
