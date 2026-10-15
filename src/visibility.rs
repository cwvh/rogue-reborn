//! Which rooms can be seen from a room, by walking the portals that are in view.
//!
//! The levels hold "impossible geometry": the inside of a plane is bigger than its hull,
//! so drawing every room at once shows the hull through the cabin and the cabin through
//! the hull. The original engine drew only the room the player was in plus whatever was
//! visible through the portals, and so does this.

use std::collections::{HashMap, HashSet};

use crate::collision::{Cut, SolidRoom};
use crate::map::Map;

/// A screen rectangle in normalized device coordinates: min x, min y, max x, max y
pub type Rect = [f32; 4];

const FULL: Rect = [-1.0, -1.0, 1.0, 1.0];
/// How close the player must be to a portal's plane to be standing in it
const TOUCH: f32 = 40.0;
/// How far outside a portal's opening (in its plane) the player still stands in it
const OPENING_SLACK: f32 = 10.0;
/// How much of a room's floor plan must lie inside another's for it to stand in it
const NESTED: f32 = 0.6;
/// How far in from a room's volume the doorway cut starts. The walls of the room around it
/// meet the volume's edge to within a couple of units, and the stray surfaces of an
/// overlapping room reach it too; this keeps the first and cuts nearly all of the second.
const WALL: f32 = 4.0;
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
    centers: HashMap<u32, [f32; 3]>,
    /// The number of each room, in the order of `map.rooms.rooms`
    numbers: Vec<Option<u32>>,
    /// Where each room is in `map.rooms.rooms`, by number
    index: HashMap<u32, usize>,
    /// Each portal's corners as listed, and in drawing order
    corners: Vec<Vec<[f32; 3]>>,
    polygons: Vec<Vec<[f32; 3]>>,
    /// For each portal, a point on the side of each of its two rooms, when it can be told
    /// which side is which: (room, point) for the room and the room opposite
    sides: Vec<[(u32, Option<[f32; 3]>); 2]>,
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
        let mut centers = HashMap::new();
        for room in &map.rooms.rooms {
            let Ok(number) = room.section_name.parse::<u32>() else { continue };
            let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
            for b in room.sherman_levels.iter().flat_map(|l| &l.tm_with_aabb) {
                let (o, a) = (&b.tm.position, &b.aabb);
                let local = [(a[0] + a[3]) / 2.0, (a[1] + a[4]) / 2.0, (a[2] + a[5]) / 2.0];
                let (x, y, z) = (&b.tm.x_axis, &b.tm.y_axis, &b.tm.z_axis);
                let center = [
                    o.x + x.x * local[0] + y.x * local[1] + z.x * local[2],
                    o.y + x.y * local[0] + y.y * local[1] + z.y * local[2],
                    o.z + x.z * local[0] + y.z * local[1] + z.z * local[2],
                ];
                let volume = ((a[3] - a[0]) * (a[4] - a[1]) * (a[5] - a[2])).abs().max(1.0);
                for i in 0..3 {
                    sum[i] += center[i] * volume;
                }
                weight += volume;
            }
            if weight > 0.0 {
                centers.insert(number, [sum[0] / weight, sum[1] / weight, sum[2] / weight]);
            }
        }

        let numbers: Vec<Option<u32>> = map.rooms.rooms.iter().map(|r| r.section_name.parse().ok()).collect();
        let index: HashMap<u32, usize> = numbers.iter().enumerate().filter_map(|(i, n)| n.map(|n| (n, i))).collect();
        let corners: Vec<Vec<[f32; 3]>> = map.portals.portals.iter()
            .map(|p| p.coordinates.iter().map(|v| [v.x, v.y, v.z]).collect())
            .collect();
        let polygons = corners.iter().map(|c| ordered(c)).collect();
        let sides = map.portals.portals.iter().zip(&corners).map(|(p, c)| owned_sides(map, &index, c, p.room, p.opposite_room)).collect();
        let footprints: HashMap<u32, Vec<[f32; 4]>> = map.rooms.rooms.iter().zip(&numbers)
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
        Self { by_room, outdoors, centers, numbers, index, corners, polygons, sides, footprints, objects, unowned, object_rooms }
    }

    /// The volume of a room that meets a portal, floor plan only (min x, min z, max x,
    /// max z), for cutting `cut`, the room it overlaps there. A room can hold volumes that
    /// overlap others elsewhere (an upper floor over the whole hall below), so the one at the
    /// doorway is the only one that says where the two rooms overlap there.
    ///
    /// An edge is pulled in by a wall's thickness where `cut`'s own walls meet it, so they
    /// stay. Elsewhere the stray surfaces of `cut` reach the edge, and stopping short of it
    /// would leave a slice of them standing inside the room, in front of its wall.
    fn bounds_at(&self, room: u32, cut: u32, portal: usize) -> Option<[f32; 4]> {
        let corners = &self.corners[portal];
        let span = |axis: usize| corners.iter().map(|c| c[axis]).fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
        let (x, z) = (span(0), span(2));
        let gap = |lo: f32, hi: f32, (a, b): (f32, f32)| (a - hi).max(lo - b).max(0.0);
        let distance = |f: &[f32; 4]| gap(f[0], f[2], x).hypot(gap(f[1], f[3], z));
        let main = self.footprints.get(&room)?.iter().min_by(|a, b| distance(a).total_cmp(&distance(b)))?;
        let walls = self.footprints.get(&cut).map(Vec::as_slice).unwrap_or_default();
        // The coordinate of an edge, moved in unless a wall of `cut` stands there
        let edge = |at: f32, axes: [usize; 2], inward: f32| {
            if walls.iter().any(|f| axes.iter().any(|&a| (f[a] - at).abs() <= WALL)) {
                at + inward * WALL
            } else {
                at
            }
        };
        let (x0, x1) = (edge(main[0], [0, 2], 1.0), edge(main[2], [0, 2], -1.0));
        let (z0, z1) = (edge(main[1], [1, 3], 1.0), edge(main[3], [1, 3], -1.0));
        // A volume too small to pull in is used as it is
        let fits = |lo: f32, hi: f32| hi - lo > 0.0;
        Some(if fits(x0, x1) && fits(z0, z1) { [x0, z0, x1, z1] } else { *main })
    }

    /// The volumes of `room` the player is in (a `touch` point is inside) that reach through
    /// `other`, pulled in by a wall's thickness (min x, y, z, max x, y, z). On rm11 the stairs
    /// of room 121 climb through the back wall and the ceiling of room 119, and there 119's
    /// surfaces are not really there. A volume that holds most of `other` where they meet is
    /// only a rough box around both, as a hall's is around the room in its corner: `other`'s
    /// walls inside it are real.
    fn reaching_through(&self, map: &Map, room: u32, other: u32, touch: &[[f32; 3]]) -> Vec<[f32; 6]> {
        let (Some(&mine), Some(&theirs)) = (self.index.get(&room), self.index.get(&other)) else { return Vec::new() };
        let volumes = |i: usize| map.rooms.rooms[i].sherman_levels.iter().flat_map(|l| &l.tm_with_aabb);
        let overlap = |a: &[f32; 6], b: &[f32; 6]| (0..3).all(|i| a[i] < b[i + 3] && b[i] < a[i + 3]);
        let plan = |b: &[f32; 6]| [b[0], b[2], b[3], b[5]];
        let mut holes = Vec::new();
        for v in volumes(mine).filter(|v| touch.iter().any(|&t| v.outside(t) == 0.0)) {
            let b = v.bounds();
            let met: Vec<[f32; 6]> = volumes(theirs).map(|o| o.bounds()).filter(|o| overlap(o, &b)).collect();
            let encloses = met.iter().any(|o| crate::map::nested_fraction(&[plan(o)], &[plan(&b)]) >= 0.5);
            if met.is_empty() || encloses {
                continue;
            }
            holes.push([b[0] + WALL, b[1] + WALL, b[2] + WALL, b[3] - WALL, b[4] - WALL, b[5] - WALL]);
        }
        holes
    }

    /// A point just inside `room`'s side of a portal, if the portal's sides are known. It says
    /// which way the room lies from the portal wherever the camera or the player is: they can
    /// stand on one side while the floor under them, and so the room drawing starts from,
    /// already belongs to the other.
    fn side_point(&self, portal: usize, room: u32) -> Option<[f32; 3]> {
        self.sides.get(portal)?.iter().find(|(r, _)| *r == room).and_then(|(_, p)| *p)
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
    /// the rooms their portals lead to, only beyond the portal (a neighbor overlaps the
    /// player's room, and its walls on this side are not really there); and the other
    /// rooms the player is well inside (`around`), whole, since they can hold the floor.
    ///
    /// Standing in a portal (a `touch` point in its opening, see `stands_in`), the player's own room is
    /// cut at the portal too, as it overlaps the room beyond.
    pub fn solid_rooms(&self, map: &Map, starts: &[u32], around: &[u32], touch: &[[f32; 3]]) -> Vec<SolidRoom> {
        let mut rooms: Vec<SolidRoom> = starts.iter().map(|&r| SolidRoom::whole(r)).collect();
        let mut neighbors: Vec<SolidRoom> = Vec::new();
        for &start in starts {
            let Some(&center) = self.centers.get(&start) else { continue };
            for &i in self.by_room.get(&start).map(Vec::as_slice).unwrap_or_default() {
                let portal = &map.portals.portals[i];
                let other = if portal.room == start { portal.opposite_room } else { portal.room };
                if starts.contains(&other) {
                    continue;
                }
                let corners = &self.corners[i];
                let touched = stands_in(corners, touch);
                // Positive beyond the portal. The player is on their room's side, which says
                // which that is, even in the doorway (exactly in the plane there is no side,
                // and portal_plane gives none). The room's middle is only a fallback: in a big
                // or L-shaped room it can sit on the far side of a portal.
                let reference = self.side_point(i, start).or(touch.first().copied()).unwrap_or(center);
                let beyond = portal_plane(corners, reference);
                let entry = match neighbors.iter_mut().position(|n| n.room == other) {
                    Some(at) => at,
                    None => {
                        neighbors.push(SolidRoom { room: other, keep: Vec::new(), cut: Vec::new(), holes: Vec::new() });
                        neighbors.len() - 1
                    }
                };
                match beyond {
                    Some(p) => neighbors[entry].keep.push(p),
                    // No plane to cut by: the whole room counts
                    None => neighbors[entry] = SolidRoom::whole(other),
                }
                if let Some(p) = beyond {
                    if touched {
                        if let Some(r) = rooms.iter_mut().find(|r| r.room == start) {
                            // Only where it overlaps the room beyond
                            r.cut.push(Cut { plane: [-p[0], -p[1], -p[2], -p[3]], within: self.bounds_at(other, start, i) });
                        }
                    }
                }
            }
        }
        for &start in starts {
            for n in &mut neighbors {
                n.holes.extend(self.reaching_through(map, start, n.room, touch));
            }
        }
        rooms.extend(neighbors);
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

    /// The drawing before `rect_plan` (`--portal-paths`): what to draw from `start`, a room the
    /// player is in: the room, and for each portal of it that is in view the room beyond, and
    /// so on, once for each path through portals. `clip` takes a map-space point
    /// to clip space (x, y, z, w) for the current view, and `eye` is where the camera is.
    ///
    /// The renderer draws each room only through its portal's polygon (stencil), and
    /// only beyond the portal's plane. A room overlapping the one in front of it can have
    /// walls past the portal, inside the room beyond, and those must not show.
    ///
    /// A portal with one of the `touch` points in its opening (the player's feet and
    /// eyes) is one the player is standing in: both rooms are drawn whole, each cut at the
    /// portal's plane.
    ///
    /// `blocked` says, for each portal, whether it is shut (a closed door): nothing is seen
    /// through it.
    pub fn plan<'a>(&'a self, map: &Map, start: u32, touch: &[[f32; 3]], eye: [f32; 3], blocked: &[bool], clip: &dyn Fn([f32; 3]) -> [f32; 4]) -> PlanNode<'a> {
        let mut budget = MAX_NODES;
        let mut node = PlanNode { room: start, plane: None, polygon: &[], touched: false, cut_in_self: None, cut_in_parent: None, children: Vec::new() };
        self.grow(map, &mut node, FULL, touch, eye, blocked, clip, &mut vec![start], &mut budget);
        node
    }

    /// Every room seen from the `starts`, each once, nearest first (by portals), with the part
    /// of the screen it may draw in: the bounding rectangle of all the portals it is seen
    /// through, each cut to the rectangle of the room in front. A room is drawn whole within
    /// it, not cut at its portals' planes or masked to their exact openings, and the depth
    /// test sorts the rooms out. The original engine draws this way: on rm11 it shows the
    /// hall's wall that stands in 120's doorway, seen from the hall, and so does this.
    pub fn rect_plan(&self, map: &Map, starts: &[u32], touch: &[[f32; 3]], eye: [f32; 3], blocked: &[bool], clip: &dyn Fn([f32; 3]) -> [f32; 4]) -> Vec<RectRoom> {
        // Each room's rectangle only grows, but it can grow by small steps many times
        const MAX_STEPS: usize = 10_000;
        let mut out: Vec<RectRoom> = starts.iter().map(|&room| RectRoom { room, rect: FULL, doorways: Vec::new() }).collect();
        // What each room can see through: its portals' rectangles cut to the rooms' in front.
        // The rectangle a room is drawn in is its portals' whole rectangles: on rm08 the shed's
        // room (201) has no portal into the sky (501), and the crane's tower, seen over the
        // shed, rose past the sky portals' cut rectangles.
        let mut reach: Vec<Rect> = vec![FULL; starts.len()];
        let mut at: HashMap<u32, usize> = starts.iter().enumerate().map(|(i, &s)| (s, i)).collect();
        let mut queue: std::collections::VecDeque<u32> = starts.iter().copied().collect();
        let mut steps = 0;
        while let Some(room) = queue.pop_front() {
            steps += 1;
            if steps > MAX_STEPS {
                break;
            }
            let rect = reach[at[&room]];
            for &i in self.by_room.get(&room).map(Vec::as_slice).unwrap_or_default() {
                if blocked.get(i).copied().unwrap_or(false) {
                    continue;
                }
                let portal = &map.portals.portals[i];
                let other = if portal.room == room { portal.opposite_room } else { portal.room };
                let corners = &self.corners[i];
                let touched = stands_in(corners, touch);
                let projected = if touched { Some(rect) } else { project(corners, clip) };
                let Some((whole, seen)) = projected.and_then(|r| Some((intersect(r, FULL)?, intersect(r, rect)?))) else { continue };
                let entry = *at.entry(other).or_insert_with(|| {
                    out.push(RectRoom { room: other, rect: whole, doorways: Vec::new() });
                    reach.push(seen);
                    queue.push_back(other);
                    out.len() - 1
                });
                let r = &mut out[entry];
                if !contains(r.rect, whole) {
                    r.rect = [r.rect[0].min(whole[0]), r.rect[1].min(whole[1]), r.rect[2].max(whole[2]), r.rect[3].max(whole[3])];
                }
                if !contains(reach[entry], seen) {
                    let s = reach[entry];
                    reach[entry] = [s[0].min(seen[0]), s[1].min(seen[1]), s[2].max(seen[2]), s[3].max(seen[3])];
                    queue.push_back(other);
                }
                // Only an opening the camera looks into the room through: from the wrong side
                // (rm08's 203 is L-shaped, and its landing is on 521's side of pt203_521), the
                // room's things past the portal are behind the camera's own room
                let into = self.side_point(i, other).zip(portal_plane(corners, eye))
                    .is_some_and(|(p, plane)| plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3] > 0.0);
                if into && !touched && !starts.contains(&other) && starts.contains(&room) {
                    if let Some(b) = self.bounds_at(other, room, i).filter(|_| !r.doorways.iter().any(|d| d.0 == i)) {
                        r.doorways.push((i, b));
                    }
                }
            }
        }
        out
    }

    /// A portal's corners in drawing order
    pub fn polygon(&self, portal: usize) -> &[[f32; 3]] {
        &self.polygons[portal]
    }

    /// The room `start` and the rooms of the doorways the player stands in, as `plan` gives
    /// them (each cut at the doorway where they overlap), without the rooms seen through other
    /// portals: those are `rect_plan`'s
    pub fn touched_plan<'a>(&'a self, map: &Map, start: u32, touch: &[[f32; 3]], eye: [f32; 3], blocked: &[bool]) -> PlanNode<'a> {
        let mut node = PlanNode { room: start, plane: None, polygon: &[], touched: false, cut_in_self: None, cut_in_parent: None, children: Vec::new() };
        self.grow_touched(map, &mut node, touch, eye, blocked, &mut vec![start]);
        node
    }

    fn grow_touched<'a>(&'a self, map: &Map, node: &mut PlanNode<'a>, touch: &[[f32; 3]], eye: [f32; 3], blocked: &[bool], path: &mut Vec<u32>) {
        for &i in self.by_room.get(&node.room).map(Vec::as_slice).unwrap_or_default() {
            let portal = &map.portals.portals[i];
            let other = if portal.room == node.room { portal.opposite_room } else { portal.room };
            let corners = &self.corners[i];
            if path.contains(&other) || blocked.get(i).copied().unwrap_or(false) || !stands_in(corners, touch) {
                continue;
            }
            let near_side = self.side_point(i, node.room).unwrap_or(eye);
            let mut child = PlanNode {
                room: other,
                plane: portal_plane(corners, near_side),
                polygon: &self.polygons[i],
                touched: true,
                cut_in_self: self.bounds_at(node.room, other, i),
                cut_in_parent: self.bounds_at(other, node.room, i),
                children: Vec::new(),
            };
            path.push(other);
            self.grow_touched(map, &mut child, touch, eye, blocked, path);
            path.pop();
            node.children.push(child);
        }
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
            let touched = stands_in(corners, touch);
            let rect = if touched {
                Some(rect)
            } else {
                project(corners, clip).and_then(|r| intersect(r, rect))
            };
            let Some(rect) = rect else { continue };
            *budget -= 1;
            // The camera is on this room's side of the portal, in the doorway too (exactly in
            // its plane there is no side, and portal_plane gives none)
            let near_side = self.side_point(i, node.room).unwrap_or(eye);
            let mut child = PlanNode {
                room: other,
                plane: portal_plane(corners, near_side),
                polygon: &self.polygons[i],
                touched,
                // Each room's cut is limited to where it overlaps the other
                cut_in_self: if touched { self.bounds_at(node.room, other, i) } else { None },
                cut_in_parent: if touched { self.bounds_at(other, node.room, i) } else { None },
                children: Vec::new(),
            };
            path.push(other);
            self.grow(map, &mut child, rect, touch, eye, blocked, clip, path, budget);
            path.pop();
            node.children.push(child);
        }
    }
}

