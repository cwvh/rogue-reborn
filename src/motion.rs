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
