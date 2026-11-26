use super::{App, Desk, Studio, StudioItem, TaskControls, Update};
use anyhow::{Context, Result, bail, ensure};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    sync::Arc,
};
use workshop::{
    Store,
    studio::{Job, Recipe},
};
fn item(job: &Job) -> StudioItem {
    StudioItem {
        id: job.id as i32,
        area: job.recipe.area().into(),
        title: job.title.as_str().into(),
        state: job.state.as_str().into(),
        date: job.created.as_str().into(),
        detail: job.detail.as_str().into(),
    }
}

impl App {
    pub(super) fn stop_sound(&self) {
        let ui = self.ui.unwrap();
        let studio = ui.global::<Studio>();
        if studio.get_playing_sound() != 0 {
            self.sound_timer.stop();
            #[cfg(windows)]
            unsafe {
                windows_sys::Win32::Media::Audio::PlaySoundW(
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    0,
                );
            }
            studio.set_playing_sound(0);
        }
    }

    #[cfg(windows)]
    fn play_sound(&self, job: &Job, store: &Store) -> Result<()> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Media::Audio::{
            PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT,
        };
        ensure!(
            job.state == "Ready" && matches!(job.recipe, Recipe::Sound { .. }),
            "Select a finished sound effect."
        );
        let ui = self.ui.unwrap();
        let stopping = ui.global::<Studio>().get_playing_sound() == job.id as i32;
        self.stop_sound();
        if stopping {
            return Ok(());
        }
        let file = job.folder(store).join(job.filename());
        let mut audio = fs::File::open(&file)?;
        let mut header = [0u8; 44];
        audio.read_exact(&mut header)?;
        let bytes = u32::from_le_bytes(header[40..44].try_into()?);
        let bytes_per_second = u32::from_le_bytes(header[28..32].try_into()?);
        ensure!(
            bytes_per_second > 0 && &header[8..12] == b"WAVE",
            "Invalid WAV preview."
        );
        // Workshop renders mono, 16-bit PCM with a 44-byte header.
        let duration =
            std::time::Duration::from_secs_f64(f64::from(bytes) / f64::from(bytes_per_second));
        let name: Vec<_> = file.as_os_str().encode_wide().chain(Some(0)).collect();
        ensure!(
            unsafe {
                PlaySoundW(
                    name.as_ptr(),
                    std::ptr::null_mut(),
                    SND_FILENAME | SND_ASYNC | SND_NODEFAULT,
                )
            } != 0,
            "Windows could not play this sound. Check your audio output."
        );
        ui.global::<Studio>().set_playing_sound(job.id as i32);
        let weak = ui.as_weak();
        self.sound_timer
            .start(slint::TimerMode::SingleShot, duration, move || {
                if let Some(ui) = weak.upgrade() {
                    ui.global::<Studio>().set_playing_sound(0);
                }
            });
        Ok(())
    }

    pub(super) fn close_inspector(&self) {
        self.inspector.take();
        let ui = self.ui.unwrap();
        let studio = ui.global::<Studio>();
        studio.set_inspecting(false);
        studio.set_inspect_id(0);
        studio.set_inspect_title("".into());
        studio.set_inspect_image(Default::default());
        studio.set_inspect_ready(false);
        studio.set_inspect_result("".into());
        studio.set_inspect_faces("".into());
        studio.set_inspect_error("".into());
    }
    pub(super) fn refresh_studio(&self, store: &Store, selection: Option<i64>) -> Result<()> {
        let ui = self.ui.unwrap();
        let studio = ui.global::<Studio>();
        if studio.get_area() == "Packs" {
            return self.refresh_packs(store, None);
        }
        let jobs = store.find_studio_jobs(&studio.get_area(), &studio.get_query(), false, None)?;
        studio.set_custom_task(
            self.task_overrides
                .borrow()
                .contains_key(studio.get_area().as_str()),
        );
        let rows: Vec<_> = jobs.iter().map(item).collect();
        studio.set_jobs(ModelRc::new(VecModel::from(rows)));
        let selected = jobs
            .iter()
            .find(|j| Some(j.id) == selection)
            .or_else(|| jobs.first());
        self.select_studio_job(store, selected)
    }
    fn select_studio_job(&self, store: &Store, job: Option<&Job>) -> Result<()> {
        let ui = self.ui.unwrap();
        let studio = ui.global::<Studio>();
        let next = job.map_or(0, |j| j.id);
        if i64::from(studio.get_selected().id) != next {
            studio.set_show_summary(false);
            studio.set_show_log(job.is_some_and(|j| j.state != "Ready"));
        }
        if let Some(report) = job.filter(|j| j.recipe.area() == "Ghidra") {
            studio.set_run_log(workshop::studio::reports::log_tail(report, store)?.into());
        } else {
            studio.set_run_log("".into());
        }
        if studio.get_inspecting()
            && job.is_none_or(|j| j.id != i64::from(studio.get_selected().id) || j.state != "Ready")
        {
            self.close_inspector();
        }
        studio.set_selected(job.map(item).unwrap_or_default());
        studio.set_code("".into());
        studio.set_ai_summary("".into());
        studio.set_preview(Default::default());
        if let Some(job) = job.filter(|j| j.state == "Ready") {
            let folder = job.folder(store);
            if job.recipe.area() == "Ghidra" {
                if job.settings.summarize {
                    let summary =
                        fs::read_to_string(folder.join("ai-summary.md")).unwrap_or_else(|_| {
                            "Open the run log for the status of this summary.".into()
                        });
                    studio.set_ai_summary(summary.into());
                }
                let mut bytes = Vec::new();
                let file = fs::File::open(folder.join(job.filename()))?;
                file.take(96 * 1024 + 1).read_to_end(&mut bytes)?;
                let more = bytes.len() > 96 * 1024;
                bytes.truncate(96 * 1024);
                let mut code = String::from_utf8_lossy(&bytes).into_owned();
                if more {
                    code.push_str(
                        "\n\n/* Preview ends here. Save the report for all functions. */",
                    );
                }
                studio.set_code(code.into());
            } else {
                studio.set_preview(
                    slint::Image::load_from_path(&folder.join("preview.png"))
                        .context("Could not load the asset preview.")?,
                );
            }
        }
        Ok(())
    }
    fn studio_command(&self, cmd: &str, asset: Option<i64>) -> Result<()> {
        let ui = self.ui.unwrap();
        if cmd == "close-inspector" {
            self.close_inspector();
            return Ok(());
        }
        if cmd == "search" && !ui.get_studio_open() {
            return Ok(());
        }
        let studio = ui.global::<Studio>();
        let store = self
            .store
            .borrow()
            .clone()
            .context("Open a project first.")?;
        if !studio.get_working() {
            studio.set_message("".into());
        }
        let selected =
            || store.studio_job(asset.unwrap_or_else(|| studio.get_selected().id.into()));
        match cmd {
            #[cfg(windows)]
            "play" => self.play_sound(&selected()?, &store)?,
            #[cfg(not(windows))]
            "play" => bail!("Save the WAV and open it in your audio player."),
            "edit-task" => {
                ui.global::<TaskControls>()
                    .invoke_open(true, studio.get_area());
                studio.set_editing_run(true);
            }
            "log-tick" => {
                let job = selected()?;
                if studio.get_selected().state.as_str() != job.state {
                    self.refresh_studio(&store, Some(job.id))?;
                }
                let log = workshop::studio::reports::log_tail(&job, &store)?;
                if let Some((_, event)) = log.lines().last().and_then(|s| s.split_once("  "))
                    && (job.state == "Running"
                        || (studio.get_working() && event.starts_with("Requesting AI summary")))
                {
                    studio.set_message(event.into());
                }
                if log != studio.get_run_log().as_str() {
                    studio.set_run_log(log.into());
                }
            }
            "inspect" => {
                let job = selected()?;
                ensure!(
                    job.recipe.area() == "Models" && job.state == "Ready",
                    "Select a finished model."
                );
                self.close_inspector();
                self.inspector
                    .replace(Some(super::inspector::Inspector::open(
                        &ui,
                        job.folder(&store).join(job.filename()),
                    )));
                studio.set_inspect_id(job.id as i32);
                studio.set_inspect_title(job.title.into());
                studio.set_inspecting(true);
            }
            "refresh" | "search" => {
                self.refresh_studio(&store, Some(studio.get_selected().id.into()))?
            }
            "run" | "retry" | "fresh" => {
                ensure!(
                    !studio.get_working(),
                    "Wait for the active run or cancel it."
                );
                let previous = if matches!(cmd, "retry" | "fresh") {
                    Some(selected()?)
                } else {
                    None
                };
                let request = match &previous {
                    Some(job) => job.recipe.clone(),
                    None => Recipe::Decompile {
                        binary: PathBuf::from(studio.get_binary_path().as_str()),
                        filter: studio.get_filter().to_string(),
                    },
                };
                let settings = previous.as_ref().map(|j| j.settings.clone()).or_else(|| {
                    if request.uses_task_settings() {
                        self.task_overrides.borrow().get(request.area()).cloned()
                    } else {
                        None
                    }
                });
                let id = store.queue_studio_job(&request, settings.as_ref())?;
                if cmd == "run" && request.uses_task_settings() {
                    self.task_overrides.borrow_mut().remove(request.area());
                }
                studio.set_query("".into());
                let (stop, cancel) = tokio::sync::oneshot::channel();
                self.studio_cancel.replace(Some(stop));
                studio.set_working(true);
                studio.set_message("Decompiling the binary…".into());
                let tx = self.updates.clone();
                let worker = Arc::clone(&store);
                let fresh = cmd == "fresh";
                std::thread::spawn(move || {
                    let result = worker.run_studio_job_with_refresh(id, cancel, fresh);
                    let _ = tx.send(Update::StudioFinished(worker.project.clone(), id, result));
                });
                self.refresh_studio(&store, Some(id))?;
            }
            "cancel" => {
                if let Some(cancel) = self.studio_cancel.take() {
                    let _ = cancel.send(());
                }
                studio.set_message("Cancelling…".into());
            }
            "reuse" => {
                let job = selected()?;
                if job.recipe.uses_task_settings() {
                    self.task_overrides
                        .borrow_mut()
                        .insert(job.recipe.area().into(), job.settings);
                    studio.set_custom_task(true);
                }
                if let Recipe::Decompile { binary, filter } = job.recipe {
                    studio.set_binary_path(binary.to_string_lossy().as_ref().into());
                    studio.set_filter(filter.into());
                }
            }
            "pick" => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&store.project)
                    .set_title("Choose a local game binary")
                    .pick_file()
                {
                    studio.set_binary_path(path.to_string_lossy().as_ref().into());
                }
            }
            "locate" => {
                let (name, path) = if studio.get_area() == "Assets" {
                    (
                        "Blender",
                        rfd::FileDialog::new()
                            .set_title("Locate Blender executable")
                            .add_filter("Application", &["exe"])
                            .pick_file(),
                    )
                } else {
                    (
                        "Ghidra",
                        rfd::FileDialog::new()
                            .set_title("Locate Ghidra installation folder")
                            .pick_folder(),
                    )
                };
                if let Some(path) = path {
                    store.set_studio_tool(name, &path)?;
                    studio.set_message(format!("{name} connected").into());
                }
            }
            "files" => {
                Command::new("explorer.exe")
                    .arg(selected()?.folder(&store))
                    .spawn()?;
            }
            "export" | "attach" | "annotate" | "edit" => {
                let job = selected()?;
                ensure!(job.state == "Ready", "Select a finished asset.");
                let folder = job.folder(&store);
                let file = folder.join(job.filename());
                match cmd {
                    "export" => {
                        let extension = file.extension().and_then(|v| v.to_str()).unwrap_or("bin");
                        let stem: String = job
                            .title
                            .chars()
                            .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
                            .take(55)
                            .collect();
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("Save asset")
                            .set_directory(&store.project)
                            .set_file_name(format!("{}.{}", stem.trim(), extension))
                            .add_filter(extension, &[extension])
                            .save_file()
                        {
                            let mut temp = tempfile::NamedTempFile::new_in(
                                path.parent().unwrap_or(Path::new(".")),
                            )?;
                            std::io::copy(&mut fs::File::open(&file)?, temp.as_file_mut())?;
                            temp.persist(&path)?;
                            studio.set_message(format!("Saved {}", path.display()).into());
                        }
                    }
                    "annotate" => {
                        self.annotate(&file)?;
                    }
                    "attach" => {
                        ensure!(!ui.get_busy(), "Wait for the current chat operation.");
                        ui.invoke_open_asset_draft();
                        let mut all = self.attachments.borrow_mut();
                        let draft = all.entry(self.id.get()).or_default();
                        if !draft.contains(&file) {
                            draft.push(file);
                        }
                        drop(all);
                        self.render_attachments();
                        ui.invoke_focus_draft();
                    }
                    _ => {
                        Command::new(store.studio_tool("Blender")?)
                            .arg("--disable-autoexec")
                            .arg(folder.join("model.blend"))
                            .spawn()?;
                    }
                }
            }
            _ => bail!("Unknown studio action"),
        }
        Ok(())
    }
}

