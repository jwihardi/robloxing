use super::{catalog::Effort, *};
use crate::worker_policy::EFFORTS;
use tokio::io::AsyncReadExt;

pub fn program() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("WORKSHOP_ANTIGRAVITY") {
        let path = PathBuf::from(path);
        return dunce::canonicalize(path)
            .context("WORKSHOP_ANTIGRAVITY must point to the Antigravity executable");
    }
    let name = if cfg!(windows) { "agy.exe" } else { "agy" };
    let path = std::env::var_os("PATH").unwrap_or_default();
    let install = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("agy/bin"))
    } else {
        std::env::home_dir().map(|p| p.join(".local/bin"))
    };
    std::env::split_paths(&path)
        .chain(install)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|exe| exe.is_file())
        .context("Install Antigravity CLI, then refresh Workshop")
}

pub(super) fn variant<'a>(id: &'a str, label: &'a str) -> (&'a str, &'a str, Option<&'static str>) {
    if let Some((base, suffix)) = id.rsplit_once('-')
        && let Some((name, level)) = label.strip_suffix(')').and_then(|s| s.rsplit_once(" ("))
        && level.eq_ignore_ascii_case(suffix)
        && let Some(&effort) = EFFORTS.iter().find(|&&e| e == suffix)
    {
        return (base, name, Some(effort));
    }
    (id, label, None)
}

pub async fn sign_out() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut io = gemini::spawn_cli(
            scratch.path(),
            &["acp"],
            tempfile::tempfile()?,
            (program()?, None),
            "Antigravity",
        )?;
        gemini::initialize(&mut io.stdin).await?;
        let init = gemini::studio_response("Antigravity", &mut io, 1, None, false).await?;
        ensure!(
            init["agentCapabilities"]["auth"]["logout"].is_object(),
            "Use /logout in Antigravity to sign out with this CLI version"
        );
        gemini::rpc(&mut io.stdin, 2, "logout", json!({})).await?;
        gemini::studio_response("Antigravity", &mut io, 2, None, false).await?;
        Ok(())
    })
    .await
    .context("Antigravity sign-out timed out")?
}

pub(super) async fn studio(
    instructions: &str,
    prompt: &Message,
    schema: Value,
    inference: &(String, String),
    image: bool,
) -> Result<Value> {
    let scratch = tempfile::tempdir()?;
    let mut io = gemini::spawn_cli(
        scratch.path(),
        &["acp", "--sandbox", "--include-default-customizations=false"],
        tempfile::tempfile()?,
        (program()?, None),
        "Antigravity",
    )?;
    gemini::initialize(&mut io.stdin).await?;
    gemini::studio_response("Antigravity", &mut io, 1, None, false).await?;
    gemini::rpc(
        &mut io.stdin,
        2,
        "session/new",
        json!({"cwd":scratch.path(),"mcpServers":[]}),
    )
    .await?;
    let session = gemini::studio_response("Antigravity", &mut io, 2, None, false).await?;
    io.session = session["sessionId"]
        .as_str()
        .context("Missing Studio session")?
        .into();
    if let Some(model) = session_model(&session["models"], &inference.0, &inference.1)?
        && session["models"]["currentModelId"] != model
    {
        gemini::rpc(
            &mut io.stdin,
            3,
            "session/set_model",
            json!({"sessionId":io.session,"modelId":model}),
        )
        .await?;
        gemini::studio_response("Antigravity", &mut io, 3, None, false).await?;
    }
    let mut request = if image {
        "Use the built-in image-generator subagent to generate one image for the following brief. Wait for its finished image before replying. Use no other tools.".to_owned()
    } else {
        gemini::rpc(
            &mut io.stdin,
            4,
            "session/set_mode",
            json!({"sessionId":io.session,"modeId":"plan"}),
        )
        .await?;
        gemini::studio_response("Antigravity", &mut io, 4, None, false).await?;
        instructions.to_owned()
    };
    if !schema.is_null() {
        request.push_str(&format!(
            "\nReturn only JSON matching this schema: {schema}"
        ));
    }
    request.push_str(&format!("\n\n{}", prompt.text));
    let input = Message {
        text: request,
        files: prompt.files.clone(),
        queued_id: None,
    };
    gemini::rpc(
        &mut io.stdin,
        5,
        "session/prompt",
        json!({"sessionId":io.session,"prompt":gemini::content(&input)?}),
    )
    .await?;
    let mut text = String::new();
    let reply = gemini::studio_response("Antigravity", &mut io, 5, Some(&mut text), image).await?;
    ensure!(
        reply["stopReason"] == "end_turn",
        "Antigravity ended the Studio request: {}",
        reply["stopReason"]
    );
    let text = text.trim();
    if image {
        let root = std::env::home_dir()
            .context("Cannot find the Antigravity artifact folder")?
            .join(".gemini/antigravity-cli/brain");
        let path = session_image(&root, &io.session).with_context(|| {
            format!(
                "Antigravity did not return a generated image. {}",
                text.chars().take(250).collect::<String>()
            )
        })?;
        return Ok(json!({"savedPath":path}));
    }
    if schema.is_null() {
        return Ok(json!({"text":text}));
    }
    let raw = text
        .strip_prefix("```json")
        .and_then(|s| s.trim().strip_suffix("```"))
        .unwrap_or(text);
    let object: Value =
        serde_json::from_str(raw).context("Antigravity returned invalid Studio JSON")?;
    ensure!(object.is_object(), "Expected a Studio JSON object");
    Ok(json!({"text":object.to_string()}))
}

