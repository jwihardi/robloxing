use crate::{Appearance, Desk, ThemeChoice, ThemeColors};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use slint::{Color, ComponentHandle, ModelRc, VecModel};
use std::{fs, path::Path};

const DEFAULT_KEY: &str = "workshop-dark";
const DEFAULT_CSS: &str = include_str!("../themes/workshop-dark.css");
const PRESETS: &[(&str, &str, &str)] = &[
    (DEFAULT_KEY, "Workshop Dark", DEFAULT_CSS),
    ("nord", "Nord", include_str!("../themes/nord.css")),
    (
        "catppuccin-mocha",
        "Catppuccin Mocha",
        include_str!("../themes/catppuccin-mocha.css"),
    ),
    (
        "solarized-light",
        "Solarized Light",
        include_str!("../themes/solarized-light.css"),
    ),
];

fn hex_color(value: &str) -> Result<Color> {
    let hex = value.strip_prefix('#').unwrap_or_default();
    ensure!(
        matches!(hex.len(), 3 | 6) && hex.bytes().all(|b| b.is_ascii_hexdigit()),
        "Use a hex color such as #abc or #aabbcc."
    );
    let rgb = u32::from_str_radix(hex, 16)?;
    Ok(if hex.len() == 3 {
        Color::from_rgb_u8(
            ((rgb >> 8) as u8) * 17,
            ((rgb >> 4 & 15) as u8) * 17,
            ((rgb & 15) as u8) * 17,
        )
    } else {
        Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
    })
}

fn apply_css(css: &str, mut palette: ThemeColors) -> Result<ThemeColors> {
    let mut clean = String::new();
    let mut rest = css.trim_start_matches('\u{feff}');
    while let Some((before, comment)) = rest.split_once("/*") {
        clean.push_str(before);
        clean.push(' ');
        rest = comment
            .split_once("*/")
            .context("Close the CSS comment with */.")?
            .1;
    }
    clean.push_str(rest);
    let mut body = clean.trim();
    if let Some(block) = body.strip_prefix(":root") {
        body = block
            .trim()
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .context("Wrap the :root declarations in { and }.")?;
    }
    ensure!(
        !body.contains(['{', '}']),
        "Use one :root block or a list of color variables."
    );
    let mut has_colors = false;
    for line in body.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .context("Enter a declaration such as --accent: #b9c9ff;")?;
        let name = name.trim();
        let field = match name {
            "--rail" => &mut palette.rail,
            "--brand-fill" => &mut palette.brand_fill,
            "--popup" => &mut palette.popup,
            "--track" => &mut palette.track,
            "--canvas" => &mut palette.canvas,
            "--sidebar" => &mut palette.sidebar,
            "--surface" => &mut palette.surface,
            "--selected" => &mut palette.selected,
            "--line" => &mut palette.line,
            "--text" => &mut palette.text,
            "--muted" => &mut palette.muted,
            "--accent" => &mut palette.accent,
            "--accent-hover" => &mut palette.accent_hover,
            "--disabled" => &mut palette.disabled,
            "--ample" => &mut palette.ample,
            "--halfway" => &mut palette.halfway,
            "--scarce" => &mut palette.scarce,
            _ => bail!(
                "Unknown theme variable ‘{}’.",
                name.chars().take(48).collect::<String>()
            ),
        };
        *field = hex_color(value.trim()).with_context(|| format!("Invalid {name}"))?;
        has_colors = true;
    }
    ensure!(
        has_colors,
        "Enter at least one color variable to preview your theme."
    );
    Ok(palette)
}

fn default_colors() -> ThemeColors {
    apply_css(DEFAULT_CSS, ThemeColors::default()).expect("bundled Workshop theme")
}

fn choice(key: &str, name: &str, css: &str, custom: bool) -> Result<ThemeChoice> {
    Ok(ThemeChoice {
        key: key.into(),
        name: name.into(),
        css: css.into(),
        custom,
        colors: apply_css(css, default_colors())?,
    })
}

fn presets() -> Vec<ThemeChoice> {
    PRESETS
        .iter()
        .map(|(key, name, css)| choice(key, name, css, false).expect("bundled theme"))
        .collect()
}

