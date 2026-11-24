//! Run in a disposable copy of the fixture. stdout is Claude's original JSONL.
use anyhow::{Result, ensure};
use clap::Parser;
use std::{fs::File, path::PathBuf, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};
#[derive(Parser)]
struct Probe {
    #[arg(long)]
    copy: PathBuf,
    #[arg(long, default_value = "sonnet")]
    model: String,
    #[arg(long)]
    resume: Option<String>,
    #[arg(long)]
    cancel_after: Option<u64>,
    prompt: String,
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let p = Probe::parse();
    let mut args = vec![
        "-p",
        "--verbose",
        "--output-format",
        "stream-json",
        "--model",
        &p.model,
        "--permission-mode",
        "acceptEdits",
        "--tools",
        "Read,Edit,Write",
        "--allowedTools",
        "Read,Edit,Write",
    ];
    if let Some(session) = &p.resume {
        args.extend(["--resume", session]);
    }
    args.extend(["--", &p.prompt]);
    let mut child = workshop::provider::managed(
        "claude",
        &args,
        &p.copy,
        std::process::Stdio::piped(),
        File::create(std::env::temp_dir().join("workshop-claude-probe.stderr"))?,
    )?;
    drop(child.stdin().take());
    let mut lines = BufReader::new(child.stdout().take().unwrap()).lines();
    let timeout = async {
        match p.cancel_after {
            Some(s) => tokio::time::sleep(Duration::from_secs(s)).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            line=lines.next_line()=>match line? {Some(line)=>println!("{line}"),None=>break},
            _=&mut timeout=>{child.start_kill()?; child.wait().await?; eprintln!("Probe cancelled");return Ok(());}
        }
    }
    ensure!(
        child.wait().await?.success(),
        "Claude probe failed. See the result event and provider sign-in status."
    );
    Ok(())
}
