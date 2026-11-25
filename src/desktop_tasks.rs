use super::{App, Desk, PolicyModel, Studio, TaskControls, policy_choices};
use anyhow::{Result, bail, ensure};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::rc::Rc;
use workshop::{
    data_dir,
    provider_permissions::{ProviderPermissions, TaskRole},
    studio::settings::TaskSettings,
    worker_policy::WorkerPolicy,
};

impl App {
    pub(super) fn task_settings(&self, field: &str, value: &str) -> Result<()> {
        let ui = self.ui.unwrap();
        let controls = ui.global::<TaskControls>();
        let area = controls.get_area().to_string();
        let run = controls.get_for_run();
        let home = data_dir()?;
        let role = match area.as_str() {
            "Workers" if !run => TaskRole::Workers,
            "Images" => TaskRole::Images,
            "Models" => TaskRole::Models,
            "Ghidra" => TaskRole::Summaries,
            _ => bail!("Unknown task settings."),
        };
        let defaults = if role == TaskRole::Workers {
            let agent = match self.store.borrow().as_ref() {
                Some(store) => store.worker_defaults()?,
                None => WorkerPolicy::global(&home)?.unwrap_or_default(),
            };
            TaskSettings {
                agent,
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
        controls.set_choices(policy_choices(
            &setting.agent,
            &self.catalog.borrow(),
            &self.permissions.borrow(),
            role,
        ));
        controls.set_function_limit(i32::from(setting.function_limit));
        controls.set_seconds(i32::from(setting.seconds_per_function));
        controls.set_summary(setting.summarize);
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
    match label {
        "Codex" => Ok("codex"),
        "Claude Code" => Ok("claude"),
        "Antigravity" => Ok("antigravity"),
        "Gemini CLI" => Ok("gemini"),
        _ => bail!("Unknown provider"),
    }
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
            let home = data_dir()?;
            ProviderPermissions::set(
                &home,
                provider_id(&label)?,
                TaskRole::from_id(&row.value)?,
                enabled,
            )?;
            a.permissions.replace(ProviderPermissions::load(&home)?);
            row.selected = enabled;
            Ok(())
        })();
        rows.set_row_data(index as usize, row);
        if result.is_ok() {
            a.filter_preferences(&ui.get_settings_query());
        }
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
