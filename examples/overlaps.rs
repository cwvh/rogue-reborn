use anyhow::Result;
use rogue_reborn::collision::{CollisionWorld, Params};
use rogue_reborn::visibility::{RectRoom, Visibility};
use rogue_reborn::{door, glass, levels, map};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

/// Which rooms' surfaces cut through each other where a player can see it: the "impossible
/// geometry" that drawing every room at once would show.
///
/// Two triangles of different rooms cross when each one reaches more than `--depth` units
/// past the other's plane, along a shared line longer than `--length`. Walls that merely
/// meet, or overlap by a modeling slop, do not count, and neither do crossings where either
/// triangle is alpha-tested (.tga: trees are made of crossing planes). A crossing is seen
/// when nothing stands between it and an eye standing in some room's volume. Each pair of
/// rooms that crosses where it is seen gets a command for a view of it.
///
///   cargo run --release --target x86_64-pc-windows-gnu --example overlaps -- [levels...] [--depth N] [--length N] [--all] [--cull-backs] [--rects]
///
/// With no levels, every level under data/map. --all also lists pairs that cross only
/// where no one can see it (walls running into the ground). --cull-backs sees as the game
/// does with that flag: a face seen from behind is not drawn (unless its material is
/// two-sided), so it neither shows nor hides anything. What is left then is what culling
/// does not fix.
///
/// --rects sees as the default drawing does (backs culled too): from each eye, the rooms the
/// game would start from (the floors under its feet, doors open) and `rect_plan`'s rooms, each
/// only within its rectangle, drawn in order with the doorway wipes before them and sorted out
/// by depth. What is left is what the game shows. (The rectangles are taken from six square
/// views around the eye, not the game's own, and the cuts near a doorway the player stands in
/// are left out.)
///
/// With --rects, two environment variables look closer: OV_VIEW=x,y,z,yaw,pitch prints the
/// object the drawing shows at each point of a 32 by 24 grid over a 1024 by 768 frame from that
/// camera (to check against the game's --rooms screenshot), and OV_EYE=x,y,z with OV_PAIR=a,b
/// prints that eye's plans and which arms of the pair's crossings it sees.
fn main() -> Result<()> {
    let mut names = Vec::new();
    let mut opts = Options { depth: 8.0, length: 16.0, all: false, cull: false, rects: false };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--depth" => opts.depth = args.next().and_then(|v| v.parse().ok()).unwrap_or(opts.depth),
            "--length" => opts.length = args.next().and_then(|v| v.parse().ok()).unwrap_or(opts.length),
            "--all" => opts.all = true,
            "--cull-backs" => opts.cull = true,
            "--rects" => {
                opts.rects = true;
                opts.cull = true;
            }
            _ => names.push(a),
        }
    }
    let levels: Vec<(String, PathBuf)> = if names.is_empty() {
        levels::list_levels().into_iter().map(|l| (l.1, l.2)).collect()
    } else {
        names.iter().map(|n| levels::find_level(n).map(|p| (n.clone(), p)).map_err(anyhow::Error::msg)).collect::<Result<_>>()?
    };
    for (name, path) in levels {
        match map::read(&path) {
            Ok(level) => report(&name, &level, &opts),
            Err(e) => println!("{name}: could not read ({e:#})"),
        }
    }
    Ok(())
}

struct Options {
    depth: f32,
    length: f32,
    all: bool,
    cull: bool,
    rects: bool,
}

type V3 = [f32; 3];

/// Objects that belong to no room still block the view
const NO_ROOM: u32 = u32::MAX;
const CELL: f32 = 128.0;
/// A triangle covering more cells than this is tested against everything instead
const BIG: i64 = 512;
const EYE: f32 = 165.0;
/// How far above or below its volume a floor can be
const SLAB: f32 = 20.0;
/// How far apart eyes stand in a room's volume, and how far they look
const EYE_SPACING: f32 = 96.0;
const SIGHT: f32 = 4000.0;
/// The most crossings of a pair of rooms to look for, longest first
const LOOKS: usize = 60;
/// The most eyes to try on a crossing, nearest first
const EYES_PER_CROSSING: usize = 400;
/// How far off the other surface to look at each arm of a crossing
const ARM: f32 = 6.0;
/// How far from a crossing a view of it is best, and the least
const VIEW: f32 = 400.0;
const MIN_VIEW: f32 = 200.0;

struct Tri {
    room: u32,
    object: usize,
    /// Alpha-tested foliage, fences and the like
    alpha: bool,
    material: usize,
    /// Shown from behind as well as in front, even when backs are culled
    two_sided: bool,
    /// The side it is seen from (its corners turn counterclockwise there)
    normal: V3,
    p: [V3; 3],
    lo: V3,
    hi: V3,
}

struct Crossing {
    s: usize,
    t: usize,
    length: f32,
    /// The ends of the line the two triangles share
    ends: [V3; 2],
}

