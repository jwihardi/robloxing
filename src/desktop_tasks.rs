use super::{
    App, Desk, PolicyChoices, PolicyModel, Studio, TaskControls, TaskModelPage, TaskProvider,
    policy_choices,
};
use anyhow::{Context, Result, bail, ensure};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::rc::Rc;
use workshop::{
    data_dir,
    provider_permissions::{PROVIDERS, ProviderPermissions, TaskRole, provider_label},
    studio::settings::TaskSettings,
    worker_policy::WorkerPolicy,
};

fn task_role(area: &str) -> Result<TaskRole> {
    Ok(match area {
        "Workers" => TaskRole::Workers,
        "Images" => TaskRole::Images,
        "Models" => TaskRole::Models,
        "Sounds" => TaskRole::Sounds,
        "Ghidra" => TaskRole::Summaries,
        _ => bail!("Unknown task settings."),
    })
}

fn set_task_models(
    controls: &TaskControls<'_>,
    mut choices: PolicyChoices,
    permissions: &ProviderPermissions,
    role: TaskRole,
    for_run: bool,
) {
    let models = choices
        .models
        .iter()
        .map(|mut model| {
            if workshop::provider::provider_for(&model.value) == "antigravity"
                && let Some(name) = model.label.strip_suffix(" (Antigravity)")
            {
                model.label = name.into();
            }
            model
        })
        .collect::<Vec<_>>();
    let groups = PROVIDERS
        .into_iter()
        .filter(|id| role.supports(id))
        .filter_map(|id| {
            let enabled = permissions.allows(id, role);
            if for_run && !enabled {
                return None;
            }
            let indices = models
                .iter()
                .enumerate()
                .filter(|(_, m)| workshop::provider::provider_for(&m.value) == id)
                .map(|(index, _)| index as i32)
                .collect::<Vec<_>>();
            Some(TaskProvider {
                id: id.into(),
                label: provider_label(id).into(),
                enabled,
                models: ModelRc::new(VecModel::from(indices)),
            })
        })
        .collect::<Vec<_>>();
    controls.set_selected_models(models.iter().filter(|m| m.selected).count() as i32);
    choices.models = ModelRc::new(VecModel::from(models));
    controls.set_choices(choices);
    // Selection and effort edits update the existing controls, preserving keyboard focus.
    let previous = controls.get_providers();
    if previous.row_count() != groups.len()
        || previous.iter().zip(&groups).any(|(old, new)| {
            old.id != new.id
                || old.enabled != new.enabled
                || !old.models.iter().eq(new.models.iter())
        })
    {
        controls.set_providers(ModelRc::new(VecModel::from(groups)));
    }
}

// A section uses one heading row and as many full-width model rows as will fit.
// Repeat its provider heading if a very large catalog continues onto another page.
fn model_page(
    groups: ModelRc<TaskProvider>,
    columns: i32,
    rows: i32,
    requested: i32,
) -> TaskModelPage {
    let columns = columns.max(1) as usize;
    let rows = rows.max(2) as usize;
    let mut pages = vec![Vec::new()];
    let mut remaining = rows;
    for group in groups.iter() {
        let total = group.models.row_count();
        let mut start = 0;
        loop {
            if remaining < 1 + usize::from(start < total) {
                pages.push(Vec::new());
                remaining = rows;
            }
            let take = (total - start).min(columns * (remaining - 1));
            let models = if take == total {
                group.models.clone()
            } else {
                ModelRc::new(VecModel::from(
                    (start..start + take)
                        .filter_map(|i| group.models.row_data(i))
                        .collect::<Vec<_>>(),
                ))
            };
            pages.last_mut().unwrap().push(TaskProvider {
                models,
                ..group.clone()
            });
            remaining -= 1 + take.div_ceil(columns);
            start += take;
            if start == total {
                break;
            }
        }
    }
    let count = pages.len() as i32;
    let number = requested.clamp(0, count - 1);
    TaskModelPage {
        groups: ModelRc::new(VecModel::from(pages.swap_remove(number as usize))),
        number,
        count,
    }
}

