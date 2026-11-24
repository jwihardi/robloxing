use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    collections::HashSet,
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};
use tokio::process::Command;

pub struct Row {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub action: &'static str,
    pub button: &'static str,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Provider {
    Codex,
    Claude,
}

impl Provider {
    pub fn from_label(label: &str) -> Self {
        if label == "Claude Code" {
            Self::Claude
        } else {
            Self::Codex
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
        }
    }
    fn program(self) -> OsString {
        match self {
            Self::Codex => workshop::provider::codex_program().into(),
            Self::Claude => env::var_os("WORKSHOP_CLAUDE").unwrap_or_else(|| "claude".into()),
        }
    }
}

fn command(program: OsString) -> Command {
    let mut cmd = Command::new(program);
    cmd.kill_on_drop(true).stdin(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    cmd
}
async fn output(cmd: &mut Command) -> Result<Output> {
    tokio::time::timeout(Duration::from_secs(120), cmd.output())
        .await
        .context("The command timed out. Try refreshing.")?
        .context("Could not start the CLI")
}
async fn checked(cmd: &mut Command) -> Result<Output> {
    let out = output(cmd).await?;
    if !out.status.success() {
        let reason = String::from_utf8_lossy(&out.stderr);
        let line = reason
            .lines()
            .rev()
            .find(|s| !s.trim().is_empty())
            .unwrap_or("The command failed");
        bail!("{}", line.chars().take(350).collect::<String>());
    }
    Ok(out)
}
async fn account(p: Provider) -> Row {
    let args: &[&str] = if p == Provider::Codex {
        &["login", "status"]
    } else {
        &["auth", "status", "--json"]
    };
    let status = output(command(p.program()).args(args)).await;
    let (detail, action, button) = match status {
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            ("CLI not installed or missing from PATH", "", "")
        }
        Ok(out) => {
            let text = format!(
                "{} {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
            .to_lowercase();
            let signed_in = if p == Provider::Claude {
                serde_json::from_slice::<Value>(&out.stdout)
                    .ok()
                    .and_then(|v| v["loggedIn"].as_bool())
            } else if text.contains("not logged in") || text.contains("not signed in") {
                Some(false)
            } else if out.status.success() && text.contains("logged in") {
                Some(true)
            } else {
                None
            };
            match signed_in {
                Some(true) => ("Signed in", "logout", "Sign out"),
                Some(false) => ("Signed out", "login", "Sign in"),
                None => ("Could not verify sign-in", "login", "Sign in"),
            }
        }
        Err(_) => ("Could not check account", "login", "Sign in"),
    };
    Row {
        id: p.label().into(),
        label: p.label().into(),
        detail: detail.into(),
        action,
        button,
    }
}

pub async fn list(tab: &str, p: Provider, browse: bool) -> Result<Vec<Row>> {
    match tab {
        "Providers" => {
            let (codex, claude) = tokio::join!(account(Provider::Codex), account(Provider::Claude));
            Ok(vec![codex, claude])
        }
        "Plugins" => {
            let mut cmd = command(p.program());
            cmd.args(["plugin", "list", "--json"]);
            if browse {
                cmd.arg("--available");
            }
            let value: Value = serde_json::from_slice(&checked(&mut cmd).await?.stdout)
                .context("The provider returned an unreadable plugin list")?;
            plugin_rows(&value, browse)
        }
        "Skills" => skills(p),
        _ => bail!("Unknown settings page"),
    }
}

fn plugin_rows(value: &Value, browse: bool) -> Result<Vec<Row>> {
    let installed = value
        .as_array()
        .or_else(|| value["installed"].as_array())
        .context("Missing installed plugin list")?;
    let mut ids = HashSet::new();
    let mut rows = Vec::new();
    for (items, present) in [
        (Some(installed), true),
        (value["available"].as_array().filter(|_| browse), false),
    ] {
        for item in items.into_iter().flatten() {
            let Some(id) = item["pluginId"].as_str().or_else(|| item["id"].as_str()) else {
                continue;
            };
            if !ids.insert(id) {
                continue;
            }
            let (name, market) = id.split_once('@').unwrap_or((id, ""));
            let state = if !present {
                "Available"
            } else if item["enabled"] == false {
                "Disabled"
            } else {
                "Installed"
            };
            rows.push(Row {
                id: id.into(),
                label: item["name"].as_str().unwrap_or(name).into(),
                detail: [
                    state,
                    item["marketplaceName"].as_str().unwrap_or(market),
                    item["description"].as_str().unwrap_or(""),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
                action: if item["uninstallable"] == false && present {
                    ""
                } else if present {
                    "remove"
                } else {
                    "install"
                },
                button: if present { "Uninstall" } else { "Install" },
            });
        }
    }
    rows.sort_by_key(|r| r.label.to_lowercase());
    Ok(rows)
}

pub fn official_plugin(provider: &str, id: &str) -> bool {
    let market = id.rsplit_once('@').map(|(_, market)| market);
    match provider {
        "Codex" => matches!(
            market,
            Some("openai-curated-remote" | "openai-bundled" | "openai-primary-runtime")
        ),
        "Claude Code" => market == Some("claude-plugins-official"),
        _ => false,
    }
}

fn skill_roots(p: Provider) -> Result<Vec<PathBuf>> {
    let home = env::home_dir().context("Cannot find the user folder")?;
    Ok(match p {
        Provider::Codex => vec![
            env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"))
                .join("skills"),
            home.join(".agents/skills"),
        ],
        Provider::Claude => vec![
            env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude"))
                .join("skills"),
        ],
    })
}
fn skills(p: Provider) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    for root in skill_roots(p)? {
        if !root.exists() {
            continue;
        }
        for entry in fs::read_dir(&root)? {
            let path = entry?.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') || !path.join("SKILL.md").is_file() {
                continue;
            }
            rows.push(Row {
                id: path.to_string_lossy().into(),
                label: name.into(),
                detail: root.to_string_lossy().into(),
                action: "remove",
                button: "Uninstall",
            });
        }
    }
    rows.sort_by_key(|r| r.label.to_lowercase());
    Ok(rows)
}