fn database(home: &Path) -> Result<Connection> {
    fs::create_dir_all(home)?;
    let db = Connection::open(home.join("preferences.sqlite"))?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS custom_themes (
        id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL COLLATE NOCASE UNIQUE, css TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS appearance (id INTEGER PRIMARY KEY CHECK (id=1), theme TEXT NOT NULL);")?;
    Ok(db)
}

fn set_active(db: &Connection, key: &str) -> Result<()> {
    db.execute("INSERT INTO appearance(id,theme) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET theme=excluded.theme", [key])?;
    Ok(())
}

fn save_theme(db: &mut Connection, edit_key: &str, name: &str, css: &str) -> Result<()> {
    let name = name.trim();
    ensure!(!name.is_empty(), "Give your theme a name.");
    ensure!(
        !PRESETS.iter().any(|(_, n, _)| n.eq_ignore_ascii_case(name)),
        "Choose a name different from a built-in theme."
    );
    apply_css(css, default_colors())?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let same_name: Option<i64> = tx
        .query_row("SELECT id FROM custom_themes WHERE name=?1", [name], |r| {
            r.get(0)
        })
        .optional()?;
    ensure!(
        !same_name.is_some_and(|id| id.to_string() != edit_key),
        "Another theme already uses this name."
    );
    let id = if edit_key.is_empty() {
        tx.execute(
            "INSERT INTO custom_themes(name,css) VALUES(?1,?2)",
            params![name, css],
        )?;
        tx.last_insert_rowid()
    } else {
        let id = edit_key.parse::<i64>()?;
        ensure!(
            tx.execute(
                "UPDATE custom_themes SET name=?1,css=?2 WHERE id=?3",
                params![name, css, id]
            )? == 1,
            "This theme was removed. Create a new theme to save these colors."
        );
        id
    };
    set_active(&tx, &id.to_string())?;
    tx.commit()?;
    Ok(())
}

fn remove_theme(db: &mut Connection, key: &str) -> Result<()> {
    let id = key
        .parse::<i64>()
        .context("Only custom themes can be removed.")?;
    let tx = db.transaction()?;
    tx.execute("DELETE FROM custom_themes WHERE id=?1", [id])?;
    tx.execute(
        "UPDATE appearance SET theme=?1 WHERE id=1 AND theme=?2",
        [DEFAULT_KEY, key],
    )?;
    tx.commit()?;
    Ok(())
}

fn catalog(db: &Connection) -> Result<(Vec<ThemeChoice>, String)> {
    let mut entries = presets();
    let mut stmt =
        db.prepare("SELECT id,name,css FROM custom_themes ORDER BY name COLLATE NOCASE")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        entries.push(choice(
            &row.get::<_, i64>(0)?.to_string(),
            row.get_ref(1)?.as_str()?,
            row.get_ref(2)?.as_str()?,
            true,
        )?);
    }
    let active = db
        .query_row("SELECT theme FROM appearance WHERE id=1", [], |r| r.get(0))
        .optional()?
        .unwrap_or_else(|| DEFAULT_KEY.to_owned());
    Ok((entries, active))
}

fn publish(ui: &Desk, choices: Vec<ThemeChoice>, key: &str) {
    let current = choices
        .iter()
        .find(|t| t.key == key)
        .unwrap_or(&choices[0])
        .clone();
    let canvas = current.colors.canvas;
    let brightness = u32::from(canvas.red()) * 299
        + u32::from(canvas.green()) * 587
        + u32::from(canvas.blue()) * 114;
    ui.invoke_apply_colors(current.colors.clone(), brightness >= 128_000);
    let page = ui.global::<Appearance>();
    page.set_current(current);
    page.set_choices(ModelRc::new(VecModel::from(choices)));
}

fn refresh(ui: &Desk, db: &Connection) -> Result<()> {
    let (choices, key) = catalog(db)?;
    publish(ui, choices, &key);
    Ok(())
}

fn with_database(ui: &Desk, operation: impl FnOnce(&mut Connection) -> Result<()>) {
    let result = (|| -> Result<()> {
        let mut db = database(&workshop::data_dir()?)?;
        operation(&mut db)?;
        refresh(ui, &db)
    })();
    let page = ui.global::<Appearance>();
    match result {
        Ok(()) => page.set_status("".into()),
        Err(e) => page.set_status(format!("{e:#}").into()),
    }
}

pub fn reload(ui: &Desk) {
    with_database(ui, |_| Ok(()));
}

