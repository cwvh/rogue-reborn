//! Reader for character animations (`.DAM`).
//!
//! Layout as understood: `f32` version (1.365), `u32` total frame count,
//! `u32` number of stored key frames, `u32` key interval (a key every N
//! frames, or 1 when every frame is stored), then the key frames. Each is
//! 304 bytes: `u32` bone count (18), the root bone's position as three
//! `f32`s, and one rotation quaternion `[x, y, z, w]` per bone, indexed by
//! animation index (see `skeleton`). After the key frames comes a
//! length-prefixed motion name and a few floats that are not read.

use std::path::Path;

use anyhow::{Context, Result, ensure};

const HEADER: usize = 16;
const KEY_SIZE: usize = 304;

#[derive(Clone, Debug)]
pub struct Key {
    pub root: [f32; 3],
    pub rotations: Vec<[f32; 4]>,
}

#[derive(Clone, Debug)]
pub struct Motion {
    pub name: String,
    /// Length in frames
    pub frames: u32,
    /// A key is stored every this many frames
    pub key_interval: u32,
    pub keys: Vec<Key>,
}

pub fn read(path: &Path) -> Result<Motion> {
    let data = std::fs::read(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    parse(&data)
}

fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    let b = d.get(o..o + 4).context("unexpected end of DAM")?;
    Ok(u32::from_le_bytes(b.try_into().unwrap()))
}

fn f32_at(d: &[u8], o: usize) -> Result<f32> {
    Ok(f32::from_bits(u32_at(d, o)?))
}

pub fn parse(d: &[u8]) -> Result<Motion> {
    let frames = u32_at(d, 4)?;
    let key_count = u32_at(d, 8)? as usize;
    let key_interval = u32_at(d, 12)?.max(1);
    ensure!((1..10_000).contains(&key_count), "implausible key count {key_count}");

    let mut keys = Vec::with_capacity(key_count);
    for k in 0..key_count {
        let o = HEADER + k * KEY_SIZE;
        let bones = u32_at(d, o).with_context(|| format!("key {k}"))? as usize;
        ensure!(bones == 18, "key {k}: {bones} bones");
        let root = [f32_at(d, o + 4)?, f32_at(d, o + 8)?, f32_at(d, o + 12)?];
        let mut rotations = Vec::with_capacity(bones);
        for b in 0..bones {
            let q = o + 16 + 16 * b;
            rotations.push(normalize([f32_at(d, q)?, f32_at(d, q + 4)?, f32_at(d, q + 8)?, f32_at(d, q + 12)?]));
        }
        keys.push(Key { root, rotations });
    }

    // Motion name after the keys
    let at = HEADER + key_count * KEY_SIZE;
    let name = match u32_at(d, at) {
        Ok(len) if (1..128).contains(&len) => d.get(at + 4..at + 4 + len as usize - 1)
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default(),
        _ => String::new(),
    };

    Ok(Motion { name, frames, key_interval, keys })
}

impl Motion {
    /// Root position and per-bone rotations at a frame, interpolating between
    /// the stored keys. Frames past the last key hold its pose.
    pub fn sample(&self, frame: f32) -> ([f32; 3], Vec<[f32; 4]>) {
        let pos = (frame / self.key_interval as f32).max(0.0);
        let i = (pos.floor() as usize).min(self.keys.len() - 1);
        let j = (i + 1).min(self.keys.len() - 1);
        let t = if i == j { 0.0 } else { (pos - i as f32).clamp(0.0, 1.0) };
        let (a, b) = (&self.keys[i], &self.keys[j]);
        let root = [
            a.root[0] + (b.root[0] - a.root[0]) * t,
            a.root[1] + (b.root[1] - a.root[1]) * t,
            a.root[2] + (b.root[2] - a.root[2]) * t,
        ];
        let rotations = a.rotations.iter().zip(&b.rotations).map(|(qa, qb)| slerp(*qa, *qb, t)).collect();
        (root, rotations)
    }
}

/// How long a body usually takes to ease from one motion into the next. The original blends
/// over 5 to 10 frames (0.17 to 0.33 s) where it names a length.
pub const CROSSFADE_SECONDS: f32 = 0.25;

