use super::{Connection, send};
use crate::usage::{UsageWindow, account_windows};
use anyhow::{Context, Result, ensure};
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub model: String,
    #[serde(default)]
    pub(super) display_name: Option<String>,
    #[serde(skip)]
    pub resolved_model: Option<String>,
    pub supported_reasoning_efforts: Vec<Effort>,
    pub default_reasoning_effort: String,
    pub(super) is_default: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Effort {
    pub reasoning_effort: String,
}

#[derive(Clone, Default)]
pub struct Catalog {
    pub models: Vec<Model>,
    pub default_model: String,
    pub default_effort: String,
}

impl Catalog {
    pub fn display_name(&self, selection: &str) -> Option<&str> {
        self.models
            .iter()
            .find(|m| m.model == selection)?
            .display_name
            .as_deref()
    }
    pub fn claude_label(&self, selection: &str) -> Option<String> {
        let value = selection.strip_prefix("claude:")?;
        let model = self.models.iter().find(|entry| entry.model == selection);
        Some(super::claude::model_label(
            value,
            model.and_then(|m| m.display_name.as_deref()),
            model.and_then(|m| m.resolved_model.as_deref()),
        ))
    }
}

pub(super) async fn request(
    io: &mut Connection,
    id: i32,
    method: &str,
    params: Value,
) -> Result<Value> {
    send(
        &mut io.stdin,
        json!({"id": id, "method": method, "params": params}),
    )
    .await?;
    while let Some(line) = io.lines.next_line().await? {
        let msg: Value = serde_json::from_str(&line)?;
        if msg["id"] == id {
            ensure!(
                msg.get("error").is_none(),
                "{method}: {}",
                msg["error"]["message"]
            );
            return msg.get("result").cloned().context("Missing Codex response");
        }
    }
    anyhow::bail!("Codex closed during {method}")
}

pub(super) async fn initialized(project: &Path) -> Result<Connection> {
    let mut io = Connection::spawn(project.to_owned(), tempfile::tempfile()?)?;
    request(&mut io, 1, "initialize", json!({"clientInfo":{"name":"workshop","title":"Workshop","version":env!("CARGO_PKG_VERSION")}})).await?;
    send(&mut io.stdin, json!({"method":"initialized"})).await?;
    Ok(io)
}

pub(super) async fn codex_limits(project: &Path) -> Result<Vec<UsageWindow>> {
    let mut io = initialized(project).await?;
    read_limits(&mut io).await
}

async fn read_limits(io: &mut Connection) -> Result<Vec<UsageWindow>> {
    let snapshot = request(io, -1, "account/rateLimits/read", json!({})).await?;
    account_windows(
        snapshot["rateLimitsByLimitId"]
            .get("codex")
            .unwrap_or(&snapshot["rateLimits"]),
    )
    .context("Codex did not report account limits")
}

