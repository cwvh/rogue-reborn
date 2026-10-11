//! Which rooms can be seen from a room, by walking the portals that are in view.
//!
//! The levels hold "impossible geometry": the inside of a plane is bigger than its hull,
//! so drawing every room at once shows the hull through the cabin and the cabin through
//! the hull. The original engine drew only the room the player was in plus whatever was
//! visible through the portals, and so does this.

use std::collections::{HashMap, HashSet};

use crate::collision::SolidRoom;
use crate::map::Map;

/// A screen rectangle in normalised device coordinates: min x, min y, max x, max y
pub type Rect = [f32; 4];

const FULL: Rect = [-1.0, -1.0, 1.0, 1.0];
/// How close the player must be to a portal to be standing in it
const TOUCH: f32 = 40.0;
/// How much of a room's floor plan must lie inside another's for it to stand in it
const NESTED: f32 = 0.6;
/// How many portals deep to look
const MAX_DEPTH: usize = 8;
/// The most portals to follow in one frame
const MAX_NODES: usize = 96;
/// Portals seen edge-on have no area; give them this much so they still count
const SLIVER: f32 = 0.005;

pub struct Visibility {
    /// For each room number, the indices of the portals that touch it
    by_room: HashMap<u32, Vec<usize>>,
    /// Rooms whose geometry spans most of the map: the outdoors, which surrounds and
    /// overlaps everything else
    outdoors: HashSet<u32>,
    /// The middle of each room (volume-weighted over its level boxes), to tell which
    /// side of a portal a room is on
    centres: HashMap<u32, [f32; 3]>,
    /// The number of each room, in the order of `map.rooms.rooms`
    numbers: Vec<Option<u32>>,
    /// Where each room is in `map.rooms.rooms`, by number
    index: HashMap<u32, usize>,
    /// Each portal's corners as listed, and in drawing order
    corners: Vec<Vec<[f32; 3]>>,
    polygons: Vec<Vec<[f32; 3]>>,
    /// The floor plan of each room, by number
    footprints: HashMap<u32, Vec<[f32; 4]>>,
    /// The objects of each room (indices into `map.geometries.objects`), by number
    objects: HashMap<u32, Vec<usize>>,
    /// The objects that belong to no room, and the room of each object
    unowned: Vec<usize>,
    object_rooms: Vec<Option<u32>>,
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

