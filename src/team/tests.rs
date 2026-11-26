use super::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn pack_tools_share_visual_references_and_copy_exact_assets_into_each_assignment() -> Result<()> {
    let tmp = tempdir()?;
    let home = tmp.path().join("home");
    let store = Store::open(&tmp.path().join("project"), &home)?;
    let lead = store.prepare("Design a harbor", "", None)?;
    let child = store.child_store(lead.id, 9)?;
    let worker = child.prepare("Use the harbor assets", "", None)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let call = |s: &Store, a: &Attempt, args: Value| {
        runtime.block_on(dispatch_tool(
            s,
            a,
            &json!({"tool":"workshop_asset_pack","arguments":args}),
            &|_| {},
        ))
    };
    call(
        &child,
        &worker,
        json!({"action":"create","pack":"harbor","name":"Harbor","style":"Dark painted iron"}),
    )?;
    assert!(
        store
            .asset_packs()
            .create("../escape", "bad", "bad")
            .is_err()
    );
    assert!(
        store
            .asset_packs()
            .create("harbor", "Other", "different")
            .is_err()
    );
    // Real Blender publication is covered by tests/blender_packs.py. Seed its output
    // here to exercise native provider envelopes, shared lookup and review isolation.
    let lib = store.asset_packs();
    let dir = lib.member_dir("harbor", "buoy")?;
    fs::create_dir(&dir)?;
    let record = json!({"key":"buoy","name":"Buoy","description":"Teal iron marker","source_sha256":"fixture","glb_sha256":"fixture",
        "blend_sha256":"fixture","preview_sha256":"fixture","collection":"Buoy","meshes":1,"triangles":320,
        "dimensions":[1.0,1.0,2.0],"meters_per_unit":0.5,"materials":[{"name":"Iron","color":"#224455"}],"palette":["#337799"]});
    fs::write(dir.join("asset.json"), serde_json::to_vec(&record)?)?;
    let png = dir.join("preview.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([33, 77, 99, 255])).save(&png)?;
    fs::write(dir.join("model.glb"), b"saved mesh bytes")?;
    fs::write(dir.join("model.blend"), b"saved native bytes")?;
    store.finish(lead.id, "review", "Published the pack")?;
    let later = store.prepare("Build a dock", "", None)?;
    let inspect = json!({"action":"inspect","pack":"harbor","asset":"buoy"});
    let response = call(&child, &worker, inspect.clone())?;
    let png_bytes = fs::read(&png)?;
    assert_eq!(response.image.as_deref(), Some(png_bytes.as_slice()));
    let codex = ToolOutput::codex(Ok(response));
    assert_eq!(
        codex["contentItems"][1]["imageUrl"],
        format!("data:image/png;base64,{}", STANDARD.encode(&png_bytes))
    );
    let mcp = ToolOutput::mcp(call(&store, &later, inspect));
    assert_eq!(mcp["content"][1]["data"], STANDARD.encode(png_bytes));
    assert_eq!(mcp["content"][1]["mimeType"], "image/png");
    for (s, a) in [(&store, &later), (&child, &worker)] {
        let use_it = |dest| {
            call(
                s,
                a,
                json!({"action":"use","pack":"harbor","asset":"buoy","destination":dest}),
            )
        };
        use_it("Art/Buoy.blend")?;
        use_it("Art/Buoy.blend")?;
        use_it("Art/Buoy.glb")?;
        assert!(use_it("../Buoy.glb").is_err());
        assert_eq!(
            fs::read(s.work(a.id).join("Art/Buoy.blend"))?,
            b"saved native bytes"
        );
        fs::write(s.work(a.id).join("Art/Buoy.glb"), b"local change")?;
        assert!(use_it("Art/Buoy.glb").is_err());
        assert_eq!(
            fs::read(s.work(a.id).join("Art/Buoy.glb"))?,
            b"local change"
        );
    }
    assert!(!store.project.join("Art").exists());
    let other = Store::open(&tmp.path().join("other"), &home)?;
    assert!(other.asset_packs().list()?.is_empty());
    Ok(())
}

#[test]
fn generated_assets_stay_with_the_chat_and_respect_provider_permissions() -> Result<()> {
    use crate::provider_permissions::{ProviderPermissions, TaskRole};
    let root = tempdir()?;
    let project = root.path().join("project");
    let home = root.path().join("data");
    let store = Store::open(&project, &home)?;
    let lead = store.prepare("Make a tile", "", None)?;
    let tool = "workshop_generate_asset";
    assert!(
        tools(false)
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == tool)
    );
    assert!(
        !tools(true)
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == tool)
    );
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let call = |arguments| {
        rt.block_on(dispatch(
            &store,
            &lead,
            &json!({"tool":tool,"arguments":arguments}),
            &|_| {},
        ))
    };
    for provider in ["codex", "antigravity"] {
        ProviderPermissions::set(&home, provider, TaskRole::Images, false)?;
    }
    assert!(
        call(json!({"kind":"image","brief":"A forest"}))
            .unwrap_err()
            .to_string()
            .contains("Enable a provider")
    );
    assert!(
        call(json!({"kind":"image","brief":"Use a reference","reference_image":"chat"})).is_err()
    );
    let result: Value = serde_json::from_str(&call(
        json!({"kind":"texture","material":"Moss","seed":42}),
    )?)?;
    let job = result["id"].as_i64().unwrap();
    assert_eq!(store.studio_job(job)?.state, "Ready");
    let receipt = store.messages_after(lead.id, 0)?.pop().unwrap();
    assert_eq!(receipt.asset, Some(job));
    assert_eq!(receipt.maker.as_deref(), Some("Local texture generator"));
    assert!(store.is_chat_image(receipt.image.as_ref().unwrap()));
    assert!(!store.work(lead.id).join("image.png").exists());
    store.finish(lead.id, "review", "Delivered")?;
    let other = store.prepare("Another conversation", "", None)?;
    assert!(store.chat_outputs(Some(other.id))?.is_empty());
    drop(store);
    let reopened = Store::open(&project, &home)?;
    let outputs = reopened.chat_outputs(Some(lead.id))?;
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].asset, Some(job));
    assert!(outputs[0].preview.as_ref().unwrap().is_file());
    assert_eq!(reopened.studio_history("Assets")?.len(), 1);
    Ok(())
}

