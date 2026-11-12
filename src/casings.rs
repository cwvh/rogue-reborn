//! Spent casings thrown out of a gun as it fires. They fall and bounce on the level.
//!
//! The original draws no casings. It only has a shell sound per weapon.
//! Each client makes its own casings from the shots it is told of.
//! They are for looks only. They are not sent and not kept in the sim.
//! A map unit is about a centimeter.

use crate::collision::CollisionWorld;
use crate::util::next_rng;
use crate::weapon::Weapon;

type V3 = [f32; 3];
/// Column-major, as OpenGL keeps it
pub type Mat4 = [f32; 16];

pub const GRAVITY: f32 = 980.0;
/// How far the center stays from a surface
const RADIUS: f32 = 0.6;
/// Half the thickness: the center's height over the floor at rest
const REST_HEIGHT: f32 = 0.5;
pub const WIDTH: f32 = 1.0;
const BOUNCINESS: f32 = 0.35;
const GRIP: f32 = 0.7;
const SETTLES_BELOW: f32 = 40.0;
const MOST_BOUNCES: u32 = 3;
/// Slowdown on the floor, per second
const FRICTION: f32 = 12.0;
pub const REST_SECONDS: f32 = 8.0;
pub const FADE_SECONDS: f32 = 1.0;
/// Casings still flying after this long fell out of the level
const MOST_FLIGHT_SECONDS: f32 = 12.0;
pub const MOST: usize = 96;
/// Revolutions per second of the tumble in the air
const SPIN_RANGE: (f32, f32) = (15.0, 30.0);

fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn mul(a: V3, s: f32) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn len(a: V3) -> f32 { dot(a, a).sqrt() }

pub fn apply(m: &Mat4, p: V3) -> V3 {
    add(apply_dir(m, p), [m[12], m[13], m[14]])
}

pub fn apply_dir(m: &Mat4, d: V3) -> V3 {
    [m[0] * d[0] + m[4] * d[1] + m[8] * d[2], m[1] * d[0] + m[5] * d[1] + m[9] * d[2], m[2] * d[0] + m[6] * d[1] + m[10] * d[2]]
}

pub fn invert_affine(m: &Mat4) -> Option<Mat4> {
    let a = |r: usize, c: usize| m[c * 4 + r];
    let cof = |r: usize, c: usize| {
        let (r1, r2, c1, c2) = ((r + 1) % 3, (r + 2) % 3, (c + 1) % 3, (c + 2) % 3);
        a(r1, c1) * a(r2, c2) - a(r1, c2) * a(r2, c1)
    };
    let det = a(0, 0) * cof(0, 0) + a(0, 1) * cof(0, 1) + a(0, 2) * cof(0, 2);
    if det.abs() < 1e-12 {
        return None;
    }
    let mut out = [0.0f32; 16];
    // The adjugate is the transposed cofactors. The inverse is adjugate / det.
    for r in 0..3 {
        for c in 0..3 {
            out[c * 4 + r] = cof(c, r) / det;
        }
    }
    let t = apply_dir(&out, [m[12], m[13], m[14]]);
    (out[12], out[13], out[14], out[15]) = (-t[0], -t[1], -t[2], 1.0);
    Some(out)
}

/// Where a weapon throws its casings from, in its model's space (forward -Y, up +Z).
/// The port's x is how far out to the gun's right it sits.
/// Which way the right is depends on how the gun is drawn (see `Casings::eject`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ejector {
    pub port: V3,
    pub length: f32,
}

impl Ejector {
    /// None for a shotgun, which ejects nothing as it fires.
    /// The port sits back from the muzzle by an amount that depends on the weapon class.
    pub fn of(w: &Weapon) -> Option<Ejector> {
        w.shell_sound.as_ref()?;
        let (back, length) = match w.class.as_str() {
            "pistol" | "machine pistol" => (7.0, 2.0),
            "long arm" | "sniper" => (42.0, 4.5),
            "compact" => (12.0, 2.0),
            _ => (20.0, 2.0),
        };
        Some(Ejector { port: [w.muzzle[0] + 1.5, w.muzzle[1] + back, w.muzzle[2] + 1.0], length })
    }
}

/// A casing's first strike on a floor, where its sound plays
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    pub at: V3,
    pub owner: usize,
}

#[derive(Clone, Debug)]
pub struct Casing {
    pub pos: V3,
    vel: V3,
    /// Degrees about the vertical, then end over end
    pub yaw: f32,
    pub pitch: f32,
    /// Degrees per second
    spin: f32,
    pub length: f32,
    pub owner: usize,
    bounces: u32,
    landed: bool,
    resting: bool,
    /// Seconds in flight, restarted once it comes to rest
    since: f32,
}

