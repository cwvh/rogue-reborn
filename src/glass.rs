//! Breakable glass panes.
//!
//! A MAP lists each pane twice: as a dynamic object (`Glass`, with the
//! transform that places it) and as a small quad in the geometry list under
//! the same name, in the object's own space. This module puts the quads in the
//! world and answers whether a ray hits one, and builds the falling shards for
//! a pane that has broken.

use crate::map::{self, DynamicObjectKind};

type V3 = [f32; 3];

#[derive(Clone, Debug)]
pub struct PaneTri {
    pub p: [V3; 3],
    pub uv: [[f32; 2]; 3],
}

#[derive(Clone, Debug)]
pub struct Pane {
    pub name: String,
    /// Index into the MAP's materials
    pub material: usize,
    /// Fraction of light let through (the material's opacity)
    pub opacity: f32,
    /// One side of the pane: the MAP stores both, for a two-sided material
    pub tris: Vec<PaneTri>,
}

impl Pane {
    pub fn center(&self) -> V3 {
        let mut c = [0.0; 3];
        let mut n: f32 = 0.0;
        for t in &self.tris {
            for p in t.p {
                for a in 0..3 {
                    c[a] += p[a];
                }
                n += 1.0;
            }
        }
        c.map(|v| v / n.max(1.0))
    }

    /// Distance along unit `dir` from `origin` to the pane, if the ray crosses it
    pub fn raycast(&self, origin: V3, dir: V3) -> Option<f32> {
        self.tris.iter()
            .filter_map(|t| crate::combat::ray_triangle(origin, dir, t.p[0], t.p[1], t.p[2]))
            .fold(None, |best: Option<f32>, d| Some(best.map_or(d, |b| b.min(d))))
    }

    /// Distance from `point` to the nearest corner or the centre, for blast radius checks
    pub fn distance_to(&self, point: V3) -> f32 {
        let dist = |a: V3| ((a[0] - point[0]).powi(2) + (a[1] - point[1]).powi(2) + (a[2] - point[2]).powi(2)).sqrt();
        self.tris.iter().flat_map(|t| t.p).map(dist).chain([dist(self.center())]).fold(f32::MAX, f32::min)
    }
}

/// Every glass pane in the level, placed in the world
pub fn panes(level: &map::Map) -> Vec<Pane> {
    let mut out = Vec::new();
    for d in &level.dynamic_objects.dynamic_objects {
        if !matches!(d.kind, DynamicObjectKind::Glass { .. }) {
            continue;
        }
        let Some(object) = level.geometries.objects.iter().find(|o| o.name == d.name) else { continue };
        let tm = &d.tm;
        let place = |v: &map::Vertex| -> V3 {
            [
                tm.x_axis.x * v.x + tm.y_axis.x * v.y + tm.z_axis.x * v.z + tm.position.x,
                tm.x_axis.y * v.x + tm.y_axis.y * v.y + tm.z_axis.y * v.z + tm.position.y,
                tm.x_axis.z * v.x + tm.y_axis.z * v.y + tm.z_axis.z * v.z + tm.position.z,
            ]
        };
        let mut tris = Vec::new();
        let mut seen: Vec<[u16; 3]> = Vec::new();
        let mut material = 0;
        for od in &object.object_datas {
            material = od.mn as usize;
            let n = od.faces.face_indices.len().min(od.faces.texture_indices.len());
            for k in 0..n {
                let (i0, i1, i2) = od.faces.face_indices[k];
                let mut key = [i0, i1, i2];
                key.sort_unstable();
                if seen.contains(&key) {
                    continue; // the back side of one already taken
                }
                seen.push(key);
                let (t0, t1, t2) = od.faces.texture_indices[k];
                let (Some(a), Some(b), Some(c)) = (
                    object.vertices.get(i0 as usize),
                    object.vertices.get(i1 as usize),
                    object.vertices.get(i2 as usize),
                ) else { continue };
                let uv = |t: u16| od.texture_vertices.uv_coords.get(t as usize).map_or([0.0; 2], |u| [u.u, u.v]);
                tris.push(PaneTri { p: [place(a), place(b), place(c)], uv: [uv(t0), uv(t1), uv(t2)] });
            }
        }
        let opacity = level.materials.materials.get(material).map_or(1.0, |m| m.opacity);
        out.push(Pane { name: d.name.clone(), material, opacity, tris });
    }
    out
}