async fn catalog_connection(project: &Path) -> Result<(Connection, Catalog)> {
    let catalog = tokio::time::timeout(Duration::from_secs(20), async {
        let mut io = initialized(project).await?;
        let config = request(&mut io, 2, "config/read", json!({"cwd":project})).await?;
        let mut models: Vec<Model> = Vec::new();
        let mut cursor = Value::Null;
        for id in 3.. {
            let page = request(
                &mut io,
                id,
                "model/list",
                json!({"cursor":cursor,"limit":100}),
            )
            .await?;
            models.extend(serde_json::from_value::<Vec<Model>>(page["data"].clone())?);
            let next = &page["nextCursor"];
            if next.is_null() {
                break;
            }
            ensure!(
                next != &cursor && id < 100,
                "Invalid model catalog pagination"
            );
            cursor = next.clone();
        }
        ensure!(!models.is_empty(), "Codex returned no models");
        let default_model = config["config"]["model"]
            .as_str()
            .or_else(|| {
                models
                    .iter()
                    .find(|m| m.is_default)
                    .map(|m| m.model.as_str())
            })
            .unwrap_or_default()
            .to_owned();
        Ok::<_, anyhow::Error>((
            io,
            Catalog {
                models,
                default_model,
                default_effort: config["config"]["model_reasoning_effort"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            },
        ))
    })
    .await
    .context("Codex model discovery timed out")??;
    Ok(catalog)
}

async fn all_providers(project: &Path) -> Result<(Option<Connection>, Catalog)> {
    let (codex, claude, gemini, antigravity) = tokio::join!(
        catalog_connection(project),
        tokio::time::timeout(Duration::from_secs(8), super::claude::models(project)),
        super::gemini::models(project),
        super::antigravity::models(project),
    );
    let claude = claude
        .context("Claude model discovery timed out")
        .and_then(|r| r);
    let (io, mut catalog) = match codex {
        Ok((io, catalog)) => (Some(io), catalog),
        Err(e) if claude.is_err() && gemini.is_err() && antigravity.is_err() => anyhow::bail!(
            "Model discovery failed. Codex: {e:#}. Claude: {}. Gemini: {}. Antigravity: {}",
            claude.err().unwrap(),
            gemini.err().unwrap(),
            antigravity.err().unwrap()
        ),
        Err(_) => (None, Catalog::default()),
    };
    if let Ok(models) = claude {
        catalog.models.extend(models);
    }
    if let Ok(other) = antigravity {
        catalog.models.extend(other.models);
    }
    if let Ok(other) = gemini {
        catalog.models.extend(other.models);
    }
    Ok((io, catalog))
}

pub(crate) async fn worker_catalog(project: &Path, only: Option<&str>) -> Result<Catalog> {
    match only {
        Some("codex") => Ok(catalog_connection(project).await?.1),
        Some("antigravity") => super::antigravity::models(project).await,
        Some("gemini") => super::gemini::models(project).await,
        Some("claude") => Ok(Catalog {
            models: tokio::time::timeout(Duration::from_secs(8), super::claude::models(project))
                .await
                .context("Claude model discovery timed out")??,
            default_model: "claude:default".into(),
            default_effort: String::new(),
        }),
        _ => Ok(all_providers(project).await?.1),
    }
}

pub fn model_catalog(project: &Path) -> Result<Catalog> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(all_providers(project))
        .map(|(_, catalog)| catalog)
}

/// Publish the available providers as each local CLI responds. Keep picker ordering
/// and the Codex default stable regardless of which CLI answered first.
pub fn discover_models(project: &Path, mut publish: impl FnMut(Catalog)) -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let sources = ["codex", "claude", "antigravity", "gemini"];
            let mut pending: FuturesUnordered<_> = sources
                .iter()
                .enumerate()
                .map(|(i, source)| async move { (i, worker_catalog(project, Some(source)).await) })
                .collect();
            let mut catalogs: [Option<Catalog>; 4] = Default::default();
            let mut errors = Vec::new();
            while let Some((i, result)) = pending.next().await {
                match result {
                    Ok(found) => {
                        catalogs[i] = Some(found);
                        let mut combined = catalogs[0].clone().unwrap_or_default();
                        for other in catalogs[1..].iter().flatten() {
                            combined.models.extend_from_slice(&other.models);
                        }
                        publish(combined);
                    }
                    Err(e) => errors.push(format!("{}: {e:#}", sources[i])),
                }
            }
            ensure!(
                catalogs.iter().any(Option::is_some),
                "{}",
                errors.join(". ")
            );
            Ok(())
        })
}

pub fn load_models_and_usage(
    project: &Path,
    loaded: impl FnOnce(Catalog),
) -> Result<Option<Vec<UsageWindow>>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let (io, catalog) = all_providers(project).await?;
            loaded(catalog);
            let Some(mut io) = io else {
                return Ok(None);
            };
            Ok(
                tokio::time::timeout(Duration::from_secs(6), read_limits(&mut io))
                    .await
                    .ok()
                    .and_then(Result::ok),
            )
        })
}