        // An object that covers at least half of the map in both directions is the outdoors
        let extent = |vertices: &[crate::map::Vertex]| {
            vertices.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |e, v| [e[0].min(v.x), e[1].min(v.z), e[2].max(v.x), e[3].max(v.z)])
        };
        let all: Vec<_> = map.geometries.objects.iter().flat_map(|o| o.vertices.iter().cloned()).collect();
        let whole = extent(&all);
        let (width, depth) = (whole[2] - whole[0], whole[3] - whole[1]);
        let outdoors = map.geometries.objects.iter()
            .filter(|o| {
                let e = extent(&o.vertices);
                e[2] - e[0] >= width * 0.5 && e[3] - e[1] >= depth * 0.5
            })
            .filter_map(|o| crate::map::object_room(&o.name))
            .collect();
        let mut centres = HashMap::new();
        for room in &map.rooms.rooms {
            let Ok(number) = room.section_name.parse::<u32>() else { continue };
            let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
            for b in room.sherman_levels.iter().flat_map(|l| &l.tm_with_aabb) {
                let (o, a) = (&b.tm.position, &b.aabb);
                let local = [(a[0] + a[3]) / 2.0, (a[1] + a[4]) / 2.0, (a[2] + a[5]) / 2.0];
                let (x, y, z) = (&b.tm.x_axis, &b.tm.y_axis, &b.tm.z_axis);
                let centre = [
                    o.x + x.x * local[0] + y.x * local[1] + z.x * local[2],
                    o.y + x.y * local[0] + y.y * local[1] + z.y * local[2],
                    o.z + x.z * local[0] + y.z * local[1] + z.z * local[2],
                ];
                let volume = ((a[3] - a[0]) * (a[4] - a[1]) * (a[5] - a[2])).abs().max(1.0);
                for i in 0..3 {
                    sum[i] += centre[i] * volume;
                }
                weight += volume;
            }
            if weight > 0.0 {
                centres.insert(number, [sum[0] / weight, sum[1] / weight, sum[2] / weight]);
            }
        }

        let numbers: Vec<Option<u32>> = map.rooms.rooms.iter().map(|r| r.section_name.parse().ok()).collect();
        let index: HashMap<u32, usize> = numbers.iter().enumerate().filter_map(|(i, n)| n.map(|n| (n, i))).collect();
        let corners: Vec<Vec<[f32; 3]>> = map.portals.portals.iter()
            .map(|p| p.coordinates.iter().map(|v| [v.x, v.y, v.z]).collect())
            .collect();
        let polygons = corners.iter().map(|c| ordered(c)).collect();
        let footprints = map.rooms.rooms.iter().zip(&numbers)
            .filter_map(|(r, n)| n.map(|n| (n, r.footprints())))
            .collect();
        let mut objects: HashMap<u32, Vec<usize>> = HashMap::new();
        let mut unowned = Vec::new();
        let mut object_rooms = Vec::with_capacity(map.geometries.objects.len());
        for (i, o) in map.geometries.objects.iter().enumerate() {
            let room = crate::map::object_room(&o.name).filter(|n| index.contains_key(n));
            match room {
                Some(n) => objects.entry(n).or_default().push(i),
                None => unowned.push(i),
            }
            object_rooms.push(room);
        }
        Self { by_room, outdoors, centres, numbers, index, corners, polygons, footprints, objects, unowned, object_rooms }
    }

    /// Whether the map has a room with this number
    pub fn is_room(&self, number: u32) -> bool {
        self.index.contains_key(&number)
    }

    /// The number of the room at this index in `map.rooms.rooms`
    pub fn number_of(&self, index: usize) -> Option<u32> {
        self.numbers.get(index).copied().flatten()
    }

    /// The objects (indices into `map.geometries.objects`) that belong to this room
    pub fn objects_of(&self, room: u32) -> &[usize] {
        self.objects.get(&room).map(Vec::as_slice).unwrap_or_default()
    }

    /// The objects that belong to no room
    pub fn unowned_objects(&self) -> &[usize] {
        &self.unowned
    }

    /// The room an object belongs to, if it belongs to one
    pub fn room_of_object(&self, object: usize) -> Option<u32> {
        self.object_rooms.get(object).copied().flatten()
    }

    /// The rooms the player is in, as room numbers: every room whose volume contains
    /// the point `p` (just above the feet), except the outdoors, which contains the whole
    /// level. Rooms overlap each other in places (a hall and the gallery above it), and
    /// each has to count or its floor or walls would vanish. Standing outside, or in no
    /// volume at all, gives the outdoors or the nearest room.
    pub fn player_rooms(&self, map: &Map, p: [f32; 3], prev: Option<usize>) -> Vec<u32> {
        let number = |i: usize| self.numbers[i];
        let deep = map.rooms.well_inside(p, 0.2, 100.0);
        let mut rooms: Vec<u32> = deep.iter().filter_map(|&i| number(i)).filter(|n| !self.outdoors.contains(n)).collect();
        if rooms.is_empty() {
            let inside = map.rooms.containing(p);
            rooms = inside.iter().filter_map(|&i| number(i)).filter(|n| !self.outdoors.contains(n)).collect();
            if rooms.is_empty() {
                rooms = inside.iter().filter_map(|&i| number(i)).collect();
            }
        }
        if rooms.is_empty() {
            rooms = map.rooms.locate(p, prev).and_then(number).into_iter().collect();
        }
        rooms.sort();
        rooms
    }

    /// The rooms that stop the player and bullets: the rooms drawing starts from, whole;
    /// the rooms their portals lead to, only beyond the portal (a neighbour overlaps the
    /// player's room, and its walls on this side are not really there); and the other
    /// rooms the player is well inside (`around`), whole, since they can hold the floor.
    ///
    /// Standing in a portal (within `TOUCH` of a `touch` point), the player's own room is
    /// cut at the portal too, as it overlaps the room beyond.
    pub fn solid_rooms(&self, map: &Map, starts: &[u32], around: &[u32], touch: &[[f32; 3]]) -> Vec<SolidRoom> {
        let mut rooms: Vec<SolidRoom> = starts.iter().map(|&r| SolidRoom::whole(r)).collect();
        let mut neighbours: Vec<SolidRoom> = Vec::new();
        for &start in starts {
            let Some(&centre) = self.centres.get(&start) else { continue };
            for &i in self.by_room.get(&start).map(Vec::as_slice).unwrap_or_default() {
                let portal = &map.portals.portals[i];
                let other = if portal.room == start { portal.opposite_room } else { portal.room };
                if starts.contains(&other) {
                    continue;
                }
                let corners = &self.corners[i];
                let touched = touch.iter().any(|&t| distance_to_polygon(corners, t) <= TOUCH);
                // Positive beyond the portal. The player is on their room's side, which says
                // which that is, even in the doorway (exactly in the plane there is no side,
                // and portal_plane gives none). The room's middle is only a fallback: in a big
                // or L-shaped room it can sit on the far side of a portal.
                let reference = touch.first().copied().unwrap_or(centre);
                let beyond = portal_plane(corners, reference);
                let entry = match neighbours.iter_mut().position(|n| n.room == other) {
                    Some(at) => at,
                    None => {
                        neighbours.push(SolidRoom { room: other, keep: Vec::new(), cut: Vec::new() });
                        neighbours.len() - 1
                    }
                };
                match beyond {
                    Some(p) => neighbours[entry].keep.push(p),
                    // No plane to cut by: the whole room counts
                    None => neighbours[entry] = SolidRoom::whole(other),
                }
                if let Some(p) = beyond {
                    if touched {
                        if let Some(r) = rooms.iter_mut().find(|r| r.room == start) {
                            r.cut.push([-p[0], -p[1], -p[2], -p[3]]);
                        }
                    }
                }
            }
        }
        rooms.extend(neighbours);
        // A room the player is inside but that has no portal to theirs counts only if
        // their room stands mostly within it, like a stairwell in a hall, which can hold the
        // floor beside the stairs. One that merely shares a wall's thickness does not.
        for &n in around {
            if rooms.iter().any(|r| r.room == n) {
                continue;
            }
            let Some(outer) = self.footprints.get(&n) else { continue };
            let holds = starts.iter().filter_map(|s| self.footprints.get(s)).any(|inner| crate::map::nested_fraction(inner, outer) >= NESTED);
            if holds {
                rooms.push(SolidRoom::whole(n));
            }
        }
        rooms
    }

    /// What to draw from `start`, a room the player is in: the room, and for each portal
    /// of it that is in view the room beyond, and so on. `clip` takes a map-space point
    /// to clip space (x, y, z, w) for the current view, and `eye` is where the camera is.
    ///
    /// The renderer draws each room only through its portal's polygon (stencil), and
    /// only beyond the portal's plane. A room overlapping the one in front of it can have
    /// walls past the portal, inside the room beyond, and those must not show.
    ///
    /// A portal within `TOUCH` units of one of the `touch` points (the player's feet and
    /// eyes) is one the player is standing in: both rooms are drawn whole, each cut at the
    /// portal's plane.
    ///
    /// `blocked` says, for each portal, whether it is shut (a closed door): nothing is seen
    /// through it.
    pub fn plan<'a>(&'a self, map: &Map, start: u32, touch: &[[f32; 3]], eye: [f32; 3], blocked: &[bool], clip: &dyn Fn([f32; 3]) -> [f32; 4]) -> PlanNode<'a> {
        let mut budget = MAX_NODES;
        let mut node = PlanNode { room: start, plane: None, polygon: &[], touched: false, children: Vec::new() };
        self.grow(map, &mut node, FULL, touch, eye, blocked, clip, &mut vec![start], &mut budget);
        node
    }

    #[allow(clippy::too_many_arguments)]
    fn grow<'a>(&'a self, map: &Map, node: &mut PlanNode<'a>, rect: Rect, touch: &[[f32; 3]], eye: [f32; 3], blocked: &[bool], clip: &dyn Fn([f32; 3]) -> [f32; 4], path: &mut Vec<u32>, budget: &mut usize) {
        if path.len() > MAX_DEPTH {
            return;
        }
        for &i in self.by_room.get(&node.room).map(Vec::as_slice).unwrap_or_default() {
            let portal = &map.portals.portals[i];
            let other = if portal.room == node.room { portal.opposite_room } else { portal.room };
            if path.contains(&other) || *budget == 0 || blocked.get(i).copied().unwrap_or(false) {
                continue;
            }
            let corners = &self.corners[i];
            let touched = touch.iter().any(|&p| distance_to_polygon(corners, p) <= TOUCH);
            let rect = if touched {
                Some(rect)
            } else {
                project(corners, clip).and_then(|r| intersect(r, rect))
            };
            let Some(rect) = rect else { continue };
            *budget -= 1;
            // The camera is on this room's side of the portal, in the doorway too (exactly in
            // its plane there is no side, and portal_plane gives none)
            let near_side = eye;
            let mut child = PlanNode {
                room: other,
                plane: portal_plane(corners, near_side),
                polygon: &self.polygons[i],
                touched,
                children: Vec::new(),
            };
            path.push(other);
            self.grow(map, &mut child, rect, touch, eye, blocked, clip, path, budget);
            path.pop();
            node.children.push(child);
        }
    }
}

