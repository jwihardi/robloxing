use super::{App, AssetPacks, Desk, PackMember, PackRow, Update};
use anyhow::{Context, Result, ensure};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::rc::Rc;
use workshop::Store;

impl App {
    pub(super) fn refresh_packs(&self, store: &Store, selection: Option<&str>) -> Result<()> {
        let ui = self.ui.unwrap();
        let view = ui.global::<AssetPacks>();
        let library = store.asset_packs();
        let packs = library.list()?;
        let previous = view.get_pack();
        let selected = selection.unwrap_or(&previous.key);
        let selected = packs
            .iter()
            .find(|p| p.key == selected)
            .or_else(|| packs.first());
        let row = |p: &workshop::studio::packs::Pack| PackRow {
            key: p.key.as_str().into(),
            name: p.name.as_str().into(),
        };
        view.set_packs(ModelRc::new(VecModel::from(
            packs.iter().map(row).collect::<Vec<_>>(),
        )));
        view.set_pack(selected.map(row).unwrap_or_default());
        view.set_style(
            selected
                .map(|p| p.style.as_str())
                .unwrap_or_default()
                .into(),
        );
        view.set_message("".into());
        let mut rows = Vec::new();
        if let Some(pack) = selected {
            for m in library.members(&pack.key)? {
                let [x, y, z] = m
                    .geometry
                    .dimensions
                    .map(|d| d * m.geometry.meters_per_unit);
                let path = library.member_dir(&pack.key, &m.key)?.join("preview.png");
                rows.push(PackMember {
                    key: m.key.into(),
                    name: m.name.into(),
                    description: m.description.into(),
                    geometry: format!(
                        "{} triangles · {x:.2} × {y:.2} × {z:.2} m",
                        m.geometry.triangles
                    )
                    .into(),
                    preview: slint::Image::load_from_path(&path)
                        .map_err(|_| anyhow::anyhow!("Cannot load preview {}", path.display()))?,
                });
            }
        }
        let active = view.get_member();
        let member = rows
            .iter()
            .find(|m| previous.key == view.get_pack().key && m.key == active.key)
            .or_else(|| rows.first())
            .cloned()
            .unwrap_or_default();
        view.set_member(member);
        view.set_members(ModelRc::new(VecModel::from(rows)));
        Ok(())
    }

    fn pack_action(&self, command: &str) -> Result<()> {
        let ui = self.ui.unwrap();
        let view = ui.global::<AssetPacks>();
        let pack = view.get_pack();
        let store = self
            .store
            .borrow()
            .clone()
            .context("Open a project first.")?;
        match command {
            "import" => {
                if let Some(folder) = rfd::FileDialog::new()
                    .set_title("Choose an exported asset pack")
                    .set_directory(&store.project)
                    .pick_folder()
                {
                    let updates = self.updates.clone();
                    view.set_message("Importing pack…".into());
                    std::thread::spawn(move || {
                        let result = store.asset_packs().import(&folder).map(|p| p.key);
                        let _ = updates.send(Update::PackImported(store.project.clone(), result));
                    });
                }
            }
            "chat" => {
                ensure!(!ui.get_busy(), "Wait for the current chat operation.");
                ui.invoke_open_asset_draft();
                let mut draft = ui.get_draft().to_string();
                if !draft.is_empty() {
                    draft.push('\n');
                }
                draft.push_str(&format!(
                    "Use the {} Blender asset pack ({}).",
                    pack.name, pack.key
                ));
                let member = view.get_member();
                if !member.key.is_empty() {
                    draft.push_str(&format!(" Reference {} ({}).", member.name, member.key));
                }
                ui.set_draft(draft.into());
                ui.invoke_focus_draft();
            }
            "export" => {
                if let Some(folder) = rfd::FileDialog::new()
                    .set_title("Export Blender asset pack into folder")
                    .set_directory(&store.project)
                    .pick_folder()
                {
                    let updates = self.updates.clone();
                    let key = pack.key.to_string();
                    view.set_message("Exporting pack…".into());
                    std::thread::spawn(move || {
                        let result = store.asset_packs().export(&key, &folder);
                        let _ =
                            updates.send(Update::PackExported(store.project.clone(), key, result));
                    });
                }
            }
            _ => anyhow::bail!("Unknown asset pack action."),
        }
        Ok(())
    }
}

pub(super) fn connect(ui: &Desk, app: &Rc<App>) {
    let a = app.clone();
    ui.global::<AssetPacks>().on_select_pack(move |key| {
        if let Some(store) = a.store.borrow().as_ref()
            && let Err(e) = a.refresh_packs(store, Some(&key))
        {
            a.ui.unwrap()
                .global::<AssetPacks>()
                .set_message(format!("{e:#}").into());
        }
    });
    let weak = ui.as_weak();
    ui.global::<AssetPacks>().on_select_member(move |key| {
        let ui = weak.unwrap();
        let view = ui.global::<AssetPacks>();
        if let Some(member) = view.get_members().iter().find(|m| m.key == key) {
            view.set_member(member);
        }
    });
    let a = app.clone();
    ui.global::<AssetPacks>().on_action(move |cmd| {
        if let Err(e) = a.pack_action(&cmd) {
            a.ui.unwrap()
                .global::<AssetPacks>()
                .set_message(format!("{e:#}").into());
        }
    });
}
