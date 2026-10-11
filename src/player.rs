//! What the player's body is doing: moving, firing, reloading, throwing.

use rogue_reborn::motion;

// What the player's body is doing, which picks the animation
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Movement {
    Idle,
    Walk,
    Run,
    Back,
    StrafeLeft,
    StrafeRight,
    /// Standing and shooting, or reloading, with the weapon
    Fire,
    Reload,
    /// Throwing a grenade
    Throw,
}

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

pub const RELOAD_SECONDS: f32 = 2.2;

/// A grenade throw: from the start of the swing to the end, and how far into it the grenade
/// leaves the hand (0..1)
pub const THROW_SECONDS: f32 = 0.85;
pub const THROW_RELEASE: f32 = 0.35;
/// A throw under way
#[derive(Clone, Copy)]
pub struct Throw {
    pub start: f32,
    /// How long the throw was held for before letting go (0..1)
    pub charge: f32,
    /// Whether the grenade has left the hand
    pub thrown: bool,
}

// Bones driven by the weapon hold rather than the locomotion cycle
pub fn is_upper_body(name: &str) -> bool {
    matches!(name, "Chest" | "LeftCollar" | "RightCollar" | "LeftUpArm" | "LeftLowArm" | "LeftHand"
        | "RightUpArm" | "RightLowArm" | "RightHand" | "Neck" | "Head")
}

// Ground speed (units per second) at which each animation plays at its own
// pace without the feet sliding. These are the speeds with the least slip of
// a planted foot, found with `SPEED=100 cargo run --release --example
// motion-info <motion>`. A planted foot is never perfectly still, so some
// slip is left.
pub fn nominal_speed(m: Movement) -> f32 {
    match m {
        Movement::Idle | Movement::Fire | Movement::Reload | Movement::Throw => 1.0,
        Movement::Walk => 257.0,
        Movement::Run => 440.0,
        Movement::Back => 126.0,
        Movement::StrafeLeft => 139.0,
        Movement::StrafeRight => 139.0,
    }
}

// Ground speed the player actually moves at. Matches the game where known
// (tuned by the user with --speed-scale); otherwise the animation's own pace.
pub fn ground_speed(m: Movement) -> f32 {
    match m {
        Movement::Run => 735.0,
        Movement::Walk => 514.0,
        Movement::Back => 378.0,
        Movement::StrafeLeft | Movement::StrafeRight => 348.0,
        other => nominal_speed(other),
    }
}

// Pick the movement from the held keys
pub fn movement_from_keys(forward: bool, back: bool, left: bool, right: bool, run: bool) -> Movement {
    if forward {
        if run { Movement::Run } else { Movement::Walk }
    } else if back {
        Movement::Back
    } else if left && !right {
        Movement::StrafeLeft
    } else if right && !left {
        Movement::StrafeRight
    } else {
        Movement::Idle
    }
}
