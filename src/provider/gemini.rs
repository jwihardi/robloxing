use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::{HashSet, VecDeque};
use tokio::io::AsyncReadExt;
mod bridge;
pub(super) use bridge::Bridge;
pub use bridge::{is_bridge_process, serve_stdio};

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|file| file.is_file())
}

fn node() -> Result<PathBuf> {
    on_path(if cfg!(windows) { "node.exe" } else { "node" })
        .context("Node.js is required by the Gemini CLI npm installation")
}

fn launcher() -> Result<(PathBuf, Option<PathBuf>)> {
    if let Some(path) = std::env::var_os("WORKSHOP_GEMINI") {
        let path = PathBuf::from(path);
        if matches!(
            path.extension().and_then(|x| x.to_str()),
            Some("js" | "mjs" | "cjs")
        ) {
            ensure!(path.is_file(), "Gemini CLI entry point is missing");
            return Ok((node()?, Some(path)));
        }
        let executable = if path.is_file() {
            Some(dunce::canonicalize(path)?)
        } else {
            path.to_str().and_then(on_path)
        };
        return Ok((
            executable.context("WORKSHOP_GEMINI does not point to an installed Gemini CLI")?,
            None,
        ));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path).filter(|dir| dir.is_absolute()) {
        let exe = dir.join(if cfg!(windows) {
            "gemini.exe"
        } else {
            "gemini"
        });
        if exe.is_file() {
            return Ok((exe, None));
        }
        if cfg!(windows) && dir.join("gemini.cmd").is_file() {
            // Launch the npm binary through Node directly. cmd.exe cannot safely
            // pass arbitrary arguments and opens an extra console on Windows.
            let package = dir.join("node_modules/@google/gemini-cli");
            let manifest: Value =
                serde_json::from_reader(File::open(package.join("package.json"))?)?;
            let entry = package.join(
                manifest["bin"]["gemini"]
                    .as_str()
                    .context("Missing Gemini npm entry point")?,
            );
            ensure!(entry.is_file(), "Gemini CLI installation is incomplete");
            return Ok((node()?, Some(entry)));
        }
    }
    bail!("Install Gemini CLI with npm install -g @google/gemini-cli, then refresh Workshop")
}

pub fn installed() -> bool {
    launcher().is_ok()
}

impl Store {
    pub fn gemini_project_trusted(&self) -> bool {
        self.archive_dir.join("gemini-project-trust").is_file()
    }
    pub fn set_gemini_project_trusted(&self, trust: bool) -> Result<()> {
        let path = self.archive_dir.join("gemini-project-trust");
        if trust {
            File::create(path)?;
        } else if let Err(e) = std::fs::remove_file(path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(e.into());
        }
        Ok(())
    }
}

fn spawn(dir: &Path, args: &[&str], stderr: File) -> Result<Connection> {
    spawn_cli(dir, args, stderr, launcher()?, "Gemini CLI")
}

pub(super) fn spawn_cli(
    dir: &Path,
    args: &[&str],
    stderr: File,
    (program, entry): (PathBuf, Option<PathBuf>),
    provider: &str,
) -> Result<Connection> {
    let cmd = CommandWrap::with_new(&program, |cmd| {
        cmd.args(entry)
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr);
    });
    let mut child = spawn_managed(cmd, provider)?;
    let stdin = child.stdin().take().context("Missing provider stdin")?;
    let stdout = child.stdout().take().context("Missing provider stdout")?;
    Ok(Connection {
        child,
        stdin,
        lines: Frames {
            reader: BufReader::new(stdout),
            pending: Vec::new(),
        },
        cwd: dir.into(),
        used_tools: false,
        pinned_inference: None,
        session: String::new(),
        gemini_bridge: None,
    })
}

