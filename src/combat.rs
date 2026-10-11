//! Shooting, hit zones, damage and what characters do when hit.
//!
//! Everything here is plain logic with no rendering, so it can be tested.
//! Positions are in MAP space (Y up). A character is placed with the same
//! transform the viewer draws it with: translate, rotate about Y by `yaw`
//! degrees (so model +Z, the way it faces, points along `(sin yaw, 0, cos
//! yaw)`), then per-axis scale.

use crate::crp::{Crp, Skin};
use crate::skeleton::Skeleton;

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Möller-Trumbore ray/triangle test. `dir` need not be a unit vector; the
/// result is the ray parameter `t` (in units of `dir`). Triangles are hit
/// from either side.
pub fn ray_triangle(origin: V3, dir: V3, a: V3, b: V3, c: V3) -> Option<f32> {
    let e1 = sub(b, a);
    let e2 = sub(c, a);
    let p = cross(dir, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(origin, a);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(dir, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, q) * inv;
    (t > 1e-4).then_some(t)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyPart {
    Head,
    Torso,
    Abdomen,
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

/// The body part a skeleton bone belongs to
pub fn body_part(bone: &str) -> BodyPart {
    match bone {
        "Head" | "Neck" => BodyPart::Head,
        "Chest" | "LeftCollar" | "RightCollar" => BodyPart::Torso,
        "LeftUpArm" | "LeftLowArm" | "LeftHand" => BodyPart::LeftArm,
        "RightUpArm" | "RightLowArm" | "RightHand" => BodyPart::RightArm,
        "LeftUpLeg" | "LeftLowLeg" | "LeftFoot" => BodyPart::LeftLeg,
        "RightUpLeg" | "RightLowLeg" | "RightFoot" => BodyPart::RightLeg,
        _ => BodyPart::Abdomen,
    }
}

/// Health lost for a hit on a body part (a character has 100)
pub fn damage(part: BodyPart) -> f32 {
    match part {
        BodyPart::Head => 100.0,
        BodyPart::Torso => 34.0,
        BodyPart::Abdomen => 34.0,
        BodyPart::LeftArm | BodyPart::RightArm => 15.0,
        BodyPart::LeftLeg | BodyPart::RightLeg => 20.0,
    }
}

/// Which side of a character a shot came from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    Front,
    Back,
    Left,
    Right,
}

/// Name of the injured-upright reaction motion for a hit
pub fn hit_reaction(part: BodyPart, from: Facing) -> &'static str {
    use BodyPart::*;
    match (part, from) {
        (Head, Facing::Back) => "thiu_hb",
        (Head, _) => "thiu_hf",
        (Torso, Facing::Front) => "thiu_cf",
        (Torso, Facing::Back) => "thiu_cb",
        (Torso, Facing::Left) => "thiu_cl",
        (Torso, Facing::Right) => "thiu_cr",
        (Abdomen, _) => "thiu_ab",
        (LeftArm, _) => "thiu_la",
        (RightArm, _) => "thiu_ra",
        (LeftLeg, _) => "thiu_ll",
        (RightLeg, _) => "thiu_rl",
    }
}

/// Name of a death motion. Head shots have their own; others vary with `roll`.
pub fn death_motion(part: BodyPart, roll: u32) -> &'static str {
    if part == BodyPart::Head {
        ["thdh_1", "thdh_2"][(roll % 2) as usize]
    } else {
        ["thdu_1", "thdu_2", "thdu_3", "thdu_4"][(roll % 4) as usize]
    }
}

/// Where a character is and how it is drawn
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub pos: V3,
    pub yaw_deg: f32,
    pub scale: V3,
}

impl Placement {
    /// A world-space point in the character's model space
    pub fn to_local(&self, p: V3) -> V3 {
        let d = sub(p, self.pos);
        let l = self.dir_to_local_unscaled(d);
        [l[0] / self.scale[0], l[1] / self.scale[1], l[2] / self.scale[2]]
    }

    /// A world-space direction (or offset) in model space
    pub fn dir_to_local(&self, d: V3) -> V3 {
        let l = self.dir_to_local_unscaled(d);
        [l[0] / self.scale[0], l[1] / self.scale[1], l[2] / self.scale[2]]
    }

    fn dir_to_local_unscaled(&self, d: V3) -> V3 {
        // Inverse of the draw rotation about Y by `yaw`
        let (s, c) = self.yaw_deg.to_radians().sin_cos();
        [d[0] * c - d[2] * s, d[1], d[0] * s + d[2] * c]
    }

