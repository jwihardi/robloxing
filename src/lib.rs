mod check;
mod git;
pub mod provider;

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
    pub patch: String,
    pub candidate: String,
}

pub struct Store {
    pub project: PathBuf,
    pub dir: PathBuf,
    db: Mutex<Connection>,
    _lock: File,
}

#[derive(Clone, PartialEq)]
pub struct Activity {
    pub time: String,
    pub kind: String,
    pub summary: String,
}

impl Store {
    /// Recent human-readable updates for the agent window, including older provider events.
    pub fn recent_activity(&self, id: i64) -> Result<Vec<Activity>> {
        let db = self.db.lock().unwrap();
        let mut query = db.prepare(
            "SELECT strftime('%H:%M',recorded,'localtime'),kind,summary FROM events
             WHERE attempt=? AND summary IS NOT NULL AND
             (kind IN ('agent/message','item/completed','turn/started','check','steer/sent',
                       'steer/error','steer/unavailable','error') OR
              (kind='item/started' AND summary IN ('Editing project files','Thinking through the task')))
             ORDER BY id DESC LIMIT 40",
        )?;
        Ok(query
            .query_map([id], |r| {
                Ok(Activity {
                    time: r.get(0)?,
                    kind: r.get(1)?,
                    summary: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn open(folder: &Path, home: &Path) -> Result<Self> {
        fs::create_dir_all(folder)?;
        fs::create_dir_all(home)?;
        let folder = dunce::canonicalize(folder)?;
        let home = dunce::canonicalize(home)?;
        ensure!(
            !home.starts_with(&folder) && !folder.starts_with(&home),
            "Choose a project folder outside Workshop's data directory."
        );
        let key = folder.to_string_lossy().to_string();
        #[cfg(windows)]
        let key = key.to_lowercase();
        let dir = home.join(digest(key.as_bytes()));
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
        let db = Connection::open(dir.join("workshop.sqlite"))?;
        db.execute_batch(include_str!("schema.sql"))?;
        db.execute(
            "INSERT OR IGNORE INTO projects(id,path) VALUES(1,?1)",
            [project.to_string_lossy()],
        )?;
        db.execute("INSERT OR IGNORE INTO milestones(id,project,name) VALUES(1,1,'One recoverable change')", [])?;
        let store = Self {
            project,
            dir,
            db: Mutex::new(db),
            _lock: lock,
        };
        for a in store.attempts()? {
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
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT a.id,t.id,t.prompt,a.state,a.action,a.head,a.base,a.session,a.model,a.patch,a.candidate FROM attempts a JOIN tasks t ON a.task=t.id ORDER BY a.id DESC")?;
        Ok(stmt
            .query_map([], |r| {
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
                    patch: r.get(9)?,
                    candidate: r.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn attempt(&self, id: i64) -> Result<Attempt> {
        self.attempts()?
            .into_iter()
            .find(|a| a.id == id)
            .context("Select an attempt.")
    }
    pub fn folder(&self, id: i64) -> PathBuf {
        self.dir.join(format!("attempt-{id}"))
    }
    pub fn work(&self, id: i64) -> PathBuf {
        self.folder(id).join("work")
    }

    pub fn prepare(&self, prompt: &str, model: &str, retry: Option<i64>) -> Result<Attempt> {
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
                tx.last_insert_rowid()
            };
            tx.execute("INSERT INTO attempts(task,head,model,state,action) VALUES(?1,?2,?3,'preparing','Creating isolated copy')",params![task,head,model])?;
            let id = tx.last_insert_rowid();
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
        self.db.lock().unwrap().execute(
            "UPDATE attempts SET session=?2 WHERE id=?1",
            params![id, session],
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
