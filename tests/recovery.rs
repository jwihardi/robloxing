use anyhow::Result;
use std::{fs, path::PathBuf, process::Command};
use tempfile::{TempDir, tempdir};
use workshop::Store;

struct Case {
    _temp: TempDir,
    project: PathBuf,
    home: PathBuf,
}
impl Case {
    fn new() -> Result<Self> {
        let temp = tempdir()?;
        let project = temp.path().join("My Game");
        let home = temp.path().join("data");
        fs::create_dir(&project)?;
        fs::write(project.join(".gitattributes"), "*.luau text eol=lf\n")?;
        fs::write(project.join("score.luau"), "return 0\n")?;
        drop(Store::open(&project, &home)?);
        Ok(Self {
            _temp: temp,
            project,
            home,
        })
    }
    fn open(&self) -> Result<Store> {
        Store::open(&self.project, &self.home)
    }
    fn git(&self, args: &[&str]) -> Vec<u8> {
        let out = Command::new("git")
            .current_dir(&self.project)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success());
        out.stdout
    }
}
#[test]
fn interrupt_review_accept_twice_preserves_creator_and_index() -> Result<()> {
    let case = Case::new()?;
    let store = case.open()?;
    fs::write(case.project.join("score.luau"), "-- staged\nreturn 0\n")?;
    case.git(&["add", "score.luau"]);
    let index = case.git(&["diff", "--cached"]);
    fs::write(
        case.project.join("score.luau"),
        "-- staged and edited\nreturn 0\n",
    )?;
    fs::write(case.project.join("creator.txt"), "untracked")?;
    let a = store.prepare("fix score", "", None)?;
    assert_eq!(
        fs::read_to_string(store.work(a.id).join("creator.txt"))?,
        "untracked"
    );
    fs::write(
        store.work(a.id).join("score.luau"),
        "-- staged and edited\nreturn 2\n",
    )?;
    fs::write(store.work(a.id).join("added.bin"), [0, 4, 0, 8])?;
    assert!(case.open().is_err());
    drop(store);
    let store = case.open()?;
    let a = store.attempt(a.id)?;
    assert_eq!(a.state, "interrupted");
    assert!(!store.patch(&a)?.is_empty());
    fs::write(case.project.join("creator.txt"), "edited during review")?;
    store.accept(a.id)?;
    store.accept(a.id)?;
    assert_eq!(
        fs::read_to_string(case.project.join("score.luau"))?,
        "-- staged and edited\nreturn 2\n"
    );
    assert_eq!(fs::read(case.project.join("added.bin"))?, [0, 4, 0, 8]);
    assert_eq!(case.git(&["diff", "--cached"]), index);
    assert_eq!(
        fs::read_to_string(case.project.join("creator.txt"))?,
        "edited during review"
    );
    Ok(())
}
#[test]
fn changed_destination_and_corrupt_artifact_never_overwrite_creator() -> Result<()> {
    let case = Case::new()?;
    let store = case.open()?;
    let a = store.prepare("fix", "", None)?;
    fs::write(store.work(a.id).join("score.luau"), "return 4\n")?;
    store.finish(a.id, "review", "Ready")?;
    let a = store.attempt(a.id)?;
    fs::write(case.project.join("score.luau"), "return 9\n")?;
    assert!(store.accept(a.id).is_err());
    assert_eq!(
        fs::read_to_string(case.project.join("score.luau"))?,
        "return 9\n"
    );
    fs::write(
        store.folder(a.id).join(format!("{}.patch", a.patch)),
        b"bad",
    )?;
    assert!(store.accept(a.id).is_err());
    store.discard(a.id)?;
    let next = store.prepare(&a.prompt, "", Some(a.id))?;
    assert_eq!(next.task, a.task);
    assert_ne!(next.id, a.id);
    assert_eq!(
        fs::read_to_string(store.work(next.id).join("score.luau"))?,
        "return 9\n"
    );
    Ok(())
}
#[test]
fn reopen_recovers_partial_application_and_records_one_decision() -> Result<()> {
    let case = Case::new()?;
    let store = case.open()?;
    let a = store.prepare("two files", "", None)?;
    fs::write(store.work(a.id).join("score.luau"), "return 1\n")?;
    fs::write(store.work(a.id).join("note.txt"), "done\n")?;
    store.finish(a.id, "review", "Ready")?;
    store.status(a.id, "applying", "Applying")?;
    fs::write(case.project.join("note.txt"), "done\n")?;
    drop(store);
    let store = case.open()?;
    assert_eq!(store.attempt(a.id)?.state, "accepted");
    assert_eq!(
        fs::read_to_string(case.project.join("score.luau"))?,
        "return 1\n"
    );
    store.accept(a.id)?;
    let db = rusqlite::Connection::open(store.dir.join("workshop.sqlite"))?;
    assert_eq!(
        db.query_row("SELECT count(*) FROM decisions", [], |r| r.get::<_, i64>(0))?,
        1
    );
    Ok(())
}

#[test]
fn opening_unborn_repo_preserves_staged_and_unstaged_versions() -> Result<()> {
    let tmp = tempdir()?;
    let project = tmp.path().join("existing");
    fs::create_dir(&project)?;
    let git = |args: &[&str]| {
        Command::new("git")
            .current_dir(&project)
            .args(args)
            .output()
    };
    assert!(git(&["init"])?.status.success());
    fs::write(project.join("note.txt"), "staged")?;
    assert!(git(&["add", "note.txt"])?.status.success());
    fs::write(project.join("note.txt"), "working")?;
    let store = Store::open(&project, &tmp.path().join("data"))?;
    assert_eq!(git(&["show", ":note.txt"])?.stdout, b"staged");
    let a = store.prepare("read project", "", None)?;
    assert_eq!(fs::read(store.work(a.id).join("note.txt"))?, b"working");
    Ok(())
}
