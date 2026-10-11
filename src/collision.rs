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

/// Tunables for the player body. Units are MAP units (about a centimetre: the
/// storeys in rm11 are 400 units apart).
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub radius: f32,
    /// Feet to top of head
    pub height: f32,
    /// Feet to eye
    pub eye_height: f32,
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
            step_up: 45.0,
            step_down: 45.0,
            max_slope_deg: 50.0,
            gravity: 1800.0,
        }
    }
}

/// Player body state. `pos` is the point on the floor between the feet.
#[derive(Clone, Copy, Debug)]
pub struct Player {
    pub pos: V3,
    pub vel_y: f32,
    pub on_ground: bool,
}

impl Player {
    pub fn new(pos: V3) -> Self {
        Self { pos, vel_y: 0.0, on_ground: false }
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
    /// The MAP material this face uses (`usize::MAX` for none)
    material: usize,
    /// The room this face belongs to (`NO_ROOM` when its object is not a room's)
    room: u32,
}

/// Marks faces that are solid whichever rooms are active
const NO_ROOM: u32 = u32::MAX;

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
    /// Which room numbers are solid right now; None means all of them
    active: Option<Vec<bool>>,
}

impl CollisionWorld {
    pub fn from_map(level: &map::Map, params: Params) -> Self {
        let mut tris = Vec::new();
        let glass = crate::glass::panes(level);
        let glass_names: Vec<&str> = glass.iter().map(|g| g.name.as_str()).collect();
        let room_numbers: Vec<u32> = level.rooms.rooms.iter().filter_map(|r| r.section_name.parse().ok()).collect();
        for object in &level.geometries.objects {
            // Glass panes are stored in their own space; they are added placed, below
            if glass_names.contains(&object.name.as_str()) {
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
                    tris.push(Tri { a, b, c, n, off: false, material: od.mn as usize, room });
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
                tris.push(Tri { a, b, c, n: scale(gn, 1.0 / gl), off: false, material: usize::MAX, room: NO_ROOM });
            }
            groups.push((pane.name.clone(), first..tris.len()));
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
            active: None,
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

    pub fn params(&self) -> &Params { &self.params }

    /// Make only the faces of these rooms solid (everything else, such as the glass,
    /// stays solid), or all of them with None. The level overlaps itself: the outside
    /// of a plane sits inside its cabin, so only the rooms around the player may count.
    pub fn set_active_rooms(&mut self, rooms: Option<&[u32]>) {
        self.active = rooms.map(|rooms| {
            let mut on = vec![false; rooms.iter().copied().max().map_or(0, |m| m as usize + 1)];
            for &r in rooms {
                on[r as usize] = true;
            }
            on
        });
    }

    /// Whether a face can currently stop anything
    fn solid(&self, t: &Tri) -> bool {
        !t.off && (t.room == NO_ROOM || self.active.as_ref().is_none_or(|on| on.get(t.room as usize).copied().unwrap_or(false)))
    }

    /// Distance along the unit vector `dir` from `origin` to the nearest
    /// triangle in the level within `max_dist`, treating every triangle as
    /// solid from both sides (used for bullets and line of sight).
    pub fn raycast(&self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> Option<f32> {
        let mut best: Option<f32> = None;
        for t in self.tris.iter().filter(|t| self.solid(t)) {
            if let Some(d) = crate::combat::ray_triangle(origin, dir, t.a, t.b, t.c) {
                if d <= max_dist && best.map_or(true, |b| d < b) {
                    best = Some(d);
                }
            }
        }
        best
    }

    /// Like `raycast`, but says what was hit: how far, which way the surface faces and its material
    pub fn raycast_hit(&self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> Option<RayHit> {
        let mut best: Option<(f32, &Tri)> = None;
        for t in self.tris.iter().filter(|t| self.solid(t)) {
            if let Some(d) = crate::combat::ray_triangle(origin, dir, t.a, t.b, t.c) {
                if d <= max_dist && best.map_or(true, |(b, _)| d < b) {
                    best = Some((d, t));
                }
            }
        }
        best.map(|(dist, t)| {
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
            if !self.solid(t) || t.n[1] < ny_min {
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

    /// Depenetrate the body (a stack of spheres) from wall-like triangles,
    /// moving only horizontally.
    fn push_out_of_walls(&self, p: &mut Player) {
        let prm = &self.params;
        let ny_floor = self.walkable_ny();
        // Spheres start above step height so ledges the player can climb are
        // not treated as walls
        let lo = prm.step_up + prm.radius;
        let hi = (prm.height - prm.radius).max(lo);
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
                    let centre = [p.pos[0], p.pos[1] + h, p.pos[2]];
                    let q = closest_on_triangle(centre, t);
                    let away = sub(centre, q);
                    let dist = len(away);
                    if dist >= prm.radius {
                        continue;
                    }
                    // Horizontal push away from the contact
                    let (mut hx, mut hz) = (away[0], away[2]);
                    let mut hl = (hx * hx + hz * hz).sqrt();
                    if hl < 1e-4 {
                        // Centre is on the triangle; use its normal
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
        let hi = (prm.height - prm.radius).max(lo);
        let mut worst = f32::INFINITY;
        for k in 0..3 {
            let h = lo + (hi - lo) * (k as f32 / 2.0);
            let centre = [p.pos[0], p.pos[1] + h, p.pos[2]];
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
                        let d = len(sub(centre, closest_on_triangle(centre, t))) - prm.radius;
                        worst = worst.min(d);
                    }
                }
            }
        }
        worst
    }
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
            let travelled = ((p.pos[0] - start[0]).powi(2) + (p.pos[2] - start[2]).powi(2)).sqrt();
            if travelled < 7500.0 * 0.9 {
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
