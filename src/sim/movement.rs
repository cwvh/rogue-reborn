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

/// Ground speed crouched. The original's are not known; these keep the feet of the crouched
/// motions about as close to the ground's pace as the standing ones are.
pub fn crouched_speed(m: Movement) -> f32 {
    match m {
        Movement::Run => 300.0,
        Movement::Walk => 120.0,
        Movement::Back => 90.0,
        Movement::StrafeLeft | Movement::StrafeRight => 115.0,
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

/// A reload under way: how far through it is (0 to 1), and whether the magazine was empty
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Reloading {
    pub progress: f32,
    pub empty: bool,
}

/// The MP5 magazine changes, as the original picks them for a weapon whose reload class is
/// "subgun" (every primary weapon): twm_mp2 with rounds still in the magazine, twm_mp1 once it
/// is empty, and kneeling twm_mp4 and twm_mp3
pub const RELOAD_MOTION: &str = "twm_mp2";
pub const EMPTY_RELOAD_MOTION: &str = "twm_mp1";
pub const CROUCHED_RELOAD_MOTION: &str = "twm_mp4";
pub const CROUCHED_EMPTY_RELOAD_MOTION: &str = "twm_mp3";

/// A reload's beats: seconds into the reload, and the frame of its motion to show then. Each
/// beat falls just before its click in C_SBMAG1.WAV (the one magazine sound the weapon has), as
/// the sound of a hand comes a moment after it is seen. `frames` is the motion's length.
struct Beats {
    frames: f32,
    beats: &'static [(f32, f32)],
}

/// twm_mp2 runs the HK manual of arms: a fresh magazine from the pouch, the cocking handle back
/// and locked, the old magazine stripped, the new one seated, and the handle slapped down
const RELOAD_BEATS: Beats = Beats {
    frames: 100.0,
    beats: &[
        (0.0, 0.0),
        (0.40, 27.0), // hand on the cocking handle, pulling it back
        (0.60, 34.0), // handle locked open
        (1.05, 44.0), // old magazine stripped from the well
        (1.60, 60.0), // new magazine into the well
        (1.80, 64.0), // seated
        (2.10, 84.0), // the slap closes the bolt
        (super::RELOAD_SECONDS, 99.0),
    ],
};

/// twm_mp1 strips the empty magazine, takes a fresh one from the pouch, seats it and slaps the
/// bolt release on the left of the receiver, as on an M16
const EMPTY_RELOAD_BEATS: Beats = Beats {
    frames: 81.0,
    beats: &[
        (0.0, 0.0),
        (0.60, 12.0), // empty magazine stripped from the well
        (1.05, 28.0), // fresh one out of the pouch
        (1.60, 50.0), // into the well
        (1.80, 56.0), // seated
        (2.10, 60.0), // the bolt release, slapped
        (super::RELOAD_SECONDS, 72.0),
    ],
};

/// twm_mp4 keeps the old magazine: a fresh one from the pouch, the old one pulled down out of
/// the well with the same hand, the fresh one pushed up in, and the old one put away. The
/// reload is over on the way back from the pouch, and the hand eases back to the gun.
const CROUCHED_RELOAD_BEATS: Beats = Beats {
    frames: 86.0,
    beats: &[
        (0.0, 0.0),
        (0.45, 18.0), // at the pouch for a fresh magazine
        (1.25, 40.0), // old magazine pulled out of the well
        (1.60, 48.0), // fresh one into the well
        (1.80, 52.0), // seated
        (2.10, 66.0), // old one into the pouch
        (super::RELOAD_SECONDS, 72.0),
    ],
};

/// twm_mp3 strips the empty magazine, takes a fresh one from the pouch, seats it and slaps the
/// gun, as twm_mp1 does standing
const CROUCHED_EMPTY_RELOAD_BEATS: Beats = Beats {
    frames: 73.0,
    beats: &[
        (0.0, 0.0),
        (0.60, 13.0), // empty magazine stripped from the well
        (1.05, 27.0), // fresh one out of the pouch
        (1.60, 44.0), // into the well
        (1.80, 50.0), // seated
        (2.10, 56.0), // the slap
        (super::RELOAD_SECONDS, 64.0),
    ],
};

/// The frame of the reload motion to show for a reload, standing or crouched, in a motion of
/// `frames` frames. The beats are those of the original's motion for it; another motion is
/// stretched to fit.
pub fn reload_frame(r: Reloading, crouched: bool, frames: f32) -> f32 {
    let Beats { frames: length, beats } = match (r.empty, crouched) {
        (false, false) => RELOAD_BEATS,
        (true, false) => EMPTY_RELOAD_BEATS,
        (false, true) => CROUCHED_RELOAD_BEATS,
        (true, true) => CROUCHED_EMPTY_RELOAD_BEATS,
    };
    let t = r.progress.clamp(0.0, 1.0) * super::RELOAD_SECONDS;
    let frame = beats.windows(2).find(|w| t <= w[1].0).map_or(beats[beats.len() - 1].1, |w| {
        let ((t0, f0), (t1, f1)) = (w[0], w[1]);
        f0 + (f1 - f0) * (t - t0) / (t1 - t0)
    });
    frame / (length - 1.0) * (frames - 1.0).max(0.0)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reload_runs_through_its_motion_on_the_beats() {
        let at = |progress, empty| Reloading { progress, empty };
        assert_eq!(reload_frame(at(0.0, false), false, 100.0), 0.0);
        assert_eq!(reload_frame(at(1.0, false), false, 100.0), 99.0);
        // The slaps land as their sound does: the cocking handle, or the bolt release once empty
        let slap = 2.10 / super::super::RELOAD_SECONDS;
        assert!((reload_frame(at(slap, false), false, 100.0) - 84.0).abs() < 1e-3);
        assert!((reload_frame(at(slap, true), false, 81.0) - 60.0).abs() < 1e-3);
        // The empty reload ends back on the grip, before its motion's last frame
        assert_eq!(reload_frame(at(1.0, true), false, 81.0), 72.0);
        // Kneeling, the fresh magazine seats on the same click
        let seated = 1.80 / super::super::RELOAD_SECONDS;
        assert!((reload_frame(at(seated, false), true, 86.0) - 52.0).abs() < 1e-3);
        assert!((reload_frame(at(seated, true), true, 73.0) - 50.0).abs() < 1e-3);
        // Another motion is stretched to fit
        assert!((reload_frame(at(1.0, false), false, 50.0) - 49.0).abs() < 1e-3);
        for empty in [false, true] {
            for crouched in [false, true] {
                let frames: Vec<f32> = (0..=100).map(|i| reload_frame(at(i as f32 / 100.0, empty), crouched, 100.0)).collect();
                assert!(frames.windows(2).all(|w| w[1] >= w[0]), "{frames:?}");
            }
        }
    }
}
