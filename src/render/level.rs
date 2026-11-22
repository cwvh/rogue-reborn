//! Tables about the level's objects, built once for drawing it.

use std::collections::HashMap;
use rogue_reborn::{coplanar, map, sim};
use crate::textures;

pub struct LevelDraw {
    // Faces lying over others in the same plane (a door frame on its wall) get drawn on top
    pub coplanar_layers: Vec<coplanar::Layers>,
    // Which objects are glass panes, drawn in place
    pub pane_objects: Vec<bool>,
    pub sprite_blends: Vec<Option<bool>>,
    // Lamps and the like are drawn as lit as they are, whatever light falls on them
    pub glowing: Vec<bool>,
    pub all_objects: Vec<usize>,
    // Doors: each leaf is drawn where its door is. Space or Backspace opens and closes the
    // one being looked at.
    pub door_leaf: Vec<bool>,
    pub doors_by_room: HashMap<u32, Vec<usize>>,
    pub all_doors: Vec<usize>,
}

/// How a material's soft sprite is blended over the level, if it is one: `Some(true)` adds
/// (a light, whose material glows), `Some(false)` blends (a shadow, a stain). Other .tga
/// materials are cut out with an alpha test instead: foliage (two-sided), and sprites whose
/// pixels are nearly all solid or clear (a wheel's face), which blending would only soften
/// at the edges. Glass panes are drawn apart.
fn sprite_blend(m: &map::Material, map_num: &str) -> Option<bool> {
    if !textures::is_tga(&m.filename) || m.two_sided {
        return None;
    }
    let glows = m.emissive_strength > 0.0;
    (glows || textures::has_soft_alpha(map_num, &m.filename)).then_some(glows)
}

impl LevelDraw {
    pub fn new(level: &map::Map, sim: &sim::Sim, map_num: &str) -> LevelDraw {
        let coplanar_layers = level.geometries.objects.iter().map(coplanar::layers).collect();
        let pane_objects = level.geometries.objects.iter().map(|o| sim.panes.iter().any(|g| g.name == o.name)).collect();
        let sprite_blends = level.materials.materials.iter().map(|m| sprite_blend(m, map_num)).collect();
        let glowing = level.materials.materials.iter().map(|m| m.emissive_strength > 0.0).collect();
        let all_objects = (0..level.geometries.objects.len()).collect();
        let door_leaf = (0..level.geometries.objects.len()).map(|i| sim.doors.iter().any(|d| d.model_index == i)).collect();
        let mut doors_by_room: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, d) in sim.doors.iter().enumerate() {
            for &r in &d.rooms {
                doors_by_room.entry(r).or_default().push(i);
            }
        }
        let all_doors = (0..sim.doors.len()).collect();
        LevelDraw { coplanar_layers, pane_objects, sprite_blends, glowing, all_objects, door_leaf, doors_by_room, all_doors }
    }
}