struct Scene {
    tris: Vec<Tri>,
    grid: HashMap<[i32; 3], Vec<usize>>,
    big: Vec<usize>,
    /// Which query last tested each triangle
    stamp: Vec<u32>,
    query: u32,
    /// Faces seen from behind are not drawn
    cull: bool,
}

fn cell(p: V3) -> [i32; 3] {
    p.map(|c| (c / CELL).floor() as i32)
}

impl Scene {
    fn new(level: &map::Map, cull: bool) -> Self {
        let rooms: HashSet<u32> = level.rooms.rooms.iter().filter_map(|r| r.section_name.parse().ok()).collect();
        // Door leaves and glass panes are kept in their own space, away from where they are drawn
        let doors: HashSet<usize> = door::doors(level).iter().map(|d| d.model_index).collect();
        let panes: HashSet<String> = glass::panes(level).into_iter().map(|p| p.name).collect();
        let mut tris = Vec::new();
        for (i, o) in level.geometries.objects.iter().enumerate() {
            if doors.contains(&i) || panes.contains(&o.name) {
                continue;
            }
            let room = map::object_room(&o.name).filter(|r| rooms.contains(r)).unwrap_or(NO_ROOM);
            let v = |k: u16| o.vertices.get(k as usize).map(|v| [v.x, v.y, v.z]);
            for od in &o.object_datas {
                let material = level.materials.materials.get(od.mn as usize);
                let alpha = material.is_some_and(|m| m.filename.to_ascii_lowercase().ends_with(".tga"));
                let two_sided = material.is_some_and(|m| m.two_sided);
                for &(a, b, c) in &od.faces.face_indices {
                    let (Some(a), Some(b), Some(c)) = (v(a), v(b), v(c)) else { continue };
                    let lo = [0, 1, 2].map(|k| a[k].min(b[k]).min(c[k]));
                    let hi = [0, 1, 2].map(|k| a[k].max(b[k]).max(c[k]));
                    let normal = plane(&[a, b, c]).map_or([0.0; 3], |p| p.0);
                    tris.push(Tri { room, object: i, alpha, material: od.mn as usize, two_sided, normal, p: [a, b, c], lo, hi });
                }
            }
        }
        let mut grid: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
        let mut big = Vec::new();
        for (i, t) in tris.iter().enumerate() {
            let (a, b) = (cell(t.lo), cell(t.hi));
            if (0..3).map(|k| (b[k] - a[k] + 1) as i64).product::<i64>() > BIG {
                big.push(i);
                continue;
            }
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        grid.entry([x, y, z]).or_default().push(i);
                    }
                }
            }
        }
        let stamp = vec![0; tris.len()];
        Self { tris, grid, big, stamp, query: 0, cull }
    }

    /// Every pair of triangles of different rooms that cross
    fn crossings(&self, opts: &Options) -> Vec<Crossing> {
        let mut out = Vec::new();
        let overlap = |s: &Tri, t: &Tri| {
            let lo = [0, 1, 2].map(|k| s.lo[k].max(t.lo[k]));
            let hi = [0, 1, 2].map(|k| s.hi[k].min(t.hi[k]));
            (s.room != t.room && s.room != NO_ROOM && t.room != NO_ROOM && (0..3).all(|k| lo[k] <= hi[k])).then_some(lo)
        };
        let mut test = |i: usize, j: usize| {
            let (s, t) = (&self.tris[i], &self.tris[j]);
            if let Some((length, ends)) = crossing(&s.p, &t.p, opts.depth).filter(|c| c.0 >= opts.length) {
                out.push(Crossing { s: i, t: j, length, ends });
            }
        };
        for (key, list) in &self.grid {
            for (n, &i) in list.iter().enumerate() {
                for &j in &list[n + 1..] {
                    // Each pair once, in the cell where their boxes' overlap starts
                    if overlap(&self.tris[i], &self.tris[j]).is_some_and(|lo| cell(lo) == *key) {
                        test(i, j);
                    }
                }
            }
        }
        for (n, &i) in self.big.iter().enumerate() {
            for j in 0..self.tris.len() {
                if j != i && !self.big[..=n].contains(&j) && overlap(&self.tris[i], &self.tris[j]).is_some() {
                    test(i, j);
                }
            }
        }
        out
    }

    /// Whether triangle `i` is drawn seen from `from`
    fn shows(&self, i: usize, from: V3) -> bool {
        let t = &self.tris[i];
        !self.cull || t.two_sided || dot(t.normal, sub(from, t.p[0])) > 0.0
    }

    /// The nearest triangle the segment from `a` to `b` passes through: how far along (as a
    /// fraction of the way) and which, leaving out `skip`, alpha-tested ones, and ones seen
    /// from behind when backs are culled
    fn hit(&mut self, a: V3, b: V3, skip: [usize; 2]) -> Option<(f32, usize)> {
        self.query += 1;
        let q = self.query;
        let d = sub(b, a);
        let mut best: Option<(f32, usize)> = None;
        let cull = self.cull;
        let try_tri = |i: usize, stamp: &mut Vec<u32>, tris: &Vec<Tri>, best: &mut Option<(f32, usize)>| {
            let t = &tris[i];
            if stamp[i] == q || skip.contains(&i) || t.alpha || (cull && !t.two_sided && dot(t.normal, d) > 0.0) {
                return;
            }
            stamp[i] = q;
            if let Some(t) = segment_hits(a, d, &tris[i].p) {
                if best.is_none_or(|b| t < b.0) {
                    *best = Some((t, i));
                }
            }
        };
        for &i in &self.big {
            try_tri(i, &mut self.stamp, &self.tris, &mut best);
        }
        // Walk the cells along the segment (Amanatides and Woo)
        let mut c = cell(a);
        let end = cell(b);
        let mut t_max = [0.0f32; 3];
        let mut t_delta = [0.0f32; 3];
        let mut step = [0i32; 3];
        for k in 0..3 {
            if d[k].abs() < 1e-9 {
                t_max[k] = f32::INFINITY;
                t_delta[k] = f32::INFINITY;
            } else {
                step[k] = if d[k] > 0.0 { 1 } else { -1 };
                let edge = (c[k] + if d[k] > 0.0 { 1 } else { 0 }) as f32 * CELL;
                t_max[k] = (edge - a[k]) / d[k];
                t_delta[k] = CELL / d[k].abs();
            }
        }
        loop {
            if let Some(list) = self.grid.get(&c) {
                for &i in list {
                    try_tri(i, &mut self.stamp, &self.tris, &mut best);
                }
            }
            if c == end {
                break;
            }
            let k = if t_max[0] < t_max[1] { if t_max[0] < t_max[2] { 0 } else { 2 } } else if t_max[1] < t_max[2] { 1 } else { 2 };
            if t_max[k] > 1.0 {
                break;
            }
            // Hits found so far that come before the next cell are final
            if best.is_some_and(|b| b.0 < t_max[k]) {
                break;
            }
            c[k] += step[k];
            t_max[k] += t_delta[k];
        }
        best
    }

    /// The eyes of a player standing on the first floor below `top` (down to `bottom`)
    /// with room to stand: flat, under both feet, and with headroom. Wall tops and the
    /// tops of trunks and posts are passed through.
    fn stand(&mut self, top: V3, bottom: V3) -> Option<V3> {
        let mut from = top;
        for _ in 0..6 {
            let (t, i) = self.hit(from, bottom, [usize::MAX; 2])?;
            let floor = lerp(from, bottom, t);
            let flat = plane(&self.tris[i].p).is_some_and(|(n, _)| n[1].abs() > 0.7);
            let feet = [[16.0, 0.0], [-16.0, 0.0], [0.0, 16.0], [0.0, -16.0]].iter().all(|o| {
                let above = [floor[0] + o[0], floor[1] + 8.0, floor[2] + o[1]];
                let below = [above[0], floor[1] - 8.0, above[2]];
                self.hit(above, below, [usize::MAX; 2]).is_some()
            });
            let eye = [floor[0], floor[1] + EYE, floor[2]];
            if flat && feet && self.hit(floor_up(floor), eye, [usize::MAX; 2]).is_none() {
                return Some(eye);
            }
            from = [floor[0], floor[1] - 1.0, floor[2]];
        }
        None
    }

    /// Every triangle the segment from `a` to `b` passes through, nearest first: how far
    /// along (as a fraction of the way) and which, leaving out alpha-tested ones
    fn along(&mut self, a: V3, b: V3) -> Vec<(f32, usize)> {
        self.query += 1;
        let q = self.query;
        let d = sub(b, a);
        let mut out = Vec::new();
        let mut try_tri = |i: usize, stamp: &mut Vec<u32>, tris: &Vec<Tri>| {
            if stamp[i] == q || tris[i].alpha {
                return;
            }
            stamp[i] = q;
            if let Some(t) = segment_hits(a, d, &tris[i].p) {
                out.push((t, i));
            }
        };
        for &i in &self.big {
            try_tri(i, &mut self.stamp, &self.tris);
        }
        let mut c = cell(a);
        let end = cell(b);
        let mut t_max = [0.0f32; 3];
        let mut t_delta = [0.0f32; 3];
        let mut step = [0i32; 3];
        for k in 0..3 {
            if d[k].abs() < 1e-9 {
                t_max[k] = f32::INFINITY;
                t_delta[k] = f32::INFINITY;
            } else {
                step[k] = if d[k] > 0.0 { 1 } else { -1 };
                let edge = (c[k] + if d[k] > 0.0 { 1 } else { 0 }) as f32 * CELL;
                t_max[k] = (edge - a[k]) / d[k];
                t_delta[k] = CELL / d[k].abs();
            }
        }
        loop {
            if let Some(list) = self.grid.get(&c) {
                for &i in list {
                    try_tri(i, &mut self.stamp, &self.tris);
                }
            }
            if c == end {
                break;
            }
            let k = if t_max[0] < t_max[1] { if t_max[0] < t_max[2] { 0 } else { 2 } } else if t_max[1] < t_max[2] { 1 } else { 2 };
            if t_max[k] > 1.0 {
                break;
            }
            c[k] += step[k];
            t_max[k] += t_delta[k];
        }
        out.sort_by(|x, y| x.0.total_cmp(&y.0));
        out
    }

    /// Where a player's eyes could be: on a grid over each of the room's volumes, at eye
    /// height over the floor found below
    fn eyes(&mut self, level: &map::Map) -> Vec<(u32, V3)> {
        let mut out = Vec::new();
        for room in &level.rooms.rooms {
            let Ok(number) = room.section_name.parse::<u32>() else { continue };
            for b in room.sherman_levels.iter().flat_map(|l| &l.tm_with_aabb) {
                let a = &b.aabb;
                let (o, x, y, z) = (&b.tm.position, &b.tm.x_axis, &b.tm.y_axis, &b.tm.z_axis);
                let world = |l: V3| [
                    o.x + x.x * l[0] + y.x * l[1] + z.x * l[2],
                    o.y + x.y * l[0] + y.y * l[1] + z.y * l[2],
                    o.z + x.z * l[0] + y.z * l[1] + z.z * l[2],
                ];
                let steps = |lo: f32, hi: f32| (((hi - lo) / EYE_SPACING).floor() as usize).clamp(1, 24);
                let (nx, nz) = (steps(a[0], a[3]), steps(a[2], a[5]));
                for i in 0..nx {
                    for k in 0..nz {
                        let lx = a[0] + (a[3] - a[0]) * (i as f32 + 0.5) / nx as f32;
                        let lz = a[2] + (a[5] - a[2]) * (k as f32 + 0.5) / nz as f32;
                        // On the floor the volume holds, not on a roof over it
                        let top = world([lx, a[4] + SLAB, lz]);
                        let bottom = world([lx, a[1] - SLAB, lz]);
                        if let Some(eye) = self.stand(top, bottom) {
                            out.push((number, eye));
                        }
                    }
                }
            }
        }
        out
    }
}