impl Casing {
    pub fn is_resting(&self) -> bool {
        self.resting
    }

    pub fn scale(&self) -> f32 {
        if self.resting { (1.0 - (self.since - REST_SECONDS) / FADE_SECONDS).clamp(0.0, 1.0) } else { 1.0 }
    }

    pub fn done(&self) -> bool {
        if self.resting { self.since >= REST_SECONDS + FADE_SECONDS } else { self.since >= MOST_FLIGHT_SECONDS }
    }

    // Width, thickness and length directions in map space
    fn axes(&self) -> [V3; 3] {
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        let (sy, cy) = self.yaw.to_radians().sin_cos();
        let turn = |v: V3| {
            let v = [v[0], v[1] * cp - v[2] * sp, v[1] * sp + v[2] * cp];
            [v[0] * cy + v[2] * sy, v[1], -v[0] * sy + v[2] * cy]
        };
        [turn([1.0, 0.0, 0.0]), turn([0.0, 1.0, 0.0]), turn([0.0, 0.0, 1.0])]
    }

    fn touch(&mut self, normal: V3, h: f32, landings: &mut Vec<Landing>) {
        let into = -dot(self.vel, normal);
        let floor = normal[1] > 0.7;
        if floor && !self.landed && into > 20.0 {
            self.landed = true;
            landings.push(Landing { at: sub(self.pos, mul(normal, RADIUS)), owner: self.owner });
        }
        if into <= 0.0 {
            return;
        }
        let along = add(self.vel, mul(normal, into));
        if floor && (self.bounces >= MOST_BOUNCES || into < SETTLES_BELOW) {
            self.vel = mul(along, (-FRICTION * h).exp());
            self.spin = 0.0;
            if len(self.vel) < 8.0 {
                self.vel = [0.0; 3];
                self.pos = sub(self.pos, mul(normal, RADIUS - REST_HEIGHT));
                self.resting = true;
                self.since = 0.0;
            }
        } else {
            self.vel = add(mul(along, GRIP), mul(normal, into * BOUNCINESS));
            self.spin *= 0.5;
            self.bounces += 1;
        }
    }

    pub fn faces(&self) -> [([V3; 4], V3); 6] {
        let axes = self.axes();
        let s = self.scale();
        let half = [WIDTH / 2.0 * s, WIDTH / 2.0 * s, self.length / 2.0 * s];
        let mut faces = [([[0.0; 3]; 4], [0.0; 3]); 6];
        for (i, face) in faces.iter_mut().enumerate() {
            let (k, sign) = (i / 2, if i % 2 == 0 { 1.0 } else { -1.0 });
            let (u, v) = ((k + 1) % 3, (k + 2) % 3);
            let normal = mul(axes[k], sign);
            let center = add(self.pos, mul(normal, half[k]));
            let corner = |a: f32, b: f32| add(center, add(mul(axes[u], a * half[u]), mul(axes[v], b * half[v])));
            *face = ([corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)], normal);
        }
        faces
    }
}

pub struct Casings {
    list: Vec<Casing>,
    rng: u64,
}

impl Default for Casings {
    fn default() -> Self {
        Casings { list: Vec::new(), rng: 0x9E37_79B9_7F4A_7C15 }
    }
}

