use crate::{Desk, Transcript};
use anyhow::{Context, Result, bail, ensure};
use percent_encoding::percent_decode_str;
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
};
use url::Url;

enum Destination {
    File(PathBuf),
    Web(Url),
}

fn destination(directory: &str, target: &str) -> Result<Destination> {
    let target = target.trim();
    ensure!(
        !target.is_empty() && !target.chars().any(char::is_control),
        "Invalid link"
    );
    let path = if !Path::new(target).is_absolute()
        && let Ok(url) = Url::parse(target)
    {
        match url.scheme() {
            "https" | "http" => return Ok(Destination::Web(url)),
            "file" if url.host_str().is_none() => {
                url.to_file_path().ok().context("Invalid file link")?
            }
            _ => bail!("Unsupported link type"),
        }
    } else {
        let decoded = percent_decode_str(target).decode_utf8()?;
        Path::new(directory).join(decoded.as_ref())
    };
    ensure!(
        path.is_absolute()
            && !path.to_string_lossy().starts_with(r"\\")
            && !path.to_string_lossy().starts_with("//"),
        "Choose a local file"
    );
    Ok(Destination::File(path))
}

pub(super) fn attachment(target: &str) -> Option<(String, String)> {
    let path = target.split(['?', '#']).next()?;
    let name = percent_decode_str(path.rsplit(['/', '\\']).next()?)
        .decode_utf8()
        .ok()?;
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    let kind = match ext.as_str() {
        "zip" | "7z" | "tar" | "gz" => "Archive",
        "rbxl" | "rbxlx" => "Roblox place",
        "blend" | "fbx" | "glb" | "gltf" => "3D asset",
        "pdf" | "docx" => "Document",
        "png" | "jpg" | "jpeg" | "webp" => "Image",
        "wav" | "ogg" | "mp3" => "Audio",
        "mp4" | "webm" => "Video",
        _ => return None,
    };
    Some((name.into_owned(), kind.to_owned()))
}

fn command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd
}

fn open(directory: &str, target: &str) -> Result<()> {
    match destination(directory, target)? {
        Destination::Web(url) => webbrowser::open(url.as_str())?,
        Destination::File(path) => {
            let path = dunce::canonicalize(&path)
                .with_context(|| format!("File unavailable: {}", path.display()))?;
            let ext = path
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let viewable = matches!(
                ext.as_str(),
                "zip"
                    | "pdf"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "webp"
                    | "wav"
                    | "mp3"
                    | "ogg"
                    | "mp4"
                    | "webm"
                    | "txt"
                    | "md"
            );
            #[cfg(windows)]
            {
                let mut cmd = command("explorer.exe");
                if path.is_dir() || viewable {
                    cmd.arg(path);
                } else {
                    cmd.arg(format!("/select,{}", path.display()));
                }
                cmd.spawn()?;
            }
            #[cfg(not(windows))]
            {
                let show = if viewable || path.is_dir() {
                    path.as_path()
                } else {
                    path.parent().context("Missing parent folder")?
                };
                command(if cfg!(target_os = "macos") {
                    "open"
                } else {
                    "xdg-open"
                })
                .arg(show)
                .spawn()?;
            }
        }
    }
    Ok(())
}

fn thumbnail(source: Destination) -> Result<image::RgbaImage> {
    let tmp;
    let path = match source {
        Destination::File(path) => path,
        Destination::Web(url) => {
            tmp = tempfile::NamedTempFile::new()?;
            let status = command(if cfg!(windows) { "curl.exe" } else { "curl" })
                .args([
                    "--disable",
                    "--fail",
                    "--silent",
                    "--location",
                    "--max-redirs",
                    "3",
                    "--proto",
                    "=http,https",
                    "--proto-redir",
                    "=http,https",
                    "--max-time",
                    "15",
                    "--max-filesize",
                    "12582912",
                    "--output",
                ])
                .arg(tmp.path())
                .arg("--")
                .arg(url.as_str())
                .status()?;
            ensure!(status.success(), "Image download failed");
            tmp.path().to_owned()
        }
    };
    ensure!(
        path.is_file() && path.metadata()?.len() <= 32 * 1024 * 1024,
        "Image is too large to preview"
    );
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    reader.limits(limits);
    Ok(reader.decode()?.thumbnail(720, 360).into_rgba8())
}

pub(super) fn connect(ui: &Desk) {
    let weak = ui.as_weak();
    ui.global::<Transcript>().on_open(move |dir, target| {
        if let Err(e) = open(&dir, &target)
            && let Some(ui) = weak.upgrade()
        {
            ui.set_notice(format!("{e:#}").into());
        }
    });
    // The cache belongs to the UI thread. Only encoded files and pixel bytes cross threads.
    let cache = Rc::new(RefCell::new(HashMap::<(String, String), Image>::new()));
    let previews = cache.clone();
    ui.global::<Transcript>().on_preview(move |dir, target, _| {
        previews
            .borrow()
            .get(&(dir.to_string(), target.to_string()))
            .cloned()
            .unwrap_or_default()
    });
    let (send, receive) = std::sync::mpsc::channel::<(String, String, Destination)>();
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        for (dir, target, source) in receive {
            let result = thumbnail(source);
            if weak
                .upgrade_in_event_loop(move |ui| {
                    let panel = ui.global::<Transcript>();
                    if panel.get_directory().as_str() != dir {
                        return;
                    }
                    let preview = result
                        .ok()
                        .map(|pixels| {
                            Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                                pixels.as_raw(),
                                pixels.width(),
                                pixels.height(),
                            ))
                        })
                        .unwrap_or_default();
                    panel.invoke_received(dir.into(), target.into(), preview);
                })
                .is_err()
            {
                break;
            }
        }
    });
    let clear = cache.clone();
    ui.global::<Transcript>()
        .on_clear_previews(move || clear.borrow_mut().clear());
    let pending = cache.clone();
    ui.global::<Transcript>()
        .on_load_preview(move |dir, target| {
            let key = (dir.to_string(), target.to_string());
            if pending.borrow().contains_key(&key) {
                return;
            }
            if let Ok(source) = destination(&dir, &target) {
                pending.borrow_mut().insert(key.clone(), Image::default());
                let _ = send.send((key.0, key.1, source));
            }
        });
    let weak = ui.as_weak();
    ui.global::<Transcript>()
        .on_received(move |dir, target, image| {
            if let Some(ui) = weak.upgrade() {
                let panel = ui.global::<Transcript>();
                cache
                    .borrow_mut()
                    .insert((dir.to_string(), target.to_string()), image);
                panel.set_revision(panel.get_revision().wrapping_add(1));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_document_links_in_the_sending_chat() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let file = dir.join("The project.zip");
        for link in [
            "The%20project.zip".to_owned(),
            Url::from_file_path(&file).unwrap().into(),
            file.to_string_lossy().into_owned(),
        ] {
            assert!(
                matches!(destination(dir.to_str().unwrap(), &link), Ok(Destination::File(path)) if path == file)
            );
        }
        assert!(matches!(
            destination("", "https://example.com/picture.png"),
            Ok(Destination::Web(_))
        ));
        assert!(destination("", "javascript:alert(1)").is_err());
        assert!(destination("", "file://server/share/file.zip").is_err());
    }
}
