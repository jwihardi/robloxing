use super::{Connection, send};
use crate::provider_permissions::PROVIDERS;
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

async fn provider_catalog(project: &Path, source: &str) -> Result<(Option<Connection>, Catalog)> {
    let catalog = match source {
        "codex" => {
            let (io, catalog) = catalog_connection(project).await?;
            return Ok((Some(io), catalog));
        }
        "claude" => Catalog {
            models: tokio::time::timeout(Duration::from_secs(8), super::claude::models(project))
                .await
                .context("Claude model discovery timed out")??,
            default_model: "claude:default".into(),
            default_effort: String::new(),
        },
        "antigravity" => super::antigravity::models(project).await?,
        "gemini" => super::gemini::models(project).await?,
        _ => anyhow::bail!("Unknown model provider {source}"),
    };
    Ok((None, catalog))
}

async fn gather<F: Future<Output = Result<(Option<Connection>, Catalog)>>>(
    load: impl Fn(&'static str) -> F,
    mut publish: impl FnMut(&Catalog),
) -> Result<(Option<Connection>, Catalog)> {
    let mut pending: FuturesUnordered<_> = PROVIDERS
        .into_iter()
        .enumerate()
        .map(|(i, source)| {
            let request = load(source);
            async move { (i, request.await) }
        })
        .collect();
    let mut parts: [Option<Catalog>; PROVIDERS.len()] = Default::default();
    let mut errors = Vec::new();
    let mut connection = None;
    let mut combined = Catalog::default();
    while let Some((i, result)) = pending.next().await {
        match result {
            Ok((io, found)) => {
                if io.is_some() {
                    connection = io;
                }
                parts[i] = Some(found);
                combined = parts[0].clone().unwrap_or_default();
                for catalog in parts[1..].iter().flatten() {
                    combined.models.extend_from_slice(&catalog.models);
                }
                publish(&combined);
            }
            Err(e) => errors.push(format!("{}: {e:#}", PROVIDERS[i])),
        }
    }
    ensure!(
        parts.iter().any(Option::is_some),
        "Model discovery failed. {}",
        errors.join(". ")
    );
    Ok((connection, combined))
}

#[derive(Default)]
pub struct CatalogSources {
    sources: [tokio::sync::OnceCell<Catalog>; PROVIDERS.len()],
}

impl CatalogSources {
    pub(crate) async fn get(&self, project: &Path, source: &str) -> Result<&Catalog> {
        let i = PROVIDERS
            .iter()
            .position(|&p| p == source)
            .context("Unknown model provider")?;
        self.sources[i]
            .get_or_try_init(|| async { Ok(provider_catalog(project, source).await?.1) })
            .await
    }

    pub(crate) fn from_catalog(catalog: &Catalog) -> Self {
        let cache = Self::default();
        for (source, slot) in PROVIDERS.into_iter().zip(&cache.sources) {
            let models: Vec<_> = catalog
                .models
                .iter()
                .filter(|m| super::provider_for(&m.model) == source)
                .cloned()
                .collect();
            if !models.is_empty() {
                let _ = slot.set(Catalog {
                    models,
                    default_model: catalog.default_model.clone(),
                    default_effort: catalog.default_effort.clone(),
                });
            }
        }
        cache
    }

    /// Publish incremental snapshots in picker order while workers share the same in-flight requests.
    pub fn discover(&self, project: &Path, mut publish: impl FnMut(Catalog)) -> Result<()> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(gather(
                |source| async move { Ok((None, self.get(project, source).await?.clone())) },
                |c| publish(c.clone()),
            ))?;
        Ok(())
    }
}

pub(crate) async fn worker_catalog(project: &Path, only: Option<&str>) -> Result<Catalog> {
    match only {
        Some(source) => Ok(provider_catalog(project, source).await?.1),
        None => Ok(gather(|source| provider_catalog(project, source), |_| {})
            .await?
            .1),
    }
}

pub fn model_catalog(project: &Path) -> Result<Catalog> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(worker_catalog(project, None))
}

pub fn discover_models(project: &Path, publish: impl FnMut(Catalog)) -> Result<()> {
    CatalogSources::default().discover(project, publish)
}

pub fn load_models_and_usage(
    project: &Path,
    loaded: impl FnOnce(Catalog),
) -> Result<Option<Vec<UsageWindow>>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let (io, catalog) = gather(|source| provider_catalog(project, source), |_| {}).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn workers_join_pending_discovery_without_waiting_for_another_provider() -> Result<()> {
        let cache = CatalogSources::default();
        let index = |source| PROVIDERS.iter().position(|&p| p == source).unwrap();
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let codex = cache.sources[index("codex")].get_or_init(|| async {
            started.send(()).unwrap();
            released.await.unwrap();
            Catalog {
                default_model: "fixture-codex".into(),
                ..Default::default()
            }
        });
        let other = Catalog {
            default_model: "claude:sonnet".into(),
            ..Default::default()
        };
        assert!(cache.sources[index("claude")].set(other).is_ok());
        let requests = async {
            waiting.await?;
            let missing = tempfile::tempdir()?.path().join("absent");
            let worker = cache.get(&missing, "codex");
            tokio::pin!(worker);
            assert!(futures_util::poll!(&mut worker).is_pending());
            assert_eq!(
                cache.get(&missing, "claude").await?.default_model,
                "claude:sonnet"
            );
            release.send(()).unwrap();
            assert_eq!(worker.await?.default_model, "fixture-codex");
            assert_eq!(
                cache.get(&missing, "codex").await?.default_model,
                "fixture-codex"
            );
            Ok::<_, anyhow::Error>(())
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(codex, requests).1
        })
        .await??;
        Ok(())
    }
}