fn session_image(root: &Path, session: &str) -> Result<PathBuf> {
    ensure!(
        !session.is_empty() && session.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'),
        "Invalid image session ID"
    );
    let root = dunce::canonicalize(root)?;
    let dir = dunce::canonicalize(root.join(session))?;
    ensure!(
        dir.parent() == Some(root.as_path()),
        "Invalid image session folder"
    );
    // Only accept a real image file created in this new session. Model-supplied paths are untrusted.
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && matches!(
                entry.path().extension().and_then(|v| v.to_str()),
                Some("png" | "jpg" | "jpeg" | "webp")
            )
            && entry.metadata()?.len() <= 32 * 1024 * 1024
        {
            let path = dunce::canonicalize(entry.path())?;
            ensure!(
                path.parent() == Some(dir.as_path()),
                "Image is outside its session folder"
            );
            return Ok(path);
        }
    }
    bail!("No image artifact in this session")
}

fn catalog(entries: &[(&str, &str)]) -> Result<Catalog> {
    ensure!(
        !entries.is_empty(),
        "Antigravity returned no models. Check your sign-in in Settings → Providers."
    );
    let mut models = vec![CatalogModel {
        model: "antigravity:default".into(),
        display_name: Some("Antigravity default".into()),
        resolved_model: None,
        supported_reasoning_efforts: vec![],
        default_reasoning_effort: String::new(),
        is_default: false,
    }];
    for &(id, label) in entries {
        let (base, name, effort) = variant(id, label);
        let grouped = effort.is_some()
            && entries
                .iter()
                .any(|&(other, title)| other != id && variant(other, title).0 == base);
        let key = format!("antigravity:{}", if grouped { base } else { id });
        let entry = match models.iter_mut().find(|m| m.model == key) {
            Some(entry) => entry,
            None => {
                let display = if grouped { name } else { label };
                models.push(CatalogModel {
                    model: key,
                    display_name: Some(format!("{display} (Antigravity)")),
                    resolved_model: None,
                    supported_reasoning_efforts: vec![],
                    default_reasoning_effort: effort.unwrap_or_default().into(),
                    is_default: false,
                });
                models.last_mut().unwrap()
            }
        };
        if let Some(level) = effort
            && !entry
                .supported_reasoning_efforts
                .iter()
                .any(|e| e.reasoning_effort == level)
        {
            entry.supported_reasoning_efforts.push(Effort {
                reasoning_effort: level.into(),
            });
            entry
                .supported_reasoning_efforts
                .sort_by_key(|e| EFFORTS.iter().position(|&s| s == e.reasoning_effort));
        }
    }
    Ok(Catalog {
        models,
        default_model: "antigravity:default".into(),
        default_effort: String::new(),
    })
}

