use anyhow::Result;
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

macro_rules! skill_files {
    ($($path:literal),+ $(,)?) => {
        &[$(($path, include_bytes!(concat!("../../skills/game-ui-taste/", $path)))),+]
    };
}

const GAME_UI_FILES: &[(&str, &[u8])] = skill_files![
    "LICENSE",
    "README.md",
    "references/art-direction.md",
    "references/review.md",
    "references/runtime-and-input.md",
    "references/screen-patterns.md",
    "assets/review.template.json",
    "scripts/review_gate.py",
    "SKILL.md",
];

// Leads and workers share this Workshop-owned copy, outside the editable project.
pub(super) fn game_ui_taste(home: &Path) -> Result<PathBuf> {
    let folder = home.join("bundled-skills/1.0.0/game-ui-taste");
    for &(name, contents) in GAME_UI_FILES {
        let path = folder.join(name);
        match fs::read(&path) {
            Ok(existing) if existing == contents => continue,
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent)?;
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        staged.write_all(contents)?;
        staged.persist(path)?;
    }
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_the_complete_skill_from_the_binary() -> Result<()> {
        let home = tempfile::tempdir()?;
        let folder = game_ui_taste(home.path())?;
        fs::remove_file(folder.join("references/review.md"))?;
        fs::write(folder.join("scripts/review_gate.py"), "incomplete")?;
        assert_eq!(game_ui_taste(home.path())?, folder);
        for &(name, bytes) in GAME_UI_FILES {
            assert_eq!(fs::read(folder.join(name))?, bytes);
        }
        Ok(())
    }
}
