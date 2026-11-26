use crate::{
    Attempt, Store, data_dir,
    provider::{self, Control},
    studio::{Recipe, reports::Archive},
};
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{
    io::{self, BufRead, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(about = "Run a task, inspect its saved change, and apply it to your project.")]
struct Args {
    /// Existing Git root or a new project folder
    #[arg(long, default_value = ".")]
    project: PathBuf,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Open,
    /// Browse, publish and export project-wide Blender asset libraries.
    Packs {
        #[command(subcommand)]
        action: PackAction,
    },
    /// Manage the Blender MCP connection shared by the provider adapters.
    BlenderMcp {
        #[command(subcommand)]
        action: BlenderAction,
    },
    /// Trust this project's Gemini configuration in Workshop copies, or restore the CLI's default trust.
    GeminiTrust {
        #[arg(action=clap::ArgAction::Set)]
        enabled: bool,
    },
    /// List the installed provider's model IDs and supported effort levels.
    Models {
        #[arg(long,value_parser=crate::provider_permissions::PROVIDERS)]
        provider: Option<String>,
    },
    /// Create game assets or inspect a local binary.
    Studio {
        #[command(subcommand)]
        action: StudioAction,
    },
    List,
    /// Show actionable project work, including while the desktop has the project open.
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u16).range(1..=200))]
        limit: u16,
    },
    /// Show the workers and checkpoint for a lead conversation.
    Agents {
        id: i64,
    },
    /// Set project defaults. The optional check is a JSON array of executable and arguments.
    Team {
        #[arg(long)]
        lead_model: Option<String>,
        #[arg(long)]
        worker_model: Option<String>,
        #[arg(long)]
        lead_effort: Option<String>,
        #[arg(long)]
        worker_effort: Option<String>,
        /// Clear this project's worker model and effort overrides and follow the global defaults.
        #[arg(long, conflicts_with_all = ["worker_model", "worker_effort"])]
        inherit_workers: bool,
        #[arg(long)]
        parallelism: Option<usize>,
        /// Automatically assign divisible work. Set false to delegate only by request.
        #[arg(long)]
        auto_delegate: Option<bool>,
        #[arg(long)]
        check_args: Option<String>,
    },
    /// Stream provider JSON. Type /cancel or a steering message on stdin.
    Run {
        /// Use the selected CLI and its configured model unless --model is supplied.
        #[arg(long,value_parser=crate::provider_permissions::PROVIDERS)]
        provider: Option<String>,
        prompt: String,
        #[arg(long, default_value = "")]
        model: String,
        /// Reasoning effort for this turn (must be supported by the selected model).
        #[arg(long)]
        effort: Option<String>,
    },
    /// Continue a goal interrupted by process shutdown, in its original working copy.
    RecoverGoal {
        id: Option<i64>,
    },
    Retry {
        id: i64,
    },
    Resume {
        id: i64,
        #[arg(default_value = "Continue the task and verify the result.")]
        direction: String,
    },
    /// Send the first queued message and continue the queue after each successful run.
    SendNext {
        id: i64,
    },
    Diff {
        id: i64,
    },
    Accept {
        id: i64,
    },
    Discard {
        id: i64,
    },
    Check {
        id: i64,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
}
#[derive(Subcommand)]
enum PackAction {
    List,
    Show {
        pack: String,
        asset: Option<String>,
    },
    Create {
        pack: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        style: String,
    },
    Add {
        pack: String,
        asset: String,
        source: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        collection: Option<String>,
    },
    /// Write a portable Blender Asset Browser library into a new subfolder.
    Export {
        pack: String,
        folder: PathBuf,
    },
    /// Add an exported Workshop pack to this project's asset library.
    Import {
        folder: PathBuf,
    },
}

