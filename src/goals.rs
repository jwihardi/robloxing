use crate::Store;
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, PartialEq)]
pub struct ChatGoal {
    pub objective: String,
    pub status: String,
    pub detail: String,
}

/// Match a whole command at the start of a message. A bare command opens the editor.
pub fn command(text: &str) -> Option<&str> {
    let tail = text.trim().strip_prefix("/goal")?;
    (tail.is_empty() || tail.starts_with(char::is_whitespace)).then(|| tail.trim())
}

pub(crate) fn set(db: &Connection, id: i64, objective: &str) -> Result<()> {
    let objective = objective.trim();
    ensure!(
        !objective.is_empty(),
        "Describe what you want the goal to achieve."
    );
    db.execute(
        "INSERT INTO chat_goals(attempt,objective,status) VALUES(?1,?2,'active')
         ON CONFLICT(attempt) DO UPDATE SET objective=excluded.objective,status='active',detail=''",
        params![id, objective],
    )?;
    Ok(())
}

impl Store {
    pub fn retry_prompt(&self, attempt: &crate::Attempt) -> Result<String> {
        Ok(self
            .chat_goal(attempt.id)?
            .map_or_else(|| attempt.prompt.clone(), |g| g.objective))
    }

    pub fn chat_goal(&self, id: i64) -> Result<Option<ChatGoal>> {
        Ok(self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT objective,CASE WHEN status='verifying' THEN 'active' ELSE status END,detail
                 FROM chat_goals WHERE attempt=?1",
                [id],
                |r| {
                    Ok(ChatGoal {
                        objective: r.get(0)?,
                        status: r.get(1)?,
                        detail: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn clear_goal(&self, id: i64) -> Result<()> {
        let changed = self.db.lock().unwrap().execute(
            "DELETE FROM chat_goals WHERE attempt=?1 AND NOT EXISTS
             (SELECT 1 FROM attempts WHERE id=?1 AND state IN ('running','preparing'))",
            [id],
        )?;
        ensure!(changed == 1, "Pause the goal before removing it.");
        Ok(())
    }

    pub fn pause_goal(&self, id: i64, detail: &str) -> Result<()> {
        self.db.lock().unwrap().execute(
            "UPDATE chat_goals SET status='paused',detail=?2 WHERE attempt=?1 AND status IN ('active','verifying')",
            params![id, detail],
        )?;
        Ok(())
    }

    pub(crate) fn report_goal(&self, id: i64, status: &str, evidence: &str) -> Result<()> {
        let kind = match status {
            "complete" => "goal/complete",
            "progress" => "goal/progress",
            _ => anyhow::bail!("Choose progress or complete."),
        };
        ensure!(
            !evidence.trim().is_empty(),
            "Describe what you did and verified."
        );
        let db = self.db.lock().unwrap();
        let written = db.execute(
            "INSERT INTO events(attempt,kind,summary) SELECT attempt,?2,?3 FROM chat_goals
             WHERE attempt=?1 AND status='active'",
            params![id, kind, evidence.trim()],
        )?;
        ensure!(written == 1, "This chat has no active goal.");
        Ok(())
    }
}

pub(crate) enum Next {
    Stop,
    Continue(String),
}

/// Only reports made during the current provider turn can advance the goal.
pub(crate) struct GoalRun {
    objective: String,
    after: i64,
    reminded: bool,
    progress: String,
}

impl GoalRun {
    pub(crate) fn start(store: &Store, id: i64) -> Result<Option<Self>> {
        if store.worker {
            return Ok(None);
        }
        Ok(store
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT objective,detail,(SELECT COALESCE(MAX(id),0) FROM events WHERE attempt=?1)
             FROM chat_goals WHERE attempt=?1 AND status='active'",
                [id],
                |r| {
                    Ok(Self {
                        objective: r.get(0)?,
                        progress: r.get(1)?,
                        after: r.get(2)?,
                        reminded: false,
                    })
                },
            )
            .optional()?)
    }

    pub(crate) fn reset_reports(&mut self, store: &Store, id: i64) -> Result<()> {
        self.after = store.db.lock().unwrap().query_row(
            "SELECT COALESCE(MAX(id),0) FROM events WHERE attempt=?1",
            [id],
            |r| r.get(0),
        )?;
        Ok(())
    }

    pub(crate) fn advance(&mut self, store: &Store, id: i64) -> Result<Next> {
        let mut db = store.db.lock().unwrap();
        let tx = db.transaction()?;
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM chat_goals WHERE attempt=?1 AND status='active')",
            [id],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(Next::Stop);
        }
        let report = tx.query_row(
            "SELECT kind,summary FROM events WHERE attempt=?1 AND id>?2 AND kind IN ('goal/progress','goal/complete') ORDER BY id DESC LIMIT 1",
            params![id,self.after], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))
        ).optional()?;
        // Older resumed sessions and text-only connections may lack the new tool.
        // Accept an explicit footer only after asking for it, from this turn's last assistant message.
        let report = match report {
            None if self.reminded => tx.query_row(
                "SELECT summary FROM events WHERE attempt=?1 AND id>?2 AND kind='agent/message' ORDER BY id DESC LIMIT 1",
                params![id,self.after], |r| r.get::<_,String>(0)
            ).optional()?.and_then(|text| footer(&text)),
            other => other,
        };
        self.after = tx.query_row(
            "SELECT COALESCE(MAX(id),0) FROM events WHERE attempt=?1",
            [id],
            |r| r.get(0),
        )?;
        let (status, detail, next) = match report {
            Some((kind, note)) if kind == "goal/complete" => ("verifying", note, Next::Stop),
            Some((kind, note)) if kind == "goal/blocked" => ("paused", note, Next::Stop),
            Some((_, note)) if note != self.progress => {
                self.progress.clone_from(&note);
                let direction = format!("Continue the chat goal: {}\nLast verified progress: {note}\nTake the next unfinished step. {}", self.objective, reporting(self.reminded));
                ("active", note, Next::Continue(direction))
            }
            Some(_) => ("paused", "The agent repeated its last progress report. Review the result before resuming.".into(), Next::Stop),
            None if self.reminded => ("paused", "The agent finished without reporting a goal outcome. Review its reply before resuming.".into(), Next::Stop),
            None => {
                self.reminded = true;
                return Ok(Next::Continue(format!("Check the chat goal against the work so far: {}\nContinue any unfinished work. {}", self.objective, reporting(true))));
            }
        };
        let changed = tx.execute(
            "UPDATE chat_goals SET status=?2,detail=?3 WHERE attempt=?1 AND status='active'",
            params![id, status, detail],
        )?;
        tx.commit()?;
        Ok(if changed == 1 { next } else { Next::Stop })
    }
}