/// A point on triangle `t`, a little way from `mid` (on the line where it crosses the plane
/// `n`, `d`) toward `side` of that plane
fn arm(t: &[V3; 3], mid: V3, n: V3, d: f32, side: f32) -> Option<V3> {
    let (v, h) = t.iter().map(|&v| (v, (dot(n, v) - d) * side)).max_by(|a, b| a.1.total_cmp(&b.1))?;
    if h <= 0.0 {
        return None;
    }
    // From the line toward that corner, until a few units off the plane
    let f = (ARM / h).min(0.5);
    Some(lerp(mid, v, f))
}

fn floor_up(p: V3) -> V3 {
    [p[0], p[1] + 1.0, p[2]]
}

/// One of six square views around an eye (90 degrees across), as a camera's axes
struct Face {
    forward: V3,
    right: V3,
    up: V3,
    /// From the slope off the forward axis to the screen's -1..1
    scale: [f32; 2],
}

fn faces() -> [Face; 6] {
    let axes: [V3; 6] = [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];
    axes.map(|forward| {
        let hint = if forward[1].abs() > 0.5 { [0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0] };
        let right = cross(forward, hint);
        let up = cross(right, forward);
        Face { forward, right, up, scale: [1.0, 1.0] }
    })
}

/// The game's camera: 60 degrees high on a 4:3 screen, turned as `--yaw` and `--pitch` turn it
/// (the camera works with z negated, as `--pos` takes it)
fn game_view(yaw: f32, pitch: f32, aspect: f32) -> Face {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    let forward = [-y.sin() * p.cos(), p.sin(), -y.cos() * p.cos()];
    let right = [y.cos(), 0.0, -y.sin()];
    let up = cross(right, forward);
    let flip = |v: V3| [v[0], v[1], -v[2]];
    let t = 30f32.to_radians().tan();
    Face { forward: flip(forward), right: flip(right), up: flip(up), scale: [1.0 / (t * aspect), 1.0 / t] }
}

