use super::{Provider, command};
use anyhow::{Context, Result, bail, ensure};
use std::{env, path::Path, process::Output, time::Duration};
use tokio::process::Command;

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
