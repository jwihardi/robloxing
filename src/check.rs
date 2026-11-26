use crate::{Store, git};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
};

impl Store {
    pub fn check(&self, id: i64, args: &[String]) -> Result<String> {
        let a = self.attempt(id)?;
        ensure!(
            !a.candidate.is_empty() && !matches!(a.state.as_str(), "running" | "preparing"),
            "Wait for the attempt to finish."
        );
        let work = self.work(id);
        ensure!(
            git::working_tree(&work)? == a.candidate,
            "The working copy changed since review. Resume or retry before checking."
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let configured = !args.is_empty() && self.team_settings()?.check_args == args;
        if configured {
            // If this recheck fails or is interrupted, an older pass must not authorize acceptance.
            self.db
                .lock()
                .unwrap()
                .execute("DELETE FROM verified_team WHERE lead=?1", [id])?;
        }
        let msg = runtime.block_on(self.run_check(id, args, &a.candidate))?;
        ensure!(
            git::working_tree(&work)? == a.candidate,
            "Check modified the reviewed files. Resume or retry to review its edits."
        );
        if configured {
            self.record_verification(id, &a.candidate, args)?;
        }
        Ok(msg)
    }
    pub(crate) async fn run_check(
        &self,
        id: i64,
        args: &[String],
        revision: &str,
    ) -> Result<String> {
        let (exe, args) = args.split_first().context("Choose a check command.")?;
        let work = self.work(id);
        let folder = self.folder(id);
        self.artifact(id, "check", "checks.log")?;
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(folder.join("checks.log"))?;
        writeln!(log, "\nChecking {} with {} {:?}", revision, exe, args)?;
        let start = log.metadata()?.len();
        let success = {
            let mut child = crate::provider::managed(
                exe,
                &args.iter().map(String::as_str).collect::<Vec<_>>(),
                &work,
                log.try_clone()?.into(),
                log.try_clone()?,
            )?;
            drop(child.stdin().take());
            let status = child.wait().await?;
            status.success()
        };
        let mut reader = File::open(folder.join("checks.log"))?;
        reader.seek(SeekFrom::Start(
            start.max(reader.metadata()?.len().saturating_sub(4096)),
        ))?;
        let mut preview = Vec::new();
        reader.read_to_end(&mut preview)?;
        let msg = if success {
            "Check passed"
        } else {
            "Check failed"
        };
        self.event(id, "check", Some(msg))?;
        ensure!(
            success,
            "Check failed. See {}\n{}",
            folder.join("checks.log").display(),
            String::from_utf8_lossy(&preview)
        );
        Ok(format!(
            "{msg}. {}",
            String::from_utf8_lossy(&preview).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checking_a_stale_copy_leaves_only_the_reviewed_revision_pinned() -> Result<()> {
        let scratch = tempfile::tempdir()?;
        let store = Store::open(
            &scratch.path().join("project"),
            &scratch.path().join("home"),
        )?;
        let chat = store.prepare("Make an asset", "", None)?;
        let work = store.work(chat.id);
        let asset = work.join("asset.bin");
        std::fs::write(&asset, [0, 1, 2])?;
        store.finish(chat.id, "review", "Ready")?;
        let refs = || {
            git::git(
                &work,
                &[
                    "for-each-ref",
                    "--format=%(refname)",
                    "refs/workshop/reviews",
                ],
            )
        };
        let before = refs()?;
        assert_eq!(before.lines().count(), 1);
        std::fs::write(asset, [3, 4, 5])?;
        assert!(
            store
                .check(chat.id, &[])
                .unwrap_err()
                .to_string()
                .contains("changed since review")
        );
        assert_eq!(refs()?, before);
        Ok(())
    }
}
