//! Level lighting, as the original does it: every corner of the level is lit once at load from
//! the lights of its own room. Characters are lit as they move, by the level's ambient light and
//! the two strongest lights of the room they stand in.

use std::collections::HashMap;

use crate::crp::Crp;
use crate::dmp::{Dmp, Kind, Light};
use crate::map::{Map, object_room};

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let n = dot(v, v).sqrt();
    if n > 1e-9 { [v[0] / n, v[1] / n, v[2] / n] } else { v }
}

/// How far into a light's range `p` is, as 1 at the light and 0 at the edge. None outside.
fn falloff(light: &Light, p: [f32; 3]) -> Option<f32> {
    let d = sub(p, light.position);
    let (d2, r2) = (dot(d, d), light.range * light.range);
    (d2 < r2).then(|| 1.0 - d2 / r2)
}

fn in_cone(light: &Light, p: [f32; 3]) -> bool {
    let cos = dot(unit(sub(p, light.position)), unit(light.direction));
    cos >= (light.cone_degrees.to_radians() / 2.0).cos()
}

/// What a light adds to a level corner at `p` with the unit normal `n`
fn level_light(light: &Light, p: [f32; 3], n: [f32; 3]) -> f32 {
    match light.kind {
        Kind::Directional => (-dot(light.direction, n)).max(0.0),
        Kind::Point => {
            let Some(f) = falloff(light, p) else { return 0.0 };
            if dot(sub(light.position, p), n) > 0.0 { f } else { 0.0 }
        }
        Kind::Spot => {
            let Some(f) = falloff(light, p) else { return 0.0 };
            if dot(light.direction, n) < 0.0 && in_cone(light, p) { f } else { 0.0 }
        }
    }
}

pub struct Lighting {
    pub ambient: [f32; 3],
    rooms: HashMap<u32, Vec<Light>>,
}

impl Lighting {
    /// The lights by the number of the room they are in. Lights in a room that has no number are left out.
    pub fn new(dmp: Dmp) -> Lighting {
        let mut rooms: HashMap<u32, Vec<Light>> = HashMap::new();
        for light in dmp.lights {
            if let Some(room) = object_room(&light.room) {
                rooms.entry(room).or_default().push(light);
            }
        }
        Lighting { ambient: dmp.ambient, rooms }
    }

    fn lights_of(&self, room: u32) -> &[Light] {
        self.rooms.get(&room).map(Vec::as_slice).unwrap_or_default()
    }

    /// The light for a character in `room` with its chest at `chest`: the level's ambient light
    /// (doubled where it is dim) and the two strongest lights there
    pub fn character(&self, room: Option<u32>, chest: [f32; 3]) -> CharacterLight {
        let ambient = if self.ambient.iter().all(|&c| c < 0.16) { self.ambient.map(|c| c * 2.0) } else { self.ambient };
        let mut best: Vec<(f32, &Light)> = Vec::with_capacity(3);
        for light in room.map(|r| self.lights_of(r)).unwrap_or_default() {
            let reach = match light.kind {
                Kind::Directional => 1.0,
                _ => falloff(light, chest).unwrap_or(0.0),
            };
            let strength = reach * light.diffuse.iter().sum::<f32>();
            if strength > 0.0 {
                best.push((strength, light));
                best.sort_by(|a, b| b.0.total_cmp(&a.0));
                best.truncate(2);
            }
        }
        let lights: Vec<Light> = best.into_iter().map(|(_, l)| l.clone()).collect();
        // A gun has no normals, so it takes one color with half of each light. The half is our
        // guess at the angle, not the original's.
        let mut tint = ambient;
        for l in &lights {
            let reach = if l.kind == Kind::Directional { 1.0 } else { falloff(l, chest).unwrap_or(0.0) };
            for k in 0..3 {
                tint[k] += 0.5 * reach * l.diffuse[k];
            }
        }
        CharacterLight { ambient, lights, tint: tint.map(|c| c.min(1.0)) }
    }
}

