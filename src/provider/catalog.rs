use super::{Connection, send};
use crate::usage::{UsageWindow, account_windows};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub model: String,
    pub supported_reasoning_efforts: Vec<Effort>,
    pub default_reasoning_effort: String,
    is_default: bool,
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

async fn catalog_connection(project: &Path) -> Result<(Connection, Catalog)> {
    let mut io = Connection::spawn(project.to_owned(), tempfile::tempfile()?)?;
    let catalog = tokio::time::timeout(Duration::from_secs(20), async {
            request(&mut io, 1, "initialize", json!({"clientInfo":{"name":"workshop","title":"Workshop","version":env!("CARGO_PKG_VERSION")}})).await?;
            send(&mut io.stdin, json!({"method":"initialized"})).await?;
            let config = request(&mut io, 2, "config/read", json!({"cwd":project})).await?;
            let mut models: Vec<Model> = Vec::new();
            let mut cursor = Value::Null;
            for id in 3.. {
                let page = request(&mut io, id, "model/list", json!({"cursor":cursor,"limit":100})).await?;
                models.extend(serde_json::from_value::<Vec<Model>>(page["data"].clone())?);
                let next = &page["nextCursor"];
                if next.is_null() { break; }
                ensure!(next != &cursor && id < 100, "Invalid model catalog pagination");
                cursor = next.clone();
            }
            ensure!(!models.is_empty(), "Codex returned no models");
            let default_model = config["config"]["model"].as_str()
                .or_else(|| models.iter().find(|m| m.is_default).map(|m| m.model.as_str()))
                .unwrap_or_default().to_owned();
            Ok::<_, anyhow::Error>(Catalog { models, default_model, default_effort: config["config"]["model_reasoning_effort"].as_str().unwrap_or_default().to_owned() })
        }).await.context("Codex model discovery timed out")??;
    Ok((io, catalog))
}

pub(crate) async fn worker_catalog(project: &Path) -> Result<Catalog> {
    Ok(catalog_connection(project).await?.1)
}

pub fn model_catalog(project: &Path) -> Result<Catalog> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(catalog_connection(project))
        .map(|(_, catalog)| catalog)
}

pub fn load_models_and_usage(
    project: &Path,
    loaded: impl FnOnce(Catalog),
) -> Result<Option<Vec<UsageWindow>>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let (mut io, catalog) = catalog_connection(project).await?;
            loaded(catalog);
            let limits = tokio::time::timeout(
                Duration::from_secs(6),
                request(&mut io, -1, "account/rateLimits/read", json!({})),
            )
            .await
            .ok()
            .and_then(Result::ok);
            Ok(limits.and_then(|v| {
                account_windows(
                    v["rateLimitsByLimitId"]
                        .get("codex")
                        .unwrap_or(&v["rateLimits"]),
                )
            }))
        })
}
