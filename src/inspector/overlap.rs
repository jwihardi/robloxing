use super::mesh::Face;
use glam::{DVec2, DVec3, Vec3};
use std::sync::atomic::{AtomicBool, Ordering};

// Positions have already been centered and scaled to a unit radius.
const PLANE_EPS: f64 = 1e-7;
const MAX_COMPARISONS: usize = 2_000_000;

struct Triangle {
    corners: [DVec3; 3],
    normal: DVec3,
    min: DVec3,
    max: DVec3,
}

impl Triangle {
    fn new(corners: [DVec3; 3]) -> Self {
        let [a, b, c] = corners;
        Self {
            corners,
            normal: (b - a).cross(c - a).normalize_or_zero(),
            min: a.min(b).min(c),
            max: a.max(b).max(c),
        }
    }
    fn projects_onto(&self, other: &Self) -> bool {
        if self.normal.dot(other.normal).abs() < 1. - 1e-10
            || other
                .corners
                .iter()
                .any(|p| (*p - self.corners[0]).dot(self.normal).abs() > PLANE_EPS)
        {
            return false;
        }
        let axis = self.normal.abs().max_position();
        let project = |p: DVec3| DVec2::new(p[(axis + 1) % 3], p[(axis + 2) % 3]);
        let a = self.corners.map(project);
        let b = other.corners.map(project);
        // Strict separating-axis test excludes faces that only share an edge or a point.
        for tri in [&a, &b] {
            for i in 0..3 {
                let edge = (tri[(i + 1) % 3] - tri[i]).perp();
                let span = |points: &[DVec2; 3]| {
                    let dots = points.map(|p| p.dot(edge));
                    (
                        dots[0].min(dots[1]).min(dots[2]),
                        dots[0].max(dots[1]).max(dots[2]),
                    )
                };
                let (al, ah) = span(&a);
                let (bl, bh) = span(&b);
                if ah.min(bh) - al.max(bl) <= edge.length() * 1e-10 {
                    return false;
                }
            }
        }
        true
    }
}

pub(super) fn find(vertices: &[Vec3], faces: &[Face], cancelled: &AtomicBool) -> (Vec<bool>, bool) {
    let triangles: Vec<_> = faces
        .iter()
        .map(|f| Triangle::new(f.indices.map(|i| vertices[i].as_dvec3())))
        .collect();
    let mut found = vec![false; faces.len()];
    let mut order: Vec<_> = (0..faces.len())
        .filter(|&i| triangles[i].normal != DVec3::ZERO)
        .collect();
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = -lo;
    for &i in &order {
        let mid = (triangles[i].min + triangles[i].max) * 0.5;
        lo = lo.min(mid);
        hi = hi.max(mid);
    }
    let axis = (hi - lo).max_position();
    order.sort_unstable_by(|&a, &b| triangles[a].min[axis].total_cmp(&triangles[b].min[axis]));
    let mut checked = 0;
    for (k, &i) in order.iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return (found, false);
        }
        let a = &triangles[i];
        for &j in &order[k + 1..] {
            let b = &triangles[j];
            if b.min[axis] > a.max[axis] + PLANE_EPS {
                break;
            }
            checked += 1;
            if checked > MAX_COMPARISONS {
                return (found, false);
            }
            if (found[i] && found[j])
                || (a.max + PLANE_EPS).cmplt(b.min).any()
                || (b.max + PLANE_EPS).cmplt(a.min).any()
            {
                continue;
            }
            if a.projects_onto(b) {
                found[i] = true;
                found[j] = true;
            }
        }
    }
    (found, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_duplicate_reverse_shared_edge_and_distinct_layers() {
        let pair = |points: [[f32; 3]; 3]| {
            let mut v = vec![Vec3::ZERO, Vec3::X, Vec3::Y];
            v.extend(points.map(Vec3::from_array));
            let faces = [[0, 1, 2], [3, 4, 5]].map(|indices| Face {
                indices,
                color: [0; 3],
            });
            find(&v, &faces, &AtomicBool::new(false))
        };
        assert_eq!(
            pair([[0., 1., 0.], [1., 0., 0.], [0., 0., 0.]]),
            (vec![true, true], true)
        );
        assert_eq!(
            pair([[0.25, -0.5, 0.], [0.25, 1.5, 0.], [0.5, 0.5, 0.]]).0,
            [true, true]
        );
        for tri in [
            [[1., 1., 0.], [0., 1., 0.], [1., 0., 0.]],
            [[0., 0., 0.], [-1., 0., 0.], [0., -1., 0.]],
            [[0., 0., 1e-4], [1., 0., 1e-4], [0., 1., 1e-4]],
            [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.2]],
            [[0., 0., 0.], [0.2, 0., 0.], [0.5, 0., 0.]],
        ] {
            assert_eq!(pair(tri), (vec![false, false], true));
        }
    }
}