/// The light on one character, in the space of the level or (after `local`) of the character's own
#[derive(Clone, Debug)]
pub struct CharacterLight {
    ambient: [f32; 3],
    lights: Vec<Light>,
    /// One color for the whole character, for what is drawn without normals
    pub tint: [f32; 3],
}

impl CharacterLight {
    /// The same light seen from a character standing at `feet`, turned `yaw_degrees` about the
    /// up axis and scaled by `scale`
    pub fn local(&self, feet: [f32; 3], yaw_degrees: f32, scale: [f32; 3]) -> CharacterLight {
        let (s, c) = yaw_degrees.to_radians().sin_cos();
        let size = (scale[0] + scale[1] + scale[2]) / 3.0;
        // The inverse of the turn: the model's x and z carried back
        let turn = |v: [f32; 3]| [c * v[0] - s * v[2], v[1], s * v[0] + c * v[2]];
        let lights = self.lights.iter().map(|l| Light {
            position: turn(sub(l.position, feet)).map(|v| v / size),
            direction: turn(l.direction),
            range: l.range / size,
            ..l.clone()
        }).collect();
        CharacterLight { ambient: self.ambient, lights, tint: self.tint }
    }

    fn at(&self, p: [f32; 3], n: [f32; 3]) -> [f32; 3] {
        let mut color = self.ambient;
        for l in &self.lights {
            let k = match l.kind {
                Kind::Directional => (-dot(l.direction, n)).max(0.0),
                Kind::Point => match falloff(l, p) {
                    Some(f) => dot(unit(sub(l.position, p)), n).max(0.0) * f,
                    None => 0.0,
                },
                Kind::Spot => match falloff(l, p) {
                    Some(f) if in_cone(l, p) => dot(unit(sub(l.position, p)), n).max(0.0) * f,
                    _ => 0.0,
                },
            };
            for i in 0..3 {
                color[i] += k * l.diffuse[i];
            }
        }
        color.map(|c| c.clamp(0.0, 1.0))
    }

    /// A color for each corner of the posed model, three floats each, in the order the parts'
    /// triangles come. Each triangle is lit by its own normal.
    pub fn corner_colors(&self, model: &Crp, verts: &[[f32; 3]], out: &mut Vec<f32>) {
        out.clear();
        for part in &model.parts {
            for t in &part.triangles {
                let [a, b, c] = t.map(|i| verts[i as usize]);
                let (e, f) = (sub(b, a), sub(c, a));
                let n = unit([e[1] * f[2] - e[2] * f[1], e[2] * f[0] - e[0] * f[2], e[0] * f[1] - e[1] * f[0]]);
                for p in [a, b, c] {
                    out.extend(self.at(p, n));
                }
            }
        }
    }
}

/// The color of each corner of each face of each part of each object of a level, in the order of
/// the faces: `colors[object][part][face]`
pub struct Baked {
    pub colors: Vec<Vec<Vec<[[f32; 3]; 3]>>>,
}

fn center(vertices: &[crate::map::Vertex]) -> [f32; 3] {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in vertices {
        for (k, c) in [v.x, v.y, v.z].into_iter().enumerate() {
            lo[k] = lo[k].min(c);
            hi[k] = hi[k].max(c);
        }
    }
    [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0)
}

