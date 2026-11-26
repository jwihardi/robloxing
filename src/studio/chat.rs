use super::{BuildContext, Job, Recipe, Store, TaskSettings};
use crate::{Attempt, provider, provider_permissions::TaskRole};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::params;
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
pub(crate) struct AssetRequest {
    #[serde(flatten)]
    recipe: ChatRecipe,
    title: Option<String>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    effort: String,
    source_asset: Option<i64>,
    reference_image: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ChatRecipe {
    Image {
        brief: String,
        #[serde(default)]
        style: String,
    },
    Model {
        brief: String,
    },
    Texture {
        material: String,
        seed: u32,
    },
    Starter {
        template: String,
        brief: String,
    },
    Scene {
        path: Option<String>,
    },
}

impl Recipe {
    pub(super) fn agent_role(&self) -> Option<TaskRole> {
        match self {
            Self::Image { .. } => Some(TaskRole::Images),
            Self::ModelBrief { .. } => Some(TaskRole::Models),
            _ => None,
        }
    }
}

pub struct ChatOutput {
    pub title: String,
    pub detail: String,
    pub shot: String,
    pub image: Option<PathBuf>,
    pub preview: Option<PathBuf>,
    pub asset: Option<i64>,
}

fn image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp"
            )
        })
}

impl Store {
    fn chat_reference_image(&self, lead: &Attempt) -> Result<PathBuf> {
        let db = self.db.lock().unwrap();
        let pair = self.comparison_before(&db, lead.id, i64::MAX)?;
        if let Some(pair) = pair.as_ref().filter(|p| !p.previous_output) {
            return Ok(pair.markup.as_ref().unwrap_or(&pair.original).clone());
        }
        let last_input: Option<i64> = db.query_row(
            "SELECT MAX(id) FROM events WHERE attempt=?1 AND kind IN ('user/followup','steer/sent')",
            [lead.id], |r| r.get(0))?;
        let (sql, id) = match last_input {
            Some(event) => (
                "SELECT path FROM event_files WHERE event=?1 ORDER BY rowid",
                event,
            ),
            None => (
                "SELECT path FROM task_attachments WHERE task=?1 ORDER BY rowid",
                lead.task,
            ),
        };
        let mut files = db.prepare(sql)?;
        for file in files.query_map([id], |r| Ok(PathBuf::from(r.get::<_, String>(0)?)))? {
            let path = file?;
            if image_file(&path) {
                return Ok(path);
            }
        }
        pair.map(|p| p.original).context(
            "Attach a reference image, mark up an image or choose a previous image from this chat.",
        )
    }
    fn asset_input_file(&self, lead: &Attempt, path: &str) -> Result<PathBuf> {
        let work = dunce::canonicalize(self.work(lead.id))?;
        let path = dunce::canonicalize(work.join(path)).context("Cannot find the input file.")?;
        ensure!(path.is_file(), "Choose a regular input file.");
        if path.starts_with(&work) {
            return Ok(path);
        }
        let db = self.db.lock().unwrap();
        let mut files = db.prepare(
            "SELECT path FROM task_attachments WHERE task=?1 UNION ALL
            SELECT f.path FROM event_files f JOIN events e ON e.id=f.event WHERE e.attempt=?2",
        )?;
        for file in files.query_map(params![lead.task, lead.id], |r| r.get::<_, String>(0))? {
            if dunce::canonicalize(file?)
                .is_ok_and(|p| p == path || (p.is_dir() && path.starts_with(p)))
            {
                return Ok(path);
            }
        }
        bail!("Use a file in the chat's working copy or attach it to the request.")
    }

