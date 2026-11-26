//! Project-scoped game asset jobs. Only finished runs expose deliverables.
mod chat;
pub mod reports;
pub mod settings;
use settings::TaskSettings;
mod textures;
mod tools;

use crate::{Store, provider};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::oneshot;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Recipe {
    Image { brief: String, style: String },
    Texture { material: String, seed: u32 },
    Model { template: String, name: String },
    ModelBrief { brief: String },
    Blend { source: PathBuf },
    Decompile { binary: PathBuf, filter: String },
}

impl Recipe {
    pub fn uses_task_settings(&self) -> bool {
        matches!(
            self,
            Self::Image { .. } | Self::ModelBrief { .. } | Self::Decompile { .. }
        )
    }
    pub fn area(&self) -> &'static str {
        match self {
            Self::Image { .. } | Self::Texture { .. } => "Images",
            Self::Model { .. } | Self::ModelBrief { .. } | Self::Blend { .. } => "Models",
            Self::Decompile { .. } => "Ghidra",
        }
    }
    fn title(&self) -> String {
        let title = match self {
            Self::Image { brief, .. } | Self::ModelBrief { brief } => brief.trim(),
            Self::Texture { material, seed } => return format!("{material} tile · {seed}"),
            Self::Model { name, .. } => name.trim(),
            Self::Blend { source } | Self::Decompile { binary: source, .. } => source
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled"),
        };
        title
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(90)
            .collect()
    }
    fn validate(&self) -> Result<()> {
        match self {
            Self::Image { brief, .. } => {
                ensure!(!brief.trim().is_empty(), "Describe the image to generate.")
            }
            Self::ModelBrief { brief } => {
                ensure!(!brief.trim().is_empty(), "Describe the model to create.")
            }
            Self::Texture { material, .. } => ensure!(
                matches!(material.as_str(), "Moss" | "Basalt" | "Sand"),
                "Choose a tile material."
            ),
            Self::Model { template, name } => {
                ensure!(!name.trim().is_empty(), "Name the model.");
                ensure!(
                    matches!(template.as_str(), "Crystal" | "Barrel" | "Planter"),
                    "Choose a starter mesh."
                );
            }
            Self::Blend { source } => ensure!(
                source.is_file()
                    && source
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("blend")),
                "Choose an existing Blender scene."
            ),
            Self::Decompile { binary, .. } => {
                ensure!(binary.is_file(), "Choose a local game binary.")
            }
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub struct Job {
    pub id: i64,
    pub title: String,
    pub state: String,
    pub detail: String,
    pub created: String,
    pub recipe: Recipe,
    pub settings: TaskSettings,
}
impl Job {
    pub fn filename(&self) -> &'static str {
        match self.recipe.area() {
            "Images" => "image.png",
            "Models" => "model.glb",
            _ => "functions.c",
        }
    }
    pub fn folder(&self, store: &Store) -> PathBuf {
        store.dir.join("studio").join(self.id.to_string())
    }
}

#[derive(Default)]
struct BuildContext<'a> {
    inference: Option<&'a (String, String)>,
    reference: Option<&'a Path>,
    delivery: Option<(i64, &'a str)>,
}

struct RunningJob<'a> {
    store: &'a Store,
    id: i64,
    finished: bool,
}