#[test]
fn posted_image_survives_project_edits_and_restricts_sources_to_the_working_copy() -> Result<()> {
    let tmp = tempdir()?;
    let store = Store::open(&tmp.path().join("p"), &tmp.path().join("data"))?;
    let lead = store.prepare("Show the result", "", None)?;
    let work = store.work(lead.id);
    let source = work.join("result.png");
    let pixels = image::RgbaImage::from_pixel(12, 18, image::Rgba([10, 20, 30, 77]));
    pixels.save(&source)?;
    let outside = tmp.path().join("outside.png");
    pixels.save(&outside)?;
    let rt = tokio::runtime::Builder::new_current_thread().build()?;
    let send = |path: &str| {
        rt.block_on(dispatch(&store, &lead,
        &json!({"tool":"workshop_send_image","arguments":{"path":path,"caption":"Draft layout"}}), &|_| {}))
    };
    assert!(send(outside.to_str().unwrap()).is_err());
    send("result.png")?;
    fs::remove_file(source)?;
    let messages = store.messages_after(lead.id, 0)?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "Draft layout");
    assert!(!messages[0].from_user);
    assert_eq!(
        image::open(messages[0].image.as_ref().unwrap())?.into_rgba8(),
        pixels
    );
    Ok(())
}

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
                    "workshop_asset_pack",
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
fn a_worker_inherits_the_latest_delivered_direction() -> Result<()> {
    let tmp = tempdir()?;
    let store = Store::open(&tmp.path().join("project"), &tmp.path().join("home"))?;
    let lead = store.prepare("Make a menu", "", None)?;
    store.event(lead.id, "user/followup", Some("Use red"))?;
    store.event(lead.id, "steer/sent", Some("Use blue"))?;
    store.direction_unavailable(lead.id, &"Use orange".into(), "Delivery interrupted.")?;
    let context = store.assignment_context(lead.id, &assignment("menu", &[], &["menu"]))?;
    assert!(context.contains("Current request: Use blue"));
    assert!(!context.contains("Use orange"));
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
         ALTER TABLE team_settings DROP COLUMN override_worker_model;
         ALTER TABLE team_settings DROP COLUMN override_worker_effort;
         ALTER TABLE team_settings DROP COLUMN auto_delegate;
         UPDATE team_settings SET worker_model='existing-worker';",
    )?;
    drop(store);
    let store = Store::open(&project, &home)?;
    assert_eq!(store.team_settings()?.worker_model, "existing-worker");
    assert_eq!(
        store.worker_defaults_with_source()?.0.models,
        ["existing-worker"]
    );
    store.save_worker_defaults(&crate::worker_policy::WorkerPolicy::default())?;
    assert_eq!(
        store.worker_defaults_with_source()?,
        (Default::default(), false)
    );
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
    let mut settings = first.team_settings()?;
    settings.worker_model = "project-model".into();
    settings.override_worker_model = true;
    first.configure_team(&settings)?;
    assert_eq!(first.effective_worker_policy(a.id)?, expanded);
    first.set_chat_worker_policy(a.id, None)?;
    assert_eq!(
        first.effective_worker_policy(a.id)?,
        WorkerPolicy {
            models: vec!["project-model".into()],
            max_effort: "low".into()
        }
    );
    other.save_worker_defaults(&WorkerPolicy::default())?;
    assert_eq!(
        first.effective_worker_policy(a.id)?,
        WorkerPolicy {
            models: vec!["project-model".into()],
            max_effort: String::new()
        }
    );
    assert_eq!(other.worker_defaults()?, WorkerPolicy::default());
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
        rt.block_on(store.worker_inference(&policy, &task, "codex"))?,
        ("large".into(), "medium".into())
    );
    task.model = "small".into();
    assert!(
        rt.block_on(store.worker_inference(&policy, &task, "codex"))
            .is_err()
    );
    policy.models.push("small".into());
    task.effort = "low".into();
    assert_eq!(
        rt.block_on(store.worker_inference(&policy, &task, "codex"))?,
        ("small".into(), "low".into())
    );
    task.model = "large".into();
    policy.max_effort = "low".into();
    assert!(
        rt.block_on(store.worker_inference(&policy, &task, "codex"))
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn inherited_route_skips_unavailable_providers_but_keeps_native_defaults() -> Result<()> {
    use crate::provider_permissions::{ProviderPermissions, TaskRole};
    use crate::worker_policy::WorkerPolicy;

    let tmp = tempdir()?;
    let home = tmp.path().join("data");
    let mut store = Store::open(&tmp.path().join("project"), &home)?;
    let allowed = WorkerPolicy::default();
    let model = serde_json::json!({
        "model": "antigravity:default", "isDefault": false,
        "defaultReasoningEffort": "", "supportedReasoningEfforts": []
    });
    store.cache_worker_catalog(&provider::Catalog {
        models: vec![serde_json::from_value(model)?],
        ..Default::default()
    });
    // An uncached provider cannot start discovery in this directory.
    store.project = tmp.path().join("missing-catalog-directory");
    assert_eq!(
        store
            .policy_inference(TaskRole::Images, &allowed, "", "", "claude")
            .await?,
        ("antigravity:default".into(), String::new()),
    );
    assert_eq!(
        store
            .policy_inference(TaskRole::Images, &allowed, "", "", "codex")
            .await?,
        (String::new(), String::new()),
    );
    ProviderPermissions::set(&home, "antigravity", TaskRole::Images, false)?;
    assert_eq!(
        store
            .policy_inference(TaskRole::Images, &allowed, "", "", "claude")
            .await?,
        (String::new(), String::new()),
    );
    Ok(())
}

#[tokio::test]
async fn provider_roles_apply_to_inherited_workers_and_saved_studio_choices() -> Result<()> {
    use crate::provider_permissions::{PROVIDERS, ProviderPermissions, TaskRole};
    use crate::worker_policy::WorkerPolicy;

    let tmp = tempdir()?;
    let home = tmp.path().join("home");
    let policy = WorkerPolicy::default();
    let task = assignment("one", &[], &[]);
    // Permissions can be set before the first project is opened.
    ProviderPermissions::set(&home, "claude", TaskRole::Workers, false)?;
    let store = Store::open(&tmp.path().join("project"), &home)?;
    let cached = ["fixture", "claude:default", "antigravity:default"].map(|id| {
        serde_json::json!({"model": id, "isDefault": false, "defaultReasoningEffort": "low",
            "supportedReasoningEfforts": [{"reasoningEffort":"low"}]})
    });
    store.cache_worker_catalog(&provider::Catalog {
        models: serde_json::from_value(serde_json::json!(cached))?,
        default_model: "fixture".into(),
        default_effort: "low".into(),
    });
    assert_eq!(
        store.worker_inference(&policy, &task, "claude").await?,
        ("".into(), "".into())
    );
    let pinned = WorkerPolicy {
        models: vec!["claude:default".into()],
        max_effort: "low".into(),
    };
    assert!(
        store
            .worker_inference(&pinned, &task, "claude")
            .await
            .is_err()
    );
    let mut explicit = task.clone();
    explicit.model = "claude:default".into();
    assert!(
        store
            .worker_inference(&policy, &explicit, "codex")
            .await
            .is_err()
    );
    assert_eq!(
        store
            .policy_inference(TaskRole::Models, &pinned, "", "", "codex")
            .await?
            .0,
        "claude:default"
    );

    ProviderPermissions::set(&home, "codex", TaskRole::Images, false)?;
    assert!(ProviderPermissions::set(&home, "claude", TaskRole::Images, true).is_err());
    assert_eq!(
        store
            .policy_inference(TaskRole::Images, &policy, "", "", "codex")
            .await?
            .0,
        "antigravity:default"
    );
    ProviderPermissions::set(&home, "antigravity", TaskRole::Images, false)?;
    assert!(
        store
            .policy_inference(TaskRole::Images, &policy, "", "", "codex")
            .await
            .unwrap_err()
            .to_string()
            .contains("Settings → Providers")
    );

    let second = Store::open(&tmp.path().join("second"), &home)?;
    for p in PROVIDERS {
        ProviderPermissions::set(&home, p, TaskRole::Workers, false)?;
    }
    assert!(
        second
            .worker_inference(&policy, &task, "codex")
            .await
            .is_err()
    );
    ProviderPermissions::set(&home, "claude", TaskRole::Workers, true)?;
    assert_eq!(
        store.worker_inference(&pinned, &task, "codex").await?,
        ("claude:default".into(), "low".into())
    );
    Ok(())
}
