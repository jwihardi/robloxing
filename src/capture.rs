use anyhow::{Context, Result, ensure};
use std::{fs, path::PathBuf, process::Command};
use workshop::{Store, data_dir};

pub fn requested(text: &str) -> bool {
    let request = text
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .to_ascii_lowercase();
    let mut request = request.as_str();
    for prefix in ["please ", "can you ", "could you ", "please "] {
        request = request.strip_prefix(prefix).unwrap_or(request);
    }
    let request = request.trim_end_matches(" please");
    if matches!(request, "/screenshot" | "screenshot") {
        return true;
    }
    [
        "take a screenshot",
        "capture a screenshot",
        "send a screenshot",
        "send me a screenshot",
        "show me a screenshot",
    ]
    .iter()
    .filter_map(|verb| request.strip_prefix(verb))
    .any(|tail| {
        matches!(tail, "" | " for me" | " and send it" | " and send it to me")
            || tail.starts_with(" of ")
    })
}

pub fn image_path(key: &str, full: bool) -> Result<PathBuf> {
    ensure!(
        key.starts_with("shot-") && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
        "Invalid screenshot"
    );
    Ok(data_dir()?.join("screenshots").join(key).join(if full {
        "capture.png"
    } else {
        "preview.png"
    }))
}

pub fn take(store: &Store, attempt: Option<i64>) -> Result<bool> {
    ensure!(cfg!(windows), "Screen capture requires Windows.");
    let root = data_dir()?.join("screenshots");
    fs::create_dir_all(&root)?;
    let stage = tempfile::Builder::new().prefix("shot-").tempdir_in(&root)?;
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").context("Cannot find Windows")?)
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut cmd = Command::new(powershell);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let output = cmd
        .current_dir(stage.path())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-STA",
            "-Command",
            include_str!("capture.ps1"),
        ])
        .output()
        .context("Open the screenshot picker")?;
    ensure!(
        output.status.success(),
        "Screenshot failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    if !stage.path().join("capture.png").is_file() {
        return Ok(false);
    }
    ensure!(
        stage.path().join("preview.png").is_file(),
        "Could not save screenshot preview"
    );
    let key = stage
        .path()
        .file_name()
        .and_then(|s| s.to_str())
        .context("Invalid capture directory")?;
    store.post_screenshot(attempt, key)?;
    let _ = stage.keep();
    Ok(true)
}