impl App {
    fn set_task_provider(&self, provider: &str, role: TaskRole, enabled: bool) -> Result<()> {
        let home = data_dir()?;
        ProviderPermissions::set(&home, provider, role, enabled)?;
        self.permissions.replace(ProviderPermissions::load(&home)?);
        if role == TaskRole::Workers {
            self.clear_sessions();
            if let Some(store) = self.store.borrow().as_ref() {
                self.render_worker_policy(store, &self.catalog.borrow())?;
            }
        }
        self.filter_preferences(&self.ui.unwrap().get_settings_query());
        Ok(())
    }

    pub(super) fn task_settings(&self, field: &str, value: &str) -> Result<()> {
        let ui = self.ui.unwrap();
        let controls = ui.global::<TaskControls>();
        let area = controls.get_area().to_string();
        let run = controls.get_for_run();
        let home = data_dir()?;
        let role = task_role(&area)?;
        ensure!(
            !run || role != TaskRole::Workers,
            "Use chat subagent settings for worker overrides."
        );
        let defaults = if role == TaskRole::Workers {
            TaskSettings {
                agent: WorkerPolicy::global(&home)?.unwrap_or_default(),
                ..Default::default()
            }
        } else {
            TaskSettings::defaults(&home, &area)?
        };
        let mut drafts = self.task_overrides.borrow_mut();
        let mut setting = if run {
            drafts
                .get(&area)
                .cloned()
                .unwrap_or_else(|| defaults.clone())
        } else {
            defaults.clone()
        };
        if field != "open" {
            match field {
                "inherit" if run => {
                    drafts.remove(&area);
                    setting = defaults;
                }
                "customize" if run => {}
                "model" | "effort" => setting.agent.edit(field, value)?,
                "elevenlabs" if role == TaskRole::Sounds => setting.elevenlabs = value == "true",
                "summary" if area == "Ghidra" => setting.summarize = value == "true",
                "functions" if area == "Ghidra" => setting.function_limit = value.parse()?,
                "seconds" if area == "Ghidra" => setting.seconds_per_function = value.parse()?,
                _ => bail!("Unknown task setting."),
            }
            setting.validate()?;
            if run && field != "inherit" {
                ensure!(
                    field == "customize" || drafts.contains_key(&area),
                    "Turn off Use task defaults to edit this run."
                );
                drafts.insert(area.clone(), setting.clone());
            } else if role == TaskRole::Workers {
                setting.agent.save_global(&home)?;
                self.clear_sessions();
                if let Some(store) = self.store.borrow().as_ref() {
                    self.render_worker_policy(store, &self.catalog.borrow())?;
                }
            } else if !run {
                setting.save(&home, &area)?;
            }
        }
        controls.set_inherited(run && !drafts.contains_key(&area));
        let permissions = self.permissions.borrow();
        let choices = policy_choices(&setting.agent, &self.catalog.borrow(), &permissions, role);
        set_task_models(&controls, choices, &permissions, role, run);
        controls.set_function_limit(i32::from(setting.function_limit));
        controls.set_seconds(i32::from(setting.seconds_per_function));
        controls.set_summary(setting.summarize);
        controls.set_elevenlabs(setting.elevenlabs);
        controls
            .set_sound_key(std::env::var("ELEVENLABS_API_KEY").is_ok_and(|s| !s.trim().is_empty()));
        controls.set_error("".into());
        if self.store.borrow().is_none()
            && (area != "Ghidra" || setting.summarize)
            && !ui.get_catalog_ready()
            && ui.get_catalog_status().is_empty()
        {
            self.load_models(home);
        }
        let studio = ui.global::<Studio>();
        studio.set_custom_task(drafts.contains_key(studio.get_area().as_str()));
        Ok(())
    }
}
fn provider_id(label: &str) -> Result<&'static str> {
    PROVIDERS
        .into_iter()
        .find(|id| provider_label(id) == label)
        .context("Unknown provider")
}

pub(super) fn task_count(permissions: &ProviderPermissions, label: &str) -> Option<String> {
    let provider = provider_id(label).ok()?;
    let roles = TaskRole::ALL.into_iter().filter(|r| r.supports(provider));
    let total = roles.clone().count();
    let enabled = roles
        .filter(|role| permissions.allows(provider, *role))
        .count();
    Some(format!("{enabled} of {total} tasks"))
}

