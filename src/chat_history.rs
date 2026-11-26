use crate::Store;
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use std::{fs, io::ErrorKind, path::Path};

// The live and retired maxima are both indexed. Publishing an ID and its
// workspace record happens in the caller's existing write transaction.
pub(crate) fn next_chat_id(db: &Connection) -> Result<i64> {
    Ok(db.query_row(
        "SELECT 1+max(ifnull((SELECT max(id) FROM attempts),0),
        ifnull((SELECT max(id) FROM retired_workspaces WHERE kind='chat'),0))",
        [],
        |r| r.get(0),
    )?)
}

pub(crate) fn next_worker_id(db: &Connection) -> Result<i64> {
    Ok(db.query_row(
        "SELECT 1+max(ifnull((SELECT max(id) FROM workers),0),
        ifnull((SELECT max(id) FROM retired_workspaces WHERE kind='worker'),0))",
        [],
        |r| r.get(0),
    )?)
}

fn remove_workspace(root: &Path, kind: &str, id: i64) -> Result<()> {
    ensure!(id > 0, "Invalid private workspace ID.");
    let root = dunce::canonicalize(root)?;
    let path = match kind {
        "chat" => root.join(format!("attempt-{id}")),
        "worker" => root.join("workers").join(id.to_string()),
        _ => anyhow::bail!("Unknown private workspace kind."),
    };
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
        Ok(metadata) => ensure!(
            metadata.is_dir() && dunce::canonicalize(&path)? == path,
            "Expected a private workspace directory inside Workshop data."
        ),
    }
    fs::remove_dir_all(path)?;
    Ok(())
}

impl Store {
    pub fn is_deleted_chat(&self, id: i64) -> Result<bool> {
        Ok(self.db.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM retired_workspaces WHERE kind='chat' AND id=?1)",
            [id],
            |r| r.get(0),
        )?)
    }

    /// Remove the conversation atomically and record its private directories for cleanup.
    pub fn delete_chat(&self, id: i64) -> Result<()> {
        let removed = self
            .db
            .lock()
            .unwrap()
            .execute("DELETE FROM attempts WHERE id=?1", [id])?;
        ensure!(
            removed > 0 || self.is_deleted_chat(id)?,
            "This chat could not be found."
        );
        Ok(())
    }

    /// Returns true when a directory is still in use or could not be removed.
    pub fn clean_deleted_workspaces(&self) -> Result<bool> {
        let _cleanup = self.workspace_cleanup.lock().unwrap();
        let rows = {
            let db = self.db.lock().unwrap();
            let mut query = db.prepare("SELECT kind,id FROM retired_workspaces WHERE pending=1")?;
            query
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut pending = false;
        for (kind, id) in rows {
            if remove_workspace(&self.dir, &kind, id).is_ok() {
                self.db.lock().unwrap().execute(
                    "UPDATE retired_workspaces SET pending=0 WHERE kind=?1 AND id=?2",
                    params![kind, id],
                )?;
            } else {
                pending = true;
            }
        }
        Ok(pending)
    }
}