/// A room to draw, and the rooms seen through its portals
#[derive(Clone, Debug)]
pub struct PlanNode<'a> {
    pub room: u32,
    /// The plane of the portal this room is seen through (a, b, c, d), with
    /// a*x + b*y + c*z + d >= 0 on the far side from the camera: the only side that
    /// belongs to the view. None for the room the player is in.
    pub plane: Option<[f32; 4]>,
    /// The portal's corners in drawing order, for masking the screen to its opening
    pub polygon: &'a [[f32; 3]],
    /// The player is standing in this portal: the two rooms are drawn whole, each cut at
    /// the portal's plane, since no polygon on screen describes a doorway the camera is in
    pub touched: bool,
    pub children: Vec<PlanNode<'a>>,
}

impl PlanNode<'_> {
    /// Every room in the plan, this one included
    pub fn rooms(&self, out: &mut Vec<u32>) {
        out.push(self.room);
        for c in &self.children {
            c.rooms(out);
        }
    }
}

/// A portal's corners in an order that goes around the outline, whatever order the file
/// lists them in, so they can be drawn as a polygon
pub fn ordered(corners: &[[f32; 3]]) -> Vec<[f32; 3]> {
    if corners.len() < 4 {
        return corners.to_vec();
    }
    let n = corners.len() as f32;
    let c = corners.iter().fold([0.0; 3], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n, a[2] + p[2] / n]);
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    // The outline's normal: the one from the first three corners that are not in a line
    let mut normal = [0.0; 3];
    for w in corners.windows(3) {
        let m = cross(sub(w[1], w[0]), sub(w[2], w[0]));
        if dot(m, m) > 1e-6 {
            normal = m;
            break;
        }
    }
    let u = sub(corners[0], c);
    let v = cross(normal, u);
    let mut out = corners.to_vec();
    out.sort_by(|a, b| {
        let angle = |p: &[f32; 3]| dot(sub(*p, c), v).atan2(dot(sub(*p, c), u));
        angle(a).total_cmp(&angle(b))
    });
    out
}

