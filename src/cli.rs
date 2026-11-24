use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{
    io::{self, BufRead, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use workshop::{
    Attempt, Store, data_dir,
    provider::{self, Control},
    studio::{Recipe, reports::Archive},
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
    /// Create game assets or inspect a local binary.
    Studio {
        #[command(subcommand)]
        action: StudioAction,
    },
    List,
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
        #[arg(long)]
        parallelism: Option<usize>,
        #[arg(long)]
        check_args: Option<String>,
    },
    /// Stream provider JSON. Type /cancel or a steering message on stdin.
    Run {
        prompt: String,
        #[arg(long, default_value = "")]
        model: String,
    },
    Retry {
        id: i64,
    },
    Resume {
        id: i64,
        #[arg(default_value = "Continue the task and verify the result.")]
        direction: String,
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
#[derive(clap::Args)]
struct StudioAgentOptions {
    /// Select a single Codex model for this run.
    #[arg(long)]
    model: Option<String>,
    /// Maximum Codex reasoning effort for this run.
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
        let mut value = workshop::studio::settings::TaskSettings::defaults(&home, area)?;
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
                "Enable --summary to use a Codex model for a decompilation."
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
fn run(store: &Store, a: Attempt, prompt: &str) -> Result<()> {
    eprintln!("Attempt {} in {}", a.id, store.work(a.id).display());
    let (tx, rx) = provider::channel();
    let input = tx.clone();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let c = if line.trim() == "/cancel" {
                Control::Cancel
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
fn main() -> Result<()> {
    let args = Args::parse();
    if let Action::Studio { action } = args.command {
        return studio(&args.project, action);
    }
    let store = Store::open(&args.project, &data_dir()?)?;
    match args.command {
        Action::Studio { .. } => unreachable!(),
        Action::Agents { id } => {
            println!("{}", serde_json::to_string_pretty(&store.workers(id)?)?);
            println!("{}", store.handoff(id)?);
        }
        Action::Team {
            lead_model,
            worker_model,
            lead_effort,
            worker_effort,
            parallelism,
            check_args,
        } => {
            let mut s = store.team_settings()?;
            if let Some(v) = lead_model {
                s.lead_model = v;
            }
            if let Some(v) = worker_model {
                s.worker_model = v;
            }
            if let Some(v) = lead_effort {
                s.lead_effort = v;
            }
            if let Some(v) = worker_effort {
                s.worker_effort = v;
            }
            if let Some(v) = parallelism {
                s.parallelism = v;
            }
            if let Some(v) = check_args {
                s.check_args = serde_json::from_str(&v)?;
            }
            store.configure_team(&s)?;
            println!("{}", serde_json::to_string_pretty(&s)?);
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
        Action::Run { prompt, model } => {
            run(&store, store.prepare(&prompt, &model, None)?, &prompt)?
        }
        Action::Retry { id } => {
            let a = store.attempt(id)?;
            run(
                &store,
                store.prepare(&a.prompt, &a.model, Some(id))?,
                &a.prompt,
            )?;
        }
        Action::Resume { id, direction } => run(&store, store.resume(id)?, &direction)?,
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
