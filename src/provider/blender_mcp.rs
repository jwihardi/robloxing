use super::{Frames, managed, send};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path, process::Stdio, time::Duration};
use tokio::io::BufReader;

#[derive(Clone, Serialize, Deserialize)]
pub struct Server {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub(crate) tools: Vec<String>,
}

fn file(home: &Path) -> std::path::PathBuf {
    home.join("blender-mcp.json")
}

pub fn configured(home: &Path) -> Result<Option<Server>> {
    match fs::read(file(home)) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).context("Invalid Blender MCP connection")?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn disconnect(home: &Path) -> Result<()> {
    match fs::remove_file(file(home)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => Ok(result?),
    }
}

impl Server {
    pub fn new(command: String, args: Vec<String>) -> Self {
        Self {
            command,
            args,
            tools: Vec::new(),
        }
    }

    pub(super) fn codex(&self) -> Value {
        json!({"command":self.command,"args":self.args})
    }
    pub(super) fn claude(&self) -> Value {
        json!({"type":"stdio","command":self.command,"args":self.args})
    }
    pub(super) fn acp(&self) -> Value {
        json!({"name":"blender","command":self.command,"args":self.args,"env":[]})
    }

    fn from_codex(entry: &Value) -> Result<Self> {
        ensure!(
            entry["enabled"] == true && entry["transport"]["type"] == "stdio",
            "Choose an enabled stdio Blender connection in Codex."
        );
        let transport = &entry["transport"];
        ensure!(
            entry["enabled_tools"].is_null()
                && entry["disabled_tools"].as_array().is_none_or(Vec::is_empty)
                && transport["cwd"].is_null()
                && transport["env"].as_object().is_none_or(|m| m.is_empty())
                && transport["env_vars"].as_array().is_none_or(Vec::is_empty),
            "This Codex entry uses custom tool or environment settings. Choose a standalone Blender MCP executable to share."
        );
        Ok(serde_json::from_value(transport.clone())?)
    }

    /// Import only on request. Opening a chat never starts a CLI to look up another CLI.
    pub async fn discover() -> Result<Self> {
        let mut cmd = tokio::process::Command::new(super::codex_program());
        cmd.args(["mcp", "get", "blender", "--json"])
            .kill_on_drop(true)
            .stdin(Stdio::null());
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        if let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(5), cmd.output()).await
            && out.status.success()
        {
            return Self::from_codex(&serde_json::from_slice(&out.stdout)?);
        }
        Ok(Self::new("blender-mcp".into(), Vec::new()))
    }

    fn tool_count(count: usize) -> String {
        let noun = if count == 1 { "tool" } else { "tools" };
        format!("Blender MCP answered with {count} {noun}.")
    }

    pub async fn connect(&self, home: &Path) -> Result<String> {
        let mut server = self.clone();
        server.tools = self.discover_tools().await?;
        let message = Self::tool_count(server.tools.len());
        fs::create_dir_all(home)?;
        let mut staged = tempfile::NamedTempFile::new_in(home)?;
        serde_json::to_writer(&mut staged, &server)?;
        staged.flush()?;
        staged.persist(file(home))?;
        Ok(message)
    }

    pub async fn test(&self) -> Result<String> {
        Ok(Self::tool_count(self.discover_tools().await?.len()))
    }

    async fn discover_tools(&self) -> Result<Vec<String>> {
        tokio::time::timeout(Duration::from_secs(12), self.handshake())
            .await
            .context("Blender MCP timed out during tool discovery")?
    }

    async fn handshake(&self) -> Result<Vec<String>> {
        let scratch = tempfile::tempdir()?;
        let args: Vec<_> = self.args.iter().map(String::as_str).collect();
        let mut child = managed(&self.command, &args, scratch.path(), Stdio::piped(), tempfile::tempfile()?)
            .context("Could not start Blender MCP. Choose its executable or configure the blender server in Codex.")?;
        let mut stdin = child.stdin().take().context("Blender MCP stdin")?;
        let mut frames = Frames {
            reader: BufReader::new(child.stdout().take().context("Blender MCP stdout")?),
            pending: Vec::new(),
        };
        async fn response(frames: &mut Frames, id: i64) -> Result<Value> {
            while let Some(line) = frames.next_line().await? {
                let msg: Value = serde_json::from_str(&line)?;
                if msg["id"] == id {
                    ensure!(
                        msg.get("error").is_none(),
                        "Blender MCP: {}",
                        msg["error"]["message"]
                    );
                    return msg
                        .get("result")
                        .cloned()
                        .context("Missing Blender MCP reply");
                }
            }
            bail!("Blender MCP closed before answering")
        }
        send(&mut stdin,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"workshop","version":env!("CARGO_PKG_VERSION")}}})).await?;
        response(&mut frames, 1).await?;
        send(
            &mut stdin,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await?;
        let mut params = json!({});
        let mut seen = std::collections::HashSet::new();
        let mut names = Vec::new();
        for id in 2.. {
            send(
                &mut stdin,
                json!({"jsonrpc":"2.0","id":id,"method":"tools/list","params":params}),
            )
            .await?;
            let catalog = response(&mut frames, id).await?;
            for entry in catalog["tools"]
                .as_array()
                .context("Blender MCP returned no tool catalog")?
            {
                names.push(
                    entry["name"]
                        .as_str()
                        .context("Missing Blender MCP tool name")?
                        .to_owned(),
                );
            }
            let Some(cursor) = catalog["nextCursor"].as_str().filter(|c| !c.is_empty()) else {
                break;
            };
            ensure!(
                seen.insert(cursor.to_owned()),
                "Blender MCP repeated a tool catalog page"
            );
            params = json!({"cursor":cursor});
        }
        ensure!(!names.is_empty(), "Blender MCP has no available tools");
        Ok(names)
    }
}
