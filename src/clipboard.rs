use anyhow::{Context, Result};
use arboard::{Clipboard, Error};
use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use std::path::PathBuf;
use workshop::Store;

/// None leaves ordinary text paste, selection and undo to the native editor.
pub fn attachments(store: Option<&Store>) -> Result<Option<Vec<PathBuf>>> {
    let mut clipboard = Clipboard::new().context("Open clipboard")?;
    match clipboard.get().file_list() {
        Ok(files) if !files.is_empty() => return Ok(Some(files)),
        Ok(_) | Err(Error::ContentNotAvailable) => {}
        Err(e) => return Err(e.into()),
    }
    let image = match clipboard.get_image() {
        Ok(image) => image,
        Err(Error::ContentNotAvailable) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let store = store.context("Open a project to attach an image")?;
    let mut png = Vec::new();
    PngEncoder::new(&mut png).write_image(
        &image.bytes,
        image.width.try_into()?,
        image.height.try_into()?,
        ExtendedColorType::Rgba8,
    )?;
    Ok(Some(vec![store.save_chat_image(&png)?]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_shortcut_consumes_attachments_and_falls_back_to_text() -> Result<()> {
        use slint::{
            ComponentHandle,
            platform::{
                self, Key, Platform, WindowAdapter, WindowEvent,
                software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
            },
        };
        use std::{cell::Cell, rc::Rc};
        struct TestPlatform;
        impl Platform for TestPlatform {
            fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
                Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
            }
            fn clipboard_text(&self, _: platform::Clipboard) -> Option<String> {
                Some("pasted text".into())
            }
        }
        platform::set_platform(Box::new(TestPlatform)).unwrap();
        let ui = crate::Desk::new()?;
        ui.set_project("test project".into());
        ui.show()?;
        ui.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        ui.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(1000., 700.),
        });
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
        ui.invoke_focus_draft();
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: "x".into() });
        assert_eq!(ui.get_draft(), "x", "composer has keyboard focus");
        ui.set_draft("".into());
        let calls = Rc::new(Cell::new(0));
        let count = calls.clone();
        ui.on_paste_attachments(move || {
            count.set(count.get() + 1);
            count.get() == 1
        });
        let paste = || {
            for text in [Key::Control.into(), "v".into()] {
                ui.window().dispatch_event(WindowEvent::KeyPressed { text });
            }
            for text in ["v".into(), Key::Control.into()] {
                ui.window()
                    .dispatch_event(WindowEvent::KeyReleased { text });
            }
        };
        paste();
        assert_eq!(calls.get(), 1);
        assert!(ui.get_draft().is_empty());
        paste();
        assert_eq!(ui.get_draft(), "pasted text");
        Ok(())
    }

    #[test]
    #[ignore = "run under xvfb-run with WORKSHOP_TEST_PRIVATE_CLIPBOARD=1"]
    fn native_clipboard_preserves_pixels_paths_and_text_fallback() -> Result<()> {
        anyhow::ensure!(std::env::var("WORKSHOP_TEST_PRIVATE_CLIPBOARD").as_deref() == Ok("1"));
        let dir = tempfile::tempdir()?;
        let project = dir.path().join("project");
        let store = Store::open(&project, &dir.path().join("data"))?;
        let mut owner = Clipboard::new()?;
        let pixels = vec![23, 0, 255, 255, 71, 50, 1, 128];
        owner.set_image(arboard::ImageData {
            width: 2,
            height: 1,
            bytes: pixels.clone().into(),
        })?;
        let images = attachments(Some(&store))?.expect("image attachment");
        assert_eq!(images.len(), 1);
        assert_eq!(image::open(&images[0])?.to_rgba8().as_raw(), &pixels);
        let file = project.join("art notes.txt");
        std::fs::write(&file, "test file")?;
        let paths = vec![file, project];
        owner.set().file_list(&paths)?;
        assert_eq!(attachments(Some(&store))?, Some(paths));
        owner.set_text("insert at cursor")?;
        assert!(attachments(Some(&store))?.is_none());
        Ok(())
    }
}
