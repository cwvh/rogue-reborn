//! First-person collision against the level's render triangles.
//!
//! The MAP's per-object collision lists are too sparse to walk on (many rooms
//! have none), so this builds a world from the actual geometry and answers two
//! questions: "what is the floor under this point?" and "push this body out of
//! the walls". Everything here is in MAP space (Y up, Z as stored in the file).
//! Callers that render with `glScalef(1, 1, -1)` must negate Z when converting
//! camera positions to and from MAP space.

use crate::map;

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn scale(a: V3, s: f32) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn len(a: V3) -> f32 { dot(a, a).sqrt() }

/// Tunables for the player body. Units are MAP units (about a centimeter: the
/// stories in rm11 are 400 units apart).
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub radius: f32,
    /// Feet to top of head
    pub height: f32,
    /// Feet to eye
    pub eye_height: f32,
    /// Feet to top of head and to eye, crouched
    pub crouch_height: f32,
    pub crouch_eye_height: f32,
    /// Largest ledge that can be walked up without jumping
    pub step_up: f32,
    /// Largest drop that is walked down (stairs) instead of fallen
    pub step_down: f32,
    /// Steepest walkable slope, degrees from horizontal
    pub max_slope_deg: f32,
    /// Units per second squared
    pub gravity: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            radius: 30.0,
            height: 180.0,
            eye_height: 165.0,
            crouch_height: 125.0,
            crouch_eye_height: 110.0,
            step_up: 45.0,
            step_down: 45.0,
            max_slope_deg: 50.0,
            gravity: 1800.0,
        }
    }
}

impl Params {
    /// Feet to top of head, standing or crouched
    pub fn body_height(&self, crouched: bool) -> f32 {
        if crouched { self.crouch_height } else { self.height }
    }

    /// Feet to eye, standing or crouched
    pub fn eye(&self, crouched: bool) -> f32 {
        if crouched { self.crouch_eye_height } else { self.eye_height }
    }
}

/// Player body state. `pos` is the point on the floor between the feet.
#[derive(Clone, Copy, Debug)]
pub struct Player {
    pub pos: V3,
    pub vel_y: f32,
    pub on_ground: bool,
    /// Crouched, which makes the body shorter
    pub crouched: bool,
}

impl Player {
    pub fn new(pos: V3) -> Self {
        Self { pos, vel_y: 0.0, on_ground: false, crouched: false }
    }
}

struct Tri {
    a: V3,
    b: V3,
    c: V3,
    /// Unit normal taken from the MAP's face normal, which defines which side
    /// of the triangle is up.
    n: V3,
    /// Switched off, like a pane of glass that has broken
    off: bool,
    /// Left out because its room is not one that counts around the player right now
    hidden: bool,
    /// The MAP material this face uses (`usize::MAX` for none)
    material: usize,
    /// The room this face belongs to (`NO_ROOM` when its object is not a room's)
    room: u32,
}

/// Marks faces that are solid whichever rooms are active
const NO_ROOM: u32 = u32::MAX;
/// How close two floors' heights must be to count as the same floor
const SAME_FLOOR: f32 = 0.5;

/// Where a ray struck the level
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub dist: f32,
    /// Unit normal of the surface, facing back along the ray
    pub normal: V3,
    /// Index into the MAP's materials, if the face has one
    pub material: Option<usize>,
    /// The room whose geometry was hit
    pub room: Option<u32>,
}

pub struct CollisionWorld {
    tris: Vec<Tri>,
    /// Named groups of triangles that can be switched off: (name, range into `tris`)
    groups: Vec<(String, std::ops::Range<usize>)>,
    // Uniform grid over XZ; each cell lists the triangles overlapping it
    min_x: f32,
    min_z: f32,
    cell: f32,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
    params: Params,
    /// The doors, each a leaf of triangles that moves as the door opens
    doors: Vec<DoorBody>,
    /// What goes through the faces of each MAP material: `BULLETS` and `GRENADES`, from the
    /// game's texture properties (`gunpass` on a railing, `grenadepass` on a catwalk)
    passes: Vec<u8>,
}

/// Faces bullets go through
const BULLETS: u8 = 1;
/// Faces grenades go through
const GRENADES: u8 = 2;

/// A door's leaf in the collision world
struct DoorBody {
    door: crate::door::Door,
    /// The leaf's triangles in the door's own space
    local: Vec<[V3; 3]>,
    /// Where they are in `tris`
    range: std::ops::Range<usize>,
}

/// One triangle of a door leaf, with the door `open` this far
fn placed_tri(door: &crate::door::Door, p: [V3; 3], open: f32, material: usize) -> Tri {
    let (a, b, c) = (door.place(p[0], open), door.place(p[1], open), door.place(p[2], open));
    let g = cross(sub(b, a), sub(c, a));
    let l = len(g);
    let n = if l > 1e-6 { scale(g, 1.0 / l) } else { [0.0, 1.0, 0.0] };
    Tri { a, b, c, n, off: false, hidden: false, material, room: NO_ROOM }
}

/// A room that is solid around the player, and how much of it
#[derive(Clone, Debug, PartialEq)]
pub struct SolidRoom {
    pub room: u32,
    /// Planes (a, b, c, d) with a*x + b*y + c*z + d >= 0 on the kept side. When there
    /// are any, only the parts of the room on the kept side of at least one count. A room
    /// next door overlaps the player's, and only what is beyond the portal between them
    /// is really in it.
    pub keep: Vec<[f32; 4]>,
    /// What to leave out of the player's own room where it runs past a portal they are
    /// standing in, into the room beyond. Every cut applies.
    pub cut: Vec<Cut>,
    /// Boxes (min x, y, z, max x, y, z) the room does not count inside: a volume of the
    /// player's room that reaches through this one, like rm11's stairs climbing through the
    /// wall and ceiling of the room below. This room's surfaces there are not really there.
    /// What is within `HOLE_MARGIN` of a portal between the two still counts.
    pub holes: Vec<[f32; 6]>,
}

/// How far beyond the portal to a neighbor a hole starts. A volume reaching through a
/// doorway by a wall's thickness must not take the neighbor's jambs with it.
pub const HOLE_MARGIN: f32 = 50.0;