pub(super) async fn models(dir: &Path) -> Result<Catalog> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut io = spawn(dir, &["models", "-o", "json"], tempfile::tempfile()?)?;
        let mut out = Vec::new();
        io.lines
            .reader
            .take(1024 * 1024 + 1)
            .read_to_end(&mut out)
            .await?;
        ensure!(
            out.len() <= 1024 * 1024,
            "Gemini model catalog exceeds 1 MiB"
        );
        ensure!(
            io.child.wait().await?.success(),
            "Gemini model discovery failed"
        );
        let entries: Vec<Value> =
            serde_json::from_slice(&out).context("Invalid Gemini model catalog")?;
        let mut models = vec![];
        for name in
            std::iter::once("default").chain(entries.iter().filter_map(|e| e["modelId"].as_str()))
        {
            let model = format!("gemini:{name}");
            if !models.iter().any(|m: &CatalogModel| m.model == model) {
                models.push(CatalogModel {
                    model,
                    display_name: None,
                    resolved_model: None,
                    supported_reasoning_efforts: vec![],
                    default_reasoning_effort: String::new(),
                    is_default: false,
                });
            }
        }
        Ok(Catalog {
            models,
            default_model: "gemini:default".into(),
            default_effort: String::new(),
        })
    })
    .await
    .context("Gemini model discovery timed out")?
}

fn model_name<'a>(inference: (&'a str, &str)) -> Result<&'a str> {
    ensure!(
        inference.1.is_empty(),
        "Gemini CLI manages reasoning effort automatically. Choose the default effort."
    );
    inference
        .0
        .strip_prefix("gemini:")
        .filter(|s| !s.is_empty())
        .context("Choose a Gemini model")
}

pub(super) fn connection(store: &Store, a: &Attempt, stderr: File) -> Result<Connection> {
    let skill = bundled_skills::game_ui_taste(&store.preferences_home)?;
    let skill_dir = skill.to_str().context("Invalid bundled skill path")?;
    if a.provider == "antigravity" {
        let mut io = spawn_cli(
            &store.work(a.id),
            &[
                "acp",
                "--stdio",
                "--dangerously-skip-permissions",
                "--add-dir",
                skill_dir,
            ],
            stderr,
            (antigravity::program()?, None),
            "Antigravity CLI",
        )?;
        io.gemini_bridge = Some(Bridge::new(store.worker, false)?);
        io.pinned_inference = Some((a.model.clone(), a.effort.clone()));
        return Ok(io);
    }
    let model = model_name((&a.model, &a.effort))?;
    let mut args = vec![
        "--acp",
        "--approval-mode",
        "yolo",
        "--include-directories",
        skill_dir,
    ];
    let bridge = Bridge::new(store.worker, store.gemini_project_trusted())?;
    if bridge.project_trusted
        && std::env::var("GEMINI_CLI_TRUST_WORKSPACE").as_deref() != Ok("false")
    {
        args.push("--skip-trust");
    }
    let policy = bridge.policy();
    args.extend([
        "--policy",
        policy.to_str().context("Invalid Gemini policy path")?,
    ]);
    if model != "default" {
        args.extend(["--model", model]);
    }
    let mut io = spawn(&store.work(a.id), &args, stderr)?;
    io.gemini_bridge = Some(bridge);
    io.pinned_inference = Some((a.model.clone(), a.effort.clone()));
    Ok(io)
}

pub(super) fn content(message: &Message) -> Result<Vec<Value>> {
    let mut blocks = Vec::new();
    for mut item in message
        .input()?
        .as_array_mut()
        .context("Invalid message")?
        .drain(..)
    {
        if let Some(path) = item["path"].as_str() {
            let mut bytes = Vec::new();
            File::open(path)?
                .take(10 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() <= 10 * 1024 * 1024,
                "ACP image attachments must be 10 MiB or smaller"
            );
            let mime = image::guess_format(&bytes)?.to_mime_type();
            item = json!({"type":"image","mimeType":mime,"data":STANDARD.encode(bytes)});
        }
        blocks.push(item);
    }
    Ok(blocks)
}

pub(super) async fn rpc(
    stdin: &mut ChildStdin,
    id: u32,
    method: &str,
    params: Value,
) -> Result<()> {
    send(
        stdin,
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
    )
    .await
}

