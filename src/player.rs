//! The player's animations.

use rogue_reborn::motion;
use rogue_reborn::sim::movement::{Movement, Reloading};

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
}

impl PlayerMotions {
    pub fn get(&self, m: Movement) -> Option<&motion::Motion> {
        match m {
            Movement::Idle => &self.idle,
            Movement::Walk => &self.walk,
            Movement::Run => &self.run,
            Movement::Back => &self.back,
            Movement::StrafeLeft => &self.left,
            Movement::StrafeRight => &self.right,
            Movement::Fire => &self.fire,
            Movement::Reload => &self.reload,
            Movement::Throw => &self.throw,
        }.as_ref()
    }

    /// The motion for a reload, as the magazine was
    pub fn reload(&self, r: Reloading) -> Option<&motion::Motion> {
        if r.empty { &self.reload_empty } else { &self.reload }.as_ref()
    }
}

/// A body's ease from one motion into the next. What it is playing is its movement, and for a
/// reload whether the magazine was empty (the two reloads are different motions).
pub type BodyFade = motion::Crossfade<(Movement, Option<bool>)>;

// Bones driven by the weapon hold rather than the locomotion cycle
pub fn is_upper_body(name: &str) -> bool {
    matches!(name, "Chest" | "LeftCollar" | "RightCollar" | "LeftUpArm" | "LeftLowArm" | "LeftHand"
        | "RightUpArm" | "RightLowArm" | "RightHand" | "Neck" | "Head")
}