async fn output(dir: &Path, args: &[&str]) -> Result<String> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut io = gemini::spawn_cli(
            dir,
            args,
            tempfile::tempfile()?,
            (program()?, None),
            "Antigravity CLI",
        )?;
        let mut out = String::new();
        io.lines
            .reader
            .take(1024 * 1024 + 1)
            .read_to_string(&mut out)
            .await?;
        ensure!(
            out.len() <= 1024 * 1024,
            "Antigravity response exceeds 1 MiB"
        );
        ensure!(
            io.child.wait().await?.success(),
            "Antigravity request failed. Check your sign-in in Settings → Providers."
        );
        Ok(out)
    })
    .await
    .context("Antigravity request timed out")?
}

pub(super) async fn models(dir: &Path) -> Result<Catalog> {
    let (out, current) = tokio::join!(output(dir, &["models"]), current_model(dir));
    let out = out?;
    let entries: Vec<_> = out
        .lines()
        .filter_map(|line| {
            let (id, label) = line.trim().split_once(char::is_whitespace)?;
            if !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_./:".contains(&b))
            {
                return None;
            }
            Some((id, label.trim()))
        })
        .collect();
    let mut catalog = catalog(&entries)?;
    if let Ok(current) = current
        && let Some(&(id, label)) = entries.iter().find(|(id, _)| *id == current)
    {
        let (base, _, effort) = variant(id, label);
        if let Some(m) = catalog
            .models
            .iter()
            .skip(1)
            .find(|m| {
                m.model
                    .strip_prefix("antigravity:")
                    .is_some_and(|raw| raw == id || raw == base)
            })
            .cloned()
        {
            let alias = &mut catalog.models[0];
            alias.supported_reasoning_efforts = m.supported_reasoning_efforts;
            alias.default_reasoning_effort = effort.unwrap_or_default().into();
            alias.resolved_model = Some(current);
        }
    }
    Ok(catalog)
}

async fn current_model(dir: &Path) -> Result<String> {
    let out = output(dir, &["-p", "/model", "--output-format", "json"]).await?;
    let data: Value = serde_json::from_str(&out)?;
    data["command"]["data"]["id"]
        .as_str()
        .map(str::to_owned)
        .context("Antigravity did not report its current model")
}

pub async fn connected() -> bool {
    let Ok(dir) = tempfile::tempdir() else {
        return false;
    };
    current_model(dir.path()).await.is_ok()
}

pub fn usage_key(model: &str) -> &str {
    match model.strip_prefix("antigravity:") {
        Some(id) if id.starts_with("gemini-") => "antigravity:gemini",
        Some(id) if id.starts_with("claude-") || id.starts_with("gpt-") => "antigravity:3p",
        _ => model,
    }
}

pub(super) async fn account_limits_with_default(project: &Path) -> Result<AccountLimits> {
    let (limits, model) = tokio::join!(account_limits(), current_model(project));
    let mut limits = limits?;
    if let Ok(id) = model
        && let Some(windows) = limits.get(usage_key(&format!("antigravity:{id}"))).cloned()
    {
        limits.insert("antigravity:default".into(), windows);
    }
    Ok(limits)
}

