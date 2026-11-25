use crate::{App, Comparison, Desk, Sketch, capture};
use anyhow::{Context, Result, ensure};
use image::{ColorType, ImageEncoder, Pixel, Rgba, RgbaImage, codecs::png::PngEncoder, imageops};
use slint::{Color, ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};
use std::{path::Path, rc::Rc, time::Duration};

pub(crate) fn set_turn_image(turn: &mut crate::ChatTurn, path: Option<std::path::PathBuf>) {
    if let Some(path) = path
        && let Ok(image) = Image::load_from_path(&path)
    {
        turn.sketch = image;
        turn.image_path = path.to_string_lossy().as_ref().into();
    }
}

const WIDTH: u32 = 960;
const HEIGHT: u32 = 480;
const PAPER: Rgba<u8> = Rgba([250, 249, 246, 255]);
type Point = (f32, f32);

struct Stroke {
    color: [u8; 3],
    points: Vec<Point>,
}

pub(crate) struct Draft {
    open: bool,
    drawing: bool,
    strokes: Vec<Stroke>,
    original: Option<RgbaImage>,
    paper: RgbaImage,
    pixels: RgbaImage,
    replacement: Option<Box<Draft>>,
}

impl Default for Draft {
    fn default() -> Self {
        Self::new(RgbaImage::from_pixel(WIDTH, HEIGHT, PAPER), None)
    }
}

// Normalized points let the preview and original-resolution export replay identical strokes.
fn ink(pixels: &mut RgbaImage, a: Point, b: Point, color: [u8; 3]) {
    let (w, h) = pixels.dimensions();
    let scale = |(x, y): Point| {
        (
            x.clamp(0., 1.) * (w - 1) as f32,
            y.clamp(0., 1.) * (h - 1) as f32,
        )
    };
    let (ax, ay) = scale(a);
    let (bx, by) = scale(b);
    let (dx, dy) = (bx - ax, by - ay);
    let length_sq = (dx * dx + dy * dy).max(f32::EPSILON);
    let radius = (w.max(h) as f32 / WIDTH as f32 * 2.3).max(0.7) + 0.5;
    for y in (ay.min(by) - radius).max(0.) as u32..=(ay.max(by) + radius).min((h - 1) as f32) as u32
    {
        // Bound each scan line to the segment's footprint, even for long diagonal pen moves.
        let (x1, x2) = if dy.abs() > f32::EPSILON {
            let t1 = ((y as f32 - ay - radius) / dy).clamp(0., 1.);
            let t2 = ((y as f32 - ay + radius) / dy).clamp(0., 1.);
            (ax + t1 * dx, ax + t2 * dx)
        } else {
            (ax, bx)
        };
        for x in
            (x1.min(x2) - radius).max(0.) as u32..=(x1.max(x2) + radius).min((w - 1) as f32) as u32
        {
            let (px, py) = (x as f32 - ax, y as f32 - ay);
            let t = ((px * dx + py * dy) / length_sq).clamp(0., 1.);
            let alpha = (radius - (px - t * dx).hypot(py - t * dy)).clamp(0., 1.);
            if alpha > 0. {
                pixels.get_pixel_mut(x, y).blend(&Rgba([
                    color[0],
                    color[1],
                    color[2],
                    (alpha * 255.).round() as u8,
                ]));
            }
        }
    }
}

