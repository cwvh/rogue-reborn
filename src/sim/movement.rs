//! How the player moves: the kinds of movement, how fast each goes, and which way the
//! player faces.

/// What the player's body is doing, which picks the animation
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

/// Ground speed (units per second) at which each animation plays at its own pace without
/// the feet sliding. These are the speeds with the least slip of a planted foot, found with
/// `SPEED=100 cargo run --release --example motion-info <motion>`. A planted foot is never
/// perfectly still, so some slip is left.
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

/// Ground speed the player actually moves at. Matches the game where known (tuned by the
/// user with --speed-scale); otherwise the animation's own pace.
pub fn ground_speed(m: Movement) -> f32 {
    match m {
        Movement::Run => 735.0,
        Movement::Walk => 514.0,
        Movement::Back => 378.0,
        Movement::StrafeLeft | Movement::StrafeRight => 348.0,
        other => nominal_speed(other),
    }
}

/// Pick the movement from the held keys
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

/// The unit vector a view with this yaw and pitch (degrees) looks along, in camera space,
/// where Z is the opposite of the map's
pub fn forward(yaw: f32, pitch: f32) -> [f32; 3] {
    let (yaw_rad, pitch_rad) = (yaw.to_radians(), pitch.to_radians());
    [
        -yaw_rad.sin() * pitch_rad.cos(),
        pitch_rad.sin(),
        -yaw_rad.cos() * pitch_rad.cos(),
    ]
}

/// The unit vector to the right of a view with this yaw, in camera space
pub fn right(yaw: f32) -> [f32; 3] {
    let yaw_rad = yaw.to_radians();
    [yaw_rad.cos(), 0.0, -yaw_rad.sin()]
}
