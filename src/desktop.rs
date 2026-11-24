#![cfg_attr(windows, windows_subsystem = "windows")]
mod preferences;
use anyhow::{Context, Result};
use slint::{ComponentHandle, ModelRc, Timer, VecModel};
use std::{
    cell::{Cell, RefCell},
    fs,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use workshop::{
    Attempt, Store, data_dir,
    provider::{self, Control},
    usage::{ContextUsage, UsageWindow},
    worker_policy::{EFFORTS, WorkerPolicy},
};

slint::slint! { #[edition = "1.19"] export {Desk, ChatTurn, WorkerRow, PickerData, PickerOption, MeterData, PolicyModel, SettingRow} from "ui/workshop.slint"; }

enum Update {
    Preferences(u64, Result<Vec<preferences::Row>>, String),
    PreferenceChanged(Result<String>, bool),
    Opened(Arc<Store>),
    Models(PathBuf, Result<provider::Catalog>),
    Context(i64, ContextUsage),
    AccountUsage(PathBuf, Vec<UsageWindow>),
    Started(i64),
    Delta(String),
    ReplyStored,
    TranscriptChanged,
    WorkersChanged,
    Warm(Box<provider::Session>),
    Finished(Result<String>),
}

#[derive(Clone)]
struct Updates {
    tx: mpsc::SyncSender<Update>,
    pending: Arc<AtomicBool>,
    ui: slint::Weak<Desk>,
}
impl Updates {
    fn send(&self, update: Update) -> Result<()> {
        self.tx.send(update)?;
        if !self.pending.swap(true, Ordering::AcqRel) {
            self.ui
                .upgrade_in_event_loop(|ui| ui.invoke_updates_ready())?;
        }
        Ok(())
    }
}
struct App {
    settings_generation: Cell<u64>,
    settings_read: RefCell<Option<tokio::sync::oneshot::Sender<()>>>,
    settings_rows: RefCell<Vec<preferences::Row>>,
    catalog: RefCell<provider::Catalog>,
    ui: slint::Weak<Desk>,
    store: RefCell<Option<Arc<Store>>>,
    attempts: RefCell<Vec<Attempt>>,
    turns: Rc<VecModel<ChatTurn>>,
    displayed: Cell<Option<i64>>,
    first_event: Cell<i64>,
    pages: RefCell<Vec<i64>>,
    chat_offset: Cell<usize>,
    drafts: RefCell<std::collections::HashMap<Option<i64>, String>>,
    attachments: RefCell<std::collections::HashMap<Option<i64>, Vec<PathBuf>>>,
    draft_policy: RefCell<Option<WorkerPolicy>>,
    id: Cell<Option<i64>>,
    control: RefCell<Option<tokio::sync::mpsc::UnboundedSender<Control>>>,
    updates: Updates,
    receiver: mpsc::Receiver<Update>,
    session: RefCell<Option<provider::Session>>,
}
const DEFAULT_MODEL: &str = "Default model";

fn context_meter(context: ContextUsage, fresh: bool) -> MeterData {
    fn tokens(n: i64) -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.)
        } else if n >= 1_000 {
            format!("{:.1}k", n as f64 / 1_000.)
        } else {
            n.to_string()
        }
    }
    let detail = match (context.used, context.capacity) {
        (Some(used), Some(limit)) => format!("{} / {} tokens", tokens(used), tokens(limit)),
        (Some(used), None) => format!("{} tokens used", tokens(used)),
        (None, Some(limit)) => format!("{} token window", tokens(limit)),
        _ if fresh => "After first response".into(),
        _ => "Awaiting token count".into(),
    };
    MeterData {
        label: "Context".into(),
        remaining: context.fraction_left().unwrap_or(-1.0) as f32,
        detail: detail.into(),
    }
}

fn usage_meters(windows: Vec<UsageWindow>) -> ModelRc<MeterData> {
    let meters: Vec<_> = windows
        .into_iter()
        .map(|window| {
            let (label, detail) = match window.minutes {
                Some(10080) => ("7d".into(), "Weekly limit".into()),
                Some(n) if n % 1440 == 0 => {
                    (format!("{}d", n / 1440), format!("{}-day limit", n / 1440))
                }
                Some(n) if n % 60 == 0 => {
                    (format!("{}h", n / 60), format!("{}-hour limit", n / 60))
                }
                Some(n) => (format!("{n}m"), format!("{n}-minute limit")),
                None => ("Usage".into(), "Account limit".into()),
            };
            MeterData {
                label: label.into(),
                remaining: window.percent_left as f32 / 100.,
                detail: detail.into(),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(meters))
}

fn model_label(id: &str) -> String {
    if let Some(tail) = id.strip_prefix("gpt-") {
        let mut parts = tail.split('-');
        let mut label = format!("GPT-{}", parts.next().unwrap_or_default());
        for part in parts {
            label.push(' ');
            label.push_str(&capitalized(part));
        }
        label
    } else {
        id.to_owned()
    }
}
fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| format!("{}{}", c.to_uppercase(), chars.as_str()))
        .unwrap_or_default()
}

