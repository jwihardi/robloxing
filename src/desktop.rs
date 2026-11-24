#![cfg_attr(windows, windows_subsystem = "windows")]
use anyhow::{Context, Result};
use slint::{ComponentHandle, ModelRc, StandardListViewItem, Timer, TimerMode, VecModel};
use std::{
    cell::{Cell, RefCell},
    fs,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, mpsc},
    time::Duration,
};
use workshop::{
    Attempt, Store, data_dir,
    provider::{self, Control},
};

slint::slint! { #[edition = "1.19"] export {Desk} from "ui/workshop.slint"; }

enum Update {
    Opened(Arc<Store>),
    Started(i64),
    Finished(Result<String>),
}
struct App {
    ui: slint::Weak<Desk>,
    store: RefCell<Option<Arc<Store>>>,
    attempts: RefCell<Vec<Attempt>>,
    id: Cell<Option<i64>>,
    control: RefCell<Option<tokio::sync::mpsc::UnboundedSender<Control>>>,
    updates: mpsc::Sender<Update>,
    receiver: mpsc::Receiver<Update>,
    timer: Timer,
}
fn title(state: &str) -> &str {
    match state {
        "running" => "In progress",
        "review" => "Ready for review",
        "accepted" => "Accepted",
        "discarded" => "Discarded",
        "cancelled" => "Cancelled",
        "interrupted" => "Interrupted",
        "preparing" => "Preparing",
        "applying" => "Applying",
        "failed" => "Failed",
        _ => state,
    }
}
impl App {
    fn render(&self) -> Result<()> {
        let ui = self.ui.unwrap();
        let slot = self.store.borrow();
        let Some(store) = slot.as_ref() else {
            return Ok(());
        };
        let rows = store.attempts()?;
        ui.set_rows(ModelRc::new(VecModel::from(
            rows.iter()
                .map(|a| {
                    StandardListViewItem::from(
                        format!("#{}  {}", a.id, a.prompt.lines().next().unwrap_or("Task"))
                            .as_str(),
                    )
                })
                .collect::<Vec<_>>(),
        )));
        let selected = rows.iter().position(|a| Some(a.id) == self.id.get());
        ui.set_selection(selected.map_or(-1, |i| i as i32));
        ui.set_running(false);
        if let Some(index) = selected {
            let a = &rows[index];
            ui.set_task(a.prompt.as_str().into());
            ui.set_state_label(title(&a.state).into());
            ui.set_action(a.action.as_str().into());
            ui.set_revision(format!("Base  {}", a.base.chars().take(8).collect::<String>()).into());
            ui.set_running(a.state == "running" && self.control.borrow().is_some());
            let done = matches!(
                a.state.as_str(),
                "review" | "failed" | "interrupted" | "cancelled"
            );
            ui.set_can_review((done || a.state == "applying") && !a.patch.is_empty());
            ui.set_can_discard(done);
            ui.set_can_resume(done && !a.session.is_empty());
            ui.set_can_retry(done || a.state == "discarded" || a.state == "accepted");
            ui.set_has_check(
                !a.candidate.is_empty()
                    && a.state != "running"
                    && store.work(a.id).join("check.luau").is_file(),
            );
            ui.set_patch(String::from_utf8_lossy(&store.patch(a)?).as_ref().into());
        }
        self.attempts.replace(rows);
        Ok(())
    }
    fn start(
        self: &Rc<Self>,
        work: impl FnOnce(mpsc::Sender<Update>) -> Result<String> + Send + 'static,
    ) {
        let ui = self.ui.unwrap();
        if ui.get_busy() {
            return;
        }
        ui.set_busy(true);
        let tx = self.updates.clone();
        std::thread::spawn(move || {
            let r = work(tx.clone());
            let _ = tx.send(Update::Finished(r));
        });
        let app = Rc::downgrade(self);
        self.timer
            .start(TimerMode::Repeated, Duration::from_millis(200), move || {
                if let Some(app) = app.upgrade()
                    && let Err(e) = app.poll()
                {
                    app.ui.unwrap().set_notice(format!("{e:#}").into());
                }
            });
    }
    fn poll(&self) -> Result<()> {
        let ui = self.ui.unwrap();
        for update in self.receiver.try_iter() {
            match update {
                Update::Opened(store) => {
                    ui.set_project(
                        store
                            .project
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .as_ref()
                            .into(),
                    );
                    ui.set_milestone(store.milestone()?.into());
                    self.id.set(store.attempts()?.first().map(|a| a.id));
                    self.store.replace(Some(store));
                    ui.set_draft("".into());
                    ui.set_direction("".into());
                }
                Update::Started(id) => {
                    self.id.set(Some(id));
                    ui.set_draft("".into());
                    ui.set_direction("".into());
                    self.render()?;
                }
                Update::Finished(result) => {
                    self.timer.stop();
                    self.control.take();
                    ui.set_busy(false);
                    ui.set_notice(
                        match result {
                            Ok(msg) => msg,
                            Err(e) => format!("{e:#}"),
                        }
                        .into(),
                    );
                    self.render()?;
                }
            }
        }
        if ui.get_running()
            && let Some(id) = self.id.get()
            && let Some(store) = self.store.borrow().as_ref()
        {
            ui.set_action(store.attempt(id)?.action.as_str().into());
        }
        Ok(())
    }
    fn open(self: &Rc<Self>, path: PathBuf) {
        if self
            .store
            .borrow()
            .as_ref()
            .is_some_and(|s| s.project == path)
        {
            return;
        }
        self.ui.unwrap().set_notice("Opening project…".into());
        self.start(move |tx| {
            let home = data_dir()?;
            let s = Arc::new(Store::open(&path, &home)?);
            fs::write(
                home.join("last-project.txt"),
                s.project.to_string_lossy().as_bytes(),
            )?;
            tx.send(Update::Opened(s))?;
            Ok(String::new())
        });
    }
    fn operate(self: &Rc<Self>, op: &str) -> Result<()> {
        let ui = self.ui.unwrap();
        if op == "steer" || op == "cancel" {
            let c = if op == "cancel" {
                Control::Cancel
            } else {
                Control::Steer(ui.get_direction().to_string())
            };
            self.control
                .borrow()
                .as_ref()
                .context("Run is ending.")?
                .send(c)
                .map_err(|_| anyhow::anyhow!("Run has ended."))?;
            if op == "steer" {
                ui.set_direction("".into());
            }
            return Ok(());
        }
        if ui.get_busy() {
            return Ok(());
        }
        let store = self.store.borrow().clone().context("Open a project.")?;
        let id = self.id.get();
        let draft = ui.get_draft().to_string();
        let direction = ui.get_direction().to_string();
        let op = op.to_owned();
        let (tx, rx) = provider::channel();
        self.control.replace(Some(tx));
        ui.set_notice(
            match op.as_str() {
                "check" => "Running luau check.luau…",
                "accept" => "Applying change…",
                "discard" => "Saving decision…",
                _ => "Preparing run…",
            }
            .into(),
        );
        self.start(move |events| {
            if op == "submit" || op == "retry" || op == "resume" {
                let a = match op.as_str() {
                    "submit" => store.prepare(&draft, "", None)?,
                    "retry" => {
                        let old = store.attempt(id.context("Select a task.")?)?;
                        store.prepare(&old.prompt, &old.model, Some(old.id))?
                    }
                    _ => store.resume(id.context("Select a task.")?)?,
                };
                let prompt = if op == "resume" {
                    if direction.trim().is_empty() {
                        "Continue the task and verify the result."
                    } else {
                        &direction
                    }
                } else {
                    &a.prompt
                };
                events.send(Update::Started(a.id))?;
                provider::run(&store, &a, prompt, rx, false)?;
                return Ok(String::new());
            }
            let id = id.context("Select a task.")?;
            match op.as_str() {
                "accept" => store.accept(id)?,
                "discard" => store.discard(id)?,
                "check" => return store.check(id, &["luau".into(), "check.luau".into()]),
                _ => anyhow::bail!("Unknown operation"),
            }
            Ok(String::new())
        });
        Ok(())
    }
}
fn main() -> Result<()> {
    let ui = Desk::new()?;
    let (tx, rx) = mpsc::channel();
    let app = Rc::new(App {
        ui: ui.as_weak(),
        store: Default::default(),
        attempts: Default::default(),
        id: Cell::new(None),
        control: Default::default(),
        updates: tx,
        receiver: rx,
        timer: Timer::default(),
    });
    let a = app.clone();
    ui.on_open_project(move || {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open project folder")
            .pick_folder()
        {
            a.open(path);
        }
    });
    let a = app.clone();
    ui.on_select_row(move |i| {
        if a.ui.unwrap().get_busy() {
            return;
        }
        let selected = usize::try_from(i)
            .ok()
            .and_then(|i| a.attempts.borrow().get(i).map(|r| r.id));
        if selected == a.id.get() {
            return;
        }
        a.id.set(selected);
        let ui = a.ui.unwrap();
        ui.set_direction("".into());
        ui.set_notice("".into());
        if let Err(e) = a.render() {
            ui.set_notice(format!("{e:#}").into());
        }
    });
    let a = app.clone();
    ui.on_operate(move |op| {
        if let Err(e) = a.operate(&op) {
            a.ui.unwrap().set_notice(format!("{e:#}").into());
        }
    });
    let path = std::env::args_os().nth(1).map(PathBuf::from).or_else(|| {
        fs::read_to_string(data_dir().ok()?.join("last-project.txt"))
            .ok()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
    });
    if let Some(path) = path {
        app.open(path);
    }
    ui.run()?;
    if let Some(c) = app.control.take() {
        let _ = c.send(Control::Cancel);
    }
    Ok(())
}
