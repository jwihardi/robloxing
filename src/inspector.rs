mod mesh;
mod overlap;
mod render;
#[cfg(test)]
mod tests;

use super::{Desk, Studio};
use render::View;
use slint::{ComponentHandle, Image, Rgb8Pixel, SharedPixelBuffer};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

pub struct Inspector {
    cancelled: Arc<AtomicBool>,
    views: mpsc::Sender<View>,
}

impl Drop for Inspector {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl Inspector {
    pub fn open(ui: &Desk, path: PathBuf) -> Self {
        let (views, requests) = mpsc::channel::<View>();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let update = |f: Box<dyn FnOnce(&Studio) + Send>| {
                let stop = stop.clone();
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    if !stop.load(Ordering::Acquire) {
                        f(&ui.global::<Studio>());
                    }
                });
            };
            let mut mesh = match mesh::load(&path, &stop) {
                Ok(m) => m,
                Err(e) => {
                    update(Box::new(move |studio| {
                        studio.set_inspect_error(format!("{e:#}").into())
                    }));
                    return;
                }
            };
            let total = mesh.faces.len();
            update(Box::new(move |studio| {
                studio.set_inspect_faces(format!("{total} triangles").into());
                studio.set_inspect_result("Checking coplanar faces…".into());
            }));
            while let Ok(mut view) = requests.recv() {
                // Use the newest pointer/resize event when the previous frame took longer to draw.
                for newest in requests.try_iter() {
                    view = newest;
                }
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if view.mode == 2 && mesh.overlaps.is_empty() {
                    let (flags, complete) = overlap::find(&mesh.vertices, &mesh.faces, &stop);
                    let count = flags.iter().filter(|&&hit| hit).count();
                    mesh.overlaps = flags;
                    let diagnostic = match (complete, count) {
                        (true, 0) => "No overlapping faces found".into(),
                        (true, n) => format!("{n} overlapping faces · shown in orange"),
                        (false, 0) => "Partial scan · no overlaps found so far".into(),
                        (false, n) => format!("Partial scan · at least {n} overlapping faces"),
                    };
                    update(Box::new(move |studio| {
                        studio.set_inspect_result(diagnostic.into())
                    }));
                }
                let Some(frame) = render::draw(&mesh, view, &stop) else {
                    break;
                };
                update(Box::new(move |studio| {
                    let pixels = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
                        &frame.rgb,
                        frame.width,
                        frame.height,
                    );
                    studio.set_inspect_image(Image::from_rgb8(pixels));
                    studio.set_inspect_ready(true);
                }));
            }
        });
        Self { cancelled, views }
    }

    pub fn request(&self, yaw: f32, pitch: f32, zoom: f32, mode: i32, width: i32, height: i32) {
        if width > 0 && height > 0 {
            let _ = self.views.send(View {
                yaw,
                pitch,
                zoom,
                mode,
                width: width as u32,
                height: height as u32,
            });
        }
    }
}
