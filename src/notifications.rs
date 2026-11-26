use crate::{App, Desk, Studio, Update};
use anyhow::{Context, Result, ensure};
use rusqlite::Connection;
use slint::ComponentHandle;
use std::{
    path::{Path, PathBuf},
    rc::Rc,
};
use workshop::{Attempt, Store, data_dir};

pub(crate) enum Destination {
    Chat(i64),
    Studio(i64),
}

pub(crate) struct Target {
    project: PathBuf,
    destination: Destination,
}

impl Target {
    pub(crate) fn chat(project: PathBuf, id: i64) -> Self {
        Self {
            project,
            destination: Destination::Chat(id),
        }
    }
    pub(crate) fn studio(project: PathBuf, id: i64) -> Self {
        Self {
            project,
            destination: Destination::Studio(id),
        }
    }
}

fn preference(home: &Path, update: Option<bool>) -> Result<bool> {
    std::fs::create_dir_all(home)?;
    let db = Connection::open(home.join("preferences.sqlite"))?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS notifications(id INTEGER PRIMARY KEY CHECK(id=1), enabled INTEGER NOT NULL)")?;
    if let Some(value) = update {
        db.execute("INSERT INTO notifications VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled", [value])?;
    }
    Ok(db.query_row(
        "SELECT COALESCE((SELECT enabled FROM notifications WHERE id=1),0)",
        [],
        |r| r.get(0),
    )?)
}

#[cfg(test)]
#[test]
fn notifications_are_opt_in_and_remember_the_last_choice() -> Result<()> {
    let home = tempfile::tempdir()?;
    assert!(!preference(home.path(), None)?);
    assert!(preference(home.path(), Some(true))?);
    assert!(preference(home.path(), None)?);
    assert!(!preference(home.path(), Some(false))?);
    assert!(!preference(home.path(), None)?);
    Ok(())
}

pub(crate) fn connect(ui: &Desk) {
    match data_dir().and_then(|p| preference(&p, None)) {
        Ok(enabled) => ui.set_attention_enabled(enabled),
        Err(e) => ui.set_settings_status(e.to_string().into()),
    }
    let weak = ui.as_weak();
    ui.on_save_attention(move |enabled| {
        let ui = weak.unwrap();
        match data_dir().and_then(|p| preference(&p, Some(enabled))) {
            Ok(value) => {
                ui.set_attention_enabled(value);
                ui.set_settings_status("".into());
            }
            Err(e) => ui.set_settings_status(format!("Could not save notifications. {e}").into()),
        }
    });
}

pub(crate) fn chat_reason(
    store: &Store,
    start: &Attempt,
    end: &Attempt,
    had_active_goal: bool,
    previous_output: i64,
    previous_checkpoint: Option<(String, String)>,
) -> Result<Option<&'static str>> {
    if end.state == "failed" {
        return Ok(Some("Chat needs your attention"));
    }
    if end.state != "review" {
        return Ok(None);
    }
    if had_active_goal && let Some(goal) = store.chat_goal(end.id)? {
        match goal.status.as_str() {
            "complete" => return Ok(Some("Goal complete")),
            "paused" => return Ok(Some("Goal needs your attention")),
            _ => {}
        }
    }
    if let Some((_, kind)) = store
        .last_asset_output(end.id)?
        .filter(|(event, _)| *event > previous_output)
    {
        return Ok(Some(if kind == "workshop/model" {
            "3D model ready for review"
        } else {
            "Image ready for review"
        }));
    }
    let changes =
        !end.patch.is_empty() && (start.patch.is_empty() || start.candidate != end.candidate);
    let checkpoint = store.current_checkpoint(end.id)?;
    if (changes || checkpoint.is_some() && checkpoint != previous_checkpoint)
        && store.candidate_ready(end, &store.workers(end.id)?)?
    {
        return Ok(Some(if changes {
            "Changes ready for review"
        } else {
            "Decisions ready for review"
        }));
    }
    Ok(None)
}

#[cfg(test)]
#[test]
fn review_alerts_require_new_actionable_work() -> Result<()> {
    let home = tempfile::tempdir()?;
    let store = Store::open(&home.path().join("project"), &home.path().join("data"))?;
    let start = store.prepare("Update the project", "", None)?;
    store.finish(start.id, "review", "Hello")?;
    let reply = store.attempt(start.id)?;
    assert_eq!(chat_reason(&store, &start, &reply, false, 0, None)?, None);
    store.checkpoint(start.id, "Use the smaller layout", "Update the menu")?;
    assert_eq!(
        chat_reason(&store, &reply, &reply, false, 0, None)?,
        Some("Decisions ready for review")
    );
    let checkpoint = store.current_checkpoint(start.id)?;
    std::fs::write(store.work(start.id).join("layout.txt"), "small")?;
    store.finish(start.id, "review", "Updated")?;
    let edited = store.attempt(start.id)?;
    assert_eq!(
        chat_reason(&store, &reply, &edited, false, 0, checkpoint.clone())?,
        Some("Changes ready for review")
    );
    assert_eq!(
        chat_reason(&store, &edited, &edited, false, 0, checkpoint.clone())?,
        None
    );
    store.status(start.id, "cancelled", "Stopped")?;
    assert_eq!(
        chat_reason(
            &store,
            &reply,
            &store.attempt(start.id)?,
            false,
            0,
            checkpoint
        )?,
        None
    );
    Ok(())
}

