mod check;
pub mod cli;
mod git;
pub mod provider;
pub mod screenshots;
pub mod studio;
pub mod team;
pub mod usage;
pub mod worker_policy;

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn data_dir() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("WORKSHOP_HOME") {
        return Ok(p.into());
    }
    Ok(PathBuf::from(
        std::env::var_os("LOCALAPPDATA").context("Set WORKSHOP_HOME to a local data folder.")?,
    )
    .join("Robloxing/Workshop"))
}

#[derive(Clone, Debug)]
pub struct Attempt {
    pub id: i64,
    pub task: i64,
    pub prompt: String,
    pub state: String,
    pub action: String,
    pub head: String,
    pub base: String,
    pub session: String,
    pub model: String,
    pub effort: String,
    pub patch: String,
    pub candidate: String,
}

pub struct Store {
    pub project: PathBuf,
    pub dir: PathBuf,
    archive_dir: PathBuf,
    db: Mutex<Connection>,
    _lock: File,
    worker: bool,
    worker_catalog: tokio::sync::Mutex<Option<provider::Catalog>>,
    controls: Mutex<
        std::collections::HashMap<i64, tokio::sync::mpsc::UnboundedSender<provider::Control>>,
    >,
}

pub struct ChatMessage {
    pub screenshot: bool,
    pub id: i64,
    pub from_user: bool,
    pub delivery_notice: bool,
    pub text: String,
    pub dispatched_workers: Vec<i64>,
}