impl Drop for RunningJob<'_> {
    fn drop(&mut self) {
        if !self.finished
            && let Ok(db) = self.store.db.lock()
        {
            let _ = db.execute(
                "UPDATE studio_jobs SET state='Cancelled' WHERE id=?1 AND state='Running'",
                [self.id],
            );
        }
    }
}
fn same_contents(left: &Path, right: &Path) -> Result<bool> {
    let mut left = fs::File::open(left)?;
    let mut right = fs::File::open(right)?;
    if left.metadata()?.len() != right.metadata()?.len() {
        return Ok(false);
    }
    let mut a = [0u8; 32768];
    let mut b = [0u8; 32768];
    loop {
        let n = left.read(&mut a)?;
        if n == 0 {
            return Ok(true);
        }
        right.read_exact(&mut b[..n])?;
        if a[..n] != b[..n] {
            return Ok(false);
        }
    }
}
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    let recipe: String = r.get(5)?;
    #[derive(Deserialize)]
    struct Request {
        #[serde(flatten)]
        recipe: Recipe,
        #[serde(default)]
        settings: TaskSettings,
    }
    let request: Request = serde_json::from_str(&recipe).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(Job {
        id: r.get(0)?,
        title: r.get(1)?,
        state: r.get(2)?,
        detail: r.get(3)?,
        created: r.get(4)?,
        recipe: request.recipe,
        settings: request.settings,
    })
}
impl Store {
    pub(crate) fn import_studio_asset(
        &self,
        attempt: i64,
        asset: i64,
        destination: &str,
    ) -> Result<()> {
        let job = self.studio_job(asset)?;
        ensure!(
            job.state == "Ready" && matches!(job.recipe.area(), "Images" | "Models"),
            "Choose a ready game image or model."
        );
        let relative = destination.replace('\\', "/");
        let segments: Vec<_> = relative.split('/').collect();
        ensure!(
            segments.iter().all(|s| {
                let device = s
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .trim_end()
                    .to_ascii_uppercase();
                !s.is_empty()
                    && !s.starts_with('.')
                    && !s.ends_with([' ', '.'])
                    && s.chars().all(|c| c.is_alphanumeric() || " _-.".contains(c))
                    && !matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                    && !["COM", "LPT"].iter().any(|p| {
                        device.strip_prefix(p).is_some_and(|n| {
                            n.chars().count() == 1 && n.chars().all(char::is_numeric)
                        })
                    })
            }),
            "Use a project-relative asset path, such as Art/Prop.glb."
        );
        let file = Path::new(&relative);
        ensure!(
            file.extension()
                .zip(Path::new(job.filename()).extension())
                .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b)),
            "Use the asset's original file extension."
        );
        let mut dir = dunce::canonicalize(self.work(attempt))?;
        for segment in &segments[..segments.len() - 1] {
            dir.push(segment);
            if let Err(e) = fs::create_dir(&dir)
                && e.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(e.into());
            }
            ensure!(
                dir.is_dir() && dunce::canonicalize(&dir)? == dir,
                "Choose a directory inside the review copy without links."
            );
        }
        let dest = dir.join(segments.last().unwrap());
        let source = job.folder(self).join(job.filename());
        if fs::symlink_metadata(&dest).is_ok() {
            ensure!(
                dest.is_file()
                    && dunce::canonicalize(&dest)? == dest
                    && same_contents(&dest, &source)?,
                "A different file already exists at the destination. Choose another name."
            );
            return Ok(());
        }
        let mut temp = tempfile::NamedTempFile::new_in(&dir)?;
        std::io::copy(&mut fs::File::open(source)?, temp.as_file_mut())?;
        temp.persist_noclobber(dest)?;
        Ok(())
    }
    pub fn studio_history(&self, area: &str) -> Result<Vec<Job>> {
        self.find_studio_jobs(area, "", false, None)
    }
    pub fn find_studio_jobs(
        &self,
        area: &str,
        query: &str,
        ready_only: bool,
        limit: Option<u32>,
    ) -> Result<Vec<Job>> {
        let db = self.db.lock().unwrap();
        let mut q = db.prepare("SELECT id,title,state,detail,datetime(created,'localtime'),request FROM studio_jobs
            WHERE (area=?1 OR (?1='Assets' AND area IN ('Images','Models'))) AND (instr(lower(title),lower(?2))>0 OR (area='Ghidra' AND
            (instr(lower(json_extract(request,'$.filter')),lower(?2))>0 OR EXISTS(
            SELECT 1 FROM decompile_records r WHERE r.job=id AND (instr(lower(r.symbols),lower(?2))>0 OR instr(r.input_sha,lower(?2))>0)))))
            AND (?3=0 OR state='Ready') ORDER BY id DESC LIMIT ?4")?;
        Ok(q.query_map(
            params![area, query, ready_only, limit.map(i64::from).unwrap_or(-1)],
            row,
        )?
        .collect::<rusqlite::Result<_>>()?)
    }
    pub fn studio_job(&self, id: i64) -> Result<Job> {
        Ok(self.db.lock().unwrap().query_row("SELECT id,title,state,detail,datetime(created,'localtime'),request FROM studio_jobs WHERE id=?1", [id], row)?)
    }
    pub fn start_studio_job(&self, recipe: &Recipe) -> Result<i64> {
        self.queue_studio_job(recipe, None)
    }
    pub fn queue_studio_job(
        &self,
        recipe: &Recipe,
        overrides: Option<&TaskSettings>,
    ) -> Result<i64> {
        self.queue_named_studio_job(recipe, overrides, None)
    }
    fn queue_named_studio_job(
        &self,
        recipe: &Recipe,
        overrides: Option<&TaskSettings>,
        title: Option<&str>,
    ) -> Result<i64> {
        recipe.validate()?;
        let settings = match overrides {
            Some(value) => value.clone(),
            None if recipe.uses_task_settings() => TaskSettings::defaults(
                self.dir.parent().context("Missing preferences folder")?,
                recipe.area(),
            )?,
            None => TaskSettings::default(),
        };
        settings.validate()?;
        let mut request = serde_json::to_value(recipe)?;
        request["settings"] = serde_json::to_value(settings)?;
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO studio_jobs(area,title,request,state) VALUES(?1,?2,?3,'Queued')",
            params![
                recipe.area(),
                title
                    .and_then(|s| s.trim().lines().next())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.chars().take(72).collect())
                    .unwrap_or_else(|| recipe.title()),
                serde_json::to_string(&request)?
            ],
        )?;
        Ok(db.last_insert_rowid())
    }
    pub fn run_studio_job(&self, id: i64, cancel: oneshot::Receiver<()>) -> Result<()> {
        self.run_studio_job_with_refresh(id, cancel, false)
    }
    pub fn run_studio_job_with_refresh(
        &self,
        id: i64,
        cancel: oneshot::Receiver<()>,
        fresh: bool,
    ) -> Result<()> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(self.execute_studio_job(id, cancel, fresh, BuildContext::default()))
    }
    async fn execute_studio_job(
        &self,
        id: i64,
        mut cancel: oneshot::Receiver<()>,
        fresh: bool,
        context: BuildContext<'_>,
    ) -> Result<()> {
        ensure!(
            self.db.lock().unwrap().execute(
                "UPDATE studio_jobs SET state='Running' WHERE id=?1 AND state='Queued'",
                [id]
            )? == 1,
            "This job has already started."
        );
        let mut running = RunningJob {
            store: self,
            id,
            finished: false,
        };
        let job = self.studio_job(id)?;
        let folder = job.folder(self);
        let result = async {
            fs::create_dir_all(folder.parent().context("Missing job directory")?)?;
            fs::create_dir(&folder)?;
            if let Recipe::Decompile { binary, filter } = &job.recipe {
                reports::log(
                    &self.db.lock().unwrap(),
                    &folder,
                    &format!(
                        "Started run #{id} · {}\nFilter: {} · up to {} matching functions · {} s per function",
                        binary.display(),
                        if filter.trim().is_empty() {
                            "all"
                        } else {
                            filter.trim()
                        },
                        job.settings.function_limit,
                        job.settings.seconds_per_function
                    ),
                )?;
            }
            let detail = tokio::select! {
                biased;
                _ = &mut cancel => bail!("Cancelled"),
                result = tokio::time::timeout(Duration::from_secs(900), self.build_asset(&job, &folder, fresh, &context)) =>
                    result.context("The tool timed out after 15 minutes")?,
            }?;
            if matches!(job.recipe, Recipe::Decompile { .. }) && job.settings.summarize {
                // Preserve access to the completed local report if Workshop exits during the summary.
                self.db.lock().unwrap().execute(
                    "UPDATE studio_jobs SET state='Ready',detail=?2 WHERE id=?1",
                    params![id, detail],
                )?;
                self.run_report_summary(&job, &folder, &detail, cancel).await;
            }
            if let Some((attempt, maker)) = context.delivery {
                self.deliver_chat_asset(&job, attempt, maker, &detail)?;
            }
            Ok::<_, anyhow::Error>(detail)
        }.await;
        let (state, detail) = match &result {
            Ok(detail) => ("Ready", detail.clone()),
            Err(e) if e.to_string() == "Cancelled" => ("Cancelled", String::new()),
            Err(e) => ("Failed", format!("{e:#}")),
        };
        if result.is_err() || context.delivery.is_none() {
            self.db.lock().unwrap().execute(
                "UPDATE studio_jobs SET state=?2,detail=?3 WHERE id=?1",
                params![id, state, detail],
            )?;
        }
        running.finished = true;
        if job.recipe.area() == "Ghidra" && folder.is_dir() {
            reports::log(
                &self.db.lock().unwrap(),
                &folder,
                &if detail.is_empty() {
                    state.to_owned()
                } else {
                    format!("{state} · {detail}")
                },
            )?;
        }
        result.map(|_| ())
    }
    async fn run_report_summary(
        &self,
        job: &Job,
        folder: &Path,
        detail: &str,
        cancel: oneshot::Receiver<()>,
    ) {
        let request = async {
            reports::log(
                &self.db.lock().unwrap(),
                folder,
                "Requesting AI summary of the saved report.",
            )?;
            let inference = self
                .policy_inference(
                    crate::provider_permissions::TaskRole::Summaries,
                    &job.settings.agent,
                    "",
                    "",
                    "codex",
                )
                .await?;
            provider::summarize_report(folder, detail, &inference).await
        };
        let result = tokio::select! {
            biased;
            _ = cancel => Err(anyhow::anyhow!("Summary cancelled.")),
            result = tokio::time::timeout(Duration::from_secs(120), request) =>
                result.context("Summary timed out after two minutes.").and_then(|r| r),
        };
        let message = match result {
            Ok(()) => "AI summary saved.".into(),
            Err(e) => format!("AI summary unavailable: {e:#}"),
        };
        let _ = reports::log(&self.db.lock().unwrap(), folder, &message);
    }
    async fn build_asset(
        &self,
        job: &Job,
        folder: &Path,
        fresh: bool,
        context: &BuildContext<'_>,
    ) -> Result<String> {
        let recipe = &job.recipe;
        let inference = match (recipe.agent_role(), context.inference) {
            (Some(role), None) => Some(
                self.policy_inference(role, &job.settings.agent, "", "", "codex")
                    .await?,
            ),
            (_, selection) => selection.cloned(),
        };
        match recipe {
            Recipe::Image { brief, style } => {
                provider::generate_image(
                    folder,
                    brief,
                    style,
                    inference.as_ref().context("Missing image agent")?,
                    context.reference,
                )
                .await?
            }
            Recipe::Texture { material, seed } => textures::generate(folder, material, *seed)?,
            Recipe::Model { .. } | Recipe::ModelBrief { .. } | Recipe::Blend { .. } => {
                return tools::model(self, folder, job, inference.as_ref(), context.reference)
                    .await;
            }
            Recipe::Decompile { binary, filter } => {
                return tools::decompile(self, job, folder, binary, filter, fresh).await;
            }
        }
        let img = image::open(folder.join("image.png"))?;
        img.thumbnail(480, 480).save(folder.join("preview.png"))?;
        Ok(format!("PNG · {} × {}", img.width(), img.height()))
    }
}
