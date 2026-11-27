use super::{Provider, command};
use anyhow::{Context, Result, bail, ensure};
use std::process::Stdio;
use std::{env, path::Path, process::Output, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;

pub async fn antigravity_login(
    mut input: oneshot::Receiver<String>,
    mut cancelled: oneshot::Receiver<()>,
    mut publish_link: impl FnMut(String),
) -> Result<String> {
    let dir = tempfile::tempdir()?;
    let mut child = command(Provider::Antigravity.program()?)
        .args(["acp", "--login"])
        .env("ANTIGRAVITY_REMOTE_OOB_AUTH", "true")
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Could not start Antigravity sign-in")?;
    let mut input_pipe = child.stdin.take();
    let mut stdout = BufReader::new(child.stdout.take().context("Missing sign-in output")?).lines();
    let mut stderr =
        BufReader::new(child.stderr.take().context("Missing sign-in diagnostics")?).lines();
    let (mut out_open, mut err_open) = (true, true);
    tokio::time::timeout(Duration::from_secs(600), async {
        loop {
            let line = tokio::select! {
                _ = &mut cancelled => bail!("Sign-in cancelled."),
                status = child.wait() => {
                    ensure!(status?.success(), "Antigravity could not finish sign-in. Start again for a fresh code.");
                    return Ok("Sign-in finished.".into());
                }
                code = &mut input, if input_pipe.is_some() => {
                    let mut stdin = input_pipe.take().context("Missing sign-in input")?;
                    let code = code.context("Sign-in cancelled")?;
                    stdin.write_all(code.as_bytes()).await?;
                    stdin.write_all(b"\n").await?;
                    stdin.shutdown().await?;
                    continue;
                }
                line = stdout.next_line(), if out_open => { let line = line?; out_open = line.is_some(); line }
                line = stderr.next_line(), if err_open => { let line = line?; err_open = line.is_some(); line }
            };
            if let Some(url) = line.as_deref().and_then(signin_url) {
                publish_link(url);
            }
        }
    }).await.context("Sign-in timed out. Start again for a fresh code.")?
}

fn signin_url(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|word| {
        let url = url::Url::parse(word).ok()?;
        let host = url.host_str()?;
        (url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && (host == "accounts.google.com"
                || host == "antigravity.google"
                || host.ends_with(".antigravity.google")))
        .then(|| url.to_string())
    })
}

impl Provider {
    fn override_name(self) -> &'static str {
        match self {
            Self::Codex => "WORKSHOP_CODEX",
            Self::Claude => "WORKSHOP_CLAUDE",
            Self::Antigravity => "WORKSHOP_ANTIGRAVITY",
        }
    }

    pub(super) fn missing_cli(self) -> (&'static str, &'static str, &'static str) {
        if env::var_os(self.override_name()).is_some() {
            let message = match self {
                Self::Codex => "Check the executable configured in WORKSHOP_CODEX",
                Self::Claude => "Check the executable configured in WORKSHOP_CLAUDE",
                Self::Antigravity => "Check the executable configured in WORKSHOP_ANTIGRAVITY",
            };
            (message, "", "")
        } else {
            ("CLI not installed", "install", "Install")
        }
    }

    fn installer_url(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Self::Codex, true) => "https://chatgpt.com/codex/install.ps1",
            (Self::Claude, true) => "https://claude.ai/install.ps1",
            (Self::Antigravity, true) => "https://antigravity.google/cli/install.ps1",
            (Self::Codex, false) => "https://chatgpt.com/codex/install.sh",
            (Self::Claude, false) => "https://claude.ai/install.sh",
            (Self::Antigravity, false) => "https://antigravity.google/cli/install.sh",
        }
    }

    pub(super) async fn install(self) -> Result<String> {
        ensure!(
            env::var_os(self.override_name()).is_none(),
            "Check {}",
            self.override_name()
        );
        let dir = tempfile::tempdir()?;
        let script = dir.path().join(if cfg!(windows) {
            "install.ps1"
        } else {
            "install.sh"
        });
        #[cfg(windows)]
        let curl = system32()?.join("curl.exe").into_os_string();
        #[cfg(not(windows))]
        let curl = "curl".into();
        let mut download = command(curl);
        download
            .current_dir(dir.path())
            .args([
                "--disable",
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--output",
            ])
            .arg(&script)
            .arg(self.installer_url());
        run(&mut download, "Download installer", 90).await?;
        run_script(&script).await?;
        let mut version = command(self.program()?);
        version.current_dir(dir.path()).arg("--version");
        run(&mut version, "Verify installed CLI", 20).await?;
        Ok(format!("{} installed.", self.label()))
    }
}

#[cfg(windows)]
fn system32() -> Result<std::path::PathBuf> {
    Ok(std::path::PathBuf::from(
        env::var_os("SystemRoot").context("Missing Windows system directory")?,
    )
    .join("System32"))
}

async fn run_script(script: &Path) -> Result<()> {
    #[cfg(windows)]
    let mut installer = {
        let mut cmd = command(
            system32()?
                .join("WindowsPowerShell/v1.0/powershell.exe")
                .into_os_string(),
        );
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ]);
        cmd
    };
    #[cfg(not(windows))]
    let mut installer = command("/bin/bash".into());
    installer
        .current_dir(script.parent().context("Missing installer directory")?)
        .env("CODEX_NON_INTERACTIVE", "1")
        .arg(script);
    run(&mut installer, "Run installer", 600).await?;
    Ok(())
}

async fn run(cmd: &mut Command, step: &str, seconds: u64) -> Result<Output> {
    let out = tokio::time::timeout(Duration::from_secs(seconds), cmd.output())
        .await
        .with_context(|| format!("{step} timed out"))?
        .with_context(|| format!("{step} could not start"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let reason = stderr
            .lines()
            .rev()
            .chain(stdout.lines().rev())
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("Command failed");
        bail!(
            "{step} ({status}): {reason}",
            status = out.status,
            reason = reason.chars().take(350).collect::<String>()
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn installer_runs_in_its_own_directory_and_surfaces_failure() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (filename, success, failure) = if cfg!(windows) {
            (
                "fixture setup.ps1",
                "Set-Content result $env:CODEX_NON_INTERACTIVE",
                "[Console]::Error.WriteLine('fixture error'); exit 7",
            )
        } else {
            (
                "fixture setup.sh",
                "echo \"$CODEX_NON_INTERACTIVE\" > result",
                "echo 'fixture error' >&2; exit 7",
            )
        };
        let script = dir.path().join(filename);
        std::fs::write(&script, success)?;
        run_script(&script).await?;
        assert_eq!(
            std::fs::read_to_string(dir.path().join("result"))?.trim(),
            "1"
        );
        std::fs::write(&script, failure)?;
        assert!(
            run_script(&script)
                .await
                .unwrap_err()
                .to_string()
                .contains("fixture error")
        );
        Ok(())
    }
}
