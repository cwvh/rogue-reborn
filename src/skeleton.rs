//! Reader for character skeletons (`.SKL`) and pose evaluation.
//!
//! The format is undocumented; what is read here was worked out from the
//! files. A skeleton file is an `f32` version (1.5), a `u32` bone count (18)
//! and then one record per bone, in leaf-to-root order (RightFoot ... Hips).
//! Each record is 130 bytes, a length-prefixed name, and at least 39 more
//! bytes (a few bones carry extra, so records are located by name). In
//! the first part, at offset 86, is the bone's bind position (three `f32`s)
//! in model space; bind rotations are all identity. The `u32` right after
//! the name is the bone's index in animation data (Hips = 0, RightFoot = 17).
//! The record layout is not 4-byte aligned, so fields are read at their byte
//! offsets.
//!
//! The parent of each bone is not read from the file (no field found); it
//! comes from the standard biped layout by name.

use anyhow::{Context, Result, ensure};

use crate::motion::quat_rotate;

const PRE_NAME: usize = 130;
const POST_NAME: usize = 39;
const BIND_POS_AT: usize = 86;

#[derive(Clone, Debug)]
pub struct Bone {
    pub name: String,
    /// Index of this bone's rotation in motion data
    pub anim_index: usize,
    /// Bind-pose position in model space
    pub bind_pos: [f32; 3],
    /// Index into `Skeleton::bones`
    pub parent: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

/// Parent bone name for each standard biped bone
fn parent_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "Chest" | "LeftUpLeg" | "RightUpLeg" => "Hips",
        "Neck" | "LeftCollar" | "RightCollar" => "Chest",
        "Head" => "Neck",
        "LeftUpArm" => "LeftCollar",
        "LeftLowArm" => "LeftUpArm",
        "LeftHand" => "LeftLowArm",
        "RightUpArm" => "RightCollar",
        "RightLowArm" => "RightUpArm",
        "RightHand" => "RightLowArm",
        "LeftLowLeg" => "LeftUpLeg",
        "LeftFoot" => "LeftLowLeg",
        "RightLowLeg" => "RightUpLeg",
        "RightFoot" => "RightLowLeg",
        _ => return None,
    })
}

fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    let b = d.get(o..o + 4).context("unexpected end of SKL")?;
    Ok(u32::from_le_bytes(b.try_into().unwrap()))
}

pub fn read(path: &std::path::Path) -> Result<Skeleton> {
    let data = std::fs::read(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    parse(&data)
}

pub fn parse(d: &[u8]) -> Result<Skeleton> {
    let count = u32_at(d, 4).context("bone count")? as usize;
    ensure!((1..=64).contains(&count), "implausible bone count {count}");

    // Records are not all the same size (some bones carry extra bytes after
    // the name), so find each one by its name: a length, letters, a NUL and
    // then a small animation index. Records come one after another.
    let mut bones: Vec<Bone> = Vec::with_capacity(count);
    let mut at = 8 + PRE_NAME;
    while bones.len() < count && at + 8 < d.len() {
        let len = u32_at(d, at)? as usize;
        let name = (3..=32).contains(&len).then(|| d.get(at + 4..at + 4 + len)).flatten()
            .filter(|b| b[len - 1] == 0 && b[..len - 1].iter().all(u8::is_ascii_alphabetic))
            .map(|b| String::from_utf8_lossy(&b[..len - 1]).into_owned());
        let anim_index = u32_at(d, at + 4 + len.min(64)).unwrap_or(u32::MAX) as usize;
        match name {
            Some(name) if anim_index < count && at >= BIND_POS_AT + 8 - (PRE_NAME - BIND_POS_AT) => {
                let f = |o: usize| -> Result<f32> { Ok(f32::from_bits(u32_at(d, o)?)) };
                let p = at - (PRE_NAME - BIND_POS_AT);
                bones.push(Bone {
                    name,
                    anim_index,
                    bind_pos: [f(p)?, f(p + 4)?, f(p + 8)?],
                    parent: None,
                });
                // Jump past the name and the fixed part of the tail
                at += 4 + len + POST_NAME;
            }
            _ => at += 1,
        }
    }
    ensure!(bones.len() == count, "found {} of {count} bones", bones.len());

    for i in 0..bones.len() {
        bones[i].parent = parent_name(&bones[i].name)
            .and_then(|pn| bones.iter().position(|b| b.name == pn));
    }
    Ok(Skeleton { bones })
}

/// A bone's world transform: rotation as a quaternion `[x, y, z, w]` and
/// translation
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    pub rot: [f32; 4],
    pub pos: [f32; 3],
}

impl Transform {
    /// Column-major 4x4 matrix for OpenGL (rotation then translation)
    pub fn matrix(&self) -> [f32; 16] {
        let [x, y, z, w] = self.rot;
        [
            1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w), 0.0,
            2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w), 0.0,
            2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y), 0.0,
            self.pos[0], self.pos[1], self.pos[2], 1.0,
        ]
    }
}

