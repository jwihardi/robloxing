use super::{Connection, catalog::request, send};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::ImageReader;
use serde_json::{Value, json};
use std::{io::Cursor, path::Path};

async fn art_turn(
    dir: &Path,
    instructions: &str,
    prompt: &str,
    schema: Value,
    item_type: &str,
) -> Result<Value> {
    let mut io = Connection::spawn(dir.to_owned(), tempfile::tempfile()?)?;
    request(
        &mut io,
        1,
        "initialize",
        json!({
            "clientInfo":{"name":"workshop","title":"Workshop","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"experimentalApi":true}
        }),
    )
    .await?;
    send(&mut io.stdin, json!({"method":"initialized"})).await?;
    let thread = request(
        &mut io,
        2,
        "thread/start",
        json!({
            "cwd":dir, "ephemeral":true, "approvalPolicy":"never", "sandbox":"read-only",
            "config":{"features.image_generation":true,"features.multi_agent":false},
            "developerInstructions":instructions
        }),
    )
    .await?;
    let id = thread["thread"]["id"]
        .as_str()
        .context("Missing studio session")?;
    send(
        &mut io.stdin,
        json!({"id":3,"method":"turn/start","params":{"threadId":id,
        "outputSchema":schema, "input":[{"type":"text","text":prompt}]}}),
    )
    .await?;
    let mut output = None;
    while let Some(line) = io.lines.next_line().await? {
        let event: Value = serde_json::from_str(&line)?;
        if event["id"] == 3 && event.get("error").is_some() {
            bail!("Studio request failed: {}", event["error"]["message"]);
        }
        if event.get("method").is_some() && event.get("id").is_some() {
            send(
                &mut io.stdin,
                json!({"id":event["id"],"error":{"code":-32601,"message":"Unsupported studio tool"}}),
            )
            .await?;
            continue;
        }
        let p = &event["params"];
        if p["threadId"] != id {
            continue;
        }
        match event["method"].as_str() {
            Some("item/completed") if p["item"]["type"] == item_type => {
                output = Some(p["item"].clone())
            }
            Some("turn/completed") => {
                ensure!(
                    p["turn"]["status"] == "completed",
                    "{}",
                    p["turn"]["error"]["message"]
                        .as_str()
                        .unwrap_or("The studio request did not complete.")
                );
                return output.context("Codex did not produce the requested asset. Check provider sign-in and capabilities.");
            }
            _ => {}
        }
    }
    bail!("Codex closed before returning the asset")
}

pub(crate) async fn generate_image(dir: &Path, prompt: &str, style: &str) -> Result<()> {
    let item=art_turn(dir,"Use the built-in image generation tool to produce one game image for the user's art brief. Use no other tools.",
        &format!("Art direction: {style}.\n{prompt}"), Value::Null,"imageGeneration").await?;
    ensure!(
        item["failure"].is_null(),
        "Image generation failed: {}",
        item["failure"]
    );
    let decoded = if let Some(path) = item["savedPath"]
        .as_str()
        .filter(|p| Path::new(p).is_file())
    {
        ImageReader::open(path)?.with_guessed_format()?.decode()?
    } else {
        let encoded = item["result"].as_str().filter(|s| !s.is_empty()).context(
            "Codex returned no local image. Update the standalone Codex CLI and try again.",
        )?;
        let bytes = STANDARD.decode(encoded)?;
        ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()?
            .decode()?
    };
    decoded.save(dir.join("image.png"))?;
    Ok(())
}

pub(crate) async fn design_model(dir: &Path, brief: &str) -> Result<()> {
    let vector = json!({"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3});
    let schema = json!({"type":"object","properties":{"parts":{"type":"array","minItems":1,"maxItems":64,
        "items":{"type":"object","additionalProperties":false,"properties":{
            "name":{"type":"string"},"shape":{"type":"string","enum":["box","ellipsoid","cylinder","cone","torus"]},
            "size":vector,"center":vector,"rotation":vector,"color":{"type":"string"}},
            "required":["name","shape","size","center","rotation","color"]}}},"required":["parts"],"additionalProperties":false});
    let item=art_turn(dir,"Design a cohesive low-poly game prop using at most 64 named primitive meshes. Return just the JSON scene description. Size is full local XYZ dimensions in metres, center is XYZ position, rotation is XYZ Euler degrees, color is a six-digit sRGB hex value. Z is up; stand the prop on z=0. Give its silhouette distinctive proportions and connected parts. Use no tools.",
        brief,schema,"agentMessage").await?;
    let description: Value =
        serde_json::from_str(item["text"].as_str().context("Missing model design")?)?;
    std::fs::write(
        dir.join("design.json"),
        serde_json::to_vec_pretty(&description)?,
    )?;
    Ok(())
}
