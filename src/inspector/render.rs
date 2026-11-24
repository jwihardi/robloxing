use super::mesh::Mesh;
use glam::{Mat3, Vec2, Vec3};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy)]
pub struct View {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub mode: i32,
    pub width: u32,
    pub height: u32,
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

fn bounds(p: [Vec3; 3], w: u32, h: u32) -> (usize, usize, usize, usize) {
    let lo = p[0].min(p[1]).min(p[2]);
    let hi = p[0].max(p[1]).max(p[2]);
    (
        lo.x.max(0.) as usize,
        hi.x.ceil().clamp(0., w as f32) as usize,
        lo.y.max(0.) as usize,
        hi.y.ceil().clamp(0., h as f32) as usize,
    )
}

pub fn draw(mesh: &Mesh, view: View, cancelled: &AtomicBool) -> Option<Frame> {
    let scale = (1000. / view.width.max(1) as f32)
        .min(720. / view.height.max(1) as f32)
        .min(1.);
    let mut w = ((view.width as f32 * scale).round() as u32).max(1);
    let mut h = ((view.height as f32 * scale).round() as u32).max(1);
    let rotation = Mat3::from_rotation_x(view.pitch.to_radians())
        * Mat3::from_rotation_y(view.yaw.to_radians());
    let camera: Vec<_> = mesh.vertices.iter().map(|&v| rotation * v).collect();
    let project = |w: u32, h: u32| {
        let size = w.min(h) as f32 * 0.44 * view.zoom;
        camera
            .iter()
            .map(|p| {
                Vec3::new(
                    w as f32 * 0.5 + p.x * size,
                    h as f32 * 0.5 - p.y * size,
                    p.z,
                )
            })
            .collect::<Vec<_>>()
    };
    let mut projected = project(w, h);
    // Bound raster work on dense, highly overdrawn scenes independently of face count.
    let coverage: u64 = mesh
        .faces
        .iter()
        .map(|f| {
            let (x, r, y, b) = bounds(f.indices.map(|i| projected[i]), w, h);
            r.saturating_sub(x) as u64 * b.saturating_sub(y) as u64
        })
        .sum();
    if coverage > 8_000_000 {
        let ratio = (8_000_000. / coverage as f64).sqrt();
        w = ((w as f64 * ratio).round() as u32).max(1);
        h = ((h as f64 * ratio).round() as u32).max(1);
        projected = project(w, h);
    }
    let mut frame = Frame {
        width: w,
        height: h,
        rgb: vec![0; w as usize * h as usize * 3],
    };
    for (y, row) in frame.rgb.chunks_exact_mut(w as usize * 3).enumerate() {
        let tone = 33 - (6 * y / h as usize) as u8;
        row.as_chunks_mut::<3>().0.fill([tone, tone + 1, tone + 2]);
    }
    let mut depth = vec![f32::NEG_INFINITY; w as usize * h as usize];
    let light = Vec3::new(-0.45, 0.75, 0.55).normalize();
    for (idx, face) in mesh.faces.iter().enumerate() {
        if idx % 64 == 0 && cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let [a, b, c] = face.indices.map(|i| projected[i]);
        let ab = (b - a).truncate();
        let ac = (c - a).truncate();
        let area = ab.perp_dot(ac);
        if area.abs() < 0.0001 {
            continue;
        }
        let [u, v, t] = face.indices.map(|i| camera[i]);
        let mut normal = (v - u).cross(t - u).normalize_or_zero();
        if normal.z < 0. {
            normal = -normal;
        }
        let highlight = view.mode == 2 && mesh.overlaps[idx];
        let base = if highlight {
            [255, 150, 77]
        } else if view.mode != 0 {
            [126, 137, 145]
        } else {
            face.color
        };
        let shade = 0.49 + 0.49 * light.dot(normal).max(0.);
        let fill = base.map(|k| (k as f32 * shade) as u8);
        let line = if highlight {
            [255, 213, 160]
        } else {
            [208, 218, 222]
        };
        let edge = if view.mode == 1 || highlight {
            0.78
        } else {
            0.
        };
        let thresholds =
            Vec3::new(ab.length(), ac.length(), (ac - ab).length()) * (edge / area.abs());
        let (left, right, top, bottom) = bounds([a, b, c], w, h);
        let step_s = ac.y / area;
        let step_t = -ab.y / area;
        for y in top..bottom {
            let offset = Vec2::new(left as f32 + 0.5 - a.x, y as f32 + 0.5 - a.y);
            let mut s = offset.perp_dot(ac) / area;
            let mut t = ab.perp_dot(offset) / area;
            for x in left..right {
                let rem = 1. - s - t;
                if s >= 0. && t >= 0. && rem >= 0. {
                    // Highlighted planes remain visible through surrounding geometry.
                    let z = a.z * rem + b.z * s + c.z * t + if highlight { 4. } else { 0. };
                    let index = y * w as usize + x;
                    if z > depth[index] {
                        depth[index] = z;
                        let color = if edge > 0.
                            && (t < thresholds.x || s < thresholds.y || rem < thresholds.z)
                        {
                            line
                        } else {
                            fill
                        };
                        frame.rgb[index * 3..index * 3 + 3].copy_from_slice(&color);
                    }
                }
                s += step_s;
                t += step_t;
            }
        }
    }
    Some(frame)
}
