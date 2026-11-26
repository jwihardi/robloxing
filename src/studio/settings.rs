use crate::worker_policy::WorkerPolicy;
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskSettings {
    pub agent: WorkerPolicy,
    pub summarize: bool,
    pub elevenlabs: bool,
    pub function_limit: u16,
    pub seconds_per_function: u16,
}
impl Default for TaskSettings {
    fn default() -> Self {
        Self {
            agent: WorkerPolicy::default(),
            summarize: false,
            elevenlabs: false,
            function_limit: 64,
            seconds_per_function: 12,
        }
    }
}
impl TaskSettings {
    pub fn validate(&self) -> Result<()> {
        self.agent.validate()?;
        ensure!(
            (1..=256).contains(&self.function_limit),
            "Choose 1–256 functions per run."
        );
        ensure!(
            (1..=120).contains(&self.seconds_per_function),
            "Choose 1–120 seconds per function."
        );
        Ok(())
    }
    fn preferences(home: &Path, area: &str) -> Result<Connection> {
        ensure!(
            matches!(area, "Images" | "Models" | "Sounds" | "Ghidra"),
            "Unknown Studio task."
        );
        fs::create_dir_all(home)?;
        let db = Connection::open(home.join("preferences.sqlite"))?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS studio_defaults(area TEXT PRIMARY KEY,settings TEXT NOT NULL)")?;
        Ok(db)
    }
    pub fn defaults(home: &Path, area: &str) -> Result<Self> {
        let db = Self::preferences(home, area)?;
        let row: Option<String> = db
            .query_row(
                "SELECT settings FROM studio_defaults WHERE area=?",
                [area],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default())
    }
    pub fn save(&self, home: &Path, area: &str) -> Result<()> {
        self.validate()?;
        Self::preferences(home,area)?.execute("INSERT INTO studio_defaults VALUES(?1,?2) ON CONFLICT(area) DO UPDATE SET settings=excluded.settings",
            params![area,serde_json::to_string(self)?])?;
        Ok(())
    }
}
