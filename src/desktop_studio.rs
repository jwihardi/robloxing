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
const IMAGE_STYLES: [&str; 3] = [
    "Concept art",
    "A single inventory icon on a transparent background",
    "A grid-aligned sprite sheet on a transparent background",
];

fn item(job: &Job) -> StudioItem {
    StudioItem {
        id: job.id as i32,
        title: job.title.as_str().into(),
        state: job.state.as_str().into(),
        date: job.created.as_str().into(),
        detail: job.detail.as_str().into(),
    }
}

fn recipe(studio: &Studio<'_>) -> Result<Recipe> {
    let area = studio.get_area();
    Ok(match area.as_str() {
        "Images" if studio.get_image_mode() == 3 => Recipe::Texture {
            material: ["Moss", "Basalt", "Sand"][studio.get_material() as usize].into(),
            seed: studio
                .get_seed()
                .trim()
                .parse()
                .context("Enter a seed between 0 and 4294967295.")?,
        },
        "Images" => Recipe::Image {
            brief: studio.get_brief().to_string(),
            style: IMAGE_STYLES
                .get(studio.get_image_mode() as usize)
                .map(|s| s.to_string())
                .unwrap_or_else(|| studio.get_custom_style().to_string()),
        },
        "Models" if studio.get_model_mode() == 0 => Recipe::ModelBrief {
            brief: studio.get_model_brief().to_string(),
        },
        "Models" if studio.get_model_mode() == 4 => Recipe::Blend {
            source: PathBuf::from(studio.get_scene_path().as_str()),
        },
        "Models" => Recipe::Model {
            template: ["Crystal", "Barrel", "Planter"][(studio.get_model_mode() - 1) as usize]
                .into(),
            name: studio.get_model_name().to_string(),
        },
        "Ghidra" => Recipe::Decompile {
            binary: PathBuf::from(studio.get_binary_path().as_str()),
            filter: studio.get_filter().to_string(),
        },
        _ => bail!("Select a studio area."),
    })
}
impl App {
    pub(super) fn close_inspector(&self) {
        self.inspector.take();
        let ui = self.ui.unwrap();
        let studio = ui.global::<Studio>();
        studio.set_inspecting(false);
        studio.set_inspect_image(Default::default());
        studio.set_inspect_ready(false);
        studio.set_inspect_result("".into());
        studio.set_inspect_faces("".into());
        studio.set_inspect_error("".into());
    }
    pub(super) fn refresh_studio(&self, store: &Store, selection: Option<i64>) -> Result<()> {
        let ui = self.ui.unwrap();
        let studio = ui.global::<Studio>();
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
    fn studio_command(&self, cmd: &str) -> Result<()> {
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
        let selected = || store.studio_job(studio.get_selected().id.into());
        match cmd {
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
                studio.set_inspecting(true);
            }
            "refresh" | "search" => {
                self.refresh_studio(&store, Some(studio.get_selected().id.into()))?
            }
            "run" | "retry" | "rebuild" | "fresh" => {
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
                    None if cmd == "rebuild" => Recipe::Blend {
                        source: selected()?.folder(&store).join("model.blend"),
                    },
                    _ => recipe(&studio)?,
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
                studio.set_message(format!("Running in {}…", request.area()).into());
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
                match job.recipe {
                    Recipe::Image { brief, style } => {
                        if let Some(i) = IMAGE_STYLES.iter().position(|s| *s == style) {
                            studio.set_image_mode(i as i32);
                        } else {
                            studio.set_custom_style(style.into());
                            studio.set_image_mode(4);
                        }
                        studio.set_brief(brief.into());
                    }
                    Recipe::Texture { material, seed } => {
                        studio.set_image_mode(3);
                        studio.set_material(match material.as_str() {
                            "Basalt" => 1,
                            "Sand" => 2,
                            _ => 0,
                        });
                        studio.set_seed(seed.to_string().into());
                    }
                    Recipe::ModelBrief { brief } => {
                        studio.set_model_mode(0);
                        studio.set_model_brief(brief.into());
                    }
                    Recipe::Model { template, name } => {
                        studio.set_model_mode(match template.as_str() {
                            "Barrel" => 2,
                            "Planter" => 3,
                            _ => 1,
                        });
                        studio.set_model_name(name.into());
                    }
                    Recipe::Blend { source } => {
                        studio.set_model_mode(4);
                        studio.set_scene_path(source.to_string_lossy().as_ref().into());
                    }
                    Recipe::Decompile { binary, filter } => {
                        studio.set_binary_path(binary.to_string_lossy().as_ref().into());
                        studio.set_filter(filter.into());
                    }
                }
            }
            "pick" => {
                let dialog = rfd::FileDialog::new().set_directory(&store.project);
                if studio.get_area() == "Models" {
                    if let Some(path) = dialog.add_filter("Blender scene", &["blend"]).pick_file() {
                        studio.set_scene_path(path.to_string_lossy().as_ref().into());
                    }
                } else if let Some(path) =
                    dialog.set_title("Choose a local game binary").pick_file()
                {
                    studio.set_binary_path(path.to_string_lossy().as_ref().into());
                }
            }
            "locate" => {
                let (name, path) = if studio.get_area() == "Models" {
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
                            .set_title("Save studio asset")
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
        if let Err(e) = a.studio_command(&cmd) {
            a.ui.unwrap()
                .global::<Studio>()
                .set_message(format!("{e:#}").into());
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
