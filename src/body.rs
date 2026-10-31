//! How a player's body is posed for what they are doing: the motion the legs play, what the
//! arms do over it, a peek or a flinch, and the hips' turn toward a side step. The viewer draws
//! it, easing from one pose into the next; the sim shoots at it.

use crate::motion::{Motion, quat_mul};
use crate::sim::movement::{
    CROUCHED_EMPTY_RELOAD_MOTION, CROUCHED_RELOAD_MOTION, EMPTY_RELOAD_MOTION, Movement, RELOAD_MOTION, Reloading, reload_frame,
};
use crate::sim::peek::Peek;
use crate::skeleton::Skeleton;

pub struct PlayerMotions {
    pub idle: Option<Motion>,
    pub walk: Option<Motion>,
    pub run: Option<Motion>,
    pub back: Option<Motion>,
    pub left: Option<Motion>,
    pub right: Option<Motion>,
    /// Stepping to the side at a walk
    pub walk_left: Option<Motion>,
    pub walk_right: Option<Motion>,
    pub fire: Option<Motion>,
    /// Reloading with rounds left in the magazine, and once it is empty
    pub reload: Option<Motion>,
    pub reload_empty: Option<Motion>,
    pub throw: Option<Motion>,
    /// Crouched: kneeling with the gun up, and moving crouched
    pub crouch_idle: Option<Motion>,
    pub crouch_walk: Option<Motion>,
    pub crouch_run: Option<Motion>,
    pub crouch_back: Option<Motion>,
    pub crouch_left: Option<Motion>,
    pub crouch_right: Option<Motion>,
    /// Crouching down from standing, and standing up from crouched
    pub crouch_down: Option<Motion>,
    pub stand_up: Option<Motion>,
    /// Reloading kneeling, with rounds left in the magazine and once it is empty
    pub crouch_reload: Option<Motion>,
    pub crouch_reload_empty: Option<Motion>,
    /// Stepping round in place after turning, standing and crouched
    pub turn: Option<Motion>,
    pub crouch_turn: Option<Motion>,
    /// Peeking round a corner to the left and right (see `sim::peek`), standing and crouched
    pub peek_left: Option<Motion>,
    pub peek_right: Option<Motion>,
    pub crouch_peek_left: Option<Motion>,
    pub crouch_peek_right: Option<Motion>,
}

impl PlayerMotions {
    /// The motion for a movement, standing or crouched. Firing, reloading and throwing are
    /// the same motions crouched: the body kneels and the arms play them.
    pub fn get(&self, m: Movement, crouched: bool) -> Option<&Motion> {
        match (m, crouched) {
            (Movement::Idle, false) => &self.idle,
            (Movement::Walk, false) => &self.walk,
            (Movement::Run, false) => &self.run,
            (Movement::Back, false) => &self.back,
            (Movement::StrafeLeft, false) => &self.left,
            (Movement::StrafeRight, false) => &self.right,
            (Movement::WalkLeft, false) => &self.walk_left,
            (Movement::WalkRight, false) => &self.walk_right,
            (Movement::Idle, true) => &self.crouch_idle,
            (Movement::Walk, true) => &self.crouch_walk,
            (Movement::Run, true) => &self.crouch_run,
            (Movement::Back, true) => &self.crouch_back,
            (Movement::StrafeLeft | Movement::WalkLeft, true) => &self.crouch_left,
            (Movement::StrafeRight | Movement::WalkRight, true) => &self.crouch_right,
            (Movement::Fire, _) => &self.fire,
            (Movement::Reload, _) => &self.reload,
            (Movement::Throw, _) => &self.throw,
        }.as_ref()
    }

    /// The motion that takes a body into a stance: crouching down, or standing up
    pub fn stance_change(&self, crouched: bool) -> Option<&Motion> {
        if crouched { &self.crouch_down } else { &self.stand_up }.as_ref()
    }

    /// The motion the feet step round in after turning in place, standing or crouched
    pub fn turn(&self, crouched: bool) -> Option<&Motion> {
        if crouched { &self.crouch_turn } else { &self.turn }.as_ref()
    }

