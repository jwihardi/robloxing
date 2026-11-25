use super::{App, Desk, Studio, TaskControls, policy_choices};
use anyhow::{Result, bail, ensure};
use slint::ComponentHandle;
use std::rc::Rc;
use workshop::{data_dir, studio::settings::TaskSettings};

impl App {
    pub(super) fn task_settings(&self, field: &str, value: &str) -> Result<()> {
        let ui = self.ui.unwrap();
        let controls = ui.global::<TaskControls>();
        let area = controls.get_area().to_string();
        let run = controls.get_for_run();
        let home = data_dir()?;
        let defaults = TaskSettings::defaults(&home, &area)?;
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
                "model" => {
                    if value.is_empty() {
                        setting.agent.models.clear();
                    } else if setting.agent.models.iter().any(|id| id == value) {
                        setting.agent.models.retain(|id| id != value);
                    } else {
                        setting.agent.models.push(value.to_string());
                    }
                }
                "effort" => setting.agent.max_effort = value.into(),
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
            } else if !run {
                setting.save(&home, &area)?;
            }
        }
        controls.set_inherited(run && !drafts.contains_key(&area));
        let mut catalog = self.catalog.borrow().clone();
        if area == "Images" {
            catalog
                .models
                .retain(|m| workshop::provider::provider_for(&m.model) == "codex");
        }
        controls.set_choices(policy_choices(&setting.agent, &catalog));
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
pub(super) fn connect(ui: &Desk, app: &Rc<App>) {
    let a = app.clone();
    ui.global::<TaskControls>().on_change(move |field, value| {
        if let Err(e) = a.task_settings(&field, &value) {
            a.ui.unwrap()
                .global::<TaskControls>()
                .set_error(format!("{e:#}").into());
        }
    });
}