pub(super) fn connect(ui: &Desk, app: &Rc<App>) {
    let a = app.clone();
    ui.global::<Studio>()
        .on_inspect_view(move |yaw, pitch, zoom, mode, w, h| {
            if let Some(inspector) = a.inspector.borrow().as_ref() {
                inspector.request(yaw, pitch, zoom, mode, w, h);
            }
        });
    let a = app.clone();
    ui.global::<Studio>().on_command(move |cmd| {
        if let Err(e) = a.studio_command(&cmd, None) {
            let ui = a.ui.unwrap();
            ui.global::<Studio>().set_message(format!("{e:#}").into());
            if ui.global::<Studio>().get_area() == "Packs" {
                ui.global::<super::AssetPacks>()
                    .set_message(format!("{e:#}").into());
            }
        }
    });
    let a = app.clone();
    ui.global::<Studio>().on_asset_action(move |id, cmd| {
        if let Err(e) = a.studio_command(&cmd, Some(i64::from(id))) {
            let ui = a.ui.unwrap();
            ui.set_notice(format!("{e:#}").into());
            ui.global::<Studio>().set_message(format!("{e:#}").into());
        }
    });
    let a = app.clone();
    ui.global::<Studio>().on_select(move |id| {
        if let Some(store) = a.store.borrow().as_ref()
            && let Err(e) = store
                .studio_job(id.into())
                .and_then(|job| a.select_studio_job(store, Some(&job)))
        {
            a.ui.unwrap()
                .global::<Studio>()
                .set_message(format!("{e:#}").into());
        }
    });
}
