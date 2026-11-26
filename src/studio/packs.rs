//! Project-wide Blender asset libraries. Each pack and each member becomes visible
//! in one rename, so parallel workers and readers never see an unfinished member.
use super::{copy_to_work, reports::fingerprint, tools};
use crate::{Attempt, Store, provider, team::ToolOutput};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

const CATALOG: &str = "blender_assets.cats.txt";
const MEMBER_FILES: [&str; 4] = ["asset.json", "model.blend", "model.glb", "preview.png"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pack {
    pub version: u32,
    pub key: String,
    pub name: String,
    pub style: String,
    pub catalog_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    pub color: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Geometry {
    pub collection: String,
    pub meshes: u64,
    pub triangles: u64,
    /// Axis-aligned XYZ extents in Blender units.
    pub dimensions: [f64; 3],
    pub meters_per_unit: f64,
    pub materials: Vec<Material>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Member {
    pub key: String,
    pub name: String,
    pub description: String,
    pub palette: Vec<String>,
    source_sha256: String,
    blend_sha256: String,
    glb_sha256: String,
    preview_sha256: String,
    source_collection: Option<String>,
    #[serde(flatten)]
    pub geometry: Geometry,
}

pub struct Library {
    root: PathBuf,
}

fn key(value: &str) -> Result<&str> {
    let bytes = value.as_bytes();
    ensure!(
        !bytes.is_empty()
            && bytes.len() <= 64
            && bytes[0].is_ascii_alphanumeric()
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
            && !matches!(value, "con" | "nul" | "aux" | "prn")
            && !["com", "lpt"].iter().any(|prefix| value
                .strip_prefix(prefix)
                .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit())),
        "Use a lowercase key such as harbor-props, with letters, numbers and hyphens."
    );
    Ok(value)
}

fn label(value: &str, max: usize) -> Result<&str> {
    let value = value.trim();
    ensure!(
        !value.is_empty() && value.chars().count() <= max,
        "Enter text between 1 and {max} characters."
    );
    Ok(value)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_reader(
        fs::File::open(path).with_context(|| format!("Cannot read {}", path.display()))?,
    )
    .with_context(|| format!("Invalid {}", path.display()))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn checksum(path: &Path) -> Result<String> {
    fingerprint(fs::File::open(path)?, io::sink())
}

fn preview_palette(path: &Path) -> Result<Vec<String>> {
    let image = image::open(path)?.thumbnail(96, 96).into_rgba8();
    let mut bins = [[0u32; 4]; 512];
    for pixel in image.pixels().filter(|p| p[3] >= 192) {
        let [r, g, b, _] = pixel.0;
        let index = (usize::from(r >> 5) * 8 + usize::from(g >> 5)) * 8 + usize::from(b >> 5);
        let bin = &mut bins[index];
        bin[0] += 1;
        for (total, channel) in bin[1..].iter_mut().zip([r, g, b]) {
            *total += u32::from(channel);
        }
    }
    bins.sort_by_key(|bin| std::cmp::Reverse(bin[0]));
    Ok(bins
        .iter()
        .take(6)
        .take_while(|b| b[0] > 0)
        .map(|[n, r, g, b]| format!("#{:02X}{:02X}{:02X}", r / n, g / n, b / n))
        .collect())
}

fn plain_file(root: &Path, relative: &Path) -> Result<PathBuf> {
    let path = root.join(relative);
    ensure!(
        path.is_file() && dunce::canonicalize(&path)? == path,
        "Expected a regular library file at {}",
        path.display()
    );
    Ok(path)
}

fn directories(parent: &Path) -> Result<Vec<String>> {
    let mut result = Vec::new();
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(result),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if key(&name).is_ok() && entry.file_type()?.is_dir() {
            result.push(name);
        }
    }
    result.sort();
    Ok(result)
}

fn stage(parent: &Path) -> Result<tempfile::TempDir> {
    fs::create_dir_all(parent)?;
    Ok(tempfile::Builder::new()
        .prefix(".publishing-")
        .tempdir_in(parent)?)
}

impl Store {
    pub fn asset_packs(&self) -> Library {
        Library::in_archive(&self.archive_dir)
    }
}

impl Library {
    pub fn in_archive(archive: &Path) -> Self {
        Self {
            root: archive.join("asset-packs"),
        }
    }

    pub fn list(&self) -> Result<Vec<Pack>> {
        directories(&self.root)?
            .iter()
            .map(|k| self.pack(k))
            .collect()
    }

    pub fn pack(&self, pack: &str) -> Result<Pack> {
        let result: Pack = read_json(&self.root.join(key(pack)?).join("pack.json"))
            .with_context(|| format!("Cannot open asset pack '{pack}'"))?;
        ensure!(
            result.version == 1 && result.key == pack,
            "Unsupported or mismatched pack manifest."
        );
        Ok(result)
    }

    pub fn members(&self, pack: &str) -> Result<Vec<Member>> {
        self.pack(pack)?;
        directories(&self.root.join(pack).join("assets"))?
            .iter()
            .map(|k| self.member(pack, k))
            .collect()
    }

    pub fn member_dir(&self, pack: &str, member: &str) -> Result<PathBuf> {
        Ok(self.root.join(key(pack)?).join("assets").join(key(member)?))
    }

    pub fn member(&self, pack: &str, member: &str) -> Result<Member> {
        let result: Member = read_json(&self.member_dir(pack, member)?.join("asset.json"))
            .with_context(|| format!("Cannot open '{member}' in pack '{pack}'"))?;
        ensure!(result.key == member, "Mismatched asset manifest.");
        Ok(result)
    }

    pub fn create(&self, id: &str, name: &str, style: &str) -> Result<Pack> {
        let id = key(id)?;
        let hash = crate::digest(id.as_bytes());
        let catalog_id = format!(
            "{}-{}-8{}-a{}-{}",
            &hash[..8],
            &hash[8..12],
            &hash[13..16],
            &hash[17..20],
            &hash[20..32]
        );
        let pack = Pack {
            version: 1,
            key: id.into(),
            name: label(name, 100)?.into(),
            style: label(style, 3000)?.into(),
            catalog_id,
        };
        let temp = stage(&self.root)?;
        write_json(&temp.path().join("pack.json"), &pack)?;
        fs::create_dir(temp.path().join("assets"))?;
        fs::write(
            temp.path().join(CATALOG),
            format!("VERSION 1\n\n{}:{}:{}\n", pack.catalog_id, id, id),
        )?;
        let destination = self.root.join(id);
        if let Err(e) = fs::rename(temp.path(), &destination) {
            if !destination.exists() {
                return Err(e.into());
            }
            ensure!(
                self.pack(id)? == pack,
                "A pack with this key and different art direction already exists."
            );
        }
        Ok(pack)
    }

    /// Import a Workshop export as one publication, checking every artifact before
    /// exposing the pack to agents or the desktop.
    pub fn import(&self, folder: &Path) -> Result<Pack> {
        let folder = dunce::canonicalize(folder)?;
        let pack: Pack = read_json(&plain_file(&folder, Path::new("pack.json"))?)?;
        key(&pack.key)?;
        ensure!(pack.version == 1, "Unsupported pack format.");
        let dest = self.root.join(&pack.key);
        ensure!(
            !dest.try_exists()?,
            "A pack named '{}' already exists in this project.",
            pack.key
        );
        let temp = stage(&self.root)?;
        let incoming = Library {
            root: temp.path().to_owned(),
        };
        let canonical = incoming.create(&pack.key, &pack.name, &pack.style)?;
        ensure!(
            canonical == pack,
            "The pack catalog does not match its manifest."
        );
        let assets = folder.join("assets");
        ensure!(
            assets.is_dir() && dunce::canonicalize(&assets)? == assets,
            "Missing asset directory."
        );
        for id in directories(&assets)? {
            let member_path = Path::new("assets").join(&id);
            let manifest = plain_file(&folder, &member_path.join("asset.json"))?;
            let member: Member = read_json(&manifest)?;
            ensure!(member.key == id, "Mismatched asset key for {id}.");
            label(&member.name, 100)?;
            label(&member.description, 600)?;
            ensure!(
                member.geometry.triangles > 0
                    && member.geometry.meshes > 0
                    && member.geometry.meters_per_unit > 0.0
                    && member.geometry.dimensions.iter().all(|d| *d >= 0.0),
                "Invalid geometry for {id}."
            );
            let to = incoming.member_dir(&pack.key, &id)?;
            fs::create_dir(&to)?;
            write_json(&to.join("asset.json"), &member)?;
            for (file, hash) in [
                ("model.blend", &member.blend_sha256),
                ("model.glb", &member.glb_sha256),
                ("preview.png", &member.preview_sha256),
            ] {
                let source = plain_file(&folder, &member_path.join(file))?;
                let actual =
                    fingerprint(fs::File::open(source)?, fs::File::create(to.join(file))?)?;
                ensure!(&actual == hash, "Checksum mismatch for {id}/{file}.");
            }
            let preview = to.join("preview.png");
            ensure!(
                fs::metadata(&preview)?.len() <= 2 * 1024 * 1024
                    && image::image_dimensions(&preview)? == (480, 480),
                "Invalid asset preview for {id}."
            );
        }
        fs::rename(incoming.root.join(&pack.key), dest)?;
        Ok(pack)
    }

    /// Copy only published library files. New members can be published while exporting.
    pub fn export(&self, pack: &str, parent: &Path) -> Result<PathBuf> {
        let members = self.members(pack)?;
        let source = self.root.join(pack);
        fs::create_dir_all(parent)?;
        let parent = dunce::canonicalize(parent)?;
        ensure!(
            !parent.starts_with(dunce::canonicalize(&self.root)?),
            "Choose an export folder outside the Workshop library."
        );
        let destination = parent.join(pack);
        ensure!(
            !destination.try_exists()?,
            "The export folder {} already exists. Choose another folder.",
            destination.display()
        );
        let temp = stage(&parent)?;
        fs::copy(source.join("pack.json"), temp.path().join("pack.json"))?;
        fs::copy(source.join(CATALOG), temp.path().join(CATALOG))?;
        let assets = temp.path().join("assets");
        fs::create_dir(&assets)?;
        for member in members {
            let from = self.member_dir(pack, &member.key)?;
            let to = assets.join(&member.key);
            fs::create_dir(&to)?;
            for file in MEMBER_FILES {
                fs::copy(from.join(file), to.join(file))?;
            }
        }
        fs::rename(temp.path(), &destination)?;
        Ok(destination)
    }

    pub(super) fn style_reference(
        &self,
        id: &str,
        exemplar: Option<&str>,
    ) -> Result<(String, Option<PathBuf>)> {
        let pack = self.pack(id)?;
        let mut direction = format!(
            "\n\nAsset pack: {} ({id})\nArt direction: {}\nMatch the established palette, scale and silhouette language.",
            pack.name, pack.style
        );
        let mut preview = None;
        let mut members = self.members(id)?;
        if let Some(key) = exemplar {
            let selected = members
                .iter()
                .position(|m| m.key == key)
                .context("The requested visual reference is not in this pack.")?;
            members.swap(0, selected);
        }
        for member in members.into_iter().take(6) {
            let geometry = member.geometry;
            let colors = geometry
                .materials
                .iter()
                .take(12)
                .map(|m| m.color.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let meters = geometry.dimensions.map(|d| d * geometry.meters_per_unit);
            direction.push_str(&format!("\n{}: {}. XYZ dimensions in metres {:?}. Material base colors: {colors}. Rendered swatches: {}.", member.name, member.description, meters, member.palette.join(", ")));
            if preview.is_none() {
                preview = Some(self.member_dir(id, &member.key)?.join("preview.png"));
            }
        }
        Ok((direction, preview))
    }
}

pub struct Publish<'a> {
    pub pack: &'a str,
    pub key: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub source: &'a Path,
    pub collection: Option<&'a str>,
}

pub(crate) fn definition() -> Value {
    let string = json!({"type":"string"});
    json!({"type":"function","name":"workshop_asset_pack",
        "description":"Share reusable Blender collections and a written art direction across this project's chats and workers. list discovers packs; inspect(pack) lists members, inspect(pack,asset) returns the actual preview image, palette and dimensions. create takes pack (stable lowercase key), name and style. add takes pack, asset (stable key), name, description and either source_asset (finished model ID) or path (.blend in this assignment or attached), optionally collection (exact Blender collection). use copies an existing member's editable .blend or game-ready .glb to destination in the working copy. Prefer reuse before creating a matching variant.",
        "inputSchema":{"type":"object","properties":{"action":{"type":"string","enum":["list","inspect","create","add","use"]},"pack":string,"asset":string,"name":string,
            "style":string,"description":string,"source_asset":{"type":"integer","minimum":1},"path":string,"collection":string,"destination":string,
            "offset":{"type":"integer","minimum":0,"description":"Next offset for list or inspect(pack)."}},"required":["action"],"additionalProperties":false}})
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Action {
    List {
        #[serde(default)]
        offset: usize,
    },
    Inspect {
        pack: String,
        asset: Option<String>,
        #[serde(default)]
        offset: usize,
    },
    Create {
        pack: String,
        name: String,
        style: String,
    },
    Add {
        pack: String,
        asset: String,
        name: String,
        description: String,
        source_asset: Option<i64>,
        path: Option<String>,
        collection: Option<String>,
    },
    Use {
        pack: String,
        asset: String,
        destination: String,
    },
}

fn page<T: Serialize>(items: &[T], offset: usize) -> Value {
    let start = offset.min(items.len());
    let end = (start + 20).min(items.len());
    json!({"items":items[start..end],"total":items.len(),"next_offset":(end < items.len()).then_some(end)})
}

pub(crate) async fn dispatch(
    store: &Store,
    attempt: &Attempt,
    args: &Value,
    notify: &dyn Fn(provider::ReplyUpdate<'_>),
) -> Result<ToolOutput> {
    let library = store.asset_packs();
    let result = match serde_json::from_value(args.clone()).context("Invalid asset pack request")? {
        Action::List { offset } => page(&library.list()?, offset),
        Action::Inspect {
            pack,
            asset: None,
            offset,
        } => {
            let members = library
                .members(&pack)?
                .into_iter()
                .map(|m| json!({"key":m.key,"name":m.name,"description":m.description}))
                .collect::<Vec<_>>();
            json!({"pack":library.pack(&pack)?, "assets":page(&members, offset)})
        }
        Action::Inspect {
            pack,
            asset: Some(asset),
            ..
        } => {
            let mut member = library.member(&pack, &asset)?;
            member.geometry.materials.truncate(64);
            let preview = library.member_dir(&pack, &asset)?.join("preview.png");
            ensure!(
                fs::metadata(&preview)?.len() <= 2 * 1024 * 1024,
                "Invalid library preview size."
            );
            return Ok(ToolOutput {
                text: json!({"pack":library.pack(&pack)?, "asset":member}).to_string(),
                image: Some(fs::read(preview)?),
            });
        }
        Action::Create { pack, name, style } => json!(library.create(&pack, &name, &style)?),
        Action::Add {
            pack,
            asset,
            name,
            description,
            source_asset,
            path,
            collection,
        } => {
            let source = match (source_asset, path) {
                (Some(id), None) => store.pack_source_job(id)?,
                (None, Some(path)) => store.asset_input_file(attempt, &path)?,
                _ => bail!("Choose a finished model ID or one .blend path."),
            };
            notify(provider::ReplyUpdate::Activity(
                "Publishing a Blender asset…",
            ));
            let member = library
                .publish(
                    &store.studio_tool("Blender")?,
                    Publish {
                        pack: &pack,
                        key: &asset,
                        name: &name,
                        description: &description,
                        source: &source,
                        collection: collection.as_deref(),
                    },
                )
                .await?;
            json!({"published":member.key, "pack":pack, "collection":member.geometry.collection})
        }
        Action::Use {
            pack,
            asset,
            destination,
        } => {
            library.member(&pack, &asset)?;
            let ext = Path::new(&destination)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            ensure!(
                matches!(ext.as_str(), "blend" | "glb"),
                "Use a .blend or .glb destination."
            );
            let source = library
                .member_dir(&pack, &asset)?
                .join(format!("model.{ext}"));
            copy_to_work(&store.work(attempt.id), &source, &destination)?;
            json!({"pack":pack, "asset":asset, "destination":destination})
        }
    };
    Ok(ToolOutput::text(result.to_string()))
}

impl Library {
    pub async fn publish(&self, blender: &Path, input: Publish<'_>) -> Result<Member> {
        let pack = self.pack(input.pack)?;
        let destination = self.member_dir(input.pack, input.key)?;
        let name = label(input.name, 100)?;
        let description = label(input.description, 600)?;
        let source = dunce::canonicalize(input.source)?;
        ensure!(
            source.is_file()
                && source
                    .extension()
                    .is_some_and(|s| s.eq_ignore_ascii_case("blend")),
            "Choose an editable .blend source."
        );
        let source_sha256 = checksum(&source)?;
        let parent = destination.parent().context("Missing pack directory")?;
        let build = stage(parent)?;
        let request = json!({"type":"pack_asset","source":source,"name":name,"description":description,"collection":input.collection,"catalog_id":pack.catalog_id});
        tokio::time::timeout(
            Duration::from_secs(900),
            tools::run_model(blender, build.path(), &request),
        )
        .await
        .context("Blender exceeded the 15-minute asset build limit.")??;
        let geometry: Geometry = read_json(&build.path().join("geometry.json"))?;
        let member = Member {
            key: input.key.into(),
            name: name.into(),
            description: description.into(),
            source_sha256,
            palette: preview_palette(&build.path().join("preview.png"))?,
            blend_sha256: checksum(&build.path().join("model.blend"))?,
            glb_sha256: checksum(&build.path().join("model.glb"))?,
            preview_sha256: checksum(&build.path().join("preview.png"))?,
            source_collection: input.collection.map(str::to_owned),
            geometry,
        };
        let publish = build.path().join("member");
        fs::create_dir(&publish)?;
        write_json(&publish.join("asset.json"), &member)?;
        for file in &MEMBER_FILES[1..] {
            fs::rename(build.path().join(file), publish.join(file))?;
        }
        if let Err(e) = fs::rename(&publish, &destination) {
            if !destination.exists() {
                return Err(e.into());
            }
            let existing = self.member(input.pack, input.key)?;
            ensure!(
                existing.name == name
                    && existing.description == description
                    && existing.source_sha256 == member.source_sha256
                    && existing.source_collection == member.source_collection
                    && existing.glb_sha256 == member.glb_sha256,
                "This key belongs to a different asset. Give the new variant its own key."
            );
            return Ok(existing);
        }
        Ok(member)
    }
}

impl Store {
    fn pack_source_job(&self, id: i64) -> Result<PathBuf> {
        let db = Connection::open_with_flags(
            self.archive_dir.join("workshop.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let ready: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM studio_jobs WHERE id=?1 AND area='Models' AND state='Ready')", [id], |r| r.get(0))?;
        ensure!(ready, "Choose the ID of a finished model in this project.");
        Ok(self
            .archive_dir
            .join("studio")
            .join(id.to_string())
            .join("model.blend"))
    }
}
