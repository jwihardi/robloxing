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
            } else {
                Control::Steer(line)
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
