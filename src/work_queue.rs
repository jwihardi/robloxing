use crate::Store;
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, params};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
pub struct WorkItem {
    pub id: i64,
    pub title: String,
    pub provider: String,
    pub status: &'static str,
    pub next_action: String,
    pub running_workers: i64,
    pub queued_workers: i64,
    pub queued_messages: i64,
}

// Each row is the latest attempt at a task. Read the same chat, worker and goal
// records the scheduler writes, in a single SQLite snapshot.
fn page(db: &Connection, offset: usize, limit: usize) -> Result<Vec<WorkItem>> {
    let mut stmt = db.prepare_cached(
        "WITH pending AS (
           SELECT a.id,t.prompt,a.provider,a.state,a.action,a.patch,g.status goal,g.detail,
                  c.next_task,b.summary blocker,
                  (SELECT detail FROM workers WHERE lead=a.id AND state='blocked' ORDER BY id LIMIT 1) worker_blocker,
                  (SELECT count(*) FROM workers WHERE lead=a.id AND state='running') running,
                  (SELECT count(*) FROM workers WHERE lead=a.id AND state='queued') waiting,
                  (SELECT count(*) FROM queued_messages WHERE attempt=a.id) messages
           FROM attempts a JOIN tasks t ON t.id=a.task
           LEFT JOIN chat_goals g ON g.attempt=a.id LEFT JOIN checkpoints c ON c.lead=a.id
           LEFT JOIN events b ON b.id=(SELECT id FROM events WHERE attempt=a.id
                 AND kind IN ('workshop/blocker','workshop/blocker_resolved') ORDER BY id DESC LIMIT 1)
                 AND b.kind='workshop/blocker'
           WHERE a.state NOT IN ('accepted','discarded')
             AND NOT EXISTS (SELECT 1 FROM attempts newer WHERE newer.task=a.task AND newer.id>a.id)
         ), ranked AS (
           SELECT *, CASE
             WHEN state='applying' AND action LIKE 'Acceptance needs attention.%' THEN 0
             WHEN state IN ('preparing','running','applying') THEN 2
             WHEN state='cancelled' AND messages=0 AND coalesce(goal,'')<>'paused' THEN NULL
             WHEN blocker IS NOT NULL OR worker_blocker IS NOT NULL OR state='failed' THEN 0
             WHEN goal='paused' OR state='interrupted' THEN 1
             WHEN state='review' AND (patch<>'' OR next_task IS NOT NULL) THEN 3
             WHEN messages>0 OR waiting>0 OR goal IN ('active','verifying') THEN 4
           END priority FROM pending
         )
         SELECT id,substr(prompt,1,180),provider,priority,
           substr(CASE priority
             WHEN 0 THEN coalesce(blocker,nullif(worker_blocker,''),action)
             WHEN 1 THEN coalesce(nullif(detail,''),action)
             WHEN 2 THEN action
             WHEN 3 THEN CASE WHEN patch<>'' THEN 'Review the proposed changes' ELSE coalesce(nullif(next_task,''),'Review the project handoff') END
             ELSE CASE WHEN messages>0 THEN 'Send the next queued message' ELSE coalesce(nullif(detail,''),'Continue the chat') END
           END,1,500),running,waiting,messages
         FROM ranked WHERE priority IS NOT NULL ORDER BY priority,id DESC LIMIT ?1 OFFSET ?2",
    )?;
    let rows = stmt.query_map(
        params![limit.clamp(1, 201) as i64, i64::try_from(offset)?],
        |r| {
            let title: String = r.get(1)?;
            Ok(WorkItem {
                id: r.get(0)?,
                title: title.lines().next().unwrap_or("Untitled chat").to_owned(),
                provider: r.get(2)?,
                status: [
                    "Needs attention",
                    "Paused",
                    "In progress",
                    "Review",
                    "Queued",
                ][r.get::<_, usize>(3)?],
                next_action: r.get(4)?,
                running_workers: r.get(5)?,
                queued_workers: r.get(6)?,
                queued_messages: r.get(7)?,
            })
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

impl Store {
    pub fn work_queue(&self, offset: usize, limit: usize) -> Result<Vec<WorkItem>> {
        page(&self.db.lock().unwrap(), offset, limit)
    }
}

pub fn read(project: &Path, home: &Path, offset: usize, limit: usize) -> Result<Vec<WorkItem>> {
    let path = Store::project_data_dir(project, home)?.join("workshop.sqlite");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .context("Could not read the project's work queue")?;
    page(&db, offset, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_readers_share_blockers_handoffs_and_retries_with_the_open_project() -> Result<()> {
        let scratch = tempfile::tempdir()?;
        let home = scratch.path().join("data");
        let project = scratch.path().join("project");
        let store = Store::open(&project, &home)?;
        let chat = store.prepare("hello", "", None)?;
        store.finish(chat.id, "review", "Hello")?;
        assert!(store.work_queue(0, 10)?.is_empty());
        let review = store.prepare("Plan the menu", "", None)?;
        store.checkpoint(review.id, "Use a radial menu", "Playtest with a controller")?;
        store.finish(review.id, "review", "Ready")?;
        let blocked = store.prepare("Test the lobby", "", None)?;
        store.event(
            blocked.id,
            "workshop/blocker",
            Some("Choose the test place"),
        )?;
        store.finish(blocked.id, "failed", "Needs input")?;
        let rows = read(&project, &home, 0, 5)?;
        assert_eq!(
            rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            [blocked.id, review.id]
        );
        assert_eq!(rows[0].next_action, "Choose the test place");
        assert_eq!(
            store.work_queue(1, 1)?[0].next_action,
            "Playtest with a controller"
        );
        let retry = store.prepare("Test the lobby", "", Some(blocked.id))?;
        let rows = read(&project, &home, 0, 10)?;
        assert_eq!((rows[0].id, rows[0].status), (retry.id, "In progress"));
        assert_eq!(rows.len(), 2);
        store.finish(retry.id, "cancelled", "Stopped")?;
        let paused = store.prepare("/goal Playtest the lobby", "", None)?;
        store.pause_goal(paused.id, "Waiting for the playtest")?;
        store.finish(paused.id, "cancelled", "Stopped")?;
        let rows = read(&project, &home, 0, 10)?;
        assert_eq!((rows[0].id, rows[0].status), (paused.id, "Paused"));
        assert_eq!(rows[0].next_action, "Waiting for the playtest");
        assert_eq!(rows.len(), 2);
        Ok(())
    }
}
