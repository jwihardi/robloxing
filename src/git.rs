use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::tempdir;

fn command(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
        .args([
            "--no-pager",
            "--literal-pathspecs",
            "-c",
            "core.hooksPath=NUL",
            "-c",
            "user.name=Workshop",
            "-c",
            "user.email=workshop@localhost",
            "-c",
            "commit.gpgSign=false",
        ])
        .env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd
}

fn output(cmd: &mut Command) -> Result<Vec<u8>> {
    let out = cmd
        .output()
        .context("Run Git. Install Git and make it available on PATH.")?;
    ensure!(
        out.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(out.stdout)
}

fn string(bytes: Vec<u8>) -> Result<String> {
    Ok(String::from_utf8(bytes)?.trim().to_owned())
}

pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    string(output(command(dir).args(args))?)
}

pub fn project(dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let dir = dunce::canonicalize(dir)?;
    let existing = dir.join(".git").exists();
    match git(&dir, &["rev-parse", "--show-toplevel"]) {
        Ok(root) => ensure!(
            dunce::canonicalize(root)? == dir,
            "Open the repository root folder."
        ),
        Err(e) if dir.join(".git").exists() => return Err(e),
        Err(_) => {
            git(&dir, &["init"])?;
        }
    }
    if git(&dir, &["rev-parse", "--verify", "HEAD"]).is_err() {
        if existing {
            // Give an unborn repository a revision without consuming its creator's index.
            let empty = git(&dir, &["mktree"])?;
            let head = git(
                &dir,
                &["commit-tree", &empty, "-m", "Initial project revision"],
            )?;
            git(&dir, &["update-ref", "HEAD", &head, ""])?;
        } else {
            git(&dir, &["add", "-A"])?;
            git(
                &dir,
                &["commit", "--allow-empty", "-m", "Initial project snapshot"],
            )?;
        }
    }
    Ok(dir)
}

// Copy the real index so staged additions, including force-added files, participate.
// All subsequent staging for this snapshot happens in the temporary index.
fn tree(dir: &Path) -> Result<String> {
    let tmp = tempdir()?;
    let index = tmp.path().join("index");
    let real = git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?;
    if Path::new(&real).exists() {
        fs::copy(real, &index)?;
    }
    let run = |args: &[&str]| output(command(dir).env("GIT_INDEX_FILE", &index).args(args));
    ensure!(
        run(&["ls-files", "-u"])?.is_empty(),
        "Resolve the project's Git conflicts first."
    );
    run(&["add", "-A", "--", "."])?;
    string(run(&["write-tree"])?)
}

pub fn isolate(source: &Path, work: &Path, head: &str) -> Result<String> {
    let base = git(
        source,
        &[
            "commit-tree",
            &tree(source)?,
            "-p",
            head,
            "-m",
            "Task starting point",
        ],
    )?;
    fs::create_dir_all(work)?;
    git(work, &["init"])?;
    output(
        command(work)
            .args(["fetch", "--no-tags"])
            .arg(source)
            .arg(&base),
    )?;
    git(work, &["checkout", "--detach", &base])?;
    Ok(base)
}

pub fn capture(work: &Path, base: &str) -> Result<(String, Vec<u8>)> {
    let candidate = tree(work)?;
    let bytes = diff(work, base, &candidate, &[])?;
    // Keep even an uncommitted candidate reachable across gc and process restarts.
    git(
        work,
        &[
            "update-ref",
            &format!("refs/workshop/reviews/{candidate}"),
            &candidate,
        ],
    )?;
    Ok((candidate, bytes))
}

fn diff(work: &Path, base: &str, candidate: &str, paths: &[&str]) -> Result<Vec<u8>> {
    output(
        command(work)
            .args([
                "diff",
                "--binary",
                "--full-index",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                base,
                candidate,
                "--",
            ])
            .args(paths),
    )
}

fn entries(dir: &Path, tree: &str) -> Result<BTreeMap<String, String>> {
    let bytes = output(command(dir).args(["ls-tree", "-rz", tree]))?;
    let mut files = BTreeMap::new();
    for row in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let (entry, name) = std::str::from_utf8(row)?
            .split_once('\t')
            .context("Invalid Git tree")?;
        files.insert(name.to_owned(), entry.to_owned());
    }
    Ok(files)
}

pub struct Review<'a> {
    pub source: &'a Path,
    pub work: &'a Path,
    pub head: &'a str,
    pub base: &'a str,
    pub candidate: &'a str,
    pub patch: &'a Path,
}

impl Review<'_> {
    pub fn apply(&self, recovering: bool, begin: impl FnOnce() -> Result<()>) -> Result<()> {
        ensure!(
            git(self.source, &["rev-parse", "HEAD"])? == self.head,
            "Project revision changed. Retry the task against the current revision."
        );
        let original = entries(self.work, self.base)?;
        let proposed = entries(self.work, self.candidate)?;
        let current = entries(self.source, &tree(self.source)?)?;
        let mut pending = Vec::new();
        let mut changed = 0;
        for name in original
            .keys()
            .chain(proposed.keys().filter(|n| !original.contains_key(*n)))
        {
            if original.get(name) == proposed.get(name) {
                continue;
            }
            changed += 1;
            if recovering && current.get(name) == proposed.get(name) {
                continue;
            }
            if current.get(name) != original.get(name) {
                bail!(
                    "{name} changed since this task started. Your file is intact. Retry against the current project."
                );
            }
            pending.push(name.as_str());
        }
        if pending.is_empty() {
            return Ok(());
        }
        let tmp = tempdir()?;
        let partial;
        let path = if pending.len() == changed {
            self.patch
        } else {
            partial = tmp.path().join("remaining.patch");
            fs::write(
                &partial,
                diff(self.work, self.base, self.candidate, &pending)?,
            )?;
            &partial
        };
        let apply = |extra: &[&str]| {
            output(
                command(self.source)
                    .args(["apply", "--binary", "--whitespace=nowarn"])
                    .args(extra)
                    .arg("--")
                    .arg(OsStr::new(path)),
            )
        };
        apply(&["--check"])?;
        begin()?;
        apply(&[])?;
        Ok(())
    }
}
