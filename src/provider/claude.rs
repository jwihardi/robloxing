use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::{HashSet, VecDeque};

const STREAM: &[&str] = &[
    "-p",
    "--verbose",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
];
const BRIDGE: &str = r#"{"mcpServers":{"workshop":{"type":"sdk","name":"workshop"}}}"#;

fn spawn(cwd: &Path, args: &[&str], stderr: File) -> Result<Connection> {
    let program = std::env::var("WORKSHOP_CLAUDE").unwrap_or_else(|_| "claude".into());
    let mut child = managed(&program, args, cwd, Stdio::piped(), stderr)?;
    let stdin = child.stdin().take().context("Claude stdin")?;
    let stdout = child.stdout().take().context("Claude stdout")?;
    Ok(Connection {
        child,
        stdin,
        lines: Frames {
            reader: BufReader::new(stdout),
            pending: Vec::new(),
        },
        cwd: cwd.to_owned(),
        session: String::new(),
        used_tools: false,
        pinned_inference: None,
        gemini_bridge: None,
    })
}

pub(super) fn connection(store: &Store, a: &Attempt, stderr: File) -> Result<Connection> {
    let instructions = instructions(store, a)?;
    let mut args = STREAM.to_vec();
    args.extend([
        "--include-partial-messages",
        "--permission-mode",
        "acceptEdits",
        "--disallowedTools",
        "Agent,Task,Workflow,EnterWorktree,ExitWorktree",
        "--allowedTools",
        "mcp__workshop",
        "--mcp-config",
        BRIDGE,
        "--append-system-prompt",
        &instructions,
    ]);
    add_inference(&mut args, &a.model, &a.effort)?;
    if !a.session.is_empty() {
        args.extend(["--resume", &a.session]);
    }
    let mut io = spawn(&store.work(a.id), &args, stderr)?;
    io.pinned_inference = Some((a.model.clone(), a.effort.clone()));
    Ok(io)
}

fn add_inference<'a>(args: &mut Vec<&'a str>, model: &'a str, effort: &'a str) -> Result<()> {
    let model = model
        .strip_prefix("claude:")
        .filter(|m| !m.is_empty())
        .context("Choose a Claude model")?;
    // The catalog's "default" clears overrides. Omitting --model can pick ANTHROPIC_MODEL instead.
    args.extend(["--model", model]);
    if !effort.is_empty() {
        args.extend(["--effort", effort]);
    }
    Ok(())
}

fn init(servers: &[&str]) -> Value {
    json!({"type":"control_request","request_id":"workshop-init","request":{"subtype":"initialize","sdkMcpServers":servers}})
}

