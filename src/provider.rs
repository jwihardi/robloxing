use crate::{
    Attempt, Store, team,
    usage::{ContextUsage, UsageWindow, account_windows},
};
pub mod antigravity;
mod art;
pub mod blender_mcp;
mod catalog;
mod claude;
pub mod gemini;
pub mod plugins;

pub fn provider_for(model: &str) -> &'static str {
    if model.starts_with("claude:") {
        "claude"
    } else if model.starts_with("antigravity:") {
        "antigravity"
    } else if model.starts_with("gemini:") {
        "gemini"
    } else {
        "codex"
    }
}
use anyhow::{Context, Result, bail, ensure};
pub(crate) use art::{design_model, design_sound, generate_image, summarize_report};
pub(crate) use catalog::worker_catalog;
pub use catalog::{
    Catalog, CatalogSources, Model as CatalogModel, discover_models, load_models_and_usage,
    model_catalog,
};

pub type AccountLimits = std::collections::HashMap<String, Vec<UsageWindow>>;

pub async fn account_limits(provider: &str, project: &Path) -> Result<AccountLimits> {
    tokio::time::timeout(Duration::from_secs(12), async {
        let windows = match provider {
            "codex" => catalog::codex_limits(project).await?,
            "claude" => claude::account_limits().await?,
            "antigravity" => return antigravity::account_limits_with_default(project).await,
            _ => return Ok(AccountLimits::new()),
        };
        Ok(AccountLimits::from([(provider.to_owned(), windows)]))
    })
    .await
    .context("Account usage lookup timed out")?
}
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
    sync::mpsc,
    time::Instant,
};

pub enum Control {
    Steer(Message),
    Queue(Message),
    SteerWorker(i64, String),
    Cancel,
    StopWorker(i64),
}
pub type Controls = mpsc::UnboundedReceiver<Control>;
pub fn codex_program() -> String {
    std::env::var("WORKSHOP_CODEX").unwrap_or_else(|_| "codex".into())
}
pub fn channel() -> (mpsc::UnboundedSender<Control>, Controls) {
    mpsc::unbounded_channel()
}