pub(super) fn connect(ui: &Desk, app: &Rc<App>) {
    ui.global::<TaskControls>().on_model_page(model_page);
    let a = app.clone();
    ui.global::<TaskControls>()
        .on_set_provider(move |id, enabled| {
            let ui = a.ui.unwrap();
            let controls = ui.global::<TaskControls>();
            let change = (|| -> Result<()> {
                ensure!(
                    !controls.get_for_run(),
                    "Provider permissions are shared across projects."
                );
                a.set_task_provider(&id, task_role(&controls.get_area())?, enabled)
            })();
            let refreshed = a.task_settings("open", "");
            if let Err(error) = change.and(refreshed) {
                controls.set_error(format!("{error:#}").into());
            }
        });
    let a = app.clone();
    ui.on_show_provider_roles(move |label| {
        let ui = a.ui.unwrap();
        let result = (|| -> Result<()> {
            let provider = provider_id(&label)?;
            let permissions = ProviderPermissions::load(&data_dir()?)?;
            let roles = TaskRole::ALL
                .into_iter()
                .filter(|role| role.supports(provider))
                .map(|role| PolicyModel {
                    value: role.id().into(),
                    label: role.label().into(),
                    selected: permissions.allows(provider, role),
                })
                .collect::<Vec<_>>();
            a.permissions.replace(permissions);
            a.filter_preferences(&ui.get_settings_query());
            ui.set_permission_error("".into());
            ui.set_provider_roles(ModelRc::new(VecModel::from(roles)));
            Ok(())
        })();
        if let Err(e) = result {
            ui.set_provider_roles(Default::default());
            ui.set_permission_error(format!("{e:#}").into());
        }
    });
    let a = app.clone();
    ui.on_set_provider_role(move |label, index, enabled| {
        let ui = a.ui.unwrap();
        let rows = ui.get_provider_roles();
        let Some(mut row) = rows.row_data(index as usize) else {
            return;
        };
        let result = (|| -> Result<()> {
            a.set_task_provider(
                provider_id(&label)?,
                TaskRole::from_id(&row.value)?,
                enabled,
            )?;
            row.selected = enabled;
            Ok(())
        })();
        rows.set_row_data(index as usize, row);
        ui.set_permission_error(
            result
                .err()
                .map(|e| format!("{e:#}"))
                .unwrap_or_default()
                .into(),
        );
    });
    let a = app.clone();
    ui.global::<TaskControls>().on_change(move |field, value| {
        if let Err(e) = a.task_settings(&field, &value) {
            a.ui.unwrap()
                .global::<TaskControls>()
                .set_error(format!("{e:#}").into());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_pages_keep_model_order_and_repeat_the_correct_provider() {
        let section = |name: &str, indices: std::ops::Range<i32>| TaskProvider {
            id: name.into(),
            label: name.into(),
            enabled: !indices.is_empty(),
            models: ModelRc::new(VecModel::from(indices.collect::<Vec<_>>())),
        };
        let groups = ModelRc::new(VecModel::from(vec![
            section("codex", 0..9),
            section("claude", 9..9),
            section("gemini", 9..12),
        ]));
        let wide = model_page(groups.clone(), 4, 8, 25);
        assert_eq!(
            (wide.number, wide.count, wide.groups.row_count()),
            (0, 1, 3)
        );
        let small = model_page(groups.clone(), 2, 3, -1);
        assert_eq!((small.number, small.count), (0, 4));
        let mut seen = Vec::new();
        for i in 0..small.count {
            let page = model_page(groups.clone(), 2, 3, i);
            let mut rows = 0;
            for group in page.groups.iter() {
                rows += 1 + group.models.row_count().div_ceil(2);
                for index in group.models.iter() {
                    assert_eq!(
                        group.id.as_str(),
                        if index < 9 { "codex" } else { "gemini" }
                    );
                    seen.push(index);
                }
            }
            assert!(rows <= 3);
        }
        assert_eq!(seen, (0..12).collect::<Vec<_>>());
        assert_eq!(model_page(groups, 2, 3, 42).number, small.count - 1);
        assert_eq!(model_page(ModelRc::default(), 0, 0, 9).count, 1);
    }
}
