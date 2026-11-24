use super::Recipe;
use crate::{Store, provider};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::OptionalExtension;
use std::{
    env, fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Stdio,
};

fn valid_tool(name: &str, path: &Path) -> bool {
    match name {
        "Blender" => {
            path.is_file()
                && path
                    .file_stem()
                    .is_some_and(|n| n.eq_ignore_ascii_case("blender"))
        }
        "Ghidra" => {
            path.join("Ghidra/Framework/Utility/lib/Utility.jar")
                .is_file()
                && path.join("support/launch.properties").is_file()
        }
        _ => false,
    }
}
impl Store {
    pub fn set_studio_tool(&self, name: &str, path: &Path) -> Result<()> {
        ensure!(
            valid_tool(name, path),
            "Choose {}.",
            if name == "Blender" {
                "the Blender executable"
            } else {
                "the Ghidra installation folder containing support and Ghidra"
            }
        );
        self.db.lock().unwrap().execute(
            "INSERT OR REPLACE INTO preferences.studio_tools(name,path) VALUES(?1,?2)",
            (name, dunce::canonicalize(path)?.to_string_lossy()),
        )?;
        Ok(())
    }
    pub fn studio_tool(&self, name: &str) -> Result<PathBuf> {
        let stored: Option<String> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT path FROM preferences.studio_tools WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(path) = stored.map(PathBuf::from) {
            ensure!(
                valid_tool(name, &path),
                "The saved {name} installation has moved. Choose Locate {name}."
            );
            return Ok(path);
        }
        if let Some(path) = env::var_os(if name == "Blender" {
            "WORKSHOP_BLENDER"
        } else {
            "WORKSHOP_GHIDRA"
        })
        .map(PathBuf::from)
        {
            ensure!(
                valid_tool(name, &path),
                "Check the configured {name} installation or choose Locate {name}."
            );
            return Ok(path);
        }
        let mut candidates = Vec::new();
        if name == "Ghidra" {
            candidates.extend(env::var_os("GHIDRA_INSTALL_DIR").map(PathBuf::from));
        } else {
            if let Some(programs) = env::var_os("ProgramFiles") {
                let root = PathBuf::from(programs).join("Blender Foundation");
                if let Ok(entries) = fs::read_dir(root) {
                    let mut versions = entries
                        .flatten()
                        .map(|p| p.path().join("blender.exe"))
                        .collect::<Vec<_>>();
                    versions.sort_by(|a, b| b.cmp(a));
                    candidates.extend(versions);
                }
            }
            if let Some(steam) = env::var_os("ProgramFiles(x86)") {
                candidates
                    .push(PathBuf::from(steam).join("Steam/steamapps/common/Blender/blender.exe"));
            }
            if let Some(path) = env::var_os("PATH") {
                candidates.extend(
                    env::split_paths(&path)
                        .filter(|p| p.is_absolute())
                        .map(|p| {
                            p.join(if cfg!(windows) {
                                "blender.exe"
                            } else {
                                "blender"
                            })
                        }),
                );
            }
        }
        candidates
            .into_iter()
            .find(|p| valid_tool(name, p))
            .with_context(|| format!("Choose Locate {name} to connect your installation."))
    }
}

async fn run(program: &Path, args: &[&str], cwd: &Path) -> Result<()> {
    let log_path = cwd.join("tool.log");
    let log = fs::File::create(&log_path)?;
    let mut child = provider::managed(
        program.to_str().context("Unsupported executable path")?,
        args,
        cwd,
        Stdio::from(log.try_clone()?),
        log,
    )?;
    drop(child.stdin().take());
    if !child.wait().await?.success() {
        let mut file = fs::File::open(log_path)?;
        file.seek(SeekFrom::Start(file.metadata()?.len().saturating_sub(2048)))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)?;
        bail!(
            "{} failed.\n{}",
            program.file_name().unwrap_or_default().to_string_lossy(),
            String::from_utf8_lossy(&tail).trim()
        );
    }
    Ok(())
}
pub(super) async fn model(store: &Store, cwd: &Path, recipe: &Recipe) -> Result<String> {
    let blender = store.studio_tool("Blender")?;
    if let Recipe::ModelBrief { brief } = recipe {
        provider::design_model(cwd, brief).await?;
    }
    fs::write(cwd.join("model.py"), include_str!("model.py"))?;
    fs::write(cwd.join("request.json"), serde_json::to_vec(recipe)?)?;
    run(
        &blender,
        &[
            "--background",
            "--factory-startup",
            "--disable-autoexec",
            "--threads",
            "2",
            "--python-exit-code",
            "1",
            "--python",
            "model.py",
        ],
        cwd,
    )
    .await?;
    for name in ["model.blend", "model.glb", "preview.png"] {
        ensure!(
            fs::metadata(cwd.join(name)).is_ok_and(|m| m.len() > 0),
            "Blender did not write {name}."
        );
    }
    Ok(fs::read_to_string(cwd.join("summary.txt"))?)
}
pub(super) async fn decompile(
    store: &Store,
    cwd: &Path,
    binary: &Path,
    filter: &str,
) -> Result<String> {
    let ghidra = store.studio_tool("Ghidra")?;
    let jar = ghidra.join("Ghidra/Framework/Utility/lib/Utility.jar");
    let java = env::var_os("JAVA_HOME")
        .map(|p| {
            PathBuf::from(p).join(if cfg!(windows) {
                "bin/java.exe"
            } else {
                "bin/java"
            })
        })
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("java"));
    // Ghidra project URLs reject hidden path components, including tempfile's default prefix.
    let scratch = tempfile::Builder::new()
        .prefix("workshop-ghidra-")
        .tempdir()?;
    let script = scratch.path();
    fs::write(
        script.join("WorkshopGameFunctions.java"),
        include_str!("WorkshopGameFunctions.java"),
    )?;
    fs::write(script.join("filter.txt"), filter.trim())?;
    let binary = dunce::canonicalize(binary)?;
    let props = fs::read_to_string(ghidra.join("support/launch.properties"))?;
    let platform = if cfg!(windows) {
        "VMARGS_WINDOWS="
    } else if cfg!(target_os = "macos") {
        "VMARGS_MACOS="
    } else {
        "VMARGS_LINUX="
    };
    let mut args: Vec<&str> = props
        .lines()
        .map(str::trim)
        .filter_map(|line| {
            line.strip_prefix("VMARGS=")
                .or_else(|| line.strip_prefix(platform))
        })
        .filter(|s| !s.is_empty())
        .collect();
    args.extend([
        "-Xmx2G",
        "-Djava.awt.headless=true",
        "-cp",
        jar.to_str().context("Invalid Ghidra path")?,
        "ghidra.Ghidra",
        "ghidra.app.util.headless.AnalyzeHeadless",
        script.to_str().context("Invalid job path")?,
        "Workshop",
        "-import",
        binary.to_str().context("Invalid binary path")?,
        "-max-cpu",
        "2",
        "-analysisTimeoutPerFile",
        "180",
        "-scriptPath",
        script.to_str().unwrap(),
        "-postScript",
        "WorkshopGameFunctions.java",
        "-deleteProject",
    ]);
    run(&java, &args, cwd).await?;
    let summary = fs::read_to_string(script.join("summary.txt")).context(
        "Ghidra produced no report. Check the binary format, Java version and tool.log.",
    )?;
    ensure!(script.join("functions.c").is_file(), "{summary}");
    fs::copy(script.join("functions.c"), cwd.join("functions.c"))?;
    Ok(summary)
}