fn plain_component(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(['.', '-'])
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}
fn copy_skill(source: &Path, dest: &Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_name() == ".git" {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                metadata.file_attributes() & 0x400 == 0,
                "Skill contains a linked file or folder"
            );
        }
        let path = dest.join(entry.file_name());
        if metadata.is_dir() {
            fs::create_dir(&path)?;
            copy_skill(&entry.path(), &path)?;
        } else {
            ensure!(
                metadata.is_file(),
                "Skill contains a linked or special file"
            );
            fs::copy(entry.path(), path)?;
        }
    }
    Ok(())
}
fn install_folder(source: &Path, root: &Path) -> Result<()> {
    ensure!(
        source.join("SKILL.md").is_file(),
        "Select a skill folder containing SKILL.md"
    );
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .context("Missing skill folder name")?;
    ensure!(!name.starts_with('.'), "Choose a named skill folder");
    fs::create_dir_all(root)?;
    ensure!(
        !root.canonicalize()?.starts_with(source.canonicalize()?),
        "Choose an individual skill folder"
    );
    let dest = root.join(name);
    ensure!(
        fs::symlink_metadata(&dest).is_err(),
        "A skill named {name} is already installed"
    );
    let staged = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(root)?;
    copy_skill(source, staged.path())?;
    fs::rename(staged.path(), dest)?;
    Ok(())
}
async fn install_skill(p: Provider, source: &str) -> Result<()> {
    let roots = skill_roots(p)?;
    if Path::new(source).is_absolute() {
        return install_folder(Path::new(source), &roots[0]);
    }
    let url = source
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")
        .context("Paste the HTTPS URL of a GitHub skill folder")?;
    let parts: Vec<_> = url.split('/').collect();
    ensure!(
        parts.len() >= 2
            && parts.iter().all(|s| plain_component(s))
            && (parts.len() == 2 || (parts.len() >= 4 && parts[2] == "tree")),
        "Use a GitHub repository URL or a link to its skill folder"
    );
    let temp = tempfile::tempdir()?;
    let repo = parts[1].trim_end_matches(".git");
    let checkout = temp.path().join(repo);
    let mut git = command("git".into());
    git.env("GIT_TERMINAL_PROMPT", "0")
        .args(["clone", "--depth", "1", "--quiet"]);
    if let Some(branch) = parts.get(3) {
        git.args(["--branch", branch]);
    }
    checked(
        git.arg("--")
            .arg(format!("https://github.com/{}/{repo}.git", parts[0]))
            .arg(&checkout),
    )
    .await?;
    let mut src = checkout.clone();
    for part in parts.iter().skip(4) {
        src.push(part);
    }
    ensure!(
        src.canonicalize()?.starts_with(checkout.canonicalize()?),
        "Skill source is outside the selected repository"
    );
    install_folder(&src, &roots[0])
}

