use super::{
    mesh::decode,
    render::{View, draw},
};
use anyhow::Result;
use serde_json::json;
use std::sync::atomic::AtomicBool;

fn instanced_triangle() -> Vec<u8> {
    // All four nodes share a mesh. Two coincide through different parent transforms.
    // The last is far away in a scene that is not selected.
    let description = json!({
        "asset":{"version":"2.0"}, "scene":0, "scenes":[{"nodes":[0,2,3]},{"nodes":[4]}],
        "nodes":[{"translation":[1.,0.,0.],"children":[1]}, {"translation":[-1.,0.,0.],"mesh":0},
                 {"mesh":0},{"mesh":0,"translation":[0.,0.,1.]},{"mesh":0,"translation":[100.,0.,0.]}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
        "buffers":[{"byteLength":44}],
        "bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":6}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0.,0.,0.],"max":[1.,1.,0.]},
                     {"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}]
    });
    let mut json = description.to_string().into_bytes();
    json.resize(json.len().next_multiple_of(4), b' ');
    let mut bin: Vec<_> = [0_f32, 0., 0., 1., 0., 0., 0., 1., 0.]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    bin.extend([0_u16, 1, 2].into_iter().flat_map(u16::to_le_bytes));
    bin.resize(44, 0);
    let mut glb = b"glTF".to_vec();
    glb.extend(2_u32.to_le_bytes());
    glb.extend(((28 + json.len() + bin.len()) as u32).to_le_bytes());
    for (tag, data) in [(b"JSON", json), (b"BIN\0", bin)] {
        glb.extend((data.len() as u32).to_le_bytes());
        glb.extend(tag);
        glb.extend(data);
    }
    glb
}

#[test]
fn inspects_active_scene_instances_and_draws_orbit_zoom_and_modes() -> Result<()> {
    let stop = AtomicBool::new(false);
    let mut mesh = decode(&instanced_triangle(), &stop)?;
    let (flags, complete) = super::overlap::find(&mesh.vertices, &mesh.faces, &stop);
    mesh.overlaps = flags;
    assert_eq!(mesh.faces.len(), 3);
    assert_eq!(mesh.overlaps.iter().filter(|&&v| v).count(), 2);
    assert!(complete);
    let view = View {
        yaw: 0.,
        pitch: 0.,
        zoom: 1.,
        mode: 0,
        width: 240,
        height: 200,
    };
    let frame = |v| draw(&mesh, v, &stop).unwrap().rgb;
    let surface = frame(view);
    assert_eq!(surface.len(), 240 * 200 * 3);
    assert_ne!(surface, frame(View { yaw: 37., ..view }));
    assert_ne!(surface, frame(View { zoom: 0.6, ..view }));
    assert_ne!(surface, frame(View { mode: 1, ..view }));
    let orange = frame(View { mode: 2, ..view });
    assert!(
        orange
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|p| p[0] > p[2].saturating_add(50))
            .count()
            > 300
    );
    // The duplicated faces at z=0 are visible through the unflagged face at z=1.
    assert_eq!(surface, frame(view));
    Ok(())
}