pub(super) async fn initialize(stdin: &mut ChildStdin) -> Result<()> {
    rpc(
        stdin,
        1,
        "initialize",
        json!({"protocolVersion":1,"clientCapabilities":{},
        "clientInfo":{"name":"workshop","version":env!("CARGO_PKG_VERSION")}}),
    )
    .await
}

pub(super) async fn next(lines: &mut Frames) -> Result<Value> {
    let line = lines
        .next_line()
        .await?
        .context("Provider closed before replying. Check provider.stderr.log.")?;
    serde_json::from_str(&line).context("Invalid provider ACP event")
}

pub(super) fn result<'a>(msg: &'a Value, provider: &str) -> Result<&'a Value> {
    if let Some(e) = msg.get("error") {
        if e["code"] == -32000 {
            if provider == "Gemini CLI" {
                bail!(
                    "Gemini CLI needs authentication. For a personal Google account, choose an Antigravity model. For Gemini CLI, configure your API key or Vertex AI credentials."
                );
            }
            bail!(
                "{provider} needs authentication. Open Settings → Providers → {provider} to sign in."
            );
        }
        bail!(
            "{provider}: {}",
            e["message"].as_str().unwrap_or("Request failed")
        );
    }
    msg.get("result").context("Missing ACP result")
}

pub async fn sign_in() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut io = spawn(dir.path(), &["--acp"], tempfile::tempfile()?)?;
    tokio::time::timeout(Duration::from_secs(600), async {
        initialize(&mut io.stdin).await?;
        loop {
            let msg = next(&mut io.lines).await?;
            match msg["id"].as_u64() {
                Some(1) => {
                    let init = result(&msg, "Gemini CLI")?;
                    ensure!(
                        init["authMethods"]
                            .as_array()
                            .is_some_and(|ms| ms.iter().any(|m| m["id"] == "oauth-personal")),
                        "This Gemini CLI does not offer Google sign-in"
                    );
                    rpc(
                        &mut io.stdin,
                        2,
                        "authenticate",
                        json!({"methodId":"oauth-personal"}),
                    )
                    .await?;
                }
                Some(2) => {
                    if let Some(e) = msg.get("error") {
                        bail!("Google sign-in: {}", e["message"]);
                    }
                    return Ok(());
                }
                _ => {}
            }
        }
    })
    .await
    .context("Google sign-in timed out")?
}