impl Draft {
    fn new(paper: RgbaImage, original: Option<RgbaImage>) -> Self {
        Self {
            pixels: paper.clone(),
            paper,
            original,
            open: true,
            drawing: false,
            strokes: Vec::new(),
            replacement: None,
        }
    }
    fn from_image(path: &Path) -> Result<Self> {
        let original = image::open(path)
            .context("Could not open the image for annotation.")?
            .into_rgba8();
        let (w, h) = original.dimensions();
        ensure!(w > 0 && h > 0, "The image is empty.");
        let scale = WIDTH as f64 / w.max(h) as f64;
        let paper = imageops::resize(
            &original,
            ((w as f64 * scale).round() as u32).max(1),
            ((h as f64 * scale).round() as u32).max(1),
            imageops::FilterType::Triangle,
        );
        Ok(Self::new(paper, Some(original)))
    }
    fn begin(&mut self, p: Point, color: Color) {
        let color = [color.red(), color.green(), color.blue()];
        self.strokes.push(Stroke {
            color,
            points: vec![p],
        });
        ink(&mut self.pixels, p, p, color);
        self.drawing = true;
    }
    fn extend(&mut self, p: Point) {
        if self.drawing
            && let Some(stroke) = self.strokes.last_mut()
            && let Some(&last) = stroke.points.last()
            && last != p
        {
            ink(&mut self.pixels, last, p, stroke.color);
            stroke.points.push(p);
        }
    }
    fn replay(strokes: &[Stroke], pixels: &mut RgbaImage) {
        for stroke in strokes {
            let start = stroke.points[0];
            ink(pixels, start, start, stroke.color);
            for pair in stroke.points.windows(2) {
                ink(pixels, pair[0], pair[1], stroke.color);
            }
        }
    }
    fn undo(&mut self) {
        self.drawing = false;
        self.strokes.pop();
        self.pixels.clone_from(&self.paper);
        Self::replay(&self.strokes, &mut self.pixels);
    }
    pub(crate) fn has_content(&self) -> bool {
        self.original.is_some() || !self.strokes.is_empty()
    }
    fn encode(pixels: &RgbaImage) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes).write_image(
            pixels.as_raw(),
            pixels.width(),
            pixels.height(),
            ColorType::Rgba8.into(),
        )?;
        Ok(bytes)
    }
    pub(crate) fn save(&self, store: &workshop::Store) -> Result<std::path::PathBuf> {
        if let Some(original) = &self.original {
            let clean = Self::encode(original)?;
            if self.strokes.is_empty() {
                return store.save_chat_image(&clean);
            }
            return store.save_annotation(&clean, &self.png()?);
        }
        store.save_chat_image(&self.png()?)
    }
    pub(crate) fn png(&self) -> Result<Vec<u8>> {
        let mut full;
        let pixels = if let Some(original) = &self.original {
            full = original.clone();
            Self::replay(&self.strokes, &mut full);
            &full
        } else {
            &self.pixels
        };
        Self::encode(pixels)
    }
}

impl App {
    pub(crate) fn annotate(&self, path: &Path) -> Result<()> {
        let draft = Draft::from_image(path)?;
        let ui = self.ui.unwrap();
        ui.invoke_open_asset_draft();
        let key = self.id.get();
        let mut drafts = self.sketches.borrow_mut();
        if let Some(current) = drafts.get_mut(&key)
            && current.has_content()
        {
            current.replacement = Some(Box::new(draft));
            current.open = true;
        } else {
            drafts.insert(key, draft);
        }
        drop(drafts);
        self.render_sketch();
        ui.invoke_focus_draft();
        Ok(())
    }
    pub(crate) fn render_sketch(&self) {
        let ui = self.ui.unwrap();
        let panel = ui.global::<Sketch>();
        let drafts = self.sketches.borrow();
        let draft = drafts.get(&self.id.get());
        panel.set_expanded(draft.is_some_and(|d| d.open));
        panel.set_has_ink(draft.is_some_and(|d| !d.strokes.is_empty()));
        panel.set_replacing(draft.is_some_and(|d| d.replacement.is_some()));
        panel.set_on_image(draft.is_some_and(|d| d.original.is_some()));
        panel.set_preview(
            draft
                .map(|d| {
                    let p = &d.pixels;
                    Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                        p.as_raw(),
                        p.width(),
                        p.height(),
                    ))
                })
                .unwrap_or_default(),
        );
    }
}

