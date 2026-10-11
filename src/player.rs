//! The player's animations.

use rogue_reborn::motion;
use rogue_reborn::sim::movement::Movement;

pub struct PlayerMotions {
    pub idle: Option<motion::Motion>,
    pub walk: Option<motion::Motion>,
    pub run: Option<motion::Motion>,
    pub back: Option<motion::Motion>,
    pub left: Option<motion::Motion>,
    pub right: Option<motion::Motion>,
    pub fire: Option<motion::Motion>,
    pub reload: Option<motion::Motion>,
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
}

// Bones driven by the weapon hold rather than the locomotion cycle
pub fn is_upper_body(name: &str) -> bool {
    matches!(name, "Chest" | "LeftCollar" | "RightCollar" | "LeftUpArm" | "LeftLowArm" | "LeftHand"
        | "RightUpArm" | "RightLowArm" | "RightHand" | "Neck" | "Head")
}
