//! Which rooms can be seen from a room, by walking the portals that are in view.
//!
//! The levels hold "impossible geometry": the inside of a plane is bigger than its hull,
//! so drawing every room at once shows the hull through the cabin and the cabin through
//! the hull. The original engine drew only the room the player was in plus whatever was
//! visible through the portals, and so does this.

use std::collections::HashMap;

use crate::map::Map;

/// A screen rectangle in normalised device coordinates: min x, min y, max x, max y
pub type Rect = [f32; 4];

const FULL: Rect = [-1.0, -1.0, 1.0, 1.0];
/// How close the player must be to a portal to be standing in it
const TOUCH: f32 = 40.0;
/// How many portals deep to look
const MAX_DEPTH: usize = 8;
/// Portals seen edge-on have no area; give them this much so they still count
const SLIVER: f32 = 0.005;

pub struct Visibility {
    /// For each room number, the indices of the portals that touch it
    by_room: HashMap<u32, Vec<usize>>,
}

impl Visibility {
    pub fn new(map: &Map) -> Self {
        let mut by_room: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, p) in map.portals.portals.iter().enumerate() {
            by_room.entry(p.room).or_default().push(i);
            if p.opposite_room != p.room {
                by_room.entry(p.opposite_room).or_default().push(i);
            }
        }
        Self { by_room }
    }

    /// A room and the rooms its portals lead to
    pub fn with_neighbours(&self, map: &Map, room: u32) -> Vec<u32> {
        let mut rooms = vec![room];
        for &i in self.by_room.get(&room).map(Vec::as_slice).unwrap_or_default() {
            let p = &map.portals.portals[i];
            let other = if p.room == room { p.opposite_room } else { p.room };
            if !rooms.contains(&other) {
                rooms.push(other);
            }
        }
        rooms.sort();
        rooms
    }

    /// The rooms visible from `start`, each with the screen rectangle it can be seen
    /// through (the whole screen for `start` itself). `clip` takes a map-space point to
    /// clip space (x, y, z, w) for the current view.
    ///
    /// A portal within `TOUCH` units of one of the `touch` points (the player's feet and
    /// eyes) lets its room through unclipped: standing in a doorway, or looking along the
    /// wall it is in, would otherwise squeeze the room beyond to a sliver.
    pub fn visible(&self, map: &Map, start: u32, touch: &[[f32; 3]], clip: &dyn Fn([f32; 3]) -> [f32; 4]) -> HashMap<u32, Rect> {
        let mut out = HashMap::new();
        self.walk(map, start, FULL, touch, clip, &mut vec![], &mut out);
        out
    }

    fn walk(&self, map: &Map, room: u32, rect: Rect, touch: &[[f32; 3]], clip: &dyn Fn([f32; 3]) -> [f32; 4], path: &mut Vec<u32>, out: &mut HashMap<u32, Rect>) {
        match out.get_mut(&room) {
            // Already seen through a window that covers this one: nothing new beyond it
            Some(seen) if contains(*seen, rect) => return,
            Some(seen) => *seen = union(*seen, rect),
            None => { out.insert(room, rect); }
        }
        if path.len() >= MAX_DEPTH {
            return;
        }
        path.push(room);
        for &i in self.by_room.get(&room).map(Vec::as_slice).unwrap_or_default() {
            let portal = &map.portals.portals[i];
            let other = if portal.room == room { portal.opposite_room } else { portal.room };
            if path.contains(&other) {
                continue;
            }
            let corners: Vec<[f32; 3]> = portal.coordinates.iter().map(|v| [v.x, v.y, v.z]).collect();
            let through = if touch.iter().any(|&p| distance_to_polygon(&corners, p) <= TOUCH) {
                Some(rect)
            } else {
                project(&corners, clip).and_then(|r| intersect(r, rect))
            };
            if let Some(through) = through {
                self.walk(map, other, through, touch, clip, path, out);
            }
        }
        path.pop();
    }
}