pub fn connect(ui: &Desk) {
    // Set the palette before the first frame, including when preferences are unavailable.
    publish(ui, presets(), DEFAULT_KEY);
    reload(ui);
    let page = ui.global::<Appearance>();
    let weak = ui.as_weak();
    page.on_select_theme(move |key| {
        let ui = weak.unwrap();
        with_database(&ui, |db| {
            if !PRESETS.iter().any(|(k, _, _)| *k == key.as_str()) {
                let exists: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM custom_themes WHERE id=?1)",
                    [key.as_str()],
                    |r| r.get(0),
                )?;
                ensure!(
                    exists,
                    "This theme was removed. Reopen Appearance to refresh the list."
                );
            }
            set_active(db, &key)
        });
    });
    let weak = ui.as_weak();
    page.on_validate(move |css| {
        let ui = weak.unwrap();
        let page = ui.global::<Appearance>();
        match apply_css(&css, default_colors()) {
            Ok(colors) => {
                page.set_preview(colors);
                page.set_syntax_error("".into());
            }
            Err(e) => page.set_syntax_error(format!("{e:#}").into()),
        }
    });
    let weak = ui.as_weak();
    page.on_save(move || {
        let ui = weak.unwrap();
        let page = ui.global::<Appearance>();
        with_database(&ui, |db| {
            save_theme(db, &page.get_edit_key(), &page.get_name(), &page.get_css())?;
            page.set_editing(false);
            Ok(())
        });
    });
    let weak = ui.as_weak();
    page.on_remove(move || {
        let ui = weak.unwrap();
        let page = ui.global::<Appearance>();
        with_database(&ui, |db| {
            remove_theme(db, &page.get_edit_key())?;
            page.set_editing(false);
            Ok(())
        });
    });
    let weak = ui.as_weak();
    page.on_load_file(move || {
        let ui = weak.unwrap();
        let page = ui.global::<Appearance>();
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Load theme CSS")
            .add_filter("Theme CSS", &["css"])
            .pick_file()
        {
            match fs::read_to_string(&path) {
                Ok(css) => {
                    if page.get_name().trim().is_empty()
                        && let Some(name) = path.file_stem()
                    {
                        page.set_name(name.to_string_lossy().as_ref().into());
                    }
                    page.set_css(css.into());
                    page.set_status("".into());
                }
                Err(e) => page.set_status(format!("Could not read theme file. {e}").into()),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_accepts_partial_palettes_and_names_invalid_variables() -> Result<()> {
        let defaults = default_colors();
        let palette = apply_css(
            "\u{feff}/* my theme */ :root { --accent: #123; --text: #ABCDEF }",
            defaults.clone(),
        )?;
        assert_eq!(palette.accent, Color::from_rgb_u8(0x11, 0x22, 0x33));
        assert_eq!(palette.text, Color::from_rgb_u8(0xab, 0xcd, 0xef));
        assert_eq!(palette.canvas, defaults.canvas);
        for invalid in [
            "--accnt: #fff",
            "--accent: red",
            ":root { --accent: #fff",
            "/* unfinished",
            ":root {}",
            "body { color: #123 }",
        ] {
            assert!(apply_css(invalid, defaults.clone()).is_err(), "{invalid}");
        }
        assert!(
            apply_css("--accent: #xyz", defaults)
                .unwrap_err()
                .to_string()
                .contains("--accent")
        );
        Ok(())
    }

    #[test]
    fn saved_themes_survive_restart_and_failed_edits_preserve_selection() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let mut db = database(temp.path())?;
        assert_eq!(catalog(&db)?.1, DEFAULT_KEY);
        save_theme(&mut db, "", "Amethyst", "--accent: #abc")?;
        let first_key = catalog(&db)?.1;
        save_theme(&mut db, "", "Forest", "--accent: #123")?;
        assert!(save_theme(&mut db, &first_key, "FOREST", "--accent: #456").is_err());
        assert!(save_theme(&mut db, &first_key, "Amethyst", "--accent: #xyz").is_err());
        assert!(save_theme(&mut db, "", "nord", "--text: #fff").is_err());
        let (choices, selected) = catalog(&db)?;
        assert_eq!(
            choices
                .iter()
                .find(|c| c.key == first_key.as_str())
                .unwrap()
                .css,
            "--accent: #abc"
        );
        assert_eq!(
            choices
                .iter()
                .find(|c| c.key == selected.as_str())
                .unwrap()
                .name,
            "Forest"
        );
        save_theme(&mut db, &first_key, "Iris", "--accent: #def")?;
        drop(db);
        let mut reopened = database(temp.path())?;
        let (choices, selected) = catalog(&reopened)?;
        assert_eq!(choices.len(), PRESETS.len() + 2);
        let active = choices.iter().find(|c| c.key == selected.as_str()).unwrap();
        assert_eq!(active.name, "Iris");
        assert_eq!(active.colors.accent, Color::from_rgb_u8(0xdd, 0xee, 0xff));
        remove_theme(&mut reopened, &selected)?;
        let (choices, selected) = catalog(&reopened)?;
        assert_eq!(choices.len(), PRESETS.len() + 1);
        assert_eq!(selected, DEFAULT_KEY);
        Ok(())
    }
}
