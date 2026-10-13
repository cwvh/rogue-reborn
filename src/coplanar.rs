//! Faces lying in the same plane and overlapping, like a door frame's jamb drawn over the
//! wall it is set into. Their depths are equal, so which one shows depends on rounding and
//! changes as the camera moves: the surface flickers. The smaller face is the one meant to
//! show, so it is drawn with a depth offset toward the camera.

use std::collections::HashMap;

use crate::map::Object;

/// How far above the surfaces beneath them the faces of an object are, one entry per face
/// of each `object_datas` entry (0 means not covering anything).
pub struct Layers {
    pub per_face: Vec<Vec<u8>>,
    /// The highest layer used, so objects with no overlaps are drawn in one pass
    pub max: u8,
}

/// The most layers told apart; a face under more than this shares the top one
const MAX_LAYER: u8 = 4;
/// Planes closer than this (in map units) are the same plane
const PLANE_TOLERANCE: f32 = 0.05;
/// Triangles overlapping by less than this (in map units) only touch
const TOUCH: f32 = 0.05;
/// Faces below this area are skipped
const MIN_AREA: f32 = 1e-3;

struct Face {
    od: usize,
    index: usize,
    corners: [[f32; 3]; 3],
    area: f32,
    /// The two axes left once the plane's strongest one is dropped
    axes: [usize; 2],
}

pub fn layers(object: &Object) -> Layers {
    let mut per_face: Vec<Vec<u8>> = object.object_datas.iter()
        .map(|od| vec![0; od.faces.face_indices.len()])
        .collect();

    // Group by plane: the normal rounded to hundredths, then the distance in unit steps
    let mut planes: HashMap<([i32; 3], i32), Vec<Face>> = HashMap::new();
    for (od_index, od) in object.object_datas.iter().enumerate() {
        for (index, &(i0, i1, i2)) in od.faces.face_indices.iter().enumerate() {
            let corners = [i0, i1, i2].map(|i| {
                let v = &object.vertices[i as usize];
                [v.x, v.y, v.z]
            });
            let n = cross(sub(corners[1], corners[0]), sub(corners[2], corners[0]));
            let length = dot(n, n).sqrt();
            if length / 2.0 < MIN_AREA {
                continue;
            }
            let mut n = n.map(|c| c / length);
            // A plane and its back face are not the same surface, but two faces
            // wound the opposite way are, so orient the normal consistently
            let flip = n.iter().copied().find(|c| c.abs() > 0.01).is_some_and(|c| c < 0.0);
            if flip {
                n = n.map(|c| -c);
            }
            let d = dot(n, corners[0]);
            let strongest = (0..3).max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs())).unwrap();
            let axes = match strongest {
                0 => [1, 2],
                1 => [0, 2],
                _ => [0, 1],
            };
            let key = (n.map(|c| (c * 100.0).round() as i32), d.floor() as i32);
            planes.entry(key).or_default().push(Face { od: od_index, index, corners, area: length / 2.0, axes });
        }
    }

    // A face's own bucket and the ones either side of it hold every face within the tolerance
    let keys: Vec<_> = planes.keys().copied().collect();
    for key in keys {
        let mut group: Vec<&Face> = Vec::new();
        for step in -1..=1 {
            if let Some(faces) = planes.get(&(key.0, key.1 + step)) {
                group.extend(faces);
            }
        }
        let mine = &planes[&key];
        for face in mine {
            let d_face = plane_distance(face);
            let mut covered = 0u8;
            for other in &group {
                if other.area <= face.area * 1.001 || (plane_distance(other) - d_face).abs() > PLANE_TOLERANCE {
                    continue;
                }
                if overlap(face, other) {
                    covered = (covered + 1).min(MAX_LAYER);
                }
            }
            per_face[face.od][face.index] = covered;
        }
    }
    let max = per_face.iter().flatten().copied().max().unwrap_or(0);
    Layers { per_face, max }
}

fn plane_distance(face: &Face) -> f32 {
    let n = cross(sub(face.corners[1], face.corners[0]), sub(face.corners[2], face.corners[0]));
    let length = dot(n, n).sqrt();
    let n = n.map(|c| c / length);
    let sign = if n.iter().copied().find(|c| c.abs() > 0.01).is_some_and(|c| c < 0.0) { -1.0 } else { 1.0 };
    sign * dot(n, face.corners[0])
}

/// Whether two coplanar triangles share more than an edge or a corner
fn overlap(a: &Face, b: &Face) -> bool {
    let project = |f: &Face| f.corners.map(|c| [c[a.axes[0]], c[a.axes[1]]]);
    let (ta, tb) = (project(a), project(b));
    for t in [&ta, &tb] {
        for i in 0..3 {
            let (p, q) = (t[i], t[(i + 1) % 3]);
            let axis = [q[1] - p[1], p[0] - q[0]];
            let length = (axis[0] * axis[0] + axis[1] * axis[1]).sqrt();
            if length == 0.0 {
                continue;
            }
            let range = |t: &[[f32; 2]; 3]| {
                let s = t.map(|c| (c[0] * axis[0] + c[1] * axis[1]) / length);
                (s[0].min(s[1]).min(s[2]), s[0].max(s[1]).max(s[2]))
            };
            let (a_min, a_max) = range(&ta);
            let (b_min, b_max) = range(&tb);
            if a_max <= b_min + TOUCH || b_max <= a_min + TOUCH {
                return false;
            }
        }
    }
    true
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map;
    use std::path::Path;

    #[test]
    fn a_door_frame_set_into_a_wall_is_drawn_over_it() {
        let path = Path::new("data/map/m00/citystreet_large.map");
        if !path.exists() {
            return;
        }
        let level = map::read(path).unwrap();
        // Room 3's wall at z=34737 and the jamb of the doorframe set into it
        let object = level.geometries.objects.iter().find(|o| o.name == "3").unwrap();
        let layers = layers(object);
        let mut jamb = 0;
        let mut wall = 0;
        for (od_index, od) in object.object_datas.iter().enumerate() {
            let material = &level.materials.materials[od.mn as usize].filename;
            for (index, &(i0, i1, i2)) in od.faces.face_indices.iter().enumerate() {
                let v = [i0, i1, i2].map(|i| &object.vertices[i as usize]);
                if v.iter().any(|p| p.z != 34737.0) || !v.iter().any(|p| p.x == 32867.0 && p.y == 33167.0) {
                    continue;
                }
                match material.as_str() {
                    "tan_wood_frame.bmp" => jamb = jamb.max(layers.per_face[od_index][index]),
                    "plywood_tan.bmp" => wall = wall.max(layers.per_face[od_index][index]),
                    _ => {}
                }
            }
        }
        assert!(jamb > wall, "jamb layer {jamb}, wall layer {wall}");
    }

    #[test]
    fn touching_faces_are_not_overlapping() {
        let face = |corners: [[f32; 3]; 3]| Face { od: 0, index: 0, corners, area: 0.5, axes: [0, 1] };
        let a = face([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        let beside = face([[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]);
        let over = face([[0.2, 0.2, 0.0], [0.9, 0.2, 0.0], [0.2, 0.7, 0.0]]);
        assert!(!overlap(&a, &beside));
        assert!(overlap(&a, &over));
    }
}