/// A room to draw whole, within `rect`
#[derive(Clone, Debug)]
pub struct RectRoom {
    pub room: u32,
    pub rect: Rect,
    /// The portals of the player's rooms it is seen through, from its known far side, except
    /// those the player stands in, each with this room's floor plan there (min x, min z, max x,
    /// max z; see bounds_at). The original engine hides the player's room inside it, seen
    /// through the portal: on rm11, 121's stairs inside 119, seen from the hall.
    pub doorways: Vec<(usize, [f32; 4])>,
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
    /// For a touched portal: the box (min x, min z, max x, max z) this room is cut within,
    /// which is where it overlaps the room it was reached from...
    pub cut_in_self: Option<[f32; 4]>,
    /// ...and the box the room it was reached from is cut within, where it overlaps this one
    pub cut_in_parent: Option<[f32; 4]>,
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

/// The unit normal of a portal's plane, from the first three corners that are not in a line
fn plane_normal(corners: &[[f32; 3]]) -> Option<[f32; 3]> {
    let origin = *corners.first()?;
    for w in corners.windows(2).skip(1) {
        let (u, v) = ([w[0][0] - origin[0], w[0][1] - origin[1], w[0][2] - origin[2]], [w[1][0] - origin[0], w[1][1] - origin[1], w[1][2] - origin[2]]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len > 1e-3 {
            return Some([n[0] / len, n[1] / len, n[2] / len]);
        }
    }
    None
}

/// How far from a portal's plane to look for what room is on each side
const PROBES: [f32; 3] = [20.0, 40.0, 80.0];
/// How far from the plane the point standing for a room's side is
const SIDE_DISTANCE: f32 = 20.0;

/// Which side of a portal belongs to each of its rooms: a point on that side. Rooms overlap,
/// so a point near the portal is often in both; but a stairwell's volume ends at the portal's
/// plane and the hall's floor goes on past it, so a point on one side is in the hall alone.
/// Each point in one room only is a vote for that room's side. None when the votes do not say.
fn owned_sides(map: &Map, index: &HashMap<u32, usize>, corners: &[[f32; 3]], a: u32, b: u32) -> [(u32, Option<[f32; 3]>); 2] {
    let unknown = [(a, None), (b, None)];
    let (Some(n), Some(&ia), Some(&ib)) = (plane_normal(corners), index.get(&a), index.get(&b)) else { return unknown };
    let lo = |axis: usize| corners.iter().map(|c| c[axis]).fold(f32::MAX, f32::min);
    let hi = |axis: usize| corners.iter().map(|c| c[axis]).fold(f32::MIN, f32::max);
    let center = [(lo(0) + hi(0)) / 2.0, (lo(1) + hi(1)) / 2.0, (lo(2) + hi(2)) / 2.0];
    // A doorway's volumes are thin floors and boxes: look low in it, where the floor is
    let at = |s: f32, d: f32| {
        let mut p = [center[0] + s * d * n[0], center[1] + s * d * n[1], center[2] + s * d * n[2]];
        if n[1].abs() < 0.5 {
            p[1] = lo(1) + 20.0;
        }
        p
    };
    let (mut score_a, mut score_b) = (0.0f32, 0.0f32);
    for s in [1.0, -1.0] {
        for d in PROBES {
            let p = at(s, d);
            match (map.rooms.rooms[ia].contains(p, 50.0), map.rooms.rooms[ib].contains(p, 50.0)) {
                (true, false) => score_a += s,
                (false, true) => score_b += s,
                _ => {}
            }
        }
    }
    // The two rooms are on opposite sides: one that is sure gives the other's side too
    let side = match (score_a.partial_cmp(&0.0), score_b.partial_cmp(&0.0)) {
        (Some(x), Some(y)) if x == y && x != std::cmp::Ordering::Equal => return unknown,
        (Some(std::cmp::Ordering::Greater), _) | (_, Some(std::cmp::Ordering::Less)) => 1.0,
        (Some(std::cmp::Ordering::Less), _) | (_, Some(std::cmp::Ordering::Greater)) => -1.0,
        _ => match sides_by_volumes(map, ia, ib, center, n) {
            Some(side) => side,
            None => return unknown,
        },
    };
    [(a, Some(at(side, SIDE_DISTANCE))), (b, Some(at(-side, SIDE_DISTANCE)))]
}

/// Where every point near a portal is in both rooms, as in rm11's courtyard, where room 5's
/// ground reaches 1500 units past its portal into room 6: the side of each room's volume at
/// the portal (the nearest, and the smallest of those it is inside), by where its middle is.
/// 1.0 when room `ia` is on the side `n` points to, -1.0 when it is on the other side, and
/// None unless the two middles are on opposite sides.
fn sides_by_volumes(map: &Map, ia: usize, ib: usize, center: [f32; 3], n: [f32; 3]) -> Option<f32> {
    let middle_side = |i: usize| {
        let at = map.rooms.rooms[i].sherman_levels.iter().flat_map(|l| &l.tm_with_aabb)
            .map(|v| (v.outside(center), v.bounds()))
            .min_by(|(d, b), (e, c)| d.total_cmp(e).then(size(b).total_cmp(&size(c))))?.1;
        let middle = [(at[0] + at[3]) / 2.0, (at[1] + at[4]) / 2.0, (at[2] + at[5]) / 2.0];
        Some((0..3).map(|k| (middle[k] - center[k]) * n[k]).sum::<f32>())
    };
    let (sa, sb) = (middle_side(ia)?, middle_side(ib)?);
    (sa * sb < 0.0).then(|| sa.signum())
}

/// The volume of a box (min x, y, z, max x, y, z)
fn size(b: &[f32; 6]) -> f32 {
    (b[3] - b[0]) * (b[4] - b[1]) * (b[5] - b[2])
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

/// Whether one of the `touch` points (the player's feet and eyes) stands in a portal: within
/// `TOUCH` of its plane, and within its opening. Near its edge is not in it: on rm02's
/// stairwell, the feet on the top landing are 30 units above the top of the doorway below.
fn stands_in(corners: &[[f32; 3]], touch: &[[f32; 3]]) -> bool {
    let Some(n) = plane_normal(corners) else {
        return touch.iter().any(|&p| distance_to_polygon(corners, p) <= TOUCH);
    };
    let c = corners[0];
    touch.iter().any(|&p| {
        let off = (p[0] - c[0]) * n[0] + (p[1] - c[1]) * n[1] + (p[2] - c[2]) * n[2];
        let in_plane = [p[0] - off * n[0], p[1] - off * n[1], p[2] - off * n[2]];
        off.abs() <= TOUCH && distance_to_polygon(corners, in_plane) <= OPENING_SLACK
    })
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
    fn standing_in_a_portal_means_in_its_opening() {
        let door = [[0.0, 0.0, 0.0], [0.0, 100.0, 0.0], [0.0, 100.0, 50.0], [0.0, 0.0, 50.0]];
        assert!(stands_in(&door, &[[30.0, 10.0, 25.0]]), "in the doorway, 30 units from its plane");
        assert!(!stands_in(&door, &[[50.0, 10.0, 25.0]]), "too far from its plane");
        // rm02's stairwell: on the landing above, close to the plane but over the doorway
        assert!(!stands_in(&door, &[[-20.0, 130.0, 25.0]]), "30 units above the doorway");
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
    fn a_neighbor_counts_only_beyond_the_portal() {
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

    // On rm11 the stairs of room 121 stand inside room 119. Standing in the doorway, 121 is
    // cut where it overlaps 119, out to 119's walls: stopping a few units short left a slice
    // of the stairs standing in front of the wall.
    #[test]
    fn the_overlapping_room_is_cut_out_to_the_walls() {
        let path = std::path::Path::new("data/map/rm11/rm11.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let map = crate::map::read(path).expect("rm11 loads");
        let vis = Visibility::new(&map);
        let door = map.portals.portals.iter().position(|p| p.name == "pd119_121").expect("the door's portal");
        let feet = [29491.9, 32767.0, 39544.6];
        let eye = [feet[0], feet[1] + 165.0, feet[2]];
        let everywhere = |_: [f32; 3]| [0.0, 0.0, 0.0, 1.0];
        let blocked = vec![false; map.portals.portals.len()];
        let plan = vis.plan(&map, 119, &[feet, eye], eye, &blocked, &everywhere);
        let node = plan.children.iter().find(|c| c.room == 121).expect("121 is seen from the doorway");
        let b = node.cut_in_self.expect("121 is cut within 119");
        // 119 spans x 29477..29957 and z 39317..40117; 121's own walls are not at its far edges
        assert_eq!(b, [29477.0, 39317.0, 29957.0, 40117.0], "portal {door}");
    }

    // On rm11 the stairs volume of room 121 reaches through the back wall and the ceiling of
    // room 119: on the stairs, 119 does not count there. 121's ground floor volume is a
    // rough box around all of 119, and in the hall 119's walls are real.
    #[test]
    fn a_neighbor_does_not_count_where_the_players_room_reaches_through_it() {
        let path = std::path::Path::new("data/map/rm11/rm11.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let map = crate::map::read(path).expect("rm11 loads");
        let vis = Visibility::new(&map);
        let holes = |feet: [f32; 3]| {
            let rooms = vis.solid_rooms(&map, &[121], &[], &[feet, [feet[0], feet[1] + 155.0, feet[2]]]);
            rooms.iter().find(|r| r.room == 119).expect("119 is next to 121").holes.clone()
        };
        let on_stairs = holes([29863.0, 33057.0, 40087.0]);
        assert_eq!(on_stairs.len(), 1, "{on_stairs:?}");
        let h = on_stairs[0];
        let inside = |p: [f32; 3]| (0..3).all(|i| p[i] > h[i] && p[i] < h[i + 3]);
        assert!(inside([29863.0, 33080.0, 40117.0]), "119's back wall across the stairs");
        assert!(inside([29901.0, 33127.0, 40031.0]), "119's ceiling over the stairs");
        assert!(holes([29300.0, 32777.0, 39557.0]).is_empty(), "in the hall");
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

    // On rm11 the hall (room 121) has a wall at x = 30017 that stands inside room 120, 60 units
    // past the doorway from 119 (p120_119, at x = 29957). From the hall's doorway into 119 it
    // covers part of that far doorway, and the original engine shows it there (BUGS.txt).
    #[test]
    fn the_halls_wall_stands_inside_the_room_past_119() {
        let path = std::path::Path::new("data/map/rm11/rm11.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let map = crate::map::read(path).expect("rm11 loads");
        let vis = Visibility::new(&map);
        let p = [30017.0, 32866.5, 40051.4];
        // Whether a point (z, y) is inside a triangle in the plane x = 30017
        let inside = |q: [f32; 2], t: [[f32; 2]; 3]| {
            let side = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]);
            let s = [side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0])];
            s.iter().all(|&v| v >= 0.0) || s.iter().all(|&v| v <= 0.0)
        };
        let wall = vis.objects_of(121).iter().map(|&i| &map.geometries.objects[i]).any(|o| {
            o.object_datas.iter().flat_map(|od| &od.faces.face_indices).any(|&(a, b, c)| {
                let v = [a, b, c].map(|k| &o.vertices[k as usize]);
                v.iter().all(|v| (v.x - p[0]).abs() < 0.5) && inside([p[2], p[1]], v.map(|v| [v.z, v.y]))
            })
        });
        assert!(wall, "121 has a wall face there");
        let rooms: Vec<u32> = map.rooms.containing(p).into_iter().filter_map(|i| vis.number_of(i)).collect();
        assert!(rooms.contains(&120) && !rooms.contains(&119), "inside 120: {rooms:?}");
    }

    // Standing in the hall's doorway into 119 on rm11, room 120 past 119 is drawn once, and
    // not in front of what is already drawn inside the opening of p120_119, which is not one
    // of the hall's doorways: the hall's wall standing inside 120 shows there, as in the
    // original engine
    #[test]
    fn the_room_past_the_next_doorway_is_left_to_the_depth_test() {
        let path = std::path::Path::new("data/map/rm11/rm11.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let map = crate::map::read(path).expect("rm11 loads");
        let vis = Visibility::new(&map);
        let far_door = map.portals.portals.iter().position(|p| p.name == "p120_119").expect("the far doorway");
        let eye = [29467.9, 32932.0, 39542.9];
        let feet = [eye[0], eye[1] - 165.0, eye[2]];
        let everywhere = |_: [f32; 3]| [0.0, 0.0, 0.0, 1.0];
        let blocked = vec![false; map.portals.portals.len()];
        let plan = vis.rect_plan(&map, &[121], &[feet, eye], eye, &blocked, &everywhere);
        let room120: Vec<&RectRoom> = plan.iter().filter(|r| r.room == 120).collect();
        assert_eq!(room120.len(), 1, "each room once");
        assert!(!room120[0].doorways.iter().any(|d| d.0 == far_door), "{:?}", room120[0].doorways);
    }

    // From rm11's hall, 77 units from the doorway into 119 (too
    // far for the doorway cut), the original engine hides the hall's stairs that stand inside
    // 119, seen through that doorway, but not the hall's wall inside 120, one room further:
    // 119 is drawn in front inside the doorway's opening, within its floor plan there
    #[test]
    fn the_room_past_the_players_doorway_is_drawn_in_front_within_its_floor_plan() {
        let path = std::path::Path::new("data/map/rm11/rm11.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let map = crate::map::read(path).expect("rm11 loads");
        let vis = Visibility::new(&map);
        let door = map.portals.portals.iter().position(|p| p.name == "pd119_121").expect("the doorway");
        let eye = [29400.5, 32932.0, 39432.1];
        let feet = [eye[0], eye[1] - 165.0, eye[2]];
        let everywhere = |_: [f32; 3]| [0.0, 0.0, 0.0, 1.0];
        let blocked = vec![false; map.portals.portals.len()];
        let plan = vis.rect_plan(&map, &[121], &[feet, eye], eye, &blocked, &everywhere);
        let room119 = plan.iter().find(|r| r.room == 119).expect("119 is seen from the hall");
        let &(_, b) = room119.doorways.iter().find(|d| d.0 == door).expect("through the hall's doorway");
        let inside = |x: f32, z: f32| b[0] <= x && x <= b[2] && b[1] <= z && z <= b[3];
        assert!(inside(29863.0, 40087.0), "the hall's stairs: {b:?}");
        assert!(!inside(30017.0, 40051.4), "the hall's wall inside 120: {b:?}");
    }

    // rm08's alleys are open cells, nearly every boundary a portal,
    // and the old plan ran out of its budget before it reached the alley (203) beside the player
    // in room 201. And standing on 203's landing, on room 521's side of pt203_521, that portal is
    // seen from behind: 521 must not be drawn in front inside it.
    #[test]
    fn every_room_in_view_is_planned_once() {
        let path = std::path::Path::new("data/map/rm08/rm08.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let map = crate::map::read(path).expect("rm08 loads");
        let vis = Visibility::new(&map);
        let everywhere = |_: [f32; 3]| [0.0, 0.0, 0.0, 1.0];
        let blocked = vec![false; map.portals.portals.len()];
        let body = |eye: [f32; 3]| [[eye[0], eye[1] - 165.0, eye[2]], eye];

        let eye = [34202.9, 32567.0, 29624.9];
        let plan = vis.rect_plan(&map, &[201], &body(eye), eye, &blocked, &everywhere);
        let mut rooms: Vec<u32> = plan.iter().map(|r| r.room).collect();
        assert!(rooms.contains(&203), "the alley beside the player: {rooms:?}");
        rooms.sort();
        let all = rooms.len();
        rooms.dedup();
        assert_eq!(rooms.len(), all, "each room once");

        let behind = map.portals.portals.iter().position(|p| p.name == "pt203_521" && p.coordinates.iter().all(|v| v.z == 30117.0)).expect("the canal side's portal");
        let eye = [34352.5, 32567.0, 29911.0];
        let plan = vis.rect_plan(&map, &[203], &body(eye), eye, &blocked, &everywhere);
        let room521 = plan.iter().find(|r| r.room == 521).expect("521 is in view");
        assert!(!room521.doorways.iter().any(|d| d.0 == behind), "{:?}", room521.doorways);
    }

    // The cut a room gets where the player stands in a doorway is limited to where it overlaps
    // the room beyond: rm11's courtyard (room 1) has real ground past the gatehouse
    // doorway's plane, away from room 100.
    #[test]
    fn a_doorway_cut_is_limited_to_the_other_rooms_footprint() {
        let path = std::path::Path::new("data/map/rm11/rm11.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let map = crate::map::read(path).expect("rm11 loads");
        let vis = Visibility::new(&map);
        let feet = [33243.0, 32677.0, 34205.0];
        let eye = [feet[0], feet[1] + 155.0, feet[2]];
        let rooms = vis.solid_rooms(&map, &[1], &[], &[feet, eye]);
        let r1 = rooms.iter().find(|r| r.room == 1).unwrap();
        let cut = r1.cut.iter().find(|c| c.within.is_some()).expect("room 1 is cut at the doorway");
        let w = cut.within.unwrap();
        // Room 100's own footprint: inside it, room 1 is cut; elsewhere it is not
        assert!(33113.0 >= w[0] && 33113.0 <= w[2] && 35000.0 >= w[1] && 35000.0 <= w[3]);
        assert!(!(31000.0 >= w[0] && 31000.0 <= w[2]), "ground well to the side is not cut");
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

    // On citystreet_large the stair up from room 3's hall is room 2, joined by the portal p03_02
    // in the plane x = 33317, room 2 on the low side. Walking off the bottom step the floor under
    // the player is room 3's before the player has crossed the plane, so drawing starts from room 3
    // with the camera on room 2's side; room 2 must still be the side the camera is on.
    // On rm11 the courtyard is rooms 5 and 6, joined by portal pt05_06 at x = 32637. Room 5's
    // ground reaches 1500 units past it into 6, so every point near it is in both rooms; the
    // middles of their volumes still say which side is which. Taken from the camera instead,
    // standing in 6 with the camera on 5's side (third person) cut 6's own walls away.
    #[test]
    fn a_courtyard_portal_knows_its_sides_where_both_rooms_reach_past_it() {
        let path = std::path::Path::new("data/map/rm11/rm11.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let map = crate::map::read(path).expect("rm11 loads");
        let vis = Visibility::new(&map);
        let i = map.portals.portals.iter().position(|p| p.name == "pt05_06").expect("the courtyard portal");
        let five = vis.side_point(i, 5).expect("room 5's side is known");
        let six = vis.side_point(i, 6).expect("room 6's side is known");
        assert!(five[0] < 32637.0 && six[0] > 32637.0, "5 at {five:?}, 6 at {six:?}");
    }

    #[test]
    fn a_portal_knows_which_side_is_which_room_wherever_the_camera_is() {
        let path = std::path::Path::new("data/map/m00/citystreet_large.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let map = crate::map::read(path).expect("citystreet_large loads");
        let vis = Visibility::new(&map);
        let side = |plane: [f32; 4], x: f32| plane[0] * x + plane[3] + plane[1] * 33000.0 + plane[2] * 35350.0;
        for x in [33300.0, 33313.4, 33330.0] {
            let feet = [x, 32905.8, 35326.3];
            let eye = [x, feet[1] + 165.0, feet[2]];
            // Only what is within a few hundred units is in view, so the rest cannot use up the plan
            let near = |p: [f32; 3]| if (p[0] - x).hypot(p[2] - feet[2]) < 400.0 { [0.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 0.0, -1.0] };
            let plan = vis.plan(&map, 3, &[feet, eye], eye, &[], &near);
            let stair = plan.children.iter().find(|c| c.room == 2).unwrap_or_else(|| panic!("room 2 is beside 3: {:?}", plan.children.iter().map(|c| c.room).collect::<Vec<_>>()));
            let plane = stair.plane.expect("the portal has a plane");
            assert!(side(plane, 33200.0) > 0.0, "the stair is on the low side, camera at {x}");
            assert!(side(plane, 33400.0) < 0.0, "and the hall is not, camera at {x}");
            // The collision cut agrees
            let rooms = vis.solid_rooms(&map, &[3], &[], &[feet, eye]);
            let r2 = rooms.iter().find(|r| r.room == 2).expect("room 2 is next to 3");
            let counts = |p: [f32; 3]| r2.keep.iter().any(|k| k[0] * p[0] + k[1] * p[1] + k[2] * p[2] + k[3] >= 0.0);
            assert!(counts([33200.0, 33000.0, 35350.0]), "the stair is solid, camera at {x}");
            assert!(!counts([33400.0, 33000.0, 35350.0]), "the hall is not room 2's, camera at {x}");
        }
    }

    #[test]
    fn rectangles_combine() {
        assert_eq!(intersect([0.0, 0.0, 1.0, 1.0], [0.5, 0.5, 2.0, 2.0]), Some([0.5, 0.5, 1.0, 1.0]));
        assert_eq!(intersect([0.0, 0.0, 1.0, 1.0], [2.0, 2.0, 3.0, 3.0]), None);
        assert!(contains(FULL, [0.0, 0.0, 0.5, 0.5]));
    }
}