/// Eases a body from the pose it was in into a new motion, instead of jumping to the new
/// motion's pose. `K` says which motion is playing; when it changes, the pose last drawn eases
/// into the new one over the time given for it, so a change in the middle of a fade starts
/// from where the body is.
pub struct Crossfade<K> {
    key: Option<K>,
    from: Option<([f32; 3], Vec<[f32; 4]>)>,
    start: f32,
    seconds: f32,
    last: ([f32; 3], Vec<[f32; 4]>),
    last_time: f32,
}

impl<K> Default for Crossfade<K> {
    fn default() -> Self {
        Crossfade { key: None, from: None, start: 0.0, seconds: CROSSFADE_SECONDS, last: ([0.0; 3], Vec::new()), last_time: f32::NEG_INFINITY }
    }
}

impl<K: PartialEq> Crossfade<K> {
    /// The motion the body is playing, as last given
    pub fn key(&self) -> Option<&K> {
        self.key.as_ref()
    }

    /// Ease the pose of `key` at time `now` (seconds), the root position and per-bone
    /// rotations as `Motion::sample` gives them, from where the body was. `seconds` is how long
    /// easing into `key` takes, if it is new (0 cuts to it). A body not drawn for a while
    /// starts the new motion as it is.
    pub fn apply(&mut self, key: K, now: f32, seconds: f32, root: &mut [f32; 3], rotations: &mut [[f32; 4]]) {
        if self.key.as_ref() != Some(&key) {
            let recent = now - self.last_time <= CROSSFADE_SECONDS && self.last.1.len() == rotations.len();
            self.from = (recent && seconds > 0.0).then(|| self.last.clone());
            self.start = now;
            self.seconds = seconds;
            self.key = Some(key);
        }
        if let Some((from_root, from)) = &self.from {
            let t = ((now - self.start) / self.seconds).clamp(0.0, 1.0);
            if t < 1.0 {
                // Smoothstep: leaves and arrives at rest
                let s = t * t * (3.0 - 2.0 * t);
                for (r, f) in rotations.iter_mut().zip(from) {
                    *r = slerp(*f, *r, s);
                }
                for k in 0..3 {
                    root[k] = from_root[k] + (root[k] - from_root[k]) * s;
                }
            } else {
                self.from = None;
            }
        }
        self.last.0 = *root;
        self.last.1.clear();
        self.last.1.extend_from_slice(rotations);
        self.last_time = now;
    }
}

pub fn normalize(q: [f32; 4]) -> [f32; 4] {
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if l < 1e-6 { [0.0, 0.0, 0.0, 1.0] } else { [q[0] / l, q[1] / l, q[2] / l, q[3] / l] }
}

pub fn slerp(a: [f32; 4], mut b: [f32; 4], t: f32) -> [f32; 4] {
    let mut dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    if dot < 0.0 {
        b = [-b[0], -b[1], -b[2], -b[3]];
        dot = -dot;
    }
    if dot > 0.9995 {
        return normalize([
            a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t,
        ]);
    }
    let theta = dot.clamp(-1.0, 1.0).acos();
    let (sa, sb) = (((1.0 - t) * theta).sin() / theta.sin(), (t * theta).sin() / theta.sin());
    [a[0] * sa + b[0] * sb, a[1] * sa + b[1] * sb, a[2] * sa + b[2] * sb, a[3] * sa + b[3] * sb]
}

/// Hamilton product `a * b`: applies `b` first, then `a`
pub fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