    /// A model-space point in world space
    pub fn to_world(&self, p: V3) -> V3 {
        let (s, c) = self.yaw_deg.to_radians().sin_cos();
        let (x, y, z) = (p[0] * self.scale[0], p[1] * self.scale[1], p[2] * self.scale[2]);
        [self.pos[0] + x * c + z * s, self.pos[1] + y, self.pos[2] - x * s + z * c]
    }

    /// Unit vector the character faces, in world space
    pub fn forward(&self) -> V3 {
        let (s, c) = self.yaw_deg.to_radians().sin_cos();
        [s, 0.0, c]
    }

    /// Which side of the character a shot travelling along `dir` hits
    pub fn facing_of_shot(&self, dir: V3) -> Facing {
        let to_attacker = [-dir[0], 0.0, -dir[2]];
        let len = (to_attacker[0] * to_attacker[0] + to_attacker[2] * to_attacker[2]).sqrt();
        if len < 1e-6 {
            return Facing::Front;
        }
        let a = [to_attacker[0] / len, 0.0, to_attacker[2] / len];
        let f = self.forward();
        let ahead = dot(a, f);
        // Model +X is the character's right
        let right = [f[2], 0.0, -f[0]];
        if ahead > 0.7 {
            Facing::Front
        } else if ahead < -0.7 {
            Facing::Back
        } else if dot(a, right) > 0.0 {
            Facing::Right
        } else {
            Facing::Left
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    /// Distance along the (unit) shot direction
    pub distance: f32,
    pub part: BodyPart,
    pub facing: Facing,
    pub point: V3,
}

/// Cast a shot against a posed character. `verts` are its model-space vertex
/// positions for the current pose; `dir` is a unit vector.
pub fn raycast_character(
    crp: &Crp,
    skeleton: &Skeleton,
    skin: &Skin,
    verts: &[V3],
    place: &Placement,
    origin: V3,
    dir: V3,
) -> Option<Hit> {
    let o = place.to_local(origin);
    let d = place.dir_to_local(dir);
    let mut best: Option<(f32, usize)> = None;
    let mut index = 0;
    for part in &crp.parts {
        for t in &part.triangles {
            let (a, b, c) = (verts[t[0] as usize], verts[t[1] as usize], verts[t[2] as usize]);
            if let Some(dist) = ray_triangle(o, d, a, b, c) {
                if best.map_or(true, |(bd, _)| dist < bd) {
                    best = Some((dist, index));
                }
            }
            index += 1;
        }
    }
    let (distance, tri) = best?;
    let t = crp.parts.iter().flat_map(|p| &p.triangles).nth(tri)?;
    let bone = skeleton.bones.get(*skin.bone.get(t[0] as usize)?)?;
    let mut part = body_part(&bone.name);

    // The Hips bone moves the lower torso and the Chest bone only the upper
    // chest and shoulders, with triangles joining them, so split the trunk by
    // the triangle's bind-pose height instead of by its first bone
    if matches!(part, BodyPart::Torso | BodyPart::Abdomen) {
        // Interpolate the bind-pose height at the exact point hit
        let hit_local = [o[0] + d[0] * distance, o[1] + d[1] * distance, o[2] + d[2] * distance];
        let (a, b, c) = (verts[t[0] as usize], verts[t[1] as usize], verts[t[2] as usize]);
        let (u, v, w) = barycentric(hit_local, a, b, c);
        let height = |i: u32| crp.vertices[i as usize][1];
        let at_hit = u * height(t[0]) + v * height(t[1]) + w * height(t[2]);
        if let Some(split) = trunk_split(crp, skeleton, skin) {
            part = if at_hit > split { BodyPart::Torso } else { BodyPart::Abdomen };
        }
    }
    Some(Hit {
        distance,
        part,
        facing: place.facing_of_shot(dir),
        point: [origin[0] + dir[0] * distance, origin[1] + dir[1] * distance, origin[2] + dir[2] * distance],
    })
}

/// Barycentric weights `(a, b, c)` of `p` (assumed on the triangle's plane)
fn barycentric(p: V3, a: V3, b: V3, c: V3) -> (f32, f32, f32) {
    let (v0, v1, v2) = (sub(b, a), sub(c, a), sub(p, a));
    let (d00, d01, d11) = (dot(v0, v0), dot(v0, v1), dot(v1, v1));
    let (d20, d21) = (dot(v2, v0), dot(v2, v1));
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-12 {
        return (1.0, 0.0, 0.0);
    }
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    (1.0 - v - w, v, w)
}

/// Bind-pose height dividing the abdomen from the chest: halfway between
/// the top of the Hips bone's vertices and the bottom of the Chest bone's
fn trunk_split(crp: &Crp, skeleton: &Skeleton, skin: &Skin) -> Option<f32> {
    let bone = |name: &str| skeleton.bones.iter().position(|b| b.name == name);
    let (hips, chest) = (bone("Hips")?, bone("Chest")?);
    let mut hips_top = f32::MIN;
    let mut chest_bottom = f32::MAX;
    for (v, &b) in skin.bone.iter().enumerate() {
        let y = crp.vertices[v][1];
        if b == hips { hips_top = hips_top.max(y); }
        if b == chest { chest_bottom = chest_bottom.min(y); }
    }
    (hips_top > f32::MIN && chest_bottom < f32::MAX).then(|| (hips_top + chest_bottom) / 2.0)
}

/// What a character is doing as a result of being shot
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NpcState {
    Idle,
    /// Playing an injury reaction that began at `start` seconds
    Reacting { motion: &'static str, start: f32 },
    /// Dead; the death motion began at `start` and holds its last frame
    Dead { motion: &'static str, start: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitResult {
    Injured,
    Killed,
    /// The target was already dead
    Ignored,
}

#[derive(Clone, Copy, Debug)]
pub struct Health {
    pub hp: f32,
    pub state: NpcState,
}

impl Health {
    pub fn new() -> Self {
        Self { hp: 100.0, state: NpcState::Idle }
    }

    pub fn is_dead(&self) -> bool {
        matches!(self.state, NpcState::Dead { .. })
    }

    /// Apply a hit at time `now` (seconds). `roll` picks between variants.
    pub fn apply_hit(&mut self, hit: &Hit, now: f32, roll: u32) -> HitResult {
        if self.is_dead() {
            return HitResult::Ignored;
        }
        self.hp -= damage(hit.part);
        if self.hp <= 0.0 {
            self.hp = 0.0;
            self.state = NpcState::Dead { motion: death_motion(hit.part, roll), start: now };
            HitResult::Killed
        } else {
            self.state = NpcState::Reacting { motion: hit_reaction(hit.part, hit.facing), start: now };
            HitResult::Injured
        }
    }
}

impl Default for Health {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn hit(part: BodyPart) -> Hit {
        Hit { distance: 1.0, part, facing: Facing::Front, point: [0.0; 3] }
    }

    #[test]
    fn ray_hits_a_triangle_from_either_side() {
        let (a, b, c) = ([0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]);
        let front = ray_triangle([2.0, 2.0, 5.0], [0.0, 0.0, -1.0], a, b, c);
        let back = ray_triangle([2.0, 2.0, -5.0], [0.0, 0.0, 1.0], a, b, c);
        assert_eq!(front, Some(5.0));
        assert_eq!(back, Some(5.0));
    }

    #[test]
    fn ray_misses_outside_and_behind() {
        let (a, b, c) = ([0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]);
        assert_eq!(ray_triangle([8.0, 8.0, 5.0], [0.0, 0.0, -1.0], a, b, c), None);
        assert_eq!(ray_triangle([2.0, 2.0, 5.0], [0.0, 0.0, 1.0], a, b, c), None);
    }

    #[test]
    fn bones_map_to_body_parts() {
        assert_eq!(body_part("Head"), BodyPart::Head);
        assert_eq!(body_part("RightLowArm"), BodyPart::RightArm);
        assert_eq!(body_part("LeftFoot"), BodyPart::LeftLeg);
        assert_eq!(body_part("Hips"), BodyPart::Abdomen);
        assert_eq!(body_part("Chest"), BodyPart::Torso);
    }

    #[test]
    fn placement_round_trips_points() {
        let p = Placement { pos: [100.0, 5.0, -40.0], yaw_deg: 37.0, scale: [1.0, 0.97, 1.2] };
        let local = [3.0, 50.0, -8.0];
        let back = p.to_local(p.to_world(local));
        for k in 0..3 {
            assert!((back[k] - local[k]).abs() < 1e-3, "{back:?}");
        }
    }

    #[test]
    fn forward_matches_the_drawing_rotation() {
        // Model +Z rotated by yaw about Y is (sin yaw, 0, cos yaw)
        let p = Placement { pos: [0.0; 3], yaw_deg: 90.0, scale: [1.0; 3] };
        let w = p.to_world([0.0, 0.0, 1.0]);
        let f = p.forward();
        for k in 0..3 {
            assert!((w[k] - f[k]).abs() < 1e-5, "{w:?} vs {f:?}");
        }
        assert!((f[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn shots_are_classified_by_side() {
        let p = Placement { pos: [0.0; 3], yaw_deg: 0.0, scale: [1.0; 3] }; // faces +Z
        // A shot travelling -Z comes from in front of someone facing +Z
        assert_eq!(p.facing_of_shot([0.0, 0.0, -1.0]), Facing::Front);
        assert_eq!(p.facing_of_shot([0.0, 0.0, 1.0]), Facing::Back);
        // Model +X is the character's right: a shot travelling -X comes from the right
        assert_eq!(p.facing_of_shot([-1.0, 0.0, 0.0]), Facing::Right);
        assert_eq!(p.facing_of_shot([1.0, 0.0, 0.0]), Facing::Left);
    }

    #[test]
    fn reaction_and_death_names_exist_as_motions() {
        let dir = Path::new("data/motion");
        if !dir.exists() {
            return;
        }
        let have = |name: &str| std::fs::read_dir(dir).unwrap().flatten().any(|e| {
            e.path().file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(name))
        });
        let parts = [BodyPart::Head, BodyPart::Torso, BodyPart::Abdomen, BodyPart::LeftArm,
            BodyPart::RightArm, BodyPart::LeftLeg, BodyPart::RightLeg];
        for part in parts {
            for from in [Facing::Front, Facing::Back, Facing::Left, Facing::Right] {
                let m = hit_reaction(part, from);
                assert!(have(m), "missing reaction motion {m}");
            }
            for roll in 0..4 {
                let m = death_motion(part, roll);
                assert!(have(m), "missing death motion {m}");
            }
        }
    }

    #[test]
    fn a_head_shot_kills_outright() {
        let mut h = Health::new();
        assert_eq!(h.apply_hit(&hit(BodyPart::Head), 1.0, 0), HitResult::Killed);
        assert!(h.is_dead());
        assert_eq!(h.state, NpcState::Dead { motion: "thdh_1", start: 1.0 });
    }

    #[test]
    fn body_shots_injure_then_kill() {
        let mut h = Health::new();
        assert_eq!(h.apply_hit(&hit(BodyPart::Torso), 1.0, 0), HitResult::Injured);
        assert!(matches!(h.state, NpcState::Reacting { motion: "thiu_cf", .. }));
        assert_eq!(h.apply_hit(&hit(BodyPart::Torso), 2.0, 0), HitResult::Injured);
        assert_eq!(h.apply_hit(&hit(BodyPart::Torso), 3.0, 1), HitResult::Killed);
        assert_eq!(h.hp, 0.0);
        assert!(matches!(h.state, NpcState::Dead { motion: "thdu_2", .. }));
    }

    #[test]
    fn the_dead_stay_dead() {
        let mut h = Health::new();
        h.apply_hit(&hit(BodyPart::Head), 1.0, 0);
        let before = h.state;
        assert_eq!(h.apply_hit(&hit(BodyPart::Torso), 5.0, 2), HitResult::Ignored);
        assert_eq!(h.state, before);
    }

    #[test]
    fn shooting_a_posed_soldier_hits_the_right_part() {
        let Ok(c) = crate::crp::read(Path::new("data/character/team_hrt/Arctic1_H_a.CRP")) else { return };
        let skel = crate::skeleton::read(Path::new("data/character/team_hrt/Arctic1_H_a.SKL")).unwrap();
        let skin = c.skin(&skel).unwrap();
        let hips = skel.bones.iter().find(|b| b.name == "Hips").unwrap().bind_pos;
        let verts = skin.apply(&skel.pose(hips, &vec![[0.0, 0.0, 0.0, 1.0]; 18]));
        // Stand the model at x=500, facing +Z, feet on y=0
        let (min, _) = c.bounds();
        let place = Placement { pos: [500.0, -min[1], 0.0], yaw_deg: 0.0, scale: [1.0; 3] };
        let aim = |height: f32| raycast_character(&c, &skel, &skin, &verts, &place,
            [500.0, height, 300.0], [0.0, 0.0, -1.0]);

        // A shot at the head height of a standing 200-unit character
        let head = aim(190.0).expect("hit at head height");
        assert_eq!(head.part, BodyPart::Head);
        assert_eq!(head.facing, Facing::Front);
        // Chest height
        assert_eq!(aim(150.0).expect("chest hit").part, BodyPart::Torso);
        // Belt height is the abdomen
        assert_eq!(aim(112.0).expect("abdomen hit").part, BodyPart::Abdomen);
        // Shin height
        // (aimed at the right leg, 8 units off the centre line)
        let leg = raycast_character(&c, &skel, &skin, &verts, &place, [508.0, 40.0, 300.0], [0.0, 0.0, -1.0])
            .expect("leg hit");
        assert_eq!(leg.part, BodyPart::RightLeg);
        // Way above the head
        assert!(aim(400.0).is_none());
    }
}