/// What the default drawing does from one eye: the rooms it starts from, and each face's
/// rectangle plan
struct Seer {
    eye: V3,
    starts: Vec<u32>,
    faces: Vec<Face>,
    plans: Vec<Vec<RectRoom>>,
}

/// The game's side of the level: rooms, portals and floors. Doors are taken open: players
/// open them, and the door leaves are left out of the scene.
struct Game {
    visibility: Visibility,
    world: CollisionWorld,
    blocked: Vec<bool>,
}

impl Game {
    fn new(level: &map::Map) -> Self {
        let visibility = Visibility::new(level);
        let world = CollisionWorld::from_map(level, Params::default());
        let blocked = vec![false; level.portals.portals.len()];
        Self { visibility, world, blocked }
    }

    fn seer(&self, level: &map::Map, eye: V3) -> Option<Seer> {
        self.seer_through(level, eye, faces().into())
    }

    fn seer_through(&self, level: &map::Map, eye: V3, faces: Vec<Face>) -> Option<Seer> {
        let feet = [eye[0], eye[1] - EYE, eye[2]];
        let starts: Vec<u32> = self.world.floor_rooms(feet).into_iter().filter(|&n| self.visibility.is_room(n)).collect();
        if starts.is_empty() {
            return None;
        }
        let body = [[feet[0], feet[1] + 10.0, feet[2]], eye];
        let plans = faces.iter().map(|f| {
            let clip = |p: [f32; 3]| {
                let d = sub(p, eye);
                [dot(d, f.right) * f.scale[0], dot(d, f.up) * f.scale[1], 0.0, dot(d, f.forward)]
            };
            self.visibility.rect_plan(level, &starts, &body, eye, &self.blocked, &clip)
        }).collect();
        Some(Seer { eye, starts, faces, plans })
    }

