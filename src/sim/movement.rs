//! How the player moves: the kinds of movement, how fast each goes, and which way the
//! player faces.

/// What the player's body is doing, which picks the animation
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Movement {
    Idle,
    Walk,
    Run,
    Back,
    /// Backing up with the run key held (wk2_bk)
    RunBack,
    /// Stepping to the side running (tms_l2 / tms_r2), and walking (tms_l1 / tms_r1), as the
    /// original picks them. Crouched there is the one pace.
    StrafeLeft,
    StrafeRight,
    WalkLeft,
    WalkRight,
    /// Standing and shooting, or reloading, with the weapon
    Fire,
    Reload,
    /// Throwing a grenade
    Throw,
}

/// Ground speed (units per second) of each movement: the travel per frame in the original's
/// motion times `MOTION_FPS`. Standing and healthy: tmu_wk2, tmu_rn, tmu_bk2, wk2_bk,
/// tms_l2/r2 and (walking) tms_l1/r1.
pub fn ground_speed(m: Movement) -> f32 {
    match m {
        Movement::Walk => 300.0,
        Movement::Run => 494.0,
        Movement::Back => 139.0,
        Movement::RunBack => 296.0,
        Movement::StrafeLeft => 263.0,
        Movement::StrafeRight => 266.0,
        Movement::WalkLeft => 69.0,
        Movement::WalkRight => 61.0,
        Movement::Idle | Movement::Fire | Movement::Reload | Movement::Throw => 1.0,
    }
}

/// Crouched: rmc_rn, tmc_fd, tmc_bk and tmc_lf/rt. A crouched strafe has one motion a side,
/// walking or running.
pub fn crouched_speed(m: Movement) -> f32 {
    match m {
        Movement::Run => 360.0,
        Movement::Walk => 70.0,
        Movement::Back | Movement::RunBack => 45.0,
        Movement::StrafeLeft | Movement::WalkLeft => 41.0,
        Movement::StrafeRight | Movement::WalkRight => 55.0,
        other => ground_speed(other),
    }
}

/// The speed of the original's wounded motion: rmu_wrn, tmu_wwk2, tmu_wbk2, tmu_wlf1/wrt1
/// (walking or running) and, crouched, rmc_wrn. The wounded strafe is faster than a healthy
/// walking strafe; `pace` keeps the slower one.
pub fn limp_speed(m: Movement, crouched: bool) -> f32 {
    match (m, crouched) {
        (Movement::Run, false) => 373.0,
        (Movement::Run, true) => 213.0,
        (_, true) => crouched_speed(m),
        (Movement::Walk, false) => 202.0,
        (Movement::Back | Movement::RunBack, false) => 189.0,
        (Movement::StrafeLeft | Movement::WalkLeft, false) => 104.0,
        (Movement::StrafeRight | Movement::WalkRight, false) => 123.0,
        _ => ground_speed(m),
    }
}

/// The ground speed at which a motion plays at its own pace, so the feet do not slide. The
/// wound slowdown is left out, so wounds slow the animation too.
pub fn animation_pace(m: Movement, crouched: bool, limp: bool) -> f32 {
    match (limp, crouched) {
        (true, _) => limp_speed(m, crouched),
        (false, true) => crouched_speed(m),
        (false, false) => ground_speed(m),
    }
}

/// The original stretches every animation by 5% for each wound, up to 40% (0x5c78c0)
pub fn wound_slowdown(wounds: u8) -> f32 {
    1.0 / (1.0 + 0.05 * wounds as f32).min(1.4)
}

pub fn pace(m: Movement, crouched: bool, wounds: u8, leg_wounds: u8) -> f32 {
    let base = if crouched { crouched_speed(m) } else { ground_speed(m) };
    let speed = if leg_wounds > 0 { limp_speed(m, crouched).min(base) } else { base };
    speed * wound_slowdown(wounds)
}

/// Pick the movement from the held keys
pub fn movement_from_keys(forward: bool, back: bool, left: bool, right: bool, run: bool) -> Movement {
    if forward {
        if run { Movement::Run } else { Movement::Walk }
    } else if back {
        if run { Movement::RunBack } else { Movement::Back }
    } else if left && !right {
        if run { Movement::StrafeLeft } else { Movement::WalkLeft }
    } else if right && !left {
        if run { Movement::StrafeRight } else { Movement::WalkRight }
    } else {
        Movement::Idle
    }
}