pub fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let w = q[3];
    let cross = |a: [f32; 3], b: [f32; 3]| [
        a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0],
    ];
    let t = cross(u, v).map(|x| 2.0 * x);
    let c = cross(u, t);
    [v[0] + w * t[0] + c[0], v[1] + w * t[1] + c[1], v[2] + w * t[2] + c[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotating_x_by_90_about_z_gives_y() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let r = quat_rotate([0.0, 0.0, h, h], [1.0, 0.0, 0.0]);
        assert!((r[0]).abs() < 1e-5 && (r[1] - 1.0).abs() < 1e-5 && r[2].abs() < 1e-5, "{r:?}");
    }

    #[test]
    fn quat_mul_applies_right_operand_first() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let rz = [0.0, 0.0, h, h];
        let rx = [h, 0.0, 0.0, h];
        let v = [0.0, 1.0, 0.0];
        let both = quat_rotate(quat_mul(rz, rx), v);
        let seq = quat_rotate(rz, quat_rotate(rx, v));
        for k in 0..3 {
            assert!((both[k] - seq[k]).abs() < 1e-5);
        }
    }

    #[test]
    fn slerp_halfway_is_half_the_angle() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let q = slerp([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, h, h], 0.5);
        let angle = 2.0 * q[3].clamp(-1.0, 1.0).acos();
        assert!((angle - std::f32::consts::FRAC_PI_4).abs() < 1e-4, "{angle}");
    }

    /// A turn of `degrees` about Z
    fn about_z(degrees: f32) -> [f32; 4] {
        let h = degrees.to_radians() / 2.0;
        [0.0, 0.0, h.sin(), h.cos()]
    }

    fn degrees_about_z(q: [f32; 4]) -> f32 {
        2.0 * q[2].atan2(q[3]).to_degrees()
    }

    #[test]
    fn a_crossfade_eases_from_the_last_pose_into_the_new_motion() {
        let mut fade = Crossfade::default();
        let step = |fade: &mut Crossfade<u8>, key, now, degrees, height| {
            let (mut root, mut rots) = ([0.0, height, 0.0], [about_z(degrees)]);
            fade.apply(key, now, if key == 4 { 0.0 } else { CROSSFADE_SECONDS }, &mut root, &mut rots);
            (degrees_about_z(rots[0]), root[1])
        };
        // The first motion shows as it is, and so does more of the same one
        assert_eq!(step(&mut fade, 1, 0.0, 0.0, 100.0), (0.0, 100.0));
        assert_eq!(step(&mut fade, 1, 0.01, 0.0, 100.0), (0.0, 100.0));
        // A new motion starts from the pose before it, is halfway there halfway through, and
        // arrives
        let (a, y) = step(&mut fade, 2, 0.02, 90.0, 90.0);
        assert!(a.abs() < 1e-3 && (y - 100.0).abs() < 1e-3, "{a} {y}");
        let (a, y) = step(&mut fade, 2, 0.02 + CROSSFADE_SECONDS / 2.0, 90.0, 90.0);
        assert!((a - 45.0).abs() < 1e-2 && (y - 95.0).abs() < 1e-3, "{a} {y}");
        assert_eq!(step(&mut fade, 2, 0.02 + CROSSFADE_SECONDS, 90.0, 90.0).1, 90.0);
        // Changing again partway through starts from where the body is
        step(&mut fade, 3, 0.5, 0.0, 90.0);
        let (mid, _) = step(&mut fade, 3, 0.5 + CROSSFADE_SECONDS / 2.0, 0.0, 90.0);
        let (from, _) = step(&mut fade, 1, 0.5 + CROSSFADE_SECONDS / 2.0 + 0.01, -90.0, 90.0);
        assert!((from - mid).abs() < 2.0 && mid > 30.0, "{mid} {from}");
        // A body not drawn for a while takes up its motion as it is
        assert_eq!(step(&mut fade, 2, 5.0, 60.0, 90.0).0.round(), 60.0);
        // A motion given no time to ease into is cut to at once
        assert_eq!(step(&mut fade, 4, 5.01, -60.0, 90.0).0.round(), -60.0);
    }

    #[test]
    fn reads_a_crouch_motion() {
        let p = Path::new("data/motion/cr_up.DAM");
        if !p.exists() {
            return;
        }
        let m = read(p).unwrap();
        assert_eq!(m.name, "cr_up");
        assert_eq!((m.frames, m.keys.len(), m.key_interval), (20, 5, 4));
        // The hips rise as the crouch comes up
        assert!(m.keys[4].root[1] > m.keys[0].root[1] + 40.0);
        assert!(m.keys[0].rotations.iter().all(|q| (q.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-3));
    }

    #[test]
    fn parses_every_motion_file() {
        let Ok(dir) = std::fs::read_dir("data/motion") else { return };
        let mut n = 0;
        for f in dir.flatten() {
            let p = f.path();
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dam")) {
                let m = read(&p).unwrap_or_else(|e| panic!("{}: {e:#}", p.display()));
                assert!(!m.name.is_empty(), "{} has no name", p.display());
                n += 1;
            }
        }
        eprintln!("{n} motions parsed");
    }
}