    /// Whether the drawing shows triangle `own` at `point`, seen from the seer's eye
    fn shows(&self, scene: &mut Scene, seer: &Seer, own: usize, point: V3) -> bool {
        let to = sub(point, seer.eye);
        let distance = dot(to, to).sqrt();
        // Everything along the line of sight, a little past the point
        self.surface(scene, seer, to.map(|c| c / distance), distance + 300.0)
            .is_some_and(|(d, i)| i == own && (d - distance).abs() < 1.0)
    }

    /// What the drawing shows along `dir` (a unit vector) from the seer's eye, out to `reach`:
    /// how far, and which triangle
    fn surface(&self, scene: &mut Scene, seer: &Seer, dir: V3, reach: f32) -> Option<(f32, usize)> {
        let eye = seer.eye;
        let (f, face) = seer.faces.iter().enumerate().max_by(|a, b| dot(dir, a.1.forward).total_cmp(&dot(dir, b.1.forward))).unwrap();
        let w = dot(dir, face.forward);
        if w <= 0.0 {
            return None;
        }
        let at = [dot(dir, face.right) / w * face.scale[0], dot(dir, face.up) / w * face.scale[1]];
        let hits: Vec<(f32, usize)> = scene.along(eye, [eye[0] + dir[0] * reach, eye[1] + dir[1] * reach, eye[2] + dir[2] * reach]).into_iter()
            .map(|(t, i)| (t * reach, i))
            .filter(|&(_, i)| scene.shows(i, eye))
            .collect();
        let nearest = |room: u32| hits.iter().find(|&&(_, i)| scene.tris[i].room == room).copied();
        let mut depth: Option<(f32, usize)> = None;
        let draw = |room: u32, depth: &mut Option<(f32, usize)>| {
            if let Some(h) = nearest(room) {
                if depth.is_none_or(|d| h.0 < d.0) {
                    *depth = Some(h);
                }
            }
        };
        draw(NO_ROOM, &mut depth);
        for &r in &seer.starts {
            draw(r, &mut depth);
        }
        for rr in seer.plans[f].iter().filter(|rr| !seer.starts.contains(&rr.room)) {
            let r = rr.rect;
            if at[0] < r[0] || at[0] > r[2] || at[1] < r[1] || at[1] > r[3] {
                continue;
            }
            // The doorway wipes, before the room is drawn
            for &(p, plan) in &rr.doorways {
                let Some((through, _)) = depth else { break };
                let Some(opening) = crosses_polygon(eye, dir, self.visibility.polygon(p)) else { continue };
                if opening >= through {
                    continue;
                }
                let s = [eye[0] + dir[0] * through, eye[1] + dir[1] * through, eye[2] + dir[2] * through];
                let height = self.visibility.height(rr.room).map_or([f32::MIN, f32::MAX], |h| [h[0] - 0.25, h[1] - 1.0]);
                if s[0] >= plan[0] && s[0] <= plan[2] && s[2] >= plan[1] && s[2] <= plan[3] && s[1] >= height[0] && s[1] <= height[1] {
                    depth = None;
                }
            }
            draw(rr.room, &mut depth);
        }
        depth
    }
}

