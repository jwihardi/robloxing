use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
};
use workshop::{
    Attempt, Store, data_dir,
    provider::{self, Control},
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
    let store = Store::open(&args.project, &data_dir()?)?;
    match args.command {
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