pub fn managed(
    program: &str,
    args: &[&str],
    cwd: &Path,
    stdout: Stdio,
    stderr: File,
) -> Result<Box<dyn ChildWrapper>> {
    spawn_managed(
        CommandWrap::with_new(program, |cmd| {
            cmd.args(args)
                .current_dir(cwd)
                .stdin(Stdio::piped())
                .stdout(stdout)
                .stderr(stderr);
        }),
        program,
    )
}
fn spawn_managed(mut cmd: CommandWrap, program: &str) -> Result<Box<dyn ChildWrapper>> {
    crate::git::clear_repository_env(cmd.command_mut().as_std_mut());
    cmd.wrap(KillOnDrop);
    #[cfg(windows)]
    {
        use process_wrap::tokio::{CreationFlags, JobObject};
        let mut flags = CreationFlags(Default::default());
        flags.0.0 = 0x08000000; // CREATE_NO_WINDOW
        cmd.wrap(flags).wrap(JobObject);
    }
    cmd.spawn()
        .with_context(|| format!("Start {program}. Check that it is installed and on PATH."))
}
fn append(path: &Path) -> Result<File> {
    Ok(OpenOptions::new().create(true).append(true).open(path)?)
}
async fn send(stdin: &mut ChildStdin, v: Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&v)?;
    bytes.push(b'\n');
    stdin.write_all(&bytes).await?;
    stdin.flush().await?;
    Ok(())
}
pub struct Message {
    pub text: String,
    pub files: Vec<PathBuf>,
    pub(crate) queued_id: Option<i64>,
    pub(crate) followup: bool,
}
impl From<&str> for Message {
    fn from(text: &str) -> Self {
        Self {
            text: text.into(),
            files: Vec::new(),
            queued_id: None,
            followup: false,
        }
    }
}
impl Message {
    fn record_direction(
        &self,
        store: &Store,
        chat: i64,
        notify: &dyn Fn(ReplyUpdate<'_>),
    ) -> Result<()> {
        if self.queued_id.is_some() {
            store.message_event(chat, "steer/sent", self)?;
            notify(ReplyUpdate::TranscriptChanged);
        }
        Ok(())
    }

    fn record_followup(
        &self,
        store: &Store,
        a: &Attempt,
        notify: &dyn Fn(ReplyUpdate<'_>),
    ) -> Result<()> {
        if store.worker {
            notify(ReplyUpdate::WorkerPromptSent);
        }
        if !a.session.is_empty() || self.followup {
            store.message_event(a.id, "user/followup", self)?;
            notify(ReplyUpdate::TranscriptChanged);
        }
        Ok(())
    }
    pub fn compose(text: &str, files: Vec<PathBuf>) -> Self {
        let mut text = text.to_owned();
        if !files.is_empty() {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str("Attachments");
            for path in &files {
                text.push('\n');
                text.push_str(&path.to_string_lossy());
            }
        }
        Self {
            text,
            files,
            queued_id: None,
            followup: false,
        }
    }
    fn input(&self) -> Result<Value> {
        let mut items = vec![json!({"type":"text","text":self.text})];
        // Larger files, binary documents and directories stay available by local path.
        let mut text_budget = 128 * 1024u64;
        for path in &self.files {
            let meta = path
                .metadata()
                .with_context(|| format!("Cannot access attachment {}", path.display()))?;
            if meta.is_dir() {
                continue;
            }
            ensure!(
                meta.is_file(),
                "Choose a regular file or folder: {}",
                path.display()
            );
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp") {
                items.push(json!({"type":"localImage","path":path}));
            } else if meta.len() <= text_budget && text_budget > 0 {
                let mut bytes = Vec::new();
                File::open(path)?
                    .take(text_budget + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() as u64 <= text_budget
                    && !bytes.contains(&0)
                    && let Ok(contents) = String::from_utf8(bytes)
                {
                    text_budget -= contents.len() as u64;
                    items.push(json!({"type":"text","text":format!("Contents of {}\n{}",path.display(),contents)}));
                }
            }
        }
        Ok(Value::Array(items))
    }
}
fn tool_activity(name: &str) -> &str {
    match name.rsplit("__").next().unwrap_or(name) {
        "generate_image" | "image_generation" | "imagegen" => "Generating an image…",
        "workshop_delegate" => "Dispatching subagents…",
        "workshop_generate_asset" => "Preparing the asset…",
        "workshop_asset_pack" => "Working with the Blender asset library…",
        "Bash" | "PowerShell" => "Running a command…",
        "WebSearch" | "web_search" => "Searching the web…",
        "Read" | "read_file" => "Reading a file…",
        "Edit" | "Write" | "write_file" => "Editing project files…",
        _ => name,
    }
}
fn label(v: &Value) -> Option<String> {
    let p = &v["params"];
    let s = match v["method"].as_str()? {
        "item/started" => match p["item"]["type"].as_str()? {
            "commandExecution" => p["item"]["command"].as_str().unwrap_or("Running a command"),
            "fileChange" => "Editing project files",
            "reasoning" => "Thinking…",
            "imageGeneration" => "Generating an image…",
            "webSearch" => "Searching the web…",
            "mcpToolCall" | "dynamicToolCall" => {
                p["item"]["tool"].as_str().unwrap_or("Using a tool")
            }
            _ => return None,
        },
        "item/completed" if p["item"]["type"] == "agentMessage" => {
            return p["item"]["text"].as_str().map(str::to_owned);
        }
        "turn/started" => "Working in the isolated copy",
        "error" => p["error"]["message"].as_str()?,
        _ => return None,
    };
    Some(s.chars().take(500).collect())
}

fn instructions(store: &Store, a: &Attempt) -> Result<String> {
    let mut instructions = if !store.worker {
        let context = store.handoff(a.id)?;
        let limit = store.team_settings()?.parallelism;
        format!(
            "You are the lead agent in Workshop. Work inside this isolated project. Follow the project’s delegation setting attached to the current request. When the creator specifically requests workers in the task itself, call workshop_delegate even for simple work. Honor the requested number of assignments when feasible. Report only the workers actually dispatched and their returned outcomes. If delegation fails, explain what happened. Answer greetings and small, indivisible requests directly. When automatic delegation is enabled, or the creator asks for workers, manage the task from intake through verification without waiting for the user to divide it up or request workers. Inspect just enough to decide scope, then dispatch independent deliverables together. You own the acceptance criteria, interfaces and integrated result. Use a sequential or mixed plan where one assignment needs another's output. Include a scoped test or read-only acceptance review when it adds independent coverage. Honor explicit limits and requests to work alone. The project allows up to {limit} concurrent workers. Use the provider prefixes claude:, antigravity: and gemini: for models served by those CLIs. Use Workshop tools for all delegation so workers appear in the chat. Omit the model to use the configured worker default. Choose a suitable allowed model and effort for each assignment or omit them for defaults. Give each worker a stable key, a short title, exact relative edit paths ([] for read-only), a concrete deliverable and observable acceptance criteria. Use depends_on for prerequisites. You can dispatch additional work as findings arrive. When a worker or check finds a problem, inspect the evidence, repair it or retry its owning key, and continue pending work. Respect a user-stopped worker. Inspect the integrated result against the original request and run relevant behavioral checks using the project’s own tools before finishing. A clean diff alone does not establish functional correctness. Report what passed. If your local tools fail, use the available worker checkouts when feasible without asking for delegation permission. When no authorized route remains, call workshop_blocked with the external blocker before ending. Use workshop_checkpoint for lasting decisions or unfinished work. Project handoff:\n{context}"
        )
    } else {
        String::from(
            "You are a Workshop worker in a private copy. Complete only the assigned deliverable within the declared edit scope, verify the acceptance criteria, and report concrete evidence and any remaining gaps. Use workshop_report_result with needs_changes when the criteria are unmet or a review finds an actionable defect. For a successful handoff, give downstream workers the relevant interfaces, findings and checks. The lead integrates your edits. Do not delegate.",
        )
    };
    if !store.worker {
        instructions.push_str(" For an explicit request to inspect or edit the connected Blender scene, use its Blender MCP tools when available. When the creator asks for a new or revised asset in this chat, call workshop_generate_asset. Use kind=sound for synthesized game audio. You choose the generator, give it a complete brief, and can select the model and effort within the creator's settings. Omit those fields to route automatically. Use reference_image=chat when revising a posted image or sketch, or source_asset to revise a saved design. Only the lead can call this tool. For a standalone asset, call it directly as the specialist assignment, including when automatic project delegation is enabled. The tool verifies and delivers the finished file. Use its result as evidence of delivery. Delegate additional project work when the request calls for it. Continue the conversation here. Use workshop_send_image for an existing project image. Use workshop_import_asset when the request also needs the file in the project.");
    }
    instructions.push_str(" For questions about compiled game modules, check workshop_find_reports first and use workshop_read_report to inspect any saved result. Report partial coverage and the run ID when relevant.");
    instructions.push_str(" For Blender art packs or a matching project prop, use workshop_asset_pack. Inspect the pack's art direction and individual preview images, reuse an existing member when suitable, and publish finished members when asked to build a pack. Pass style_pack when generating a matching model. The library is shared by project chats and workers.");
    Ok(instructions)
}

async fn open_thread(
    stdin: &mut ChildStdin,
    store: &Store,
    a: &Attempt,
    legacy_assets: Option<&gemini::Bridge>,
) -> Result<()> {
    let mut params =
        json!({"cwd":store.work(a.id),"approvalPolicy":"never","sandbox":"danger-full-access"});
    params["config"] = json!({"features.multi_agent":false, "features.image_generation":false});
    if let Some(server) = blender_mcp::configured(&store.preferences_home)? {
        params["config"]["mcp_servers.blender"] = server.codex();
    }
    let mut instructions = instructions(store, a)?;
    if let Some(bridge) = legacy_assets {
        params["config"]["mcp_servers.workshop_assets"] = bridge.codex_config()?;
        instructions.push_str(" In this resumed chat use mcp__workshop_assets__workshop_asset_pack for the Blender library.");
        if !store.worker {
            instructions.push_str(
                " Use mcp__workshop_assets__workshop_generate_asset for style-matched models.",
            );
        }
    }
    if a.session.is_empty() {
        params["dynamicTools"] = team::tools(store.worker);
    } else if let Some(cli) = std::env::current_exe()
        .ok()
        .and_then(|p| {
            p.parent()
                .map(|d| d.join(format!("workshop-cli{}", std::env::consts::EXE_SUFFIX)))
        })
        .filter(|p| p.is_file())
        && let Ok(project) = store.report_archive().and_then(|archive| archive.project())
    {
        instructions.push_str(&format!(" If those tools are unavailable, use the read-only Workshop CLI at {}. Supply the project path {project} via --project for both studio reports QUERY and studio read-report ID --offset BYTE.",cli.display()));
    }
    params["developerInstructions"] = json!(instructions);
    if !a.model.is_empty() {
        params["model"] = json!(a.model);
    }
    let method = if a.session.is_empty() {
        "thread/start"
    } else {
        params["threadId"] = json!(a.session);
        params["excludeTurns"] = json!(true);
        "thread/resume"
    };
    send(stdin, json!({"id":2,"method":method,"params":params})).await
}

async fn start_turn(
    stdin: &mut ChildStdin,
    session: &str,
    prompt: &Message,
    a: &Attempt,
    team_preference: Option<&str>,
) -> Result<()> {
    let mut input = prompt.input()?;
    if let Some(preference) = team_preference {
        input
            .as_array_mut()
            .context("Expected turn input items")?
            .push(json!({"type":"text","text":preference}));
    }
    let mut params = json!({"threadId":session,"input":input});
    if !a.model.is_empty() {
        params["model"] = json!(a.model);
    }
    if !a.effort.is_empty() {
        params["effort"] = json!(a.effort);
    }
    send(stdin, json!({"id":3,"method":"turn/start","params":params})).await
}

// Keep partial frames across select! cancellation while bounding a malformed provider line.
struct Frames {
    reader: BufReader<ChildStdout>,
    pending: Vec<u8>,
}
impl Frames {
    async fn next_line(&mut self) -> Result<Option<String>> {
        self.next_line_with_limit(16 * 1024 * 1024).await
    }
    async fn next_line_with_limit(&mut self, limit: usize) -> Result<Option<String>> {
        loop {
            let bytes = self.reader.fill_buf().await?;
            if bytes.is_empty() {
                ensure!(
                    self.pending.is_empty(),
                    "Provider closed in the middle of an event"
                );
                return Ok(None);
            }
            let end = bytes.iter().position(|b| *b == b'\n');
            let count = end.unwrap_or(bytes.len());
            ensure!(
                self.pending.len() + count <= limit,
                "Provider response exceeded the {} MiB limit",
                limit / (1024 * 1024)
            );
            self.pending.extend_from_slice(&bytes[..count]);
            self.reader.consume(count + usize::from(end.is_some()));
            if end.is_some() {
                return Ok(Some(String::from_utf8(std::mem::take(&mut self.pending))?));
            }
        }
    }
}

struct Connection {
    child: Box<dyn ChildWrapper>,
    used_tools: bool,
    stdin: ChildStdin,
    lines: Frames,
    cwd: PathBuf,
    pinned_inference: Option<(String, String)>,
    gemini_bridge: Option<gemini::Bridge>,
    session: String,
}

pub struct Session {
    io: Connection,
    runtime: tokio::runtime::Runtime,
}

impl Session {
    fn open(store: &Store, a: &Attempt) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let io = {
            let _entered = runtime.enter();
            Connection::open(store, a)?
        };
        Ok(Self { io, runtime })
    }
}

impl Connection {
    fn open(store: &Store, a: &Attempt) -> Result<Self> {
        let stderr = append(&store.folder(a.id).join("provider.stderr.log"))?;
        if a.provider == "claude" {
            return claude::connection(store, a, stderr);
        }
        if matches!(a.provider.as_str(), "gemini" | "antigravity") {
            return gemini::connection(store, a, stderr);
        }
        let mut io = Self::spawn(store.work(a.id), stderr)?;
        // Codex keeps the original dynamic tool list on resume. Older chats receive
        // the missing capability via the same local MCP transport used by ACP.
        if !a.session.is_empty()
            && !store.db.lock().unwrap().query_row(
                "SELECT EXISTS(SELECT 1 FROM codex_asset_sessions WHERE session=?1 AND pack_tools=1)",
                [&a.session],
                |r| r.get::<_, bool>(0),
            )?
        {
            io.gemini_bridge = Some(gemini::Bridge::legacy_assets(store.worker)?);
        }
        Ok(io)
    }
    fn spawn(cwd: PathBuf, stderr: File) -> Result<Self> {
        let exe = codex_program();

        let cmd = CommandWrap::with_new(&exe, |cmd| {
            cmd.args(["app-server", "--listen", "stdio://"])
                .current_dir(&cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(stderr);
            // The new standalone server has its own sandbox runner. A Codex Desktop
            // launcher's registered Core identity belongs only to that parent process.
            #[cfg(windows)]
            cmd.env_remove("CODEX_WINDOWS_REGISTERED_CORE");
        });
        let mut child = spawn_managed(cmd, &exe)?;
        let stdin = child.stdin().take().context("Provider stdin")?;
        let stdout = child.stdout().take().context("Provider stdout")?;
        Ok(Connection {
            child,
            used_tools: false,
            stdin,
            lines: Frames {
                reader: BufReader::new(stdout),
                pending: Vec::new(),
            },
            cwd,
            pinned_inference: None,
            gemini_bridge: None,
            session: String::new(),
        })
    }
}

pub enum ReplyUpdate<'a> {
    Delta(&'a str),
    Stored,
    TranscriptChanged,
    WorkersChanged,
    WorkerPromptSent,
    WorkerInference(&'a str, &'a str),
    Usage(i64),
    Context(ContextUsage),
    AccountUsage(&'static str, Vec<UsageWindow>),
    Activity(&'a str),
}

fn team_preference(store: &Store, id: i64) -> Result<Option<String>> {
    use crate::provider_permissions::{PROVIDERS, ProviderPermissions, TaskRole};
    if store.worker {
        return Ok(None);
    }
    let permissions = ProviderPermissions::load(&store.preferences_home)?;
    let enabled: Vec<_> = PROVIDERS
        .into_iter()
        .filter(|p| permissions.allows(p, TaskRole::Workers))
        .collect();
    let mut message = if enabled.is_empty() {
        "[Workshop provider settings] The creator has disabled all worker providers. Work directly. If the creator asks for delegation, explain that a worker provider must first be enabled in Settings → Providers."
    } else if store.team_settings()?.auto_delegate {
        "[Workshop project setting: automatic delegation] Use subagents for independent project deliverables in this request. Assign the separate owners with workshop_delegate before implementing their parts yourself. Manage dependencies, fixes and final verification. Answer conversation and small indivisible tasks directly. The creator’s explicit directions about workers take precedence."
    } else {
        "[Workshop project setting: delegation by request] Work directly unless the creator explicitly asks for subagents in this request."
    }.to_owned();
    if !enabled.is_empty() && enabled.len() < PROVIDERS.len() {
        message.push_str(&format!(
            " Enabled worker providers: {}. Omit the model to use an allowed default.",
            enabled.join(", ")
        ));
    }
    if !enabled.is_empty() {
        let general = store.effective_worker_policy(id)?;
        let design = crate::studio::settings::TaskSettings::defaults(
            &store.preferences_home,
            "LevelDesign",
        )?;
        message.push_str(&format!(" Worker limits: {}. Additional level design limits: {}. Empty models means any enabled model, and empty max_effort means uncapped. For level layouts, traversal, blockouts and encounter pacing, set the assignment specialty to level_design when available and name the design work in its title. Omit model and effort for automatic selection. Workshop prefers eligible Claude models for level design and uses low effort for routine workers or medium for design when supported. Request more effort only when the assignment warrants it.", serde_json::to_string(&general)?, serde_json::to_string(&design.agent)?));
    }
    message.push_str(" To create a new standalone image, sound effect or 3D model, call workshop_generate_asset directly from this lead session. To reuse an existing Blender pack member, use workshop_asset_pack. It assigns the specialist and delivers the file. Project workers handle any additional project deliverables.");
    for role in [TaskRole::Images, TaskRole::Models, TaskRole::Sounds] {
        if role == TaskRole::Sounds
            && crate::studio::settings::TaskSettings::defaults(&store.preferences_home, "Sounds")?
                .elevenlabs
        {
            message.push_str(" Sound effects use ElevenLabs SFX v3. Omit model and effort when calling workshop_generate_asset for sound.");
            continue;
        }
        let providers: Vec<_> = PROVIDERS
            .into_iter()
            .filter(|p| permissions.allows(p, role))
            .collect();
        let choices = if providers.is_empty() {
            "none".into()
        } else {
            providers.join(", ")
        };
        message.push_str(&format!(" {} providers: {choices}.", role.label()));
    }
    if let Some(goal) = store.chat_goal(id)?.filter(|g| g.status == "active") {
        message.push_str(&format!(
            "\n[Workshop chat goal]\n{}\nKeep working toward this objective across turns. {}",
            goal.objective,
            crate::goals::reporting(false)
        ));
    }
    Ok(Some(message))
}

fn retries_stopped(
    tool: Option<&str>,
    args: &Value,
    stopped_keys: &std::collections::HashSet<String>,
) -> bool {
    match tool {
        Some("workshop_delegate") => args["tasks"].as_array().is_some_and(|tasks| {
            tasks
                .iter()
                .any(|t| t["key"].as_str().is_some_and(|k| stopped_keys.contains(k)))
        }),
        Some("workshop_resolve") => args["key"]
            .as_str()
            .is_some_and(|k| stopped_keys.contains(k)),
        _ => false,
    }
}

enum Finish {
    Done,
    Cancelled,
    Continue(Message),
}
#[derive(Default)]
struct Repairs {
    count: u8,
    previous: Option<(String, String)>,
    goal: Option<crate::goals::GoalRun>,
}
impl Repairs {
    fn start(store: &Store, a: &Attempt) -> Result<Self> {
        Ok(Self {
            goal: crate::goals::GoalRun::start(store, a.id)?,
            ..Default::default()
        })
    }

    fn advance_goal(
        &mut self,
        store: &Store,
        id: i64,
        notify: &dyn Fn(ReplyUpdate<'_>),
    ) -> Result<Finish> {
        let Some(goal) = &mut self.goal else {
            return Ok(Finish::Done);
        };
        let next = goal.advance(store, id)?;
        notify(ReplyUpdate::TranscriptChanged);
        Ok(match next {
            crate::goals::Next::Stop => Finish::Done,
            crate::goals::Next::Continue(direction) => {
                notify(ReplyUpdate::Activity("Continuing the goal…"));
                Finish::Continue(direction.as_str().into())
            }
        })
    }

    fn reset_goal_reports(&mut self, store: &Store, id: i64) -> Result<()> {
        if let Some(goal) = &mut self.goal {
            goal.reset_reports(store, id)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn verify(
        &mut self,
        store: &Store,
        a: &Attempt,
        controls: &mut Controls,
        notify: &dyn Fn(ReplyUpdate<'_>),
        used_tools: bool,
        has_workers: bool,
        stopped: bool,
        unavailable: Option<anyhow::Error>,
    ) -> Result<Finish> {
        if store.worker {
            return unavailable.map_or(Ok(Finish::Done), Err);
        }
        if !used_tools
            && !has_workers
            && self.goal.is_none()
            && unavailable.is_none()
            && !store.has_open_blocker(a.id)?
        {
            return Ok(Finish::Done);
        }
        let verified = if let Some(error) = unavailable {
            Err(error)
        } else {
            let check = store.verify_final(a);
            tokio::pin!(check);
            loop {
                tokio::select! {
                    biased;
                    input = controls.recv() => match input {
                        None | Some(Control::Cancel) => return Ok(Finish::Cancelled),
                        Some(Control::Steer(message)) => {
                            let message = store.stage_direction(a.id, message)?;
                            self.reset_goal_reports(store, a.id)?;
                            return Ok(Finish::Continue(message));
                        }
                        Some(Control::Queue(message)) => {
                            store.queue_message(a.id, &message)?;
                            notify(ReplyUpdate::TranscriptChanged);
                        }
                        Some(Control::SteerWorker(id,text)) => route_worker_direction(store,a.id,id,&text,notify)?,
                        Some(Control::StopWorker(_)) => {}
                    },
                    result = &mut check => break result,
                }
            }
        };
        let Err(error) = verified else {
            if store.has_open_blocker(a.id)? {
                store.event(a.id, "workshop/blocker_resolved", None)?;
            }
            return self.advance_goal(store, a.id, notify);
        };
        self.reset_goal_reports(store, a.id)?;
        if stopped {
            return Err(error.context("A worker was stopped by the user."));
        }
        let feedback = format!("{error:#}");
        let tree = crate::git::working_tree(&store.work(a.id))?;
        let fingerprint = (tree, feedback.clone());
        if self.count >= 2 || self.previous.as_ref() == Some(&fingerprint) {
            return Err(error.context("Automatic repair stopped. Review the remaining blocker."));
        }
        self.previous = Some(fingerprint);
        self.count += 1;
        let repairs = self.count;
        let notice = format!(
            "Verification needs a repair (pass {repairs} of 2). {}",
            feedback.chars().take(1200).collect::<String>()
        );
        store.event(a.id, "workshop/repair", Some(&notice))?;
        notify(ReplyUpdate::TranscriptChanged);
        notify(ReplyUpdate::Activity(&notice));
        let direction = format!(
            "Workshop verification found unfinished work in the current request. Continue in this same isolated project and fix the cause. Retry the original owning keys, keep completed work, and verify the integrated result. If a worker exhausted its retries, inspect and repair directly before workshop_resolve. Report anything you genuinely cannot resolve.\n\n{feedback}"
        );
        Ok(Finish::Continue(direction.as_str().into()))
    }
}

struct TurnResult {
    finished: bool,
    reuse_connection: bool,
}

async fn exchange(
    io: &mut Connection,
    store: &Store,
    a: &Attempt,
    prompt: &Message,
    controls: &mut Controls,
    stream: bool,
    notify: &dyn Fn(ReplyUpdate<'_>),
) -> Result<TurnResult> {
    let result = match a.provider.as_str() {
        "claude" => claude::exchange(io, store, a, prompt, controls, stream, notify).await,
        "gemini" | "antigravity" => {
            gemini::exchange(io, store, a, prompt, controls, stream, notify).await
        }
        _ => codex_exchange(io, store, a, prompt, controls, stream, notify).await,
    };
    let unconfirmed = store.finish_pending_directions(Some(a.id))?;
    if unconfirmed {
        notify(ReplyUpdate::TranscriptChanged);
    }
    let finished = result?;
    // A late reply for an unconfirmed input belongs to this connection. A
    // fresh process can resume the session without consuming that stale reply.
    Ok(TurnResult {
        finished,
        reuse_connection: finished && !io.used_tools && !unconfirmed,
    })
}

async fn codex_exchange(
    io: &mut Connection,
    store: &Store,
    a: &Attempt,
    prompt: &Message,
    controls: &mut Controls,
    stream: bool,
    notify: &dyn Fn(ReplyUpdate<'_>),
) -> Result<bool> {
    let mut log = append(&store.folder(a.id).join("events.jsonl"))?;
    store.artifact(a.id, "events", "events.jsonl")?;
    store.artifact(a.id, "stderr", "provider.stderr.log")?;
    let Connection {
        stdin,
        lines,
        session,
        used_tools,
        gemini_bridge: legacy_assets,
        ..
    } = io;
    if session.is_empty() {
        send(stdin,json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"workshop","title":"Workshop","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}})).await?;
    } else {
        // Check the live server since it can unload an idle conversation.
        send(
            stdin,
            json!({"id":5,"method":"thread/loaded/list","params":{}}),
        )
        .await?;
    }
    let team_preference = team_preference(store, a.id)?;
    let mut turn = String::new();
    let mut next_id = 10;
    let mut directions = std::collections::VecDeque::<Message>::new();
    let mut awaiting_ack = std::collections::HashMap::new();
    let mut cancelling = false;
    let mut stopped_keys = std::collections::HashSet::new();
    let mut repairs = Repairs::start(store, a)?;
    let mut external_blocker = None;
    let mut runner_failure = None;
    let mut deadline = Some(Instant::now() + Duration::from_secs(90));
    type ToolFuture<'a> =
        std::pin::Pin<Box<dyn std::future::Future<Output = Result<team::ToolOutput>> + 'a>>;
    let mut pending_tool: Option<(Option<Value>, ToolFuture<'_>)> = None;
    let mut bridge_tick = tokio::time::interval(Duration::from_millis(100));
    enum Next {
        Deadline,
        Input(Option<Control>),
        Line(Option<String>),
        Tool(Result<team::ToolOutput>),
        Bridge,
    }
    loop {
        if !turn.is_empty() && !cancelling {
            while let Some(message) = directions.pop_front() {
                match message.input() {
                    Ok(input) => {
                        send(stdin,json!({"id":next_id,"method":"turn/steer","params":{"threadId":session,"expectedTurnId":turn,"input":input}})).await?;
                        awaiting_ack.insert(next_id, message);
                        next_id += 1;
                    }
                    Err(error) => {
                        store.direction_unavailable(
                            a.id,
                            &message,
                            &format!("Could not send direction: {error:#}"),
                        )?;
                        notify(ReplyUpdate::TranscriptChanged);
                    }
                }
            }
        }
        let timeout = async {
            match deadline {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        let poll_bridge =
            legacy_assets.is_some() && !turn.is_empty() && pending_tool.is_none() && !cancelling;
        let tool_result = async {
            match pending_tool.as_mut() {
                Some((_, future)) => future.await,
                None => std::future::pending().await,
            }
        };
        let next = tokio::select! {
            result=tool_result=>Next::Tool(result),
            _=bridge_tick.tick(), if poll_bridge => Next::Bridge,
            _ = timeout => Next::Deadline,
            control = controls.recv() => Next::Input(control),
            line = lines.next_line() => Next::Line(line?),
        };
        match next {
            Next::Bridge => {
                if let Some(call) = legacy_assets
                    .as_ref()
                    .expect("active legacy bridge")
                    .call()?
                {
                    *used_tools = true;
                    let args = json!({"tool":call["name"],"arguments":call["arguments"]});
                    pending_tool = Some((
                        None,
                        Box::pin(async move { team::dispatch_tool(store, a, &args, notify).await }),
                    ));
                }
            }
            Next::Tool(result) => {
                let (id, _) = pending_tool.take().expect("completed tool request");
                if id.is_none() {
                    legacy_assets
                        .as_ref()
                        .expect("active legacy bridge")
                        .reply(result)?;
                    continue;
                }
                send(
                    stdin,
                    json!({"id":id,"result":team::ToolOutput::codex(result)}),
                )
                .await?;
            }
            Next::Deadline => {
                if cancelling {
                    return Ok(false);
                }
                bail!("Codex did not start a turn within 90 seconds. Check provider.stderr.log.");
            }
            Next::Input(control) => match control {
                None | Some(Control::Cancel) => {
                    if pending_tool.take().is_some() {
                        store.interrupt_workers(a.id, true)?;
                        notify(ReplyUpdate::WorkersChanged);
                    }
                    if turn.is_empty() || cancelling {
                        return Ok(false);
                    }
                    send(stdin,json!({"id":4,"method":"turn/interrupt","params":{"threadId":session,"turnId":turn}})).await?;
                    cancelling = true;
                    deadline = Some(Instant::now() + Duration::from_secs(3));
                }
                Some(Control::StopWorker(id)) => {
                    if store.stop_worker(id).is_ok()
                        && let Some(worker) = store.workers(a.id)?.into_iter().find(|w| w.id == id)
                    {
                        stopped_keys.insert(worker.assignment.key);
                    }
                }
                Some(Control::Steer(message)) => {
                    if cancelling {
                        store.direction_unavailable(
                            a.id,
                            &message,
                            "The run is stopping. Send your direction after it stops.",
                        )?;
                        notify(ReplyUpdate::TranscriptChanged);
                    } else {
                        directions.push_back(store.stage_direction(a.id, message)?);
                    }
                }
                Some(Control::Queue(message)) => {
                    store.queue_message(a.id, &message)?;
                    notify(ReplyUpdate::TranscriptChanged);
                }
                Some(Control::SteerWorker(id, text)) => {
                    route_worker_direction(store, a.id, id, &text, notify)?
                }
            },
            Next::Line(line) => {
                let line = line.context("Codex closed its output. See provider.stderr.log.")?;
                writeln!(log, "{line}")?;
                if stream {
                    println!("{line}");
                }
                let v: Value = serde_json::from_str(&line).context("Invalid Codex event")?;
                let id = v["id"].as_i64();
                if v.get("method").is_none()
                    && let Some(message) = id.and_then(|id| awaiting_ack.remove(&id))
                {
                    if v["error"].is_null() {
                        message.record_direction(store, a.id, notify)?;
                        repairs.reset_goal_reports(store, a.id)?;
                    } else {
                        store.direction_unavailable(
                            a.id,
                            &message,
                            v["error"]["message"]
                                .as_str()
                                .unwrap_or("Codex rejected this direction."),
                        )?;
                        notify(ReplyUpdate::TranscriptChanged);
                    }
                    if id != Some(3) {
                        continue;
                    }
                }
                if !v["error"].is_null() {
                    if id == Some(5) && v["error"]["code"] == -32601 {
                        open_thread(stdin, store, a, legacy_assets.as_ref()).await?;
                        continue;
                    }
                    bail!("Codex: {}", v["error"]["message"]);
                }
                if let Some(method) = v["method"].as_str() {
                    if method == "item/tool/call"
                        && !v["id"].is_null()
                        && v["params"]["threadId"] == session.as_str()
                        && !cancelling
                    {
                        *used_tools = true;
                        let args = &v["params"]["arguments"];
                        if v["params"]["tool"] == "workshop_blocked"
                            && !store.worker
                            && pending_tool.is_none()
                            && v["params"]["turnId"] == turn
                        {
                            external_blocker = args["reason"]
                                .as_str()
                                .filter(|s| !s.trim().is_empty())
                                .map(|s| s.chars().take(4000).collect::<String>());
                        }
                        let retries_stopped =
                            retries_stopped(v["params"]["tool"].as_str(), args, &stopped_keys);
                        if retries_stopped {
                            send(stdin,json!({"id":v["id"],"result":{"success":false,"contentItems":[{"type":"inputText","text":"The user stopped this assignment. Leave it stopped until they resume the task."}]}})).await?;
                            continue;
                        }
                        if pending_tool.is_some() || v["params"]["turnId"] != turn {
                            send(stdin,json!({"id":v["id"],"result":{"success":false,"contentItems":[{"type":"inputText","text":"Wait for the active delegation batch before submitting another request."}]}})).await?;
                        } else {
                            let args = v["params"].clone();
                            pending_tool = Some((
                                Some(v["id"].clone()),
                                Box::pin(async move {
                                    team::dispatch_tool(store, a, &args, notify).await
                                }),
                            ));
                        }
                        continue;
                    }
                    if !v["id"].is_null() {
                        send(stdin,json!({"id":v["id"],"error":{"code":-32601,"message":"Unsupported client request"}})).await?;
                        continue;
                    }
                    if method.starts_with("item/")
                        && (v["params"]["threadId"] != session.as_str()
                            || v["params"]["turnId"] != turn)
                    {
                        continue;
                    }
                    // Release the process tree before capturing edits if a tool could have left work running.
                    if method == "item/started"
                        && !matches!(
                            v["params"]["item"]["type"].as_str(),
                            Some("userMessage" | "agentMessage" | "reasoning")
                        )
                    {
                        *used_tools = true;
                    }
                    if method == "item/completed"
                        && v["params"]["threadId"] == session.as_str()
                        && v["params"]["turnId"] == turn
                        && v["params"]["item"]["type"] == "commandExecution"
                    {
                        let item = &v["params"]["item"];
                        if item["status"] == "failed" && item["exitCode"].as_i64() == Some(-1) {
                            runner_failure = Some(
                                item["aggregatedOutput"]
                                    .as_str()
                                    .unwrap_or("Command runner could not start")
                                    .chars()
                                    .take(1500)
                                    .collect::<String>(),
                            );
                        } else if item["status"] == "completed" {
                            runner_failure = None;
                        }
                    }
                    if method == "item/agentMessage/delta"
                        && v["params"]["threadId"] == session.as_str()
                        && v["params"]["turnId"] == turn
                        && let Some(delta) = v["params"]["delta"].as_str()
                    {
                        notify(ReplyUpdate::Delta(delta));
                    }
                    if method == "account/rateLimits/updated"
                        && let Some(windows) = account_windows(&v["params"]["rateLimits"])
                    {
                        notify(ReplyUpdate::AccountUsage("codex", windows));
                    }
                    if method == "thread/tokenUsage/updated"
                        && v["params"]["threadId"] == session.as_str()
                        && let Some(tokens) =
                            v["params"]["tokenUsage"]["total"]["totalTokens"].as_i64()
                    {
                        let context = ContextUsage::from_report(&v["params"]["tokenUsage"]);
                        store.record_usage(a.id, tokens, context)?;
                        notify(ReplyUpdate::Usage(tokens));
                        notify(ReplyUpdate::Context(context));
                    }
                    if !method.ends_with("delta") && !method.ends_with("Delta") {
                        if !store.worker
                            && method == "item/completed"
                            && v["params"]["threadId"] == session.as_str()
                            && v["params"]["turnId"] == turn
                            && v["params"]["item"]["type"] == "imageGeneration"
                        {
                            let saved =
                                art::generated_image(&v["params"]["item"], &store.work(a.id))
                                    .and_then(|image| {
                                        store.post_chat_image(a.id, &image, "Generated image")
                                    });
                            if let Err(error) = saved {
                                store.event(
                                    a.id,
                                    "workshop/image_error",
                                    Some(&format!("Could not load the generated image. {error:#}")),
                                )?;
                            }
                            notify(ReplyUpdate::TranscriptChanged);
                        }
                        let kind = if method == "item/completed"
                            && v["params"]["item"]["type"] == "agentMessage"
                        {
                            "agent/message"
                        } else {
                            method
                        };
                        let action = label(&v);
                        store.event(a.id, kind, action.as_deref())?;
                        if method == "turn/started" && v["params"]["threadId"] == session.as_str() {
                            notify(ReplyUpdate::Activity("Waiting for a reply…"));
                        }
                        if method == "item/started"
                            && v["params"]["threadId"] == session.as_str()
                            && let Some(action) = action.as_deref()
                        {
                            let display = if v["params"]["item"]["type"] == "commandExecution"
                                && !store.worker
                            {
                                "Running a command…"
                            } else {
                                tool_activity(action)
                            };
                            notify(ReplyUpdate::Activity(display));
                        }
                        if kind == "agent/message" {
                            notify(ReplyUpdate::Stored);
                        }
                    }
                    if method == "turn/completed"
                        && v["params"]["threadId"] == session.as_str()
                        && v["params"]["turn"]["id"] == turn
                    {
                        let result = &v["params"]["turn"];
                        if cancelling || result["status"] == "interrupted" {
                            return Ok(false);
                        }
                        ensure!(
                            result["status"] == "completed",
                            "Codex turn {}: {}",
                            result["status"],
                            result["error"]
                        );
                        ensure!(
                            pending_tool.is_none(),
                            "Lead ended its turn during a worker assignment"
                        );
                        if let Some(reason) = &external_blocker {
                            bail!("Task blocked: {reason}");
                        }
                        let workers = store.workers(a.id)?;
                        let unavailable = if let Some(reason) = &runner_failure
                            && !workers.iter().any(|w| w.state == "finished")
                            && (!store.worker || store.reported_result(a.id)?.is_none())
                            && crate::git::unchanged_since(&store.work(a.id), &a.base)?
                        {
                            Some(anyhow::anyhow!(
                                "Workspace command runner unavailable: {reason}"
                            ))
                        } else {
                            None
                        };
                        let direction = match repairs
                            .verify(
                                store,
                                a,
                                controls,
                                notify,
                                *used_tools,
                                !workers.is_empty(),
                                !stopped_keys.is_empty(),
                                unavailable,
                            )
                            .await?
                        {
                            Finish::Done => return Ok(true),
                            Finish::Cancelled => return Ok(false),
                            Finish::Continue(direction) => direction,
                        };
                        start_turn(stdin, session, &direction, a, team_preference.as_deref())
                            .await?;
                        if direction.queued_id.is_some() {
                            awaiting_ack.insert(3, direction);
                        }
                        turn.clear();
                        deadline = Some(Instant::now() + Duration::from_secs(90));
                        continue;
                    }
                } else {
                    match id {
                        Some(1) => {
                            send(stdin, json!({"method":"initialized"})).await?;
                            open_thread(stdin, store, a, legacy_assets.as_ref()).await?;
                        }
                        Some(2) => {
                            *session = v["result"]["thread"]["id"]
                                .as_str()
                                .context("Codex did not return a session ID")?
                                .into();
                            store.bind_session(
                                a.id,
                                session,
                                v["result"]["model"].as_str().unwrap_or_default(),
                                v["result"]["reasoningEffort"].as_str().unwrap_or_default(),
                            )?;
                            if store.worker {
                                let reported = &v["result"];
                                let model = if a.model.is_empty() {
                                    reported["model"].as_str().unwrap_or_default()
                                } else {
                                    &a.model
                                };
                                let effort = if a.effort.is_empty() {
                                    reported["reasoningEffort"].as_str().unwrap_or_default()
                                } else {
                                    &a.effort
                                };
                                notify(ReplyUpdate::WorkerInference(model, effort));
                            }
                            if a.session.is_empty() {
                                store.db.lock().unwrap().execute(
                                    "INSERT OR REPLACE INTO codex_asset_sessions(session,pack_tools) VALUES(?1,1)",
                                    [&*session],
                                )?;
                            }
                            start_turn(stdin, session, prompt, a, team_preference.as_deref())
                                .await?;
                            prompt.record_followup(store, a, notify)?;
                        }
                        Some(3) => {
                            turn = v["result"]["turn"]["id"]
                                .as_str()
                                .context("Codex did not return a turn ID")?
                                .into();
                            deadline = None;
                        }
                        Some(5) => {
                            let loaded = v["result"]["data"]
                                .as_array()
                                .context("Codex did not return the loaded thread IDs")?;
                            if loaded.iter().any(|id| id == session.as_str()) {
                                start_turn(stdin, session, prompt, a, team_preference.as_deref())
                                    .await?;
                                prompt.record_followup(store, a, notify)?;
                            } else {
                                open_thread(stdin, store, a, legacy_assets.as_ref()).await?;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

pub(crate) async fn worker_turn(
    store: &Store,
    a: &Attempt,
    mut rx: Controls,
    notify: &dyn Fn(ReplyUpdate<'_>),
) -> Result<Option<String>> {
    let result = async {
        let mut io = Connection::open(store, a)?;
        let mut message: Message = a.prompt.as_str().into();
        message.files = store.task_attachments(a.task)?;
        Box::pin(exchange(
            &mut io, store, a, &message, &mut rx, false, notify,
        ))
        .await
        .map(|result| result.finished)
    }
    .await;
    rx.close();
    while let Ok(control) = rx.try_recv() {
        if let Control::Steer(message) = control {
            store.direction_unavailable(
                a.id,
                &message,
                "The worker finished before receiving this direction.",
            )?;
            notify(ReplyUpdate::TranscriptChanged);
        }
    }
    let report = store.reported_result(a.id)?;
    let result = result.and_then(|finished| {
        if finished && let Some((kind, detail)) = &report {
            ensure!(
                kind != "worker/needs_changes",
                "Worker acceptance criteria unmet: {detail}"
            );
        }
        Ok(finished)
    });
    let summary = match &result {
        Ok(true) => "Ready for integration".into(),
        Ok(false) => "Worker interrupted".into(),
        Err(e) => format!("{e:#}"),
    };
    store.finish(
        a.id,
        if matches!(result, Ok(true)) {
            "review"
        } else {
            "failed"
        },
        &summary,
    )?;
    ensure!(result?, "Worker interrupted");
    Ok(report.map(|(_, summary)| summary))
}

pub fn run(
    store: &Store,
    a: &Attempt,
    prompt: impl Into<Message>,
    controls: Controls,
    stream: bool,
) -> Result<()> {
    run_turn(&mut None, store, a, prompt, controls, stream, |update| {
        if stream
            && matches!(update, ReplyUpdate::WorkersChanged)
            && let Ok(workers) = store.workers(a.id)
        {
            let rows: Vec<_> = workers
                .iter()
                .map(|w| {
                    json!({
                        "id": w.id, "key": w.assignment.key, "state": w.state,
                        "retries": w.tries.saturating_sub(1), "tokens": w.tokens, "model": w.model, "effort": w.effort, "limits": w.limits
                    })
                })
                .collect();
            println!(
                "{}",
                json!({"method": "workshop/workers", "params": {
                    "attemptId": a.id, "workers": rows
                }})
            );
        }
    })
}

pub fn run_turn(
    cached: &mut Option<Session>,
    store: &Store,
    a: &Attempt,
    prompt: impl Into<Message>,
    mut controls: Controls,
    stream: bool,
    notify: impl FnMut(ReplyUpdate<'_>),
) -> Result<()> {
    let observer = std::cell::RefCell::new(notify);
    let notify = |event: ReplyUpdate<'_>| observer.borrow_mut()(event);
    let mut attempt = a.clone();
    let mut prompt = prompt.into();
    let outcome = (|| -> Result<()> {
        loop {
            let finished = run_one_turn(
                cached,
                store,
                &attempt,
                prompt,
                &mut controls,
                stream,
                &notify,
            )?;
            if !finished
                || drain_finished_controls(store, a.id, &mut controls, &notify)?
                || controls.is_closed()
            {
                return Ok(());
            }
            let Some(next) = store.claim_queued(a.id, false)? else {
                return Ok(());
            };
            (attempt, prompt) = next;
            notify(ReplyUpdate::TranscriptChanged);
        }
    })();
    // Close before the final drain so every successful send during snapshotting
    // or shutdown gets either a queued follow-up or a delivery receipt.
    controls.close();
    drain_finished_controls(store, a.id, &mut controls, &notify)?;
    outcome
}

fn drain_finished_controls(
    store: &Store,
    chat: i64,
    controls: &mut Controls,
    notify: &dyn Fn(ReplyUpdate<'_>),
) -> Result<bool> {
    let mut stopped = false;
    while let Ok(control) = controls.try_recv() {
        match control {
            Control::Cancel => stopped = true,
            Control::Queue(message) => store.save_queued(chat, &message, true)?,
            Control::Steer(message) => store.direction_unavailable(
                chat,
                &message,
                "The run ended before this direction could be sent.",
            )?,
            Control::SteerWorker(id, text) => {
                route_worker_direction(store, chat, id, &text, notify)?
            }
            Control::StopWorker(_) => {}
        }
        notify(ReplyUpdate::TranscriptChanged);
    }
    Ok(stopped)
}

fn run_one_turn(
    cached: &mut Option<Session>,
    store: &Store,
    a: &Attempt,
    mut prompt: Message,
    controls: &mut Controls,
    stream: bool,
    notify: &dyn Fn(ReplyUpdate<'_>),
) -> Result<bool> {
    let queued_id = prompt.queued_id;
    let result: Result<bool> = (|| {
        if a.session.is_empty() && prompt.files.is_empty() {
            prompt.files = store.task_attachments(a.task)?;
        }
        let mut session = match cached.take() {
            Some(mut live) => {
                if !a.session.is_empty()
                    && live.io.session == a.session
                    && live.io.cwd == store.work(a.id)
                    && live.io.gemini_bridge.as_ref().is_none_or(|b| {
                        a.provider != "gemini"
                            || b.project_trusted == store.gemini_project_trusted()
                    })
                    && live
                        .io
                        .pinned_inference
                        .as_ref()
                        .is_none_or(|(m, e)| m == &a.model && e == &a.effort)
                    && matches!(live.io.child.try_wait(), Ok(None))
                {
                    live
                } else {
                    drop(live);
                    Session::open(store, a)?
                }
            }
            None => Session::open(store, a)?,
        };
        let result = session.runtime.block_on(exchange(
            &mut session.io,
            store,
            a,
            &prompt,
            controls,
            stream,
            notify,
        ))?;
        if result.reuse_connection {
            *cached = Some(session);
        }
        Ok(result.finished)
    })();
    if !matches!(result, Ok(true)) {
        if let Some(id) = queued_id {
            store
                .db
                .lock()
                .unwrap()
                .execute("UPDATE queued_messages SET sending=0 WHERE id=?1", [id])?;
        }
        store.interrupt_workers(a.id, matches!(result, Ok(false)))?;
    }
    let failure;
    let (state, action) = match &result {
        Ok(true) => ("review", "Finished"),
        Ok(false) => (
            "cancelled",
            "Cancelled. Any edits are available for review.",
        ),
        Err(e) => {
            failure = format!("{e:#}");
            ("failed", failure.as_str())
        }
    };
    if state != "review" && !store.worker {
        store.pause_goal(a.id, action)?;
    }
    if let Err(e) = store.finish_turn(a.id, state, action, true) {
        store.pause_goal(a.id, "Workshop could not save the changes.")?;
        if store.attempt(a.id)?.state != "applying" {
            store.status(
                a.id,
                "failed",
                &format!("Could not save the changes. Working copy retained. {e:#}"),
            )?;
        }
        return Err(e);
    }
    result
}

fn route_worker_direction(
    store: &Store,
    lead: i64,
    id: i64,
    text: &str,
    notify: &dyn Fn(ReplyUpdate<'_>),
) -> Result<()> {
    if let Err(error) = store.steer_worker(lead, id, text.into()) {
        store.event(
            lead,
            "worker/delivery_error",
            Some(&format!("Worker {id}: {error}")),
        )?;
    }
    notify(ReplyUpdate::TranscriptChanged);
    Ok(())
}