pub async fn account_limits() -> Result<std::collections::HashMap<String, Vec<UsageWindow>>> {
    let dir = tempfile::tempdir()?;
    let usage = output(dir.path(), &["-p", "/usage", "--output-format", "json"]).await?;
    let usage: Value = serde_json::from_str(&usage)?;
    let groups = usage["command"]["data"]["groups"]
        .as_array()
        .context("Antigravity did not report account limits")?;
    let mut limits = std::collections::HashMap::<String, Vec<UsageWindow>>::new();
    for bucket in groups
        .iter()
        .filter_map(|g| g["buckets"].as_array())
        .flatten()
    {
        let Some((group, _)) = bucket["id"].as_str().and_then(|id| id.rsplit_once('-')) else {
            continue;
        };
        let minutes = match bucket["window"].as_str() {
            Some("weekly") => 10080,
            Some("5h") => 300,
            _ => continue,
        };
        let percent_left = bucket["remaining_fraction"]
            .as_f64()
            .map(|v| (100. * v.clamp(0., 1.)).round() as i64);
        limits
            .entry(format!("antigravity:{group}"))
            .or_default()
            .push(UsageWindow {
                minutes: Some(minutes),
                percent_left,
            });
    }
    for group in limits.values_mut() {
        group.sort_by_key(|w| w.minutes);
    }
    Ok(limits)
}

pub(super) fn session_model<'a>(
    models: &'a Value,
    selection: &str,
    effort: &str,
) -> Result<Option<&'a str>> {
    let id = selection
        .strip_prefix("antigravity:")
        .context("Choose an Antigravity model")?;
    if id == "default" && effort.is_empty() {
        return Ok(None);
    }
    let current = models["currentModelId"].as_str().unwrap_or_default();
    let available = models["availableModels"]
        .as_array()
        .context("Antigravity did not return its session models")?;
    let id = if id == "default" {
        let name = available
            .iter()
            .find(|m| m["modelId"] == current)
            .and_then(|m| m["name"].as_str())
            .unwrap_or(current);
        variant(current, name).0
    } else {
        id
    };
    for m in available {
        let Some(raw) = m["modelId"].as_str() else {
            continue;
        };
        let (family, _, level) = variant(raw, m["name"].as_str().unwrap_or(raw));
        if (raw == id || family == id) && (effort.is_empty() || level == Some(effort)) {
            return Ok(Some(raw));
        }
    }
    bail!(
        "Antigravity does not offer {selection} with the selected effort ({effort}). Refresh the model list."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn images_must_belong_to_the_new_session() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("abcd");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(root.path().join("other.jpg"), b"outside").unwrap();
        assert!(session_image(root.path(), "../abcd").is_err());
        assert!(session_image(root.path(), "abcd").is_err());
        std::fs::write(dir.join("cube.jpg"), b"candidate").unwrap();
        assert_eq!(
            session_image(root.path(), "abcd")
                .unwrap()
                .file_name()
                .unwrap(),
            "cube.jpg"
        );
    }
    #[test]
    fn live_variants_round_trip_without_conflating_accounts() {
        let entries = [
            ("g-high", "G (High)"),
            ("g-low", "G (Low)"),
            ("oss-medium", "OSS (Medium)"),
            ("claude-test", "Claude (Thinking)"),
        ];
        let c = catalog(&entries).unwrap();
        assert_eq!(c.models.len(), 4);
        assert_eq!(c.models[1].model, "antigravity:g");
        assert_eq!(
            c.models[1].supported_reasoning_efforts[0].reasoning_effort,
            "low"
        );
        assert_eq!(c.models[1].default_reasoning_effort, "high");
        assert_eq!(c.models[2].model, "antigravity:oss-medium");
        assert!(c.models[3].supported_reasoning_efforts.is_empty());
        let m = json!({"currentModelId":"g-high","availableModels":entries.map(|(id,name)|json!({"modelId":id,"name":name}))});
        assert_eq!(
            session_model(&m, "antigravity:g", "low").unwrap(),
            Some("g-low")
        );
        assert_eq!(
            session_model(&m, "antigravity:g", "").unwrap(),
            Some("g-high")
        );
        assert_eq!(
            session_model(&m, "antigravity:g-low", "").unwrap(),
            Some("g-low")
        );
        assert!(session_model(&m, "antigravity:g", "max").is_err());
        assert!(session_model(&m, "antigravity:claude-test", "low").is_err());
        assert_eq!(session_model(&m, "antigravity:default", "").unwrap(), None);
    }
}