fn packs(project: &Path, action: PackAction) -> Result<()> {
    use crate::studio::packs::{Library, Publish};
    use serde_json::json;
    let home = data_dir()?;
    let library = Library::in_archive(&Store::project_data_dir(project, &home)?);
    let value = match action {
        PackAction::List => json!(library.list()?),
        PackAction::Show {
            pack,
            asset: Some(asset),
        } => {
            let dir = library.member_dir(&pack, &asset)?;
            json!({"pack":library.pack(&pack)?, "asset":library.member(&pack, &asset)?, "preview":dir.join("preview.png"),
                "blend":dir.join("model.blend"), "glb":dir.join("model.glb")})
        }
        PackAction::Show { pack, asset: None } => {
            json!({"pack":library.pack(&pack)?,"assets":library.members(&pack)?})
        }
        PackAction::Create { pack, name, style } => json!(library.create(&pack, &name, &style)?),
        PackAction::Add {
            pack,
            asset,
            source,
            name,
            description,
            collection,
        } => {
            let blender = Store::installed_studio_tool(&home, "Blender")?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            json!(runtime.block_on(library.publish(
                &blender,
                Publish {
                    pack: &pack,
                    key: &asset,
                    name: &name,
                    description: &description,
                    source: &source,
                    collection: collection.as_deref()
                }
            ))?)
        }
        PackAction::Export { pack, folder } => json!({"folder":library.export(&pack, &folder)?}),
        PackAction::Import { folder } => json!(library.import(&folder)?),
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

#[derive(Subcommand)]
enum BlenderAction {
    Status,
    /// Use the existing Codex blender entry or a local stdio executable.
    Connect {
        #[arg(long)]
        command: Option<String>,
        #[arg(long = "arg", requires = "command", allow_hyphen_values = true)]
        args: Vec<String>,
    },
    Test,
    Disconnect,
}

fn blender_mcp(action: BlenderAction) -> Result<()> {
    use provider::blender_mcp::{self, Server};
    let home = data_dir()?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            match action {
                BlenderAction::Status => match blender_mcp::configured(&home)? {
                    Some(server) => println!("Shared Blender MCP executable: {}", server.command),
                    None => println!("Blender MCP sharing is off."),
                },
                BlenderAction::Connect { command, args } => {
                    let server = match command {
                        Some(exe) => Server::new(exe, args),
                        None => Server::discover().await?,
                    };
                    println!("{}", server.connect(&home).await?);
                }
                BlenderAction::Test => {
                    let server = blender_mcp::configured(&home)?
                        .ok_or_else(|| anyhow::anyhow!("Connect Blender MCP first."))?;
                    println!("{}", server.test().await?);
                }
                BlenderAction::Disconnect => {
                    blender_mcp::disconnect(&home)?;
                    println!("Blender MCP sharing is off.");
                }
            }
            Ok(())
        })
}
#[derive(clap::Args)]
struct StudioAgentOptions {
    /// Select a single provider model for this run.
    #[arg(long)]
    model: Option<String>,
    /// Maximum reasoning effort for this run.
    #[arg(long)]
    max_effort: Option<String>,
}
#[derive(Subcommand)]
enum StudioAction {
    Prop {
        brief: String,
        #[command(flatten)]
        agent: StudioAgentOptions,
    },
    Image {
        brief: String,
        #[arg(long, default_value = "Concept art")]
        style: String,
        #[command(flatten)]
        agent: StudioAgentOptions,
    },
    Texture {
        material: String,
        #[arg(long, default_value_t = 1)]
        seed: u32,
    },
    Model {
        template: String,
        name: String,
    },
    Blend {
        source: PathBuf,
    },
    Decompile {
        binary: PathBuf,
        #[arg(long, default_value = "")]
        filter: String,
        /// Run Ghidra even when an identical archived report exists.
        #[arg(long)]
        fresh: bool,
        #[arg(long)]
        function_limit: Option<u16>,
        #[arg(long)]
        seconds_per_function: Option<u16>,
        /// Generate an AI summary, or use --summary=false to skip it.
        #[arg(long,num_args=0..=1,require_equals=true,default_missing_value="true")]
        summary: Option<bool>,
        #[command(flatten)]
        agent: StudioAgentOptions,
    },
    /// Search archived runs by binary, filter, fingerprint or function.
    Reports {
        #[arg(default_value = "")]
        query: String,
        #[arg(long)]
        before: Option<i64>,
    },
    /// Read saved pseudocode or logs. Offsets are bytes.
    ReadReport {
        id: i64,
        #[arg(long,default_value="report",value_parser=["report","functions","log","tool-log","summary"])]
        part: String,
        #[arg(long, default_value_t = 0)]
        offset: u64,
    },
    History {
        area: String,
    },
}
fn studio(project: &Path, action: StudioAction) -> Result<()> {
    let home = data_dir()?;
    let archive = || Archive::open(&Store::project_data_dir(project, &home)?);
    let fresh = matches!(action, StudioAction::Decompile { fresh: true, .. });
    let agent = match &action {
        StudioAction::Image { agent, .. } => Some(("Images", agent)),
        StudioAction::Prop { agent, .. } => Some(("Models", agent)),
        StudioAction::Decompile { agent, .. } => Some(("Ghidra", agent)),
        _ => None,
    };
    let mut settings = None;
    if let Some((area, agent)) = agent {
        let mut value = crate::studio::settings::TaskSettings::defaults(&home, area)?;
        if let StudioAction::Decompile {
            function_limit,
            seconds_per_function,
            summary,
            ..
        } = &action
        {
            if let Some(n) = function_limit {
                value.function_limit = *n;
            }
            if let Some(n) = seconds_per_function {
                value.seconds_per_function = *n;
            }
            if let Some(enabled) = summary {
                value.summarize = *enabled;
            }
            anyhow::ensure!(
                value.summarize || (agent.model.is_none() && agent.max_effort.is_none()),
                "Enable --summary to use an AI model for a decompilation."
            );
        }
        if let Some(model) = &agent.model {
            value.agent.models = vec![model.clone()];
        }
        if let Some(effort) = &agent.max_effort {
            value.agent.max_effort = effort.clone();
        }
        settings = Some(value);
    }
    let recipe = match action {
        StudioAction::Image { brief, style, .. } => Recipe::Image { brief, style },
        StudioAction::Texture { material, seed } => Recipe::Texture { material, seed },
        StudioAction::Model { template, name } => Recipe::Model { template, name },
        StudioAction::Prop { brief, .. } => Recipe::ModelBrief { brief },
        StudioAction::Blend { source } => Recipe::Blend {
            source: dunce::canonicalize(source)?,
        },
        StudioAction::Decompile { binary, filter, .. } => Recipe::Decompile {
            binary: dunce::canonicalize(binary)?,
            filter,
        },
        StudioAction::History { area } => {
            let store = Store::open(project, &home)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&store.studio_history(&area)?)?
            );
            return Ok(());
        }
        StudioAction::Reports { query, before } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&archive()?.search(&query, before)?)?
            );
            return Ok(());
        }
        StudioAction::ReadReport { id, part, offset } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&archive()?.read(id, &part, offset)?)?
            );
            return Ok(());
        }
    };
    let store = Store::open(project, &home)?;
    let id = store.queue_studio_job(&recipe, settings.as_ref())?;
    let (_keep, cancel) = tokio::sync::oneshot::channel();
    if recipe.area() == "Ghidra" {
        let path = store.studio_job(id)?.folder(&store).join("activity.log");
        std::thread::scope(|scope| -> Result<()> {
            let task = scope.spawn(|| store.run_studio_job_with_refresh(id, cancel, fresh));
            let mut offset = 0;
            loop {
                let finished = task.is_finished();
                if let Ok(mut file) = std::fs::File::open(&path) {
                    file.seek(SeekFrom::Start(offset))?;
                    offset += io::copy(&mut file, &mut io::stderr())?;
                }
                if finished {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            task.join()
                .map_err(|_| anyhow::anyhow!("The decompilation worker stopped unexpectedly."))?
        })?;
    } else {
        store.run_studio_job(id, cancel)?;
    }
    let job = store.studio_job(id)?;
    println!(
        "{}",
        serde_json::json!({"id":id,"title":job.title,"detail":job.detail,"folder":job.folder(&store),"file":job.filename()})
    );
    Ok(())
}
fn run(store: &Store, a: Attempt, prompt: provider::Message) -> Result<()> {
    eprintln!("Attempt {} in {}", a.id, store.work(a.id).display());
    let (tx, rx) = provider::channel();
    let input = tx.clone();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            if crate::goals::command(line.strip_prefix("/queue ").unwrap_or(&line)).is_some() {
                eprintln!("Use /cancel, then resume this chat with an updated /goal objective.");
                continue;
            }
            let c = if line.trim() == "/cancel" {
                Control::Cancel
            } else if let Some(text) = line.strip_prefix("/queue ") {
                if text.trim().is_empty() {
                    continue;
                }
                Control::Queue(text.into())
            } else if let Some(direction) = line.strip_prefix("/steer-worker ") {
                let Some((id, text)) = direction
                    .trim()
                    .split_once(' ')
                    .and_then(|(id, text)| Some((id.parse().ok()?, text)))
                else {
                    eprintln!("Use /steer-worker ID MESSAGE");
                    continue;
                };
                Control::SteerWorker(id, text.into())
            } else if let Some(id) = line
                .strip_prefix("/stop-worker ")
                .and_then(|id| id.trim().parse().ok())
            {
                Control::StopWorker(id)
            } else {
                Control::Steer(line.as_str().into())
            };
            if input.send(c).is_err() {
                break;
            }
        }
    });
    let result = provider::run(store, &a, prompt, rx, true);
    drop(tx);
    result
}

