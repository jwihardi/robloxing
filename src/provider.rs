use crate::{Attempt, Store};
use anyhow::{Context, Result, bail, ensure};
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::ChildStdin,
    sync::mpsc,
    time::Instant,
};

pub enum Control {
    Steer(String),
    Cancel,
}
pub type Controls = mpsc::UnboundedReceiver<Control>;
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
fn input(text: &str) -> Value {
    json!([{"type":"text","text":text}])
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
        "item/completed" if p["item"]["type"] == "agentMessage" => p["item"]["text"].as_str()?,
        "turn/started" => "Working in the isolated copy",
        "error" => p["error"]["message"].as_str()?,
        _ => return None,
    };
    Some(s.chars().take(500).collect())
}

async fn exchange(
    store: &Store,
    a: &Attempt,
    prompt: &str,
    mut controls: Controls,
    stream: bool,
) -> Result<bool> {
    let folder = store.folder(a.id);
    let mut log = append(&folder.join("events.jsonl"))?;
    let stderr = append(&folder.join("provider.stderr.log"))?;
    store.artifact(a.id, "events", "events.jsonl")?;
    store.artifact(a.id, "stderr", "provider.stderr.log")?;
    let exe = std::env::var("WORKSHOP_CODEX").unwrap_or_else(|_| "codex".into());
    let mut child = managed(
        &exe,
        &["app-server", "--listen", "stdio://"],
        &store.work(a.id),
        Stdio::piped(),
        stderr,
    )?;
    let mut stdin = child.stdin().take().context("Provider stdin")?;
    let stdout = child.stdout().take().context("Provider stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    send(&mut stdin,json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"workshop","title":"Workshop","version":env!("CARGO_PKG_VERSION")}}})).await?;
    let mut session = String::new();
    let mut turn = String::new();
    let mut next_id = 10;
    let mut cancelling = false;
    let mut deadline = Some(Instant::now() + Duration::from_secs(90));
    enum Next {
        Deadline,
        Input(Option<Control>),
        Line(Option<String>),
    }
    loop {
        let timeout = async {
            match deadline {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        let next = tokio::select! {
            _ = timeout => Next::Deadline,
            control = controls.recv() => Next::Input(control),
            line = lines.next_line() => Next::Line(line?),
        };
        match next {
            Next::Deadline => {
                if cancelling {
                    return Ok(false);
                }
                bail!("Codex did not start a turn within 90 seconds. Check provider.stderr.log.");
            }
            Next::Input(control) => match control {
                None | Some(Control::Cancel) => {
                    if turn.is_empty() || cancelling {
                        return Ok(false);
                    }
                    send(&mut stdin,json!({"id":4,"method":"turn/interrupt","params":{"threadId":session,"turnId":turn}})).await?;
                    cancelling = true;
                    deadline = Some(Instant::now() + Duration::from_secs(3));
                }
                Some(Control::Steer(text)) => {
                    if turn.is_empty() || cancelling {
                        store.event(
                            a.id,
                            "steer/unavailable",
                            Some("Wait for an active turn before sending direction."),
                        )?;
                        continue;
                    }
                    send(&mut stdin,json!({"id":next_id,"method":"turn/steer","params":{"threadId":session,"expectedTurnId":turn,"input":input(&text)}})).await?;
                    next_id += 1;
                    store.event(a.id, "steer/sent", Some(&text))?;
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
                    if id.is_some_and(|id| id >= 10) {
                        store.event(a.id, "steer/error", v["error"]["message"].as_str())?;
                        continue;
                    }
                    bail!("Codex: {}", v["error"]["message"]);
                }
                if let Some(method) = v["method"].as_str() {
                    if !v["id"].is_null() {
                        send(&mut stdin,json!({"id":v["id"],"error":{"code":-32601,"message":"Unsupported client request"}})).await?;
                        continue;
                    }
                    if !method.ends_with("delta") && !method.ends_with("Delta") {
                        let kind = if method == "item/completed"
                            && v["params"]["item"]["type"] == "agentMessage"
                        {
                            "agent/message"
                        } else {
                            method
                        };
                        store.event(a.id, kind, label(&v).as_deref())?;
                    }
                    if method == "turn/completed"
                        && v["params"]["threadId"] == session
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
                        return Ok(true);
                    }
                } else {
                    match id {
                        Some(1) => {
                            send(&mut stdin, json!({"method":"initialized"})).await?;
                            let mut params = json!({"cwd":store.work(a.id),"approvalPolicy":"never","sandbox":"workspace-write",
                                "developerInstructions":"Work only in the current project copy. Keep edits available for the creator to review."});
                            if !a.model.is_empty() {
                                params["model"] = json!(a.model);
                            }
                            let method = if a.session.is_empty() {
                                "thread/start"
                            } else {
                                params["threadId"] = json!(a.session);
                                "thread/resume"
                            };
                            send(&mut stdin, json!({"id":2,"method":method,"params":params}))
                                .await?;
                        }
                        Some(2) => {
                            session = v["result"]["thread"]["id"]
                                .as_str()
                                .context("Codex did not return a session ID")?
                                .into();
                            store.session(a.id, &session)?;
                            send(&mut stdin,json!({"id":3,"method":"turn/start","params":{"threadId":session,"input":input(prompt)}})).await?;
                        }
                        Some(3) => {
                            turn = v["result"]["turn"]["id"]
                                .as_str()
                                .context("Codex did not return a turn ID")?
                                .into();
                            deadline = None;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

pub fn run(
    store: &Store,
    a: &Attempt,
    prompt: &str,
    controls: Controls,
    stream: bool,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(exchange(store, a, prompt, controls, stream));
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