pub(super) async fn studio_response(
    provider: &str,
    io: &mut Connection,
    id: u32,
    mut text: Option<&mut String>,
    image: bool,
) -> Result<Value> {
    loop {
        let msg = next(&mut io.lines).await?;
        if msg.get("method").is_none() && msg["id"] == id {
            return result(&msg, provider).cloned();
        }
        if msg.get("method").is_some() && msg.get("id").is_some() {
            let response = if msg["method"] == "session/request_permission" {
                let call = &msg["params"]["toolCall"];
                let allow = image
                    && msg["params"]["sessionId"] == io.session
                    && (call["name"] == "generate_image"
                        || (call["name"] == "invoke_subagent"
                            && call["rawInput"]["TypeName"] == "image-generator"));
                let option = msg["params"]["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|o| o["kind"] == if allow { "allow_once" } else { "reject_once" });
                let outcome = option.map_or_else(
                    || json!({"outcome":"cancelled"}),
                    |o| json!({"outcome":"selected","optionId":o["optionId"]}),
                );
                json!({"result":{"outcome":outcome}})
            } else {
                json!({"error":{"code":-32601,"message":"Unsupported client request"}})
            };
            let mut frame = response;
            frame["id"] = msg["id"].clone();
            frame["jsonrpc"] = json!("2.0");
            send(&mut io.stdin, frame).await?;
        } else if msg["method"] == "session/update"
            && msg["params"]["sessionId"] == io.session
            && msg["params"]["update"]["sessionUpdate"] == "agent_message_chunk"
            && let Some(buffer) = text.as_deref_mut()
            && let Some(chunk) = msg["params"]["update"]["content"]["text"].as_str()
        {
            ensure!(
                buffer.len() + chunk.len() <= 8 * 1024 * 1024,
                "Studio response exceeds 8 MiB"
            );
            buffer.push_str(chunk);
        }
    }
}

pub(super) async fn studio(
    dir: &Path,
    instructions: &str,
    prompt: &Message,
    schema: Value,
    inference: &(String, String),
) -> Result<Value> {
    let policy_dir = tempfile::tempdir()?;
    let policy = policy_dir.path().join("studio.toml");
    std::fs::write(
        &policy,
        "[[rule]]\ntoolName = \"*\"\ndecision = \"deny\"\npriority = 999\n",
    )?;
    let model = model_name((&inference.0, &inference.1))?;
    let mut input = format!("{instructions}\n");
    if !schema.is_null() {
        input.push_str(&format!(
            "Return only JSON matching this schema: {schema}\n"
        ));
    }
    input.push_str(&prompt.text);
    if !prompt.files.is_empty() {
        let mut args = vec![
            "--acp",
            "--policy",
            policy.to_str().context("Invalid policy path")?,
        ];
        if model != "default" {
            args.extend(["--model", model]);
        }
        let mut io = spawn(dir, &args, tempfile::tempfile()?)?;
        initialize(&mut io.stdin).await?;
        studio_response("Gemini CLI", &mut io, 1, None, false).await?;
        rpc(
            &mut io.stdin,
            2,
            "session/new",
            json!({"cwd":dir,"mcpServers":[]}),
        )
        .await?;
        let session = studio_response("Gemini CLI", &mut io, 2, None, false).await?;
        io.session = session["sessionId"]
            .as_str()
            .context("Missing Studio session")?
            .into();
        if model != "default" && session["models"]["currentModelId"] != model {
            rpc(
                &mut io.stdin,
                3,
                "session/set_model",
                json!({"sessionId":io.session,"modelId":model}),
            )
            .await?;
            studio_response("Gemini CLI", &mut io, 3, None, false).await?;
        }
        let visual = Message {
            text: input,
            files: prompt.files.clone(),
            queued_id: None,
            followup: false,
        };
        rpc(
            &mut io.stdin,
            4,
            "session/prompt",
            json!({"sessionId":io.session,"prompt":content(&visual)?}),
        )
        .await?;
        let mut text = String::new();
        let reply = studio_response("Gemini CLI", &mut io, 4, Some(&mut text), false).await?;
        ensure!(
            reply["stopReason"] == "end_turn",
            "Gemini ended its visual design request: {}",
            reply["stopReason"]
        );
        return studio_text(&text, &schema);
    }
    let mut args = vec![
        "-p",
        "Answer the request on stdin.",
        "--output-format",
        "json",
        "--policy",
        policy.to_str().context("Invalid policy path")?,
    ];
    if model != "default" {
        args.extend(["--model", model]);
    }
    let io = spawn(dir, &args, tempfile::tempfile()?)?;
    let Connection {
        mut stdin,
        lines,
        mut child,
        ..
    } = io;
    let mut out = Vec::new();
    let mut reader = lines.reader.take(8 * 1024 * 1024 + 1);
    let ((), _) = tokio::try_join!(
        async {
            stdin.write_all(input.as_bytes()).await?;
            drop(stdin);
            Ok(())
        },
        reader.read_to_end(&mut out)
    )?;
    ensure!(
        out.len() <= 8 * 1024 * 1024,
        "Gemini Studio response exceeds 8 MiB"
    );
    let status = child.wait().await?;
    let result: Value = serde_json::from_slice(&out)
        .context("Gemini CLI did not return a Studio result. Check Gemini sign-in.")?;
    ensure!(
        status.success() && result.get("error").is_none(),
        "Gemini Studio: {}",
        result["error"]["message"]
    );
    let text = result["response"]
        .as_str()
        .context("Missing Gemini response")?;
    studio_text(text, &schema)
}

fn studio_text(text: &str, schema: &Value) -> Result<Value> {
    let text = text.trim();
    if schema.is_null() {
        return Ok(json!({"text":text}));
    }
    let json_text = text
        .strip_prefix("```json")
        .and_then(|s| s.trim().strip_suffix("```"))
        .unwrap_or(text);
    let object: Value =
        serde_json::from_str(json_text).context("Gemini returned invalid structured output")?;
    ensure!(object.is_object(), "Expected a JSON object from Gemini");
    Ok(json!({"text":object.to_string()}))
}

fn save_text(
    store: &Store,
    id: i64,
    text: &mut String,
    notify: &dyn Fn(ReplyUpdate<'_>),
) -> Result<()> {
    if !text.is_empty() {
        store.event(id, "agent/message", Some(text))?;
        text.clear();
        notify(ReplyUpdate::Stored);
    }
    Ok(())
}

async fn permission(stdin: &mut ChildStdin, msg: &Value) -> Result<()> {
    let options = msg["params"]["options"].as_array();
    let option = ["allow_once", "allow_always"].iter().find_map(|kind| {
        options?
            .iter()
            .find(|option| option["kind"].as_str() == Some(*kind))
    });
    let outcome = option.map_or_else(
        || json!({"outcome":"cancelled"}),
        |o| json!({"outcome":"selected","optionId":o["optionId"]}),
    );
    send(
        stdin,
        json!({"jsonrpc":"2.0","id":msg["id"],"result":{"outcome":outcome}}),
    )
    .await
}

pub(super) async fn exchange(
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
    let bridge = io
        .gemini_bridge
        .as_mut()
        .context("Missing Workshop bridge")?;
    let agy = a.provider == "antigravity";

    let blender = blender_mcp::configured(&store.preferences_home)?;
    let provider = if agy { "Antigravity" } else { "Gemini CLI" };
    let mut worker_inference: Option<(String, String)> = None;
    let mut first = content(prompt)?;
    let instructions = instructions(store, a)?;
    if instructions != bridge.instructions {
        first.insert(0, json!({"type":"text","text":instructions}));
        bridge.instructions = instructions;
    }
    if let Some(pref) = team_preference(store, a.id)? {
        first.push(json!({"type":"text","text":pref}));
    }
    let mut queued = VecDeque::from([(first, None::<Message>)]);
    let mut waiting = io.session.is_empty();
    let mut first_prompt = true;
    let mut thinking = false;
    if waiting {
        initialize(&mut io.stdin).await?;
    }
    let mut started = false;
    let mut cancelling = false;
    let mut stopped = HashSet::new();
    let mut repairs = Repairs::start(store, a)?;
    let mut blocker = None;
    let mut text = String::new();
    let mut cumulative = store.usage(a.id)?.unwrap_or_default();
    let mut context = store.context_usage(a)?;
    let mut clock = tokio::time::interval(Duration::from_millis(80));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let startup = tokio::time::sleep(Duration::from_secs(90));
    tokio::pin!(startup);
    type Dispatch<'a> =
        std::pin::Pin<Box<dyn std::future::Future<Output = Result<team::ToolOutput>> + 'a>>;
    let mut dispatch: Option<Dispatch<'_>> = None;
    loop {
        if !waiting
            && !started
            && let Some((blocks, direction)) = queued.pop_front()
        {
            rpc(
                &mut io.stdin,
                4,
                "session/prompt",
                json!({"sessionId":io.session,"prompt":blocks}),
            )
            .await?;
            thinking = false;
            notify(ReplyUpdate::Activity("Waiting for a reply…"));
            if first_prompt {
                prompt.record_followup(store, a, notify)?;
                if let Some((model, effort)) = worker_inference.take() {
                    notify(ReplyUpdate::WorkerInference(&model, &effort));
                }
                first_prompt = false;
            }
            if let Some(direction) = direction {
                direction.record_direction(store, a.id, notify)?;
            }
            started = true;
        }
        if started && !queued.is_empty() && !cancelling && dispatch.is_none() {
            send(&mut io.stdin,json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":io.session}})).await?;
            cancelling = true;
        }
        let msg = tokio::select! {
            biased;
            control = controls.recv() => {
                match control {
                    None | Some(Control::Cancel) => {
                        save_text(store, a.id, &mut text, notify)?;
                        return Ok(false);
                    }
                    Some(Control::Queue(message)) => store.queue_message(a.id, &message)?,
                    Some(Control::Steer(message)) => match content(&message) {
                        Ok(blocks) => {
                            queued.push_back((blocks, Some(store.stage_direction(a.id, message)?)));
                        }
                        Err(e) => store.direction_unavailable(a.id, &message, &e.to_string())?,
                    },
                    Some(Control::SteerWorker(id, direction)) => route_worker_direction(store, a.id, id, &direction, notify)?,
                    Some(Control::StopWorker(id)) => {
                        if store.stop_worker(id).is_ok()
                            && let Some(worker) = store.workers(a.id)?.into_iter().find(|w| w.id == id) {
                            stopped.insert(worker.assignment.key);
                        }
                    }
                }
                notify(ReplyUpdate::TranscriptChanged);
                continue;
            }
            _ = &mut startup, if waiting => bail!("{provider} did not open its session within 90 seconds. Check provider.stderr.log."),
            result = async { dispatch.as_mut().expect("active dispatch").await }, if dispatch.is_some() => {
                dispatch = None;
                bridge.reply(result)?;
                continue;
            }
            _ = clock.tick(), if started && dispatch.is_none() => {
                if let Some(call) = bridge.call()? {
                    let name = call["name"].as_str();
                    let args = &call["arguments"];
                    if retries_stopped(name, args, &stopped) {
                        bridge.reply(Err(anyhow::anyhow!("This assignment was stopped by the user.")))?;
                    } else {
                        io.used_tools = true;
                        if name == Some("workshop_blocked") && !store.worker {
                            blocker = args["reason"].as_str().map(str::to_owned);
                        }
                        let request = json!({"tool":name,"arguments":args});
                        dispatch = Some(Box::pin(async move {
                            team::dispatch_tool(store, a, &request, notify).await
                        }));
                    }
                }
                continue;
            }
            msg = next(&mut io.lines) => msg?,
        };
        if let Some(method) = msg["method"].as_str() {
            if msg.get("id").is_some() {
                if method == "session/request_permission"
                    && msg["params"]["sessionId"] == io.session
                {
                    permission(&mut io.stdin, &msg).await?;
                } else {
                    send(&mut io.stdin,json!({"jsonrpc":"2.0","id":msg["id"],"error":{"code":-32601,"message":"Unsupported client request"}})).await?;
                }
            } else if method == "session/update"
                && started
                && msg["params"]["sessionId"] == io.session
            {
                let u = &msg["params"]["update"];
                // session/load replays history before its response. Only record the live turn.
                match u["sessionUpdate"].as_str() {
                    Some("agent_thought_chunk") => {
                        if !thinking {
                            thinking = true;
                            notify(ReplyUpdate::Activity("Thinking…"));
                        }
                        continue;
                    }
                    Some("agent_message_chunk") if let Some(s) = u["content"]["text"].as_str() => {
                        thinking = false;
                        if u["_meta"]["reset"] == true {
                            text.clear();
                            notify(ReplyUpdate::Stored);
                        }
                        text.push_str(s);
                        notify(ReplyUpdate::Delta(s));
                    }
                    Some("tool_call") => {
                        thinking = false;
                        save_text(store, a.id, &mut text, notify)?;
                        io.used_tools = true;
                        let title = u["title"].as_str().unwrap_or("Provider tool");
                        store.event(a.id, "item/started", Some(title))?;
                        let phase = if u["rawInput"]["TypeName"] == "image-generator"
                            || u["name"] == "generate_image"
                        {
                            "Generating an image…"
                        } else {
                            tool_activity(title)
                        };
                        notify(ReplyUpdate::Activity(phase));
                    }
                    Some("usage_update") => {
                        context = ContextUsage {
                            used: u["used"].as_i64(),
                            capacity: u["size"].as_i64(),
                        };
                        notify(ReplyUpdate::Context(context));
                    }
                    _ => continue,
                }
                writeln!(log, "{msg}")?;
                if stream {
                    println!("{msg}");
                }
            }
            continue;
        }
        let reply = result(&msg, provider)?;
        match msg["id"].as_u64() {
            Some(1) => {
                ensure!(
                    reply["protocolVersion"] == 1,
                    "Unsupported {provider} ACP protocol"
                );
                let mut servers = vec![bridge.config()?];
                if let Some(server) = &blender {
                    servers.push(server.acp());
                }
                let mut params = json!({"cwd":io.cwd,"mcpServers":servers});
                let method = if a.session.is_empty() {
                    "session/new"
                } else {
                    params["sessionId"] = json!(a.session);
                    "session/load"
                };
                rpc(&mut io.stdin, 2, method, params).await?;
            }
            Some(2) => {
                if !bridge.discovered() {
                    let notice = if agy {
                        "Antigravity could not connect to Workshop tools in this checkout. Check provider.stderr.log before dispatching workers."
                    } else {
                        "Gemini has disabled Workshop tools in this checkout. To use workers, trust this project in Settings → Providers → Gemini project tools. If it is already trusted, check the Gemini MCP policy."
                    };
                    if let Some((first, _)) = queued.front_mut() {
                        first.push(json!({"type":"text","text":format!("{notice} If asked to delegate, explain how to enable Workshop tools.")}));
                    }
                    if store.worker {
                        bail!("{notice}");
                    }
                }
                io.session = if a.session.is_empty() {
                    reply["sessionId"]
                        .as_str()
                        .context("Missing ACP session ID")?
                        .into()
                } else {
                    a.session.clone()
                };
                store.session(a.id, &io.session)?;
                let model = if agy {
                    antigravity::session_model(&reply["models"], &a.model, &a.effort)?
                } else {
                    Some(model_name((&a.model, &a.effort))?).filter(|&m| m != "default")
                };
                if store.worker
                    && let Some(raw) = model
                        .or_else(|| reply["models"]["currentModelId"].as_str())
                        .filter(|s| !s.is_empty())
                {
                    let reported_effort = if agy {
                        let name = reply["models"]["availableModels"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|m| m["modelId"] == raw)
                            .and_then(|m| m["name"].as_str())
                            .unwrap_or(raw);
                        antigravity::variant(raw, name).2
                    } else {
                        None
                    };
                    worker_inference = Some((
                        format!("{}:{raw}", a.provider),
                        reported_effort.unwrap_or(&a.effort).to_owned(),
                    ));
                }
                if let Some(model) = model
                    && reply["models"]["currentModelId"] != model
                {
                    rpc(
                        &mut io.stdin,
                        3,
                        "session/set_model",
                        json!({"sessionId":io.session,"modelId":model}),
                    )
                    .await?;
                } else {
                    waiting = false;
                }
            }
            Some(3) => waiting = false,
            Some(4) => {
                save_text(store, a.id, &mut text, notify)?;
                let reason = reply["stopReason"].as_str().unwrap_or("unknown");
                ensure!(
                    reason == "end_turn" || (cancelling && reason == "cancelled"),
                    "{provider} ended the turn: {reason}"
                );
                ensure!(
                    dispatch.is_none(),
                    "{provider} ended during a Workshop tool call"
                );
                if let Some(tokens) = reply["usage"]["totalTokens"].as_i64() {
                    cumulative += tokens;
                    store.record_usage(a.id, cumulative, context)?;
                    notify(ReplyUpdate::Usage(cumulative));
                }
                if let Some(why) = blocker.take() {
                    bail!("Task blocked: {why}");
                }
                started = false;
                cancelling = false;
                if queued.is_empty() {
                    match repairs
                        .verify(
                            store,
                            a,
                            controls,
                            notify,
                            io.used_tools,
                            !store.workers(a.id)?.is_empty(),
                            !stopped.is_empty(),
                            None,
                        )
                        .await?
                    {
                        Finish::Done => return Ok(true),
                        Finish::Cancelled => return Ok(false),
                        Finish::Continue(m) => queued.push_back((content(&m)?, Some(m))),
                    }
                } else {
                    repairs.reset_goal_reports(store, a.id)?;
                }
            }
            _ => {}
        }
    }
}