pub async fn change(tab: &str, p: Provider, act: &str, arg: &str) -> Result<String> {
    let arg = arg.trim();
    if tab == "Providers" {
        let provider = Provider::from_label(arg);
        let mut cmd = command(provider.program());
        if provider == Provider::Claude {
            cmd.arg("auth");
        }
        match act {
            "login" => {
                cmd.arg("login")
                    .stdin(Stdio::inherit())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
                #[cfg(windows)]
                cmd.creation_flags(0x00000010); // The provider may prompt for a browser code.
                let status = tokio::time::timeout(Duration::from_secs(600), cmd.status())
                    .await
                    .context("Sign-in timed out")??;
                ensure!(status.success(), "Sign-in was cancelled or failed");
                return Ok("Sign-in finished.".into());
            }
            "logout" => {
                checked(cmd.arg("logout")).await?;
                return Ok("Signed out.".into());
            }
            _ => bail!("Unknown account action"),
        }
    }
    ensure!(
        !arg.is_empty() && !arg.starts_with('-') && !arg.chars().any(char::is_control),
        "Enter a valid source or name"
    );
    if tab == "Plugins" {
        let mut cmd = command(p.program());
        cmd.arg("plugin");
        match act {
            "marketplace" => {
                cmd.args(["marketplace", "add", arg]);
            }
            "install" => {
                cmd.args([
                    if p == Provider::Codex {
                        "add"
                    } else {
                        "install"
                    },
                    arg,
                ]);
            }
            "remove" => {
                if p == Provider::Claude {
                    cmd.args(["uninstall", "--keep-data", arg]);
                } else {
                    cmd.args(["remove", arg]);
                }
            }
            _ => bail!("Unknown plugin action"),
        }
        checked(&mut cmd).await?;
        return Ok(match act {
            "marketplace" => "Marketplace added.",
            "install" => "Plugin installed.",
            _ => "Plugin uninstalled.",
        }
        .into());
    }
    ensure!(tab == "Skills", "Unknown settings page");
    match act {
        "install" | "folder" => {
            install_skill(p, arg).await?;
            Ok("Skill installed.".into())
        }
        "remove" => {
            let path = Path::new(arg);
            ensure!(
                skills(p)?.iter().any(|r| Path::new(&r.id) == path),
                "Skill is no longer installed"
            );
            let removed = path
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("removed-skills");
            fs::create_dir_all(&removed)?;
            let archive = tempfile::Builder::new()
                .prefix("skill-")
                .tempdir_in(&removed)?;
            fs::rename(path, archive.path().join(path.file_name().unwrap()))?;
            let archive = archive.keep();
            Ok(format!(
                "Skill uninstalled. A copy is in {}.",
                archive.display()
            ))
        }
        _ => bail!("Unknown skill action"),
    }
}