/// How far the hips turn toward the way a player goes when they move forward or back and to the
/// side at once, in radians: the original's pi * 0.15 (27 degrees). The legs keep the forward or
/// back motion, and the chest turns back as far, so the gun stays on the aim.
pub const HIP_TURN: f32 = std::f32::consts::PI * 0.15;

/// How fast the hips turn there and back, in radians a second (the original's pi / 4)
pub const HIP_TURN_RATE: f32 = std::f32::consts::FRAC_PI_4;

/// How far the hips turn for the keys held (positive is to the right): toward the side when going
/// forward, away from it when backing up (the back of the hips leads), and not at all otherwise
pub fn hip_turn(forward: bool, back: bool, left: bool, right: bool) -> f32 {
    let side = match (left, right) {
        (true, false) => -1.0,
        (false, true) => 1.0,
        _ => return 0.0,
    };
    match (forward, back) {
        (true, false) => side * HIP_TURN,
        (false, true) => -side * HIP_TURN,
        _ => 0.0,
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
    fn every_speed_is_its_motions_travel_per_frame_times_the_frame_rate() {
        use Movement::*;
        if !std::path::Path::new("data/motion").exists() { return; }
        let speed = |name: &str, side: bool| {
            let v = crate::motion::read(std::path::Path::new(&format!("data/motion/{name}.DAM"))).unwrap().velocity;
            (if side { v[0] } else { v[2] }).abs() * crate::motion::MOTION_FPS
        };
        let near = |what: &str, got: f32, want: f32| assert!((got - want).abs() <= 1.0, "{what}: {got} for {want}");
        for (m, name, side) in [
            (Walk, "tmu_wk2", false), (Run, "tmu_rn", false), (Back, "tmu_bk2", false), (RunBack, "wk2_bk", false),
            (StrafeLeft, "tms_l2", true), (StrafeRight, "tms_r2", true),
            (WalkLeft, "tms_l1", true), (WalkRight, "tms_r1", true),
        ] {
            near(name, ground_speed(m), speed(name, side));
        }
        for (m, name, side) in [
            (Walk, "tmc_fd", false), (Run, "rmc_rn", false), (Back, "tmc_bk", false),
            (StrafeLeft, "tmc_lf", true), (StrafeRight, "tmc_rt", true),
        ] {
            near(name, crouched_speed(m), speed(name, side));
        }
        for (m, name, side) in [
            (Walk, "tmu_wwk2", false), (Run, "rmu_wrn", false), (Back, "tmu_wbk2", false),
            (StrafeLeft, "tmu_wlf1", true), (StrafeRight, "tmu_wrt1", true),
            (WalkLeft, "tmu_wlf1", true), (WalkRight, "tmu_wrt1", true),
        ] {
            near(name, limp_speed(m, false), speed(name, side));
        }
        near("rmc_wrn", limp_speed(Run, true), speed("rmc_wrn", false));
        near("rmu_wk2", crate::sim::terrorist::WALK_SPEED, speed("rmu_wk2", false));
        near("rmu_wk3", crate::sim::terrorist::RUN_SPEED, speed("rmu_wk3", false));
        near("rmu_wk1", crate::sim::terrorist::CREEP_SPEED, speed("rmu_wk1", false));
    }

    #[test]
    fn the_animation_pace_is_the_speed_of_the_motion_drawn() {
        use Movement::*;
        for m in [Walk, Run, Back, RunBack, StrafeLeft, StrafeRight, WalkLeft, WalkRight] {
            assert_eq!(animation_pace(m, false, false), ground_speed(m), "{m:?}");
            assert_eq!(animation_pace(m, false, true), limp_speed(m, false), "{m:?}");
            assert_eq!(animation_pace(m, true, false), crouched_speed(m), "{m:?}");
        }
        for m in [Walk, Back, StrafeLeft, WalkRight] {
            assert_eq!(animation_pace(m, true, true), crouched_speed(m), "{m:?}");
        }
        assert_eq!(animation_pace(Run, true, true), 213.0);
    }

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