    /// The motion a peek plays, to the right or left, standing or crouched
    pub fn peek(&self, right: bool, crouched: bool) -> Option<&Motion> {
        match (right, crouched) {
            (false, false) => &self.peek_left,
            (true, false) => &self.peek_right,
            (false, true) => &self.crouch_peek_left,
            (true, true) => &self.crouch_peek_right,
        }.as_ref()
    }

    /// The motion for a reload, as the magazine was, standing or kneeling
    pub fn reload(&self, r: Reloading, crouched: bool) -> Option<&Motion> {
        match (r.empty, crouched) {
            (false, false) => &self.reload,
            (true, false) => &self.reload_empty,
            (false, true) => &self.crouch_reload,
            (true, true) => &self.crouch_reload_empty,
        }.as_ref()
    }
}

impl PlayerMotions {
    /// The player's motions, each the one `pick` gives for its slot (named as in the viewer's
    /// --player-anim) and its default. What is done in place is stood on the floor, whatever
    /// height its motion was made at, so the body does not sink or rise from one into the next;
    /// crouching down stands on its last frame, where it gives way to the kneel.
    pub fn load(skeleton: Option<&Skeleton>, pick: impl Fn(&str, &str) -> Option<Motion>) -> PlayerMotions {
        let planted_at = |slot: &str, default: &str, at_end: bool| pick(slot, default).map(|mut m| {
            if let Some(skel) = skeleton {
                let frame = if at_end { m.frames.saturating_sub(1) as f32 } else { 0.0 };
                skel.plant(&mut m, frame);
            }
            m
        });
        let planted = |slot: &str, default: &str| planted_at(slot, default, false);
        PlayerMotions {
            idle: planted("idle", "up_cyc_mp5k"),
            walk: pick("walk", "tmu_wk1"),
            run: pick("run", "tmu_rn"),
            back: pick("back", "tmu_bk1"),
            left: pick("left", "tms_l2"),
            right: pick("right", "tms_r2"),
            walk_left: pick("walk_left", "tms_l1"),
            walk_right: pick("walk_right", "tms_r1"),
            fire: planted("fire", "fire_up_mp5k"),
            reload: planted("reload", RELOAD_MOTION),
            reload_empty: planted("reload_empty", EMPTY_RELOAD_MOTION),
            throw: planted("throw", "gr_ov_up"),
            // Crouched, as the original's tables have them for a compact weapon
            crouch_idle: planted("crouch_idle", "crouch_arms_mp5k"),
            crouch_walk: pick("crouch_walk", "tmc_fd"),
            crouch_run: pick("crouch_run", "rmc_rn"),
            crouch_back: pick("crouch_back", "tmc_bk"),
            crouch_left: pick("crouch_left", "tmc_lf"),
            crouch_right: pick("crouch_right", "tmc_rt"),
            crouch_down: planted_at("crouch_down", "up_cr", true),
            stand_up: planted("stand_up", "cr_up"),
            crouch_reload: planted("crouch_reload", CROUCHED_RELOAD_MOTION),
            crouch_reload_empty: planted("crouch_reload_empty", CROUCHED_EMPTY_RELOAD_MOTION),
            // Stepping round after turning in place, on the legs, as the original's
            turn: planted("turn", "tmu_plf"),
            crouch_turn: planted("crouch_turn", "tmc_plf"),
            // Peeking, as the original's: crouched it also has tac_plf and tac_prt, the same motions
            peek_left: planted("peek_left", "tau_plf"),
            peek_right: planted("peek_right", "tau_prt"),
            crouch_peek_left: planted("crouch_peek_left", "tmac_plf"),
            crouch_peek_right: planted("crouch_peek_right", "tmac_prt"),
        }
    }