fn picker(
    catalog: &provider::Catalog,
    model: &str,
    effort: &str,
    project_default: bool,
) -> PickerData {
    fn option(value: &str, label: String) -> PickerOption {
        PickerOption {
            value: value.into(),
            label: label.into(),
        }
    }
    let effective = if model.is_empty() {
        catalog.default_model.as_str()
    } else {
        model
    };
    let info = catalog.models.iter().find(|m| m.model == effective);
    let mut models = Vec::new();
    if project_default {
        models.push(option(DEFAULT_MODEL, "Default".to_owned()));
    }
    models.extend(
        catalog
            .models
            .iter()
            .map(|m| option(&m.model, model_label(&m.model))),
    );
    let selected = if project_default && model.is_empty() {
        DEFAULT_MODEL
    } else {
        effective
    };
    if !selected.is_empty() && !models.iter().any(|m| m.value == selected) {
        models.push(option(selected, model_label(selected)));
    }
    let model_index = models
        .iter()
        .position(|m| m.value == selected)
        .unwrap_or_default();
    let efforts: Vec<_> = info
        .into_iter()
        .flat_map(|m| &m.supported_reasoning_efforts)
        .map(|e| {
            option(
                &e.reasoning_effort,
                if e.reasoning_effort == "xhigh" {
                    "Extra high".into()
                } else {
                    capitalized(&e.reasoning_effort)
                },
            )
        })
        .collect();
    let default = if efforts
        .iter()
        .any(|e| e.value == catalog.default_effort.as_str())
    {
        catalog.default_effort.as_str()
    } else {
        info.map(|m| m.default_reasoning_effort.as_str())
            .unwrap_or_default()
    };
    let current = if effort.is_empty() { default } else { effort };
    let effort_index = efforts
        .iter()
        .position(|e| e.value == current)
        .map_or(-1, |i| i as i32);
    PickerData {
        current_model: if effective.is_empty() {
            "Codex default".into()
        } else {
            model_label(effective).into()
        },
        current_effort: if current.is_empty() {
            "Default".into()
        } else if effort_index >= 0 {
            efforts[effort_index as usize].label.clone()
        } else {
            capitalized(current).into()
        },
        models: ModelRc::new(VecModel::from(models)),
        efforts: ModelRc::new(VecModel::from(efforts)),
        model_index: model_index as i32,
        effort_index,
    }
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
fn worker_row(w: workshop::team::Worker) -> WorkerRow {
    let scope = if w.assignment.files.is_empty() {
        "Read only".into()
    } else {
        w.assignment.files.join(", ")
    };
    WorkerRow {
        id: w.id as i32,
        title: w.assignment.title.into(),
        state: match w.state.as_str() {
            "running" => "Working",
            "queued" => "Queued",
            "blocked" => "Blocked",
            "review" => "Ready for review",
            "integrating" => "Integrating",
            "checking" => "Checking",
            "finished" => "Finished",
            s => s,
        }
        .into(),
        running: w.state == "running",
        detail: format!("{}\n{}", w.assignment.deliverable, w.detail).into(),
        stats: format!(
            "{} · {} retries{}",
            scope,
            w.tries.saturating_sub(1),
            w.tokens
                .map(|t| format!(" · {t} tokens"))
                .unwrap_or_default()
        )
        .into(),
    }
}
impl App {
    fn refresh_chat(&self, store: &Store, attempt: &Attempt) -> Result<()> {
        let ui = self.ui.unwrap();
        let switching = self.displayed.get() != Some(attempt.id);
        let follow = switching || ui.invoke_at_end();
        if switching {
            self.pages.borrow_mut().clear();
            self.displayed.set(Some(attempt.id));
        }
        let before = self.pages.borrow().last().copied().unwrap_or(i64::MAX);
        let (messages, more) = store.message_page(attempt.id, before, 50)?;
        self.first_event.set(messages.first().map_or(0, |m| m.id));
        ui.set_earlier_messages(more);
        ui.set_later_messages(before != i64::MAX);
        let workers: Vec<_> = store
            .workers(attempt.id)?
            .into_iter()
            .map(worker_row)
            .collect();
        let mut visible_workers = std::collections::HashSet::new();
        let mut rows = Vec::new();
        if !more {
            rows.push(ChatTurn {
                from_user: true,
                body: attempt.prompt.as_str().into(),
                ..Default::default()
            });
        }
        for message in messages {
            let mut turn = ChatTurn {
                from_user: message.from_user,
                delivery_notice: message.delivery_notice,
                body: message.text.into(),
                ..Default::default()
            };
            if !message.dispatched_workers.is_empty() {
                let mut batch = Vec::new();
                for id in message.dispatched_workers {
                    if let Some(worker) = workers.iter().find(|w| i64::from(w.id) == id) {
                        batch.push(worker.clone());
                        visible_workers.insert(worker.id);
                    }
                }
                turn.dispatch_id = message.id as i32;
                turn.workers = ModelRc::new(VecModel::from(batch));
            }
            rows.push(turn);
        }
        // Keep workers from older pages and pre-dispatch-history chats accessible.
        let other_workers: Vec<_> = workers
            .into_iter()
            .filter(|w| !visible_workers.contains(&w.id))
            .collect();
        ui.set_workers(ModelRc::new(VecModel::from(other_workers)));
        self.turns.set_vec(rows);
        if follow && before == i64::MAX {
            Self::scroll_after_layout(&ui);
        }
        Ok(())
    }
    fn scroll_after_layout(ui: &Desk) {
        let weak = ui.as_weak();
        Timer::single_shot(Duration::ZERO, move || {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_scroll_end();
            }
        });
    }
    fn render_settings(&self, store: &Store) -> Result<()> {
        let s = store.team_settings()?;
        let ui = self.ui.unwrap();
        let catalog = self.catalog.borrow();
        self.render_worker_policy(store, &catalog)?;
        ui.set_chat_choices(if let Some(id) = self.id.get() {
            let a = store.attempt(id)?;
            picker(&catalog, &a.model, &a.effort, false)
        } else {
            picker(&catalog, &s.lead_model, &s.lead_effort, true)
        });
        ui.set_parallel_workers(s.parallelism == 2);
        Ok(())
    }
    fn selected_policy(&self, store: &Store) -> Result<Option<WorkerPolicy>> {
        match self.id.get() {
            Some(id) => store.chat_worker_policy(id),
            None => Ok(self.draft_policy.borrow().clone()),
        }
    }
    fn render_worker_policy(&self, store: &Store, catalog: &provider::Catalog) -> Result<()> {
        let ui = self.ui.unwrap();
        let selected = self.selected_policy(store)?;
        ui.set_custom_subagents(selected.is_some());
        let local = if ui.get_editing_chat_policy() {
            selected
        } else {
            None
        };
        ui.set_inherit_policy(local.is_none());
        let policy = match local {
            Some(p) => p,
            None => store.worker_defaults()?,
        };
        let mut models: Vec<_> = catalog.models.iter().map(|m| m.model.clone()).collect();
        for id in &policy.models {
            if !models.contains(id) {
                models.push(id.clone());
            }
        }
        ui.set_all_models(policy.models.is_empty());
        let allowed = if policy.models.is_empty() {
            "Any available model".into()
        } else {
            policy
                .models
                .iter()
                .map(|id| model_label(id))
                .collect::<Vec<_>>()
                .join(", ")
        };
        ui.set_allowed_model_names(allowed.into());
        ui.set_policy_models(ModelRc::new(VecModel::from(
            models
                .into_iter()
                .map(|id| PolicyModel {
                    selected: policy.models.contains(&id),
                    label: model_label(&id).into(),
                    value: id.into(),
                })
                .collect::<Vec<_>>(),
        )));
        let mut efforts: Vec<_> = EFFORTS
            .iter()
            .copied()
            .filter(|level| {
                *level == policy.max_effort
                    || catalog
                        .models
                        .iter()
                        .filter(|m| policy.models.is_empty() || policy.models.contains(&m.model))
                        .any(|m| {
                            m.supported_reasoning_efforts
                                .iter()
                                .any(|e| e.reasoning_effort == *level)
                        })
            })
            .map(|e| PickerOption {
                value: e.into(),
                label: if e == "xhigh" {
                    "Extra high".into()
                } else {
                    capitalized(e).into()
                },
            })
            .collect();
        efforts.push(PickerOption {
            value: "".into(),
            label: "None".into(),
        });
        ui.set_cap_index(
            efforts
                .iter()
                .position(|e| e.value == policy.max_effort.as_str())
                .unwrap_or(efforts.len() - 1) as i32,
        );
        ui.set_cap_levels(ModelRc::new(VecModel::from(efforts)));
        Ok(())
    }
    fn change_policy(&self, field: &str, value: &str) -> Result<()> {
        let slot = self.store.borrow();
        let store = slot.as_ref().context("Open a project")?;
        let ui = self.ui.unwrap();
        let chat = ui.get_editing_chat_policy();
        if field == "open" {
            return self.render_worker_policy(store, &self.catalog.borrow());
        }
        let defaults = store.worker_defaults()?;
        let mut policy = if chat {
            self.selected_policy(store)?.unwrap_or(defaults)
        } else {
            defaults
        };
        match field {
            "model" if value.is_empty() => policy.models.clear(),
            "model" => {
                if policy.models.iter().any(|m| m == value) {
                    if policy.models.len() > 1 {
                        policy.models.retain(|m| m != value);
                    }
                } else {
                    policy.models.push(value.to_owned());
                }
            }
            "effort" => policy.max_effort = value.into(),
            "inherit" | "customize" => {}
            _ => anyhow::bail!("Unknown subagent setting"),
        }
        if chat {
            let setting = (field != "inherit").then_some(policy);
            if let Some(id) = self.id.get() {
                store.set_chat_worker_policy(id, setting.as_ref())?;
            } else {
                self.draft_policy.replace(setting);
            }
        } else {
            store.save_worker_defaults(&policy)?;
        }
        self.session.take();
        self.render_worker_policy(store, &self.catalog.borrow())
    }
    fn load_models(&self, project: PathBuf) {
        let ui = self.ui.unwrap();
        ui.set_catalog_status("Loading models…".into());
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            match provider::load_models_and_usage(&project, |catalog| {
                let _ = updates.send(Update::Models(project.clone(), Ok(catalog)));
            }) {
                Ok(Some(windows)) => {
                    let _ = updates.send(Update::AccountUsage(project, windows));
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = updates.send(Update::Models(project, Err(error)));
                }
            }
        });
    }
    fn filter_preferences(&self, query: &str) {
        let query = query.to_lowercase();
        let rows: Vec<_> = self
            .settings_rows
            .borrow()
            .iter()
            .filter(|r| {
                r.label.to_lowercase().contains(&query) || r.detail.to_lowercase().contains(&query)
            })
            .map(|r| SettingRow {
                id: r.id.as_str().into(),
                label: r.label.as_str().into(),
                detail: r.detail.as_str().into(),
                action: r.action.into(),
                button: r.button.into(),
            })
            .collect();
        self.ui
            .unwrap()
            .set_settings_rows(ModelRc::new(VecModel::from(rows)));
    }
    fn load_preferences(&self, message: String) {
        let ui = self.ui.unwrap();
        if ui.get_settings_working() {
            return;
        }
        let generation = self.settings_generation.get() + 1;
        self.settings_generation.set(generation);
        self.settings_rows.borrow_mut().clear();
        self.filter_preferences("");
        let tab = ui.get_settings_tab().to_string();
        let p = preferences::Provider::from_label(&ui.get_settings_provider());
        let browse = ui.get_browse_plugins();
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        self.settings_read.replace(Some(cancel));
        ui.set_settings_status(if message.is_empty() {
            "Loading…".into()
        } else {
            format!("{message} Refreshing…").into()
        });
        let tx = self.updates.clone();
        std::thread::spawn(move || {
            let rows = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(anyhow::Error::from)
                .and_then(|rt| {
                    rt.block_on(async {
                        tokio::select! {
                            rows = preferences::list(&tab, p, browse) => rows,
                            _ = cancelled => Ok(Vec::new()),
                        }
                    })
                });
            let _ = tx.send(Update::Preferences(generation, rows, message));
        });
    }
    fn preference_action(&self, action: String, mut arg: String) {
        let ui = self.ui.unwrap();
        if ui.get_settings_working() {
            return;
        }
        let tab = ui.get_settings_tab().to_string();
        let p = preferences::Provider::from_label(&ui.get_settings_provider());
        if action == "folder" {
            let Some(path) = rfd::FileDialog::new()
                .set_title("Select a folder containing SKILL.md")
                .pick_folder()
            else {
                return;
            };
            arg = path.to_string_lossy().into_owned();
        }
        ui.set_settings_working(true);
        self.settings_read.take();
        self.settings_generation
            .set(self.settings_generation.get() + 1);
        ui.set_settings_status(
            match action.as_str() {
                "login" => "Finish signing in with your provider…",
                "remove" => "Uninstalling…",
                "install" | "folder" => "Installing…",
                _ => "Updating…",
            }
            .into(),
        );
        let tx = self.updates.clone();
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(anyhow::Error::from)
                .and_then(|rt| rt.block_on(preferences::change(&tab, p, &action, &arg)));
            let _ = tx.send(Update::PreferenceChanged(
                result,
                tab == "Providers" && arg == "Codex",
            ));
        });
    }
    fn choose_inference(&self, changing_model: bool, value: &str) -> Result<()> {
        let slot = self.store.borrow();
        let store = slot.as_ref().context("Open a project")?;
        let mut settings = store.team_settings()?;
        let chat = self.id.get();
        let catalog = self.catalog.borrow();
        let (model, effort) = if changing_model {
            let model = if value == DEFAULT_MODEL {
                String::new()
            } else {
                value.to_owned()
            };
            let effort = if chat.is_some() {
                catalog
                    .models
                    .iter()
                    .find(|m| m.model == model)
                    .map(|m| m.default_reasoning_effort.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            (model, effort)
        } else {
            let model = if let Some(id) = chat {
                store.attempt(id)?.model
            } else {
                settings.lead_model.clone()
            };
            (model, value.to_owned())
        };
        let choices = picker(&catalog, &model, &effort, chat.is_none());
        let ui = self.ui.unwrap();
        if let Some(id) = chat {
            store.set_chat_inference(id, &model, &effort)?;
            ui.set_model_name(model.as_str().into());
            if let Some(chat) = self.attempts.borrow_mut().iter_mut().find(|a| a.id == id) {
                chat.model = model;
                chat.effort = effort;
            }
        } else {
            settings.lead_model = model;
            settings.lead_effort = effort;
            store.configure_team(&settings)?;
        }
        ui.set_chat_choices(choices);
        Ok(())
    }
    fn render(&self) -> Result<()> {
        let ui = self.ui.unwrap();
        let slot = self.store.borrow();
        let Some(store) = slot.as_ref() else {
            return Ok(());
        };
        self.render_settings(store)?;
        let mut rows = store.attempts_page(self.chat_offset.get(), 41)?;
        ui.set_older_chats(rows.len() > 40);
        ui.set_newer_chats(self.chat_offset.get() > 0);
        rows.truncate(40);
        ui.set_rows(ModelRc::new(VecModel::from(
            rows.iter()
                .map(|a| a.prompt.lines().next().unwrap_or("Untitled chat").into())
                .collect::<Vec<_>>(),
        )));
        let selected = rows.iter().position(|a| Some(a.id) == self.id.get());
        ui.set_selection(selected.map_or(-1, |i| i as i32));
        ui.set_running(false);
        if selected.is_none() {
            ui.set_context_meter(context_meter(ContextUsage::default(), true));
        }
        if let Some(index) = selected {
            let a = &rows[index];
            ui.set_context_meter(context_meter(store.context_usage(a)?, false));
            ui.set_state_label(title(&a.state).into());
            ui.set_action(a.action.as_str().into());
            ui.set_model_name(a.model.as_str().into());
            ui.set_running(a.state == "running" && self.control.borrow().is_some());
            let done = matches!(
                a.state.as_str(),
                "review" | "failed" | "interrupted" | "cancelled"
            );
            let checkpoint = store.current_checkpoint(a.id)?;
            let checkpoint_only = a.state == "review" && a.patch.is_empty() && checkpoint.is_some();
            ui.set_checkpoint(
                checkpoint
                    .map(|(decision, next)| format!("{decision}\nNext: {next}"))
                    .unwrap_or_default()
                    .into(),
            );
            ui.set_checkpoint_only(checkpoint_only);
            let workers = store.workers(a.id)?;
            let verified = workers.is_empty() || store.verified_candidate(a)?;
            ui.set_can_review(
                ((done || a.state == "applying") && !a.patch.is_empty() || checkpoint_only)
                    && verified
                    && workers.iter().all(|w| w.state == "finished"),
            );
            ui.set_can_discard(done);
            ui.set_can_resume(done && !a.session.is_empty());
            ui.set_can_retry(done || a.state == "discarded" || a.state == "accepted");
            self.refresh_chat(store, a)?;
        }
        self.attempts.replace(rows);
        Ok(())
    }
    fn start(self: &Rc<Self>, work: impl FnOnce(Updates) -> Result<String> + Send + 'static) {
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
    }
    fn poll(&self) -> Result<()> {
        self.updates.pending.store(false, Ordering::Release);
        let ui = self.ui.unwrap();
        let mut partial = None::<String>;
        let mut transcript_changed = false;
        let follow = ui.invoke_at_end();
        for update in self.receiver.try_iter() {
            match update {
                Update::Preferences(generation, rows, message)
                    if generation == self.settings_generation.get() =>
                {
                    match rows {
                        Ok(rows) => {
                            let empty = rows.is_empty();
                            self.settings_rows.replace(rows);
                            self.filter_preferences(&ui.get_settings_query());
                            let empty_label = if empty && ui.get_settings_tab() == "Skills" {
                                "No user skills installed."
                            } else if empty && ui.get_browse_plugins() {
                                "No plugins available."
                            } else if empty {
                                "No plugins installed."
                            } else {
                                ""
                            };
                            ui.set_settings_status(
                                format!("{message} {empty_label}").trim().into(),
                            );
                        }
                        Err(e) => ui.set_settings_status(format!("{message} {e:#}").trim().into()),
                    }
                }
                Update::Preferences(..) => {}
                Update::PreferenceChanged(result, account_changed) => {
                    ui.set_settings_working(false);
                    match result {
                        Ok(note) => {
                            self.session.take();
                            if account_changed && let Some(s) = self.store.borrow().as_ref() {
                                ui.set_usage_meters(usage_meters(Vec::new()));
                                self.load_models(s.project.clone());
                            }
                            self.load_preferences(note);
                        }
                        Err(e) => ui.set_settings_status(format!("{e:#}").into()),
                    }
                }
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
                    ui.set_usage_meters(usage_meters(Vec::new()));
                    self.id.set(None);
                    self.chat_offset.set(0);
                    self.displayed.set(None);
                    self.drafts.borrow_mut().clear();
                    self.attachments.borrow_mut().clear();
                    self.render_attachments();
                    self.draft_policy.take();
                    self.catalog.replace(Default::default());
                    ui.set_catalog_ready(false);
                    self.load_models(store.project.clone());
                    self.store.replace(Some(store));
                    ui.set_draft("".into());
                }
                Update::Models(project, result) => {
                    if let Some(store) = self
                        .store
                        .borrow()
                        .as_ref()
                        .filter(|s| s.project == project)
                    {
                        match result {
                            Ok(catalog) => {
                                store.cache_worker_catalog(&catalog);
                                self.catalog.replace(catalog);
                                ui.set_catalog_ready(true);
                                ui.set_catalog_status("".into());
                                self.render_settings(store)?;
                            }
                            Err(e) => ui.set_catalog_status(
                                format!("Couldn’t load Codex models. Refresh in Subagents. {e}")
                                    .into(),
                            ),
                        }
                    }
                }
                Update::Context(id, context) => {
                    if self.id.get() == Some(id) {
                        ui.set_context_meter(context_meter(context, false));
                    }
                }
                Update::AccountUsage(project, windows) => {
                    if self
                        .store
                        .borrow()
                        .as_ref()
                        .is_some_and(|s| s.project == project)
                    {
                        ui.set_usage_meters(usage_meters(windows));
                    }
                }
                Update::Started(id) => {
                    ui.set_notice("".into());
                    if self.id.get().is_none() {
                        self.draft_policy.take();
                        self.chat_offset.set(0);
                    }
                    self.attachments.borrow_mut().remove(&self.id.get());
                    self.id.set(Some(id));
                    self.render_attachments();
                    ui.set_draft("".into());
                    self.displayed.set(None);
                    self.render()?;
                }
                Update::Delta(delta) => {
                    let text = partial.get_or_insert_with(|| ui.get_partial().to_string());
                    if text.len() < 128 * 1024 {
                        let remaining = 128 * 1024 - text.len();
                        let end = delta.floor_char_boundary(remaining.min(delta.len()));
                        text.push_str(&delta[..end]);
                        if end < delta.len() {
                            text.push('…');
                        }
                    }
                }
                Update::ReplyStored => {
                    partial = Some(String::new());
                    transcript_changed = true;
                }
                Update::WorkersChanged | Update::TranscriptChanged => transcript_changed = true,
                Update::Warm(session) => {
                    self.session.replace(Some(*session));
                }
                Update::Finished(result) => {
                    partial = Some(String::new());
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
        if let Some(text) = partial {
            ui.set_partial(text.into());
            if follow {
                Self::scroll_after_layout(&ui);
            }
        }
        if transcript_changed
            && ui.get_running()
            && let Some(id) = self.id.get()
            && let Some(store) = self.store.borrow().as_ref()
            && let Some(attempt) = self.attempts.borrow().iter().find(|a| a.id == id)
        {
            self.refresh_chat(store, attempt)?;
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
        self.session.take();
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
    fn render_attachments(&self) {
        let attachments = self.attachments.borrow();
        let rows: Vec<_> = attachments
            .get(&self.id.get())
            .into_iter()
            .flatten()
            .map(|p| PickerOption {
                label: p
                    .file_name()
                    .unwrap_or(p.as_os_str())
                    .to_string_lossy()
                    .as_ref()
                    .into(),
                value: p.to_string_lossy().as_ref().into(),
            })
            .collect();
        self.ui
            .unwrap()
            .set_attachments(ModelRc::new(VecModel::from(rows)));
    }
    fn draft_message(&self) -> provider::Message {
        provider::Message::compose(
            &self.ui.unwrap().get_draft(),
            self.attachments
                .borrow()
                .get(&self.id.get())
                .cloned()
                .unwrap_or_default(),
        )
    }
    fn operate(self: &Rc<Self>, op: &str) -> Result<()> {
        let ui = self.ui.unwrap();
        if op == "steer" || op == "cancel" {
            let c = if op == "cancel" {
                Control::Cancel
            } else {
                Control::Steer(self.draft_message())
            };
            self.control
                .borrow()
                .as_ref()
                .context("Run is ending.")?
                .send(c)
                .map_err(|_| anyhow::anyhow!("Run has ended."))?;
            if op == "steer" {
                self.attachments.borrow_mut().remove(&self.id.get());
                self.render_attachments();
                ui.set_draft("".into());
            }
            return Ok(());
        }
        if ui.get_busy() {
            return Ok(());
        }
        let store = self.store.borrow().clone().context("Open a project.")?;
        let id = self.id.get();
        let draft = self.draft_message();
        let draft_policy = self.draft_policy.borrow().clone();
        let op = op.to_owned();
        let (tx, rx) = provider::channel();
        self.control.replace(Some(tx));
        ui.set_notice(
            match op.as_str() {
                "accept" => "Applying change…",
                "discard" => "Saving decision…",
                _ => "Preparing run…",
            }
            .into(),
        );
        let mut session = self.session.take().filter(|_| op == "resume");
        self.start(move |events| {
            if op == "submit" || op == "retry" || op == "resume" {
                let a = match op.as_str() {
                    "submit" => store.prepare_with_files(
                        &draft.text,
                        "",
                        None,
                        draft_policy.as_ref(),
                        &draft.files,
                    )?,
                    "retry" => {
                        let old = store.attempt(id.context("Select a task.")?)?;
                        store.prepare(&old.prompt, &old.model, Some(old.id))?
                    }
                    _ => store.resume(id.context("Select a task.")?)?,
                };
                let prompt = if op == "resume" {
                    draft
                } else {
                    a.prompt.as_str().into()
                };
                events.send(Update::Started(a.id))?;
                provider::run_turn(&mut session, &store, &a, prompt, rx, false, |message| {
                    let _ = events.send(match message {
                        provider::ReplyUpdate::Delta(s) => Update::Delta(s.to_owned()),
                        provider::ReplyUpdate::Context(context) => Update::Context(a.id, context),
                        provider::ReplyUpdate::AccountUsage(windows) => {
                            Update::AccountUsage(store.project.clone(), windows)
                        }
                        provider::ReplyUpdate::Usage(_) | provider::ReplyUpdate::Activity(_) => {
                            return;
                        }
                        provider::ReplyUpdate::WorkersChanged => Update::WorkersChanged,
                        provider::ReplyUpdate::Stored => Update::ReplyStored,
                        provider::ReplyUpdate::TranscriptChanged => Update::TranscriptChanged,
                    });
                })?;
                if let Some(live) = session {
                    events.send(Update::Warm(Box::new(live)))?;
                }
                return Ok(String::new());
            }
            let id = id.context("Select a task.")?;
            match op.as_str() {
                "accept" => store.accept(id)?,
                "discard" => store.discard(id)?,
                _ => anyhow::bail!("Unknown operation"),
            }
            Ok(String::new())
        });
        Ok(())
    }
}
fn main() -> Result<()> {
    let ui = Desk::new()?;
    ui.on_has_content(|text| !text.trim().is_empty());
    let (tx, rx) = mpsc::sync_channel(256);
    let app = Rc::new(App {
        settings_generation: Default::default(),
        settings_read: Default::default(),
        settings_rows: Default::default(),
        catalog: Default::default(),
        ui: ui.as_weak(),
        store: Default::default(),
        attempts: Default::default(),
        turns: Rc::new(VecModel::default()),
        displayed: Default::default(),
        first_event: Default::default(),
        pages: Default::default(),
        chat_offset: Default::default(),
        drafts: Default::default(),
        attachments: Default::default(),
        draft_policy: Default::default(),
        id: Cell::new(None),
        control: Default::default(),
        updates: Updates {
            tx,
            pending: Default::default(),
            ui: ui.as_weak(),
        },
        receiver: rx,
        session: Default::default(),
    });
    let a = app.clone();
    ui.on_load_settings(move || a.load_preferences(String::new()));
    let a = app.clone();
    ui.on_search_settings(move |query| a.filter_preferences(&query));
    let a = app.clone();
    ui.on_settings_action(move |action, arg| {
        a.preference_action(action.to_string(), arg.to_string())
    });
    let a = app.clone();
    ui.on_page(move |chat, older| {
        if chat {
            if a.ui.unwrap().get_busy() {
                return;
            }
            a.ui.unwrap().invoke_select_row(-1);
            a.chat_offset.set(if older {
                a.chat_offset.get() + 40
            } else {
                a.chat_offset.get().saturating_sub(40)
            });
        } else if older {
            a.pages.borrow_mut().push(a.first_event.get());
        } else {
            a.pages.borrow_mut().pop();
        }
        if let Err(e) = a.render() {
            a.ui.unwrap().set_notice(format!("{e:#}").into());
        }
    });
    let a = app.clone();
    ui.on_stop_worker(move |id| {
        if let Some(tx) = a.control.borrow().as_ref() {
            let _ = tx.send(Control::StopWorker(id.into()));
        }
    });
    let a = app.clone();
    ui.on_save_parallel(move |parallel| {
        let result = (|| -> Result<()> {
            let slot = a.store.borrow();
            let store = slot.as_ref().context("Open a project")?;
            let mut s = store.team_settings()?;
            let previous_limit = s.parallelism;
            s.parallelism = if parallel { 2 } else { 1 };
            store.configure_team(&s)?;
            if s.parallelism != previous_limit {
                // The lead receives its worker limit when opening or resuming its thread.
                a.session.take();
            }
            a.render_settings(store)
        })();
        if let Err(e) = result {
            a.ui.unwrap().set_notice(format!("{e:#}").into());
        }
    });
    let a = app.clone();
    ui.on_choose_inference(move |model, value| {
        if let Err(e) = a.choose_inference(model, &value) {
            a.ui.unwrap().set_notice(format!("{e:#}").into());
        }
    });
    let a = app.clone();
    ui.on_change_policy(move |field, value| {
        if let Err(e) = a.change_policy(&field, &value) {
            a.ui.unwrap().set_notice(format!("{e:#}").into());
        }
    });
    let a = app.clone();
    ui.on_reload_models(move || {
        if let Some(store) = a.store.borrow().as_ref() {
            a.load_models(store.project.clone());
        }
    });
    ui.set_turns(app.turns.clone().into());
    let a = app.clone();
    ui.on_updates_ready(move || {
        if let Err(e) = a.poll() {
            a.ui.unwrap().set_notice(format!("{e:#}").into());
        }
    });
    let a = app.clone();
    ui.on_attach(move |folder| {
        let picker = rfd::FileDialog::new();
        let paths = if folder {
            picker
                .set_title("Attach a folder")
                .pick_folder()
                .map(|p| vec![p])
        } else {
            picker.set_title("Attach files").pick_files()
        };
        if let Some(paths) = paths {
            let mut all = a.attachments.borrow_mut();
            let draft = all.entry(a.id.get()).or_default();
            for path in paths {
                if !draft.contains(&path) {
                    draft.push(path);
                }
            }
            drop(all);
            a.render_attachments();
            a.ui.unwrap().invoke_focus_draft();
        }
    });
    let a = app.clone();
    ui.on_remove_attachment(move |index| {
        if let Some(files) = a.attachments.borrow_mut().get_mut(&a.id.get())
            && let Ok(i) = usize::try_from(index)
            && i < files.len()
        {
            files.remove(i);
        }
        a.render_attachments();
        a.ui.unwrap().invoke_focus_draft();
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
        let ui = a.ui.unwrap();
        let mut drafts = a.drafts.borrow_mut();
        drafts.insert(a.id.get(), ui.get_draft().to_string());
        a.session.take();
        a.id.set(selected);
        a.render_attachments();
        a.displayed.set(None);
        ui.set_draft(drafts.remove(&selected).unwrap_or_default().into());
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