pub(crate) fn reporting(fallback: bool) -> &'static str {
    if fallback {
        "Use workshop_goal_result to report progress or verified completion. If Workshop tools are unavailable, end your reply with one standalone line starting with [Goal progress], [Goal complete], or [Goal blocked], followed by concrete evidence or the input needed."
    } else {
        "Use workshop_goal_result with progress for a concrete new result, or complete only after checking the entire objective. Include evidence. If you need the creator to proceed, use workshop_blocked."
    }
}

fn footer(text: &str) -> Option<(String, String)> {
    let (last, before) = text
        .trim_end()
        .rsplit_once('\n')
        .map_or((text.trim_end(), ""), |(before, last)| (last, before));
    if before
        .lines()
        .filter(|line| line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~"))
        .count()
        % 2
        == 1
    {
        return None;
    }
    [
        ("[Goal progress] ", "goal/progress"),
        ("[Goal complete] ", "goal/complete"),
        ("[Goal blocked] ", "goal/blocked"),
    ]
    .into_iter()
    .find_map(|(prefix, kind)| {
        let detail = last.strip_prefix(prefix)?.trim();
        (!detail.is_empty()).then(|| (kind.into(), detail.into()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goals_require_a_whole_command_and_an_explicit_footer() {
        assert_eq!(command("  /goal  "), Some(""));
        assert_eq!(
            command("/goal\nCheck both\nviews"),
            Some("Check both\nviews")
        );
        assert_eq!(command("/goals later"), None);
        assert_eq!(command("Explain /goal"), None);
        assert_eq!(footer("Example:\n```\n[Goal complete] Example"), None);
        assert_eq!(footer("[Goal complete] Missing\nMore to do"), None);
        assert_eq!(footer("[Goal progress] "), None);
        assert_eq!(
            footer("Done\n[Goal complete] Both sizes passed.")
                .unwrap()
                .1,
            "Both sizes passed."
        );
    }

    #[test]
    fn stale_reports_cannot_complete_a_goal_or_restart_it_after_a_pause() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("project"), &dir.path().join("home"))?;
        let id = store.prepare("/goal Check both views", "", None)?.id;
        store.report_goal(id, "complete", "Old turn")?;
        let mut run = GoalRun::start(&store, id)?.unwrap();
        assert!(matches!(run.advance(&store, id)?, Next::Continue(_)));
        store.event(id, "steer/sent", Some("[Goal complete] A user message"))?;
        assert!(matches!(run.advance(&store, id)?, Next::Stop));
        assert_eq!(store.chat_goal(id)?.unwrap().status, "paused");
        assert!(store.report_goal(id, "progress", "Late result").is_err());
        assert!(matches!(run.advance(&store, id)?, Next::Stop));
        Ok(())
    }

    #[test]
    fn only_a_saved_review_completes_a_goal_and_a_retry_checks_its_own_copy() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let project = dir.path().join("project");
        let home = dir.path().join("home");
        let store = Store::open(&project, &home)?;
        let a = store.prepare("/goal Check the layout", "", None)?;
        assert_eq!(a.prompt, "Check the layout");
        let mut run = GoalRun::start(&store, a.id)?.unwrap();
        store.report_goal(a.id, "progress", "Opened the view")?;
        assert!(matches!(run.advance(&store, a.id)?, Next::Continue(_)));
        store.report_goal(a.id, "progress", "Opened the view")?;
        assert!(matches!(run.advance(&store, a.id)?, Next::Stop));
        assert_eq!(store.chat_goal(a.id)?.unwrap().status, "paused");
        store.finish(a.id, "review", "Paused")?;
        store.session(a.id, "test")?;
        store.resume_with_goal(a.id, Some("Check the new layout"))?;
        let mut run = GoalRun::start(&store, a.id)?.unwrap();
        store.report_goal(a.id, "complete", "Both sizes passed")?;
        assert!(matches!(run.advance(&store, a.id)?, Next::Stop));
        assert_eq!(store.chat_goal(a.id)?.unwrap().status, "active");
        store.finish(a.id, "review", "Saved")?;
        assert_eq!(store.chat_goal(a.id)?.unwrap().status, "complete");
        let separate = store.prepare("A regular chat", "", None)?;
        assert!(store.chat_goal(separate.id)?.is_none());
        store.finish(separate.id, "review", "Done")?;
        let retry = store.prepare(&a.prompt, "", Some(a.id))?;
        assert_eq!(store.retry_prompt(&retry)?, "Check the new layout");
        assert_eq!(store.chat_goal(retry.id)?.unwrap().detail, "");
        drop(store);
        let recovered = Store::open(&project, &home)?;
        assert_eq!(recovered.chat_goal(retry.id)?.unwrap().status, "paused");
        assert_eq!(recovered.chat_goal(a.id)?.unwrap().status, "complete");
        recovered.clear_goal(retry.id)?;
        assert!(recovered.chat_goal(retry.id)?.is_none());
        Ok(())
    }
}
