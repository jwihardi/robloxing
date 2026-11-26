use crate::{
    Store, provider,
    provider_permissions::{PROVIDERS, ProviderPermissions, TaskRole},
    team::Assignment,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const EFFORTS: &[&str] = &[
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerPolicy {
    pub models: Vec<String>,
    pub max_effort: String,
}

impl WorkerPolicy {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.max_effort.is_empty() || EFFORTS.contains(&self.max_effort.as_str()),
            "Unknown maximum effort"
        );
        ensure!(
            self.models.iter().all(|m| !m.trim().is_empty()),
            "Choose a named model"
        );
        Ok(())
    }

    pub fn edit(&mut self, field: &str, value: &str) -> Result<()> {
        match field {
            "model" if value.is_empty() => self.models.clear(),
            "model" if self.models.iter().any(|id| id == value) => {
                self.models.retain(|id| id != value)
            }
            "model" => self.models.push(value.to_owned()),
            "effort" => self.max_effort = value.to_owned(),
            _ => anyhow::bail!("Unknown model setting"),
        }
        self.validate()
    }

    fn preferences(home: &Path) -> Result<Connection> {
        std::fs::create_dir_all(home)?;
        let db = Connection::open(home.join("preferences.sqlite"))?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS worker_defaults(id INTEGER PRIMARY KEY CHECK(id=1),policy TEXT NOT NULL)")?;
        Ok(db)
    }

    pub fn global(home: &Path) -> Result<Option<Self>> {
        let row: Option<String> = Self::preferences(home)?
            .query_row("SELECT policy FROM worker_defaults WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(row.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn save_global(&self, home: &Path) -> Result<()> {
        self.validate()?;
        Self::preferences(home)?.execute(
            "INSERT INTO worker_defaults VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET policy=excluded.policy",
            [serde_json::to_string(self)?],
        )?;
        Ok(())
    }

    fn resolve(
        &self,
        model_id: &str,
        effort: &str,
        catalog: &provider::Catalog,
        source: &str,
    ) -> Result<(String, String)> {
        self.validate()?;
        let allowed = |id: &str| {
            provider::provider_for(id) == source
                && (self.models.is_empty() || self.models.iter().any(|m| m == id))
        };
        let rank = |value: &str| EFFORTS.iter().position(|&e| e == value);
        let cap = rank(&self.max_effort).unwrap_or(usize::MAX);
        let requested = if effort.is_empty() {
            cap
        } else {
            rank(effort).context("Unknown reasoning effort")?.min(cap)
        };
        let eligible = |m: &provider::CatalogModel| {
            allowed(&m.model)
                && (requested == usize::MAX
                    || m.supported_reasoning_efforts
                        .iter()
                        .any(|e| rank(&e.reasoning_effort).is_some_and(|r| r <= requested)))
        };
        let model = if model_id.is_empty() {
            catalog
                .models
                .iter()
                .find(|m| m.model == catalog.default_model && eligible(m))
                .or_else(|| {
                    catalog
                        .models
                        .iter()
                        .find(|m| m.model.ends_with(":default") && eligible(m))
                })
                .or_else(|| catalog.models.iter().find(|m| eligible(m)))
                .context("No available model matches the allowed models and effort limit")?
        } else {
            ensure!(
                allowed(model_id),
                "Model {model_id} is outside the allowed models"
            );
            catalog
                .models
                .iter()
                .find(|m| m.model == model_id)
                .with_context(|| format!("Model {model_id} is unavailable"))?
        };
        if self.max_effort.is_empty() && effort.is_empty() {
            return Ok((model.model.clone(), String::new()));
        }
        let effort = model
            .supported_reasoning_efforts
            .iter()
            .filter_map(|e| Some((rank(&e.reasoning_effort)?, &e.reasoning_effort)))
            .filter(|(rank, _)| *rank <= requested)
            .max_by_key(|(rank, _)| *rank)
            .with_context(|| {
                format!(
                    "{} has no supported reasoning effort within this limit",
                    model.model
                )
            })?;
        Ok((model.model.clone(), effort.1.clone()))
    }
}

impl Store {
    pub fn worker_defaults(&self) -> Result<WorkerPolicy> {
        let row: Option<String> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT policy FROM preferences.worker_defaults WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(policy) = row {
            return Ok(serde_json::from_str(&policy)?);
        }
        let legacy = self.team_settings()?;
        Ok(WorkerPolicy {
            models: if legacy.worker_model.is_empty() {
                vec![]
            } else {
                vec![legacy.worker_model]
            },
            max_effort: legacy.worker_effort,
        })
    }

    pub fn save_worker_defaults(&self, policy: &WorkerPolicy) -> Result<()> {
        policy.save_global(&self.preferences_home)
    }

    pub fn chat_worker_policy(&self, id: i64) -> Result<Option<WorkerPolicy>> {
        let row: Option<String> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT policy FROM chat_worker_policy WHERE attempt=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn set_chat_worker_policy(&self, id: i64, policy: Option<&WorkerPolicy>) -> Result<()> {
        let db = self.db.lock().unwrap();
        let state: String =
            db.query_row("SELECT state FROM attempts WHERE id=?1", [id], |r| r.get(0))?;
        ensure!(
            matches!(
                state.as_str(),
                "review" | "interrupted" | "failed" | "cancelled"
            ),
            "Change subagent settings between turns"
        );
        if let Some(policy) = policy {
            policy.validate()?;
            db.execute("INSERT INTO chat_worker_policy VALUES(?1,?2) ON CONFLICT(attempt) DO UPDATE SET policy=excluded.policy",
                params![id, serde_json::to_string(policy)?])?;
        } else {
            db.execute("DELETE FROM chat_worker_policy WHERE attempt=?1", [id])?;
        }
        Ok(())
    }

    pub(crate) fn effective_worker_policy(&self, id: i64) -> Result<WorkerPolicy> {
        match self.chat_worker_policy(id)? {
            Some(policy) => Ok(policy),
            None => self.worker_defaults(),
        }
    }

    pub fn cache_worker_catalog(&self, catalog: &provider::Catalog) {
        *self.worker_catalog.lock().unwrap() =
            std::sync::Arc::new(provider::CatalogSources::from_catalog(catalog));
    }

    pub fn refresh_model_sources(&self) -> std::sync::Arc<provider::CatalogSources> {
        let fresh = std::sync::Arc::new(provider::CatalogSources::default());
        *self.worker_catalog.lock().unwrap() = fresh.clone();
        fresh
    }

    pub(crate) async fn worker_inference(
        &self,
        policy: &WorkerPolicy,
        task: &Assignment,
        lead_provider: &str,
    ) -> Result<(String, String)> {
        self.policy_inference(
            TaskRole::Workers,
            policy,
            &task.model,
            &task.effort,
            lead_provider,
        )
        .await
    }

    pub(crate) async fn policy_inference(
        &self,
        role: TaskRole,
        policy: &WorkerPolicy,
        model: &str,
        effort: &str,
        preferred: &str,
    ) -> Result<(String, String)> {
        let permissions = ProviderPermissions::load(&self.preferences_home)?;
        let allows = |p: &str| permissions.allows(p, role);
        let task = role.label();
        let enabled: Vec<_> = PROVIDERS.into_iter().filter(|p| allows(p)).collect();
        ensure!(
            !enabled.is_empty(),
            "Enable a provider for {task} in Settings → Providers."
        );
        if !model.is_empty() {
            ensure!(
                allows(provider::provider_for(model)),
                "The provider for {model} is disabled for {task} in Settings → Providers."
            );
        }
        let mut sources = enabled;
        sources.retain(|source| {
            (model.is_empty() || provider::provider_for(model) == *source)
                && (policy.models.is_empty()
                    || policy
                        .models
                        .iter()
                        .any(|id| provider::provider_for(id) == *source))
        });
        ensure!(
            !sources.is_empty(),
            "The allowed models for {task} do not match an enabled provider. Check the model choices and Settings → Providers."
        );
        if policy.models.is_empty() {
            if model.is_empty() && sources.contains(&preferred) {
                sources.retain(|p| *p == preferred);
            }
            if model.is_empty()
                && effort.is_empty()
                && policy.max_effort.is_empty()
                && (sources.len() == 1 || sources[0] == "codex")
            {
                let source = sources[0];
                let selection = if source == "codex" {
                    String::new()
                } else {
                    format!("{source}:default")
                };
                return Ok((selection, String::new()));
            }
        }
        let cache = self.worker_catalog.lock().unwrap().clone();
        let mut errors = Vec::new();
        for source in sources {
            let available = match cache.get(&self.project, source).await {
                Ok(catalog) => catalog,
                Err(e) => {
                    errors.push(format!("{source}: {e:#}"));
                    continue;
                }
            };
            match policy.resolve(model, effort, available, source) {
                Ok(choice) => return Ok(choice),
                Err(e) => errors.push(format!("{source}: {e:#}")),
            }
        }
        anyhow::bail!("No eligible model for {task}. {}", errors.join(". "))
    }
}
