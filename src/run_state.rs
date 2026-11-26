use crate::{Attempt, Store, goals, provider::Message};
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};

// Older databases only record creation order. A live attempt, or a delivered
// follow-up after the latest retry started, identifies a later activation.
const LEGACY_HEADS: &str = "WITH bounds AS (
    SELECT task,min(id) first,max(id) newest,
           max(CASE WHEN state IN ('preparing','running','applying') THEN id END) live
    FROM attempts GROUP BY task
) SELECT task,coalesce(live, CASE WHEN first<>newest THEN (
    SELECT e.attempt FROM attempts a JOIN events e ON e.attempt=a.id
    WHERE a.task=bounds.task AND e.kind='user/followup'
      AND e.id>(SELECT min(id) FROM events WHERE attempt=bounds.newest)
    ORDER BY e.id DESC LIMIT 1
) END,newest) AS attempt FROM bounds";

const OPEN_BLOCKERS: &str = "SELECT e.attempt,e.summary FROM events e
    WHERE e.kind='workshop/blocker' AND NOT EXISTS (
        SELECT 1 FROM events later WHERE later.attempt=e.attempt AND later.id>e.id
        AND later.kind IN ('workshop/blocker','workshop/blocker_resolved'))";

pub(crate) fn migrate(db: &Connection) -> Result<()> {
    if !db.table_exists(None, "task_heads")? {
        db.execute_batch(&format!(
            "CREATE TABLE task_heads(task INTEGER PRIMARY KEY REFERENCES tasks, attempt INTEGER NOT NULL REFERENCES attempts);
             INSERT INTO task_heads(task,attempt) {LEGACY_HEADS};
             CREATE TRIGGER task_head_on_creation AFTER INSERT ON attempts BEGIN
                 INSERT INTO task_heads VALUES(NEW.task,NEW.id) ON CONFLICT(task) DO UPDATE SET attempt=excluded.attempt;
             END;
             CREATE TRIGGER task_head_on_resume AFTER UPDATE OF state ON attempts
             WHEN NEW.state='running' AND OLD.state NOT IN ('preparing','running') BEGIN
                 UPDATE task_heads SET attempt=NEW.id WHERE task=NEW.task;
             END;
             CREATE VIEW open_workshop_blockers AS {OPEN_BLOCKERS};
             DROP VIEW IF EXISTS recoverable_goals;"
        ))?;
    }
    db.execute_batch(
        "CREATE VIEW IF NOT EXISTS recoverable_goals AS
        SELECT g.attempt,g.objective,g.detail FROM chat_goals g
        JOIN attempts a ON a.id=g.attempt JOIN task_heads h ON h.task=a.task AND h.attempt=a.id
        WHERE g.recoverable=1 AND g.status='paused' AND a.state='interrupted'
        AND NOT EXISTS(SELECT 1 FROM open_workshop_blockers b WHERE b.attempt=a.id)",
    )?;
    Ok(())
}

// A read-only status command can inspect an older project before its first
// writable open. All compatibility objects live on this reader connection.
pub(crate) fn prepare_reader(db: &Connection) -> Result<()> {
    if db.table_exists(None, "task_heads")? {
        return Ok(());
    }
    db.execute_batch(&format!(
        "CREATE TEMP VIEW task_heads AS {LEGACY_HEADS};
         CREATE TEMP VIEW open_workshop_blockers AS {OPEN_BLOCKERS};"
    ))?;
    for (table, columns) in [
        (
            "chat_goals",
            "NULL AS attempt,NULL AS status,NULL AS detail",
        ),
        ("checkpoints", "NULL AS lead,NULL AS next_task"),
        (
            "workers",
            "NULL AS id,NULL AS lead,NULL AS state,NULL AS detail",
        ),
        ("queued_messages", "NULL AS attempt,NULL AS sending"),
    ] {
        if !db.table_exists(None, table)? {
            db.execute_batch(&format!(
                "CREATE TEMP VIEW {table} AS SELECT {columns} WHERE 0"
            ))?;
        }
    }
    Ok(())
}

pub(crate) enum Resume {
    Conversation,
    QueuedMessage,
    ReopenForQueue,
    InterruptedGoal,
}

impl Resume {
    pub(crate) fn claim(self, db: &Connection, chat: i64) -> Result<()> {
        let action = match self {
            Self::QueuedMessage => "Sending queued message",
            Self::ReopenForQueue => "Reopening original request before queued messages",
            Self::Conversation | Self::InterruptedGoal => "Resuming session",
        };
        let changed = db.execute(
            "UPDATE attempts SET state='running',action=?2 WHERE id=?1
             AND (session<>'' OR ?3) AND state IN ('interrupted','failed','cancelled','review')
             AND NOT EXISTS (SELECT 1 FROM attempts WHERE state IN ('running','preparing','applying'))",
            params![chat, action, matches!(self, Self::InterruptedGoal | Self::ReopenForQueue)],
        )?;
        ensure!(
            changed == 1,
            "This chat cannot resume. Finish any active operation and select a resumable chat."
        );
        Ok(())
    }
}

impl Store {
    pub fn retry_chat(&self, id: i64) -> Result<(Attempt, Message)> {
        let old = self.attempt(id)?;
        let attempt = self.prepare(&old.prompt, &old.model, Some(id))?;
        let message = Message::from(self.retry_prompt(&attempt)?.as_str());
        Ok((attempt, message))
    }

    /// Start a follow-up from either frontend. Resubmitting an interrupted
    /// objective resumes its checkpoint; editing it deliberately starts a new goal.
    pub fn resume_chat(
        &self,
        id: i64,
        message: Message,
        objective: Option<&str>,
    ) -> Result<(Attempt, Message)> {
        if let Some(objective) = objective
            && let Some((attempt, direction)) =
                self.recover_matching_goal(Some(id), Some(objective.trim()))?
        {
            let mut files = if attempt.session.is_empty() {
                self.task_attachments(attempt.task)?
            } else {
                Vec::new()
            };
            for file in message.files {
                if !files.contains(&file) {
                    files.push(file);
                }
            }
            return Ok((attempt, Message::compose(&direction, files)));
        }
        Ok((self.resume_with_goal(id, objective)?, message))
    }

    pub fn resume(&self, id: i64) -> Result<Attempt> {
        self.resume_with_goal(id, None)
    }

    pub fn resume_with_goal(&self, id: i64, objective: Option<&str>) -> Result<Attempt> {
        {
            let mut db = self.db.lock().unwrap();
            let tx = db.transaction()?;
            Resume::Conversation.claim(&tx, id)?;
            if let Some(objective) = objective {
                goals::set(&tx, id, objective)?;
            }
            tx.commit()?;
        }
        self.attempt(id)
    }
}