// ---- Breaking ----------------------------------------------------------------

use crate::motion::{quat_mul, quat_rotate};

fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn mul(a: V3, s: f32) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 1e-9 { mul(a, 1.0 / l) } else { [0.0, 1.0, 0.0] }
}

/// A pane as a rectangle: `origin + x * u + y * v`, with unit, perpendicular
/// `u` and `v`, for `x` in `0..width` and `y` in `0..height` (map units)
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub origin: V3,
    pub u: V3,
    pub v: V3,
    pub width: f32,
    pub height: f32,
}

impl Pane {
    /// The pane's rectangle, found from its corners
    pub fn rect(&self) -> Rect {
        let mut pts: Vec<V3> = Vec::new();
        for t in &self.tris {
            for p in t.p {
                if !pts.iter().any(|q| dot(sub(*q, p), sub(*q, p)) < 1e-2) {
                    pts.push(p);
                }
            }
        }
        // From one corner, the two nearest others are its neighbours
        let o = pts[0];
        let mut others: Vec<V3> = pts[1..].to_vec();
        others.sort_by(|a, b| dot(sub(*a, o), sub(*a, o)).total_cmp(&dot(sub(*b, o), sub(*b, o))));
        let (a, b) = (sub(others[0], o), sub(others[1], o));
        let (width, height) = (dot(a, a).sqrt(), dot(b, b).sqrt());
        Rect { origin: o, u: norm(a), v: norm(b), width, height }
    }

    /// Texture coordinates at a point on the pane
    fn uv_at(&self, p: V3) -> [f32; 2] {
        let mut best = ([0.0; 2], f32::MIN);
        for t in &self.tris {
            let (v0, v1, v2) = (sub(t.p[1], t.p[0]), sub(t.p[2], t.p[0]), sub(p, t.p[0]));
            let (d00, d01, d11) = (dot(v0, v0), dot(v0, v1), dot(v1, v1));
            let (d20, d21) = (dot(v2, v0), dot(v2, v1));
            let den = d00 * d11 - d01 * d01;
            if den.abs() < 1e-9 { continue; }
            let b1 = (d11 * d20 - d01 * d21) / den;
            let b2 = (d00 * d21 - d01 * d20) / den;
            let b0 = 1.0 - b1 - b2;
            let worst = b0.min(b1).min(b2);
            if worst > best.1 {
                best = (
                    [
                        b0 * t.uv[0][0] + b1 * t.uv[1][0] + b2 * t.uv[2][0],
                        b0 * t.uv[0][1] + b1 * t.uv[1][1] + b2 * t.uv[2][1],
                    ],
                    worst,
                );
            }
        }
        best.0
    }
}

/// Gravity on falling glass, map units per second squared (about 9.8 m/s^2)
const GRAVITY: f32 = 980.0;
/// Seconds a shard lies on the floor before it fades
const REST_SECONDS: f32 = 6.0;
const FADE_SECONDS: f32 = 1.5;
/// Longest a piece flies without finding a floor
const MAX_AIR_SECONDS: f32 = 5.0;

/// One piece of a broken pane: a flat convex polygon tumbling through the air
#[derive(Clone, Debug)]
pub struct Shard {
    /// Corners around the middle of the piece, in its own space
    local: Vec<V3>,
    uv: Vec<[f32; 2]>,
    /// The pane's normal in the piece's own space
    normal: V3,
    pos: V3,
    vel: V3,
    rot: [f32; 4],
    /// Angular velocity, radians per second about each axis
    spin: V3,
    /// Seconds still stuck in the frame; the piece does not move until then
    hang: f32,
    /// Velocity it leaves the frame with
    release: V3,
    /// Seconds spent lying on the floor, once it has landed
    rest: Option<f32>,
    /// Seconds in the air; a piece that finds no floor fades after a while
    air: f32,
}