impl Skeleton {
    /// Index of the bone with this animation index
    pub fn bone_for_anim(&self, anim_index: usize) -> Option<usize> {
        self.bones.iter().position(|b| b.anim_index == anim_index)
    }

    /// World transforms of every bone for a pose: `root` is the position of
    /// the root bone and `rotations` the per-animation-index local rotations
    /// (relative to the bind pose, in which every rotation is identity).
    pub fn pose(&self, root: [f32; 3], rotations: &[[f32; 4]]) -> Vec<Transform> {
        let mut out: Vec<Option<Transform>> = vec![None; self.bones.len()];
        // Bones are stored leaf-first, so resolve parents on demand
        fn resolve(
            sk: &Skeleton,
            i: usize,
            root: [f32; 3],
            rotations: &[[f32; 4]],
            out: &mut Vec<Option<Transform>>,
        ) -> Transform {
            if let Some(t) = out[i] {
                return t;
            }
            let bone = &sk.bones[i];
            let local = rotations.get(bone.anim_index).copied().unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let t = match bone.parent {
                None => Transform { rot: local, pos: root },
                Some(p) => {
                    let pt = resolve(sk, p, root, rotations, out);
                    let off = [
                        bone.bind_pos[0] - sk.bones[p].bind_pos[0],
                        bone.bind_pos[1] - sk.bones[p].bind_pos[1],
                        bone.bind_pos[2] - sk.bones[p].bind_pos[2],
                    ];
                    let r = quat_rotate(pt.rot, off);
                    Transform {
                        rot: crate::motion::quat_mul(pt.rot, local),
                        pos: [pt.pos[0] + r[0], pt.pos[1] + r[1], pt.pos[2] + r[2]],
                    }
                }
            };
            out[i] = Some(t);
            t
        }
        for i in 0..self.bones.len() {
            resolve(self, i, root, rotations, &mut out);
        }
        out.into_iter().map(|t| t.unwrap()).collect()
    }

    /// The bone whose bind position is nearest `p`, with the distance
    pub fn nearest_bone(&self, p: [f32; 3]) -> Option<(usize, f32)> {
        self.bones.iter().enumerate().map(|(i, b)| {
            let d = ((b.bind_pos[0] - p[0]).powi(2)
                + (b.bind_pos[1] - p[1]).powi(2)
                + (b.bind_pos[2] - p[2]).powi(2)).sqrt();
            (i, d)
        }).min_by(|a, b| a.1.total_cmp(&b.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const SKL: &str = "data/character/team_hrt/Arctic1_H_a.SKL";

    fn load() -> Option<Skeleton> {
        let p = Path::new(SKL);
        if !p.exists() {
            eprintln!("skipping: {SKL} not found");
            return None;
        }
        Some(read(p).expect("SKL parses"))
    }

    #[test]
    fn reads_the_biped_bones() {
        let Some(s) = load() else { return };
        assert_eq!(s.bones.len(), 18);
        assert_eq!(s.bones[0].name, "RightFoot");
        assert_eq!(s.bones[17].name, "Hips");
        let foot = &s.bones[0];
        assert_eq!(foot.anim_index, 17);
        assert!((foot.bind_pos[0] - 8.443).abs() < 0.01);
        assert!((foot.bind_pos[1] + 101.231).abs() < 0.01);
    }

    #[test]
    fn anim_indices_are_a_permutation() {
        let Some(s) = load() else { return };
        let mut idx: Vec<usize> = s.bones.iter().map(|b| b.anim_index).collect();
        idx.sort();
        assert_eq!(idx, (0..18).collect::<Vec<_>>());
    }

    #[test]
    fn hips_is_the_only_root() {
        let Some(s) = load() else { return };
        let roots: Vec<_> = s.bones.iter().filter(|b| b.parent.is_none()).collect();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].name, "Hips");
    }

    #[test]
    fn bind_pose_reproduces_bind_positions() {
        let Some(s) = load() else { return };
        let ident = vec![[0.0, 0.0, 0.0, 1.0]; 18];
        let hips = s.bones.iter().find(|b| b.name == "Hips").unwrap().bind_pos;
        let pose = s.pose(hips, &ident);
        for (b, t) in s.bones.iter().zip(&pose) {
            for k in 0..3 {
                assert!((b.bind_pos[k] - t.pos[k]).abs() < 1e-3, "{} {:?} vs {:?}", b.name, b.bind_pos, t.pos);
            }
        }
    }

    #[test]
    fn parses_every_skeleton_file() {
        let Ok(dirs) = std::fs::read_dir("data/character") else { return };
        let mut n = 0;
        for dir in dirs.flatten().filter(|d| d.path().is_dir()) {
            for f in std::fs::read_dir(dir.path()).unwrap().flatten() {
                let p = f.path();
                if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("skl")) {
                    let s = read(&p).unwrap_or_else(|e| panic!("{}: {e:#}", p.display()));
                    assert_eq!(s.bones.len(), 18, "{}", p.display());
                    assert_eq!(s.bones.iter().filter(|b| b.parent.is_none()).count(), 1, "{}", p.display());
                    n += 1;
                }
            }
        }
        eprintln!("{n} skeletons parsed");
    }
}