pub(crate) fn connect(ui: &Desk, app: &Rc<App>) {
    let a = app.clone();
    ui.global::<Comparison>()
        .on_open(move |source, markup, revision, previous| {
            let ui = a.ui.unwrap();
            let panel = ui.global::<Comparison>();
            let load = |path: &slint::SharedString| {
                Image::load_from_path(Path::new(path.as_str()))
                    .map_err(|_| anyhow::anyhow!("Could not open the comparison image at {path}."))
            };
            let result = (|| -> Result<()> {
                let original = load(&source)?;
                let revised = load(&revision)?;
                let marked = if markup.is_empty() {
                    Image::default()
                } else {
                    load(&markup)?
                };
                panel.set_original_path(source);
                panel.set_revision_path(revision);
                panel.set_previous(previous);
                panel.set_original(original);
                panel.set_marked(marked);
                panel.set_revision(revised);
                Ok(())
            })();
            if let Err(e) = result {
                ui.set_notice(e.to_string().into());
            }
        });
    let a = app.clone();
    ui.global::<Comparison>().on_close(move || {
        let ui = a.ui.unwrap();
        let panel = ui.global::<Comparison>();
        panel.set_revision(Default::default());
        panel.set_marked(Default::default());
        panel.set_original(Default::default());
        ui.invoke_focus_draft();
    });
    let panel = ui.global::<Sketch>();
    let a = app.clone();
    panel.on_replace(move |replace| {
        if let Some(draft) = a.sketches.borrow_mut().get_mut(&a.id.get())
            && let Some(next) = draft.replacement.take()
            && replace
        {
            *draft = *next;
        }
        a.render_sketch();
        a.ui.unwrap().invoke_focus_draft();
    });
    let a = app.clone();
    panel.on_annotate(move |source, screenshot| {
        let path = if screenshot {
            capture::image_path(&source, true)
        } else {
            Ok(std::path::PathBuf::from(source.as_str()))
        };
        if let Err(e) = path.and_then(|path| a.annotate(&path)) {
            a.ui.unwrap().set_notice(format!("{e:#}").into());
        }
    });
    let a = app.clone();
    panel.on_edit(move || {
        a.sketches.borrow_mut().entry(a.id.get()).or_default().open = true;
        a.render_sketch();
    });
    let a = app.clone();
    panel.on_close(move |remove| {
        let mut drafts = a.sketches.borrow_mut();
        let key = a.id.get();
        if let Some(draft) = drafts.get_mut(&key) {
            draft.drawing = false;
            draft.open = false;
            if remove || !draft.has_content() {
                drafts.remove(&key);
            }
        }
        drop(drafts);
        a.render_sketch();
        a.ui.unwrap().invoke_focus_draft();
    });
    let a = app.clone();
    panel.on_begin(move |x, y, color| {
        if let Some(draft) = a.sketches.borrow_mut().get_mut(&a.id.get()) {
            draft.begin((x, y), color);
        }
        a.render_sketch();
    });
    let a = app.clone();
    let pending = Rc::new(std::cell::Cell::new(false));
    panel.on_extend(move |x, y| {
        if let Some(draft) = a.sketches.borrow_mut().get_mut(&a.id.get()) {
            draft.extend((x, y));
        }
        if !pending.replace(true) {
            let a = a.clone();
            let pending = pending.clone();
            slint::Timer::single_shot(Duration::from_millis(16), move || {
                pending.set(false);
                a.render_sketch();
            });
        }
    });
    let a = app.clone();
    panel.on_finish(move || {
        if let Some(draft) = a.sketches.borrow_mut().get_mut(&a.id.get()) {
            draft.drawing = false;
        }
    });
    let a = app.clone();
    panel.on_undo(move || {
        if let Some(draft) = a.sketches.borrow_mut().get_mut(&a.id.get()) {
            draft.undo();
        }
        a.render_sketch();
    });
}
