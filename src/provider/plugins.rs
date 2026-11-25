use super::{catalog, send};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub enabled: bool,
    pub install_policy: Option<String>,
    pub availability: Option<String>,
    pub interface: Option<Interface>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Interface {
    pub display_name: Option<String>,
    pub short_description: Option<String>,
}

#[derive(Deserialize)]
pub struct Marketplace {
    pub name: String,
    pub plugins: Vec<Plugin>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    marketplaces: Vec<Marketplace>,
    #[serde(default)]
    marketplace_load_errors: Vec<LoadError>,
}

#[derive(Deserialize)]
struct LoadError {
    message: String,
}

#[derive(Deserialize)]
struct Response {
    id: Option<serde_json::Value>,
    result: Option<Catalog>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

/// Older servers without plugin/list can still use the CLI listing command.
pub async fn list(refresh: bool) -> Result<Option<Vec<Marketplace>>> {
    let mut io = catalog::initialized(&std::env::current_dir()?).await?;
    send(
        &mut io.stdin,
        json!({"id":2,"method":"plugin/list","params":{"forceRefetch":refresh}}),
    )
    .await?;
    // Large catalogs include icons and other metadata that the desktop list never displays.
    // Deserialize only its summary fields and keep the higher frame limit local to this request.
    while let Some(line) = io.lines.next_line_with_limit(128 * 1024 * 1024).await? {
        let response: Response = serde_json::from_str(&line)?;
        if response.id != Some(json!(2)) {
            continue;
        }
        if let Some(error) = response.error {
            if error.code == -32601 {
                return Ok(None);
            }
            bail!("Codex plugin catalog: {}", error.message);
        }
        let catalog = response.result.context("Missing Codex plugin catalog")?;
        if catalog.marketplaces.is_empty()
            && let Some(error) = catalog.marketplace_load_errors.first()
        {
            bail!("Could not load the plugin marketplace: {}", error.message);
        }
        return Ok(Some(catalog.marketplaces));
    }
    bail!("Codex closed before returning its plugin catalog")
}