impl Lighting {
    /// Light every corner of the level once. An object takes the lights of the room its name
    /// starts with; the door of the next room near a portal takes a room's point and spot lights too.
    pub fn bake(&self, map: &Map) -> Baked {
        let objects = &map.geometries.objects;
        let mut next_door: HashMap<usize, Vec<&Light>> = HashMap::new();
        for light in self.rooms.values().flatten().filter(|l| l.kind != Kind::Directional) {
            let Some(room) = object_room(&light.room) else { continue };
            for portal in map.portals.portals.iter().filter(|p| p.room == room && !p.coordinates.is_empty()) {
                let n = portal.coordinates.len() as f32;
                let at = portal.coordinates.iter().fold([0.0; 3], |s, v| [s[0] + v.x / n, s[1] + v.y / n, s[2] + v.z / n]);
                for (i, o) in objects.iter().enumerate() {
                    if object_room(&o.name) == Some(portal.opposite_room) && o.name.to_lowercase().contains("door") {
                        let d = sub(center(&o.vertices), at);
                        let door = next_door.entry(i).or_default();
                        // Two portals near one door must not light it twice
                        if dot(d, d) < 10000.0 && !door.iter().any(|l| std::ptr::eq(*l, light)) {
                            door.push(light);
                        }
                    }
                }
            }
        }
        let colors = objects.iter().enumerate().map(|(i, object)| {
            let own = object_room(&object.name).map(|r| self.lights_of(r)).unwrap_or_default();
            let lights: Vec<&Light> = own.iter().chain(next_door.get(&i).into_iter().flatten().copied()).collect();
            object.object_datas.iter().map(|od| {
                let faces = od.faces.face_indices.iter().zip(&od.faces.texture_indices).enumerate();
                faces.map(|(f, (&(v0, v1, v2), &(t0, t1, t2)))| {
                    let corner = |v: u16, t: u16| {
                        let p = &object.vertices[v as usize];
                        let p = [p.x, p.y, p.z];
                        let n = od.texture_vertices.normals.get(t as usize).map(|n| unit([n.x, n.y, n.z]))
                            .filter(|n| dot(*n, *n) > 0.5)
                            .unwrap_or_else(|| od.faces.normals.get(f).map_or([0.0; 3], |n| [n.x, n.y, n.z]));
                        let mut color = self.ambient;
                        for l in &lights {
                            let k = level_light(l, p, n);
                            for c in 0..3 {
                                color[c] += k * l.diffuse[c];
                            }
                        }
                        color.map(|c| c.clamp(0.0, 1.0))
                    };
                    [corner(v0, t0), corner(v1, t1), corner(v2, t2)]
                }).collect()
            }).collect()
        }).collect();
        Baked { colors }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(kind: Kind) -> Light {
        Light { room: "1".into(), name: "l".into(), kind, direction: [0.0, -1.0, 0.0], position: [0.0, 100.0, 0.0], cone_degrees: 90.0, range: 200.0, diffuse: [1.0, 0.5, 0.0] }
    }

    #[test]
    fn a_point_light_fades_to_nothing_at_its_range_and_needs_a_facing_surface() {
        let l = light(Kind::Point);
        let down = [0.0, 1.0, 0.0];
        assert!((level_light(&l, [0.0, 0.0, 0.0], down) - 0.75).abs() < 1e-6);
        assert_eq!(level_light(&l, [0.0, 0.0, 0.0], [0.0, -1.0, 0.0]), 0.0);
        assert_eq!(level_light(&l, [300.0, 0.0, 0.0], down), 0.0);
    }

    #[test]
    fn a_directional_light_follows_the_angle_of_the_surface() {
        let l = light(Kind::Directional);
        assert_eq!(level_light(&l, [0.0; 3], [0.0, 1.0, 0.0]), 1.0);
        assert_eq!(level_light(&l, [0.0; 3], [1.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn a_spot_lights_only_inside_its_cone() {
        let l = light(Kind::Spot);
        assert!(level_light(&l, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]) > 0.0);
        assert_eq!(level_light(&l, [90.0, 90.0, 0.0], [0.0, 1.0, 0.0]), 0.0);
    }

    #[test]
    fn a_character_takes_the_two_strongest_lights() {
        let mut dim = light(Kind::Point);
        dim.diffuse = [0.1; 3];
        let mut lighting = Lighting { ambient: [0.1; 3], rooms: HashMap::new() };
        lighting.rooms.insert(1, vec![dim.clone(), light(Kind::Point), dim.clone(), light(Kind::Directional)]);
        let lit = lighting.character(Some(1), [0.0, 80.0, 0.0]);
        assert_eq!(lit.lights.len(), 2);
        assert!(lit.lights.iter().all(|l| l.diffuse[0] == 1.0));
        // A dim level's ambient light is doubled
        assert_eq!(lit.ambient, [0.2; 3]);
    }
}