impl Shard {
    /// The corners in the world
    pub fn corners(&self) -> Vec<V3> {
        self.local.iter().map(|&l| add(self.pos, quat_rotate(self.rot, l))).collect()
    }

    pub fn uvs(&self) -> &[[f32; 2]] {
        &self.uv
    }

    /// 1 while solid, fading to 0 once it has lain on the floor a while
    pub fn alpha(&self) -> f32 {
        self.rest.map_or(1.0, |r| (1.0 - (r - REST_SECONDS) / FADE_SECONDS).clamp(0.0, 1.0))
    }

    pub fn gone(&self) -> bool {
        self.alpha() <= 0.0
    }

    /// Move for `dt` seconds. `ground(x, z, y)` is the height of the floor under a point.
    fn update(&mut self, dt: f32, ground: &dyn Fn(f32, f32, f32) -> Option<f32>) {
        if let Some(r) = &mut self.rest {
            *r += dt;
            return;
        }
        if self.hang > 0.0 {
            self.hang -= dt;
            if self.hang > 0.0 {
                return;
            }
            self.vel = self.release;
        }
        self.air += dt;
        if self.air > MAX_AIR_SECONDS {
            self.rest = Some(REST_SECONDS);
            return;
        }
        self.vel[1] -= GRAVITY * dt;
        self.pos = add(self.pos, mul(self.vel, dt));
        // Turn by the spin: q' = q + dt/2 * (spin, 0) * q
        let dq = quat_mul([self.spin[0], self.spin[1], self.spin[2], 0.0], self.rot);
        let q = [0, 1, 2, 3].map(|i| self.rot[i] + 0.5 * dt * dq[i]);
        let l = q.iter().map(|x| x * x).sum::<f32>().sqrt();
        self.rot = q.map(|x| x / l);

        if let Some(g) = ground(self.pos[0], self.pos[2], self.pos[1]) {
            let lowest = self.corners().iter().map(|c| c[1]).fold(f32::MAX, f32::min);
            if lowest <= g + 0.2 {
                self.pos[1] += g + 0.2 - lowest;
                if self.vel[1] < -90.0 {
                    // Bounce and skid, losing most of the energy
                    self.vel = [self.vel[0] * 0.5, -self.vel[1] * 0.25, self.vel[2] * 0.5];
                    self.spin = mul(self.spin, 0.5);
                } else {
                    self.settle(g);
                }
            }
        }
    }

    // Lie flat on the floor
    fn settle(&mut self, floor: f32) {
        let n = quat_rotate(self.rot, self.normal);
        let up = if n[1] >= 0.0 { [0.0, 1.0, 0.0] } else { [0.0, -1.0, 0.0] };
        let c = cross(n, up);
        let mut q = [c[0], c[1], c[2], 1.0 + dot(n, up)];
        let l = q.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
        q = q.map(|x| x / l);
        self.rot = quat_mul(q, self.rot);
        self.vel = [0.0; 3];
        self.spin = [0.0; 3];
        self.rest = Some(0.0);
        let lowest = self.corners().iter().map(|c| c[1]).fold(f32::MAX, f32::min);
        self.pos[1] += floor + 0.1 - lowest;
    }
}

/// A tiny glittering fragment thrown from the hole
#[derive(Clone, Debug)]
pub struct Fleck {
    pub pos: V3,
    vel: V3,
    life: f32,
    age: f32,
}

impl Fleck {
    /// 1 when new, 0 when it has burnt out
    pub fn alpha(&self) -> f32 {
        (1.0 - self.age / self.life).clamp(0.0, 1.0)
    }
}

/// A pane that has broken: its pieces and flecks, falling
#[derive(Clone, Debug)]
pub struct Fracture {
    pub material: usize,
    pub opacity: f32,
    pub shards: Vec<Shard>,
    pub flecks: Vec<Fleck>,
}

