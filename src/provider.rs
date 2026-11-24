use crate::{
    Attempt, Store, team,
    usage::{ContextUsage, UsageWindow, account_windows},
};
mod art;
mod catalog;
use anyhow::{Context, Result, bail, ensure};
pub(crate) use art::{design_model, generate_image};
pub(crate) use catalog::worker_catalog;
pub use catalog::{Catalog, Model as CatalogModel, load_models_and_usage, model_catalog};
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
    let mut cmd = CommandWrap::with_new(program, |cmd| {
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(stdout)
            .stderr(stderr);
    });
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
}
impl From<&str> for Message {
    fn from(text: &str) -> Self {
        Self {
            text: text.into(),
            files: Vec::new(),
        }
    }
}
impl Message {
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
        Self { text, files }
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
fn label(v: &Value) -> Option<String> {
    let p = &v["params"];
    let s = match v["method"].as_str()? {
        "item/started" => match p["item"]["type"].as_str()? {
            "commandExecution" => p["item"]["command"].as_str().unwrap_or("Running a command"),
            "fileChange" => "Editing project files",
            "reasoning" => "Thinking through the task",
            "mcpToolCall" => p["item"]["tool"].as_str().unwrap_or("Using a tool"),
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

async fn open_thread(stdin: &mut ChildStdin, store: &Store, a: &Attempt) -> Result<()> {
    let mut params = json!({"cwd":store.work(a.id),"approvalPolicy":"never","sandbox":"workspace-write",
        "developerInstructions":"Work only in the current project copy. Keep edits available for the creator to review."});
    params["config"] = json!({"features.multi_agent":false});
    if !store.worker {
        let context = store.handoff(a.id)?;
        let limit = store.team_settings()?.parallelism;
        let policy = store.effective_worker_policy(a.id)?;
        let policy = serde_json::to_string(&policy)?;
        params["developerInstructions"] = json!(format!(
            "You are the lead Codex agent in Workshop. Work inside this isolated project. When the user asks for subagents or workers, call workshop_delegate even for simple work. Honor the requested number of assignments when feasible. Report only the workers actually dispatched and their returned outcomes. If delegation fails, explain what happened. Otherwise handle simple requests directly. For substantial divisible work, choose a parallel, sequential, or mixed plan. Submit all known assignments together in one workshop_delegate call. The project allows up to {limit} concurrent workers. Worker policy: {policy}. An empty models list allows any available model, and an empty max_effort means uncapped. Optionally choose model and effort per assignment within this policy; omit either to use its default. Leave depends_on empty for independent work. To order steps, list the prerequisite keys on each dependent task. If the next assignment needs a result to define its scope, delegate it after that result returns. Give every worker a stable key, short title, exact relative edit paths ([] for read-only), relevant interfaces in its deliverable and acceptance criteria. Workshop schedules these assignments and integrates their changes. Retry a blocked worker using its original key and failure feedback; report an unresolved blocker instead of creating a replacement key. Use workshop_checkpoint for design decisions and next unfinished work on substantial tasks. Project handoff:\n{context}"
        ));
    } else {
        params["developerInstructions"] = json!(
            "You are a Workshop Codex worker in a private copy. Complete only the assigned deliverable within the declared edit scope, verify it and report the result. The lead integrates your edits. Do not delegate."
        );
    }
    let mut instructions = params["developerInstructions"].as_str().unwrap().to_owned();
    instructions.push_str(" For questions about compiled game modules, check workshop_find_reports first and use workshop_read_report to inspect any saved result. Report partial coverage and the run ID when relevant.");
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
) -> Result<()> {
    let mut params = json!({"threadId":session,"input":prompt.input()?});
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
        const MAX_FRAME: usize = 16 * 1024 * 1024;
        loop {
            let bytes = self.reader.fill_buf().await?;
            if bytes.is_empty() {
                ensure!(
                    self.pending.is_empty(),
                    "Codex closed in the middle of an event"
                );
                return Ok(None);
            }
            let end = bytes.iter().position(|b| *b == b'\n');
            let count = end.unwrap_or(bytes.len());
            ensure!(
                self.pending.len() + count <= MAX_FRAME,
                "Codex event exceeded the 16 MiB frame limit"
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
        Self::spawn(store.work(a.id), stderr)
    }
    fn spawn(cwd: PathBuf, stderr: File) -> Result<Self> {
        let exe = codex_program();

        let mut child = managed(
            &exe,
            &["app-server", "--listen", "stdio://"],
            &cwd,
            Stdio::piped(),
            stderr,
        )?;
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
            session: String::new(),
        })
    }
}

pub enum ReplyUpdate<'a> {
    Delta(&'a str),
    Stored,
    TranscriptChanged,
    WorkersChanged,
    Usage(i64),
    Context(ContextUsage),
    AccountUsage(Vec<UsageWindow>),
    Activity(&'a str),
}

async fn exchange(
    io: &mut Connection,
    store: &Store,
    a: &Attempt,
    prompt: &Message,
    mut controls: Controls,
    stream: bool,
    notify: &dyn Fn(ReplyUpdate<'_>),
) -> Result<bool> {
    if !a.session.is_empty() {
        store.event(a.id, "user/followup", Some(&prompt.text))?;
        notify(ReplyUpdate::TranscriptChanged);
    }
    let mut log = append(&store.folder(a.id).join("events.jsonl"))?;
    store.artifact(a.id, "events", "events.jsonl")?;
    store.artifact(a.id, "stderr", "provider.stderr.log")?;
    let Connection {
        stdin,
        lines,
        session,
        used_tools,
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
    let mut turn = String::new();
    let mut next_id = 10;
    let mut cancelling = false;
    let mut deadline = Some(Instant::now() + Duration::from_secs(90));
    type ToolFuture<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + 'a>>;
    let mut pending_tool: Option<(Value, ToolFuture<'_>)> = None;
    enum Next {
        Deadline,
        Input(Option<Control>),
        Line(Option<String>),
        Tool(Result<String>),
    }
    loop {
        let timeout = async {
            match deadline {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        let tool_result = async {
            match pending_tool.as_mut() {
                Some((_, future)) => future.await,
                None => std::future::pending().await,
            }
        };
        let next = tokio::select! {
            result=tool_result=>Next::Tool(result),
            _ = timeout => Next::Deadline,
            control = controls.recv() => Next::Input(control),
            line = lines.next_line() => Next::Line(line?),
        };
        match next {
            Next::Tool(result) => {
                let (id, _) = pending_tool.take().expect("completed tool request");
                let success = result.is_ok();
                let text = result.unwrap_or_else(|e| format!("{e:#}"));
                send(stdin,json!({"id":id,"result":{"success":success,"contentItems":[{"type":"inputText","text":text}]}})).await?;
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
                        store.interrupt_workers(a.id)?;
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
                    let _ = store.stop_worker(id);
                }
                Some(Control::Steer(text)) => {
                    if turn.is_empty() || cancelling {
                        store.event(
                            a.id,
                            "steer/unavailable",
                            Some("Wait for an active turn before sending direction."),
                        )?;
                        notify(ReplyUpdate::TranscriptChanged);
                        continue;
                    }
                    let input = match text.input() {
                        Ok(input) => input,
                        Err(error) => {
                            store.event(
                                a.id,
                                "steer/unavailable",
                                Some(&format!("Could not send direction: {error:#}")),
                            )?;
                            notify(ReplyUpdate::TranscriptChanged);
                            continue;
                        }
                    };
                    send(stdin,json!({"id":next_id,"method":"turn/steer","params":{"threadId":session,"expectedTurnId":turn,"input":input}})).await?;
                    next_id += 1;
                    store.event(a.id, "steer/sent", Some(&text.text))?;
                    notify(ReplyUpdate::TranscriptChanged);
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
                if !v["error"].is_null() {
                    if id == Some(5) && v["error"]["code"] == -32601 {
                        open_thread(stdin, store, a).await?;
                        continue;
                    }
                    if id.is_some_and(|id| id >= 10) {
                        store.event(a.id, "steer/error", v["error"]["message"].as_str())?;
                        notify(ReplyUpdate::TranscriptChanged);
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
                        if pending_tool.is_some() || v["params"]["turnId"] != turn {
                            send(stdin,json!({"id":v["id"],"result":{"success":false,"contentItems":[{"type":"inputText","text":"Wait for the active delegation batch before submitting another request."}]}})).await?;
                        } else {
                            let args = v["params"].clone();
                            pending_tool = Some((
                                v["id"].clone(),
                                Box::pin(
                                    async move { team::dispatch(store, a, &args, notify).await },
                                ),
                            ));
                        }
                        continue;
                    }
                    if !v["id"].is_null() {
                        send(stdin,json!({"id":v["id"],"error":{"code":-32601,"message":"Unsupported client request"}})).await?;
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
                        notify(ReplyUpdate::AccountUsage(windows));
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
                        let kind = if method == "item/completed"
                            && v["params"]["item"]["type"] == "agentMessage"
                        {
                            "agent/message"
                        } else {
                            method
                        };
                        let action = label(&v);
                        store.event(a.id, kind, action.as_deref())?;
                        if method == "item/started"
                            && let Some(action) = action.as_deref()
                        {
                            notify(ReplyUpdate::Activity(action));
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
                        return Ok(true);
                    }
                } else {
                    match id {
                        Some(1) => {
                            send(stdin, json!({"method":"initialized"})).await?;
                            open_thread(stdin, store, a).await?;
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
                            start_turn(stdin, session, prompt, a).await?;
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
                                start_turn(stdin, session, prompt, a).await?;
                            } else {
                                open_thread(stdin, store, a).await?;
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
    rx: Controls,
    notify: &dyn Fn(ReplyUpdate<'_>),
) -> Result<()> {
    let result = {
        let mut io = Connection::open(store, a)?;
        Box::pin(exchange(
            &mut io,
            store,
            a,
            &a.prompt.as_str().into(),
            rx,
            false,
            notify,
        ))
        .await
    };
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
    Ok(())
}

pub fn run(
    store: &Store,
    a: &Attempt,
    prompt: &str,
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
                        "retries": w.tries.saturating_sub(1), "tokens": w.tokens
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
    controls: Controls,
    stream: bool,
    notify: impl FnMut(ReplyUpdate<'_>),
) -> Result<()> {
    let observer = std::cell::RefCell::new(notify);
    let result: Result<bool> = (|| {
        let mut prompt = prompt.into();
        if a.session.is_empty() {
            prompt.files = store.task_attachments(a.task)?;
        }
        let mut session = match cached.take() {
            Some(mut live) => {
                if !a.session.is_empty()
                    && live.io.session == a.session
                    && live.io.cwd == store.work(a.id)
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
        let finished = session.runtime.block_on(exchange(
            &mut session.io,
            store,
            a,
            &prompt,
            controls,
            stream,
            &|event| observer.borrow_mut()(event),
        ))?;
        if finished && !store.workers(a.id)?.is_empty() {
            session.runtime.block_on(store.verify_final(a))?;
        }
        if finished && !session.io.used_tools {
            *cached = Some(session);
        }
        Ok(finished)
    })();
    if !matches!(result, Ok(true)) {
        store.interrupt_workers(a.id)?;
    }
    let failure;
    let (state, action) = match &result {
        Ok(true) => ("review", "Codex finished. Review the change."),
        Ok(false) => (
            "cancelled",
            "Cancelled. Any edits are available for review.",
        ),
        Err(e) => {
            failure = format!("{e:#}");
            ("failed", failure.as_str())
        }
    };
    if let Err(e) = store.finish(a.id, state, action) {
        store.status(
            a.id,
            "failed",
            &format!("Could not save the review. Working copy retained. {e:#}"),
        )?;
        return Err(e);
    }
    result.map(|_| ())
}