impl App {
    pub(crate) fn notify(&self, target: Target, title: &'static str, detail: &str) {
        let ui = self.ui.unwrap();
        if !ui.get_attention_enabled() {
            return;
        }
        let watching = match target.destination {
            Destination::Chat(id) => ui.get_watching_chat() && self.id.get() == Some(id),
            Destination::Studio(id) => {
                ui.get_watching_studio() && i64::from(ui.global::<Studio>().get_selected().id) == id
            }
        };
        if watching && !ui.window().is_minimized() && foreground() {
            return;
        }
        let updates = self.updates.clone();
        let detail: String = detail
            .lines()
            .find(|s| !s.trim().is_empty())
            .unwrap_or("Open Workshop")
            .chars()
            .take(140)
            .collect();
        std::thread::spawn(move || match show(title, &detail) {
            Ok(true) => {
                let _ = updates.send(Update::NotificationClicked(target));
            }
            Ok(false) => {}
            Err(e) => {
                let _ = updates.send(Update::NotificationError(format!(
                    "Could not show a Windows notification. {e:#}"
                )));
            }
        });
    }
    pub(crate) fn open_notification(self: &Rc<Self>) -> Result<()> {
        let ui = self.ui.unwrap();
        if ui.get_busy() {
            return Ok(());
        }
        // Switching projects releases its Studio job. Defer that navigation until the job finishes.
        if ui.global::<Studio>().get_working()
            && let Some(target) = self.pending_notification.borrow().as_ref()
            && self
                .store
                .borrow()
                .as_ref()
                .is_some_and(|s| s.project != target.project)
        {
            return Ok(());
        }
        let Some(target) = self.pending_notification.take() else {
            return Ok(());
        };
        let store = self.store.borrow().clone();
        let Some(store) = store.filter(|s| s.project == target.project) else {
            self.open_to(target.project.clone(), Some(target));
            return Ok(());
        };
        ui.window().set_minimized(false);
        ui.show()?;
        match target.destination {
            Destination::Chat(id) => {
                if store.is_deleted_chat(id)? {
                    ui.set_notice("This chat was deleted.".into());
                    return Ok(());
                }
                self.chat_offset.set(store.chat_position(id)? / 40 * 40);
                ui.invoke_show_destination(false);
                self.select_chat(Some(id));
                self.pages.borrow_mut().clear();
                self.render()?;
                let attempt = store.attempt(id)?;
                if attempt.state == "failed" {
                    ui.set_notice(attempt.action.into());
                }
                Self::scroll_after_layout(&ui);
            }
            Destination::Studio(id) => {
                let job = store.studio_job(id)?;
                self.close_inspector();
                let studio = ui.global::<Studio>();
                studio.set_editing_run(false);
                studio.set_area(
                    if job.recipe.area() == "Ghidra" {
                        "Ghidra"
                    } else {
                        "Assets"
                    }
                    .into(),
                );
                studio.set_query("".into());
                ui.invoke_show_destination(true);
                self.refresh_studio(&store, Some(id))?;
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
fn foreground() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };
    let mut pid = 0;
    // The shell owns the window handle. This API only writes the owning PID.
    unsafe {
        GetWindowThreadProcessId(GetForegroundWindow(), &mut pid);
    }
    pid == std::process::id()
}

#[cfg(not(windows))]
fn foreground() -> bool {
    false
}

fn show(title: &str, detail: &str) -> Result<bool> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::process::Command;
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").context("Cannot find Windows")?)
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut command = Command::new(powershell);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let script: Vec<u8> = include_str!("notification.ps1")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let response = command
        .args(["-NoProfile", "-NonInteractive", "-STA", "-EncodedCommand"])
        .arg(STANDARD.encode(script))
        .env("WORKSHOP_NOTICE_TITLE", title)
        .env("WORKSHOP_NOTICE_DETAIL", detail)
        .env("WORKSHOP_NOTICE_PARENT", std::process::id().to_string())
        .output()?;
    ensure!(
        response.status.success(),
        "{}",
        String::from_utf8_lossy(&response.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&response.stdout).trim() == "clicked")
}