impl Fracture {
    /// Advance by `dt` seconds. `ground(x, z, y)` gives the floor height under a point.
    pub fn update(&mut self, dt: f32, ground: &dyn Fn(f32, f32, f32) -> Option<f32>) {
        for s in &mut self.shards {
            s.update(dt, ground);
        }
        self.shards.retain(|s| !s.gone());
        for f in &mut self.flecks {
            f.age += dt;
            f.vel[1] -= GRAVITY * dt;
            f.pos = add(f.pos, mul(f.vel, dt));
        }
        self.flecks.retain(|f| f.age < f.life);
    }

    pub fn done(&self) -> bool {
        self.shards.is_empty() && self.flecks.is_empty()
    }
}

/// Break a pane where a bullet went through at `hit`, travelling along unit `dir`.
///
/// The crack pattern is a spider web: spokes out from the impact and rings
/// around it, with tiny pieces at the middle and bigger ones towards the edges.
/// Pieces by the hole are blown out along the bullet's path, the rest drop with
/// a little push, and some of those at the frame hang on for a moment first.
pub fn shatter(pane: &Pane, hit: V3, dir: V3, mut seed: u64) -> Fracture {
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 40) as f32 / (1u64 << 24) as f32
    };
    let rect = pane.rect();
    let normal = norm(cross(rect.u, rect.v));
    let (w, h) = (rect.width, rect.height);
    let to_world = |x: f32, y: f32| add(rect.origin, add(mul(rect.u, x), mul(rect.v, y)));

    // Where the bullet went through, in the pane's own 2D space
    let rel = sub(hit, rect.origin);
    let impact = [dot(rel, rect.u).clamp(0.0, w), dot(rel, rect.v).clamp(0.0, h)];
    let far = [0.0, w]
        .iter()
        .flat_map(|&x| [0.0, h].map(move |y| ((x - impact[0]).powi(2) + (y - impact[1]).powi(2)).sqrt()))
        .fold(0.0, f32::max);

    // Rings out to the farthest corner, spaced further apart as they go
    let mut radii = vec![0.0];
    let mut r = 2.5 + rand() * 2.0;
    while r < far {
        radii.push(r);
        r = r * (1.35 + rand() * 0.3) + 1.5;
    }
    radii.push(r);
    let spokes = 14 + (rand() * 6.0) as usize;
    let base = rand() * std::f32::consts::TAU;
    let angles: Vec<f32> = (0..spokes)
        .map(|k| base + std::f32::consts::TAU * (k as f32 + (rand() - 0.5) * 0.5) / spokes as f32)
        .collect();
    // Ring crossings, nudged so the cracks are not perfectly regular
    let mut grid: Vec<Vec<[f32; 2]>> = Vec::new();
    for (j, &rad) in radii.iter().enumerate() {
        let mut ring = Vec::new();
        for &a in &angles {
            ring.push(if j == 0 {
                impact
            } else {
                let (a, rad) = (a + (rand() - 0.5) * 0.16, rad * (1.0 + (rand() - 0.5) * 0.2));
                [impact[0] + a.cos() * rad, impact[1] + a.sin() * rad]
            });
        }
        grid.push(ring);
    }

    let mut cells: Vec<Vec<[f32; 2]>> = Vec::new();
    for j in 0..radii.len() - 1 {
        for k in 0..spokes {
            let k1 = (k + 1) % spokes;
            cells.push(if j == 0 {
                vec![grid[0][0], grid[1][k], grid[1][k1]]
            } else {
                vec![grid[j][k], grid[j][k1], grid[j + 1][k1], grid[j + 1][k]]
            });
        }
    }

    let mut shards = Vec::new();
    for cell in cells {
        let poly = clip_to_rect(cell, w, h);
        if poly.len() < 3 || area(&poly) < 0.4 {
            continue;
        }
        let at_edge = poly.iter().any(|p| p[0] < 0.01 || p[0] > w - 0.01 || p[1] < 0.01 || p[1] > h - 0.01);
        let world: Vec<V3> = poly.iter().map(|p| to_world(p[0], p[1])).collect();
        let centre = mul(world.iter().fold([0.0; 3], |a, &p| add(a, p)), 1.0 / world.len() as f32);
        let n = poly.len() as f32;
        let c2 = [poly.iter().map(|p| p[0]).sum::<f32>() / n, poly.iter().map(|p| p[1]).sum::<f32>() / n];
        let d = ((c2[0] - impact[0]).powi(2) + (c2[1] - impact[1]).powi(2)).sqrt();
        let from_hole = norm(sub(centre, hit));
        let in_plane = norm(sub(from_hole, mul(normal, dot(from_hole, normal))));
        let jitter = |s: f32, rand: &mut dyn FnMut() -> f32| [(rand() - 0.5) * s, (rand() - 0.5) * s, (rand() - 0.5) * s];

        let blown = d < 12.0;
        let closeness = (1.0 - d / far.max(1.0)).clamp(0.0, 1.0);
        let (vel, spin_rate) = if blown {
            // Punched out along the bullet's path
            let speed = 350.0 + rand() * 450.0;
            let v = add(add(mul(dir, speed), mul(in_plane, 60.0 + rand() * 120.0)), jitter(160.0, &mut rand));
            (v, 8.0 + rand() * 18.0)
        } else {
            let push = 20.0 + closeness * 160.0;
            let v = add(add(mul(dir, push * (0.3 + rand())), mul(in_plane, push * 0.5 * rand())), jitter(30.0, &mut rand));
            (v, 1.0 + rand() * 7.0)
        };
        // Pieces at the frame often stay put for a moment, then slip out
        let hang = if at_edge && !blown && rand() < 0.6 { 0.1 + rand() * 1.4 } else { 0.0 };
        let (vel, release) = if hang > 0.0 {
            ([0.0; 3], add(mul(dir, 15.0), jitter(20.0, &mut rand)))
        } else {
            (vel, [0.0; 3])
        };
        let axis = norm([rand() - 0.5, rand() - 0.5, rand() - 0.5]);

        shards.push(Shard {
            uv: world.iter().map(|&p| pane.uv_at(p)).collect(),
            local: world.iter().map(|&p| sub(p, centre)).collect(),
            normal,
            pos: centre,
            vel,
            rot: [0.0, 0.0, 0.0, 1.0],
            spin: mul(axis, spin_rate),
            hang,
            release,
            rest: None,
            air: 0.0,
        });
    }

    // A spray of tiny bright fragments from the hole
    let mut flecks = Vec::new();
    for _ in 0..70 {
        let off = [(rand() - 0.5) * 12.0, (rand() - 0.5) * 12.0];
        let speed = 250.0 + rand() * 650.0;
        let scatter = [(rand() - 0.5) * 300.0, (rand() - 0.5) * 300.0 + 40.0, (rand() - 0.5) * 300.0];
        flecks.push(Fleck {
            pos: to_world(impact[0] + off[0], impact[1] + off[1]),
            vel: add(mul(dir, speed), scatter),
            life: 0.5 + rand() * 0.9,
            age: 0.0,
        });
    }

    Fracture { material: pane.material, opacity: pane.opacity, shards, flecks }
}

