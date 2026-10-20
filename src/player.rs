//! The player's animations.

use rogue_reborn::motion;
use rogue_reborn::sim::movement::{HIP_TURN_RATE, Movement, Reloading};

pub struct PlayerMotions {
    pub idle: Option<motion::Motion>,
    pub walk: Option<motion::Motion>,
    pub run: Option<motion::Motion>,
    pub back: Option<motion::Motion>,
    pub left: Option<motion::Motion>,
    pub right: Option<motion::Motion>,
    pub fire: Option<motion::Motion>,
    /// Reloading with rounds left in the magazine, and once it is empty
    pub reload: Option<motion::Motion>,
    pub reload_empty: Option<motion::Motion>,
    pub throw: Option<motion::Motion>,
    /// Crouched: kneeling with the gun up, and moving crouched
    pub crouch_idle: Option<motion::Motion>,
    pub crouch_walk: Option<motion::Motion>,
    pub crouch_run: Option<motion::Motion>,
    pub crouch_back: Option<motion::Motion>,
    pub crouch_left: Option<motion::Motion>,
    pub crouch_right: Option<motion::Motion>,
    /// Crouching down from standing, and standing up from crouched
    pub crouch_down: Option<motion::Motion>,
    pub stand_up: Option<motion::Motion>,
    /// Reloading kneeling, with rounds left in the magazine and once it is empty
    pub crouch_reload: Option<motion::Motion>,
    pub crouch_reload_empty: Option<motion::Motion>,
}

impl PlayerMotions {
    /// The motion for a movement, standing or crouched. Firing, reloading and throwing are
    /// the same motions crouched: the body kneels and the arms play them.
    pub fn get(&self, m: Movement, crouched: bool) -> Option<&motion::Motion> {
        match (m, crouched) {
            (Movement::Idle, false) => &self.idle,
            (Movement::Walk, false) => &self.walk,
            (Movement::Run, false) => &self.run,
            (Movement::Back, false) => &self.back,
            (Movement::StrafeLeft, false) => &self.left,
            (Movement::StrafeRight, false) => &self.right,
            (Movement::Idle, true) => &self.crouch_idle,
            (Movement::Walk, true) => &self.crouch_walk,
            (Movement::Run, true) => &self.crouch_run,
            (Movement::Back, true) => &self.crouch_back,
            (Movement::StrafeLeft, true) => &self.crouch_left,
            (Movement::StrafeRight, true) => &self.crouch_right,
            (Movement::Fire, _) => &self.fire,
            (Movement::Reload, _) => &self.reload,
            (Movement::Throw, _) => &self.throw,
        }.as_ref()
    }

    /// The motion that takes a body into a stance: crouching down, or standing up
    pub fn stance_change(&self, crouched: bool) -> Option<&motion::Motion> {
        if crouched { &self.crouch_down } else { &self.stand_up }.as_ref()
    }

    /// The motion for a reload, as the magazine was, standing or kneeling
    pub fn reload(&self, r: Reloading, crouched: bool) -> Option<&motion::Motion> {
        match (r.empty, crouched) {
            (false, false) => &self.reload,
            (true, false) => &self.reload_empty,
            (false, true) => &self.crouch_reload,
            (true, true) => &self.crouch_reload_empty,
        }.as_ref()
    }
}

/// What a body is playing, as far as easing from one motion into the next goes
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BodyKey {
    pub movement: Movement,
    /// Reloading, and whether the magazine was empty (the two reloads are different motions)
    pub reload: Option<bool>,
    /// The arms are firing
    pub aiming: bool,
    pub crouched: bool,
    /// Crouching down or standing up
    pub changing: bool,
    /// Flinching from a wound, by when it began (as bits, so a new one eases in afresh)
    pub flinch: Option<u32>,
}

/// A body's ease from one motion into the next
pub type BodyFade = motion::Crossfade<BodyKey>;

/// What drawing a body remembers from one frame to the next
#[derive(Default)]
pub struct BodyState {
    pub fade: BodyFade,
    /// The stance it was last drawn in, and when that last changed
    stance: Option<bool>,
    changed_at: Option<f32>,
    /// How far the hips are turned (radians, positive to the right), and when that was drawn
    hips: f32,
    hips_at: Option<f32>,
}

impl BodyState {
    /// Note the body's stance at `now` (seconds), and say how long ago it last changed, if it
    /// has since the body was first drawn
    pub fn since_stance_change(&mut self, crouched: bool, now: f32) -> Option<f32> {
        if self.stance.is_some_and(|s| s != crouched) {
            self.changed_at = Some(now);
        }
        self.stance = Some(crouched);
        self.changed_at.map(|at| now - at)
    }

    /// Turn the hips toward `target` (radians) at the original's pace, from where they were when
    /// the body was last drawn, and say how far they are turned at `now` (seconds). A body not
    /// drawn for a while turns at once.
    pub fn turn_hips(&mut self, target: f32, now: f32) -> f32 {
        let step = match self.hips_at.map(|at| now - at) {
            Some(dt) if (0.0..=motion::CROSSFADE_SECONDS).contains(&dt) => HIP_TURN_RATE * dt,
            _ => f32::INFINITY,
        };
        self.hips += (target - self.hips).clamp(-step, step);
        self.hips_at = Some(now);
        self.hips
    }
}

// Bones driven by the weapon hold rather than the locomotion cycle
pub fn is_upper_body(name: &str) -> bool {
    matches!(name, "Chest" | "LeftCollar" | "RightCollar" | "LeftUpArm" | "LeftLowArm" | "LeftHand"
        | "RightUpArm" | "RightLowArm" | "RightHand" | "Neck" | "Head")
}
