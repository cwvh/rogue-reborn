//! How a character waits in its stance until something happens, as the original's terrorists
//! do (RogueSpear.exe 0x5ceb20). Each stance rests in a short loop. Now and then the character
//! fidgets, picked by chance from the stance's own, and then rests in the pose the fidget ends
//! in (that fidget's `r` motion, 0x5d1280) until the next. The wait is counted while resting
//! (0x5c6c18), here at the 30 a second the motions play at.
//!
//! What a character is doing is worked out afresh from when it started and its seed, so every
//! copy of the game poses it the same without being told.

use crate::mission::Stance;

/// One of a stance's fidgets, the rest it ends in, and the chance of it and those before it
/// (the original's tables at 0x76db18..0x76db8c)
struct Fidget {
    motion: &'static str,
    rest: &'static str,
    below: f32,
}

struct Idle {
    /// What the stance rests in before its first fidget
    rest: &'static str,
    fidgets: &'static [Fidget],
    /// The fewest and most motion frames of rest between fidgets
    wait: (u32, u32),
}

const fn fidget(motion: &'static str, rest: &'static str, below: f32) -> Fidget {
    Fidget { motion, rest, below }
}

// tib_2 is in the table twice, and the original's tib_4 is "NotInYet"
const STAND: &[Fidget] = &[
    fidget("tib_3", "tib_3r", 0.15), fidget("tib_2", "tib_2r", 0.45), fidget("tib_4a", "tib_4ar", 0.6),
    fidget("tib_5", "tib_5r", 0.65), fidget("tib_6", "tib_6r", 0.7), fidget("tib_7", "tib_7r", 0.75),
    fidget("tib_8", "tib_8r", 0.8), fidget("tib_9", "tib_9r", 0.85), fidget("tib_10", "tib_10r", 2.0),
];
const LEAN: &[Fidget] = &[fidget("til_1", "til_1r", 0.3), fidget("til_2", "til_2r", 0.6), fidget("til_3", "til_3r", 2.0)];
const SQUAT: &[Fidget] = &[fidget("tic_1", "tic_1r", 0.33), fidget("tic_2", "tic_2r", 0.66), fidget("tic_3", "tic_3r", 2.0)];
const KNEEL: &[Fidget] = &[fidget("hik_1", "hik_1r", 0.33), fidget("hik_2", "hik_2r", 0.66), fidget("hik_3", "hik_3r", 2.0)];
const SLOUCH: &[Fidget] = &[fidget("tis_1", "tis_1r", 2.0)];

// Between 120 and 400 counts, and then 120 more (0x5cfc92), or 95 to 300 and 95 more (0x5d11d6)
const LONG_WAIT: (u32, u32) = (240, 520);
const SHORT_WAIT: (u32, u32) = (190, 395);

fn of(stance: Stance) -> Idle {
    match stance {
        Stance::Stand => Idle { rest: "tib_1r", fidgets: STAND, wait: SHORT_WAIT },
        Stance::Lean => Idle { rest: "til_1r", fidgets: LEAN, wait: SHORT_WAIT },
        Stance::Squat => Idle { rest: "tic_1r", fidgets: SQUAT, wait: LONG_WAIT },
        Stance::Kneel => Idle { rest: "hik_1r", fidgets: KNEEL, wait: LONG_WAIT },
        Stance::Slouch => Idle { rest: "tis_1r", fidgets: SLOUCH, wait: LONG_WAIT },
        // Hostages' stances, which only rest for now
        Stance::Cower => Idle { rest: "hrc_r", fidgets: &[], wait: LONG_WAIT },
        Stance::Sit => Idle { rest: "tmt_crsnr", fidgets: &[], wait: LONG_WAIT },
        Stance::SitInChair => Idle { rest: "tis_1r", fidgets: &[], wait: LONG_WAIT },
    }
}

/// Every motion a stance plays, to load
pub fn motions() -> Vec<&'static str> {
    Stance::ALL.into_iter().flat_map(|s| {
        let idle = of(s);
        std::iter::once(idle.rest).chain(idle.fidgets.iter().flat_map(|f| [f.motion, f.rest]))
    }).collect()
}

/// How many frames a fidget blends in over from the rest before it, and a rest from the fidget
/// (the original's blends are 5, 8 or 10 frames)
const BLEND: f32 = 8.0;

/// A motion as it plays: its frame, and whether it loops (a rest) or plays once (a fidget)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playing<'a> {
    pub motion: &'a str,
    pub frame: f32,
    pub looping: bool,
}

