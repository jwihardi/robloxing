#![cfg_attr(windows, windows_subsystem = "windows")]
mod appearance;
mod capture;
mod desktop_studio;
mod desktop_tasks;
mod desktop_usage;
mod inspector;
mod notifications;
mod preferences;
mod sketch;
mod transcript;
use anyhow::{Context, Result};
use slint::{ComponentHandle, Model, ModelRc, Timer, VecModel};
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
    time::{Duration, Instant},
};
use workshop::{
    Attempt, Store, data_dir,
    provider::{self, Control},
    provider_permissions::{PROVIDERS, ProviderPermissions, TaskRole},
    usage::{ContextUsage, UsageWindow},
    worker_policy::{EFFORTS, WorkerPolicy},
};

slint::slint! { #[edition = "1.19"] export {Appearance, ThemeChoice, ThemeColors, Comparison, Desk, Sketch, Studio, StudioItem, TaskControls, PolicyChoices, ChatTurn, WorkerRow, WorkRow, PickerData, PickerOption, MeterData, PolicyModel, SettingRow, PluginSection, AlbumItem, OutputItem, Transcript, BlockKind, ReplyBlock} from "ui/workshop.slint"; }

enum Update {
    Attention(notifications::Target, &'static str, String),
    NotificationClicked(notifications::Target),
    NotificationError(String),
    StudioFinished(PathBuf, i64, Result<()>),
    Captured(PathBuf, Option<i64>, Option<String>, Result<bool>),
    Preferences(u64, Result<Vec<preferences::Row>>, String),
    PreferenceChanged {
        result: Result<String>,
        provider: preferences::Provider,
        account_changed: bool,
        recommendation: bool,
        changed: bool,
    },
    Opened(Arc<Store>, Option<notifications::Target>),
    Models(PathBuf, u64, Result<provider::Catalog>),
    Context(i64, ContextUsage),
    AccountUsage(PathBuf, String, Vec<UsageWindow>),
    UsageSnapshot(&'static str, u64, Option<provider::AccountLimits>),
    Started(i64, bool),
    Delta(String),
    Activity(i64, String),
    ReplyStored,
    TranscriptChanged,
    WorkersChanged,
    Warm(i64, u64, Box<provider::Session>),
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
    pending_notification: RefCell<Option<notifications::Target>>,
    inspector: RefCell<Option<inspector::Inspector>>,
    task_overrides:
        RefCell<std::collections::HashMap<String, workshop::studio::settings::TaskSettings>>,
    studio_cancel: RefCell<Option<tokio::sync::oneshot::Sender<()>>>,
    settings_generation: Cell<u64>,
    settings_read: RefCell<Option<tokio::sync::oneshot::Sender<()>>>,
    settings_page: RefCell<Option<(String, preferences::Provider, PluginSection)>>,
    settings_rows: RefCell<Rc<Vec<preferences::Row>>>,
    settings_view: Rc<VecModel<SettingRow>>,
    permissions: RefCell<ProviderPermissions>,
    plugin_catalogs: RefCell<std::collections::HashMap<preferences::Provider, CachedPlugins>>,
    plugin_limit: Cell<usize>,
    usage: RefCell<desktop_usage::AccountUsage>,
    catalog: RefCell<provider::Catalog>,
    catalog_generation: Cell<u64>,
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
    sketches: RefCell<std::collections::HashMap<Option<i64>, sketch::Draft>>,
    draft_policy: RefCell<Option<WorkerPolicy>>,
    id: Cell<Option<i64>>,
    control: RefCell<Option<tokio::sync::mpsc::UnboundedSender<Control>>>,
    updates: Updates,
    receiver: mpsc::Receiver<Update>,
    warm_sessions: RefCell<std::collections::VecDeque<(i64, Instant, provider::Session)>>,
    session_generation: Cell<u64>,
}
const DEFAULT_MODEL: &str = "Default model";
const PLUGINS_PER_PAGE: usize = 50;
const SESSION_IDLE: Duration = Duration::from_secs(300);

struct CachedPlugins {
    rows: Rc<Vec<preferences::Row>>,
    loaded: Instant,
}

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
        _ if fresh => "0 tokens used".into(),
        _ => "Awaiting token count".into(),
    };
    MeterData {
        label: "Context".into(),
        remaining: if fresh {
            1.0
        } else {
            context.fraction_left().unwrap_or(-1.0) as f32
        },
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
                remaining: window
                    .percent_left
                    .map_or(-1.0, |value| value as f32 / 100.),
                detail: detail.into(),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(meters))
}

fn model_label(catalog: &provider::Catalog, id: &str) -> String {
    if let Some(name) = id.strip_prefix("antigravity:") {
        catalog
            .display_name(id)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{name} (Antigravity)"))
    } else if let Some(name) = id.strip_prefix("gemini:") {
        format!(
            "Gemini {}",
            name.trim_start_matches("gemini-")
                .split('-')
                .map(capitalized)
                .collect::<Vec<_>>()
                .join(" ")
        )
    } else if let Some(label) = catalog.claude_label(id) {
        label
    } else if let Some(tail) = id.strip_prefix("gpt-") {
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
            .filter(|m| {
                project_default || provider::provider_for(&m.model) == provider::provider_for(model)
            })
            .map(|m| option(&m.model, model_label(catalog, &m.model))),
    );
    let selected = if project_default && model.is_empty() {
        DEFAULT_MODEL
    } else {
        effective
    };
    if !selected.is_empty() && !models.iter().any(|m| m.value == selected) {
        models.push(option(selected, model_label(catalog, selected)));
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
    let default = if provider::provider_for(effective) == "codex"
        && efforts
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
            model_label(catalog, effective).into()
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
fn policy_choices(
    policy: &WorkerPolicy,
    catalog: &provider::Catalog,
    permissions: &ProviderPermissions,
    role: TaskRole,
) -> PolicyChoices {
    let permitted = |id: &str| permissions.allows(provider::provider_for(id), role);
    let has_provider = PROVIDERS.iter().any(|p| permissions.allows(p, role));
    let mut models: Vec<_> = catalog.models.iter().map(|m| m.model.clone()).collect();
    for id in &policy.models {
        if !models.contains(id) {
            models.push(id.clone());
        }
    }
    models.retain(|id| permitted(id));
    let selected: Vec<_> = policy.models.iter().filter(|id| permitted(id)).collect();
    let allowed = if !has_provider {
        "Enable a provider in Settings → Providers".into()
    } else if policy.models.is_empty() {
        "Any available model".into()
    } else if selected.is_empty() {
        "Saved models are disabled in Settings → Providers".into()
    } else {
        selected
            .iter()
            .map(|id| model_label(catalog, id))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let rows = ModelRc::new(VecModel::from(
        models
            .into_iter()
            .map(|id| PolicyModel {
                selected: policy.models.contains(&id),
                label: model_label(catalog, &id).into(),
                value: id.into(),
            })
            .collect::<Vec<_>>(),
    ));
    let mut efforts: Vec<_> = EFFORTS
        .iter()
        .copied()
        .filter(|level| {
            *level == policy.max_effort
                || catalog
                    .models
                    .iter()
                    .filter(|m| {
                        permitted(&m.model)
                            && (policy.models.is_empty() || policy.models.contains(&m.model))
                    })
                    .any(|m| {
                        m.supported_reasoning_efforts
                            .iter()
                            .any(|e| e.reasoning_effort == *level)
                    })
        })
        .map(|e| PickerOption {
            value: e.into(),
            label: if e == "none" {
                "No reasoning".into()
            } else if e == "xhigh" {
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
    let cap_index = efforts
        .iter()
        .position(|e| e.value == policy.max_effort.as_str())
        .unwrap_or(efforts.len() - 1) as i32;
    PolicyChoices {
        has_provider,
        blocked_models: !policy.models.is_empty() && selected.is_empty(),
        all_models: has_provider && policy.models.is_empty(),
        allowed_names: allowed.into(),
        models: rows,
        levels: ModelRc::new(VecModel::from(efforts)),
        cap_index,
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
fn worker_row(w: workshop::team::Worker, catalog: &provider::Catalog) -> WorkerRow {
    let scope = if w.assignment.files.is_empty() {
        "Read only".into()
    } else {
        w.assignment.files.join(", ")
    };
    let mut stats = vec![scope];
    if let Some(id) = &w.model {
        let mut label = if id.is_empty() {
            String::from("Codex default")
        } else {
            model_label(catalog, id)
        };
        if let Some(effort) = w.effort.as_deref().filter(|s| !s.is_empty()) {
            label.push_str(&format!(" ({effort})"));
        }
        stats.push(label);
    }
    if w.tries > 1 {
        let retries = w.tries - 1;
        stats.push(if retries == 1 {
            "1 retry".into()
        } else {
            format!("{retries} retries")
        });
    }
    if let Some(tokens) = w.tokens {
        stats.push(format!("{tokens} tokens"));
    }
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
        assignment: w.assignment.deliverable.into(),
        detail: w.detail.into(),
        criteria: w.assignment.acceptance.into(),
        dependencies: w.assignment.depends_on.join(", ").into(),
        stats: stats.join(" · ").into(),
    }
}
impl App {
    fn preceding_shots(&self, store: &Store, id: Option<i64>) -> Result<Vec<ChatTurn>> {
        Ok(store
            .preceding_screenshots(id)?
            .into_iter()
            .map(|key| ChatTurn {
                shot: key.into(),
                ..Default::default()
            })
            .collect())
    }
    fn screenshot_items(items: Vec<workshop::screenshots::AlbumEntry>) -> ModelRc<AlbumItem> {
        let items: Vec<_> = items
            .into_iter()
            .map(|entry| {
                let path = std::path::Path::new(&entry.project);
                AlbumItem {
                    key: entry.key.into(),
                    date: entry.created.into(),
                    project: path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .as_ref()
                        .into(),
                }
            })
            .collect();
        ModelRc::new(VecModel::from(items))
    }
    fn refresh_work_queue(&self, offset: i32) {
        const PAGE: usize = 40;
        let ui = self.ui.unwrap();
        let store = self.store.borrow();
        let Some(store) = store.as_ref() else { return };
        let offset = offset.max(0);
        match store.work_queue(offset as usize, PAGE + 1) {
            Ok(mut rows) => {
                ui.set_more_work(rows.len() > PAGE);
                rows.truncate(PAGE);
                let items = rows
                    .into_iter()
                    .map(|r| {
                        let mut owner =
                            workshop::provider_permissions::provider_label(&r.provider).to_owned();
                        if r.running_workers + r.queued_workers != 0 {
                            owner.push_str(&format!(
                                " · {} running, {} queued workers",
                                r.running_workers, r.queued_workers
                            ));
                        }
                        if r.queued_messages != 0 {
                            owner.push_str(&format!(" · {} queued messages", r.queued_messages));
                        }
                        WorkRow {
                            id: r.id.to_string().into(),
                            title: r.title.into(),
                            status: r.status.into(),
                            detail: r.next_action.into(),
                            owner: owner.into(),
                            current: self.id.get() == Some(r.id),
                        }
                    })
                    .collect::<Vec<_>>();
                if !ui.get_work_items().iter().eq(items.iter().cloned()) {
                    ui.set_work_items(ModelRc::new(VecModel::from(items)));
                }
                ui.set_work_offset(offset);
                ui.set_work_error("".into());
            }
            Err(e) => ui.set_work_error(format!("Could not read open work. {e:#}").into()),
        }
    }
    fn refresh_album(&self) {
        let ui = self.ui.unwrap();
        match data_dir().and_then(|home| workshop::screenshots::album(&home)) {
            Ok(items) => {
                ui.set_album(Self::screenshot_items(items));
                ui.set_album_error("".into());
            }
            Err(e) => ui.set_album_error(format!("Could not open the album: {e:#}").into()),
        }
    }
    fn refresh_outputs(&self, store: &Store, chat: Option<i64>) -> Result<()> {
        let items: Vec<_> = store
            .chat_outputs(chat)?
            .into_iter()
            .map(|item| {
                let path = |p: Option<PathBuf>| {
                    p.map(|p| p.to_string_lossy().as_ref().into())
                        .unwrap_or_default()
                };
                OutputItem {
                    open_label: format!(
                        "Open {}",
                        if item.shot.is_empty() {
                            item.title.clone()
                        } else {
                            item.title.to_lowercase()
                        }
                    )
                    .into(),
                    title: item.title.into(),
                    detail: item.detail.into(),
                    shot: item.shot.into(),
                    asset_id: item.asset.unwrap_or_default() as i32,
                    path: path(item.image),
                    preview: path(item.preview),
                }
            })
            .collect();
        self.ui
            .unwrap()
            .set_outputs(ModelRc::new(VecModel::from(items)));
        Ok(())
    }
    fn capture(&self, command: Option<String>) {
        let ui = self.ui.unwrap();
        if ui.get_capturing() {
            return;
        }
        let Some(store) = self.store.borrow().clone() else {
            return;
        };
        let attempt = self.id.get();
        let tx = self.updates.clone();
        ui.set_capturing(true);
        std::thread::spawn(move || {
            let result = capture::take(&store, attempt);
            let _ = tx.send(Update::Captured(
                store.project.clone(),
                attempt,
                command,
                result,
            ));
        });
    }
    fn refresh_chat(&self, store: &Store, attempt: &Attempt) -> Result<()> {
        let ui = self.ui.unwrap();
        self.render_queue(store, attempt.id)?;
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
            .map(|w| worker_row(w, &self.catalog.borrow()))
            .collect();
        let mut rows = Vec::new();
        if !more {
            rows.extend(self.preceding_shots(store, Some(attempt.id))?);
            let mut first = ChatTurn {
                from_user: true,
                body: attempt.prompt.as_str().into(),
                ..Default::default()
            };
            sketch::set_turn_image(&mut first, store.task_chat_image(attempt.task)?);
            rows.push(first);
        }
        for message in messages {
            if message.screenshot {
                rows.push(ChatTurn {
                    shot: message.text.into(),
                    ..Default::default()
                });
                continue;
            }
            let mut turn = ChatTurn {
                from_user: message.from_user,
                delivery_notice: message.delivery_notice,
                body: message.text.into(),
                ..Default::default()
            };
            if let Some(asset) = message.asset {
                turn.asset_id = asset as i32;
                turn.asset_model = message.image.is_none();
                if let Some(maker) = message.maker {
                    let (model, effort) = maker.split_once(" · ").unwrap_or((&maker, ""));
                    let label = model_label(&self.catalog.borrow(), model);
                    turn.maker = if effort.is_empty() {
                        label
                    } else {
                        format!("{label} · {effort}")
                    }
                    .into();
                }
                if turn.asset_model {
                    let preview = store
                        .dir
                        .join("studio")
                        .join(asset.to_string())
                        .join("preview.png");
                    turn.sketch = slint::Image::load_from_path(&preview).unwrap_or_default();
                }
            }
            sketch::set_turn_image(&mut turn, message.image);
            if let Some(pair) = message.comparison {
                turn.original_path = pair.original.to_string_lossy().as_ref().into();
                turn.previous_output = pair.previous_output;
                if let Some(path) = pair.markup {
                    turn.markup_path = path.to_string_lossy().as_ref().into();
                }
            }
            if !message.dispatched_workers.is_empty() {
                let mut batch = Vec::new();
                for id in message.dispatched_workers {
                    if let Some(worker) = workers.iter().find(|w| i64::from(w.id) == id) {
                        batch.push(worker.clone());
                    }
                }
                turn.dispatch_id = message.id as i32;
                turn.workers = ModelRc::new(VecModel::from(batch));
            }
            rows.push(turn);
        }
        ui.set_workers(ModelRc::new(VecModel::from(workers)));
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
        let chat = self.id.get().map(|id| store.attempt(id)).transpose()?;
        let (model, effort) = chat
            .as_ref()
            .map_or((s.lead_model.as_str(), s.lead_effort.as_str()), |a| {
                (a.model.as_str(), a.effort.as_str())
            });
        ui.set_chat_choices(picker(&catalog, model, effort, chat.is_none()));
        self.show_provider(&ui, model, &store.project);
        ui.set_parallel_workers(s.parallelism == 2);
        ui.set_auto_delegate(s.auto_delegate);
        Ok(())
    }
    fn show_provider(&self, ui: &Desk, model: &str, project: &std::path::Path) {
        let provider = provider::provider_for(model);
        ui.set_chat_provider(capitalized(provider).into());
        let key = if provider == "antigravity" {
            provider::antigravity::usage_key(model)
        } else {
            provider
        };
        self.usage.borrow_mut().select(provider, key);
        self.refresh_usage(ui, project);
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
        ui.set_inherit_policy(selected.is_none());
        let policy = match selected {
            Some(p) => p,
            None => store.worker_defaults()?,
        };
        let choices = policy_choices(
            &policy,
            catalog,
            &self.permissions.borrow(),
            TaskRole::Workers,
        );
        ui.set_worker_provider_available(choices.has_provider);
        ui.set_worker_models_blocked(choices.blocked_models);
        ui.set_all_models(choices.all_models);
        ui.set_allowed_model_names(choices.allowed_names);
        ui.set_policy_models(choices.models);
        ui.set_cap_index(choices.cap_index);
        ui.set_cap_levels(choices.levels);
        Ok(())
    }
    fn change_policy(&self, field: &str, value: &str) -> Result<()> {
        let slot = self.store.borrow();
        let store = slot.as_ref().context("Open a project")?;
        if field == "open" {
            return self.render_worker_policy(store, &self.catalog.borrow());
        }
        let mut policy = match self.selected_policy(store)? {
            Some(p) => p,
            None => store.worker_defaults()?,
        };
        match field {
            "model" | "effort" => policy.edit(field, value)?,
            "inherit" | "customize" => {}
            _ => anyhow::bail!("Unknown subagent setting"),
        }
        let setting = (field != "inherit").then_some(policy);
        if let Some(id) = self.id.get() {
            store.set_chat_worker_policy(id, setting.as_ref())?;
        } else {
            self.draft_policy.replace(setting);
        }
        self.clear_sessions();
        self.render_worker_policy(store, &self.catalog.borrow())
    }
    fn load_models(&self, project: PathBuf) {
        let generation = self.catalog_generation.get() + 1;
        self.catalog_generation.set(generation);
        self.ui
            .unwrap()
            .set_catalog_status("Loading models…".into());
        let updates = self.updates.clone();
        let sources = self
            .store
            .borrow()
            .as_ref()
            .map(|store| store.refresh_model_sources())
            .unwrap_or_default();
        std::thread::spawn(move || {
            if let Err(error) = sources.discover(&project, |catalog| {
                let _ = updates.send(Update::Models(project.clone(), generation, Ok(catalog)));
            }) {
                let _ = updates.send(Update::Models(project, generation, Err(error)));
            }
        });
    }
    fn filter_preferences(&self, query: &str) {
        let query = query.to_lowercase();
        let terms: Vec<_> = query.split_whitespace().collect();
        let ui = self.ui.unwrap();
        let browse = ui.get_settings_tab() == "Plugins"
            && ui.get_plugin_section() == PluginSection::Marketplace;
        let official = browse && ui.get_official_only();
        let provider = ui.get_settings_provider();
        let provider_page = ui.get_settings_tab() == "Providers";
        let permissions = self.permissions.borrow();
        let all = self.settings_rows.borrow();
        let mut matching = all
            .iter()
            .filter(|r| !official || preferences::official_plugin(&provider, &r.id))
            .filter(|r| {
                if terms.is_empty() {
                    return true;
                }
                let searchable = format!("{} {} {}", r.label, r.id, r.detail).to_lowercase();
                terms.iter().all(|term| searchable.contains(term))
            });
        let limit = if browse {
            self.plugin_limit.get()
        } else {
            usize::MAX
        };
        let rows: Vec<_> = matching
            .by_ref()
            .take(limit)
            .map(|r| SettingRow {
                id: r.id.as_str().into(),
                label: r.label.as_str().into(),
                detail: r.detail.as_str().into(),
                action: r.action.into(),
                button: r.button.into(),
                task_count: if provider_page {
                    desktop_tasks::task_count(&permissions, &r.id)
                        .unwrap_or_default()
                        .into()
                } else {
                    Default::default()
                },
            })
            .collect();
        ui.set_more_plugins(browse && matching.next().is_some());
        // Keep delegates (and their focus and scroll position) across status-only updates.
        let view = &self.settings_view;
        let shown = view.row_count();
        if rows.len() < shown
            || rows
                .iter()
                .take(shown)
                .enumerate()
                .any(|(i, r)| view.row_data(i).is_none_or(|old| old.id != r.id))
        {
            view.set_vec(rows);
            return;
        }
        for (i, row) in rows.iter().take(shown).enumerate() {
            if view.row_data(i).as_ref() != Some(row) {
                view.set_row_data(i, row.clone());
            }
        }
        if rows.len() > shown {
            view.extend_from_slice(&rows[shown..]);
        }
    }
    fn load_preferences(&self, message: String, refresh: bool) {
        let ui = self.ui.unwrap();
        if ui.get_settings_working() {
            return;
        }
        let generation = self.settings_generation.get() + 1;
        self.settings_generation.set(generation);
        let tab = ui.get_settings_tab().to_string();
        let p = preferences::Provider::from_label(&ui.get_settings_provider());
        let section = ui.get_plugin_section();
        let page = (tab.clone(), p, section);
        let switched = self.settings_page.replace(Some(page.clone())).as_ref() != Some(&page);
        self.settings_read.take();
        let browse = tab == "Plugins" && section == PluginSection::Marketplace;
        let catalogs = self.plugin_catalogs.borrow();
        let cached = catalogs.get(&p).filter(|_| browse);
        if switched {
            self.plugin_limit.set(PLUGINS_PER_PAGE);
            self.settings_rows
                .replace(cached.map(|c| c.rows.clone()).unwrap_or_default());
            self.filter_preferences(&ui.get_settings_query());
        }
        if !refresh
            && message.is_empty()
            && cached.is_some_and(|c| c.loaded.elapsed() < Duration::from_secs(300))
        {
            ui.set_settings_status("".into());
            return;
        }
        if matches!(tab.as_str(), "Tasks" | "Appearance" | "Notifications") {
            if tab == "Appearance" {
                appearance::reload(&ui);
            }
            ui.set_settings_status("".into());
            return;
        }
        let store = self.store.borrow().clone();
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        self.settings_read.replace(Some(cancel));
        ui.set_settings_status(if message.is_empty() {
            if browse {
                "Loading plugin catalog…"
            } else {
                "Loading…"
            }
            .into()
        } else {
            message.clone().into()
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
                            rows = preferences::list(&tab, p, section, store.as_deref(), refresh, |partial| {
                                let _ = tx.send(Update::Preferences(generation, Ok(partial), message.clone()));
                            }) => rows,
                            _ = cancelled => Ok(Vec::new()),
                        }
                    })
                });
            let _ = tx.send(Update::Preferences(generation, rows, message));
        });
    }
    fn preference_action(&self, action: String, mut arg: String) {
        let store = self.store.borrow().clone();
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
        if action == "locate-mcp" {
            let Some(path) = rfd::FileDialog::new()
                .set_title("Choose Blender MCP executable")
                .pick_file()
            else {
                return;
            };
            arg = path.to_string_lossy().into_owned();
        }
        ui.set_settings_working(true);
        ui.set_settings_working_id(arg.as_str().into());
        self.settings_read.take();
        self.settings_generation
            .set(self.settings_generation.get() + 1);
        ui.set_settings_status(
            match action.as_str() {
                "login" => "Finish signing in with your provider…",
                "remove" => "Uninstalling…",
                "recommend" => "Installing recommendation…",
                "connect-mcp" | "locate-mcp" | "test-mcp" => "Checking Blender MCP tools…",
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
                .and_then(|rt| {
                    rt.block_on(preferences::change(
                        &tab,
                        p,
                        &action,
                        &arg,
                        store.as_deref(),
                    ))
                });
            let _ = tx.send(Update::PreferenceChanged {
                result,
                provider: p,
                account_changed: tab == "Providers"
                    && matches!(action.as_str(), "login" | "logout"),
                recommendation: action == "recommend",
                changed: action != "test-mcp",
            });
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
        let selected_model = model.clone();
        let ui = self.ui.unwrap();
        if let Some(id) = chat {
            store.set_chat_inference(id, &model, &effort)?;
            ui.set_model_name(model_label(&catalog, &model).into());
            if let Some(chat) = self.attempts.borrow_mut().iter_mut().find(|a| a.id == id) {
                chat.model = model;
                chat.effort = effort;
            }
        } else {
            settings.lead_model = model;
            settings.lead_effort = effort;
            store.configure_team(&settings)?;
        }
        self.show_provider(&ui, &selected_model, &store.project);
        ui.set_chat_choices(choices);
        Ok(())
    }
    fn clear_sessions(&self) {
        self.session_generation
            .set(self.session_generation.get() + 1);
        self.warm_sessions.borrow_mut().clear();
    }
    fn select_chat(&self, selected: Option<i64>) {
        if selected == self.id.get() {
            return;
        }
        let ui = self.ui.unwrap();
        ui.invoke_clear_recipient();
        let mut drafts = self.drafts.borrow_mut();
        drafts.insert(self.id.get(), ui.get_draft().to_string());
        self.id.set(selected);
        self.render_attachments();
        self.displayed.set(None);
        ui.set_draft(drafts.remove(&selected).unwrap_or_default().into());
        ui.set_notice("".into());
        if let Err(e) = self.render() {
            ui.set_notice(format!("{e:#}").into());
        }
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
        self.render_goal(store, selected.map(|i| rows[i].id))?;
        self.refresh_outputs(store, selected.map(|i| rows[i].id))?;
        // Keep the live Stop control mounted when an activity update refreshes this chat.
        ui.set_running(
            selected.is_some_and(|i| rows[i].state == "running") && self.control.borrow().is_some(),
        );
        if selected.is_none() {
            ui.set_queued(Default::default());
            self.turns.set_vec(self.preceding_shots(store, None)?);
            ui.set_context_meter(context_meter(ContextUsage::default(), true));
        }
        if let Some(index) = selected {
            let a = &rows[index];
            ui.set_context_meter(context_meter(store.context_usage(a)?, false));
            ui.set_state_label(title(&a.state).into());
            ui.set_action(a.action.as_str().into());
            ui.set_model_name(model_label(&self.catalog.borrow(), &a.model).into());
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
            let verified = store.candidate_ready(a, &workers)?;
            ui.set_can_review(
                ((done || a.state == "applying") && !a.patch.is_empty() || checkpoint_only)
                    && verified,
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
    fn poll(self: &Rc<Self>) -> Result<()> {
        self.updates.pending.store(false, Ordering::Release);
        let ui = self.ui.unwrap();
        let mut partial = None::<String>;
        let mut transcript_changed = false;
        let mut work_changed = false;
        let follow = ui.invoke_at_end();
        for update in self.receiver.try_iter() {
            work_changed |= matches!(
                &update,
                Update::Started(..)
                    | Update::Activity(..)
                    | Update::WorkersChanged
                    | Update::TranscriptChanged
                    | Update::Finished(_)
            );
            match update {
                Update::Attention(target, title, detail) => self.notify(target, title, &detail),
                Update::NotificationError(error) => ui.set_notice(error.into()),
                Update::NotificationClicked(target) => {
                    self.pending_notification.replace(Some(target));
                    self.open_notification()?;
                }
                Update::StudioFinished(project, id, result) => {
                    let slot = self.store.borrow();
                    if let Some(store) = slot.as_ref().filter(|s| s.project == project) {
                        let job = store.studio_job(id)?;
                        let title = match job.state.as_str() {
                            "Ready" => Some("Studio result ready for review"),
                            "Failed" => Some("Studio run needs attention"),
                            _ => None,
                        };
                        if let Some(title) = title {
                            self.notify(
                                notifications::Target::studio(project, id),
                                title,
                                &job.title,
                            );
                        }
                        self.studio_cancel.take();
                        let studio = ui.global::<Studio>();
                        studio.set_working(false);
                        studio.set_message(
                            result
                                .err()
                                .map(|e| e.to_string())
                                .unwrap_or_default()
                                .into(),
                        );
                        let selected = i64::from(studio.get_selected().id);
                        if ui.get_studio_open()
                            && let Err(e) = self.refresh_studio(
                                store,
                                Some(if selected > 0 { selected } else { id }),
                            )
                        {
                            studio.set_message(format!("{e:#}").into());
                        }
                    }
                    drop(slot);
                    self.open_notification()?;
                }
                Update::Captured(project, attempt, command, result) => {
                    ui.set_capturing(false);
                    if matches!(result, Ok(true)) {
                        self.refresh_album();
                    }
                    if self
                        .store
                        .borrow()
                        .as_ref()
                        .is_some_and(|s| s.project == project)
                        && self.id.get() == attempt
                    {
                        match result {
                            Ok(true) => {
                                if command.as_deref() == Some(ui.get_draft().as_str()) {
                                    ui.set_draft("".into());
                                }
                                self.pages.borrow_mut().clear();
                                ui.set_notice("".into());
                                self.render()?;
                                ui.invoke_show_outputs();
                                Self::scroll_after_layout(&ui);
                            }
                            Ok(false) => {}
                            Err(e) => ui.set_notice(format!("{e:#}").into()),
                        }
                    }
                }
                Update::Preferences(generation, rows, message)
                    if generation == self.settings_generation.get() =>
                {
                    match rows {
                        Ok(mut rows) => {
                            let empty = rows.is_empty();
                            if ui.get_settings_tab() == "Providers" {
                                let previous = self.settings_rows.borrow();
                                for row in rows.iter_mut().filter(|row| row.is_checking_provider())
                                {
                                    if let Some(known) =
                                        previous.iter().find(|known| known.id == row.id)
                                    {
                                        *row = known.clone();
                                    }
                                }
                            }
                            let rows = Rc::new(rows);
                            if ui.get_settings_tab() == "Plugins"
                                && ui.get_plugin_section() == PluginSection::Marketplace
                            {
                                let provider =
                                    preferences::Provider::from_label(&ui.get_settings_provider());
                                self.plugin_catalogs.borrow_mut().insert(
                                    provider,
                                    CachedPlugins {
                                        rows: rows.clone(),
                                        loaded: Instant::now(),
                                    },
                                );
                            }
                            self.settings_rows.replace(rows);
                            self.filter_preferences(&ui.get_settings_query());
                            let empty_label = if empty && ui.get_settings_tab() == "Skills" {
                                "No user skills installed."
                            } else if empty
                                && ui.get_settings_tab() == "Plugins"
                                && ui.get_plugin_section() == PluginSection::Marketplace
                            {
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
                Update::PreferenceChanged {
                    result,
                    provider,
                    account_changed,
                    recommendation,
                    changed,
                } => {
                    ui.set_settings_working(false);
                    let id = ui.get_settings_working_id();
                    ui.set_settings_working_id("".into());
                    match result {
                        Ok(note) => {
                            if account_changed {
                                self.plugin_catalogs.borrow_mut().clear();
                                if let Some(row) =
                                    Rc::make_mut(&mut self.settings_rows.borrow_mut())
                                        .iter_mut()
                                        .find(|r| r.id == id.as_str())
                                {
                                    *row = preferences::Row::checking_provider(id.as_str());
                                }
                                self.filter_preferences(&ui.get_settings_query());
                            } else if changed {
                                self.plugin_catalogs.borrow_mut().remove(&provider);
                            }
                            if recommendation {
                                if let Some(row) =
                                    Rc::make_mut(&mut self.settings_rows.borrow_mut())
                                        .iter_mut()
                                        .find(|r| r.action == "recommend" && r.id == id.as_str())
                                {
                                    row.action = "";
                                    row.button = "Installed";
                                }
                                self.filter_preferences(&ui.get_settings_query());
                            }
                            if changed {
                                self.clear_sessions();
                            }
                            if account_changed && let Some(s) = self.store.borrow().as_ref() {
                                self.usage.borrow_mut().clear();
                                ui.set_usage_meters(usage_meters(Vec::new()));
                                self.render_settings(s)?;
                                self.load_models(s.project.clone());
                            }
                            self.load_preferences(note, false);
                        }
                        Err(e) => ui.set_settings_status(format!("{e:#}").into()),
                    }
                }
                Update::Opened(store, target) => {
                    ui.invoke_reset_chat_navigation();
                    if ui.global::<Comparison>().get_visible() {
                        ui.global::<Comparison>().invoke_close();
                    }
                    if target.is_some() {
                        self.pending_notification.replace(target);
                    }
                    self.task_overrides.borrow_mut().clear();
                    self.close_inspector();
                    self.studio_cancel.take();
                    let studio = ui.global::<Studio>();
                    ui.set_studio_open(false);
                    studio.invoke_clear_project();
                    studio.set_working(false);
                    studio.set_message("".into());
                    studio.set_jobs(Default::default());
                    studio.set_selected(Default::default());
                    studio.set_preview(Default::default());
                    studio.set_code("".into());
                    ui.set_project(
                        store
                            .project
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .as_ref()
                            .into(),
                    );
                    self.usage.borrow_mut().clear();
                    ui.set_usage_meters(usage_meters(Vec::new()));
                    self.id.set(None);
                    self.chat_offset.set(0);
                    self.displayed.set(None);
                    self.drafts.borrow_mut().clear();
                    self.attachments.borrow_mut().clear();
                    self.sketches.borrow_mut().clear();
                    self.render_attachments();
                    self.draft_policy.take();
                    self.catalog.replace(Default::default());
                    ui.set_catalog_ready(false);
                    let project = store.project.clone();
                    self.store.replace(Some(store));
                    self.load_models(project);
                    ui.set_draft("".into());
                }
                Update::Models(project, generation, result) => {
                    if generation != self.catalog_generation.get() {
                        continue;
                    }
                    let slot = self.store.borrow();
                    let store = slot.as_ref();
                    if store.map_or_else(
                        || data_dir().is_ok_and(|home| home == project),
                        |s| s.project == project,
                    ) {
                        match result {
                            Ok(catalog) => {
                                self.catalog.replace(catalog);
                                ui.set_catalog_ready(true);
                                ui.set_catalog_status("".into());
                                if let Some(store) = store {
                                    self.render_settings(store)?;
                                    if let Some(id) = self.id.get() {
                                        ui.set_model_name(
                                            model_label(
                                                &self.catalog.borrow(),
                                                &store.attempt(id)?.model,
                                            )
                                            .into(),
                                        );
                                    }
                                }
                                if ui.global::<Studio>().get_editing_run()
                                    || (ui.get_settings_open() && ui.get_settings_tab() == "Tasks")
                                {
                                    self.task_settings("open", "")?;
                                }
                            }
                            Err(e) => {
                                ui.set_catalog_status(format!("Couldn’t load models. {e}").into())
                            }
                        }
                    }
                }
                Update::Context(id, context) => {
                    if self.id.get() == Some(id) {
                        ui.set_context_meter(context_meter(context, false));
                    }
                }
                Update::UsageSnapshot(source, id, limits) => {
                    let mut usage = self.usage.borrow_mut();
                    usage.finish(source, id, limits);
                    usage.display(&ui);
                }
                Update::AccountUsage(project, provider, windows) => {
                    if self
                        .store
                        .borrow()
                        .as_ref()
                        .is_some_and(|s| s.project == project)
                    {
                        let mut usage = self.usage.borrow_mut();
                        usage.observe(&provider, windows);
                        usage.display(&ui);
                    }
                }
                Update::Started(id, clear_draft) => {
                    ui.set_editing_goal(false);
                    ui.set_notice("".into());
                    if self.id.get().is_none() {
                        self.draft_policy.take();
                        self.chat_offset.set(0);
                    }
                    if clear_draft {
                        self.clear_draft();
                    }
                    self.id.set(Some(id));
                    self.render_attachments();
                    self.displayed.set(None);
                    self.render()?;
                    ui.set_live_activity(
                        format!("Connecting to {}…", ui.get_chat_provider()).into(),
                    );
                }
                Update::Activity(id, label) => {
                    if self.id.get() == Some(id) {
                        ui.set_live_activity(label.into());
                    }
                }
                Update::Delta(delta) => {
                    ui.set_live_activity("".into());
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
                Update::Warm(id, generation, session) => {
                    if generation == self.session_generation.get() {
                        let mut pool = self.warm_sessions.borrow_mut();
                        pool.retain(|(chat, _, _)| *chat != id);
                        if pool.len() >= 3 {
                            pool.pop_front();
                        }
                        pool.push_back((id, Instant::now(), *session));
                        let owner = Rc::downgrade(self);
                        Timer::single_shot(SESSION_IDLE, move || {
                            if let Some(app) = owner.upgrade() {
                                app.warm_sessions
                                    .borrow_mut()
                                    .retain(|(_, used, _)| used.elapsed() < SESSION_IDLE);
                            }
                        });
                    }
                }
                Update::Finished(result) => {
                    partial = Some(String::new());
                    ui.set_live_activity("".into());
                    if self.control.take().is_some() && ui.get_chat_provider() == "Antigravity" {
                        self.usage.borrow_mut().expire("antigravity");
                    }
                    ui.set_busy(false);
                    ui.invoke_close_delivery();
                    ui.set_notice(
                        match result {
                            Ok(msg) => msg,
                            Err(e) => format!("{e:#}"),
                        }
                        .into(),
                    );
                    self.render()?;
                    let studio = ui.global::<Studio>();
                    if ui.get_studio_open()
                        && studio.get_area() == "Assets"
                        && let Some(store) = self.store.borrow().as_ref()
                    {
                        self.refresh_studio(store, Some(i64::from(studio.get_selected().id)))?;
                    }
                    self.open_notification()?;
                }
            }
        }
        if work_changed && ui.get_work_open() {
            self.refresh_work_queue(ui.get_work_offset());
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
            self.render_goal(store, Some(id))?;
            self.refresh_outputs(store, Some(id))?;
            self.refresh_chat(store, attempt)?;
        }
        Ok(())
    }
    fn open(self: &Rc<Self>, path: PathBuf) {
        self.open_to(path, None);
    }
    fn open_to(self: &Rc<Self>, path: PathBuf, target: Option<notifications::Target>) {
        if self
            .store
            .borrow()
            .as_ref()
            .is_some_and(|s| s.project == path)
        {
            return;
        }
        self.clear_sessions();
        self.ui.unwrap().set_notice("Opening project…".into());
        self.start(move |tx| {
            let home = data_dir()?;
            let s = Arc::new(Store::open(&path, &home)?);
            fs::write(
                home.join("last-project.txt"),
                s.project.to_string_lossy().as_bytes(),
            )?;
            tx.send(Update::Opened(s, target))?;
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
        self.render_sketch();
    }
    fn draft_message(&self) -> Result<provider::Message> {
        self.message_with_attachments(&self.ui.unwrap().get_draft())
    }
    fn message_with_attachments(&self, text: &str) -> Result<provider::Message> {
        let mut message = provider::Message::compose(
            text,
            self.attachments
                .borrow()
                .get(&self.id.get())
                .cloned()
                .unwrap_or_default(),
        );
        if let Some(sketch) = self
            .sketches
            .borrow()
            .get(&self.id.get())
            .filter(|d| d.has_content())
        {
            let slot = self.store.borrow();
            let store = slot.as_ref().context("Open a project to send a sketch.")?;
            message.files.push(sketch.save(store)?);
            if message.text.trim().is_empty() {
                message.text = "Sketch".into();
            }
        }
        Ok(message)
    }
    fn render_goal(&self, store: &Store, id: Option<i64>) -> Result<()> {
        let goal = id.map(|id| store.chat_goal(id)).transpose()?.flatten();
        let ui = self.ui.unwrap();
        ui.set_goal_objective(goal.as_ref().map_or("", |g| g.objective.as_str()).into());
        ui.set_goal_status(goal.as_ref().map_or("", |g| g.status.as_str()).into());
        ui.set_recoverable_goal(goal.as_ref().is_some_and(|g| g.recoverable));
        ui.set_goal_detail(goal.as_ref().map_or("", |g| g.detail.as_str()).into());
        Ok(())
    }
    fn render_queue(&self, store: &Store, id: i64) -> Result<()> {
        let rows = store
            .queued_messages(id)?
            .into_iter()
            .map(|(key, text)| PickerOption {
                value: key.to_string().into(),
                label: text.lines().next().unwrap_or("").into(),
            })
            .collect::<Vec<_>>();
        self.ui
            .unwrap()
            .set_queued(ModelRc::new(VecModel::from(rows)));
        Ok(())
    }
    fn clear_draft(&self) {
        self.attachments.borrow_mut().remove(&self.id.get());
        self.sketches.borrow_mut().remove(&self.id.get());
        self.render_attachments();
        self.ui.unwrap().set_draft("".into());
    }
    fn operate(self: &Rc<Self>, op: &str) -> Result<()> {
        self.operate_with_goal(op, None)
    }
    fn operate_with_goal(self: &Rc<Self>, op: &str, editor: Option<&str>) -> Result<()> {
        let ui = self.ui.unwrap();
        let input = ui.get_draft();
        let from_composer = (editor.is_none() || ui.get_goal_uses_draft())
            && !(op == "retry" && ui.get_editing_goal());
        let mut objective = editor;

        if matches!(op, "submit" | "resume" | "steer" | "queue" | "steer-worker") {
            if ui.get_capturing() {
                return Ok(());
            }
            objective = objective.or_else(|| workshop::goals::command(&input));
            if objective == Some("") {
                anyhow::ensure!(
                    !ui.get_busy() || !ui.get_goal_objective().is_empty(),
                    "Stop the current response before starting a goal."
                );
                ui.invoke_open_goal(true);
                return Ok(());
            }
            if objective.is_some() {
                anyhow::ensure!(
                    matches!(op, "submit" | "resume"),
                    "Stop the current response before changing the chat goal."
                );
            }
            let draft = input.to_string();
            if objective.is_none()
                && capture::requested(&draft)
                && !ui.global::<Sketch>().get_has_content()
            {
                self.capture(Some(draft));
                return Ok(());
            }
        }
        if matches!(op, "queue" | "steer-worker") {
            let slot = self.store.borrow();
            let store = slot.as_ref().context("Open a project.")?;
            let id = self.id.get().context("Select a chat.")?;
            let message = self.draft_message()?;
            if op == "queue" {
                store.queue_message(id, &message)?;
                self.render_queue(store, id)?;
            } else {
                store.steer_worker(id, ui.get_direction_worker().into(), message)?;
                ui.invoke_clear_recipient();
                self.refresh_chat(store, &store.attempt(id)?)?;
            }
            self.clear_draft();
            ui.set_notice("".into());
            return Ok(());
        }
        if op == "clear-goal" {
            let slot = self.store.borrow();
            let store = slot.as_ref().context("Open a project.")?;
            store.clear_goal(self.id.get().context("Select a chat.")?)?;
            self.render_goal(store, self.id.get())?;
            ui.invoke_close_goal();
            return Ok(());
        }
        if op == "pause-goal" {
            let slot = self.store.borrow();
            let store = slot.as_ref().context("Open a project.")?;
            store.pause_goal(
                self.id.get().context("Select a chat.")?,
                "You paused this goal.",
            )?;
            if let Some(control) = self.control.borrow().as_ref() {
                let _ = control.send(Control::Cancel);
            }
            self.render_goal(store, self.id.get())?;
            return Ok(());
        }
        if op == "steer" || op == "cancel" {
            let c = if op == "cancel" {
                Control::Cancel
            } else {
                Control::Steer(self.draft_message()?)
            };
            self.control
                .borrow()
                .as_ref()
                .context("Run is ending.")?
                .send(c)
                .map_err(|_| anyhow::anyhow!("Run has ended."))?;
            if op == "steer" {
                self.clear_draft();
            }
            return Ok(());
        }
        if ui.get_busy() {
            return Ok(());
        }
        let store = self.store.borrow().clone().context("Open a project.")?;
        let id = self.id.get();
        let objective = objective.map(str::to_owned);
        let mut draft = if matches!(op, "submit" | "resume") {
            if let Some(text) = objective.as_deref() {
                if from_composer {
                    self.message_with_attachments(text)?
                } else {
                    text.into()
                }
            } else {
                self.draft_message()?
            }
        } else {
            "".into()
        };
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
        let generation = self.session_generation.get();
        let mut session = if matches!(op.as_str(), "resume" | "send-next") {
            let mut pool = self.warm_sessions.borrow_mut();
            pool.iter()
                .position(|(chat, _, _)| Some(*chat) == id)
                .and_then(|index| pool.remove(index))
                .map(|(_, _, session)| session)
        } else {
            None
        };
        self.start(move |events| {
            if matches!(
                op.as_str(),
                "submit" | "retry" | "resume" | "send-next" | "recover-goal"
            ) {
                let a = match op.as_str() {
                    "submit" => store.prepare_with_files(
                        &objective
                            .as_ref()
                            .map_or_else(|| draft.text.clone(), |goal| format!("/goal {goal}")),
                        "",
                        None,
                        draft_policy.as_ref(),
                        &draft.files,
                    )?,
                    "retry" => {
                        let old = store.attempt(id.context("Select a task.")?)?;
                        store.prepare(&old.prompt, &old.model, Some(old.id))?
                    }
                    "send-next" => {
                        let (attempt, next) = store
                            .resume_queued(id.context("Select a chat.")?)?
                            .context("The queue is empty.")?;
                        draft = next;
                        attempt
                    }
                    "recover-goal" => {
                        let (attempt, direction) = store
                            .recover_goal(Some(id.context("Select a chat.")?))?
                            .context("This goal is no longer waiting for recovery.")?;
                        draft = direction.as_str().into();
                        attempt
                    }
                    _ => store
                        .resume_with_goal(id.context("Select a task.")?, objective.as_deref())?,
                };
                let prompt = if matches!(op.as_str(), "resume" | "send-next" | "recover-goal") {
                    draft
                } else if op == "retry" {
                    store.retry_prompt(&a)?.as_str().into()
                } else {
                    a.prompt.as_str().into()
                };
                if op == "submit" {
                    store.bind_draft_screenshots(a.id)?;
                }
                events.send(Update::Started(
                    a.id,
                    !matches!(op.as_str(), "send-next" | "recover-goal") && from_composer,
                ))?;
                let active_goal = store.chat_goal(a.id)?.filter(|g| g.status == "active");
                let earlier_output = store.last_asset_output(a.id)?.map_or(0, |(id, _)| id);
                let earlier_checkpoint = store.current_checkpoint(a.id)?;
                let outcome =
                    provider::run_turn(&mut session, &store, &a, prompt, rx, false, |message| {
                        let _ = events.send(match message {
                            provider::ReplyUpdate::Delta(s) => Update::Delta(s.to_owned()),
                            provider::ReplyUpdate::Context(context) => {
                                Update::Context(a.id, context)
                            }
                            provider::ReplyUpdate::AccountUsage(source, windows) => {
                                Update::AccountUsage(store.project.clone(), source.into(), windows)
                            }
                            provider::ReplyUpdate::Usage(_) => return,
                            provider::ReplyUpdate::Activity(label) => {
                                Update::Activity(a.id, label.to_owned())
                            }
                            provider::ReplyUpdate::WorkersChanged => Update::WorkersChanged,
                            provider::ReplyUpdate::Stored => Update::ReplyStored,
                            provider::ReplyUpdate::TranscriptChanged => Update::TranscriptChanged,
                        });
                    });
                let finished = store.attempt(a.id)?;
                if let Some(title) = notifications::chat_reason(
                    &store,
                    &a,
                    &finished,
                    active_goal.is_some(),
                    earlier_output,
                    earlier_checkpoint,
                )? {
                    let detail = if finished.state == "failed" {
                        finished.action
                    } else {
                        active_goal.map_or_else(|| a.prompt.clone(), |g| g.objective)
                    };
                    events.send(Update::Attention(
                        notifications::Target::chat(store.project.clone(), a.id),
                        title,
                        detail,
                    ))?;
                }
                outcome?;
                if let Some(live) = session {
                    events.send(Update::Warm(a.id, generation, Box::new(live)))?;
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
    if workshop::provider::gemini::is_bridge_process() {
        return workshop::provider::gemini::serve_stdio();
    }
    let mut args = std::env::args_os().skip(1);
    let project = args.next();
    if project.as_deref() == Some(std::ffi::OsStr::new("--headless")) {
        return workshop::cli::execute(std::iter::once("workshop --headless".into()).chain(args));
    }
    if std::env::var_os("WORKSHOP_HEADLESS").as_deref() == Some(std::ffi::OsStr::new("1")) {
        return workshop::cli::execute(
            std::iter::once("workshop".into())
                .chain(project)
                .chain(args),
        );
    }
    let ui = Desk::new()?;
    transcript::connect(&ui);
    appearance::connect(&ui);
    ui.on_has_content(|text| !text.trim().is_empty());
    ui.on_initial_goal_text(|draft, current| {
        match workshop::goals::command(&draft).unwrap_or(draft.trim()) {
            "" => current,
            text => text.into(),
        }
    });
    let (tx, rx) = mpsc::sync_channel(256);
    let app = Rc::new(App {
        pending_notification: Default::default(),
        inspector: Default::default(),
        task_overrides: Default::default(),
        studio_cancel: Default::default(),
        settings_generation: Default::default(),
        settings_read: Default::default(),
        settings_rows: Default::default(),
        settings_page: Default::default(),
        settings_view: Default::default(),
        permissions: RefCell::new(ProviderPermissions::load(&data_dir()?)?),
        plugin_catalogs: Default::default(),
        plugin_limit: Cell::new(PLUGINS_PER_PAGE),
        usage: Default::default(),
        catalog: Default::default(),
        catalog_generation: Default::default(),
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
        sketches: Default::default(),
        draft_policy: Default::default(),
        id: Cell::new(None),
        control: Default::default(),
        updates: Updates {
            tx,
            pending: Default::default(),
            ui: ui.as_weak(),
        },
        receiver: rx,
        warm_sessions: Default::default(),
        session_generation: Default::default(),
    });
    desktop_studio::connect(&ui, &app);
    desktop_tasks::connect(&ui, &app);
    sketch::connect(&ui, &app);
    notifications::connect(&ui);
    let a = app.clone();
    ui.on_take_screenshot(move || a.capture(None));
    let a = app.clone();
    ui.on_refresh_album(move || a.refresh_album());
    ui.on_shot_image(|key, full| {
        capture::image_path(&key, full)
            .ok()
            .and_then(|p| slint::Image::load_from_path(&p).ok())
            .unwrap_or_default()
    });
    ui.on_file_image(|path| {
        if path.is_empty() {
            return Default::default();
        }
        slint::Image::load_from_path(std::path::Path::new(path.as_str())).unwrap_or_default()
    });
    let a = app.clone();
    ui.on_load_settings(move |refresh| a.load_preferences(String::new(), refresh));
    let a = app.clone();
    ui.on_search_settings(move |query| {
        a.plugin_limit.set(PLUGINS_PER_PAGE);
        a.filter_preferences(&query);
    });
    let a = app.clone();
    ui.on_show_more_plugins(move || {
        a.plugin_limit.set(a.plugin_limit.get() + PLUGINS_PER_PAGE);
        a.filter_preferences(&a.ui.unwrap().get_settings_query());
    });
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
    ui.on_remove_queued(move |key| {
        let result = (|| -> Result<()> {
            let slot = a.store.borrow();
            let store = slot.as_ref().context("Open a project.")?;
            let id = a.id.get().context("Select a chat.")?;
            store.remove_queued(id, key.parse()?)?;
            a.render_queue(store, id)
        })();
        if let Err(error) = result {
            a.ui.unwrap().set_notice(error.to_string().into());
        }
    });
    let a = app.clone();
    ui.on_save_scheduling(move |auto, parallel| {
        let result = (|| -> Result<()> {
            let slot = a.store.borrow();
            let store = slot.as_ref().context("Open a project")?;
            let mut s = store.team_settings()?;
            let changed = s.auto_delegate != auto || (s.parallelism == 2) != parallel;
            s.auto_delegate = auto;
            s.parallelism = if parallel { 2 } else { 1 };
            store.configure_team(&s)?;
            if changed {
                // The lead receives its worker limit when opening or resuming its thread.
                a.clear_sessions();
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
    ui.on_refresh_usage(move || {
        if let Some(store) = a.store.borrow().as_ref() {
            a.refresh_usage(&a.ui.unwrap(), &store.project);
        }
    });
    let a = app.clone();
    ui.on_reload_models(move || {
        if let Some(store) = a.store.borrow().as_ref() {
            a.load_models(store.project.clone());
        } else if let Ok(home) = data_dir() {
            a.load_models(home);
        }
    });
    ui.set_turns(app.turns.clone().into());
    ui.set_settings_rows(app.settings_view.clone().into());
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
    ui.on_load_work(move |offset| a.refresh_work_queue(offset));
    let a = app.clone();
    ui.on_find_chats(move |query| {
        if query.trim().is_empty() {
            return Default::default();
        }
        let slot = a.store.borrow();
        let Some(store) = slot.as_ref() else {
            return Default::default();
        };
        match store.search_chats(&query) {
            Ok(rows) => ModelRc::new(VecModel::from(
                rows.into_iter()
                    .map(|(id, prompt)| PickerOption {
                        value: id.to_string().into(),
                        label: prompt.lines().next().unwrap_or("Untitled chat").into(),
                    })
                    .collect::<Vec<_>>(),
            )),
            Err(error) => {
                a.ui.unwrap().set_notice(error.to_string().into());
                Default::default()
            }
        }
    });
    let a = app.clone();
    ui.on_select_hit(move |value| {
        let ui = a.ui.unwrap();
        if ui.get_busy() {
            return;
        }
        let result = (|| -> Result<()> {
            let id = value.parse::<i64>()?;
            let position = a
                .store
                .borrow()
                .as_ref()
                .context("Open a project first")?
                .chat_position(id)?;
            a.chat_offset.set(position / 40 * 40);
            a.select_chat(Some(id));
            Ok(())
        })();
        if let Err(error) = result {
            ui.set_notice(error.to_string().into());
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
        a.select_chat(selected);
    });
    let a = app.clone();
    ui.on_run_goal(move |objective| {
        if let Err(e) = a.operate_with_goal(
            if a.ui.unwrap().get_recoverable_goal()
                && objective.trim() == a.ui.unwrap().get_goal_objective().as_str()
            {
                "recover-goal"
            } else if a.id.get().is_some() {
                "resume"
            } else {
                "submit"
            },
            Some(objective.trim()),
        ) {
            a.ui.unwrap().set_notice(format!("{e:#}").into());
        }
    });
    let a = app.clone();
    ui.on_operate(move |op| {
        if let Err(e) = a.operate(&op) {
            a.ui.unwrap().set_notice(format!("{e:#}").into());
        }
    });
    let path = project.map(PathBuf::from).or_else(|| {
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