fn area(poly: &[[f32; 2]]) -> f32 {
    let mut a = 0.0;
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        a += p[0] * q[1] - q[0] * p[1];
    }
    a.abs() / 2.0
}

// Sutherland-Hodgman: the part of a polygon inside the rectangle 0..w by 0..h
fn clip_to_rect(mut poly: Vec<[f32; 2]>, w: f32, h: f32) -> Vec<[f32; 2]> {
    // (axis, limit, keep values below the limit?)
    for (axis, limit, below) in [(0, 0.0, false), (0, w, true), (1, 0.0, false), (1, h, true)] {
        let inside = |p: [f32; 2]| if below { p[axis] <= limit } else { p[axis] >= limit };
        let input = std::mem::take(&mut poly);
        for i in 0..input.len() {
            let (a, b) = (input[i], input[(i + 1) % input.len()]);
            let (ia, ib) = (inside(a), inside(b));
            if ia != ib {
                let t = (limit - a[axis]) / (b[axis] - a[axis]);
                let mut p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                p[axis] = limit;
                poly.push(p);
            }
            if ib {
                poly.push(b);
            }
        }
        if poly.is_empty() {
            break;
        }
    }
    poly
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn citystreet() -> Vec<Pane> {
        panes(&map::read(Path::new("data/map/m00/citystreet_large.map")).unwrap())
    }

    fn window() -> Pane {
        citystreet().into_iter().find(|p| p.name == "02_glass").unwrap()
    }

    #[test]
    fn citystreet_windows_are_placed_where_their_objects_are() {
        let panes = citystreet();
        assert_eq!(panes.len(), 7);
        let w = window();
        assert_eq!(w.tris.len(), 2);
        let c = w.center();
        assert!((c[0] - 32267.0).abs() < 1.0 && (c[1] - 33467.0).abs() < 1.0 && (c[2] - 34727.0).abs() < 1.0, "{c:?}");
        // A ray straight at the pane's centre hits it, one beside it misses
        let hit = w.raycast([c[0], c[1], c[2] + 500.0], [0.0, 0.0, -1.0]).unwrap();
        assert!((hit - 500.0).abs() < 1.0);
        assert!(w.raycast([c[0] + 1000.0, c[1], c[2] + 500.0], [0.0, 0.0, -1.0]).is_none());
    }

    #[test]
    fn the_rectangle_is_the_panes_size() {
        let r = window().rect();
        let (a, b) = (r.width.min(r.height), r.width.max(r.height));
        assert!((a - 100.0).abs() < 0.1 && (b - 200.0).abs() < 0.1, "{a} x {b}");
        assert!(dot(r.u, r.v).abs() < 1e-3);
    }

    #[test]
    fn the_pieces_add_up_to_the_pane() {
        let pane = window();
        let r = pane.rect();
        let normal = norm(cross(r.u, r.v));
        // One hit in the middle and one near a corner
        for (i, hit) in [pane.center(), add(r.origin, mul(r.u, 5.0))].into_iter().enumerate() {
            let f = shatter(&pane, hit, [0.0, 0.0, -1.0], 7 + i as u64);
            // A hit in the middle breaks into more pieces than one in the corner, where most of the web is off the pane
            assert!(f.shards.len() > if i == 0 { 50 } else { 25 }, "{} shards", f.shards.len());
            let sum: f32 = f.shards.iter().map(|s| {
                let c = s.corners();
                (1..c.len() - 1).map(|k| dot(cross(sub(c[k], c[0]), sub(c[k + 1], c[0])), normal).abs() / 2.0).sum::<f32>()
            }).sum();
            let whole = r.width * r.height;
            assert!((sum - whole).abs() < whole * 0.01, "pieces cover {sum} of {whole}");
            // Nothing sticks out past the pane
            for s in &f.shards {
                for c in s.corners() {
                    let rel = sub(c, r.origin);
                    assert!(dot(rel, r.u) > -0.1 && dot(rel, r.u) < r.width + 0.1, "x out of bounds");
                    assert!(dot(rel, r.v) > -0.1 && dot(rel, r.v) < r.height + 0.1, "y out of bounds");
                }
            }
        }
    }

    #[test]
    fn pieces_fall_and_lie_on_the_floor_then_fade() {
        let pane = window();
        let floor = pane.center()[1] - 150.0;
        let ground = move |_: f32, _: f32, _: f32| Some(floor);
        let mut f = shatter(&pane, pane.center(), [0.0, 0.0, -1.0], 3);
        let n = f.shards.len();
        for _ in 0..(4.0 * 60.0) as usize {
            f.update(1.0 / 60.0, &ground);
        }
        assert_eq!(f.shards.len(), n, "pieces vanished before they landed");
        assert!(f.shards.iter().all(|s| s.rest.is_some()), "some are still in the air");
        for s in &f.shards {
            for c in s.corners() {
                assert!(c[1] >= floor - 0.5, "below the floor by {}", floor - c[1]);
            }
        }
        for _ in 0..(10.0 * 60.0) as usize {
            f.update(1.0 / 60.0, &ground);
        }
        assert!(f.done());
    }
}