impl Casings {
    pub fn iter(&self) -> impl Iterator<Item = &Casing> {
        self.list.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn clear(&mut self) {
        self.list.clear();
    }

    fn between(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (next_rng(&mut self.rng) >> 40) as f32 / (1u64 << 24) as f32 * (hi - lo)
    }

    /// `gun` takes weapon space to view space and `to_world` takes view space to map space.
    /// The held gun is drawn in a narrower view than the world.
    /// Its offsets from the view's middle are `widen` times wider in the world's view.
    /// Any other gun uses 1.
    pub fn eject(&mut self, gun: &Mat4, to_world: &Mat4, ejector: &Ejector, owner: usize, widen: f32) {
        // The models are made for a left-handed world.
        // A flipped gun (as a character holds it) has its right at +X.
        // An unflipped gun (as the held gun is) has its right at -X.
        let det = gun[0] * (gun[5] * gun[10] - gun[9] * gun[6]) - gun[4] * (gun[1] * gun[10] - gun[9] * gun[2]) + gun[8] * (gun[1] * gun[6] - gun[5] * gun[2]);
        let right = if det < 0.0 { 1.0 } else { -1.0 };
        let mut p = apply(gun, [right * ejector.port[0], ejector.port[1], ejector.port[2]]);
        p[0] *= widen;
        p[1] *= widen;
        let pos = apply(to_world, p);
        // Out to the right, up and a little back, with some spread
        let faster = self.between(0.8, 1.2);
        let out = [right * self.between(180.0, 260.0), self.between(30.0, 70.0), self.between(100.0, 170.0)];
        let mut v = mul(out, faster);
        let wobble = len(v) * 0.1;
        for a in &mut v {
            *a += self.between(-wobble, wobble);
        }
        let vel = apply_dir(to_world, apply_dir(gun, v));
        let turns = self.between(SPIN_RANGE.0, SPIN_RANGE.1) * if self.between(0.0, 1.0) < 0.5 { -360.0 } else { 360.0 };
        let (yaw, pitch) = (self.between(0.0, 360.0), self.between(0.0, 360.0));
        if self.list.len() >= MOST {
            self.list.remove(0);
        }
        self.list.push(Casing { pos, vel, yaw, pitch, spin: turns, length: ejector.length, owner, bounces: 0, landed: false, resting: false, since: 0.0 });
    }

    pub fn update(&mut self, dt: f32, world: &CollisionWorld) -> Vec<Landing> {
        let mut landings = Vec::new();
        for c in &mut self.list {
            c.since += dt;
            if c.resting {
                // Settle flat, whichever way up it landed
                let goal = (c.pitch / 180.0).round() * 180.0;
                c.pitch += (goal - c.pitch) * (1.0 - (-30.0 * dt).exp());
                continue;
            }
            c.pitch += c.spin * dt;
            let mut left = dt;
            while left > 0.0 && !c.resting {
                // Small steps, so it cannot pass through a thin floor
                let h = left.min(1.0 / 120.0);
                left -= h;
                c.vel[1] -= GRAVITY * h;
                let step = mul(c.vel, h);
                let travel = len(step);
                if travel < 1e-4 {
                    continue;
                }
                let dir = mul(step, 1.0 / travel);
                match world.grenade_hit(c.pos, dir, travel + RADIUS) {
                    Some(hit) => {
                        c.pos = add(c.pos, mul(dir, (hit.dist - RADIUS).clamp(0.0, travel)));
                        c.touch(hit.normal, h, &mut landings);
                    }
                    None => {
                        c.pos = add(c.pos, step);
                        // A step along the floor can graze it without a hit. Stay above it.
                        if let Some(hit) = world.grenade_hit(c.pos, [0.0, -1.0, 0.0], RADIUS) {
                            c.pos[1] += (RADIUS - hit.dist) * hit.normal[1].abs();
                            c.touch(hit.normal, h, &mut landings);
                        }
                    }
                }
            }
        }
        self.list.retain(|c| !c.done());
        landings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gun_at(height: f32) -> Mat4 {
        let mut m = [0.0; 16];
        (m[0], m[5], m[10], m[15]) = (1.0, 1.0, 1.0, 1.0);
        m[13] = height;
        m
    }

    // Weapon space is forward -Y, up +Z. Map space is Y up.
    // This turns one into the other with no flip, so the gun's right is -X.
    fn weapon_to_map() -> Mat4 {
        // (x, y, z) in the weapon -> (x, z, -y) in the map
        let mut m = [0.0; 16];
        (m[0], m[6], m[9], m[15]) = (1.0, -1.0, 1.0, 1.0);
        m
    }

    fn ejector() -> Ejector {
        Ejector { port: [1.5, -31.0, 14.0], length: 2.0 }
    }

    fn identity() -> Mat4 {
        gun_at(0.0)
    }

    fn fired(rounds: usize) -> Casings {
        let mut c = Casings::default();
        let gun = weapon_to_map();
        for _ in 0..rounds {
            let mut g = gun;
            g[13] = 150.0;
            c.eject(&g, &identity(), &ejector(), 0, 1.0);
        }
        c
    }

    fn run(c: &mut Casings, world: &CollisionWorld, seconds: f32) -> Vec<Landing> {
        let mut landings = Vec::new();
        for _ in 0..(seconds * 60.0) as usize {
            landings.extend(c.update(1.0 / 60.0, world));
        }
        landings
    }

    #[test]
    fn the_inverse_undoes_the_matrix() {
        let mut m = [0.0; 16];
        let (s, c) = 30f32.to_radians().sin_cos();
        (m[0], m[2], m[8], m[10], m[15]) = (c, -s, s, -c, 1.0);
        m[5] = 1.0;
        (m[12], m[13], m[14]) = (100.0, -20.0, 7.0);
        let inv = invert_affine(&m).unwrap();
        let p = [3.0, 4.0, 5.0];
        let back = apply(&inv, apply(&m, p));
        for k in 0..3 {
            assert!((back[k] - p[k]).abs() < 1e-3, "{back:?}");
        }
    }

    #[test]
    fn a_casing_arcs_lands_bounces_and_comes_to_rest_to_the_right() {
        let world = CollisionWorld::flat_floor(0.0, 5000.0);
        let mut c = fired(1);
        let start = c.iter().next().unwrap().pos;
        assert!((start[1] - 164.0).abs() < 0.1 && (start[0] + 1.5).abs() < 0.1, "{start:?}");
        run(&mut c, &world, 0.05);
        assert!(c.iter().next().unwrap().pos[1] > start[1], "{:?}", c.iter().next().unwrap().pos);
        let landings = run(&mut c, &world, 2.0);
        assert_eq!(landings.len(), 1);
        let casing = c.iter().next().unwrap();
        assert!(casing.is_resting(), "{:?}", casing.pos);
        assert!((casing.pos[1] - REST_HEIGHT).abs() < 0.2, "{:?}", casing.pos);
        assert!(casing.pos[0] < -20.0, "{:?}", casing.pos);
        assert_eq!(landings[0].owner, 0);
        assert!(landings[0].at[1].abs() < 1.0, "{:?}", landings[0].at);
    }

    #[test]
    fn a_flipped_gun_throws_to_the_other_side() {
        let world = CollisionWorld::flat_floor(0.0, 5000.0);
        let mut c = Casings::default();
        let mut gun = [0.0; 16];
        (gun[0], gun[6], gun[9], gun[15]) = (1.0, 1.0, 1.0, 1.0);
        gun[13] = 150.0;
        c.eject(&gun, &identity(), &ejector(), 0, 1.0);
        assert!((c.iter().next().unwrap().pos[0] - 1.5).abs() < 0.1);
        run(&mut c, &world, 2.5);
        assert!(c.iter().next().unwrap().pos[0] > 20.0);
    }

    #[test]
    fn it_bounces_a_few_times_before_it_settles() {
        let world = CollisionWorld::flat_floor(0.0, 5000.0);
        let mut c = fired(1);
        let mut peaks = 0;
        let mut was_rising = false;
        let mut last = c.iter().next().unwrap().pos[1];
        for _ in 0..90 {
            c.update(1.0 / 60.0, &world);
            let y = c.iter().next().unwrap().pos[1];
            if was_rising && y < last {
                peaks += 1;
            }
            was_rising = y > last;
            last = y;
        }
        // The throw's peak, then at least one bounce
        assert!(peaks >= 2, "{peaks}");
    }

    #[test]
    fn the_oldest_is_recycled_at_the_cap() {
        let mut c = Casings::default();
        let gun = gun_at(100.0);
        for owner in 0..MOST + 5 {
            c.eject(&gun, &identity(), &ejector(), owner, 1.0);
        }
        assert_eq!(c.len(), MOST);
        assert_eq!(c.iter().next().unwrap().owner, 5);
        assert_eq!(c.iter().last().unwrap().owner, MOST + 4);
    }

    #[test]
    fn it_shrinks_after_its_rest_and_is_gone_after_the_fade() {
        let world = CollisionWorld::flat_floor(0.0, 5000.0);
        let mut c = fired(1);
        while !c.iter().next().unwrap().is_resting() {
            c.update(1.0 / 60.0, &world);
        }
        run(&mut c, &world, REST_SECONDS - 0.2);
        assert_eq!(c.iter().next().unwrap().scale(), 1.0);
        run(&mut c, &world, 0.5);
        let s = c.iter().next().unwrap().scale();
        assert!(s < 1.0 && s > 0.0, "{s}");
        run(&mut c, &world, FADE_SECONDS + 0.2);
        assert!(c.is_empty());
    }

    #[test]
    fn a_resting_casing_lies_flat() {
        let world = CollisionWorld::flat_floor(0.0, 5000.0);
        let mut c = fired(1);
        run(&mut c, &world, 3.0);
        let casing = c.iter().next().unwrap();
        for (corners, _) in casing.faces() {
            for p in corners {
                assert!(p[1] > -0.05 && p[1] < 1.05, "{p:?}");
            }
        }
    }

    #[test]
    fn faces_make_a_closed_box_with_outward_normals() {
        let c = fired(1);
        let casing = c.iter().next().unwrap();
        for (corners, normal) in casing.faces() {
            let middle = mul(add(add(corners[0], corners[1]), add(corners[2], corners[3])), 0.25);
            assert!(dot(sub(middle, casing.pos), normal) > 0.0);
            assert!((len(normal) - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn a_shotgun_ejects_nothing_and_a_machine_gun_does() {
        let Ok(mp5k) = crate::weapon::read(std::path::Path::new("data/kit/pw_mp5k.wpn")) else { return };
        let e = Ejector::of(&mp5k).unwrap();
        assert_eq!(e.port, [1.5, -31.0, 14.0]);
        let shotgun = crate::weapon::read(std::path::Path::new("data/kit/pw_shotgun.wpn")).unwrap();
        assert!(Ejector::of(&shotgun).is_none());
    }
}