/// The screen rectangle a convex polygon covers, or None when it is entirely behind the
/// camera. The part in front of the near plane is kept, so a doorway the camera is
/// standing in still counts.
pub fn project(corners: &[[f32; 3]], clip: &dyn Fn([f32; 3]) -> [f32; 4]) -> Option<Rect> {
    const NEAR_W: f32 = 0.01;
    let pts: Vec<[f32; 4]> = corners.iter().map(|&c| clip(c)).collect();
    // Sutherland-Hodgman against the plane w = NEAR_W
    let mut kept: Vec<[f32; 4]> = Vec::new();
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        let (a_in, b_in) = (a[3] > NEAR_W, b[3] > NEAR_W);
        if a_in {
            kept.push(a);
        }
        if a_in != b_in {
            let t = (NEAR_W - a[3]) / (b[3] - a[3]);
            kept.push(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t));
        }
    }
    if kept.is_empty() {
        return None;
    }
    let mut rect = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in &kept {
        let (x, y) = (p[0] / p[3], p[1] / p[3]);
        rect = [rect[0].min(x), rect[1].min(y), rect[2].max(x), rect[3].max(y)];
    }
    Some([rect[0] - SLIVER, rect[1] - SLIVER, rect[2] + SLIVER, rect[3] + SLIVER])
}

/// How far a point is from the bounding box of a polygon (portals are flat rectangles)
fn distance_to_polygon(corners: &[[f32; 3]], p: [f32; 3]) -> f32 {
    let mut sum = 0.0;
    for axis in 0..3 {
        let lo = corners.iter().map(|c| c[axis]).fold(f32::MAX, f32::min);
        let hi = corners.iter().map(|c| c[axis]).fold(f32::MIN, f32::max);
        let past = (lo - p[axis]).max(p[axis] - hi).max(0.0);
        sum += past * past;
    }
    f32::sqrt(sum)
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    (r[0] < r[2] && r[1] < r[3]).then_some(r)
}

fn union(a: Rect, b: Rect) -> Rect {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
}

#[cfg(test)]
mod tests {
    use super::*;

    // A camera at the origin looking down +z with a 90 degree view: clip = (x, y, z, z)
    fn view(p: [f32; 3]) -> [f32; 4] {
        [p[0], p[1], p[2], p[2]]
    }

    #[test]
    fn a_portal_ahead_covers_part_of_the_screen() {
        let door = [[-1.0, -1.0, 4.0], [1.0, -1.0, 4.0], [1.0, 1.0, 4.0], [-1.0, 1.0, 4.0]];
        let r = project(&door, &view).unwrap();
        assert!((r[0] + 0.25).abs() < 0.01 && (r[2] - 0.25).abs() < 0.01, "{r:?}");
    }

    #[test]
    fn a_portal_behind_the_camera_is_not_visible() {
        let door = [[-1.0, -1.0, -4.0], [1.0, -1.0, -4.0], [1.0, 1.0, -4.0], [-1.0, 1.0, -4.0]];
        assert!(project(&door, &view).is_none());
    }

    #[test]
    fn a_doorway_around_the_camera_fills_the_screen() {
        // One side of the doorway is behind the camera, the rest in front
        let door = [[-1.0, -1.0, -1.0], [-1.0, -1.0, 3.0], [-1.0, 1.0, 3.0], [-1.0, 1.0, -1.0]];
        let r = project(&door, &view).unwrap();
        assert!(r[2] - r[0] > 0.1, "{r:?}");
    }

    #[test]
    fn a_portal_to_the_side_is_off_screen() {
        let door = [[10.0, -1.0, 4.0], [12.0, -1.0, 4.0], [12.0, 1.0, 4.0], [10.0, 1.0, 4.0]];
        let r = project(&door, &view).unwrap();
        assert!(intersect(r, FULL).is_none());
    }

    #[test]
    fn distance_to_a_portal_is_measured_to_its_box() {
        let door = [[0.0, 0.0, 0.0], [0.0, 100.0, 0.0], [0.0, 100.0, 50.0], [0.0, 0.0, 50.0]];
        assert_eq!(distance_to_polygon(&door, [0.0, 50.0, 25.0]), 0.0);
        assert_eq!(distance_to_polygon(&door, [7.0, 50.0, 25.0]), 7.0);
        assert_eq!(distance_to_polygon(&door, [0.0, 50.0, 80.0]), 30.0);
    }

    #[test]
    fn rectangles_combine() {
        assert_eq!(intersect([0.0, 0.0, 1.0, 1.0], [0.5, 0.5, 2.0, 2.0]), Some([0.5, 0.5, 1.0, 1.0]));
        assert_eq!(intersect([0.0, 0.0, 1.0, 1.0], [2.0, 2.0, 3.0, 3.0]), None);
        assert!(contains(FULL, [0.0, 0.0, 0.5, 0.5]));
    }
}