/// How far along `dir` from `from` the line crosses inside a convex polygon
fn crosses_polygon(from: V3, dir: V3, polygon: &[V3]) -> Option<f32> {
    if polygon.len() < 3 {
        return None;
    }
    let n = cross(sub(polygon[1], polygon[0]), sub(polygon[2], polygon[0]));
    let along = dot(n, dir);
    if along.abs() < 1e-9 {
        return None;
    }
    let t = dot(n, sub(polygon[0], from)) / along;
    if t <= 0.0 {
        return None;
    }
    let p = [from[0] + dir[0] * t, from[1] + dir[1] * t, from[2] + dir[2] * t];
    let mut sign = 0.0f32;
    for k in 0..polygon.len() {
        let (a, b) = (polygon[k], polygon[(k + 1) % polygon.len()]);
        let s = dot(cross(sub(b, a), sub(p, a)), n);
        if s.abs() < 1e-6 {
            continue;
        }
        if sign == 0.0 {
            sign = s.signum();
        } else if s.signum() != sign {
            return None;
        }
    }
    Some(t)
}

/// What two rooms' crossings add up to
#[derive(Default)]
struct Pair {
    crossings: usize,
    length: f32,
    /// The crossings seen, how long they are, the rooms they are seen from, and the nearest
    /// view of one: (distance, eye, point)
    seen: usize,
    seen_length: f32,
    /// Of those, the ones between different materials: the same material on both is mostly
    /// two walls overshooting at a corner, or two cells' ground meeting
    unlike: usize,
    from: BTreeSet<u32>,
    view: Option<(f32, V3, V3)>,
    objects: BTreeSet<(usize, usize)>,
}