fn list_models(project: &Path, provider: Option<&str>) -> Result<()> {
    let project = dunce::canonicalize(project)?;
    anyhow::ensure!(project.is_dir(), "Choose a folder for model discovery");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let catalog = runtime.block_on(provider::worker_catalog(&project, provider))?;
    for model in catalog.models {
        let efforts = model
            .supported_reasoning_efforts
            .iter()
            .map(|e| e.reasoning_effort.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        if let Some(resolved) = &model.resolved_model {
            println!("{}  {}  {}", model.model, efforts, resolved);
        } else {
            println!("{}  {}", model.model, efforts);
        }
    }
    Ok(())
}

pub fn execute(args: impl Iterator<Item = std::ffi::OsString>) -> Result<()> {
    let args = Args::parse_from(args);
    match args.command {
        Action::BlenderMcp { action } => return blender_mcp(action),
        Action::Packs { action } => return packs(&args.project, action),
        Action::Studio { action } => return studio(&args.project, action),
        Action::Models { provider } => return list_models(&args.project, provider.as_deref()),
        Action::Status {
            json,
            offset,
            limit,
        } => {
            let rows =
                crate::work_queue::read(&args.project, &data_dir()?, offset, usize::from(limit))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else if rows.is_empty() {
                println!("No open work on this page.");
            } else {
                for row in rows {
                    println!(
                        "#{}  {}  {}  {}\n    {}",
                        row.id, row.status, row.provider, row.title, row.next_action
                    );
                }
            }
            return Ok(());
        }
        _ => {}
    }
    let store = Store::open(&args.project, &data_dir()?)?;
    match args.command {
        Action::Studio { .. }
        | Action::Packs { .. }
        | Action::Models { .. }
        | Action::Status { .. }
        | Action::BlenderMcp { .. } => unreachable!(),
        Action::GeminiTrust { enabled } => {
            store.set_gemini_project_trusted(enabled)?;
            println!("Gemini project trust override: {enabled}");
        }
        Action::Agents { id } => {
            println!("{}", serde_json::to_string_pretty(&store.workers(id)?)?);
            println!("{}", store.handoff(id)?);
        }
        Action::Team {
            lead_model,
            worker_model,
            lead_effort,
            worker_effort,
            inherit_workers,
            parallelism,
            auto_delegate,
            check_args,
        } => {
            let mut s = store.team_settings()?;
            if let Some(v) = lead_model {
                s.lead_model = v;
            }
            if let Some(v) = worker_model {
                s.worker_model = v;
                s.override_worker_model = true;
            }
            if let Some(v) = lead_effort {
                s.lead_effort = v;
            }
            if let Some(v) = worker_effort {
                s.worker_effort = v;
                s.override_worker_effort = true;
            }
            if inherit_workers {
                s.worker_model.clear();
                s.worker_effort.clear();
                s.override_worker_model = false;
                s.override_worker_effort = false;
            }
            if let Some(v) = auto_delegate {
                s.auto_delegate = v;
            }
            if let Some(v) = parallelism {
                s.parallelism = v;
            }
            if let Some(v) = check_args {
                s.check_args = serde_json::from_str(&v)?;
            }
            store.configure_team(&s)?;
            let mut result = serde_json::to_value(&s)?;
            result["worker_policy"] = serde_json::to_value(store.worker_defaults()?)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Action::Open => println!(
            "Opened {}\nRecords: {}",
            store.project.display(),
            store.dir.display()
        ),
        Action::List => {
            for a in store.attempts()? {
                println!("#{}  {}  {}\n    {}", a.id, a.state, a.prompt, a.action);
            }
        }
        Action::Run {
            prompt,
            model,
            provider,
            effort,
        } => {
            let model = match provider.as_deref() {
                Some(source) if model.contains(':') && provider::provider_for(&model) != source => {
                    anyhow::bail!("Choose a {source} model for --provider {source}")
                }
                Some(source @ ("claude" | "gemini" | "antigravity")) if model.is_empty() => {
                    format!("{source}:default")
                }
                Some(source @ ("claude" | "gemini" | "antigravity")) if !model.contains(':') => {
                    format!("{source}:{model}")
                }
                Some("codex")
                    if model.is_empty()
                        && provider::provider_for(&store.team_settings()?.lead_model)
                            != "codex" =>
                {
                    let a = store.prepare_inference(
                        &prompt,
                        "",
                        effort.as_deref().unwrap_or_default(),
                        None,
                        None,
                        &[],
                    )?;
                    let message = a.prompt.as_str().into();
                    return run(&store, a, message);
                }
                _ => model,
            };
            let a = if let Some(effort) = effort {
                let model = if model.is_empty() {
                    store.team_settings()?.lead_model
                } else {
                    model
                };
                store.prepare_inference(&prompt, &model, &effort, None, None, &[])?
            } else {
                store.prepare(&prompt, &model, None)?
            };
            let message = a.prompt.as_str().into();
            run(&store, a, message)?
        }
        Action::RecoverGoal { id } => {
            if let Some((attempt, prompt)) = store.recover_goal(id)? {
                run(&store, attempt, prompt.as_str().into())?;
            } else {
                println!("No interrupted goal is ready to recover.");
            }
        }
        Action::Retry { id } => {
            let (attempt, message) = store.retry_chat(id)?;
            run(&store, attempt, message)?;
        }
        Action::Resume { id, direction } => {
            let goal = crate::goals::command(&direction);
            if goal == Some("") {
                store.attempt(id)?;
                if let Some(goal) = store.chat_goal(id)? {
                    println!("{} · {}\n{}", goal.status, goal.objective, goal.detail);
                } else {
                    println!("No goal set for this chat.");
                }
            } else {
                let (attempt, message) =
                    store.resume_chat(id, goal.unwrap_or(&direction).into(), goal)?;
                run(&store, attempt, message)?;
            }
        }
        Action::SendNext { id } => {
            let (attempt, message) = store
                .resume_queued(id)?
                .ok_or_else(|| anyhow::anyhow!("The queue is empty."))?;
            run(&store, attempt, message)?;
        }
        Action::Diff { id } => io::stdout().write_all(&store.patch(&store.attempt(id)?)?)?,
        Action::Accept { id } => {
            store.accept(id)?;
            println!("Accepted #{id}");
        }
        Action::Discard { id } => {
            store.discard(id)?;
            println!("Discarded #{id}");
        }
        Action::Check { id, command } => println!("{}", store.check(id, &command)?),
    }
    Ok(())
}
