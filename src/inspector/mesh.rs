use anyhow::{Context, Result, bail, ensure};
use glam::{DMat4, DVec3, Mat4, Vec3};
use gltf::{buffer::Source, mesh::Mode};
use std::{
    fs::File,
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_FACES: usize = 150_000;
const MAX_VERTICES: usize = MAX_FACES * 3;

pub struct Face {
    pub indices: [usize; 3],
    pub color: [u8; 3],
}

pub struct Mesh {
    pub vertices: Vec<Vec3>,
    pub faces: Vec<Face>,
    pub overlaps: Vec<bool>,
}

pub fn load(path: &Path, cancelled: &AtomicBool) -> Result<Mesh> {
    let file = File::open(path).context("Could not open the model export.")?;
    ensure!(
        file.metadata()?.len() <= MAX_BYTES as u64,
        "This export exceeds the 64 MiB inspector limit. Open the scene in Blender."
    );
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_BYTES,
        "Model grew during loading. Reopen the inspector."
    );
    decode(&bytes, cancelled)
}

pub(super) fn decode(bytes: &[u8], cancelled: &AtomicBool) -> Result<Mesh> {
    let glb = gltf::Gltf::from_slice(bytes).context("Could not read the exported GLB.")?;
    let blob = glb
        .blob
        .as_deref()
        .context("The export has no embedded geometry.")?;
    let scene = glb
        .default_scene()
        .or_else(|| glb.scenes().next())
        .context("The export has no scene.")?;
    let mut nodes: Vec<_> = scene.nodes().map(|node| (node, DMat4::IDENTITY)).collect();
    let mut visited = std::collections::HashSet::new();
    let mut world = Vec::new();
    let mut faces = Vec::new();
    while let Some((node, parent)) = nodes.pop() {
        ensure!(!cancelled.load(Ordering::Relaxed), "Closed");
        ensure!(visited.insert(node.index()), "Invalid scene hierarchy.");
        let matrix = parent * Mat4::from_cols_array_2d(&node.transform().matrix()).as_dmat4();
        nodes.extend(node.children().map(|child| (child, matrix)));
        let Some(mesh) = node.mesh() else { continue };
        ensure!(
            node.skin().is_none(),
            "Skinned models can be inspected in Blender."
        );
        ensure!(
            node.weights()
                .or_else(|| mesh.weights())
                .unwrap_or_default()
                .iter()
                .all(|&w| w == 0.),
            "This scene uses active shape keys. Open it in Blender to inspect the deformed mesh."
        );
        for prim in mesh.primitives() {
            if matches!(
                prim.mode(),
                Mode::Points | Mode::Lines | Mode::LineLoop | Mode::LineStrip
            ) {
                continue;
            }
            let reader = prim.reader(|b| matches!(b.source(), Source::Bin).then_some(blob));
            let positions = reader
                .read_positions()
                .context("Missing model positions.")?;
            let base = world.len();
            let count = positions.len();
            ensure!(
                count <= MAX_VERTICES - base,
                "This mesh exceeds the inspector vertex limit. Open the scene in Blender."
            );
            world
                .extend(positions.map(|p| matrix.transform_point3(Vec3::from_array(p).as_dvec3())));
            let mat = prim.material().pbr_metallic_roughness().base_color_factor();
            let color = [mat[0], mat[1], mat[2]]
                .map(|c| (c.clamp(0., 1.).powf(1. / 2.2) * 255.).round() as u8);
            let indices: Box<dyn ExactSizeIterator<Item = usize>> = if prim.indices().is_some() {
                Box::new(
                    reader
                        .read_indices()
                        .context("Missing model indices.")?
                        .into_u32()
                        .map(|i| i as usize),
                )
            } else {
                Box::new(0..count)
            };
            let triangles = match prim.mode() {
                Mode::Triangles => {
                    ensure!(indices.len().is_multiple_of(3), "Incomplete triangle.");
                    indices.len() / 3
                }
                Mode::TriangleStrip | Mode::TriangleFan => indices.len().saturating_sub(2),
                _ => unreachable!(),
            };
            ensure!(
                triangles <= MAX_FACES - faces.len(),
                "This mesh exceeds the 150,000 triangle inspector limit. Open the scene in Blender."
            );
            let (mut first, mut a, mut b) = (0, 0, 0);
            for (n, i) in indices.enumerate() {
                ensure!(i < count, "Face references an invalid vertex.");
                let c = base + i;
                if n == 0 {
                    first = c;
                }
                let tri = match prim.mode() {
                    Mode::Triangles if n % 3 == 2 => Some([a, b, c]),
                    Mode::TriangleStrip if n >= 2 => {
                        Some(if n % 2 == 0 { [a, b, c] } else { [b, a, c] })
                    }
                    Mode::TriangleFan if n >= 2 => Some([first, b, c]),
                    _ => None,
                };
                if let Some(indices) = tri {
                    faces.push(Face { indices, color });
                }
                a = b;
                b = c;
            }
        }
    }
    ensure!(
        !faces.is_empty(),
        "The export contains no triangle surfaces."
    );
    let mut low = DVec3::splat(f64::INFINITY);
    let mut high = -low;
    for p in &world {
        ensure!(p.is_finite(), "Invalid vertex position.");
        low = low.min(*p);
        high = high.max(*p);
    }
    let center = (high + low) * 0.5;
    let radius = world
        .iter()
        .map(|p| p.distance_squared(center))
        .fold(0., f64::max)
        .sqrt();
    if !radius.is_finite() || radius == 0. {
        bail!("The mesh has no spatial extent.");
    }
    let vertices: Vec<_> = world
        .iter()
        .map(|p| ((*p - center) / radius).as_vec3())
        .collect();
    Ok(Mesh {
        vertices,
        faces,
        overlaps: Vec::new(),
    })
}