/// Human-readable family and version for Anthropic IDs such as claude-haiku-4-5-20251001.
/// Gateway deployment names are opaque and should be shown as supplied.
fn versioned_name(id: &str) -> Option<(&str, String)> {
    let (family, version) = id.strip_prefix("claude-")?.split_once('-')?;
    if family.is_empty() || !family.bytes().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let version = match version.rsplit_once('-') {
        Some((version, date)) if date.len() == 8 && date.bytes().all(|c| c.is_ascii_digit()) => {
            version
        }
        _ => version,
    };
    if !version
        .split('-')
        .all(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    let mut title = format!("{family} {}", version.replace('-', "."));
    title[..1].make_ascii_uppercase();
    Some((family, title))
}

pub(super) fn model_label(value: &str, display: Option<&str>, resolved: Option<&str>) -> String {
    let id = resolved.filter(|s| !s.is_empty()).unwrap_or(value);
    let base = id.strip_suffix("[1m]").unwrap_or(id);
    let short_value = value.strip_suffix("[1m]").unwrap_or(value);
    let display = display.filter(|s| !s.is_empty());
    let name = display.unwrap_or(short_value);
    let name = name.strip_prefix("Claude ").unwrap_or(name);
    let mut title = if let Some((family, title)) = versioned_name(base) {
        if value == "default" {
            format!("Default · {title}")
        } else if name.eq_ignore_ascii_case(family) || name == base {
            title
        } else if name.starts_with(&title) {
            name.to_owned()
        } else {
            format!("{name} · {title}")
        }
    } else if value == "default" {
        if base == "default" {
            "Default".into()
        } else {
            format!("Default · {base}")
        }
    } else if base != short_value && base != name {
        format!("{name} · {base}")
    } else if display.is_none()
        && short_value.bytes().all(|c| c.is_ascii_lowercase())
        && !short_value.is_empty()
    {
        let mut name = short_value.to_owned();
        name[..1].make_ascii_uppercase();
        name
    } else {
        name.to_owned()
    };
    if (value.ends_with("[1m]") || id.ends_with("[1m]"))
        && !title.to_ascii_lowercase().contains("1m")
    {
        title.push_str(" (1M)");
    }
    format!("Claude {title}")
}

pub(super) async fn models(project: &Path) -> Result<Vec<CatalogModel>> {
    let mut args = STREAM.to_vec();
    args.extend(["--strict-mcp-config", "--no-session-persistence"]);
    let mut io = spawn(project, &args, tempfile::tempfile()?)?;
    send(&mut io.stdin, init(&[])).await?;
    while let Some(line) = io.lines.next_line().await? {
        let event: Value = serde_json::from_str(&line)?;
        if event["response"]["request_id"] == "workshop-init" {
            return parse_models(&event["response"]["response"]["models"]);
        }
    }
    bail!("Claude closed during model discovery")
}

fn parse_models(models: &Value) -> Result<Vec<CatalogModel>> {
    models
        .as_array()
        .context("Claude did not return a model catalog")?
        .iter()
        .map(|entry| {
            let value = entry["value"]
                .as_str()
                .filter(|s| !s.is_empty())
                .context("Missing Claude model name")?;
            let levels = entry["supportedEffortLevels"]
                .as_array()
                .filter(|_| entry["supportsEffort"].as_bool() != Some(false))
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|level| catalog::Effort {
                    reasoning_effort: level.into(),
                })
                .collect();
            Ok(CatalogModel {
                model: format!("claude:{value}"),
                display_name: entry["displayName"].as_str().map(str::to_owned),
                resolved_model: entry["resolvedModel"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                supported_reasoning_efforts: levels,
                default_reasoning_effort: String::new(),
                is_default: value == "default",
            })
        })
        .collect()
}

fn user_message(message: &Message, preference: Option<&str>) -> Result<Value> {
    let input = message.input()?;
    let mut content = Vec::new();
    for item in input.as_array().context("Missing input items")? {
        if let Some(path) = item["path"].as_str() {
            let media_type = match Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                _ => "image/png",
            };
            let mut bytes = Vec::new();
            File::open(path)?
                .take(5 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() <= 5 * 1024 * 1024,
                "Claude accepts attached images up to 5 MiB each"
            );
            content.push(json!({"type":"image","source":{"type":"base64","media_type":media_type,"data":STANDARD.encode(bytes)}}));
        } else {
            content.push(item.clone());
        }
    }
    if let Some(preference) = preference {
        content.push(json!({"type":"text","text":preference}));
    }
    Ok(
        json!({"type":"user","message":{"role":"user","content":content},"parent_tool_use_id":null,"session_id":""}),
    )
}

async fn answer(stdin: &mut ChildStdin, id: &Value, data: Value) -> Result<()> {
    send(stdin,json!({"type":"control_response","response":{"subtype":"success","request_id":id,"response":data}})).await
}

async fn mcp_reply(stdin: &mut ChildStdin, id: &Value, rpc: &Value, result: Value) -> Result<()> {
    let mut message = json!({"jsonrpc":"2.0","result":result});
    if !rpc.is_null() {
        message["id"] = rpc.clone();
    }
    answer(stdin, id, json!({"mcp_response":message})).await
}

fn tool_result(result: Result<String>) -> Value {
    let error = result.is_err();
    json!({"isError":error,"content":[{"type":"text","text":result.unwrap_or_else(|e|format!("{e:#}"))}]})
}

fn tokens(usage: &Value) -> Option<i64> {
    usage.get("input_tokens")?.as_i64()?;
    Some(
        [
            "input_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
            "output_tokens",
        ]
        .iter()
        .filter_map(|k| usage[*k].as_i64())
        .sum(),
    )
}