    /// Whether the legs walk or run: a flinch or a peek takes the whole body instead
    pub fn moving(&self, movement: Movement, peek: Option<Peek>, flinch: bool) -> bool {
        let peeking = !flinch && peek.is_some_and(|k| self.peek(k.right, k.crouched).is_some());
        !peeking && matches!(movement, Movement::Walk | Movement::Run | Movement::Back | Movement::StrafeLeft | Movement::StrafeRight | Movement::WalkLeft | Movement::WalkRight)
    }
}

// Bones driven by the weapon hold rather than the locomotion cycle
pub fn is_arm(name: &str) -> bool {
    matches!(name, "LeftCollar" | "RightCollar" | "LeftUpArm" | "LeftLowArm" | "LeftHand" | "RightUpArm" | "RightLowArm" | "RightHand")
}

pub fn is_upper_body(name: &str) -> bool {
    matches!(name, "Chest" | "LeftCollar" | "RightCollar" | "LeftUpArm" | "LeftLowArm" | "LeftHand"
        | "RightUpArm" | "RightLowArm" | "RightHand" | "Neck" | "Head")
}

/// What a body is doing, for `pose`
#[derive(Clone, Copy)]
pub struct Doing<'a> {
    pub movement: Movement,
    /// How far through the movement's motion
    pub frame: f32,
    pub crouched: bool,
    pub reloading: Option<Reloading>,
    /// The gun is firing, which the arms show whatever the legs do
    pub firing: bool,
    /// A wound's flinch and how far through it, which takes the whole body while it plays
    pub flinch: Option<(&'a Motion, f32)>,
    pub peek: Option<Peek>,
    /// How far the hips are turned toward a side step, in radians (see `movement::hip_turn`)
    pub hip_turn: f32,
    /// Crouching down or standing up, which plays in place, and stepping round after turning in
    /// place: each its motion and how far through it. The viewer keeps the clocks of these.
    pub changing: Option<(&'a Motion, f32)>,
    pub shuffle: Option<(&'a Motion, f32)>,
    /// The time the fire motion and the kneel loop on, in seconds
    pub clock: f32,
}

/// A posed body, before it is put in the level
pub struct Pose {
    pub root: [f32; 3],
    pub rotations: Vec<[f32; 4]>,
    /// The arms are firing
    pub aiming: bool,
    /// Peeking, to the right or not
    pub peeking: Option<bool>,
    /// Stepping round after a turn in place
    pub shuffling: bool,
}

/// Pose a body for what it is doing. None without the motion for it.
pub fn pose(motions: &PlayerMotions, skel: &Skeleton, d: &Doing) -> Option<Pose> {
    let Doing { movement, frame, crouched, reloading, firing, flinch, peek, hip_turn, changing, shuffle, clock } = *d;
    let peeking = peek.filter(|_| flinch.is_none()).and_then(|k| motions.peek(k.right, k.crouched).map(|m| (m, k.frame, k.right)));
    let moving = motions.moving(movement, peek, flinch.is_some());
    let reload = reloading.and_then(|r| motions.reload(r, crouched).map(|m| (m, reload_frame(r, crouched, m.frames.max(1) as f32))));
    // Crouched, firing and throwing are done kneeling, by the arms, as they are while the body
    // crouches down or stands up. A reload has a kneeling motion of its own.
    let kneeling = crouched && !moving && movement != Movement::Idle && !(movement == Movement::Reload && reload.is_some());
    let arms_only = kneeling || (changing.is_some() && movement != Movement::Idle);
    // A motion that loops on its own clock, whatever the body's frame
    let looping = |m: &Motion| (clock * 30.0) % m.frames.max(1) as f32;
    let (mot, at) = match (flinch, changing, shuffle, movement, reload) {
        (Some(f), ..) => f,
        _ if peeking.is_some() => peeking.map(|(m, f, _)| (m, f))?,
        (None, Some(change), ..) => change,
        (None, None, Some(s), ..) => s,
        (None, None, None, Movement::Reload, Some(shown)) => shown,
        _ if kneeling => {
            let knee = motions.get(Movement::Idle, true)?;
            (knee, looping(knee))
        }
        _ => (motions.get(movement, crouched)?, frame),
    };
    let (root, mut rots) = mot.sample(at.min(mot.frames.saturating_sub(1) as f32));
    // A reload keeps the arms busy, so they fire only outside one, and a flinch the same
    let aiming = (firing || movement == Movement::Fire) && reload.is_none() && flinch.is_none();
    // Moving, kneeling or stepping round: the legs from the motion, and the upper body reloads,
    // fires (the fire motion looping at its own pace), throws or keeps the gun hold
    let shuffling = flinch.is_none() && peeking.is_none() && shuffle.is_some();
    if flinch.is_none() && peeking.is_none() && (moving || arms_only || shuffling) {
        let fire = motions.get(Movement::Fire, crouched).filter(|_| aiming).map(|m| (m, looping(m)));
        let throw = motions.get(Movement::Throw, crouched).filter(|_| (arms_only || shuffling) && movement == Movement::Throw).map(|m| (m, at));
        let upper = reload.or(fire).or(throw).or_else(|| motions.get(Movement::Idle, crouched).map(|hold| (hold, 0.0)));
        if let Some((upper, upper_at)) = upper {
            let (_, hold_rots) = upper.sample(upper_at);
            // The hips of a crouched move pitch well forward of the kneeling hold's, so crouched
            // the chest keeps the way it faces in the hold, not its turn from the hips (which
            // would point the gun at the floor)
            let hips = skel.bones.iter().find(|b| b.name == "Hips").map(|b| b.anim_index);
            let (from, to) = match hips.and_then(|h| Some((*hold_rots.get(h)?, *rots.get(h)?))) {
                Some((held, under)) if crouched || changing.is_some() => (held, under),
                _ => ([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]),
            };
            let undo = [-to[0], -to[1], -to[2], to[3]];
            for b in &skel.bones {
                if is_upper_body(&b.name) {
                    if let (Some(r), Some(h)) = (rots.get_mut(b.anim_index), hold_rots.get(b.anim_index)) {
                        *r = if b.name == "Chest" { quat_mul(undo, quat_mul(from, *h)) } else { *h };
                    }
                }
            }
        }
    }
    // Leaning, the arms alone still reload, fire and throw, from the leaned chest, as the
    // original plays those on the arms
    if let (Some(_), None) = (peeking, flinch) {
        let fire = motions.get(Movement::Fire, crouched).filter(|_| aiming).map(|m| (m, looping(m)));
        let throw = motions.get(Movement::Throw, crouched).filter(|_| movement == Movement::Throw).map(|m| (m, frame));
        if let Some((arms, arms_at)) = reload.or(fire).or(throw) {
            let (_, arm_rots) = arms.sample(arms_at);
            for b in skel.bones.iter().filter(|b| is_arm(&b.name)) {
                if let (Some(r), Some(a)) = (rots.get_mut(b.anim_index), arm_rots.get(b.anim_index)) {
                    *r = *a;
                }
            }
        }
    }
    // Going forward or back and to the side, the hips turn toward the way the body goes, about
    // the upright, and the chest turns back as far so the upper body keeps facing the aim. A
    // flinch takes the whole body as it is.
    let bone = |name: &str| skel.bones.iter().find(|b| b.name == name).map(|b| b.anim_index);
    if let (None, Some(h), Some(c)) = (flinch, bone("Hips"), bone("Chest")) {
        if let (Some(&hips), Some(&chest)) = (rots.get(h), rots.get(c)) {
            let inverse = |q: [f32; 4]| [-q[0], -q[1], -q[2], q[3]];
            let q = [0.0, (hip_turn / 2.0).sin(), 0.0, (hip_turn / 2.0).cos()];
            rots[h] = quat_mul(q, hips);
            rots[c] = quat_mul(inverse(hips), quat_mul(inverse(q), quat_mul(hips, chest)));
        }
    }
    Some(Pose { root, rotations: rots, aiming, peeking: peeking.map(|p| p.2), shuffling })
}
