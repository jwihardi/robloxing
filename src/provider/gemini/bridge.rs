use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, Write},
    path::Path,
    time::Duration,
};

const SWITCH: &str = "--workshop-gemini-mcp";
const DIRECTORY: &str = "WORKSHOP_GEMINI_BRIDGE";
const LIMIT: u64 = 4 * 1024 * 1024;

pub fn is_bridge_process() -> bool {
    std::env::args_os().nth(1).is_some_and(|arg| arg == SWITCH)
}

// Gemini launches this executable as a standard MCP stdio server. A private
// directory scoped to the ACP process connects it to the live chat's Store.
pub(in crate::provider) struct Bridge {
    dir: tempfile::TempDir,
    pub(super) tools: Value,
    pub(in crate::provider) project_trusted: bool,
    pub(super) instructions: String,
}

impl Bridge {
    pub(super) fn new(worker: bool, project_trusted: bool) -> Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("workshop-gemini-")
            .tempdir()?;
        let mut tools = crate::team::tools(worker);
        for tool in tools.as_array_mut().context("Invalid Workshop tools")? {
            tool.as_object_mut()
                .context("Invalid tool definition")?
                .remove("type");
        }
        publish(dir.path(), "tools", &tools)?;
        // Delegate with Workshop so the user can see and control each worker.
        fs::write(
            dir.path().join("team.toml"),
            "[[rule]]\ntoolName = \"invoke_agent\"\ndecision = \"deny\"\npriority = 999\n",
        )?;
        Ok(Self {
            dir,
            tools,
            project_trusted,
            instructions: String::new(),
        })
    }
    pub(super) fn policy(&self) -> std::path::PathBuf {
        self.dir.path().join("team.toml")
    }
    pub(super) fn discovered(&self) -> bool {
        self.dir.path().join("discovered").is_file()
    }
    pub(super) fn config(&self) -> Result<Value> {
        Ok(
            json!({"name":"workshop","command":std::env::current_exe()?,"args":[SWITCH],
            "env":[{"name":DIRECTORY,"value":self.dir.path()}]}),
        )
    }
    pub(super) fn call(&self) -> Result<Option<Value>> {
        take(self.dir.path(), "call")
    }
    pub(super) fn reply(&self, result: Result<String>) -> Result<()> {
        let error = result.is_err();
        let text = result.unwrap_or_else(|e| format!("{e:#}"));
        publish(
            self.dir.path(),
            "reply",
            &json!({"isError":error,"content":[{"type":"text","text":text}]}),
        )
    }
}

fn publish(dir: &Path, name: &str, value: &Value) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(&mut file, value)?;
    file.flush()?;
    file.persist_noclobber(dir.join(name))?;
    Ok(())
}

fn take(dir: &Path, name: &str) -> Result<Option<Value>> {
    let path = dir.join(name);
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        file.metadata()?.len() <= LIMIT,
        "Workshop tool message is too large"
    );
    let value = serde_json::from_reader(file)?;
    fs::remove_file(path)?;
    Ok(Some(value))
}

pub fn serve_stdio() -> Result<()> {
    use std::io::Read;
    let path = std::env::var_os(DIRECTORY).context("Missing Workshop MCP session")?;
    let dir = Path::new(&path);
    let tools: Value = serde_json::from_reader(fs::File::open(dir.join("tools"))?)?;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let mut line = String::new();
    loop {
        line.clear();
        if input.by_ref().take(LIMIT + 1).read_line(&mut line)? == 0 {
            return Ok(());
        }
        ensure!(line.len() as u64 <= LIMIT, "MCP request exceeds size limit");
        let rpc: Value = serde_json::from_str(&line)?;
        let Some(id) = rpc.get("id") else { continue };
        let mut reply = json!({"jsonrpc":"2.0","id":id});
        match rpc["method"].as_str() {
            Some("initialize") => {
                let requested = &rpc["params"]["protocolVersion"];
                let supported = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
                let version = requested
                    .as_str()
                    .filter(|v| supported.contains(v))
                    .unwrap_or("2025-06-18");
                reply["result"] = json!({"protocolVersion":version,"capabilities":{"tools":{}},
                    "serverInfo":{"name":"workshop","version":env!("CARGO_PKG_VERSION")}});
            }
            Some("tools/list") => {
                fs::File::create(dir.join("discovered"))?;
                reply["result"] = json!({"tools":tools});
            }
            Some("ping") => reply["result"] = json!({}),
            Some("tools/call")
                if tools
                    .as_array()
                    .is_some_and(|ts| ts.iter().any(|t| t["name"] == rpc["params"]["name"])) =>
            {
                publish(dir, "call", &rpc["params"])?;
                loop {
                    if let Some(result) = take(dir, "reply")? {
                        reply["result"] = result;
                        break;
                    }
                    if !dir.exists() {
                        bail!("Workshop session closed");
                    }
                    std::thread::sleep(Duration::from_millis(40));
                }
            }
            _ => {
                reply["error"] = json!({"code":-32601,"message":"Unknown Workshop tool or method"})
            }
        }
        serde_json::to_writer(&mut output, &reply)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
