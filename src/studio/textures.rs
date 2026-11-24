use anyhow::Result;
use image::{Rgb, RgbImage};
use std::path::Path;

fn hash(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9e3779b1) ^ y.wrapping_mul(0x85ebca6b) ^ seed;
    h = (h ^ (h >> 16)).wrapping_mul(0x7feb352d);
    h = (h ^ (h >> 15)).wrapping_mul(0x846ca68b);
    (h ^ (h >> 16)) as f32 / u32::MAX as f32
}
// Wrap the lattice at each octave so a repeated tile has a continuous seam.
fn noise(x: f32, y: f32, cells: u32, seed: u32) -> f32 {
    let (u, v) = (x * cells as f32, y * cells as f32);
    let (i, j) = (u.floor() as u32, v.floor() as u32);
    let smooth = |a: f32| a * a * (3.0 - 2.0 * a);
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let value = |dx, dy| hash((i + dx) % cells, (j + dy) % cells, seed);
    mix(
        mix(value(0, 0), value(1, 0), smooth(u.fract())),
        mix(value(0, 1), value(1, 1), smooth(u.fract())),
        smooth(v.fract()),
    )
}
pub(super) fn generate(dir: &Path, material: &str, seed: u32) -> Result<()> {
    let palette: [[u8; 3]; 5] = match material {
        "Moss" => [
            [37, 43, 39],
            [55, 68, 48],
            [76, 91, 55],
            [107, 119, 71],
            [150, 149, 94],
        ],
        "Sand" => [
            [137, 98, 66],
            [170, 128, 81],
            [198, 159, 103],
            [218, 185, 129],
            [235, 207, 158],
        ],
        _ => [
            [42, 44, 51],
            [56, 60, 69],
            [73, 78, 84],
            [93, 99, 102],
            [119, 124, 124],
        ],
    };
    let image = RgbImage::from_fn(256, 256, |x, y| {
        let (u, v) = ((x / 2) as f32 / 128.0, (y / 2) as f32 / 128.0);
        let coarse = noise(u, v, 5, seed) * 0.54 + noise(u, v, 14, seed.wrapping_add(1)) * 0.30;
        let grain = noise(u, v, 49, seed.wrapping_add(2)) * 0.16;
        let value = ((coarse + grain - 0.19) * 6.4).clamp(0.0, 3.99);
        let index = value as usize;
        let fraction = (value.fract() * 3.0).floor() / 3.0;
        Rgb(std::array::from_fn(|i| {
            let a = palette[index][i] as f32;
            (a + (palette[index + 1][i] as f32 - a) * fraction) as u8
        }))
    });
    image.save(dir.join("image.png"))?;
    Ok(())
}