/// The plane of a portal, facing away from the camera at `eye`; None when the corners do
/// not make a plane or the camera is in it
pub fn portal_plane(corners: &[[f32; 3]], eye: [f32; 3]) -> Option<[f32; 4]> {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let origin = *corners.first()?;
    let mut normal = None;
    for w in corners.windows(2).skip(1) {
        let (u, v) = (sub(w[0], origin), sub(w[1], origin));
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let len = dot(n, n).sqrt();
        if len > 1e-3 {
            normal = Some([n[0] / len, n[1] / len, n[2] / len]);
            break;
        }
    }
    let n = normal?;
    let d = -dot(n, origin);
    let side = dot(n, eye) + d;
    if side.abs() < 0.5 {
        return None;
    }
    // Positive on the side away from the camera
    let flip = if side > 0.0 { -1.0 } else { 1.0 };
    Some([n[0] * flip, n[1] * flip, n[2] * flip, d * flip])
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

#[cfg(test)]
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
    fn a_portal_plane_faces_away_from_the_camera() {
        // A doorway in the plane x = 10
        let door = [[10.0, 0.0, 0.0], [10.0, 5.0, 0.0], [10.0, 5.0, 3.0], [10.0, 0.0, 3.0]];
        let side = |plane: [f32; 4], p: [f32; 3]| plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3];
        for eye in [[0.0, 2.0, 1.0], [20.0, 2.0, 1.0]] {
            let plane = portal_plane(&door, eye).unwrap();
            assert!(side(plane, eye) < 0.0, "the camera is on the near side");
            // The far side is the one the camera is not on
            assert!(side(plane, [if eye[0] < 10.0 { 15.0 } else { 5.0 }, 2.0, 1.0]) > 0.0);
        }
        // In the doorway: no plane to cut by
        assert!(portal_plane(&door, [10.0, 2.0, 1.0]).is_none());
    }

    #[test]
    fn corners_listed_out_of_order_are_put_in_order() {
        // A square given as a bow tie: (0,0) (1,1) (1,0) (0,1)
        let bowtie = [[0.0, 0.0, 5.0], [1.0, 1.0, 5.0], [1.0, 0.0, 5.0], [0.0, 1.0, 5.0]];
        let o = ordered(&bowtie);
        // Going around, each corner shares a coordinate with the next
        for i in 0..4 {
            let (a, b) = (o[i], o[(i + 1) % 4]);
            assert!((a[0] == b[0]) != (a[1] == b[1]), "{o:?}");
        }
    }

    // On rm11, rooms 119 and 121 overlap and are joined by a doorway at x = 29477. Standing in
    // 119, only the part of 121 past that doorway is really 121.
    #[test]
    fn a_neighbour_counts_only_beyond_the_portal() {
        let path = std::path::Path::new("data/map/rm11/rm11.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let map = crate::map::read(path).expect("rm11 loads");
        let vis = Visibility::new(&map);
        let feet = [29737.0, 32777.0, 40042.5];
        let eye = [feet[0], feet[1] + 155.0, feet[2]];
        let rooms = vis.solid_rooms(&map, &[119], &[], &[feet, eye]);
        let r121 = rooms.iter().find(|r| r.room == 121).expect("121 is next to 119");
        assert!(!r121.keep.is_empty());
        let counts = |p: [f32; 3]| r121.keep.iter().any(|k| k[0] * p[0] + k[1] * p[1] + k[2] * p[2] + k[3] >= 0.0);
        assert!(counts([29000.0, 32900.0, 39557.0]), "inside 121, past the doorway");
        assert!(!counts([29737.0, 32900.0, 40042.5]), "where the player stands, in 119");
        // Room 119 itself is whole
        assert!(rooms.iter().any(|r| r.room == 119 && r.keep.is_empty() && r.cut.is_empty()));
    }

    // On rm11 the door between rooms 119 and 121 is portal pd119_121. Shut, nothing is seen
    // through it; open, room 121 is.
    #[test]
    fn a_shut_door_hides_the_room_behind_it() {
        let path = std::path::Path::new("data/map/rm11/rm11.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let map = crate::map::read(path).expect("rm11 loads");
        let vis = Visibility::new(&map);
        let door = map.portals.portals.iter().position(|p| p.name == "pd119_121").expect("the door's portal");
        // A view in which every portal is on screen
        let everywhere = |_: [f32; 3]| [0.0, 0.0, 0.0, 1.0];
        let eye = [29737.0, 32932.0, 40042.5];
        let rooms_beside = |blocked: &[bool]| -> Vec<u32> {
            vis.plan(&map, 119, &[], eye, blocked, &everywhere).children.iter().map(|c| c.room).collect()
        };
        let mut blocked = vec![false; map.portals.portals.len()];
        assert!(rooms_beside(&blocked).contains(&121), "open, 121 is seen from 119");
        blocked[door] = true;
        let beside = rooms_beside(&blocked);
        assert!(!beside.contains(&121), "shut, it is not: {beside:?}");
        assert!(beside.contains(&120), "the other doorways are unaffected");
    }

    // On rm11 the courtyard (room 1, a shell around the whole building, whose middle is far
    // past the doorway) meets room 100 at the opening p100_01 in the plane z = 34237. Walking
    // in from the courtyard the player is within reach of the opening, and room 100's floor
    // just past it must stay solid.
    #[test]
    fn the_floor_beyond_a_doorway_stays_solid_when_the_room_is_big() {
        let path = std::path::Path::new("data/map/rm11/rm11.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let map = crate::map::read(path).expect("rm11 loads");
        let vis = Visibility::new(&map);
        let feet = [33113.0, 32677.0, 34208.0];
        let eye = [feet[0], feet[1] + 155.0, feet[2]];
        let rooms = vis.solid_rooms(&map, &[1], &[], &[feet, eye]);
        let r100 = rooms.iter().find(|r| r.room == 100).expect("100 is next to 1");
        let counts = |p: [f32; 3]| r100.keep.iter().any(|k| k[0] * p[0] + k[1] * p[1] + k[2] * p[2] + k[3] >= 0.0);
        assert!(counts([33113.0, 32667.0, 34238.0]), "the floor just past the opening");
        assert!(counts([33113.0, 32667.0, 35000.0]), "and further in");
        assert!(!counts([33113.0, 32667.0, 34000.0]), "but not the courtyard side");
    }

    #[test]
    fn rectangles_combine() {
        assert_eq!(intersect([0.0, 0.0, 1.0, 1.0], [0.5, 0.5, 2.0, 2.0]), Some([0.5, 0.5, 1.0, 1.0]));
        assert_eq!(intersect([0.0, 0.0, 1.0, 1.0], [2.0, 2.0, 3.0, 3.0]), None);
        assert!(contains(FULL, [0.0, 0.0, 0.5, 0.5]));
    }
}
