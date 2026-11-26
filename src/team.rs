use crate::{Attempt, Store, git, provider};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::stream::{FuturesUnordered, StreamExt};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub lead_model: String,
    pub worker_model: String,
    pub lead_effort: String,
    pub worker_effort: String,
    #[serde(default)]
    pub override_worker_model: bool,
    #[serde(default)]
    pub override_worker_effort: bool,
    pub parallelism: usize,
    pub auto_delegate: bool,
    pub check_args: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub key: String,
    pub title: String,
    pub deliverable: String,
    pub acceptance: String,
    pub files: Vec<String>,
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub effort: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Worker {
    pub id: i64,
    pub assignment: Assignment,
    pub state: String,
    pub run: Option<i64>,
    pub tries: usize,
    pub detail: String,
    pub repeats: usize,
    pub tokens: Option<i64>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

pub(crate) struct WorkerControl {
    input: tokio::sync::mpsc::UnboundedSender<provider::Control>,
    // Taken when discovery finishes so later directions use live steering.
    preparing: Option<Vec<provider::Message>>,
}

fn clipped(s: &str, limit: usize) -> String {
    s.chars().take(limit).collect()
}
fn covers(parent: &str, child: &str) -> bool {
    let parent = parent.to_lowercase();
    let child = child.to_lowercase();
    parent == "."
        || parent == child
        || child
            .strip_prefix(&parent)
            .is_some_and(|rest| rest.starts_with('/'))
}
fn overlaps(a: &Assignment, b: &Assignment) -> bool {
    a.files
        .iter()
        .any(|p| b.files.iter().any(|q| covers(p, q) || covers(q, p)))
}
// Preserve explicit dependencies first. For overlapping paths the earlier task in
// this stable topological order owns the file until it finishes successfully.
fn next_wave(workers: &[Worker], limit: usize) -> Vec<&Worker> {
    if limit == 0 {
        return Vec::new();
    }
    let mut ordered: Vec<&Worker> = Vec::new();
    let mut visited = HashSet::new();
    let mut ready = Vec::new();
    while let Some(w) = workers.iter().find(|w| {
        !visited.contains(&w.assignment.key)
            && w.assignment
                .depends_on
                .iter()
                .all(|key| visited.contains(key))
    }) {
        if w.state == "queued"
            && ordered.iter().all(|prior| {
                prior.state == "finished"
                    || (!w.assignment.depends_on.contains(&prior.assignment.key)
                        && !overlaps(&w.assignment, &prior.assignment))
            })
        {
            ready.push(w);
            if ready.len() == limit {
                break;
            }
        }
        visited.insert(w.assignment.key.clone());
        ordered.push(w);
    }
    ready
}
fn validate(a: &Assignment) -> Result<()> {
    ensure!(
        !a.key.is_empty()
            && a.key.len() <= 64
            && a.key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
        "Use a short, stable task key"
    );
    ensure!(
        !a.title.trim().is_empty()
            && a.title.len() <= 100
            && !a.deliverable.trim().is_empty()
            && !a.acceptance.trim().is_empty(),
        "Give each worker a title, deliverable and acceptance criteria"
    );
    ensure!(
        a.files.len() <= 128 && a.deliverable.len() + a.acceptance.len() <= 24000,
        "Assignment is too large"
    );
    for p in &a.files {
        ensure!(
            p == "."
                || (!p.contains(['\\', ':', '*', '?'])
                    && p.split('/').all(|c| !c.is_empty()
                        && c != "."
                        && c != ".."
                        && !c.eq_ignore_ascii_case(".git"))),
            "Use exact project-relative file or directory paths: {p}"
        );
    }
    Ok(())
}

impl Store {
    pub fn team_settings(&self) -> Result<Settings> {
        let (mut settings, argv) = self.db.lock().unwrap().query_row(
            "SELECT lead_model, worker_model, parallelism, check_args, lead_effort,
                    worker_effort, auto_delegate, override_worker_model, override_worker_effort
             FROM team_settings WHERE id=1",
            [],
            |row| {
                Ok((
                    Settings {
                        lead_model: row.get(0)?,
                        worker_model: row.get(1)?,
                        parallelism: row.get(2)?,
                        lead_effort: row.get(4)?,
                        worker_effort: row.get(5)?,
                        auto_delegate: row.get(6)?,
                        override_worker_model: row.get(7)?,
                        override_worker_effort: row.get(8)?,
                        check_args: Vec::new(),
                    },
                    row.get::<_, String>(3)?,
                ))
            },
        )?;
        settings.check_args = serde_json::from_str(&argv)?;
        Ok(settings)
    }
    pub fn configure_team(&self, settings: &Settings) -> Result<()> {
        ensure!(
            (1..=2).contains(&settings.parallelism),
            "Choose one or two concurrent workers"
        );
        ensure!(
            settings
                .check_args
                .first()
                .is_none_or(|x| !x.trim().is_empty()),
            "Choose a check executable"
        );
        if settings.override_worker_effort {
            ensure!(
                settings.worker_effort.is_empty()
                    || crate::worker_policy::EFFORTS.contains(&settings.worker_effort.as_str()),
                "Unknown maximum worker effort"
            );
        }
        self.db.lock().unwrap().execute(
            "UPDATE team_settings SET lead_model=?1, worker_model=?2, parallelism=?3,
             check_args=?4, lead_effort=?5, worker_effort=?6, auto_delegate=?7,
             override_worker_model=?8, override_worker_effort=?9 WHERE id=1",
            params![
                settings.lead_model.trim(),
                settings.worker_model.trim(),
                settings.parallelism,
                serde_json::to_string(&settings.check_args)?,
                settings.lead_effort,
                settings.worker_effort,
                settings.auto_delegate,
                settings.override_worker_model,
                settings.override_worker_effort
            ],
        )?;
        Ok(())
    }
    pub fn workers(&self, lead: i64) -> Result<Vec<Worker>> {
        let db = self.db.lock().unwrap();
        let mut q=db.prepare("SELECT id,spec,state,run,tries,detail,repeats,tokens,model,effort FROM workers WHERE lead=?1 ORDER BY id")?;
        let rows = q.query_map([lead], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, String>(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
            ))
        })?;
        rows.map(|row| {
            let (id, spec, state, run, tries, detail, repeats, tokens, model, effort) = row?;
            Ok(Worker {
                id,
                assignment: serde_json::from_str(&spec)?,
                state,
                run,
                tries,
                detail,
                repeats,
                tokens,
                model,
                effort,
            })
        })
        .collect()
    }
    fn worker_state(&self, id: i64, state: &str, detail: &str) -> Result<()> {
        self.db.lock().unwrap().execute(
            "UPDATE workers SET state=?2,detail=?3 WHERE id=?1",
            params![id, state, clipped(detail, 4000)],
        )?;
        Ok(())
    }
    fn block_worker(&self, id: i64, err: &str) -> Result<()> {
        let reason = clipped(err, 4000);
        let signature = crate::digest(reason.as_bytes());
        self.db.lock().unwrap().execute("UPDATE workers SET state='blocked',detail=?2,repeats=CASE WHEN signature=?3 THEN repeats+1 ELSE 1 END,signature=?3 WHERE id=?1",params![id,reason,signature])?;
        Ok(())
    }
    fn child_store(&self, lead: i64, worker: i64) -> Result<Store> {
        let mut child = Store::open_role(
            &self.work(lead),
            &self.dir.join("workers").join(worker.to_string()),
            true,
        )?;
        child.archive_dir.clone_from(&self.archive_dir);
        child.preferences_home.clone_from(&self.preferences_home);
        Ok(child)
    }
    fn queue(&self, lead: i64, assignments: &[Assignment]) -> Result<()> {
        ensure!(
            !assignments.is_empty() && assignments.len() <= 16,
            "Delegate between one and sixteen tasks per batch"
        );
        let existing = self.workers(lead)?;
        let mut graph = existing
            .iter()
            .map(|w| (w.assignment.key.clone(), w.assignment.depends_on.clone()))
            .collect::<HashMap<_, _>>();
        let mut keys = HashSet::new();
        for a in assignments {
            validate(a)?;
            ensure!(keys.insert(&a.key), "Duplicate task key {}", a.key);
            if let Some(w) = existing.iter().find(|w| w.assignment.key == a.key) {
                ensure!(
                    w.assignment == *a || w.state == "blocked",
                    "Task {} already exists with another assignment",
                    a.key
                );
            }
            graph.insert(a.key.clone(), a.depends_on.clone());
        }
        ensure!(
            graph.len() <= 128,
            "Conversation already has 128 delegated tasks"
        );
        let mut ordered = HashSet::new();
        while ordered.len() < graph.len() {
            let before = ordered.len();
            for (name, deps) in &graph {
                if deps.iter().all(|dep| ordered.contains(dep)) {
                    ordered.insert(name.clone());
                }
            }
            ensure!(
                ordered.len() > before,
                "Unknown dependency or dependency cycle in delegated tasks"
            );
        }
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let mut dispatched = Vec::new();
        for a in assignments {
            let next_id = crate::chat_history::next_worker_id(&tx)?;
            let id = tx
                .query_row(
                    "INSERT INTO workers(lead,name,spec,state,id) VALUES(?1,?2,?3,'queued',?4)
                 ON CONFLICT(lead,name) DO UPDATE SET state='queued',spec=excluded.spec
                 WHERE workers.state='blocked' AND workers.repeats<2 AND workers.tries<4
                 RETURNING id",
                    params![lead, a.key, serde_json::to_string(a)?, next_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if let Some(id) = id {
                dispatched.push(id);
            }
        }
        if !dispatched.is_empty() {
            tx.execute(
                "INSERT INTO events(attempt,kind,summary) VALUES(?1,'workshop/dispatch',?2)",
                params![lead, serde_json::to_string(&dispatched)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn recover_workers(&self) -> Result<()> {
        if self.worker {
            return Ok(());
        }
        self.cancel_preparing_directions(None, None)?;
        let pending: Vec<(i64, i64, Option<i64>, String)> = {
            let db = self.db.lock().unwrap();
            let mut q = db.prepare(
                "SELECT id,lead,run,state FROM workers WHERE state IN ('running','integrating')",
            )?;
            q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        for (id, lead, run, state) in pending {
            if state == "integrating"
                && let Some(run) = run
            {
                let repair = (|| -> Result<()> {
                    let child = self.child_store(lead, id)?;
                    child.accept(run)?;
                    Ok(())
                })();
                match repair {
                    Ok(()) => self.worker_state(
                        id,
                        "checking",
                        "Recovered the integrated patch; waiting for its combined check.",
                    )?,
                    Err(e) => self.block_worker(id, &format!("{e:#}"))?,
                }
                continue;
            }
            self.worker_state(
                id,
                "blocked",
                "Interrupted. Resume the lead to finish this assignment from the saved work.",
            )?;
        }
        Ok(())
    }
    pub fn checkpoint(&self, lead: i64, decisions: &str, next: &str) -> Result<()> {
        self.db.lock().unwrap().execute("INSERT INTO checkpoints VALUES(?1,?2,?3) ON CONFLICT(lead) DO UPDATE SET decisions=?2,next_task=?3",params![lead,clipped(decisions,4000),clipped(next,2000)])?;
        Ok(())
    }
    pub fn handoff(&self, lead: i64) -> Result<String> {
        let db = self.db.lock().unwrap();
        let mut q=db.prepare("SELECT c.decisions,c.next_task FROM checkpoints c JOIN attempts a ON a.id=c.lead WHERE c.lead=?1 OR a.state='accepted' ORDER BY (c.lead=?1) DESC,c.lead DESC LIMIT 3")?;
        let rows = q
            .query_map([lead], |r| {
                Ok(format!(
                    "Decisions: {}\nNext: {}",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(q);
        drop(db);
        let mut text = rows.join("\n");
        for w in self
            .workers(lead)?
            .into_iter()
            .filter(|w| w.state != "finished")
            .take(16)
        {
            text.push_str(&format!(
                "\n{} [{}]: {} — {}",
                w.assignment.key,
                w.state,
                w.assignment.deliverable,
                clipped(&w.detail, 300)
            ));
        }
        Ok(clipped(&text, 10000))
    }
    fn assignment_context(&self, lead: i64, assignment: &Assignment) -> Result<String> {
        let attempt = self.attempt(lead)?;
        let latest: Option<String> = self.db.lock().unwrap().query_row(
            "SELECT summary FROM delivered_user_inputs WHERE attempt=?1 ORDER BY id DESC LIMIT 1",
            [lead], |row| row.get(0),
        ).optional()?;
        let mut context = format!("Project request: {}", clipped(&attempt.prompt, 3500));
        if let Some(direction) = latest {
            context.push_str(&format!("\nCurrent request: {}", clipped(&direction, 3500)));
        }
        let budget = 12000 / assignment.depends_on.len().max(1);
        for w in self
            .workers(lead)?
            .into_iter()
            .filter(|w| assignment.depends_on.contains(&w.assignment.key))
        {
            context.push_str(&format!(
                "\nPrerequisite {} [{}]: {}",
                w.assignment.key,
                w.state,
                clipped(&w.detail, budget.min(4000))
            ));
        }
        Ok(context)
    }
    pub(crate) fn reported_result(&self, run: i64) -> Result<Option<(String, String)>> {
        Ok(self.db.lock().unwrap().query_row(
            "SELECT kind,summary FROM events WHERE attempt=?1 AND kind IN ('worker/complete','worker/needs_changes') ORDER BY id DESC LIMIT 1",
            [run], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?)
    }
    pub fn current_checkpoint(&self, lead: i64) -> Result<Option<(String, String)>> {
        Ok(self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT decisions,next_task FROM checkpoints WHERE lead=?1",
                [lead],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }
    pub fn has_checkpoint(&self, lead: i64) -> Result<bool> {
        Ok(self
            .db
            .lock()
            .unwrap()
            .query_row("SELECT 1 FROM checkpoints WHERE lead=?1", [lead], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
            .is_some())
    }
}

pub(crate) fn tools(worker: bool) -> Value {
    let string = json!({"type":"string"});
    let strings = json!({"type":"array","items":string});
    let mut available = vec![
        crate::studio::packs::definition(),
        json!({"type":"function","name":"workshop_find_reports","description":"Search this project's saved decompilation history before requesting another analysis. Search by binary name, filter, symbol, entry point or input SHA-256. Each matching function lists its report byte offset, address and name. Empty query lists recent runs.","inputSchema":{"type":"object","properties":{"query":string,"before":{"type":"integer","description":"Optional next_before from the preceding page."}},"required":["query"],"additionalProperties":false}}),
        json!({"type":"function","name":"workshop_read_report","description":"Read saved pseudocode, the optional AI summary, the complete function index or a log by run ID. Use a function byte offset from search or next_offset for subsequent pages.","inputSchema":{"type":"object","properties":{"id":{"type":"integer"},"part":{"type":"string","enum":["report","functions","log","tool-log","summary"]},"offset":{"type":"integer","minimum":0}},"required":["id","part"],"additionalProperties":false}}),
    ];
    if worker {
        available.push(json!({"type":"function","name":"workshop_report_result","description":"Report your assignment outcome, verification and actionable findings to the lead and dependent workers. Use needs_changes for a blocker or failed acceptance review.","inputSchema":{"type":"object","properties":{"status":{"type":"string","enum":["complete","needs_changes"]},"summary":{"type":"string"}},"required":["status","summary"],"additionalProperties":false}}));
        return Value::Array(available);
    }
    let lead_tools = json!([
        {"type":"function","name":"workshop_generate_asset","description":"Lead-only specialist dispatch. Call this tool directly to create or revise a synthesized sound effect, image or editable 3D model and deliver it to this chat. For a sound, image or model, give it a short display title (e.g. Blue step stool) and a self-contained brief. Optionally select an allowed generation model and effort. For a revision use the returned source_asset ID, or reference_image for an image or sketch. Local alternatives: texture takes material and seed, starter takes template and brief (name), scene exports a Blender path or re-exports a prior source_asset. The result includes the saved asset ID. Import separately only when the project needs a copy.","inputSchema":{"type":"object","properties":{"kind":{"type":"string","enum":["image","model","sound","texture","starter","scene"]},"title":string,"brief":string,"style":string,"style_pack":{"type":"string","description":"For a model, use this project pack key as art direction and visual reference."},"style_asset":{"type":"string","description":"Optional member key in style_pack to use as the pictured example."},"model":string,"effort":string,"source_asset":{"type":"integer","minimum":1},"reference_image":{"type":"string","description":"Use chat for the latest marked-up image, sketch or prior image, or supply an attached or project image path."},"material":{"type":"string","enum":["Moss","Basalt","Sand"]},"seed":{"type":"integer","minimum":0,"maximum":4294967295u64},"template":{"type":"string","enum":["Crystal","Barrel","Planter"]},"path":string},"required":["kind"],"additionalProperties":false}},
        {"type":"function","name":"workshop_send_image","description":"Show an existing or locally rendered project image in chat. Use a path inside the current project copy and a short descriptive caption.","inputSchema":{"type":"object","properties":{"path":string,"caption":string},"required":["path","caption"],"additionalProperties":false}},
        {"type":"function","name":"workshop_find_assets","description":"Find saved images, sound effects and models in this project. Use the result ID to revise or import a finished asset.","inputSchema":{"type":"object","properties":{"area":{"type":"string","enum":["Images","Models","Sounds"]},"query":{"type":"string","description":"Part of the asset title, or empty for recent assets."}},"required":["area","query"],"additionalProperties":false}},
        {"type":"function","name":"workshop_import_asset","description":"Bring a saved WAV, PNG or GLB into this chat's isolated working copy at the requested project-relative path for review and acceptance.","inputSchema":{"type":"object","properties":{"id":{"type":"integer"},"destination":{"type":"string"}},"required":["id","destination"],"additionalProperties":false}},
        {"type":"function","name":"workshop_delegate","description":"Submit one plan of scoped Workshop workers. Independent tasks can run in parallel. Use depends_on to build chains, forks, or joins. A task starts after its prerequisites are integrated and checked and its edit scope is free. Reuse the same key to retry a blocked assignment or recover an interrupted batch.","inputSchema":{"type":"object","properties":{"tasks":{"type":"array","items":{"type":"object","properties":{"key":string,"title":string,"deliverable":string,"acceptance":string,"files":strings,"depends_on":strings,"model":string,"effort":string},"required":["key","title","deliverable","acceptance","files","depends_on"],"additionalProperties":false}}},"required":["tasks"],"additionalProperties":false}},
        {"type":"function","name":"workshop_resolve","description":"After you repair or complete a blocked worker's assignment yourself in the integration copy, verify the combined state and record the resolution. Supply your actual fix and verification.","inputSchema":{"type":"object","properties":{"key":string,"resolution":string},"required":["key","resolution"],"additionalProperties":false}},
        {"type":"function","name":"workshop_goal_result","description":"Report the outcome of the current chat goal. Use progress for a concrete new result that leaves work to do. Use complete only when the whole objective is satisfied and verified. Include the relevant evidence. Workshop checks the project before recording completion.","inputSchema":{"type":"object","properties":{"status":{"type":"string","enum":["progress","complete"]},"evidence":string},"required":["status","evidence"],"additionalProperties":false}},
        {"type":"function","name":"workshop_blocked","description":"Report a genuine external blocker, including a decision only the creator can make, when work cannot continue. State the specific input or capability you need and any saved progress. Workshop can notify the creator. This ends autonomous repair for this request.","inputSchema":{"type":"object","properties":{"reason":string},"required":["reason"],"additionalProperties":false}},
        {"type":"function","name":"workshop_checkpoint","description":"Save the design decisions and next unfinished task for this chat's review and project handoff.","inputSchema":{"type":"object","properties":{"decisions":string,"next_task":string},"required":["decisions","next_task"],"additionalProperties":false}}
    ]);
    available.extend(lead_tools.as_array().unwrap().iter().cloned());
    Value::Array(available)
}

impl Store {
    pub fn usage(&self, id: i64) -> Result<Option<i64>> {
        Ok(self
            .db
            .lock()
            .unwrap()
            .query_row("SELECT tokens FROM usage WHERE attempt=?1", [id], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub(crate) fn interrupt_workers(&self, lead: i64, cancelled: bool) -> Result<()> {
        self.controls.lock().unwrap().clear();
        self.cancel_preparing_directions(Some(lead), None)?;
        let reason = if cancelled {
            "Interrupted by the creator. Resume to retry this assignment."
        } else {
            "The lead stopped before this assignment finished. Resume to retry."
        };
        self.db.lock().unwrap().execute(
            "UPDATE workers SET state='blocked',detail=?2 WHERE lead=?1
             AND (state='running' OR (?3 AND state IN ('queued','checking')))",
            params![lead, reason, cancelled],
        )?;
        Ok(())
    }
    pub fn stop_worker(&self, worker: i64) -> Result<()> {
        self.controls
            .lock()
            .unwrap()
            .get(&worker)
            .context("Worker has already stopped")?
            .input
            .send(provider::Control::Cancel)
            .map_err(|_| anyhow::anyhow!("Worker has already stopped"))
    }
    pub fn steer_worker(&self, lead: i64, id: i64, message: provider::Message) -> Result<()> {
        ensure!(
            !message.text.trim().is_empty() || !message.files.is_empty(),
            "Enter a direction for this worker."
        );
        let mut controls = self.controls.lock().unwrap();
        let worker = self
            .workers(lead)?
            .into_iter()
            .find(|w| w.id == id && w.state == "running")
            .context("This worker has finished. Your direction was not sent.")?;
        let control = controls.get_mut(&id).context("This worker has stopped.")?;
        if let Some(preparing) = &mut control.preparing {
            let receipt = format!("Queued for {} · {}", worker.assignment.title, message.text);
            let mut db = self.db.lock().unwrap();
            let tx = db.transaction()?;
            let event = Self::insert_event(
                &tx,
                lead,
                "worker/direction",
                Some(&receipt),
                self.attached_chat_image(&message.files),
                Some(&message),
            )?;
            tx.execute(
                "INSERT INTO pending_worker_directions(event,worker) VALUES(?1,?2)",
                params![event, id],
            )?;
            tx.commit()?;
            preparing.push(message);
            return Ok(());
        }
        let receipt = format!("To {} · {}", worker.assignment.title, message.text);
        let image = self.attached_chat_image(&message.files).cloned();
        control
            .input
            .send(provider::Control::Steer(message))
            .map_err(|_| anyhow::anyhow!("This worker has stopped."))?;
        self.event_with_image(
            lead,
            "worker/direction",
            Some(&receipt),
            image.as_ref(),
            None,
        )
    }

    fn cancel_preparing_directions(&self, lead: Option<i64>, worker: Option<i64>) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute(
            "UPDATE events SET kind='worker/delivery_error',
                 summary='The worker stopped before delivery could be confirmed.' || char(10) || summary
             WHERE id IN (SELECT p.event FROM pending_worker_directions p JOIN workers w ON w.id=p.worker
                          WHERE (?1 IS NULL OR w.lead=?1) AND (?2 IS NULL OR w.id=?2))",
            params![lead,worker],
        )?;
        tx.execute(
            "DELETE FROM pending_worker_directions WHERE event IN
             (SELECT p.event FROM pending_worker_directions p JOIN events e ON e.id=p.event WHERE e.kind='worker/delivery_error')",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
    async fn run_assignment(
        &self,
        lead: i64,
        w: &Worker,
        policy: &crate::worker_policy::WorkerPolicy,
        mut controls: provider::Controls,
        notify: &dyn Fn(provider::ReplyUpdate<'_>),
    ) -> Result<()> {
        let lead_provider = self.attempt(lead)?.provider;
        let mut directions = String::new();
        let mut files = std::collections::BTreeSet::new();
        let (model, effort) = tokio::select! {
            biased;
            _ = controls.recv() => anyhow::bail!("Worker stopped during preparation"),
            result = self.worker_inference(policy, &w.assignment, &lead_provider) => result?,
        };
        let preparing = self
            .controls
            .lock()
            .unwrap()
            .get_mut(&w.id)
            .and_then(|control| control.preparing.take())
            .context("Worker stopped during preparation")?;
        for message in preparing {
            directions.push_str("\nCreator direction: ");
            directions.push_str(&message.text);
            files.extend(message.files);
        }
        let child = self.child_store(lead, w.id)?;
        let last = if let Some(run) = w.run {
            let previous = child.attempt(run)?;
            format!(
                "Previous attempt: {}. Integration feedback: {}. Prior worker summary: {}",
                previous.state,
                w.detail,
                clipped(&previous.action, 1800)
            )
        } else {
            String::new()
        };
        let s = &w.assignment;
        let prompt = format!(
            "{}\n\nDeliverable: {}\nAcceptance: {}\nAllowed edit paths: {:?}\nAn empty path list means read-only. Dependencies: {:?}.\n{}\n{}\n{}{directions}",
            s.title,
            s.deliverable,
            s.acceptance,
            s.files,
            s.depends_on,
            self.assignment_context(lead, s)?,
            self.handoff(lead)?,
            last
        );
        let files: Vec<_> = files.into_iter().collect();
        let a = child.prepare_inference(&prompt, &model, &effort, None, None, &files)?;
        self.db.lock().unwrap().execute(
            "UPDATE workers SET run=?2,model=?3,effort=?4,detail='Working in isolated copy' WHERE id=?1",
            params![w.id, a.id, a.model, a.effort],
        )?;
        notify(provider::ReplyUpdate::WorkersChanged);
        let update_error = std::cell::RefCell::new(None);
        let last_event = std::cell::Cell::new(0);
        let result = provider::worker_turn(&child, &a, controls, &|event| {
            let updated = match event {
                provider::ReplyUpdate::AccountUsage(..) => return notify(event),
                provider::ReplyUpdate::WorkerPromptSent => self
                    .db
                    .lock()
                    .unwrap()
                    .execute(
                        "DELETE FROM pending_worker_directions WHERE worker=?1",
                        [w.id],
                    )
                    .map(|_| ())
                    .map_err(Into::into),
                provider::ReplyUpdate::TranscriptChanged => (|| {
                    for message in child.messages_after(a.id, last_event.get())? {
                        last_event.set(message.id);
                        if message.delivery_notice {
                            self.event(
                                lead,
                                "worker/delivery_error",
                                Some(&format!("{} · {}", s.title, message.text)),
                            )?;
                            notify(provider::ReplyUpdate::TranscriptChanged);
                        }
                    }
                    Ok(())
                })(),
                provider::ReplyUpdate::Usage(tokens) => self
                    .db
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE workers SET tokens=?2 WHERE id=?1",
                        params![w.id, w.tokens.unwrap_or(0) + tokens],
                    )
                    .map(|_| ())
                    .map_err(Into::into),
                provider::ReplyUpdate::Activity(action) => {
                    self.worker_state(w.id, "running", action)
                }
                _ => return,
            };
            if let Err(e) = updated {
                update_error.replace(Some(e));
            }
            notify(provider::ReplyUpdate::WorkersChanged);
        })
        .await;
        if let Some(e) = update_error.into_inner() {
            return Err(e);
        }
        let report = result?;
        let a = child.attempt(a.id)?;
        for path in git::changed_paths(&child.work(a.id), &a.base, &a.candidate)? {
            ensure!(
                s.files.iter().any(|scope| covers(scope, &path)),
                "Worker edited {path} outside its assigned scope"
            );
        }
        self.worker_state(w.id, "review", report.as_deref().unwrap_or(&a.action))?;
        notify(provider::ReplyUpdate::WorkersChanged);
        Ok(())
    }
    async fn drive_worker(
        &self,
        lead: i64,
        w: Worker,
        policy: &crate::worker_policy::WorkerPolicy,
        controls: provider::Controls,
        notify: &dyn Fn(provider::ReplyUpdate<'_>),
    ) -> Result<()> {
        let result = self
            .run_assignment(lead, &w, policy, controls, notify)
            .await;
        self.controls.lock().unwrap().remove(&w.id);
        self.cancel_preparing_directions(Some(lead), Some(w.id))?;
        if let Err(e) = result {
            self.block_worker(w.id, &format!("{e:#}"))?;
            notify(provider::ReplyUpdate::WorkersChanged);
        }
        Ok(())
    }
    fn integrate(&self, lead: i64, w: &Worker) -> Result<()> {
        let run = w.run.context("Missing worker revision")?;
        let child = self.child_store(lead, w.id)?;
        if !child.attempt(run)?.patch.is_empty() {
            self.worker_state(w.id, "integrating", &w.detail)?;
            child.accept(run)?;
        }
        self.worker_state(w.id, "checking", &w.detail)
    }
    fn check_patch(&self, lead: &Attempt) -> Result<()> {
        let work = self.work(lead.id);
        let tree = git::working_tree(&work)?;
        git::git(&work, &["diff", "--check", &lead.base, &tree])?;
        Ok(())
    }
    async fn verify_combined(&self, lead: &Attempt, settings: &Settings) -> Result<()> {
        let work = self.work(lead.id);
        let tree = git::working_tree(&work)?;
        if self.verified(lead.id, &tree, settings)? {
            return Ok(());
        }
        if !settings.check_args.is_empty() {
            tokio::time::timeout(
                std::time::Duration::from_secs(300),
                self.run_check(lead.id, &settings.check_args, &tree),
            )
            .await
            .context("Project check timed out after five minutes")??;
            ensure!(
                git::working_tree(&work)? == tree,
                "The check modified the files being verified"
            );
        } else {
            git::git(&work, &["diff", "--check", &lead.base, &tree])?;
        }
        self.record_verification(lead.id, &tree, &settings.check_args)
    }
    pub(crate) fn record_verification(&self, id: i64, tree: &str, args: &[String]) -> Result<()> {
        self.db.lock().unwrap().execute(
            "INSERT INTO verified_team(lead,tree,check_args) VALUES(?1,?2,?3)
             ON CONFLICT(lead) DO UPDATE SET tree=excluded.tree,check_args=excluded.check_args",
            params![id, tree, serde_json::to_string(args)?],
        )?;
        Ok(())
    }
    pub(crate) fn has_open_blocker(&self, id: i64) -> Result<bool> {
        Ok(self.db.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM open_workshop_blockers WHERE attempt=?1)",
            [id],
            |row| row.get(0),
        )?)
    }
    pub fn candidate_ready(&self, a: &Attempt, workers: &[Worker]) -> Result<bool> {
        if workers.iter().any(|w| w.state != "finished") || self.has_open_blocker(a.id)? {
            return Ok(false);
        }
        let settings = self.team_settings()?;
        if workers.is_empty() {
            if a.patch.is_empty() {
                return Ok(true);
            }
            if settings.check_args.is_empty() {
                let repaired: bool = self.db.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM events WHERE attempt=?1 AND kind='workshop/repair')", [a.id], |r| r.get(0))?;
                if !repaired {
                    return Ok(true);
                }
            }
        }
        self.verified(a.id, &a.candidate, &settings)
    }
    pub(crate) fn verified(&self, id: i64, tree: &str, settings: &Settings) -> Result<bool> {
        Ok(self.db.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM verified_team WHERE lead=?1 AND tree=?2 AND check_args=?3)",params![id,tree,serde_json::to_string(&settings.check_args)?],|r|r.get(0))?)
    }
    pub(crate) async fn verify_final(&self, lead: &Attempt) -> Result<()> {
        let workers = self.workers(lead.id)?;
        let pending = workers
            .iter()
            .filter(|w| w.state != "finished")
            .map(|w| {
                format!(
                    "{} [{}]: {}",
                    w.assignment.key,
                    w.state,
                    clipped(&w.detail, 1200)
                )
            })
            .collect::<Vec<_>>();
        ensure!(
            pending.is_empty(),
            "Unfinished assignments:\n{}",
            clipped(&pending.join("\n"), 16000)
        );
        self.verify_combined(lead, &self.team_settings()?).await
    }
    async fn execute_queue(
        &self,
        lead: &Attempt,
        notify: &dyn Fn(provider::ReplyUpdate<'_>),
    ) -> Result<String> {
        let settings = self.team_settings()?;
        let policy = self.effective_worker_policy(lead.id)?;
        let first_event: i64 = self.db.lock().unwrap().query_row(
            "SELECT COALESCE(MAX(id),0) FROM events WHERE attempt=?1",
            [lead.id],
            |r| r.get(0),
        )?;
        let mut running = FuturesUnordered::new();
        loop {
            let workers = self.workers(lead.id)?;
            if !running.is_empty() && !workers.iter().any(|w| w.state == "queued") {
                // No more slots to fill. Check the final siblings together.
                running.next().await.transpose()?;
                continue;
            }
            for w in workers.iter().filter(|w| w.state == "review") {
                match self.integrate(lead.id, w) {
                    Ok(()) => {}
                    Err(e) => self.block_worker(w.id, &format!("{e:#}"))?,
                }
            }
            let checked: Vec<_> = self
                .workers(lead.id)?
                .into_iter()
                .filter(|w| w.state == "checking")
                .collect();
            if !checked.is_empty() {
                notify(provider::ReplyUpdate::WorkersChanged);
                // A downstream deliverable may be required by the project-wide test suite.
                // Release verified patch dependencies now and run the full check on the joined result.
                let result = if workers.iter().any(|w| w.state == "queued") {
                    self.check_patch(lead)
                } else {
                    self.verify_combined(lead, &settings).await
                };
                // An in-flight sibling may supply the other half of a combined check.
                if result.is_err() && !running.is_empty() {
                    running.next().await.transpose()?;
                    continue;
                }
                for w in checked {
                    if let Err(e) = &result {
                        self.block_worker(w.id, &format!("Combined check failed: {e:#}"))?;
                    } else {
                        self.worker_state(w.id, "finished", &w.detail)?;
                    }
                }
                notify(provider::ReplyUpdate::WorkersChanged);
            }
            let workers = self.workers(lead.id)?;
            let slots = settings.parallelism - running.len();
            for w in next_wave(&workers, slots) {
                self.db.lock().unwrap().execute(
                    "UPDATE workers SET state='running',tries=tries+1,run=NULL,detail='Preparing isolated copy' WHERE id=?1",
                    [w.id],
                )?;
                let (tx, rx) = provider::channel();
                self.controls.lock().unwrap().insert(
                    w.id,
                    WorkerControl {
                        input: tx,
                        preparing: Some(Vec::new()),
                    },
                );
                running.push(self.drive_worker(lead.id, w.clone(), &policy, rx, notify));
            }
            notify(provider::ReplyUpdate::WorkersChanged);
            if let Some(done) = running.next().await {
                done?;
            } else {
                // Keep waiting branches queued so fixing their owner unblocks them automatically.
                break;
            }
        }
        notify(provider::ReplyUpdate::WorkersChanged);
        let output = self
            .workers(lead.id)?
            .iter()
            .map(|w| {
                format!(
                    "{} [{}] {}: {}",
                    w.assignment.key,
                    w.state,
                    w.assignment.title,
                    clipped(&w.detail, 1200)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let directions: String = self.db.lock().unwrap().query_row(
            "SELECT COALESCE(group_concat(summary,char(10)),'') FROM
             (SELECT summary FROM events WHERE attempt=?1 AND id>?2
              AND kind IN ('worker/direction','worker/delivery_error') ORDER BY id)",
            params![lead.id, first_event],
            |row| row.get(0),
        )?;
        let output = if directions.is_empty() {
            output
        } else {
            format!("{output}\nWorker directions from the creator:\n{directions}")
        };
        let output = clipped(&output, 24000);
        ensure!(
            self.workers(lead.id)?.iter().all(|w| w.state == "finished"),
            "Worker feedback:\n{output}\nRetry the owning key. After two identical failures or four runs, repair directly and use workshop_resolve or report the blocker."
        );
        Ok(output)
    }
}

#[derive(Debug)]
pub(crate) struct ToolOutput {
    pub text: String,
    pub image: Option<Vec<u8>>,
}

impl ToolOutput {
    pub fn text(text: String) -> Self {
        Self { text, image: None }
    }

    pub fn mcp(result: Result<Self>) -> Value {
        let error = result.is_err();
        let output = result.unwrap_or_else(|e| Self::text(format!("{e:#}")));
        let mut content = vec![json!({"type":"text","text":output.text})];
        if let Some(bytes) = output.image {
            content
                .push(json!({"type":"image","mimeType":"image/png","data":STANDARD.encode(bytes)}));
        }
        json!({"isError":error,"content":content})
    }

    pub fn codex(result: Result<Self>) -> Value {
        let success = result.is_ok();
        let output = result.unwrap_or_else(|e| Self::text(format!("{e:#}")));
        let mut items = vec![json!({"type":"inputText","text":output.text})];
        if let Some(bytes) = output.image {
            items.push(json!({"type":"inputImage","imageUrl":format!("data:image/png;base64,{}",STANDARD.encode(bytes))}));
        }
        json!({"success":success,"contentItems":items})
    }
}

pub(crate) async fn dispatch_tool(
    store: &Store,
    lead: &Attempt,
    request: &Value,
    notify: &dyn Fn(provider::ReplyUpdate<'_>),
) -> Result<ToolOutput> {
    if request["tool"] == "workshop_asset_pack" {
        return crate::studio::packs::dispatch(store, lead, &request["arguments"], notify).await;
    }
    dispatch(store, lead, request, notify)
        .await
        .map(ToolOutput::text)
}

async fn dispatch(
    store: &Store,
    lead: &Attempt,
    request: &Value,
    notify: &dyn Fn(provider::ReplyUpdate<'_>),
) -> Result<String> {
    let args = &request["arguments"];
    match request["tool"].as_str() {
        Some("workshop_generate_asset") if !store.worker => {
            let request = serde_json::from_value(args.clone()).context("Invalid asset request")?;
            return store.generate_chat_asset(lead, request, notify).await;
        }
        Some("workshop_send_image") if !store.worker => {
            let work = dunce::canonicalize(store.work(lead.id))?;
            let path = args["path"].as_str().context("Choose a project image.")?;
            let path = std::path::Path::new(path);
            let path = path.strip_prefix(&work).unwrap_or(path);
            ensure!(
                path.components().all(|part| matches!(
                    part,
                    std::path::Component::CurDir | std::path::Component::Normal(_)
                )),
                "Choose an image in the current project copy."
            );
            let path =
                dunce::canonicalize(work.join(path)).context("Cannot find the project image.")?;
            ensure!(
                path.starts_with(&work) && path.is_file(),
                "Choose an image in the current project copy."
            );
            let caption = args["caption"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .context("Describe the image.")?;
            let image = image::ImageReader::open(path)?
                .with_guessed_format()?
                .decode()?;
            store.post_chat_image(lead.id, &image, &clipped(caption, 500))?;
            notify(provider::ReplyUpdate::TranscriptChanged);
            return Ok("Image added to the chat.".into());
        }
        Some("workshop_report_result") if store.worker => {
            let kind = match args["status"].as_str() {
                Some("complete") => "worker/complete",
                Some("needs_changes") => "worker/needs_changes",
                _ => anyhow::bail!("Choose complete or needs_changes."),
            };
            let summary = args["summary"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .context("Describe the result and verification.")?;
            store.event(lead.id, kind, Some(&clipped(summary, 4000)))?;
            return Ok("Assignment result recorded.".into());
        }
        Some("workshop_find_reports") => {
            return Ok(serde_json::to_string(&store.report_archive()?.search(
                args["query"].as_str().context("Provide a search query.")?,
                args["before"].as_i64(),
            )?)?);
        }
        Some("workshop_read_report") => {
            let offset = match args.get("offset") {
                Some(v) => v.as_u64().context("Choose a nonnegative byte offset.")?,
                None => 0,
            };
            return Ok(serde_json::to_string(&store.report_archive()?.read(
                args["id"].as_i64().context("Provide a run ID.")?,
                args["part"].as_str().context("Choose report or log.")?,
                offset,
            )?)?);
        }
        _ => ensure!(!store.worker, "This worker can only read archived reports."),
    }
    match request["tool"].as_str() {
        Some("workshop_find_assets") => {
            let area = args["area"]
                .as_str()
                .context("Select Images, Models or Sounds")?;
            ensure!(
                matches!(area, "Images" | "Models" | "Sounds"),
                "Select Images, Models or Sounds"
            );
            let jobs = store.find_studio_jobs(
                area,
                args["query"].as_str().context("Provide a title filter")?,
                true,
                Some(12),
            )?;
            let assets: Vec<_> = jobs.iter().map(|job| json!({"id":job.id,"title":job.title,"format":job.detail,"file":job.filename()})).collect();
            Ok(serde_json::to_string(&assets)?)
        }
        Some("workshop_import_asset") => {
            let id = args["id"].as_i64().context("Choose an asset ID")?;
            let destination = args["destination"]
                .as_str()
                .context("Choose an asset destination")?;
            store.import_studio_asset(lead.id, id, destination)?;
            Ok(format!("Imported {destination} in the review copy."))
        }
        Some("workshop_delegate") => {
            let assignments: Vec<Assignment> = serde_json::from_value(args["tasks"].clone())?;
            store.queue(lead.id, &assignments)?;
            notify(provider::ReplyUpdate::WorkersChanged);
            store.execute_queue(lead, notify).await
        }
        Some("workshop_resolve") => {
            let key = args["key"].as_str().context("Missing task key")?;
            let detail = args["resolution"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .context("Describe the resolution")?;
            let w = store
                .workers(lead.id)?
                .into_iter()
                .find(|w| w.assignment.key == key && w.state == "blocked")
                .context("Select a blocked worker")?;
            if store.workers(lead.id)?.iter().any(|w| w.state == "queued") {
                store.check_patch(lead)?;
            } else {
                store.verify_combined(lead, &store.team_settings()?).await?;
            }
            store.worker_state(w.id, "finished", detail)?;
            notify(provider::ReplyUpdate::WorkersChanged);
            store.execute_queue(lead, notify).await
        }
        Some("workshop_goal_result") => {
            let status = args["status"].as_str().context("Choose a goal status.")?;
            let evidence = args["evidence"]
                .as_str()
                .context("Describe the evidence.")?;
            store.report_goal(lead.id, status, &clipped(evidence, 4000))?;
            Ok(
                "Goal report received. Finish this response so Workshop can verify the result."
                    .into(),
            )
        }
        Some("workshop_blocked") => {
            let reason = args["reason"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .context("Explain the external blocker.")?;
            store.event(lead.id, "workshop/blocker", Some(&clipped(reason, 4000)))?;
            Ok("Blocker recorded. Describe what is needed to continue.".into())
        }
        Some("workshop_checkpoint") => {
            store.checkpoint(
                lead.id,
                args["decisions"].as_str().context("Missing decisions")?,
                args["next_task"].as_str().context("Missing next task")?,
            )?;
            Ok("Checkpoint saved for review.".into())
        }
        _ => anyhow::bail!("Unknown Workshop tool"),
    }
}
#[cfg(test)]
mod tests;
