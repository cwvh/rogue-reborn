//! The lab's numbers, held (see `net::lab` and `rrlab`). Each limit is what the lab measured
//! when it was set, with room to spare, so that a change that makes play less fair or less
//! smooth fails here. When tuning makes a figure better, tighten its limit.
//!
//! Measured on 2026-11-01. Pings are round trips.

use crate::net::lab::{Fight, Tally, fight, internet, peek};
use crate::net::link::Conditions;

/// Shots the server judges as the shooter's screen showed them: on the LAN, and from far away
/// (measured 100% for both)
const AGREE_LAN: f32 = 0.97;
const AGREE_FAR: f32 = 0.95;

/// The mean gap (units) between where a victim was on their own screen and where they were hit:
/// on the LAN, and at 100 ms (measured 40 and 58)
const GAP_LAN: f32 = 55.0;
const GAP_FAR: f32 = 80.0;

/// Seconds to be told of one's own shot, and of one's own death: on the LAN, and at 100 ms
/// (measured 43 and 26 ms; 158 and 77 ms)
const SHOT_TOLD_LAN: f32 = 0.060;
const DEATH_TOLD_LAN: f32 = 0.040;
const SHOT_TOLD_FAR: f32 = 0.190;
const DEATH_TOLD_FAR: f32 = 0.100;

/// The server moving a player from where they predicted, a minute, and the furthest (units)
/// (measured none on the LAN, one of 6 units with players at 100 ms)
const FIXES_A_MINUTE: f32 = 0.5;
const WORST_FIX: f32 = 50.0;

/// Bytes a second for 16 players: the host's upload, and the most any client downloads
/// (measured 92 KB and 7 KB)
const UPLOAD_16: f32 = 120_000.0;
const DOWNLOAD_16: f32 = 9_000.0;

/// Peeker's advantage (seconds): both on the LAN, the peeker at 100 and 150 ms, and the holder
/// at 100 ms (measured 92, 151, 190 and 137 ms)
const PEEK_LAN: f32 = 0.120;
const PEEK_FAR_100: f32 = 0.190;
const PEEK_FAR_150: f32 = 0.235;
const PEEK_HOLDER_FAR: f32 = 0.175;

/// Of the holder's hits, those on a peeker back behind the corner on their own screen: the
/// peeker at 100 ms, and the holder at 100 ms (measured 20% and 24%)
const BEHIND_PEEKER_FAR: f32 = 0.30;
const BEHIND_HOLDER_FAR: f32 = 0.33;

fn mean(v: &[f32]) -> f32 {
    assert!(!v.is_empty(), "nothing measured");
    v.iter().sum::<f32>() / v.len() as f32
}

/// Fixes a minute for each player, and the furthest
fn fixes(f: &Fight, seconds: f32) -> (f32, f32) {
    let n = (f.lan.fixes + f.remote.fixes) as f32;
    (n * 60.0 / seconds / f.meters.len() as f32, f.lan.worst_fix.max(f.remote.worst_fix))
}

fn behind(t: &Tally) -> f32 {
    t.behind as f32 / t.gaps.len().max(1) as f32
}

#[test]
fn a_fight_on_the_lan_is_fair_and_smooth() {
    let f = fight(8, 0, Conditions::lan(), 45.0);
    assert!(f.lan.shots > 100, "{f:?}");
    assert!(f.lan.agreement() >= AGREE_LAN, "agreement {}", f.lan.agreement());
    assert!(mean(&f.lan.gaps) <= GAP_LAN, "gap {}", mean(&f.lan.gaps));
    assert!(mean(&f.lan.shot_delays) <= SHOT_TOLD_LAN, "told of a shot after {}", mean(&f.lan.shot_delays));
    assert!(mean(&f.lan.death_delays) <= DEATH_TOLD_LAN, "told of a death after {}", mean(&f.lan.death_delays));
    let (a_minute, worst) = fixes(&f, 45.0);
    assert!(a_minute <= FIXES_A_MINUTE && worst <= WORST_FIX, "{a_minute} fixes a minute, the worst {worst}");
}

#[test]
fn players_far_away_are_judged_as_they_saw_and_told_soon() {
    let f = fight(8, 2, internet(0.10), 45.0);
    assert!(f.remote.shots > 20, "{f:?}");
    assert!(f.lan.agreement() >= AGREE_LAN, "on the LAN {}", f.lan.agreement());
    assert!(f.remote.agreement() >= AGREE_FAR, "far away {}", f.remote.agreement());
    assert!(mean(&f.remote.gaps) <= GAP_FAR, "gap {}", mean(&f.remote.gaps));
    assert!(mean(&f.remote.shot_delays) <= SHOT_TOLD_FAR, "told of a shot after {}", mean(&f.remote.shot_delays));
    assert!(mean(&f.remote.death_delays) <= DEATH_TOLD_FAR, "told of a death after {}", mean(&f.remote.death_delays));
    let (a_minute, worst) = fixes(&f, 45.0);
    assert!(a_minute <= FIXES_A_MINUTE && worst <= WORST_FIX, "{a_minute} fixes a minute, the worst {worst}");
}

#[test]
fn a_full_game_fits_a_home_connection() {
    let f = fight(14, 2, internet(0.10), 30.0);
    assert!(f.upload <= UPLOAD_16, "{} bytes a second up", f.upload);
    let down = f.meters.iter().map(|(_, m)| m.bytes_in).fold(0.0, f32::max);
    assert!(down <= DOWNLOAD_16, "{down} bytes a second down");
}

#[test]
fn peeking_gives_the_one_moving_only_a_little_first_look() {
    let lan = Conditions::lan();
    for (holder, peeker, limit) in [(lan, lan, PEEK_LAN), (lan, internet(0.10), PEEK_FAR_100), (lan, internet(0.15), PEEK_FAR_150), (internet(0.10), lan, PEEK_HOLDER_FAR)] {
        let p = peek(holder, peeker, 30.0);
        assert!(p.advantage.len() >= 10, "{p:?}");
        assert!(mean(&p.advantage) <= limit, "{} against {limit}", mean(&p.advantage));
    }
}

#[test]
fn few_hits_land_behind_cover() {
    let lan = Conditions::lan();
    for (holder, peeker, limit) in [(lan, internet(0.10), BEHIND_PEEKER_FAR), (internet(0.10), lan, BEHIND_HOLDER_FAR)] {
        let p = peek(holder, peeker, 30.0);
        assert!(p.holder.agreement() >= AGREE_LAN, "agreement {}", p.holder.agreement());
        assert!(behind(&p.peeker) <= limit, "{} of hits behind cover", behind(&p.peeker));
    }
}