fn result_error(event: &Value) -> String {
    event["result"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            event["errors"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| event["subtype"].as_str().unwrap_or("Unknown result").into())
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
    let preference = team_preference(store)?;
    let mut first = Some(user_message(prompt, preference)?);
    if io.session.is_empty() {
        send(&mut io.stdin, init(&["workshop"])).await?;
    } else {
        send(&mut io.stdin, first.take().unwrap()).await?;
        prompt.record_followup(store, a, notify)?;
    }
    let mut deadline = Some(Instant::now() + Duration::from_secs(90));
    let mut queued = VecDeque::new();
    let mut interrupt_requested = false;
    let mut stopped = HashSet::new();
    let mut repairs = Repairs::default();
    let mut blocker = None;
    let mut used = None;
    let mut model = String::new();
    let mut cumulative = store.usage(a.id)?.unwrap_or_default();
    let mut last_message = String::new();
    let mut tool_list = team::tools(store.worker);
    for tool in tool_list.as_array_mut().unwrap() {
        tool.as_object_mut().unwrap().remove("type");
    }
    type Dispatch<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + 'a>>;
    let mut pending: Option<(Value, Value, Dispatch<'_>)> = None;
    loop {
        if !queued.is_empty() && !interrupt_requested && pending.is_none() && deadline.is_none() {
            send(&mut io.stdin, json!({"type":"control_request","request_id":"workshop-steer","request":{"subtype":"interrupt"}})).await?;
            interrupt_requested = true;
        }
        let timeout = async {
            if let Some(end) = deadline {
                tokio::time::sleep_until(end).await
            } else {
                std::future::pending().await
            }
        };
        let completed_call = async {
            match &mut pending {
                Some((_, _, future)) => future.await,
                None => std::future::pending().await,
            }
        };
        let event = tokio::select! {
            biased;
            control = controls.recv() => {
                match control {
                    None | Some(Control::Cancel) => {
                        if pending.take().is_some() {
                            store.interrupt_workers(a.id, true)?;
                            notify(ReplyUpdate::WorkersChanged);
                        }
                        return Ok(false);
                    }
                    Some(Control::StopWorker(id)) => {
                        if store.stop_worker(id).is_ok()
                            && let Some(worker) = store.workers(a.id)?.into_iter().find(|w| w.id == id)
                        {
                            stopped.insert(worker.assignment.key);
                        }
                    }
                    Some(Control::Steer(message)) => {
                        match user_message(&message, preference) {
                            Ok(input) => {
                                queued.push_back(input);
                                store.message_event(a.id, "steer/sent", &message)?;
                            }
                            Err(error) => store.event(a.id, "steer/unavailable", Some(&error.to_string()))?,
                        }
                        notify(ReplyUpdate::TranscriptChanged);
                    }
                    Some(Control::Queue(message)) => {
                        store.queue_message(a.id, &message)?;
                        notify(ReplyUpdate::TranscriptChanged);
                    }
                    Some(Control::SteerWorker(id, text)) => route_worker_direction(store, a.id, id, &text, notify)?,
                }
                continue;
            }
            result = completed_call => {
                let (id, rpc, _) = pending.take().expect("completed Workshop tool");
                mcp_reply(&mut io.stdin, &id, &rpc, tool_result(result)).await?;
                continue;
            }
            _ = timeout => bail!("Claude did not start the turn within 90 seconds. See provider.stderr.log."),
            line = io.lines.next_line() => {
                let line = line?.context("Claude closed before finishing. See provider.stderr.log.")?;
                serde_json::from_str::<Value>(&line).context("Invalid Claude event")?
            }
        };
        let kind = event["type"].as_str().unwrap_or("");
        // The initialize response includes private CLI account metadata.
        if kind != "control_response" {
            writeln!(log, "{event}")?;
            if stream {
                println!("{event}");
            }
        }
        match kind {
            "control_response" if event["response"]["request_id"] == "workshop-init" => {
                ensure!(
                    event["response"]["subtype"] == "success",
                    "Claude initialization failed: {}",
                    event["response"]["error"]
                );
                if let Some(input) = first.take() {
                    send(&mut io.stdin, input).await?;
                    prompt.record_followup(store, a, notify)?;
                }
            }
            "control_request" => {
                let id = &event["request_id"];
                let req = &event["request"];
                if req["subtype"] != "mcp_message" || req["server_name"] != "workshop" {
                    send(&mut io.stdin,json!({"type":"control_response","response":{"subtype":"error","request_id":id,"error":"Unsupported Workshop control request"}})).await?;
                    continue;
                }
                let msg = &req["message"];
                let rpc = &msg["id"];
                let result = match msg["method"].as_str() {
                    Some("initialize") => {
                        json!({"protocolVersion":msg["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"workshop","version":env!("CARGO_PKG_VERSION")}})
                    }
                    Some("tools/list") => json!({"tools":tool_list}),
                    Some("ping" | "notifications/initialized") => json!({}),
                    Some("tools/call") => {
                        io.used_tools = true;
                        let name = msg["params"]["name"].as_str();
                        let args = &msg["params"]["arguments"];
                        if pending.is_some() {
                            tool_result(Err(anyhow::anyhow!(
                                "Wait for the current Workshop tool call to finish."
                            )))
                        } else if retries_stopped(name, args, &stopped) {
                            tool_result(Err(anyhow::anyhow!(
                                "This assignment was stopped by the user."
                            )))
                        } else {
                            if name == Some("workshop_blocked") && !store.worker {
                                blocker = args["reason"]
                                    .as_str()
                                    .filter(|s| !s.trim().is_empty())
                                    .map(str::to_owned);
                            }
                            let request = json!({"tool":name,"arguments":args});
                            pending = Some((
                                id.clone(),
                                rpc.clone(),
                                Box::pin(async move {
                                    team::dispatch(store, a, &request, notify).await
                                }),
                            ));
                            continue;
                        }
                    }
                    _ => {
                        answer(&mut io.stdin,id,json!({"mcp_response":{"jsonrpc":"2.0","id":rpc,"error":{"code":-32601,"message":"Unknown method"}}})).await?;
                        continue;
                    }
                };
                mcp_reply(&mut io.stdin, id, rpc, result).await?;
            }
            "system" if event["subtype"] == "init" => {
                let session = event["session_id"]
                    .as_str()
                    .context("Missing Claude session ID")?;
                ensure!(
                    a.session.is_empty() || a.session == session,
                    "Claude returned a different session"
                );
                io.session = session.into();
                store.session(a.id, session)?;
                deadline = None;
            }
            "stream_event" => {
                deadline = None;
                let e = &event["event"];
                if e["delta"]["type"] == "text_delta"
                    && let Some(text) = e["delta"]["text"].as_str()
                {
                    notify(ReplyUpdate::Delta(text));
                }
                if e["content_block"]["type"] == "tool_use"
                    && let Some(name) = e["content_block"]["name"].as_str()
                {
                    io.used_tools = true;
                    notify(ReplyUpdate::Activity(name));
                    store.event(a.id, "item/started", Some(name))?;
                }
            }
            "assistant" => {
                let message = &event["message"];
                used = tokens(&message["usage"]).or(used);
                if let Some(m) = message["model"].as_str() {
                    model = m.into();
                }
                let blocks = message["content"]
                    .as_array()
                    .context("Invalid Claude message")?;
                io.used_tools |= blocks.iter().any(|b| b["type"] == "tool_use");
                let text = blocks
                    .iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.is_empty() {
                    store.event(a.id, "agent/message", Some(&text))?;
                    notify(ReplyUpdate::Stored);
                    last_message = text;
                }
            }
            "rate_limit_event" => {
                let info = &event["rate_limit_info"];
                if let Some(window) = crate::usage::claude_window(info) {
                    notify(ReplyUpdate::AccountUsage("claude", vec![window]));
                }
            }
            "result" => {
                let redirected = interrupt_requested
                    && event["terminal_reason"]
                        .as_str()
                        .is_some_and(|why| why.starts_with("aborted"));
                interrupt_requested = false;
                if redirected {
                    notify(ReplyUpdate::Stored);
                }
                ensure!(
                    redirected || (event["is_error"] == false && event["subtype"] == "success"),
                    "Claude: {}",
                    result_error(&event)
                );
                ensure!(
                    pending.is_none(),
                    "Claude ended its turn during a Workshop tool call"
                );
                if let Some(text) = event["result"]
                    .as_str()
                    .filter(|t| !t.is_empty() && *t != last_message)
                {
                    store.event(a.id, "agent/message", Some(text))?;
                    notify(ReplyUpdate::Stored);
                }
                if let Some(total) = tokens(&event["usage"]) {
                    cumulative += total;
                    let context = ContextUsage {
                        used: event["usage"]["iterations"]
                            .as_array()
                            .and_then(|rows| rows.last())
                            .and_then(tokens)
                            .or(used),
                        capacity: event["modelUsage"][&model]["contextWindow"].as_i64(),
                    };
                    store.record_usage(a.id, cumulative, context)?;
                    notify(ReplyUpdate::Usage(cumulative));
                    notify(ReplyUpdate::Context(context));
                }
                if let Some(reason) = blocker.take() {
                    bail!("Task blocked: {reason}");
                }
                let input = if let Some(input) = queued.pop_front() {
                    input
                } else {
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
                        Finish::Continue(direction) => user_message(&direction, preference)?,
                    }
                };
                last_message.clear();
                send(&mut io.stdin, input).await?;
                deadline = Some(Instant::now() + Duration::from_secs(90));
            }
            _ => {}
        }
    }
}

pub(super) async fn studio(
    dir: &Path,
    instructions: &str,
    prompt: &str,
    schema: Value,
    inference: &(String, String),
) -> Result<Value> {
    let mut args = STREAM.to_vec();
    args.extend([
        "--no-session-persistence",
        "--strict-mcp-config",
        "--tools",
        "",
        "--append-system-prompt",
        instructions,
    ]);
    let schema_json = schema.to_string();
    if !schema.is_null() {
        args.extend(["--json-schema", &schema_json]);
    }
    add_inference(&mut args, &inference.0, &inference.1)?;
    let mut io = spawn(dir, &args, tempfile::tempfile()?)?;
    send(&mut io.stdin, user_message(&Message::from(prompt), None)?).await?;
    while let Some(line) = io.lines.next_line().await? {
        let event: Value = serde_json::from_str(&line)?;
        if event["type"] == "result" {
            ensure!(
                event["is_error"] == false,
                "Claude: {}",
                result_error(&event)
            );
            let text = if schema.is_null() {
                event["result"]
                    .as_str()
                    .context("Missing Claude response")?
                    .to_owned()
            } else {
                event
                    .get("structured_output")
                    .filter(|v| v.is_object())
                    .context("Missing structured Claude output")?
                    .to_string()
            };
            return Ok(json!({"text":text}));
        }
    }
    bail!("Claude closed before returning the Studio result")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_labels_follow_the_catalog_and_launch_uses_the_original_selection() -> Result<()> {
        let entries = json!([
            {"value":"default","displayName":"Default (recommended)","resolvedModel":"claude-opus-5[1m]"},
            {"value":"sonnet","displayName":"Sonnet","resolvedModel":"claude-sonnet-5-5","supportedEffortLevels":["medium","high"]},
            {"value":"haiku","displayName":"Haiku","resolvedModel":"claude-haiku-4-5-20251001","supportsEffort":false},
            {"value":"claude-fable-5[1m]","displayName":"Fable","resolvedModel":"claude-fable-5"},
            {"value":"internal/deploy","displayName":"Design Preview","resolvedModel":"internal/deploy"},
            {"value":"legacy","displayName":"Legacy Deployment"}
        ]);
        let catalog = Catalog {
            models: parse_models(&entries)?,
            ..Default::default()
        };
        for (entry, expected) in catalog.models.iter().zip([
            "Claude Default · Opus 5 (1M)",
            "Claude Sonnet 5.5",
            "Claude Haiku 4.5",
            "Claude Fable 5 (1M)",
            "Claude Design Preview",
            "Claude Legacy Deployment",
        ]) {
            assert_eq!(
                catalog.claude_label(&entry.model).as_deref(),
                Some(expected)
            );
            let mut args = Vec::new();
            add_inference(&mut args, &entry.model, "")?;
            assert_eq!(
                args,
                ["--model", entry.model.strip_prefix("claude:").unwrap()]
            );
        }
        assert_eq!(catalog.models[1].supported_reasoning_efforts.len(), 2);
        assert!(catalog.models[2].supported_reasoning_efforts.is_empty());
        assert_eq!(
            catalog.claude_label("claude:claude-sonnet-4-6").as_deref(),
            Some("Claude Sonnet 4.6")
        );
        Ok(())
    }
}
