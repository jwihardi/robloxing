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
        fs::write(project.join(".gitattributes"), "score.txt text eol=lf\n")?;
        fs::write(project.join("score.txt"), "Score: 0\n")?;
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
fn chat_search_can_reopen_a_match_beyond_the_current_history_page() -> Result<()> {
    let case = Case::new()?;
    let store = case.open()?;
    let db = rusqlite::Connection::open(store.dir.join("workshop.sqlite"))?;
    db.execute_batch(
        "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<42)
         INSERT INTO tasks(id,milestone,prompt) SELECT i,1,'Draft ' || i FROM n;
         INSERT INTO attempts(id,task,state,action,head,model)
         SELECT id,id,'accepted','','','' FROM tasks;
         UPDATE tasks SET prompt='First 100% complete' WHERE id=1;",
    )?;
    let hits = store.search_chats("100% COMPLETE")?;
    assert_eq!(hits, [(1, "First 100% complete".to_string())]);
    assert_eq!(store.search_chats("Draft")?.len(), 40);
    let position = store.chat_position(hits[0].0)?;
    assert_eq!(position, 41);
    assert_eq!(
        store.attempts_page(position / 40 * 40, 40)?[position % 40].id,
        hits[0].0
    );
    Ok(())
}

#[test]
#[ignore = "sends two messages through the locally signed-in Codex provider"]
fn live_followup_reuses_the_session_and_streams_both_replies() -> Result<()> {
    use std::time::Instant;
    use workshop::provider::{self, ReplyUpdate};

    let case = Case::new()?;
    let store = case.open()?;
    let mut session = None;
    let mut attempt = store.prepare("hi", "", None)?;
    for index in 0..2 {
        if index != 0 {
            attempt = store.resume(attempt.id)?;
        }
        let (_control, input) = provider::channel();
        let start = Instant::now();
        let mut first_text = None;
        let mut streamed = String::new();
        provider::run_turn(
            &mut session,
            &store,
            &attempt,
            "hi",
            input,
            false,
            |update| {
                if let ReplyUpdate::Delta(text) = update {
                    first_text.get_or_insert_with(|| start.elapsed());
                    streamed.push_str(text);
                }
            },
        )?;
        assert!(session.is_some());
        let messages = store.messages_after(attempt.id, 0)?;
        assert_eq!(messages.iter().filter(|m| !m.from_user).count(), index + 1);
        assert_eq!(messages.last().unwrap().text, streamed);
        eprintln!("Turn {} first text in {:?}", index + 1, first_text.unwrap());
    }
    let log = fs::read_to_string(store.folder(attempt.id).join("events.jsonl"))?;
    let events = log
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(events.iter().filter(|v| v["id"] == 1).count(), 1);
    assert!(events.iter().any(|v| {
        v["id"] == 5
            && v["result"]["data"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == &attempt.session))
    }));
    assert_eq!(events.iter().filter(|v| v["id"] == 2).count(), 1);
    Ok(())
}

#[test]
fn retries_keep_the_original_provider_and_native_sessions_stay_with_their_provider() -> Result<()> {
    let case = Case::new()?;
    let store = case.open()?;
    let chat = store.prepare("hello", "claude:sonnet", None)?;
    store.session(chat.id, "native-session")?;
    store.finish(chat.id, "review", "Ready")?;
    assert!(
        store
            .set_chat_inference(chat.id, "gpt-example", "high")
            .is_err()
    );
    store.set_chat_inference(chat.id, "claude:opus", "high")?;
    let retry = store.prepare("hello", "", Some(chat.id))?;
    assert_eq!(
        (&*retry.provider, &*retry.model, &*retry.effort),
        ("claude", "claude:opus", "high")
    );
    assert!(retry.session.is_empty());
    Ok(())
}