fn report(name: &str, level: &map::Map, opts: &Options) {
    let mut scene = Scene::new(level, opts.cull);
    let crossings = scene.crossings(opts);
    let mut by_pair: BTreeMap<(u32, u32), Vec<&Crossing>> = BTreeMap::new();
    for c in &crossings {
        let (s, t) = (&scene.tris[c.s], &scene.tris[c.t]);
        if !(s.alpha || t.alpha) {
            by_pair.entry((s.room.min(t.room), s.room.max(t.room))).or_default().push(c);
        }
    }
    let eyes = scene.eyes(level);
    let game = opts.rects.then(|| Game::new(level));
    // OV_VIEW=x,y,z,yaw,pitch (as --pos, --yaw and --pitch take them): the object the drawing
    // shows at each pixel of a 32 by 24 grid of a 1024 by 768 frame, to check against the
    // game's --rooms screenshot
    if let (Ok(v), Some(g)) = (std::env::var("OV_VIEW"), &game) {
        let v: Vec<f32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
        let eye = [v[0], v[1], -v[2]];
        let face = game_view(v[3], v[4], 1024.0 / 768.0);
        let (fw, rt, up, sc) = (face.forward, face.right, face.up, face.scale);
        if let Some(seer) = g.seer_through(level, eye, vec![face]) {
            for gy in 0..24 {
                for gx in 0..32 {
                    let (px, py) = (gx * 32 + 16, gy * 32 + 16);
                    let (nx, ny) = ((px as f32 + 0.5) / 512.0 - 1.0, 1.0 - (py as f32 + 0.5) / 384.0);
                    let d = [0, 1, 2].map(|k| fw[k] + rt[k] * nx / sc[0] + up[k] * ny / sc[1]);
                    let len = dot(d, d).sqrt();
                    let name = g.surface(&mut scene, &seer, d.map(|c| c / len), SIGHT * 4.0)
                        .map_or("-".to_string(), |(_, i)| level.geometries.objects[scene.tris[i].object].name.clone());
                    println!("pixel {px} {py} {name}");
                }
            }
        }
        return;
    }
    // OV_EYE=x,y,z (as --pos takes it) and OV_PAIR=a,b: what the drawing shows of that pair's
    // crossings from there
    if let (Ok(e), Ok(pr), Some(g)) = (std::env::var("OV_EYE"), std::env::var("OV_PAIR"), &game) {
        let v: Vec<f32> = e.split(',').filter_map(|x| x.parse().ok()).collect();
        let eye = [v[0], v[1], -v[2]];
        let pr: Vec<u32> = pr.split(',').filter_map(|x| x.parse().ok()).collect();
        let seer = g.seer(level, eye);
        println!("starts {:?}", seer.as_ref().map(|s| s.starts.clone()));
        if let Some(seer) = seer {
            for (f, plan) in seer.plans.iter().enumerate() {
                println!("face {f}: {:?}", plan.iter().map(|r| (r.room, r.rect.map(|x| (x * 100.0).round() / 100.0), r.doorways.len())).collect::<Vec<_>>());
            }
            for c in &crossings {
                let (s, t) = (&scene.tris[c.s], &scene.tris[c.t]);
                if !((s.room == pr[0] && t.room == pr[1]) || (s.room == pr[1] && t.room == pr[0])) || s.alpha || t.alpha { continue; }
                let mid = lerp(c.ends[0], c.ends[1], 0.5);
                let mut shown = Vec::new();
                for (own, other) in [(c.s, c.t), (c.t, c.s)] {
                    let Some((n, d)) = plane(&scene.tris[other].p) else { continue };
                    for side in [1.0f32, -1.0] {
                        if let Some(point) = arm(&scene.tris[own].p, mid, n, d, side) {
                            shown.push((scene.tris[own].room, g.shows(&mut scene, &seer, own, point)));
                        }
                    }
                }
                println!("crossing at {:.0},{:.0},{:.0}: {:?}", mid[0], mid[1], -mid[2], shown);
            }
        }
    }
    let mut seers: HashMap<usize, Option<Seer>> = HashMap::new();

    let mut pairs: Vec<((u32, u32), Pair)> = Vec::new();
    for (key, mut list) in by_pair {
        let mut p = Pair::default();
        for c in &list {
            p.crossings += 1;
            p.length += c.length;
            let (s, t) = (&scene.tris[c.s], &scene.tris[c.t]);
            p.objects.insert(if s.room < t.room { (s.object, t.object) } else { (t.object, s.object) });
        }
        list.sort_by(|a, b| b.length.total_cmp(&a.length));
        for c in list.iter().take(LOOKS) {
            // Where two surfaces merely meet (a wall running into the ground), one arm of
            // each is hidden, inside the other's solid, from every eye. Where one eye sees
            // both arms of one of them and an arm of the other, it pokes through the other.
            let mid = lerp(c.ends[0], c.ends[1], 0.5);
            let mut arms = Vec::with_capacity(4);
            for (own, other) in [(c.s, c.t), (c.t, c.s)] {
                let Some((n, d)) = plane(&scene.tris[other].p) else { continue };
                for side in [1.0f32, -1.0] {
                    if let Some(point) = arm(&scene.tris[own].p, mid, n, d, side) {
                        arms.push((own, point));
                    }
                }
            }
            let mut near: Vec<(f32, u32, V3, usize)> = eyes.iter().enumerate()
                .map(|(k, &(room, eye))| (dot(sub(mid, eye), sub(mid, eye)).sqrt(), room, eye, k))
                .filter(|e| e.0 < SIGHT && e.0 > 1.0)
                .collect();
            // Nearest first, but from a little way off when it can be, for a view
            near.sort_by(|a, b| (a.0 < MIN_VIEW).cmp(&(b.0 < MIN_VIEW)).then(a.0.total_cmp(&b.0)));
            let mut visible = 0;
            let mut view = None;
            for &(distance, room, eye, k) in near.iter().take(EYES_PER_CROSSING) {
                let seer = match &game {
                    Some(g) => match seers.entry(k).or_insert_with(|| g.seer(level, eye)) {
                        Some(s) => Some(&*s),
                        None => continue,
                    },
                    None => None,
                };
                let mut seen = 0;
                for (k, &(own, point)) in arms.iter().enumerate() {
                    // Not enough arms left to make three
                    if seen + arms.len() - k < 3 {
                        break;
                    }
                    // Stop just short of the surface, so its neighbors do not hide it
                    let far = dot(sub(point, eye), sub(point, eye)).sqrt();
                    let to = lerp(eye, point, 1.0 - 0.5 / far);
                    let shown = match (&game, seer) {
                        (Some(g), Some(s)) => g.shows(&mut scene, s, own, point),
                        _ => scene.shows(own, eye) && scene.hit(eye, to, [own, usize::MAX]).is_none(),
                    };
                    if shown {
                        seen += 1;
                    }
                }
                if seen >= 3 {
                    visible = seen;
                    view = Some((distance, room, eye));
                    break;
                }
            }
            if visible >= 3 {
                p.seen += 1;
                if scene.tris[c.s].material != scene.tris[c.t].material {
                    p.unlike += 1;
                }
                p.seen_length += c.length;
                if let Some((distance, room, eye)) = view {
                    p.from.insert(room);
                    if p.view.is_none_or(|v| (distance - VIEW).abs() < (v.0 - VIEW).abs()) {
                        p.view = Some((distance, eye, mid));
                    }
                }
            }
        }
        if p.seen > 0 || opts.all {
            pairs.push((key, p));
        }
    }

    let joined: HashSet<(u32, u32)> = level.portals.portals.iter()
        .map(|p| (p.room.min(p.opposite_room), p.room.max(p.opposite_room)))
        .collect();
    let rooms = level.rooms.rooms.len();
    println!("{name}: {rooms} rooms, {} triangles, {} eyes, {} pairs of rooms cross where seen{}", scene.tris.len(), eyes.len(), pairs.iter().filter(|p| p.1.seen > 0).count(), if opts.rects { " (as the default drawing)" } else if opts.cull { " (backs culled)" } else { "" });
    // Unlike materials first
    pairs.sort_by(|a, b| (b.1.unlike > 0).cmp(&(a.1.unlike > 0)).then(b.1.seen_length.total_cmp(&a.1.seen_length)));
    for ((a, b), p) in pairs {
        let mut objects: Vec<String> = p.objects.iter().take(2).map(|&(x, y)| {
            format!("{} x {}", level.geometries.objects[x].name, level.geometries.objects[y].name)
        }).collect();
        if p.objects.len() > 2 {
            objects.push(format!("+{}", p.objects.len() - 2));
        }
        let from: Vec<String> = p.from.iter().map(u32::to_string).collect();
        println!(
            "  {a:>4} {b:>4} {:6} {:4} crossings {:6.0} long; seen {:3} ({:3} unlike), {:6.0} long, from {}  [{}]",
            if joined.contains(&(a, b)) { "portal" } else { "" },
            p.crossings, p.length, p.seen, p.unlike, p.seen_length, if from.is_empty() { "-".into() } else { from.join(",") }, objects.join(", ")
        );
        if let Some((_, eye, point)) = p.view {
            // --pos takes the eye with z negated; yaw and pitch as the camera has them
            let d = sub(point, eye);
            let len = dot(d, d).sqrt();
            let pitch = (d[1] / len).asin().to_degrees();
            let yaw = (-d[0]).atan2(d[2]).to_degrees();
            // Seen as the default drawing sees it, with the doors open; else with every room drawn
            let mode = if opts.rects { "--doors-open 1" } else if opts.cull { "--no-cull" } else { "--no-cull --no-cull-backs" };
            println!("        cargo rr --level {name} --pos {:.0},{:.0},{:.0} --yaw {yaw:.1} --pitch {pitch:.1} --first-person --textured {mode}", eye[0], eye[1], -eye[2]);
        }
    }
}

fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn lerp(a: V3, b: V3, t: f32) -> V3 { [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t] }

/// Where the segment from `a` along `d` passes through triangle `p`, as a fraction of `d`
fn segment_hits(a: V3, d: V3, p: &[V3; 3]) -> Option<f32> {
    let (e1, e2) = (sub(p[1], p[0]), sub(p[2], p[0]));
    let h = cross(d, e2);
    let det = dot(e1, h);
    if det.abs() < 1e-9 {
        return None;
    }
    let s = sub(a, p[0]);
    let u = dot(s, h) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(d, q) / det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, q) / det;
    (t > 1e-4 && t < 1.0).then_some(t)
}

/// Where triangle `t` crosses the plane (unit normal `n`, offset `d`): how far it reaches
/// past the plane on its shallower side, and the segment where it crosses
fn cut(t: &[V3; 3], n: V3, d: f32) -> Option<(f32, V3, V3)> {
    let s = t.map(|p| dot(n, p) - d);
    let reach = s.iter().cloned().fold(f32::MIN, f32::max).min(-s.iter().cloned().fold(f32::MAX, f32::min));
    if reach <= 0.0 {
        return None;
    }
    let mut ends = Vec::with_capacity(2);
    for k in 0..3 {
        let (a, b) = (k, (k + 1) % 3);
        if (s[a] > 0.0) != (s[b] > 0.0) {
            ends.push(lerp(t[a], t[b], s[a] / (s[a] - s[b])));
        }
    }
    (ends.len() == 2).then(|| (reach, ends[0], ends[1]))
}

fn plane(t: &[V3; 3]) -> Option<(V3, f32)> {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let len = dot(n, n).sqrt();
    (len > 1e-3).then(|| {
        let n = n.map(|c| c / len);
        (n, dot(n, t[0]))
    })
}

/// Whether two triangles cut through each other, each reaching past the other's plane by
/// more than `depth`: the length of the line they share, and its ends
fn crossing(s: &[V3; 3], t: &[V3; 3], depth: f32) -> Option<(f32, [V3; 2])> {
    let (ns, ds) = plane(s)?;
    let (nt, dt) = plane(t)?;
    let line = cross(ns, nt);
    let l2 = dot(line, line);
    if l2 < 1e-6 {
        return None;
    }
    let (rs, s0, s1) = cut(s, nt, dt)?;
    let (rt, t0, t1) = cut(t, ns, ds)?;
    if rs.min(rt) <= depth {
        return None;
    }
    let dir = line.map(|c| c / l2.sqrt());
    let (u, v) = (dot(dir, s0), dot(dir, s1));
    let (a0, a1) = (u.min(v), u.max(v));
    let (b0, b1) = { let (x, y) = (dot(dir, t0), dot(dir, t1)); (x.min(y), x.max(y)) };
    let (lo, hi) = (a0.max(b0), a1.min(b1));
    if hi <= lo || (v - u).abs() < 1e-6 {
        return None;
    }
    // The shared line's ends, found along s's segment
    let at = |x: f32| lerp(s0, s1, (x - u) / (v - u));
    Some((hi - lo, [at(lo), at(hi)]))
}