/// Part of a room that does not count: what is on the far side of `plane` (a, b, c, d with
/// a*x + b*y + c*z + d >= 0 where it does count) and inside `within`. Without `within` it
/// is everything on the far side. The limit matters in a big room, such as a courtyard,
/// that has real surfaces beyond a doorway's plane away from the room next door.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    pub plane: [f32; 4],
    /// A floor plan box (min x, min z, max x, max z)
    pub within: Option<[f32; 4]>,
}

impl SolidRoom {
    pub fn whole(room: u32) -> Self {
        Self { room, keep: Vec::new(), cut: Vec::new(), holes: Vec::new() }
    }
}

impl CollisionWorld {
    pub fn from_map(level: &map::Map, params: Params) -> Self {
        let mut tris = Vec::new();
        let glass = crate::glass::panes(level);
        let glass_names: Vec<&str> = glass.iter().map(|g| g.name.as_str()).collect();
        let doors = crate::door::doors(level);
        let door_models: Vec<usize> = doors.iter().map(|d| d.model_index).collect();
        let room_numbers: Vec<u32> = level.rooms.rooms.iter().filter_map(|r| r.section_name.parse().ok()).collect();
        let properties = crate::cxp::TextureProperties::load();
        let passes = level.materials.materials.iter().map(|m| {
            let p = properties.get(&m.filename);
            let bullets = if p.is_some_and(|p| p.gun_pass) { BULLETS } else { 0 };
            bullets | if p.is_some_and(|p| p.grenade_pass) { GRENADES } else { 0 }
        }).collect();
        for (object_index, object) in level.geometries.objects.iter().enumerate() {
            // Glass panes and door leaves are stored in their own space; they are added
            // placed, below
            if glass_names.contains(&object.name.as_str()) || door_models.contains(&object_index) {
                continue;
            }
            let room = map::object_room(&object.name).filter(|n| room_numbers.contains(n)).unwrap_or(NO_ROOM);
            for od in &object.object_datas {
                let n = od.faces.face_indices.len().min(od.faces.normals.len());
                for i in 0..n {
                    let (i0, i1, i2) = od.faces.face_indices[i];
                    let (Some(a), Some(b), Some(c)) = (
                        object.vertices.get(i0 as usize),
                        object.vertices.get(i1 as usize),
                        object.vertices.get(i2 as usize),
                    ) else { continue };
                    let (a, b, c) = ([a.x, a.y, a.z], [b.x, b.y, b.z], [c.x, c.y, c.z]);

                    let fnorm = &od.faces.normals[i];
                    let mut n = [fnorm.x, fnorm.y, fnorm.z];
                    let l = len(n);
                    if l > 1e-6 {
                        n = scale(n, 1.0 / l);
                    } else {
                        // Fall back to the winding if the file normal is empty
                        let gn = cross(sub(b, a), sub(c, a));
                        let gl = len(gn);
                        if gl <= 1e-6 { continue; }
                        n = scale(gn, 1.0 / gl);
                    }
                    tris.push(Tri { a, b, c, n, off: false, hidden: false, material: od.mn as usize, room });
                }
            }
        }

        // Intact glass is solid until it breaks
        let mut groups = Vec::new();
        for pane in &glass {
            let first = tris.len();
            for t in &pane.tris {
                let (a, b, c) = (t.p[0], t.p[1], t.p[2]);
                let gn = cross(sub(b, a), sub(c, a));
                let gl = len(gn);
                if gl <= 1e-6 { continue; }
                tris.push(Tri { a, b, c, n: scale(gn, 1.0 / gl), off: false, hidden: false, material: usize::MAX, room: NO_ROOM });
            }
            groups.push((pane.name.clone(), first..tris.len()));
        }

        // Doors start closed, and are solid whichever rooms are around the player
        let mut bodies = Vec::new();
        for door in doors {
            let first = tris.len();
            let leaf = crate::door::leaf_triangles(level, &door);
            for (p, material) in &leaf {
                tris.push(placed_tri(&door, *p, 0.0, *material));
            }
            let local: Vec<[V3; 3]> = leaf.into_iter().map(|(p, _)| p).collect();
            bodies.push(DoorBody { range: first..tris.len(), local, door });
        }

        let (mut min_x, mut min_z) = (f32::INFINITY, f32::INFINITY);
        let (mut max_x, mut max_z) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for t in &tris {
            for p in [t.a, t.b, t.c] {
                min_x = min_x.min(p[0]);
                max_x = max_x.max(p[0]);
                min_z = min_z.min(p[2]);
                max_z = max_z.max(p[2]);
            }
        }
        if tris.is_empty() {
            (min_x, min_z, max_x, max_z) = (0.0, 0.0, 1.0, 1.0);
        }

        let cell = 200.0;
        let cols = (((max_x - min_x) / cell).floor() as usize + 1).max(1);
        let rows = (((max_z - min_z) / cell).floor() as usize + 1).max(1);
        let mut world = Self {
            tris,
            groups,
            min_x,
            min_z,
            cell,
            cols,
            rows,
            cells: vec![Vec::new(); cols * rows],
            params,
            doors: bodies,
            passes,
        };

        for ti in 0..world.tris.len() {
            let t = &world.tris[ti];
            let xs = [t.a[0], t.b[0], t.c[0]];
            let zs = [t.a[2], t.b[2], t.c[2]];
            let (x0, x1) = (xs.iter().cloned().fold(f32::INFINITY, f32::min), xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max));
            let (z0, z1) = (zs.iter().cloned().fold(f32::INFINITY, f32::min), zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max));
            let (c0, c1) = (world.col(x0), world.col(x1));
            let (r0, r1) = (world.row(z0), world.row(z1));
            for r in r0..=r1 {
                for c in c0..=c1 {
                    world.cells[r * cols + c].push(ti as u32);
                }
            }
        }
        world
    }

    /// A world that is one level square floor at height `y`, `half` units either way from the
    /// origin: for trying out things that move through the level without a MAP
    pub fn flat_floor(y: f32, half: f32) -> Self {
        let n = [0.0, 1.0, 0.0];
        let corner = |x: f32, z: f32| [x, y, z];
        let tri = |a, b, c| Tri { a, b, c, n, off: false, hidden: false, material: usize::MAX, room: NO_ROOM };
        let tris = vec![
            tri(corner(-half, -half), corner(half, -half), corner(half, half)),
            tri(corner(-half, -half), corner(half, half), corner(-half, half)),
        ];
        let cell = 2.0 * half;
        let mut world = Self { tris, groups: Vec::new(), min_x: -half, min_z: -half, cell, cols: 1, rows: 1, cells: vec![vec![0, 1]], params: Params::default(), doors: Vec::new(), passes: Vec::new() };
        world.cells[0] = vec![0, 1];
        world
    }

    pub fn params(&self) -> &Params { &self.params }

    /// The doors, in the order `set_door_open` takes them
    pub fn doors(&self) -> impl Iterator<Item = &crate::door::Door> {
        self.doors.iter().map(|d| &d.door)
    }

    /// Put door `index` open this far (0 closed to 1 open), moving its triangles
    pub fn set_door_open(&mut self, index: usize, open: f32) {
        let Some(body) = self.doors.get(index) else { return };
        let (range, door) = (body.range.clone(), body.door.clone());
        let moved: Vec<Tri> = body.local.iter().zip(range.clone())
            .map(|(p, i)| placed_tri(&door, *p, open, self.tris[i].material))
            .collect();
        for (ti, new) in range.zip(moved) {
            let old_cells = self.cells_of(&self.tris[ti]);
            for c in old_cells {
                self.cells[c].retain(|&t| t as usize != ti);
            }
            let (off, hidden) = (self.tris[ti].off, self.tris[ti].hidden);
            self.tris[ti] = Tri { off, hidden, ..new };
            for c in self.cells_of(&self.tris[ti]) {
                self.cells[c].push(ti as u32);
            }
        }
    }

    /// The grid cells a triangle covers
    fn cells_of(&self, t: &Tri) -> Vec<usize> {
        let xs = [t.a[0], t.b[0], t.c[0]];
        let zs = [t.a[2], t.b[2], t.c[2]];
        let (x0, x1) = (xs.iter().cloned().fold(f32::INFINITY, f32::min), xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max));
        let (z0, z1) = (zs.iter().cloned().fold(f32::INFINITY, f32::min), zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max));
        let mut out = Vec::new();
        for r in self.row(z0)..=self.row(z1) {
            for c in self.col(x0)..=self.col(x1) {
                out.push(r * self.cols + c);
            }
        }
        out
    }

    /// Make only the faces of these rooms solid (everything else, such as the glass,
    /// stays solid), or all of them with None. The level overlaps itself: the outside
    /// of a plane sits inside its cabin, so only the rooms around the player may count.
    pub fn set_active_rooms(&mut self, rooms: Option<&[u32]>) {
        let whole: Option<Vec<SolidRoom>> = rooms.map(|r| r.iter().map(|&n| SolidRoom::whole(n)).collect());
        self.set_solid_rooms(whole.as_deref());
    }

    /// Like `set_active_rooms`, but a room can be solid in part only (see `SolidRoom`)
    pub fn set_solid_rooms(&mut self, rooms: Option<&[SolidRoom]>) {
        let side = |planes: &[[f32; 4]], c: V3| planes.iter().map(|p| p[0] * c[0] + p[1] * c[1] + p[2] * c[2] + p[3] >= 0.0).collect::<Vec<bool>>();
        let beyond = |p: &[f32; 4], c: V3| p[0] * c[0] + p[1] * c[1] + p[2] * c[2] + p[3];
        for t in &mut self.tris {
            t.hidden = match rooms {
                None => false,
                Some(_) if t.room == NO_ROOM => false,
                Some(rooms) => {
                    let middle = scale(add(add(t.a, t.b), t.c), 1.0 / 3.0);
                    // A room listed more than once counts wherever any entry has it
                    !rooms.iter().filter(|r| r.room == t.room).any(|r| {
                        let counts = |c: &Cut| {
                            let past = !side(&[c.plane], middle)[0];
                            let inside = c.within.is_none_or(|w| middle[0] >= w[0] && middle[0] <= w[2] && middle[2] >= w[1] && middle[2] <= w[3]);
                            !(past && inside)
                        };
                        // A wall's few big triangles reach well outside a hole, so any part in it
                        // takes the triangle out
                        let in_hole = r.holes.iter().any(|h| reaches_into(t, h)) && r.keep.iter().all(|p| beyond(p, middle) > HOLE_MARGIN);
                        (r.keep.is_empty() || side(&r.keep, middle).iter().any(|&ok| ok)) && r.cut.iter().all(counts) && !in_hole
                    })
                }
            };
        }
    }

    /// The room whose floor is under `feet`: the first face below that faces up, whatever
    /// rooms are active, so it can be asked where the player stands before deciding what is
    /// solid. A ceiling is no floor: on rm11 the stairs of one room climb through the ceiling
    /// of another, which would otherwise be the nearest face under the feet.
    pub fn floor_room(&self, feet: [f32; 3]) -> Option<u32> {
        self.floor_rooms(feet).first().copied()
    }

    /// Every room with a floor at the height of the one under `feet`, in the order of their
    /// first triangle in the level. Two rooms can share a floor: rm02's deck is both the
    /// deck's (100) and the ship's outside's (605), at the same height.
    pub fn floor_rooms(&self, feet: [f32; 3]) -> Vec<u32> {
        // A ray straight down meets only triangles over that point so only the
        // grid cells around it need inspection
        let origin = [feet[0], feet[1] + 60.0, feet[2]];
        let mut hits: Vec<(f32, usize, u32)> = Vec::new();
        for r in self.row(feet[2] - 1.0)..=self.row(feet[2] + 1.0) {
            for c in self.col(feet[0] - 1.0)..=self.col(feet[0] + 1.0) {
                for &ti in &self.cells[r * self.cols + c] {
                    let ti = ti as usize;
                    let t = &self.tris[ti];
                    if t.off || t.n[1] <= 0.0 {
                        continue;
                    }
                    if let Some(d) = crate::combat::ray_triangle(origin, [0.0, -1.0, 0.0], t.a, t.b, t.c) {
                        if d <= 220.0 {
                            hits.push((d, ti, t.room));
                        }
                    }
                }
            }
        }
        let Some(nearest) = hits.iter().map(|h| h.0).reduce(f32::min) else { return Vec::new() };
        hits.retain(|h| h.0 <= nearest + SAME_FLOOR && h.2 != NO_ROOM);
        hits.sort_by_key(|h| h.1);
        let mut rooms: Vec<u32> = Vec::new();
        for (_, _, room) in hits {
            if !rooms.contains(&room) {
                rooms.push(room);
            }
        }
        rooms
    }

    /// Whether a face can currently stop anything
    fn solid(&self, t: &Tri) -> bool {
        !t.off && !t.hidden
    }

    /// The nearest face within `max_dist` along the unit vector `dir` from `origin`, solid from
    /// both sides, leaving out those that let `through` pass (`BULLETS`, `GRENADES`, or 0)
    fn nearest(&self, origin: V3, dir: V3, max_dist: f32, through: u8) -> Option<(f32, &Tri)> {
        let mut best: Option<(f32, &Tri)> = None;
        for t in self.tris.iter().filter(|t| self.solid(t)) {
            if self.passes.get(t.material).is_some_and(|p| p & through != 0) {
                continue;
            }
            if let Some(d) = crate::combat::ray_triangle(origin, dir, t.a, t.b, t.c) {
                if d <= max_dist && best.map_or(true, |(b, _)| d < b) {
                    best = Some((d, t));
                }
            }
        }
        best
    }

    /// Distance along the unit vector `dir` from `origin` to the nearest
    /// triangle in the level within `max_dist`, treating every triangle as
    /// solid from both sides (used for line of sight).
    pub fn raycast(&self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> Option<f32> {
        self.nearest(origin, dir, max_dist, 0).map(|(d, _)| d)
    }

    /// The solid triangles in the grid cells within `radius` of `center` (a square, so some
    /// are farther than that), each once
    pub fn solid_tris_near(&self, center: V3, radius: f32) -> Vec<[V3; 3]> {
        let mut near: Vec<usize> = Vec::new();
        for r in self.row(center[2] - radius)..=self.row(center[2] + radius) {
            for c in self.col(center[0] - radius)..=self.col(center[0] + radius) {
                near.extend(self.cells[r * self.cols + c].iter().map(|&i| i as usize));
            }
        }
        near.sort_unstable();
        near.dedup();
        near.into_iter().map(|i| &self.tris[i]).filter(|t| self.solid(t)).map(|t| [t.a, t.b, t.c]).collect()
    }

    /// A ray caster for many rays near one spot: only the solid triangles within `radius` of
    /// `center` are kept, so each ray costs a few hundred tests and not the whole level. It
    /// takes an origin, a unit direction and a longest distance, like `raycast`, and is only
    /// right for rays that stay within `radius` of `center`; one that leaves it may miss what is out there.
    pub fn local_raycaster(&self, center: V3, radius: f32) -> Box<dyn Fn(V3, V3, f32) -> Option<f32>> {
        let tris = self.solid_tris_near(center, radius);
        Box::new(move |origin, dir, max_dist| {
            tris.iter()
                .filter_map(|t| crate::combat::ray_triangle(origin, dir, t[0], t[1], t[2]))
                .filter(|&d| d <= max_dist)
                .reduce(f32::min)
        })
    }

    /// Like `raycast`, but says what was hit: how far, which way the surface faces and its material
    pub fn raycast_hit(&self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> Option<RayHit> {
        self.hit(origin, dir, max_dist, 0)
    }

    /// What a bullet strikes: as `raycast_hit`, through the faces bullets pass (a chain fence)
    pub fn bullet_hit(&self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> Option<RayHit> {
        self.hit(origin, dir, max_dist, BULLETS)
    }

    /// What a grenade strikes: as `raycast_hit`, through the faces grenades pass (a railing)
    pub fn grenade_hit(&self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> Option<RayHit> {
        self.hit(origin, dir, max_dist, GRENADES)
    }

    fn hit(&self, origin: V3, dir: V3, max_dist: f32, through: u8) -> Option<RayHit> {
        self.nearest(origin, dir, max_dist, through).map(|(dist, t)| {
            // The file's normals are not reliable about which side is front
            let n = if dot(t.n, dir) > 0.0 { scale(t.n, -1.0) } else { t.n };
            RayHit { dist, normal: n, material: (t.material != usize::MAX).then_some(t.material), room: (t.room != NO_ROOM).then_some(t.room) }
        })
    }

    pub fn triangle_count(&self) -> usize { self.tris.len() }

    /// Stop a named group of triangles (a glass pane) from blocking anything
    pub fn switch_off(&mut self, name: &str) {
        for (n, range) in &self.groups {
            if n == name {
                for t in &mut self.tris[range.clone()] {
                    t.off = true;
                }
            }
        }
    }

    fn col(&self, x: f32) -> usize {
        (((x - self.min_x) / self.cell).floor().max(0.0) as usize).min(self.cols - 1)
    }

    fn row(&self, z: f32) -> usize {
        (((z - self.min_z) / self.cell).floor().max(0.0) as usize).min(self.rows - 1)
    }

    fn walkable_ny(&self) -> f32 {
        self.params.max_slope_deg.to_radians().cos()
    }

    /// Height of the highest walkable surface at (x, z) that is at or below
    /// `max_y`.
    pub fn ground_at(&self, x: f32, z: f32, max_y: f32) -> Option<f32> {
        if x < self.min_x - self.cell || z < self.min_z - self.cell {
            return None;
        }
        let ny_min = self.walkable_ny();
        let mut best: Option<f32> = None;
        for &ti in &self.cells[self.row(z) * self.cols + self.col(x)] {
            let t = &self.tris[ti as usize];
            // Floors hold whichever room they belong to. Rooms overlap in walls and
            // ceilings, where a stray surface traps or blocks the player, but a floor under
            // the feet is never in the way, and a big hall is often several rooms with no
            // portal between them, so the room next door has to be walkable before the
            // player is deep inside it.
            if t.off || t.n[1] < ny_min {
                continue;
            }
            if let Some(y) = height_on_triangle(t, x, z) {
                if y <= max_y && best.map_or(true, |b| y > b) {
                    best = Some(y);
                }
            }
        }
        best
    }

    /// Advance the player by a desired horizontal displacement `(dx, dz)` over
    /// `dt` seconds: walls first (sliding), then gravity and ground snapping.
    pub fn step(&self, p: &mut Player, d: [f32; 2], dt: f32) {
        // Substep so fast movement cannot jump through a wall
        let dist = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let max_sub = self.params.radius * 0.5;
        let n = ((dist / max_sub).ceil() as usize).max(1);
        for _ in 0..n {
            p.pos[0] += d[0] / n as f32;
            p.pos[2] += d[1] / n as f32;
            self.push_out_of_walls(p);
            self.settle_vertical(p, dt / n as f32);
            // Stepping up moves the body spheres, which can reveal new contacts
            self.push_out_of_walls(p);
        }
    }

    fn settle_vertical(&self, p: &mut Player, dt: f32) {
        let prm = &self.params;
        let ground = self.ground_at(p.pos[0], p.pos[2], p.pos[1] + prm.step_up);

        if p.on_ground {
            if let Some(g) = ground {
                // Walk up steps, or down them if the drop is small enough
                if p.pos[1] - g <= prm.step_down {
                    p.pos[1] = g;
                    p.vel_y = 0.0;
                    return;
                }
            }
            p.on_ground = false;
        }

        p.vel_y -= prm.gravity * dt;
        p.pos[1] += p.vel_y * dt;
        if let Some(g) = ground {
            if p.pos[1] <= g {
                p.pos[1] = g;
                p.vel_y = 0.0;
                p.on_ground = true;
            }
        }
    }

    /// Whether a crouched player has the room to stand up: nothing solid between the top of
    /// the crouched body and the top of a standing one
    pub fn room_to_stand(&self, p: &Player) -> bool {
        let prm = &self.params;
        // A square within the body's circle, from the crouched head to the standing one
        let half = prm.radius * 0.7;
        let b = [p.pos[0] - half, p.pos[1] + prm.crouch_height, p.pos[2] - half, p.pos[0] + half, p.pos[1] + prm.height, p.pos[2] + half];
        for r in self.row(b[2])..=self.row(b[5]) {
            for c in self.col(b[0])..=self.col(b[3]) {
                for &ti in &self.cells[r * self.cols + c] {
                    let t = &self.tris[ti as usize];
                    if self.solid(t) && reaches_into(t, &b) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Depenetrate the body (a stack of spheres) from wall-like triangles,
    /// moving only horizontally.
    fn push_out_of_walls(&self, p: &mut Player) {
        let prm = &self.params;
        let ny_floor = self.walkable_ny();
        // Spheres start above step height so ledges the player can climb are
        // not treated as walls
        let lo = prm.step_up + prm.radius;
        let hi = (prm.body_height(p.crouched) - prm.radius).max(lo);
        let count = 3;

        let mut near: Vec<u32> = Vec::new();
        for _ in 0..8 {
            near.clear();
            let reach = prm.radius;
            let (c0, c1) = (self.col(p.pos[0] - reach), self.col(p.pos[0] + reach));
            let (r0, r1) = (self.row(p.pos[2] - reach), self.row(p.pos[2] + reach));
            for r in r0..=r1 {
                for c in c0..=c1 {
                    near.extend_from_slice(&self.cells[r * self.cols + c]);
                }
            }
            near.sort_unstable();
            near.dedup();

            let mut moved = false;
            for k in 0..count {
                let h = lo + (hi - lo) * (k as f32 / (count - 1) as f32);
                for &ti in &near {
                    let t = &self.tris[ti as usize];
                    if !self.solid(t) || t.n[1].abs() >= ny_floor {
                        continue;
                    }
                    // Recomputed per triangle: earlier pushes move the body
                    let center = [p.pos[0], p.pos[1] + h, p.pos[2]];
                    let q = closest_on_triangle(center, t);
                    let away = sub(center, q);
                    let dist = len(away);
                    if dist >= prm.radius {
                        continue;
                    }
                    // Horizontal push away from the contact
                    let (mut hx, mut hz) = (away[0], away[2]);
                    let mut hl = (hx * hx + hz * hz).sqrt();
                    if hl < 1e-4 {
                        // Center is on the triangle; use its normal
                        hx = t.n[0];
                        hz = t.n[2];
                        hl = (hx * hx + hz * hz).sqrt();
                        if hl < 1e-4 {
                            continue;
                        }
                    }
                    let depth = prm.radius - dist;
                    p.pos[0] += hx / hl * depth;
                    p.pos[2] += hz / hl * depth;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
    }

    /// Smallest distance from any wall-like triangle to the body's spheres;
    /// negative when penetrating. Used by tests.
    pub fn wall_clearance(&self, p: &Player) -> f32 {
        let prm = &self.params;
        let ny_floor = self.walkable_ny();
        let lo = prm.step_up + prm.radius;
        let hi = (prm.body_height(p.crouched) - prm.radius).max(lo);
        let mut worst = f32::INFINITY;
        for k in 0..3 {
            let h = lo + (hi - lo) * (k as f32 / 2.0);
            let center = [p.pos[0], p.pos[1] + h, p.pos[2]];
            let reach = prm.radius * 2.0;
            let (c0, c1) = (self.col(p.pos[0] - reach), self.col(p.pos[0] + reach));
            let (r0, r1) = (self.row(p.pos[2] - reach), self.row(p.pos[2] + reach));
            for r in r0..=r1 {
                for c in c0..=c1 {
                    for &ti in &self.cells[r * self.cols + c] {
                        let t = &self.tris[ti as usize];
                        if !self.solid(t) || t.n[1].abs() >= ny_floor {
                            continue;
                        }
                        let d = len(sub(center, closest_on_triangle(center, t))) - prm.radius;
                        worst = worst.min(d);
                    }
                }
            }
        }
        worst
    }
}

/// Whether a triangle reaches into a box (min x, y, z, max x, y, z), by the separating axis
/// test: they are apart if some axis (the box's three, the triangle's normal, or an edge of
/// one crossed with an edge of the other) has them on either side of a gap
fn reaches_into(t: &Tri, b: &[f32; 6]) -> bool {
    let center = [(b[0] + b[3]) * 0.5, (b[1] + b[4]) * 0.5, (b[2] + b[5]) * 0.5];
    let half = [(b[3] - b[0]) * 0.5, (b[4] - b[1]) * 0.5, (b[5] - b[2]) * 0.5];
    let v = [sub(t.a, center), sub(t.b, center), sub(t.c, center)];
    let apart = |axis: V3| {
        if dot(axis, axis) < 1e-12 {
            return false;
        }
        let p = v.map(|c| dot(c, axis));
        let reach = (0..3).map(|i| half[i] * axis[i].abs()).sum::<f32>();
        p[0].min(p[1]).min(p[2]) >= reach || p[0].max(p[1]).max(p[2]) <= -reach
    };
    let units = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let edges = [sub(v[1], v[0]), sub(v[2], v[1]), sub(v[0], v[2])];
    if units.iter().any(|&u| apart(u)) || apart(cross(edges[0], edges[1])) {
        return false;
    }
    !units.iter().any(|&u| edges.iter().any(|&e| apart(cross(u, e))))
}

/// Y of triangle `t` at (x, z) if that point lies inside the triangle's XZ
/// projection.
fn height_on_triangle(t: &Tri, x: f32, z: f32) -> Option<f32> {
    let (ax, az) = (t.a[0], t.a[2]);
    let (v0x, v0z) = (t.b[0] - ax, t.b[2] - az);
    let (v1x, v1z) = (t.c[0] - ax, t.c[2] - az);
    let (v2x, v2z) = (x - ax, z - az);
    let den = v0x * v1z - v1x * v0z;
    if den.abs() < 1e-6 {
        return None;
    }
    let u = (v2x * v1z - v1x * v2z) / den;
    let v = (v0x * v2z - v2x * v0z) / den;
    const EPS: f32 = 1e-4;
    if u < -EPS || v < -EPS || u + v > 1.0 + EPS {
        return None;
    }
    Some(t.a[1] + u * (t.b[1] - t.a[1]) + v * (t.c[1] - t.a[1]))
}

/// Closest point on triangle `t` to `p` (Ericson, Real-Time Collision
/// Detection 5.1.5).
fn closest_on_triangle(p: V3, t: &Tri) -> V3 {
    let (a, b, c) = (t.a, t.b, t.c);
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return add(a, scale(ab, d1 / (d1 - d3)));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return add(a, scale(ac, d2 / (d2 - d6)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return add(b, scale(sub(c, b), w));
    }
    let denom = 1.0 / (va + vb + vc);
    add(a, add(scale(ab, vb * denom), scale(ac, vc * denom)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const RM11: &str = "data/map/rm11/rm11.map";

    fn load() -> Option<CollisionWorld> {
        let path = Path::new(RM11);
        if !path.exists() {
            eprintln!("skipping: {RM11} not found");
            return None;
        }
        let level = map::read(path).expect("rm11 loads");
        Some(CollisionWorld::from_map(&level, Params::default()))
    }

    // On rm11 the stairs of room 121 climb through the ceiling of room 119 (y 33127, facing
    // down). Standing on a tread just under it, or on one level with it, the floor is 121's.
    #[test]
    fn a_ceiling_is_not_a_floor() {
        let Some(w) = load() else { return };
        assert_eq!(w.floor_rooms([29901.1, 33087.0, 40031.7]), vec![121], "a tread under 119's ceiling");
        assert_eq!(w.floor_rooms([29840.9, 33127.0, 39965.3]), vec![121], "a tread level with 119's ceiling");
        assert_eq!(w.floor_rooms([29700.0, 32767.0, 39700.0]), vec![119], "119's own floor");
    }

    // rm02's deck is in two rooms with no portal between them, the deck (100) and the ship's
    // outside (605), and each has a floor there at the same height: both are under the feet
    #[test]
    fn two_rooms_can_share_a_floor() {
        let path = Path::new("data/map/rm02/rm02.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let w = CollisionWorld::from_map(&map::read(path).expect("rm02 loads"), Params::default());
        for at in [[39365.4, 32777.0, 30775.0], [39625.4, 32777.0, 31021.8], [39448.5, 32777.0, 30826.9]] {
            let mut rooms = w.floor_rooms(at);
            rooms.sort();
            assert_eq!(rooms, vec![100, 605], "the deck at {at:?}");
        }
    }

    #[test]
    fn a_crouched_player_stands_only_where_there_is_head_room() {
        // A floor, and a slab over half of it (x from 0 on) at `height`
        let under_slab = |height: f32| {
            let mut w = CollisionWorld::flat_floor(0.0, 1000.0);
            let n = [0.0, -1.0, 0.0];
            let tri = |a, b, c| Tri { a, b, c, n, off: false, hidden: false, material: usize::MAX, room: NO_ROOM };
            let at = |x: f32, z: f32| [x, height, z];
            w.tris.push(tri(at(0.0, -1000.0), at(1000.0, 1000.0), at(1000.0, -1000.0)));
            w.tris.push(tri(at(0.0, -1000.0), at(0.0, 1000.0), at(1000.0, 1000.0)));
            w.cells[0].extend([2, 3]);
            w
        };
        let crouched = |x: f32| Player { pos: [x, 0.0, 0.0], vel_y: 0.0, on_ground: true, crouched: true };
        let low = under_slab(150.0);
        assert!(!low.room_to_stand(&crouched(500.0)));
        assert!(low.room_to_stand(&crouched(-500.0)));
        // Clear of the slab's edge by the body's width, there is room
        assert!(low.room_to_stand(&crouched(-30.0)));
        // A slab above a standing head is no bar
        assert!(under_slab(200.0).room_to_stand(&crouched(500.0)));
        // Nor one below the crouched head, which the crouched body already fits under
        assert!(under_slab(100.0).room_to_stand(&crouched(500.0)));
    }

    #[test]
    fn a_triangle_reaches_into_a_box_only_where_it_crosses_it() {
        let (a, b, c) = ([0.0, 0.0, 10.0], [100.0, 0.0, 10.0], [0.0, 100.0, 10.0]);
        let wall = Tri { a, b, c, n: [0.0, 0.0, 1.0], off: false, hidden: false, material: usize::MAX, room: NO_ROOM };
        // The triangle's middle is outside, but its corner is in the box
        assert!(reaches_into(&wall, &[-5.0, -5.0, 5.0, 5.0, 5.0, 15.0]));
        // Beside the plane, and past the slanted edge
        assert!(!reaches_into(&wall, &[-5.0, -5.0, 11.0, 5.0, 5.0, 15.0]));
        assert!(!reaches_into(&wall, &[60.0, 60.0, 5.0, 90.0, 90.0, 15.0]));
    }

    // On rm11 a door at (33562, 33167, 34767) hangs on its hinge with the leaf along +z. A
    // ray across its middle hits it closed and passes it open.
    #[test]
    fn a_closed_door_stops_a_ray_and_an_open_one_does_not() {
        let path = Path::new("data/map/rm11/rm11.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let level = map::read(path).expect("rm11 loads");
        let mut w = CollisionWorld::from_map(&level, Params::default());
        let index = w.doors().position(|d| d.name == "210_doorknobinterior01").expect("the door is there");
        let door = w.doors().nth(index).unwrap().clone();
        let middle = door.place([0.0, 130.0, 60.0], 0.0);
        let from = [middle[0] - 60.0, middle[1], middle[2]];
        let hit = w.raycast_hit(from, [1.0, 0.0, 0.0], 150.0).expect("the closed door is in the way");
        assert!((hit.dist - 60.0).abs() < 10.0, "{}", hit.dist);

        w.set_door_open(index, 1.0);
        assert!(w.raycast_hit(from, [1.0, 0.0, 0.0], 150.0).is_none(), "an open door lets it through");
        // And the leaf is now across where the ray started: along x, at the hinge's z
        let end = door.place([0.0, 130.0, 120.0], 1.0);
        assert!((end[2] - door.origin[2]).abs() < 0.01);
        let along = [(end[0] - door.origin[0]).signum(), 0.0, 0.0];
        let across = w.raycast_hit([door.origin[0] + along[0] * 60.0, middle[1], door.origin[2] - 40.0], [0.0, 0.0, 1.0], 80.0);
        assert!(across.is_some_and(|h| (h.dist - 40.0).abs() < 10.0), "the open leaf is in the way along x");

        w.set_door_open(index, 0.0);
        assert!(w.raycast_hit(from, [1.0, 0.0, 0.0], 150.0).is_some(), "and shut again");
    }

    // From inside the plane on rm05, looking at the cabin wall: the plane's hull
    // (part of the exterior, room 100) overlaps the cabin and used to get hit first
    #[test]
    fn the_exterior_does_not_stop_bullets_inside_the_plane() {
        let path = Path::new("data/map/rm05/rm05.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return;
        }
        let level = map::read(path).expect("rm05 loads");
        let mut w = CollisionWorld::from_map(&level, Params::default());
        let eye = [37792.9, 33272.0, 35970.0];
        let dir = [0.797, -0.007, 0.605];
        let all = w.raycast_hit(eye, dir, 20000.0).expect("hits something");
        assert_eq!(all.room, Some(100), "the hull is hit first with every room solid");

        // Room 401 and the rooms its portals lead to
        w.set_active_rooms(Some(&[301, 401, 402]));
        let near = w.raycast_hit(eye, dir, 20000.0).expect("hits the cabin wall");
        assert_eq!(near.room, Some(401));
        assert!(near.dist > all.dist);

        w.set_active_rooms(None);
        assert_eq!(w.raycast_hit(eye, dir, 20000.0).unwrap().room, Some(100));
    }

    // Spawn used by the wireframe example, in MAP space (camera Z is negated)
    const SPAWN: V3 = [33924.6, 33307.0, 30028.3];

    // Spawn point standing on the floor, so tests start grounded
    fn grounded(w: &CollisionWorld) -> Player {
        let mut p = Player::new(SPAWN);
        for _ in 0..240 {
            w.step(&mut p, [0.0, 0.0], 1.0 / 60.0);
        }
        assert!(p.on_ground);
        p
    }

    // Walk straight at the middle of window 02_glass in citystreet_large (its pane is at z = 34727)
    fn walk_at_window(w: &CollisionWorld) -> f32 {
        let mut p = Player::new([32267.0, 33317.0, 34850.0]);
        for _ in 0..240 {
            w.step(&mut p, [0.0, 0.0], 1.0 / 60.0);
        }
        assert!(p.on_ground);
        for _ in 0..60 {
            w.step(&mut p, [0.0, -10.0], 1.0 / 60.0);
        }
        p.pos[2]
    }

    /// For each face of `texture` on `map_path`, whether a bullet and a grenade fired straight
    /// at it from 50 units in front are stopped by that texture
    fn stopped_by(map_path: &str, texture: &str) -> Option<Vec<[bool; 2]>> {
        let level = map::read(Path::new(map_path)).ok()?;
        let w = CollisionWorld::from_map(&level, Params::default());
        let is_texture = |m: Option<usize>| m.and_then(|m| level.materials.materials.get(m)).is_some_and(|m| m.filename.eq_ignore_ascii_case(texture));
        let mut out = Vec::new();
        for object in &level.geometries.objects {
            for od in object.object_datas.iter().filter(|od| is_texture(Some(od.mn as usize))) {
                for &(i0, i1, i2) in &od.faces.face_indices {
                    let [a, b, c] = [i0, i1, i2].map(|i| { let v = &object.vertices[i as usize]; [v.x, v.y, v.z] });
                    let g = cross(sub(b, a), sub(c, a));
                    if len(g) < 1.0 {
                        continue;
                    }
                    let n = scale(g, 1.0 / len(g));
                    let from = add(scale(add(add(a, b), c), 1.0 / 3.0), scale(n, 50.0));
                    let back = scale(n, -1.0);
                    // Only where nothing else stands in front of the face
                    if w.raycast_hit(from, back, 100.0).is_some_and(|h| is_texture(h.material)) {
                        out.push([w.bullet_hit(from, back, 100.0), w.grenade_hit(from, back, 100.0)].map(|h| is_texture(h.and_then(|h| h.material))));
                    }
                }
            }
        }
        Some(out)
    }

    // Rommel.CXP: rm13's chain fence lets bullets through but not grenades, rm06's railing both
    #[test]
    fn bullets_and_grenades_go_through_the_faces_the_game_lets_them() {
        if let Some(fence) = stopped_by("data/map/rm13/rm13.map", "rm13_chainfence.bmp") {
            assert!(fence.len() >= 8, "{} fence faces", fence.len());
            assert!(fence.iter().all(|&[bullet, grenade]| !bullet && grenade), "{fence:?}");
        }
        if let Some(rail) = stopped_by("data/map/rm06/rm06.map", "cha_rail_01.bmp") {
            assert!(rail.len() >= 8, "{} railing faces", rail.len());
            assert!(rail.iter().all(|&[bullet, grenade]| !bullet && !grenade), "{rail:?}");
        }
    }

    #[test]
    fn glass_stops_bullets_until_it_breaks() {
        let level = map::read(Path::new("data/map/m00/citystreet_large.map")).unwrap();
        let mut w = CollisionWorld::from_map(&level, Params::default());
        let shoot = |w: &CollisionWorld| w.raycast([32267.0, 33467.0, 34850.0], [0.0, 0.0, -1.0], 2000.0);
        // The glass is 123 units ahead, and the player is stopped by it too
        assert!((shoot(&w).unwrap() - 123.0).abs() < 1.0);
        assert!(walk_at_window(&w) > 34727.0 + 20.0);
        // Once broken the bullet carries on to whatever is outside
        w.switch_off("02_glass");
        assert!(shoot(&w).map_or(true, |d| d > 130.0));
    }

    #[test]
    fn builds_world_from_all_geometry() {
        let Some(w) = load() else { return };
        assert!(w.triangle_count() > 20_000, "got {}", w.triangle_count());
    }

    #[test]
    fn finds_floor_below_spawn() {
        let Some(w) = load() else { return };
        let g = w.ground_at(SPAWN[0], SPAWN[2], SPAWN[1]).expect("floor under spawn");
        // A sloped surface; a roof slab at 33413 is above the spawn
        assert!((g - 32804.0).abs() < 1.0, "ground at {g}");
    }

    #[test]
    fn no_floor_far_outside_the_map() {
        let Some(w) = load() else { return };
        assert_eq!(w.ground_at(-1.0e6, -1.0e6, 1.0e6), None);
    }

    #[test]
    fn dropped_player_lands_and_stays() {
        let Some(w) = load() else { return };
        let mut p = Player::new(SPAWN);
        for _ in 0..240 {
            w.step(&mut p, [0.0, 0.0], 1.0 / 60.0);
        }
        assert!(p.on_ground);
        assert!((p.pos[1] - 32804.0).abs() < 1.0, "feet at {}", p.pos[1]);
    }

    #[test]
    fn walking_in_every_direction_never_penetrates_walls() {
        let Some(w) = load() else { return };
        for dir in 0..16 {
            let a = dir as f32 / 16.0 * std::f32::consts::TAU;
            let step = [a.cos() * 7.5, a.sin() * 7.5];
            let mut p = grounded(&w);
            for frame in 0..600 {
                w.step(&mut p, step, 1.0 / 60.0);
                assert!(p.pos.iter().all(|v| v.is_finite()));
                let c = w.wall_clearance(&p);
                assert!(c > -1.0, "dir {dir} frame {frame}: penetrating by {c} at {:?}", p.pos);
            }
        }
    }

    #[test]
    fn walls_stop_the_player() {
        let Some(w) = load() else { return };
        // Walk far in a straight line in each direction; at least one run
        // should be stopped well short of the distance an unobstructed walk
        // covers, proving walls are solid.
        let mut stopped = 0;
        for dir in 0..16 {
            let a = dir as f32 / 16.0 * std::f32::consts::TAU;
            let step = [a.cos() * 7.5, a.sin() * 7.5];
            let mut p = grounded(&w);
            let start = p.pos;
            for _ in 0..1000 {
                w.step(&mut p, step, 1.0 / 60.0);
            }
            let traveled = ((p.pos[0] - start[0]).powi(2) + (p.pos[2] - start[2]).powi(2)).sqrt();
            if traveled < 7500.0 * 0.9 {
                stopped += 1;
            }
        }
        assert!(stopped > 0, "no direction was ever blocked");
    }

    fn walk(w: &CollisionWorld, mut p: Player, dir: [f32; 2], frames: u32) -> Player {
        for _ in 0..frames {
            w.step(&mut p, [dir[0] * 7.5, dir[1] * 7.5], 1.0 / 60.0);
        }
        p
    }

    fn xz_dist(a: &Player, b: &Player) -> f32 {
        ((a.pos[0] - b.pos[0]).powi(2) + (a.pos[2] - b.pos[2]).powi(2)).sqrt()
    }

    #[test]
    fn walks_west_across_open_ground_without_falling() {
        let Some(w) = load() else { return };
        let start = grounded(&w);
        let end = walk(&w, start, [-1.0, 0.0], 600);
        assert!(end.on_ground, "fell: feet at {:?}", end.pos);
        assert!(xz_dist(&start, &end) > 4000.0, "only moved {}", xz_dist(&start, &end));
    }

    #[test]
    fn solid_wall_stops_the_player_standing() {
        let Some(w) = load() else { return };
        let start = grounded(&w);
        let end = walk(&w, start, [0.707, -0.707], 600);
        assert!(end.on_ground, "fell: feet at {:?}", end.pos);
        // 600 frames at 7.5 per frame would cover 4500 units if unobstructed
        assert!(xz_dist(&start, &end) < 600.0, "moved {}", xz_dist(&start, &end));
    }

    #[test]
    fn walks_off_the_edge_of_the_world_and_keeps_falling() {
        let Some(w) = load() else { return };
        // There is a gap in rm11's floor a few hundred units south of the spawn
        let end = walk(&w, grounded(&w), [0.0, 1.0], 120);
        assert!(!end.on_ground);
        assert!(end.pos[1] < 32000.0, "feet at {}", end.pos[1]);
    }

    #[test]
    fn courtyard_zone_from_mission_is_walkable_ground() {
        let Some(w) = load() else { return };
        // Midpoint of ZONE_RM11_COURTYARD in data/mission/RM11_MUL.mis; the
        // mission lists y=32677, which is 10 units above the floor
        let g = w.ground_at(32660.7, 37435.0, 32677.0 + 100.0).expect("floor in courtyard");
        assert!((g - 32667.0).abs() < 1.0, "ground at {g}");
    }
}