#[test]
#[ignore = "uses the locally signed-in Claude Code CLI"]
fn live_claude_streams_an_image_reply_and_resumes_after_restart() -> Result<()> {
    use workshop::provider::{self, Message, ReplyUpdate};
    let case = Case::new()?;
    let store = case.open()?;
    let reference = case.home.join("reference.png");
    image::RgbImage::from_pixel(64, 64, image::Rgb([250, 10, 20])).save(&reference)?;
    let first = "What color is the attached square? Reply in one word without tools.";
    let files = vec![reference];
    let a = store.prepare_with_files(first, "claude:sonnet", None, None, &files)?;
    let mut warm = None;
    let mut native_session = String::new();
    for (index, text) in [
        first,
        "What color did you just name? Reply in one word without tools.",
        "Repeat the same color. One word without tools.",
    ]
    .into_iter()
    .enumerate()
    {
        if index == 2 {
            drop(warm.take());
        }
        let a = if index == 0 {
            a.clone()
        } else {
            store.resume(a.id)?
        };
        let (_tx, rx) = provider::channel();
        let mut reply = String::new();
        provider::run_turn(
            &mut warm,
            &store,
            &a,
            Message::from(text),
            rx,
            false,
            |update| {
                if let ReplyUpdate::Delta(delta) = update {
                    reply.push_str(delta);
                }
            },
        )?;
        assert!(reply.to_lowercase().contains("red"), "{reply}");
        assert!(warm.is_some());
        let saved = store.attempt(a.id)?;
        assert!(store.context_usage(&saved)?.fraction_left().is_some());
        if index == 0 {
            native_session = saved.session;
        } else {
            assert_eq!(saved.session, native_session);
        }
        let replies = store
            .messages_after(a.id, 0)?
            .into_iter()
            .filter(|m| !m.from_user)
            .count();
        assert_eq!(replies, index + 1);
    }
    Ok(())
}

#[test]
fn transcript_reopens_in_order_and_can_fetch_only_new_messages() -> Result<()> {
    let case = Case::new()?;
    let store = case.open()?;
    let attempt = store.prepare("Hello", "", None)?;
    let id = attempt.id;
    store.event(id, "item/completed", Some("First reply"))?;
    store.event(id, "item/started", Some("A tool command"))?;
    store.event(id, "steer/sent", Some("Keep going"))?;
    let cursor = store.messages_after(id, 0)?.last().unwrap().id;
    store.event(id, "user/followup", Some("Explain the result"))?;
    store.event(id, "agent/message", Some("Final reply"))?;
    drop(store);

    let store = case.open()?;
    let messages = store.messages_after(id, 0)?;
    let transcript: Vec<_> = messages
        .iter()
        .map(|m| (m.from_user, m.text.as_str()))
        .collect();
    assert_eq!(
        transcript,
        [
            (false, "First reply"),
            (true, "Keep going"),
            (true, "Explain the result"),
            (false, "Final reply")
        ]
    );
    assert_eq!(store.messages_after(id, cursor)?.len(), 2);
    Ok(())
}

#[test]
fn interrupt_review_accept_twice_preserves_creator_and_index() -> Result<()> {
    let case = Case::new()?;
    let store = case.open()?;
    fs::write(case.project.join("score.txt"), "Staged edit\nScore: 0\n")?;
    case.git(&["add", "score.txt"]);
    let index = case.git(&["diff", "--cached"]);
    fs::write(case.project.join("score.txt"), "Creator edit\nScore: 0\n")?;
    fs::write(case.project.join("creator.txt"), "untracked")?;
    let a = store.prepare("fix score", "", None)?;
    assert_eq!(
        fs::read_to_string(store.work(a.id).join("creator.txt"))?,
        "untracked"
    );
    fs::write(
        store.work(a.id).join("score.txt"),
        "Creator edit\nScore: 2\n",
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
        fs::read_to_string(case.project.join("score.txt"))?,
        "Creator edit\nScore: 2\n"
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
    fs::write(store.work(a.id).join("score.txt"), "Score: 4\n")?;
    store.finish(a.id, "review", "Ready")?;
    let a = store.attempt(a.id)?;
    fs::write(case.project.join("score.txt"), "Score: 9\n")?;
    assert!(store.accept(a.id).is_err());
    assert_eq!(
        fs::read_to_string(case.project.join("score.txt"))?,
        "Score: 9\n"
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
        fs::read_to_string(store.work(next.id).join("score.txt"))?,
        "Score: 9\n"
    );
    Ok(())
}
#[test]
fn reopen_recovers_partial_application_and_records_one_decision() -> Result<()> {
    let case = Case::new()?;
    let store = case.open()?;
    let a = store.prepare("two files", "", None)?;
    fs::write(store.work(a.id).join("score.txt"), "Score: 1\n")?;
    fs::write(store.work(a.id).join("note.txt"), "done\n")?;
    store.finish(a.id, "review", "Ready")?;
    store.status(a.id, "applying", "Applying")?;
    fs::write(case.project.join("note.txt"), "done\n")?;
    drop(store);
    let store = case.open()?;
    assert_eq!(store.attempt(a.id)?.state, "accepted");
    assert_eq!(
        fs::read_to_string(case.project.join("score.txt"))?,
        "Score: 1\n"
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
