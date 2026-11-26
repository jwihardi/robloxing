use crate::{
    Store, provider,
    provider_permissions::{PROVIDERS, ProviderPermissions, TaskRole},
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

/// The policy and provider permissions used for one routing decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceLimits {
    pub policy: WorkerPolicy,
    pub providers: Vec<String>,
}

impl InferenceLimits {
    pub(crate) fn load(home: &Path, role: TaskRole, policy: &WorkerPolicy) -> Result<Self> {
        let permissions = ProviderPermissions::load(home)?;
        Ok(Self {
            policy: policy.clone(),
            providers: PROVIDERS
                .into_iter()
                .filter(|p| permissions.allows(p, role))
                .map(str::to_owned)
                .collect(),
        })
    }
    pub fn permits_provider(&self, model: &str) -> bool {
        self.providers
            .iter()
            .any(|p| p == provider::provider_for(model))
    }
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
        Ok(self.worker_defaults_with_source()?.0)
    }

    /// Effective defaults and whether this project contributes either setting.
    pub fn worker_defaults_with_source(&self) -> Result<(WorkerPolicy, bool)> {
        let db = self.db.lock().unwrap();
        let (model, effort, override_model, override_effort, global) = db.query_row(
            "SELECT worker_model, worker_effort, override_worker_model, override_worker_effort,
                    (SELECT policy FROM preferences.worker_defaults WHERE id=1)
             FROM team_settings WHERE id=1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )?;
        let uses_project = override_model
            || override_effort
            || (global.is_none() && (!model.is_empty() || !effort.is_empty()));
        let mut policy: WorkerPolicy = global
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        // Before global defaults existed, projects stored both fields here.
        if override_model || global.is_none() {
            policy.models = if model.is_empty() {
                vec![]
            } else {
                vec![model]
            };
        }
        if override_effort || global.is_none() {
            policy.max_effort = effort;
        }
        Ok((policy, uses_project))
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

    pub(crate) async fn policy_inference(
        &self,
        role: TaskRole,
        policy: &WorkerPolicy,
        model: &str,
        effort: &str,
        preferred: &str,
    ) -> Result<(String, String)> {
        let limits = InferenceLimits::load(&self.preferences_home, role, policy)?;
        self.select_inference(role, &limits, model, effort, preferred)
            .await
    }

    pub(crate) async fn select_inference(
        &self,
        role: TaskRole,
        limits: &InferenceLimits,
        model: &str,
        effort: &str,
        preferred: &str,
    ) -> Result<(String, String)> {
        let task = role.label();
        let policy = &limits.policy;
        let enabled: Vec<_> = limits.providers.iter().map(String::as_str).collect();
        ensure!(
            !enabled.is_empty(),
            "Enable a provider for {task} in Settings → Providers."
        );
        if !model.is_empty() {
            ensure!(
                limits.permits_provider(model),
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
        let inherited_model = policy.models.is_empty() && model.is_empty();
        if inherited_model && sources.contains(&preferred) {
            sources.retain(|p| *p == preferred);
        }
        let use_native_default =
            inherited_model && effort.is_empty() && policy.max_effort.is_empty();
        let native_default = |source: &str| {
            let selection = if source == "codex" {
                String::new()
            } else {
                format!("{source}:default")
            };
            (selection, String::new())
        };
        if use_native_default && sources.len() == 1 {
            return Ok(native_default(sources[0]));
        }
        let cache = self.worker_catalog.lock().unwrap().clone();
        let mut errors = Vec::new();
        for source in sources {
            let choice = cache.get(&self.project, source).await.and_then(|catalog| {
                if use_native_default {
                    ensure!(!catalog.models.is_empty(), "Provider returned no models");
                    Ok(native_default(source))
                } else {
                    policy.resolve(model, effort, catalog, source)
                }
            });
            match choice {
                Ok(choice) => return Ok(choice),
                Err(e) => errors.push(format!("{source}: {e:#}")),
            }
        }
        anyhow::bail!("No eligible model for {task}. {}", errors.join(". "))
    }
}
