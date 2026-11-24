use crate::{Store, provider, team::Assignment};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

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

    fn resolve(
        &self,
        model_id: &str,
        effort: &str,
        catalog: &provider::Catalog,
    ) -> Result<(String, String)> {
        self.validate()?;
        let allowed = |id: &str| self.models.is_empty() || self.models.iter().any(|m| m == id);
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
        policy.validate()?;
        self.db.lock().unwrap().execute(
            "INSERT INTO preferences.worker_defaults VALUES(1,?1)
            ON CONFLICT(id) DO UPDATE SET policy=excluded.policy",
            [serde_json::to_string(policy)?],
        )?;
        Ok(())
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
        if let Ok(mut cache) = self.worker_catalog.try_lock() {
            *cache = Some(catalog.clone());
        }
    }

    pub(crate) async fn worker_inference(
        &self,
        policy: &WorkerPolicy,
        task: &Assignment,
    ) -> Result<(String, String)> {
        self.policy_inference(policy, &task.model, &task.effort)
            .await
    }
    pub(crate) async fn policy_inference(
        &self,
        policy: &WorkerPolicy,
        model: &str,
        effort: &str,
    ) -> Result<(String, String)> {
        if policy.max_effort.is_empty()
            && effort.is_empty()
            && model.is_empty()
            && policy.models.is_empty()
        {
            return Ok((String::new(), String::new()));
        }
        let mut cache = self.worker_catalog.lock().await;
        if cache.is_none() {
            *cache = Some(provider::worker_catalog(&self.project).await?);
        }
        policy.resolve(model, effort, cache.as_ref().unwrap())
    }
}