/// What a character in `stance` plays `t` seconds after it started, and what it is still
/// blending from and how much (1 at the start of the blend). `frames` says how long each
/// motion is.
pub fn at(stance: Stance, seed: u64, t: f32, frames: impl Fn(&str) -> f32) -> (Playing<'static>, Option<(Playing<'static>, f32)>) {
    let idle = of(stance);
    let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || (crate::util::next_rng(&mut rng) >> 40) as f32 / (1u64 << 24) as f32;
    let (mut rest, mut start) = (idle.rest, 0.0);
    // What played before this, as it was when it ended
    let mut before: Option<Playing> = None;
    let blend = |now: Playing<'static>, before: Option<Playing<'static>>| {
        let from = before.filter(|_| now.frame < BLEND).map(|b| (b, 1.0 - now.frame / BLEND));
        (now, from)
    };
    loop {
        let wait = (idle.wait.0 as f32 + next() * (idle.wait.1 - idle.wait.0) as f32) / 30.0;
        if t < start + wait || idle.fidgets.is_empty() {
            return blend(Playing { motion: rest, frame: (t - start) * 30.0, looping: true }, before);
        }
        start += wait;
        before = Some(Playing { motion: rest, frame: wait * 30.0, looping: true });
        let r = next();
        let f = idle.fidgets.iter().find(|f| r < f.below).unwrap_or(&idle.fidgets[0]);
        let length = frames(f.motion).max(1.0) / 30.0;
        if t < start + length {
            return blend(Playing { motion: f.motion, frame: (t - start) * 30.0, looping: false }, before);
        }
        start += length;
        before = Some(Playing { motion: f.motion, frame: length * 30.0, looping: false });
        rest = f.rest;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stance_rests_then_fidgets_then_rests_in_the_pose_the_fidget_ends_in() {
        let frames = |_: &str| 90.0;
        let (now, from) = at(Stance::Lean, 3, 1.0, frames);
        assert_eq!((now.motion, now.looping, from), ("til_1r", true, None));
        assert!((now.frame - 30.0).abs() < 1e-3);
        // Each kind of thing in turn, in time: rest, fidget, the fidget's rest, each blending in
        // from the one before
        let mut seen = Vec::new();
        for i in 0..2000 {
            let (now, from) = at(Stance::Lean, 3, i as f32 * 0.05, frames);
            if seen.last() != Some(&(now.motion, now.looping)) {
                if let Some(&(motion, _)) = seen.last() {
                    let (before, weight) = from.unwrap();
                    assert_eq!(before.motion, motion);
                    assert!(weight > 0.5, "{weight}");
                }
                seen.push((now.motion, now.looping));
            }
        }
        assert!(seen.len() >= 5, "{seen:?}");
        for w in seen.windows(2) {
            match (w[0], w[1]) {
                ((_, true), (fidget, false)) => assert!(LEAN.iter().any(|f| f.motion == fidget)),
                ((fidget, false), (rest, true)) => assert_eq!(LEAN.iter().find(|f| f.motion == fidget).unwrap().rest, rest),
                other => panic!("{other:?} in {seen:?}"),
            }
        }
    }

    #[test]
    fn standing_fidgets_come_as_often_as_the_original_picks_them() {
        // tib_2 is a fidget in three and tib_5 in twenty. Long fidgets, so that every
        // character is in its first just past the longest wait.
        let frames = |_: &str| 1.0e6;
        let mut counts = std::collections::HashMap::new();
        for seed in 0..4000 {
            let (now, _) = at(Stance::Stand, seed, SHORT_WAIT.1 as f32 / 30.0 + 0.5, frames);
            if !now.looping {
                *counts.entry(now.motion).or_insert(0) += 1;
            }
        }
        let n: i32 = counts.values().sum();
        let share = |m| *counts.get(m).unwrap_or(&0) as f32 / n as f32;
        assert!((share("tib_2") - 0.3).abs() < 0.04 && (share("tib_5") - 0.05).abs() < 0.02, "{counts:?}");
        assert_eq!(share("tib_1"), 0.0);
    }

    #[test]
    fn every_stance_motion_is_in_the_game() {
        if !std::path::Path::new("data/motion").exists() {
            return;
        }
        for m in motions() {
            assert!(crate::util::find_file(std::path::Path::new("data/motion"), m, "dam").is_some(), "no motion {m}");
        }
    }
}