    fn chat_recipe(
        &self,
        lead: &Attempt,
        recipe: ChatRecipe,
        source_asset: Option<i64>,
        reference_image: Option<String>,
    ) -> Result<(Recipe, Option<PathBuf>)> {
        let source = source_asset.map(|id| self.studio_job(id)).transpose()?;
        if let Some(source) = &source {
            ensure!(source.state == "Ready", "Choose a finished source asset.");
        }
        let source_file = |area: &str, filename: &str| -> Result<PathBuf> {
            let job = source
                .as_ref()
                .filter(|j| j.recipe.area() == area)
                .context("Choose a source asset of the same kind.")?;
            let path = job.folder(self).join(filename);
            ensure!(
                path.is_file(),
                "The source asset has no {filename} to revise."
            );
            Ok(path)
        };
        ensure!(
            reference_image.is_none()
                || (matches!(recipe, ChatRecipe::Image { .. }) && source.is_none()),
            "For an image revision, choose one reference image or one source asset."
        );
        let (recipe, reference) = match recipe {
            ChatRecipe::Image { brief, style } => {
                let reference = match reference_image.as_deref() {
                    Some("chat") => Some(self.chat_reference_image(lead)?),
                    Some(path) => Some(self.asset_input_file(lead, path)?),
                    None if source.is_some() => Some(source_file("Images", "image.png")?),
                    None => None,
                };
                if let Some(p) = &reference {
                    ensure!(image_file(p), "Choose a PNG, JPEG or WebP reference image.");
                }
                (Recipe::Image { brief, style }, reference)
            }
            ChatRecipe::Model { brief } => {
                let reference = match &source {
                    Some(job) if matches!(job.recipe, Recipe::Model { .. }) => {
                        Some(source_file("Models", "request.json")?)
                    }
                    Some(_) => Some(source_file("Models", "design.json")?),
                    None => None,
                };
                (Recipe::ModelBrief { brief }, reference)
            }
            ChatRecipe::Scene { path } => {
                let path = match (path, &source) {
                    (Some(p), None) => self.asset_input_file(lead, &p)?,
                    (None, Some(_)) => source_file("Models", "model.blend")?,
                    _ => bail!("Choose a Blender scene path or a source asset to re-export."),
                };
                (Recipe::Blend { source: path }, None)
            }
            ChatRecipe::Texture { material, seed } => {
                ensure!(
                    source.is_none(),
                    "Choose a material and seed for a local tile."
                );
                (Recipe::Texture { material, seed }, None)
            }
            ChatRecipe::Starter { template, brief } => {
                ensure!(
                    source.is_none(),
                    "Choose a template and name for a starter."
                );
                (
                    Recipe::Model {
                        template,
                        name: brief,
                    },
                    None,
                )
            }
        };
        Ok((recipe, reference))
    }

