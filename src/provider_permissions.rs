use anyhow::{Context, Result, ensure};
use rusqlite::Connection;
use std::{fs, path::Path};

pub const PROVIDERS: [&str; 4] = ["codex", "claude", "antigravity", "gemini"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TaskRole {
    Workers,
    Images,
    Models,
    Summaries,
}

impl TaskRole {
    pub const ALL: [Self; 4] = [Self::Workers, Self::Images, Self::Models, Self::Summaries];

    pub fn id(self) -> &'static str {
        match self {
            Self::Workers => "workers",
            Self::Images => "images",
            Self::Models => "models",
            Self::Summaries => "summaries",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Workers => "Workers",
            Self::Images => "Image generation",
            Self::Models => "Model design",
            Self::Summaries => "Decompilation summaries",
        }
    }

    pub fn from_id(id: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|role| role.id() == id)
            .context("Unknown provider task")
    }

    pub fn supports(self, provider: &str) -> bool {
        PROVIDERS.contains(&provider)
            && (self != Self::Images || matches!(provider, "codex" | "antigravity"))
    }
}

pub struct ProviderPermissions {
    disabled: Vec<(String, String)>,
}

impl ProviderPermissions {
    fn database(home: &Path) -> Result<Connection> {
        fs::create_dir_all(home)?;
        let db = Connection::open(home.join("preferences.sqlite"))?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS disabled_provider_tasks(provider TEXT NOT NULL, role TEXT NOT NULL, PRIMARY KEY(provider, role))")?;
        Ok(db)
    }

    pub fn load(home: &Path) -> Result<Self> {
        let db = Self::database(home)?;
        let mut query = db.prepare("SELECT provider, role FROM disabled_provider_tasks")?;
        let disabled = query
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Self { disabled })
    }

    pub fn allows(&self, provider: &str, role: TaskRole) -> bool {
        role.supports(provider)
            && !self
                .disabled
                .iter()
                .any(|(p, r)| p == provider && r == role.id())
    }

    pub fn set(home: &Path, provider: &str, role: TaskRole, enabled: bool) -> Result<()> {
        ensure!(
            role.supports(provider),
            "This provider does not support {}.",
            role.label()
        );
        let db = Self::database(home)?;
        let sql = if enabled {
            "DELETE FROM disabled_provider_tasks WHERE provider=?1 AND role=?2"
        } else {
            "INSERT OR IGNORE INTO disabled_provider_tasks(provider,role) VALUES(?1,?2)"
        };
        db.execute(sql, [provider, role.id()])?;
        Ok(())
    }
}
