use super::*;
use std::fs;
use tempfile::tempdir;

fn assignment(key: &str, deps: &[&str], files: &[&str]) -> Assignment {
    Assignment {
        key: key.into(),
        model: String::new(),
        effort: String::new(),
        title: key.into(),
        deliverable: format!("Implement {key}"),
        acceptance: "Matches the project spec".into(),
        files: files.iter().map(|s| s.to_string()).collect(),
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn lead_and_worker_can_find_and_page_preexisting_reports() -> Result<()> {
    let tmp = tempdir()?;
    let project = tmp.path().join("game");
    let data = tmp.path().join("data");
    let store = Store::open(&project, &data)?;
    let binary = project.join("tile.o");
    fs::write(&binary, b"original input")?;
    let recipe = crate::studio::Recipe::Decompile {
        binary: binary.clone(),
        filter: "tile".into(),
    };
    let id = store.start_studio_job(&recipe)?;
    let dir = store.studio_job(id)?.folder(&store);
    fs::create_dir_all(&dir)?;
    let second = "/* 00002000 */\nint tile_cost(int n) { return n + 1; }\n";
    let report = format!(
        "/* 00001000 */\nint tile_count() {{ return 0; }}\n/*{}*/\n{second}",
        "é".repeat(10_000)
    );
    fs::write(dir.join("functions.c"), &report)?;
    fs::write(dir.join("tool.log"), "Original import completed")?;
    store
        .db
        .lock()
        .unwrap()
        .execute("UPDATE studio_jobs SET state='Ready' WHERE id=?", [id])?;
    let incomplete = store.start_studio_job(&recipe)?;
    drop(store);
    fs::remove_file(binary)?;
    let store = Store::open(&project, &data)?;
    let lead = store.prepare("Look up the saved tile report", "", None)?;
    let worker = store.child_store(lead.id, 77)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    for reader in [&store, &worker] {
        let call = |name: &str, args: Value| -> Result<Value> {
            Ok(serde_json::from_str(&rt.block_on(dispatch(
                reader,
                &lead,
                &json!({"tool":name,"arguments":args}),
                &|_| {},
            ))?)?)
        };
        let found = call("workshop_find_reports", json!({"query":"TILE_COST"}))?;
        assert_eq!(found["reports"][0]["id"], id);
        let offset = found["reports"][0]["functions"][0]["offset"]
            .as_u64()
            .unwrap();
        assert_eq!(
            call(
                "workshop_read_report",
                json!({"id":id,"part":"report","offset":offset})
            )?["text"],
            second
        );
        let first = call("workshop_read_report", json!({"id":id,"part":"report"}))?;
        let rest = call(
            "workshop_read_report",
            json!({"id":id,"part":"report","offset":first["next_offset"]}),
        )?;
        assert_eq!(
            format!(
                "{}{}",
                first["text"].as_str().unwrap(),
                rest["text"].as_str().unwrap()
            ),
            report
        );
        assert_eq!(
            call("workshop_read_report", json!({"id":id,"part":"log"}))?["text"],
            "Original import completed"
        );
        assert!(
            call(
                "workshop_read_report",
                json!({"id":incomplete,"part":"log"})
            )?["text"]
                .as_str()
                .unwrap()
                .contains("Interrupted")
        );
        assert!(
            call(
                "workshop_read_report",
                json!({"id":incomplete,"part":"report"})
            )
            .is_err()
        );
        if reader.worker {
            let worker_tools = tools(true);
            let names: Vec<_> = worker_tools
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect();
            assert_eq!(
                names,
                [
                    "workshop_find_reports",
                    "workshop_read_report",
                    "workshop_report_result"
                ]
            );
            assert!(call("workshop_delegate", json!({"tasks":[]})).is_err());
        }
    }
    assert_eq!(
        store
            .find_studio_jobs("Ghidra", "tile_cost", false, None)?
            .len(),
        1
    );
    let other = Store::open(&tmp.path().join("other"), &data)?;
    assert!(other.report_archive()?.search("", None)?.reports.is_empty());
    assert!(other.report_archive()?.read(id, "report", 0).is_err());
    Ok(())
}

#[test]
fn studio_import_survives_replay_and_requires_review() -> Result<()> {
    let tmp = tempdir()?;
    let project = tmp.path().join("p");
    let store = Store::open(&project, &tmp.path().join("data"))?;
    let id = store.start_studio_job(&crate::studio::Recipe::Texture {
        material: "Sand".into(),
        seed: 2,
    })?;
    let (_hold, signal) = tokio::sync::oneshot::channel();
    store.run_studio_job(id, signal)?;
    let lead = store.prepare("Use the sand tile", "", None)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let found = rt.block_on(dispatch(
        &store,
        &lead,
        &json!({"tool":"workshop_find_assets","arguments":{"area":"Images","query":"sand"}}),
        &|_| {},
    ))?;
    assert_eq!(serde_json::from_str::<Value>(&found)?[0]["id"], id);
    let import =
        json!({"tool":"workshop_import_asset","arguments":{"id":id,"destination":"Art/Dunes.png"}});
    rt.block_on(dispatch(&store, &lead, &import, &|_| {}))?;
    rt.block_on(dispatch(&store, &lead, &import, &|_| {}))?;
    for invalid in [
        "../escape.png",
        "Art/.git/file.png",
        "C:/other.png",
        "Art/NUL.png",
        "file.exe",
    ] {
        assert!(store.import_studio_asset(lead.id, id, invalid).is_err());
    }
    fs::write(store.work(lead.id).join("Art/keep.png"), "existing")?;
    assert!(
        store
            .import_studio_asset(lead.id, id, "Art/keep.png")
            .is_err()
    );
    assert_eq!(
        fs::read(store.work(lead.id).join("Art/keep.png"))?,
        b"existing"
    );
    assert!(!project.join("Art/Dunes.png").exists());
    store.finish(lead.id, "review", "Review asset")?;
    store.accept(lead.id)?;
    assert_eq!(
        image::image_dimensions(project.join("Art/Dunes.png"))?,
        (256, 256)
    );
    Ok(())
}

#[test]
fn queue_validates_the_complete_dependency_graph_before_inserting() -> Result<()> {
    let tmp = tempdir()?;
    let store = Store::open(&tmp.path().join("p"), &tmp.path().join("db"))?;
    let a = store.prepare("plan", "", None)?;
    let top = assignment("top", &["base"], &["src"]);
    let mut base = assignment("base", &["top"], &["SRC/one.txt"]);
    assert!(overlaps(&top, &base));
    assert!(!overlaps(&top, &assignment("other", &[], &["src2"])));
    assert!(store.queue(a.id, &[top.clone(), base.clone()]).is_err());
    assert!(store.queue(a.id, std::slice::from_ref(&top)).is_err());
    assert!(store.workers(a.id)?.is_empty());
    base.depends_on.clear();
    store.queue(a.id, &[top.clone(), base.clone()])?;
    store.queue(a.id, &[top, base])?;
    let mut workers = store.workers(a.id)?;
    assert_eq!(workers.len(), 2);
    assert_eq!(next_wave(&workers, 2)[0].assignment.key, "base");
    let base = workers
        .iter()
        .position(|w| w.assignment.key == "base")
        .unwrap();
    workers[base].state = "blocked".into();
    assert!(next_wave(&workers, 2).is_empty());
    workers[base].state = "finished".into();
    assert_eq!(next_wave(&workers, 2)[0].assignment.key, "top");
    assert!(!store.dir.join("workers").exists());
    Ok(())
}

#[test]
fn dispatch_receipts_track_only_accepted_assignments_in_chat_order() -> Result<()> {
    let tmp = tempdir()?;
    let project = tmp.path().join("project");
    let home = tmp.path().join("data");
    let store = Store::open(&project, &home)?;
    let lead = store.prepare("write ten files", "", None)?;
    let pair = [
        assignment("first", &[], &["first"]),
        assignment("second", &[], &["second"]),
    ];
    store.event(lead.id, "user/followup", Some("Use two subagents"))?;
    store.queue(lead.id, &pair)?;
    store.queue(lead.id, &pair)?;
    let ids: Vec<_> = store.workers(lead.id)?.iter().map(|w| w.id).collect();
    store.worker_state(ids[1], "blocked", "Retry")?;
    store.queue(lead.id, &pair)?;
    store.event(lead.id, "agent/message", Some("Done"))?;
    drop(store);

    let store = Store::open(&project, &home)?;
    let (rows, more) = store.message_page(lead.id, i64::MAX, 10)?;
    assert!(!more);
    assert_eq!(
        rows.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        [
            "Use two subagents",
            "Dispatched 2 subagents",
            "Dispatched 1 subagent",
            "Done"
        ]
    );
    assert_eq!(rows[1].dispatched_workers, ids);
    assert_eq!(rows[2].dispatched_workers, [ids[1]]);
    Ok(())
}

#[test]
fn interrupted_worker_integration_recovers_once_and_checks_the_combined_revision() -> Result<()> {
    let tmp = tempdir()?;
    let project = tmp.path().join("project");
    let home = tmp.path().join("data");
    fs::create_dir(&project)?;
    fs::write(project.join(".gitattributes"), "*.txt text eol=lf\n")?;
    let store = Store::open(&project, &home)?;
    let lead = store.prepare("create pair", "", None)?;
    store.session(lead.id, "lead")?;
    let spec = assignment("pair", &[], &["one.txt", "two.txt"]);
    store.queue(lead.id, std::slice::from_ref(&spec))?;
    let w = store.workers(lead.id)?.remove(0);
    let child = store.child_store(lead.id, w.id)?;
    let run = child.prepare("pair", "", None)?;
    for file in &spec.files {
        fs::write(child.work(run.id).join(file), "once\n")?;
    }
    child.finish(run.id, "review", "ready")?;
    store.db.lock().unwrap().execute(
        "UPDATE workers SET state='integrating',run=?1,tries=1 WHERE id=?2",
        params![run.id, w.id],
    )?;
    child.status(run.id, "applying", "applying")?;
    fs::write(store.work(lead.id).join("one.txt"), "once\n")?;
    drop(child);
    drop(store);

    let store = Store::open(&project, &home)?;
    assert_eq!(store.workers(lead.id)?[0].state, "checking");
    assert_eq!(
        fs::read_to_string(store.work(lead.id).join("two.txt"))?,
        "once\n"
    );
    assert!(!project.join("two.txt").exists());
    let lead = store.resume(lead.id)?;
    store.queue(lead.id, &[spec])?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(store.execute_queue(&lead, &|_| {}))?;
    assert_eq!(store.workers(lead.id)?[0].tries, 1);
    store.checkpoint(lead.id, "Keep one line per file", "Add guide")?;
    store.finish(lead.id, "review", "ready")?;
    assert!(store.handoff(999)?.is_empty());
    store.accept(lead.id)?;
    store.accept(lead.id)?;
    assert_eq!(fs::read_to_string(project.join("one.txt"))?, "once\n");
    let next = store.prepare("continue", "", None)?;
    assert!(store.handoff(next.id)?.contains("Add guide"));
    Ok(())
}

#[test]
fn older_projects_keep_model_settings_and_accept_per_chat_effort() -> Result<()> {
    let tmp = tempdir()?;
    let project = tmp.path().join("project");
    let home = tmp.path().join("data");
    let store = Store::open(&project, &home)?;
    let a = store.prepare("hello", "existing-model", None)?;
    store.finish(a.id, "review", "ready")?;
    store.db.lock().unwrap().execute_batch(
        "ALTER TABLE attempts DROP COLUMN effort;
         ALTER TABLE team_settings DROP COLUMN lead_effort;
         ALTER TABLE team_settings DROP COLUMN worker_effort;
         ALTER TABLE team_settings DROP COLUMN auto_delegate;
         UPDATE team_settings SET worker_model='existing-worker';",
    )?;
    drop(store);
    let store = Store::open(&project, &home)?;
    assert_eq!(store.team_settings()?.worker_model, "existing-worker");
    assert!(store.team_settings()?.auto_delegate);
    assert_eq!(store.attempt(a.id)?.model, "existing-model");
    assert!(store.attempt(a.id)?.effort.is_empty());
    store.set_chat_inference(a.id, "changed-model", "high")?;
    let retry = store.prepare("again", "changed-model", Some(a.id))?;
    assert_eq!(
        (retry.model.as_str(), retry.effort.as_str()),
        ("changed-model", "high")
    );
    assert!(store.team_settings()?.lead_effort.is_empty());
    Ok(())
}

#[test]
fn pages_cover_transcript_once_and_keep_first_prompt_separate() -> Result<()> {
    let tmp = tempdir()?;
    let store = Store::open(&tmp.path().join("p"), &tmp.path().join("d"))?;
    let lead = store.prepare("hello", "", None)?;
    for i in 0..125 {
        store.event(lead.id, "agent/message", Some(&i.to_string()))?;
    }
    let mut cursor = i64::MAX;
    let mut pages = Vec::new();
    loop {
        let (rows, older) = store.message_page(lead.id, cursor, 50)?;
        assert!(rows.len() <= 50);
        cursor = rows[0].id;
        pages.push(rows.into_iter().map(|r| r.text).collect::<Vec<_>>());
        if !older {
            break;
        }
    }
    assert_eq!(
        pages.into_iter().rev().flatten().collect::<Vec<_>>(),
        (0..125).map(|v| v.to_string()).collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn worker_policy_inherits_across_projects_and_overrides_per_chat() -> Result<()> {
    use crate::worker_policy::WorkerPolicy;
    let tmp = tempdir()?;
    let home = tmp.path().join("data");
    let p = tmp.path().join("first");
    let first = Store::open(&p, &home)?;
    let other = Store::open(&tmp.path().join("second"), &home)?;
    let global = WorkerPolicy {
        models: vec!["small".into()],
        max_effort: "low".into(),
    };
    let expanded = WorkerPolicy {
        models: vec!["small".into(), "large".into()],
        max_effort: "high".into(),
    };
    first.save_worker_defaults(&global)?;
    assert_eq!(other.worker_defaults()?, global);
    let a = first.prepare_with_policy("hard problem", "", None, Some(&expanded))?;
    first.finish(a.id, "review", "ready")?;
    let separate = first.prepare("ordinary task", "", None)?;
    assert_eq!(first.effective_worker_policy(separate.id)?, global);
    first.finish(separate.id, "review", "ready")?;
    drop(first);
    let first = Store::open(&p, &home)?;
    assert_eq!(first.effective_worker_policy(a.id)?, expanded);
    let retry = first.prepare("try again", "", Some(a.id))?;
    assert_eq!(first.effective_worker_policy(retry.id)?, expanded);
    first.set_chat_worker_policy(a.id, None)?;
    other.save_worker_defaults(&WorkerPolicy::default())?;
    assert_eq!(
        first.effective_worker_policy(a.id)?,
        WorkerPolicy::default()
    );
    assert_eq!(first.effective_worker_policy(retry.id)?, expanded);
    Ok(())
}

#[test]
fn worker_launch_enforces_model_list_and_supported_effort_ceiling() -> Result<()> {
    use crate::worker_policy::WorkerPolicy;
    let tmp = tempdir()?;
    let store = Store::open(&tmp.path().join("p"), &tmp.path().join("d"))?;
    let model = |id: &str, efforts: &[&str]| {
        serde_json::json!({
            "model":id, "isDefault":false, "defaultReasoningEffort":"low",
            "supportedReasoningEfforts":efforts.iter().map(|e|serde_json::json!({"reasoningEffort":e})).collect::<Vec<_>>(),
        })
    };
    let catalog = provider::Catalog {
        models: serde_json::from_value(serde_json::json!([
            model("small", &["low", "high"]),
            model("large", &["medium", "xhigh"])
        ]))?,
        default_model: "small".into(),
        default_effort: "low".into(),
    };
    store.cache_worker_catalog(&catalog);
    let mut policy = WorkerPolicy {
        models: vec!["large".into()],
        max_effort: "high".into(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut task = assignment("one", &[], &[]);
    task.effort = "xhigh".into();
    assert_eq!(
        rt.block_on(store.worker_inference(&policy, &task))?,
        ("large".into(), "medium".into())
    );
    task.model = "small".into();
    assert!(rt.block_on(store.worker_inference(&policy, &task)).is_err());
    policy.models.push("small".into());
    task.effort = "low".into();
    assert_eq!(
        rt.block_on(store.worker_inference(&policy, &task))?,
        ("small".into(), "low".into())
    );
    task.model = "large".into();
    policy.max_effort = "low".into();
    assert!(rt.block_on(store.worker_inference(&policy, &task)).is_err());
    Ok(())
}