    pub(crate) async fn generate_chat_asset(
        &self,
        lead: &Attempt,
        request: AssetRequest,
        notify: &dyn Fn(provider::ReplyUpdate<'_>),
    ) -> Result<String> {
        let (recipe, reference) = self.chat_recipe(
            lead,
            request.recipe,
            request.source_asset,
            request.reference_image,
        )?;
        recipe.validate()?;
        let area = recipe.area();
        notify(provider::ReplyUpdate::Activity(if area == "Images" {
            "Generating an image…"
        } else {
            "Building a 3D model…"
        }));
        let settings = TaskSettings::defaults(&self.preferences_home, area)?;
        let (selection, maker) = if let Some(role) = recipe.agent_role() {
            let selection = self
                .policy_inference(
                    role,
                    &settings.agent,
                    &request.model,
                    &request.effort,
                    &lead.provider,
                )
                .await?;
            let label = if selection.0.is_empty() {
                "Codex default"
            } else {
                &selection.0
            };
            let maker = if selection.1.is_empty() {
                label.to_owned()
            } else {
                format!("{label} · {}", selection.1)
            };
            (Some(selection), maker)
        } else {
            ensure!(
                request.model.is_empty() && request.effort.is_empty(),
                "This local recipe does not use an agent model."
            );
            (
                None,
                if area == "Images" {
                    "Local texture generator"
                } else {
                    "Blender"
                }
                .to_owned(),
            )
        };
        let id = self.queue_named_studio_job(&recipe, Some(&settings), request.title.as_deref())?;
        let (_keep_open, cancel) = tokio::sync::oneshot::channel();
        self.execute_studio_job(
            id,
            cancel,
            false,
            BuildContext {
                inference: selection.as_ref(),
                reference: reference.as_deref(),
                delivery: Some((lead.id, &maker)),
            },
        )
        .await?;
        notify(provider::ReplyUpdate::TranscriptChanged);
        let job = self.studio_job(id)?;
        Ok(serde_json::json!({"id":id,"title":job.title,"file":job.filename(),"maker":maker,"details":job.detail,"delivered_to_chat":true}).to_string())
    }

    pub(super) fn deliver_chat_asset(
        &self,
        job: &Job,
        attempt: i64,
        maker: &str,
        detail: &str,
    ) -> Result<()> {
        let image = if job.recipe.area() == "Images" {
            Some(self.save_chat_image(&fs::read(job.folder(self).join(job.filename()))?)?)
        } else {
            None
        };
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute(
            "UPDATE studio_jobs SET state='Ready',detail=?2 WHERE id=?1",
            params![job.id, detail],
        )?;
        tx.execute(
            "INSERT INTO events(attempt,kind,summary) VALUES(?1,?2,?3)",
            params![
                attempt,
                if image.is_some() {
                    "workshop/image"
                } else {
                    "workshop/model"
                },
                job.title
            ],
        )?;
        let event = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO event_assets(event,job,maker) VALUES(?1,?2,?3)",
            params![event, job.id, maker],
        )?;
        if let Some(path) = image {
            tx.execute(
                "INSERT INTO event_images(event,path) VALUES(?1,?2)",
                params![event, path.to_str().context("Invalid image path")?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn chat_outputs(&self, attempt: Option<i64>) -> Result<Vec<ChatOutput>> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT title,kind,shot,path,job, CASE date(stamp,'localtime') WHEN date('now','localtime') THEN strftime('%H:%M',stamp,'localtime') ELSE date(stamp,'localtime') END FROM (
               SELECT 'Screenshot ' || row_number() OVER (ORDER BY created,rowid) AS title, 'Screenshot' AS kind,
                      key AS shot, NULL AS path, NULL AS job, created AS stamp, rowid AS seq
               FROM preferences.screenshots WHERE project=?1 AND attempt IS ?2
               UNION ALL
               SELECT e.summary, CASE e.kind WHEN 'workshop/model' THEN '3D model' ELSE 'Image' END,
                      '', i.path, a.job, e.recorded, e.id
               FROM events e LEFT JOIN event_images i ON i.event=e.id LEFT JOIN event_assets a ON a.event=e.id
               WHERE e.attempt=?2 AND e.kind IN ('workshop/image','workshop/model')
             ) ORDER BY stamp DESC,seq DESC")?;
        Ok(stmt
            .query_map(params![self.project.to_string_lossy(), attempt], |r| {
                let asset: Option<i64> = r.get(4)?;
                let image = r.get::<_, Option<String>>(3)?.map(PathBuf::from);
                let preview = asset
                    .map(|id| {
                        self.dir
                            .join("studio")
                            .join(id.to_string())
                            .join("preview.png")
                    })
                    .or_else(|| image.clone());
                Ok(ChatOutput {
                    title: r.get(0)?,
                    detail: format!("{} · {}", r.get::<_, String>(1)?, r.get::<_, String>(5)?),
                    shot: r.get(2)?,
                    image,
                    preview,
                    asset,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions_can_read_files_attached_on_followups_only_in_their_own_chat() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = Store::open(&tmp.path().join("project"), &tmp.path().join("data"))?;
        let other = store.prepare("Earlier chat", "", None)?;
        store.finish(other.id, "review", "")?;
        let lead = store.prepare("Make a picture", "", None)?;
        let outside = tmp.path().join("reference.png");
        image::RgbaImage::new(4, 4).save(&outside)?;
        let path = outside.to_str().unwrap();
        assert!(store.asset_input_file(&lead, path).is_err());
        let message = provider::Message::compose("Use this image", vec![outside.clone()]);
        store.message_event(lead.id, "user/followup", &message)?;
        assert_eq!(
            store.asset_input_file(&lead, path)?,
            dunce::canonicalize(&outside)?
        );
        assert_eq!(store.chat_reference_image(&lead)?, outside);
        assert!(store.asset_input_file(&other, path).is_err());
        assert!(store.chat_reference_image(&other).is_err());
        store.message_event(lead.id, "user/followup", &"A separate request".into())?;
        assert!(store.chat_reference_image(&lead).is_err());
        Ok(())
    }
}