impl Store {
    pub fn messages_after(&self, attempt: i64, after: i64) -> Result<Vec<ChatMessage>> {
        self.message_range(attempt, after, i64::MAX, -1, false)
    }
    pub fn message_page(
        &self,
        attempt: i64,
        before: i64,
        limit: usize,
    ) -> Result<(Vec<ChatMessage>, bool)> {
        let mut rows =
            self.message_range(attempt, 0, before, (limit.clamp(1, 100) + 1) as i64, true)?;
        let more = rows.len() > limit.clamp(1, 100);
        if more {
            rows.pop();
        }
        rows.reverse();
        Ok((rows, more))
    }
    fn message_range(
        &self,
        attempt: i64,
        after: i64,
        before: i64,
        limit: i64,
        descending: bool,
    ) -> Result<Vec<ChatMessage>> {
        let db = self.db.lock().unwrap();
        let order = if descending { "DESC" } else { "ASC" };
        let mut stmt = db.prepare(&format!(
            "SELECT id,kind,summary FROM events
            WHERE attempt=?1 AND id>?2 AND id<?3 AND summary IS NOT NULL
            AND kind IN ('agent/message','item/completed','steer/sent','user/followup',
                         'steer/error','steer/unavailable','workshop/dispatch','workshop/screenshot','workshop/repair')
            ORDER BY id {order} LIMIT ?4"
        ))?;
        let mut rows = stmt.query(params![attempt, after, before, limit])?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next()? {
            let kind: String = row.get(1)?;
            let mut text: String = row.get(2)?;
            let mut dispatched_workers: Vec<i64> = Vec::new();
            if kind == "workshop/dispatch" {
                dispatched_workers = serde_json::from_str(&text)?;
                let count = dispatched_workers.len();
                text = format!(
                    "Dispatched {count} subagent{}",
                    if count == 1 { "" } else { "s" }
                );
            }
            messages.push(ChatMessage {
                screenshot: kind == "workshop/screenshot",
                id: row.get(0)?,
                from_user: matches!(kind.as_str(), "steer/sent" | "user/followup"),
                delivery_notice: matches!(
                    kind.as_str(),
                    "steer/error" | "steer/unavailable" | "workshop/repair"
                ),
                dispatched_workers,
                text,
            });
        }
        Ok(messages)
    }
    pub fn open(folder: &Path, home: &Path) -> Result<Self> {
        Self::open_role(folder, home, false)
    }
    pub fn project_data_dir(folder: &Path, home: &Path) -> Result<PathBuf> {
        let path = dunce::canonicalize(folder)?;
        let key = path.to_string_lossy();
        #[cfg(windows)]
        let key = key.to_lowercase();
        Ok(home.join(digest(key.as_bytes())))
    }
    fn open_role(folder: &Path, home: &Path, worker: bool) -> Result<Self> {
        fs::create_dir_all(folder)?;
        fs::create_dir_all(home)?;
        let folder = dunce::canonicalize(folder)?;
        let home = dunce::canonicalize(home)?;
        ensure!(
            !home.starts_with(&folder) && !folder.starts_with(&home),
            "Choose a project folder outside Workshop's data directory."
        );
        let dir = if worker {
            home.clone()
        } else {
            Self::project_data_dir(&folder, &home)?
        };
        fs::create_dir_all(&dir)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("project.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("This project is already open in Workshop.")?;
        let project = git::project(&folder)?;
        let mut db = Connection::open(dir.join("workshop.sqlite"))?;
        db.execute_batch(include_str!("schema.sql"))?;
        if !worker {
            db.execute(
                "ATTACH DATABASE ?1 AS preferences",
                [home.join("preferences.sqlite").to_string_lossy()],
            )?;
            db.execute_batch("CREATE TABLE IF NOT EXISTS preferences.worker_defaults(id INTEGER PRIMARY KEY CHECK(id=1),policy TEXT NOT NULL)")?;
            Self::init_screenshots(&db)?;
            db.execute_batch("CREATE TABLE IF NOT EXISTS preferences.studio_tools(name TEXT PRIMARY KEY, path TEXT NOT NULL)")?;
        }
        let tx = db.transaction()?;
        for (table, column, definition) in [
            ("team_settings", "lead_effort", "TEXT NOT NULL DEFAULT ''"),
            ("team_settings", "worker_effort", "TEXT NOT NULL DEFAULT ''"),
            (
                "team_settings",
                "auto_delegate",
                "INTEGER NOT NULL DEFAULT 1",
            ),
            ("attempts", "effort", "TEXT NOT NULL DEFAULT ''"),
        ] {
            let missing: bool = tx.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
                [table, column],
                |r| r.get(0),
            )?;
            if missing {
                tx.execute_batch(&format!(
                    "ALTER TABLE {table} ADD COLUMN {column} {definition}"
                ))?;
            }
        }
        tx.commit()?;
        studio::reports::recover(&dir, &mut db)?;
        db.execute("UPDATE studio_jobs SET state='Interrupted', detail='Workshop closed during this run.' WHERE state IN ('Running','Queued')", [])?;
        db.execute(
            "INSERT OR IGNORE INTO projects(id,path) VALUES(1,?1)",
            [project.to_string_lossy()],
        )?;
        db.execute("INSERT OR IGNORE INTO milestones(id,project,name) VALUES(1,1,'One recoverable change')", [])?;
        let store = Self {
            project,
            archive_dir: dir.clone(),
            dir,
            db: Mutex::new(db),
            _lock: lock,
            worker,
            worker_catalog: Default::default(),
            controls: Default::default(),
        };
        store.recover_workers()?;
        let interrupted = {
            let db = store.db.lock().unwrap();
            let mut query=db.prepare("SELECT id FROM attempts WHERE state IN ('preparing','running','applying') ORDER BY id")?;
            query
                .query_map([], |r| r.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for id in interrupted {
            let a = store.attempt(id)?;
            match a.state.as_str() {
                "preparing" => store.status(
                    a.id,
                    "failed",
                    "Setup was interrupted. Retry to create a working copy.",
                )?,
                "running" => {
                    if let Err(e) = store.finish(
                        a.id,
                        "interrupted",
                        "Interrupted. Review the saved output or resume.",
                    ) {
                        store.status(
                            a.id,
                            "failed",
                            &format!("Could not recover the working copy. {e:#}"),
                        )?;
                    }
                }
                "applying" => {
                    if let Err(e) = store.accept(a.id) {
                        store.status(
                            a.id,
                            "applying",
                            &format!("Acceptance needs attention. {e:#}"),
                        )?;
                    }
                }
                _ => {}
            }
        }
        Ok(store)
    }

    pub fn milestone(&self) -> Result<String> {
        Ok(self.db.lock().unwrap().query_row(
            "SELECT name FROM milestones WHERE id=1",
            [],
            |r| r.get(0),
        )?)
    }
    pub fn attempts(&self) -> Result<Vec<Attempt>> {
        self.attempt_range(None, -1, 0)
    }
    pub fn attempts_page(&self, offset: usize, limit: usize) -> Result<Vec<Attempt>> {
        self.attempt_range(None, limit.clamp(1, 101) as i64, offset as i64)
    }
    pub fn search_chats(&self, query: &str) -> Result<Vec<(i64, String)>> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT a.id, t.prompt FROM attempts a JOIN tasks t ON t.id=a.task
            WHERE instr(lower(t.prompt),lower(?1)) > 0 ORDER BY a.id DESC LIMIT 40",
        )?;
        Ok(stmt
            .query_map([query.trim()], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn chat_position(&self, id: i64) -> Result<usize> {
        Ok(self.db.lock().unwrap().query_row(
            "SELECT count(*) FROM attempts WHERE id > ?1",
            [id],
            |row| row.get(0),
        )?)
    }
    fn attempt_range(&self, id: Option<i64>, limit: i64, offset: i64) -> Result<Vec<Attempt>> {
        let db = self.db.lock().unwrap();
        let filter = if id.is_some() {
            "WHERE a.id=?1"
        } else {
            "WHERE ?1 IS NULL"
        };
        let mut stmt=db.prepare(&format!("SELECT a.id,t.id,t.prompt,a.state,a.action,a.head,a.base,a.session,a.model,a.patch,a.candidate,a.effort FROM attempts a JOIN tasks t ON a.task=t.id {filter} ORDER BY a.id DESC LIMIT ?2 OFFSET ?3"))?;
        Ok(stmt
            .query_map(params![id, limit, offset], |r| {
                Ok(Attempt {
                    id: r.get(0)?,
                    task: r.get(1)?,
                    prompt: r.get(2)?,
                    state: r.get(3)?,
                    action: r.get(4)?,
                    head: r.get(5)?,
                    base: r.get(6)?,
                    session: r.get(7)?,
                    model: r.get(8)?,
                    effort: r.get(11)?,
                    patch: r.get(9)?,
                    candidate: r.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn attempt(&self, id: i64) -> Result<Attempt> {
        self.attempt_range(Some(id), 1, 0)?
            .pop()
            .context("Select an attempt.")
    }
    pub fn folder(&self, id: i64) -> PathBuf {
        self.dir.join(format!("attempt-{id}"))
    }
    pub fn work(&self, id: i64) -> PathBuf {
        self.folder(id).join("work")
    }

    pub fn prepare(&self, prompt: &str, model: &str, retry: Option<i64>) -> Result<Attempt> {
        self.prepare_with_policy(prompt, model, retry, None)
    }
    pub fn prepare_with_policy(
        &self,
        prompt: &str,
        model: &str,
        retry: Option<i64>,
        policy: Option<&worker_policy::WorkerPolicy>,
    ) -> Result<Attempt> {
        self.prepare_with_files(prompt, model, retry, policy, &[])
    }
    pub fn prepare_with_files(
        &self,
        prompt: &str,
        model: &str,
        retry: Option<i64>,
        policy: Option<&worker_policy::WorkerPolicy>,
        files: &[PathBuf],
    ) -> Result<Attempt> {
        let settings = self.team_settings()?;
        if let Some(id) = retry {
            let old = self.attempt(id)?;
            return self.prepare_inference(prompt, model, &old.effort, retry, policy, &[]);
        }
        let effort = if model.is_empty() || model == settings.lead_model {
            &settings.lead_effort
        } else {
            ""
        };
        let model = if model.trim().is_empty() {
            &settings.lead_model
        } else {
            model
        };
        self.prepare_inference(prompt, model, effort, retry, policy, files)
    }
    pub(crate) fn task_attachments(&self, task: i64) -> Result<Vec<PathBuf>> {
        let db = self.db.lock().unwrap();
        let mut q = db.prepare("SELECT path FROM task_attachments WHERE task=?1 ORDER BY rowid")?;
        Ok(
            q.query_map([task], |r| Ok(PathBuf::from(r.get::<_, String>(0)?)))?
                .collect::<rusqlite::Result<_>>()?,
        )
    }
    pub fn set_chat_inference(&self, id: i64, model: &str, effort: &str) -> Result<()> {
        let n = self.db.lock().unwrap().execute("UPDATE attempts SET model=?2,effort=?3 WHERE id=?1 AND state IN ('review','failed','interrupted','cancelled')", params![id,model,effort])?;
        ensure!(
            n == 1,
            "Wait for the current response before changing the model."
        );
        Ok(())
    }
    fn prepare_inference(
        &self,
        prompt: &str,
        model: &str,
        effort: &str,
        retry: Option<i64>,
        policy: Option<&worker_policy::WorkerPolicy>,
        files: &[PathBuf],
    ) -> Result<Attempt> {
        ensure!(!prompt.trim().is_empty(), "Describe the task first.");
        let head = git::git(&self.project, &["rev-parse", "HEAD"])?;
        let id = {
            let mut db = self.db.lock().unwrap();
            let tx = db.transaction()?;
            ensure!(tx.query_row("SELECT count(*) FROM attempts WHERE state IN ('preparing','running','applying')", [], |r|r.get::<_,i64>(0))? == 0, "Finish the active operation first.");
            let task = if let Some(id) = retry {
                tx.query_row("SELECT task FROM attempts WHERE id=?1", [id], |r| {
                    r.get::<_, i64>(0)
                })?
            } else {
                tx.execute(
                    "INSERT INTO tasks(milestone,prompt) VALUES(1,?1)",
                    [prompt.trim()],
                )?;
                let task = tx.last_insert_rowid();
                for path in files {
                    tx.execute(
                        "INSERT INTO task_attachments(task,path) VALUES(?1,?2)",
                        params![task, path.to_str().context("Unreadable attachment path")?],
                    )?;
                }
                task
            };
            tx.execute("INSERT INTO attempts(task,head,model,effort,state,action) VALUES(?1,?2,?3,?4,'preparing','Creating isolated copy')",params![task,head,model,effort])?;
            let id = tx.last_insert_rowid();
            if let Some(policy) = policy {
                policy.validate()?;
                tx.execute(
                    "INSERT INTO chat_worker_policy(attempt,policy) VALUES(?1,?2)",
                    params![id, serde_json::to_string(policy)?],
                )?;
            } else if let Some(previous) = retry {
                tx.execute("INSERT INTO chat_worker_policy SELECT ?1,policy FROM chat_worker_policy WHERE attempt=?2", params![id,previous])?;
            }
            tx.commit()?;
            id
        };
        match git::isolate(&self.project, &self.work(id), &head) {
            Ok(base) => {
                self.db.lock().unwrap().execute("UPDATE attempts SET base=?2,state='running',action='Connecting to Codex' WHERE id=?1",params![id,base])?;
            }
            Err(e) => {
                self.status(id, "failed", &format!("{e:#}"))?;
                return Err(e);
            }
        }
        self.attempt(id)
    }

    pub fn resume(&self, id: i64) -> Result<Attempt> {
        let a = self.attempt(id)?;
        ensure!(
            !a.session.is_empty()
                && matches!(
                    a.state.as_str(),
                    "interrupted" | "failed" | "cancelled" | "review"
                ),
            "This attempt has no resumable session."
        );
        let changes=self.db.lock().unwrap().execute("UPDATE attempts SET state='running',action='Resuming session' WHERE id=?1 AND NOT EXISTS (SELECT 1 FROM attempts WHERE state IN ('running','preparing','applying'))",[id])?;
        ensure!(changes == 1, "Finish the active operation first.");
        self.attempt(id)
    }
    pub fn status(&self, id: i64, state: &str, action: &str) -> Result<()> {
        self.db.lock().unwrap().execute(
            "UPDATE attempts SET state=?2,action=?3 WHERE id=?1",
            params![id, state, action],
        )?;
        Ok(())
    }
    pub fn event(&self, id: i64, kind: &str, action: Option<&str>) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO events(attempt,kind,summary) VALUES(?1,?2,?3)",
            params![id, kind, action],
        )?;
        if let Some(action) = action {
            tx.execute(
                "UPDATE attempts SET action=?2 WHERE id=?1",
                params![id, action],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn session(&self, id: i64, session: &str) -> Result<()> {
        self.bind_session(id, session, "", "")
    }
    pub(crate) fn bind_session(
        &self,
        id: i64,
        session: &str,
        model: &str,
        effort: &str,
    ) -> Result<()> {
        self.db.lock().unwrap().execute(
            "UPDATE attempts SET session=?2, model=CASE WHEN model='' THEN ?3 ELSE model END, effort=CASE WHEN effort='' THEN ?4 ELSE effort END WHERE id=?1",
            params![id, session, model, effort],
        )?;
        Ok(())
    }
    pub fn artifact(&self, id: i64, kind: &str, path: &str) -> Result<()> {
        self.db.lock().unwrap().execute(
            "INSERT OR IGNORE INTO artifacts(attempt,kind,path) VALUES(?1,?2,?3)",
            params![id, kind, path],
        )?;
        Ok(())
    }

    pub fn finish(&self, id: i64, state: &str, action: &str) -> Result<()> {
        let a = self.attempt(id)?;
        let (candidate, bytes) = git::capture(&self.work(id), &a.base)?;
        let patch = if bytes.is_empty() {
            String::new()
        } else {
            digest(&bytes)
        };
        if !patch.is_empty() {
            let name = format!("{patch}.patch");
            let mut tmp = tempfile::NamedTempFile::new_in(self.folder(id))?;
            tmp.write_all(&bytes)?;
            tmp.as_file().sync_all()?;
            tmp.persist(self.folder(id).join(&name))?;
            self.artifact(id, "patch", &name)?;
        }
        let db = self.db.lock().unwrap();
        let mut summary = action.to_string();
        if state == "review" {
            use rusqlite::OptionalExtension;
            if let Some(last)=db.query_row("SELECT summary FROM events WHERE attempt=?1 AND kind='agent/message' ORDER BY id DESC LIMIT 1",[id],|r|r.get::<_,String>(0)).optional()? {
                summary=last;
            } else if bytes.is_empty() { summary="Codex finished without changing the working copy.".into(); }
        }
        db.execute(
            "UPDATE attempts SET state=?2,action=?3,candidate=?4,patch=?5 WHERE id=?1",
            params![id, state, summary, candidate, patch],
        )?;
        Ok(())
    }
    fn patch_path(&self, a: &Attempt) -> PathBuf {
        self.folder(a.id).join(format!("{}.patch", a.patch))
    }
    pub fn patch(&self, a: &Attempt) -> Result<Vec<u8>> {
        if a.patch.is_empty() {
            return Ok(vec![]);
        }
        let bytes = fs::read(self.patch_path(a)).context("Saved patch is missing")?;
        ensure!(
            digest(&bytes) == a.patch,
            "Saved patch has changed. Retry to produce a new review."
        );
        Ok(bytes)
    }
    pub fn accept(&self, id: i64) -> Result<()> {
        let a = self.attempt(id)?;
        if a.state == "accepted" {
            return Ok(());
        }
        if !self.worker {
            ensure!(
                self.candidate_ready(&a, &self.workers(id)?)?,
                "Resolve unfinished work and verify the final revision before accepting."
            );
            if a.state == "review" && a.patch.is_empty() && self.has_checkpoint(id)? {
                return self.decide(id, "accepted", "Decisions saved for the next chat");
            }
        }
        ensure!(
            matches!(
                a.state.as_str(),
                "review" | "interrupted" | "failed" | "cancelled" | "applying"
            ) && !a.patch.is_empty(),
            "This attempt has no change to accept."
        );
        self.patch(&a)?;
        let recovering = a.state == "applying";
        let work = self.work(id);
        let path = self.patch_path(&a);
        let review = git::Review {
            source: &self.project,
            work: &work,
            head: &a.head,
            base: &a.base,
            candidate: &a.candidate,
            patch: &path,
        };
        if let Err(e) = review.apply(recovering, || {
            self.status(id, "applying", "Applying reviewed change")
        }) {
            let state = self.attempt(id)?.state;
            self.status(id, &state, &format!("{e:#}"))?;
            return Err(e);
        }
        self.decide(id, "accepted", "Change applied to project")
    }
    pub fn discard(&self, id: i64) -> Result<()> {
        let a = self.attempt(id)?;
        if a.state == "discarded" {
            return Ok(());
        }
        if !matches!(
            a.state.as_str(),
            "review" | "interrupted" | "failed" | "cancelled"
        ) {
            bail!("This attempt cannot be discarded in its current state.");
        }
        self.decide(
            id,
            "discarded",
            "Discarded. Working copy and logs retained.",
        )
    }
    fn decide(&self, id: i64, state: &str, action: &str) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO decisions(attempt,decision) VALUES(?1,?2)",
            params![id, state],
        )?;
        tx.execute(
            "UPDATE attempts SET state=?2,action=?3 WHERE id=?1",
            params![id, state, action],
        )?;
        tx.commit()?;
        Ok(())
    }
}
