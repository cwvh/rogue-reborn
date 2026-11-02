//! What a mission's terrorists do about the players: see them, turn to them and fire, through
//! the same spread as players (sim/aim.rs). How they see, aim and when they pull the trigger are
//! the original's (docs/terrorist-ai.md); the values it sets at run time, and how fast they turn,
//! are guesses here. They take aim at a player only near where they face, and their aim swings
//! across to them, ever faster, before they fire.

use std::rc::Rc;

use crate::mission::{Alertness, Condition, Plan, Reaction, Roe, Speed, Stance, Step};
use crate::sim::characters::Gait;
use crate::sim::aim::{Aim, Gun, Pace};

type V3 = [f32; 3];

/// The normal view angle, the full width (RogueSpear.exe 0x8169f8, set at run time: a guess)
pub const NORMAL_VIEW: f32 = 90.0;
/// Within this they see a player whichever way they face (0x8169a0, set at run time: a guess)
pub const CLOSE: f32 = 200.0;
/// Degrees a second they turn toward a target (not read yet: a guess), and faster when they
/// were already alert
pub const TURN_RATE: f32 = 180.0;
pub const ALERT_TURN_RATE: f32 = 270.0;
/// How long a terrorist standing, squatting or kneeling takes to raise its weapon before it can
/// fire, easing from waiting into aiming. The original plays this by a blend whose length it
/// sets at run time (RogueSpear.exe 0x816b14): a guess, to be timed against it.
pub const RAISE_SECONDS: f32 = 0.5;
/// The original's motions for standing up with the rifle from leaning and from slouching
pub const RAISE_MOTIONS: [&str; 2] = ["rmt_llup", "rmt_slup"];

/// Which way the `k`th pellet of a shot along `dir` goes (0x588c70): off it by up to `spread`
/// in 100 across and up, `u` (each 0 to 1) saying how far, in each quarter round it in turn so
/// that the pellets spread all round
pub fn pellet(dir: V3, k: u32, spread: f32, u: [f32; 2]) -> V3 {
    let (sx, sy) = [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)][(k % 4) as usize];
    let (x, y) = (sx * u[0] * spread, sy * u[1] * spread);
    // Across: level and square to the shot; up: square to both
    let flat = (dir[0] * dir[0] + dir[2] * dir[2]).sqrt();
    let across = if flat > 1e-4 { [dir[2] / flat, 0.0, -dir[0] / flat] } else { [1.0, 0.0, 0.0] };
    let up = [dir[1] * across[2] - dir[2] * across[1], dir[2] * across[0] - dir[0] * across[2], dir[0] * across[1] - dir[1] * across[0]];
    let v = [0, 1, 2].map(|c| dir[c] * 100.0 + across[c] * x + up[c] * y);
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|c| c / len)
}

/// The motions a terrorist's arms play with a weapon, each standing and then kneeling
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arms {
    pub hold: [&'static str; 2],
    pub fire: [&'static str; 2],
    pub reload: Option<[&'static str; 2]>,
}

/// The arms a terrorist's weapon gives it, as the original's tables have them for terrorists:
/// the hold by how the weapon is held (0x5c7f70), firing by its kind (0x5c84c0), and the
/// reload by how it reloads (0x5c87d0)
pub fn arms(w: &crate::weapon::Weapon) -> Arms {
    let hold = match w.hold.as_str() {
        "pistol" | "compact" => ["rwru_p1", "rwrc_p1"],
        "subgun" => ["rwru_s1", "rwrc_s1"],
        "long arm" | "sniper" => ["rwru_l1", "rwrc_l1"],
        _ => ["rmr_up", "rmr_cr"],
    };
    let fire = match w.class.as_str() {
        "subgun" => ["rwru_s9", "twrc_s9"],
        "long arm" | "sniper" | "bolt action" => ["rwru_l9", "rwrc_l9"],
        "grenade launcher" => ["rwru_s9", "rwrc_s9"],
        _ => ["rwru_p9", "rwrc_p9"],
    };
    let reload = match w.reload_class.as_str() {
        "pistol" => Some(["ter_pis_mag_change", "mg_w1_cr"]),
        "subgun" => Some(["ter_mag_change", "twm_mp3"]),
        "shotgun" => Some(["twm_shu_loop", "twm_shu_loop"]),
        "bolt action" => Some(["twm_shu", "twm_snc"]),
        _ => None,
    };
    Arms { hold, fire, reload }
}
/// Through walls a sound carries this much of its reach (a guess)
pub const MUFFLED: f32 = 0.5;
/// Seconds a terrorist who heard something keeps its weapon up toward it (a guess)
pub const HEARD_SECONDS: f32 = 10.0;
/// How fast they walk and run, as a player does (units a second), and creep, at half a walk
pub const WALK_SPEED: f32 = 514.0;
pub const RUN_SPEED: f32 = 735.0;
pub const CREEP_SPEED: f32 = 257.0;
/// A sound that carries this far is gunfire or a blast, not footsteps or a door
const LOUD: f32 = 5000.0;

/// A noise a terrorist may hear: footsteps, which do not carry through walls, or any other
/// sound
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Noise {
    Footstep,
    Sound,
}

/// How sure a terrorist is of a threat, as the mission's triggers grade it (0x61e8d0): the
/// worst of the threats it knows of
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Threat {
    #[default]
    None,
    /// It heard something quiet: a door, or footsteps through a wall
    Suspicious,
    /// It heard footsteps or gunfire far off
    Distant,
    /// It heard them near, saw an enemy a while ago, or was shot
    Immediate,
    /// It sees an enemy now
    Deadly,
}

/// A threat a terrorist knows of, and when it last noticed it (the original keeps five, 0x61e680)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Known {
    pub sign: Sign,
    /// The player it is, when known
    pub who: Option<usize>,
    pub at: V3,
    pub since: f32,
}

/// How a terrorist came to know of a threat
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    Seen,
    Shot,
    Heard,
    /// A door, or footsteps through a wall
    Quiet,
}

/// The most threats a terrorist keeps track of
const KNOWN: usize = 5;
/// Heard nearer than this, a threat is Immediate, else Distant (0x61e8d0)
const NEAR_THREAT: f32 = 2000.0;
/// Seconds a quiet threat is remembered, and any other (0x61ee60; 15 for whatever its method
/// +0x10 picks out, hostages perhaps)
const FORGET_QUIET: f32 = 5.0;
const FORGET_THREAT: f32 = 40.0;
/// Seconds of nothing new after which a suspicious terrorist is bored again (0x61e4b0)
const CALM_SECONDS: f32 = 30.0;

/// What a team's members are sure of, which its trigger plans go by: which threat levels and
/// which awarenesses (0 none, 1 suspicious, 2 aware) any of them has
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TeamState {
    pub threats: [bool; 5],
    pub awareness: [bool; 3],
}

/// A team's orders from its mission: its plan, the plans it turns to when set off, and the
/// mission's zones (their rooms, by zone)
#[derive(Clone, Debug, Default)]
pub struct Orders {
    pub plan: Plan,
    pub reactions: Vec<Reaction>,
    pub zones: std::collections::HashMap<u32, Vec<u32>>,
}

/// Seconds a terrorist patrolling its zones stays in each room it comes to (a guess)
pub const ZONE_PAUSE: f32 = 3.0;

/// Where a plan's way sends a terrorist next
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    At(V3),
    /// A room of its zones
    Room(u32),
    /// Where the character it hunts is
    Hunt,
}

/// How far apart a team taking cover stands (0x60a5a0)
const COVER_SPACING: f32 = 150.0;
/// Seconds a hunter that got where its prey is waits before it looks for the way again
const HUNT_PAUSE: f32 = 1.0;

/// Where the `k`th of a team takes cover at `at`, watching `watch` (0x60a5a0): the first at
/// `at`, and the others in a wedge behind it, 150 units back and aside for each two, on
/// alternate sides
pub fn cover_spot(watch: V3, at: V3, k: usize) -> V3 {
    let (dx, dz) = (watch[0] - at[0], watch[2] - at[2]);
    let len = (dx * dx + dz * dz).sqrt().max(1e-3);
    let (dx, dz) = (dx / len, dz / len);
    let m = (k as f32 * 0.5).ceil() * COVER_SPACING;
    let side = if k % 2 == 0 { 1.0 } else { -1.0 };
    [at[0] - m * dx + side * m * dz, at[1], at[2] - m * dz - side * m * dx]
}

/// Which way, in degrees, the `k`th of a team of `n` faces taking cover at `at`, watching
/// `watch` (0x60a5a0): toward it, fanned out by a share of a half turn
pub fn cover_yaw(watch: V3, at: V3, k: usize, n: usize) -> f32 {
    let toward = (watch[0] - at[0]).atan2(watch[2] - at[2]).to_degrees();
    let a = 180.0 / (n + 1) as f32;
    let turn = if n % 2 == 0 {
        let a = if k < 3 { a * 0.5 } else { a };
        if k % 2 == 0 { -a } else { a }
    } else {
        match k {
            0 => 0.0,
            2 => -a,
            _ => a,
        }
    };
    toward + turn
}
/// How near a point on its way it must come before it heads for the next
const ARRIVE: f32 = 30.0;
/// The highest step up it takes walking, from one tick to the next
const STEP: f32 = 60.0;

/// How far a terrorist hears a player's footsteps at a pace: half as far as the original's
/// sounds carry for an operative with light gear on a hard floor (soundvol.txt's F_TWLEQ and
/// F_TRLEQ), and not through walls; creeping, or still, not at all. A guess, as hearing every
/// step to the sounds' full reach made them far too sharp.
pub fn footstep_reach(pace: Pace) -> f32 {
    match pace {
        Pace::Stopped | Pace::Shuffle => 0.0,
        Pace::Walk => 400.0,
        Pace::WalkFast | Pace::Run => 500.0,
    }
}
/// Seconds before a terrorist looks for a way to what it heard again, and again toward much
/// the same place (it remembers where it found none), unless a door opens or shuts
const ASK_AGAIN: f32 = 1.0;
const ASK_SAME: f32 = 5.0;
const SAME_PLACE: f32 = 500.0;
/// How many ways are looked for in a tick, among all of them: the rest wait their turn, so
/// that a shot many hear does not hold up the game (five on rm16 set off within 0.15 seconds)
const SEARCHES_PER_TICK: usize = 1;
/// Seconds a terrorist takes to draw its sidearm when its own weapon is empty (a guess: the
/// original has no motion for it)
const DRAW_SECONDS: f32 = 1.0;
/// How far a terrorist with nothing left to fire runs, at most, in units of walking
const FLEE_REACH: f32 = 2500.0;
/// Seconds a terrorist waits for someone in its way before it goes on to the next point
const BLOCKED_SECONDS: f32 = 1.0;
/// How fast terrorists too close together are pushed apart, in units a second
const PUSH_SPEED: f32 = 150.0;
/// Seconds they keep to a target they no longer see
pub const FORGET: f32 = 5.0;
/// How near where they face a player must be for them to take aim, in degrees either side:
/// a fifth of the view (0x5a83e0)
pub const AIM_CONE: f32 = 0.2 * NORMAL_VIEW;

/// What a terrorist knows and holds, which only the game's own copy keeps
#[derive(Clone, Debug)]
pub struct Mind {
    pub alertness: Alertness,
    /// Their Assault skill, out of 1, which they aim with
    pub skill: f32,
    /// Their Self-control, out of 1, which says when they pull the trigger
    pub self_control: f32,
    /// The mission's Max LOS Distance
    pub sight: f32,
    /// The player they are after, and when they last saw them
    pub target: Option<usize>,
    pub seen_at: f32,
    pub aim: Aim,
    /// Where their aim points, a unit vector from the eye, while they have a target, and how
    /// long it has been swinging toward them
    pub aim_dir: Option<V3>,
    pub swing: f32,
    /// Whether they were already alert when they took this target, which they turn to faster
    pub quick: bool,
    /// The nearest thing they last heard, and when
    pub heard: Option<(V3, f32)>,
    /// Where it last looked for a way to something it heard, and when; and where it means
    /// to go and see, how, while it waits its turn to look for the way
    pub asked: Option<(V3, f32)>,
    pub seek: Option<(V3, Gait)>,
    /// The points they are walking to in turn, each with the doorway on the way there (see
    /// `nav::Grid::path`), and how
    pub path: Vec<(V3, Option<u32>)>,
    pub gait: Gait,
    /// Their team, its orders, which of its trigger plans it has turned to (the last of them
    /// leads) and which have gone off
    pub team: usize,
    pub orders: Rc<Orders>,
    pub active: Option<usize>,
    pub fired: Vec<bool>,
    /// How sure they are of a threat, from what they know of, and whether they heard gunfire
    /// (`Trigger Sound Combat`)
    pub threat: Threat,
    pub known: Vec<Known>,
    pub heard_gunfire: bool,
    /// How aware they are that something is up (0 none, 1 suspicious, 2 aware), when that was
    /// last raised, and the alertness the mission gave them, which it never falls below
    pub awareness: u8,
    pub aware_at: f32,
    pub start: Alertness,
    /// Where they are on their plan's way: the step, the loop it is in, the step their walking
    /// is for (none when it is to see what they heard), and the speed it set
    pub step: usize,
    pub loop_at: Option<usize>,
    pub errand: Option<usize>,
    pub speed: Option<f32>,
    /// The zones it patrols, and until when it stays in the room it came to
    pub zones: Vec<u32>,
    pub pause_until: f32,
    pub gun: Gun,
    pub ammo: u32,
    /// Full magazines left
    pub magazines: u32,
    pub magazine: u32,
    /// When its reload ends, and how long it takes
    pub reload_until: Option<f32>,
    pub reload_for: f32,
    pub next_shot: f32,
    /// The sidearm it has not yet drawn: which of the characters' weapons it is, its gun, and
    /// its magazine and how many it has
    pub sidearm: Option<(usize, Gun, u32, u32)>,
    /// With nothing left to fire, what it does
    pub spent: Option<Spent>,
    /// Which of its team it is, whether it is where its plan has it take cover, whether it
    /// may fire on hostages, and the character it is firing on
    pub member: usize,
    pub covering: bool,
    pub engage_hostages: bool,
    pub prey: Option<usize>,
    /// Since when someone has stood in its way
    pub blocked_since: Option<f32>,
}

/// What a terrorist with nothing left to fire does
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spent {
    /// It has to find a way to run
    Out,
    /// It runs from the fight
    Running,
    /// It cowers where it ran to
    Hiding,
}

impl Mind {
    pub fn new(alertness: Alertness, skill: f32, self_control: f32, sight: f32, gun: Gun, magazine: u32, magazines: u32) -> Mind {
        Mind {
            alertness, skill, self_control, sight, target: None, seen_at: 0.0, aim: Aim::default(), aim_dir: None, swing: 0.0, quick: false, heard: None, asked: None, seek: None, path: Vec::new(), gait: Gait::Walk,
            team: 0, orders: Rc::new(Orders::default()), active: None, fired: Vec::new(), threat: Threat::None, known: Vec::new(), heard_gunfire: false,
            awareness: 0, aware_at: 0.0, start: alertness,
            step: 0, loop_at: None, errand: None, speed: None, zones: Vec::new(), pause_until: 0.0, gun,
            ammo: magazine, magazines: magazines.saturating_sub(1), magazine, reload_until: None, reload_for: super::RELOAD_SECONDS, next_shot: 0.0,
            sidearm: None, spent: None, blocked_since: None, member: 0, covering: false, engage_hostages: false, prey: None,
        }
    }

    /// Carrying a sidearm too, `weapon` of the characters' weapons, to draw when its own is empty
    pub fn with_sidearm(mut self, weapon: usize, gun: Gun, magazine: u32, magazines: u32) -> Mind {
        self.sidearm = Some((weapon, gun, magazine, magazines));
        self
    }

    /// One of a team, with its orders
    pub fn with_orders(mut self, team: usize, orders: Rc<Orders>) -> Mind {
        self.fired = vec![false; orders.reactions.len()];
        (self.team, self.orders) = (team, orders);
        self
    }

    /// The plan it follows now: the trigger plan it last turned to, or its first
    pub fn plan(&self) -> &Plan {
        self.active.and_then(|k| self.orders.reactions.get(k)).map_or(&self.orders.plan, |r| &r.plan)
    }

    /// Turn to the first trigger plan set off that has not gone off yet, by its priority, given
    /// what its `team` is sure of and whether a threat is known to be in a zone. Which, if one.
    /// A threat or awareness condition holds while one of the team is at just that level, as
    /// in the original (0x5f8e80): a team that goes straight to Deadly skips Immediate.
    pub fn react(&mut self, team: TeamState, in_zone: impl Fn(u32) -> bool) -> Option<usize> {
        let holds = |c: &Condition| match c {
            Condition::GloballySuspicious => team.awareness[1],
            Condition::GloballyAware => team.awareness[2],
            Condition::SuspiciousThreat => team.threats[Threat::Suspicious as usize],
            Condition::DistantThreat => team.threats[Threat::Distant as usize],
            Condition::ImmediateThreat => team.threats[Threat::Immediate as usize],
            Condition::DeadlyThreat => team.threats[Threat::Deadly as usize],
            // No alarms yet
            Condition::Sound(kind) => kind.eq_ignore_ascii_case("Combat") && self.heard_gunfire,
            Condition::ThreatInZone(zone) => in_zone(*zone),
        };
        let k = (0..self.orders.reactions.len())
            .filter(|&k| !self.fired[k] && self.orders.reactions[k].when.iter().any(holds))
            .min_by_key(|&k| self.orders.reactions[k].priority)?;
        self.fired[k] = true;
        self.active = Some(k);
        // Some rule out the rest of their priority
        if self.orders.reactions[k].only {
            let p = self.orders.reactions[k].priority;
            for (j, r) in self.orders.reactions.iter().enumerate() {
                self.fired[j] |= r.priority == p;
            }
        }
        // A new plan, from its first step
        (self.step, self.loop_at, self.speed, self.covering, self.prey) = (0, None, None, false, None);
        if self.errand.take().is_some() {
            self.path.clear();
        }
        Some(k)
    }

    /// Note a threat, or that one it knows of is still there: the same sign within 50 units
    /// of where it was (0x61e680). With no room for it, the oldest is forgotten.
    pub fn know(&mut self, sign: Sign, who: Option<usize>, at: V3, now: f32) {
        let near = |k: &Known| k.sign == sign && k.who == who && super::dist3(k.at, at) < 50.0;
        if let Some(k) = self.known.iter_mut().find(|k| near(k)) {
            (k.at, k.since) = (at, now);
            return;
        }
        if self.known.len() >= KNOWN {
            let oldest = (0..self.known.len()).min_by(|&a, &b| self.known[a].since.total_cmp(&self.known[b].since)).unwrap();
            self.known.remove(oldest);
        }
        self.known.push(Known { sign, who, at, since: now });
    }

    /// Forget the threats it has not noticed for long, and those that are `gone` (0x61ee60)
    pub fn forget(&mut self, now: f32, gone: impl Fn(usize) -> bool) {
        self.known.retain(|k| {
            let keep = if k.sign == Sign::Quiet { FORGET_QUIET } else { FORGET_THREAT };
            now - k.since <= keep && !k.who.is_some_and(&gone)
        });
    }

    /// How sure it is of a threat, standing at `feet` (0x61e8d0)
    pub fn rate(&self, feet: V3, now: f32) -> Threat {
        self.known.iter().map(|k| match k.sign {
            Sign::Seen if k.since >= now => Threat::Deadly,
            Sign::Seen | Sign::Shot => Threat::Immediate,
            Sign::Heard if super::dist3(k.at, feet) < NEAR_THREAT => Threat::Immediate,
            Sign::Heard => Threat::Distant,
            Sign::Quiet => Threat::Suspicious,
        }).max().unwrap_or_default()
    }

    /// Become as aware as `level`, if more so, and as alert (0x61e530, 0x612fd0): never below
    /// the alertness its mission gave it, and never less than alert once it is
    pub fn aware(&mut self, level: u8, now: f32) {
        let floor = self.start as u8;
        if level > self.awareness && level >= floor {
            self.awareness = level;
        }
        self.aware_at = now;
        if self.alertness != Alertness::Alert && level >= floor {
            self.alertness = [Alertness::Bored, Alertness::Ready, Alertness::Alert][level.min(2) as usize];
        }
    }

    /// Suspicious, with nothing new for a while, it is bored again, though still suspicious
    /// (0x61e4b0)
    pub fn calm(&mut self, now: f32) {
        if self.awareness == 1 && now - self.aware_at > CALM_SECONDS && (self.start as u8) < 1 {
            self.aware(0, now);
        }
    }

    /// The next place its plan's way sends it, past the steps that need no walking: patrolling
    /// zones, the room of them `pick` picks. None when it is to stay where it is: guarding a
    /// spot, or at the end of the way.
    pub fn next_place(&mut self, pick: u64) -> Option<Place> {
        let route = self.plan().route.clone();
        // Round a loop at most once a tick, in case it has nowhere to go
        let mut looped = false;
        loop {
            match route.get(self.step)? {
                Step::Move(at) => return Some(Place::At(*at)),
                Step::AddZone(z) => self.zones.push(*z),
                Step::ClearZones => self.zones.clear(),
                Step::PatrolZones => {
                    let rooms: Vec<u32> = self.zones.iter().flat_map(|z| self.orders.zones.get(z).cloned().unwrap_or_default()).collect();
                    return (!rooms.is_empty()).then(|| Place::Room(rooms[(pick % rooms.len() as u64) as usize]));
                }
                Step::Loop => self.loop_at = Some(self.step),
                Step::EndLoop => match self.loop_at {
                    Some(start) if !looped => {
                        looped = true;
                        self.step = start;
                    }
                    Some(_) => return None,
                    None => {}
                },
                Step::Speed(v) => self.speed = Some(*v),
                Step::Guard => return None,
                // Once there, it stays
                Step::Cover { watch, at } => return (!self.covering).then(|| Place::At(cover_spot(*watch, *at, self.member))),
                Step::Hunt(_) => return Some(Place::Hunt),
                Step::EngageHostages(on) => self.engage_hostages = *on,
            }
            self.step += 1;
        }
    }

    /// How it walks its plan's way: at the plan's speed, or the speed a step set
    pub fn plan_gait(&self) -> (Gait, f32) {
        match (self.speed, self.plan().speed) {
            (Some(v), _) if v < 350.0 => (Gait::Creep, v),
            (Some(v), _) if v < 650.0 => (Gait::Walk, v),
            (Some(v), _) => (Gait::Run, v),
            (None, Speed::Normal) => (Gait::Walk, WALK_SPEED),
            (None, Speed::Blitz) => (Gait::Run, RUN_SPEED),
        }
    }

    /// Whether, and how, its plan sends it to see what it heard: Investigate and Advance go, at
    /// a walk or (Blitz) a run; Cautious creeps; Defend stays put
    pub fn goes_to_see(&self) -> Option<Gait> {
        let plan = self.plan();
        match (plan.roe, plan.speed) {
            (Roe::Investigate | Roe::Advance, Speed::Blitz) => Some(Gait::Run),
            (Roe::Investigate | Roe::Advance, Speed::Normal) => Some(Gait::Walk),
            (Roe::Cautious, _) => Some(Gait::Creep),
            _ => None,
        }
    }

    /// How wide they see as alert as they are: the cone's half-angle, in radians, and how far
    /// (0x5aea40): a bored terrorist sees a narrower cone, but further
    pub fn view(&self) -> (f32, f32) {
        let k = match self.alertness {
            Alertness::Bored => 0.6,
            Alertness::Ready => 0.8,
            Alertness::Alert => 1.0,
        };
        let half = (k * NORMAL_VIEW / 2.0).to_radians();
        (half, self.sight * (NORMAL_VIEW / 2.0).to_radians().tan() / half.tan())
    }

    /// Whether they see a point `to` from their `eye`, facing `yaw` degrees, a target as
    /// `visible` as it is (1 standing still), leaving out whether anything is in the way
    pub fn could_see(&self, eye: V3, yaw: f32, to: V3, visible: f32) -> bool {
        let d = [to[0] - eye[0], to[1] - eye[1], to[2] - eye[2]];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if dist <= CLOSE {
            return true;
        }
        let (half, range) = self.view();
        let facing = [yaw.to_radians().sin(), 0.0, yaw.to_radians().cos()];
        let along = (d[0] * facing[0] + d[2] * facing[2]) / dist.max(1e-6);
        along >= half.cos() && dist <= range * visible
    }

    /// Whether they pull the trigger now, `dist` from the target (0x601610): when the spread
    /// there is at most `15 - 7s` units across, `s` their Self-control, or it has nearly closed,
    /// and never over 180. Not before the spread has opened at all.
    pub fn would_fire(&self, dist: f32) -> bool {
        if !self.aim.started {
            return false;
        }
        let across = self.aim.angle * dist;
        let time = self.gun.close * 0.5;
        let left = 1.0 - (self.aim.since / time).min(1.0);
        let s = self.self_control;
        (across <= 15.0 - 7.0 * s || left * left <= 0.15 - 0.14 * s) && across <= 180.0
    }

    /// Swing their aim toward `want`, a unit vector from the eye, `dt` on, their body facing
    /// along `facing` (0x5a83e0). The aim starts where the body faces and swings only while the
    /// target is within `AIM_CONE` of it, ever faster: up to `(1.3 - s) * 10 pi` radians a second,
    /// `s` their Assault skill, over half the gun's close time (0x5a9160). Whether it is on them.
    pub fn swing_aim(&mut self, facing: V3, want: V3, dt: f32) -> bool {
        let dir = *self.aim_dir.get_or_insert(facing);
        let dot = |a: V3, b: V3| (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0);
        // Which way across the floor: a player close below is still ahead
        let flat = (want[0] * want[0] + want[2] * want[2]).sqrt().max(1e-6);
        if dot(facing, [want[0] / flat, 0.0, want[2] / flat]) < AIM_CONE.to_radians().cos() {
            return false;
        }
        let ramp = (self.gun.close * 0.5).max(1e-3);
        self.swing = (self.swing + dt).min(ramp);
        let step = self.swing / ramp * (1.3 - self.skill) * 10.0 * std::f32::consts::PI * dt;
        let angle = dot(dir, want).acos();
        if angle <= step.max(1e-4) {
            self.aim_dir = Some(want);
            return true;
        }
        // Along the great circle from where it points toward them
        let t = step / angle;
        let (a, b) = (((1.0 - t) * angle).sin() / angle.sin(), (t * angle).sin() / angle.sin());
        self.aim_dir = Some([0, 1, 2].map(|k| dir[k] * a + want[k] * b));
        false
    }

    /// Their spread, `dt` on, holding the trigger or not, facing `yaw` degrees
    pub fn step_aim(&mut self, crouched: bool, wounded: bool, trigger: bool, yaw: f32, dt: f32) {
        let gun = self.gun;
        self.aim.step(&gun, self.skill, Pace::Stopped, crouched, wounded, trigger, super::quantize(yaw), dt);
    }
}

impl super::Sim {
    /// Each armed terrorist hears the `noises` that reach it (where, how far each carries, and
    /// whether through walls),
    /// sees the players it can, turns toward the one it is after, and fires when its spread lets
    /// it; it stops to flinch and reloads when its magazine is empty. Hearing something makes it
    /// alert, and it turns toward it with its weapon up.
    pub(super) fn terrorists(&mut self, dt: f32, noises: &[(V3, f32, Noise)], ev: &mut Vec<super::Event>) {
        // Where they can walk, worked out the first time any of them is about
        if self.nav.is_none() && self.npcs.iter().any(|n| n.mind.is_some()) {
            // From where they stand, where players come in, and where they are
            let feet = self.npcs.iter().filter(|n| n.mind.is_some()).map(|n| [n.pos[0], n.floor, n.pos[2]])
                .chain(self.spawns.iter().copied())
                .chain(self.players.iter().filter(|p| p.active).map(|p| p.player.pos));
            let seeds: Vec<(V3, Vec<u32>)> = feet.map(|f| (f, self.rooms_at(f, None))).collect();
            self.nav = Some(super::nav::Grid::new(&self.world, &self.level, &self.visibility, &seeds));
        }
        let noise_rooms: Vec<Vec<u32>> = noises.iter().map(|&(at, _, _)| self.visibility.player_rooms(&self.level, at, None)).collect();
        let now = self.time;
        let params = self.world.params();
        let (stand_eye, kneel_eye) = (params.eye_height, params.crouch_eye_height);
        let player_rooms: Vec<Vec<u32>> = self.players.iter().map(|p| self.rooms_at(p.player.pos, p.current_room)).collect();
        // What each team is sure of, and anyone, sets off their trigger plans
        let mut teams: std::collections::HashMap<usize, TeamState> = std::collections::HashMap::new();
        for m in self.npcs.iter().filter(|n| !n.health.is_dead()).filter_map(|n| n.mind.as_ref()) {
            let t = teams.entry(m.team).or_default();
            t.threats[m.threat as usize] = true;
            t.awareness[m.awareness.min(2) as usize] = true;
        }
        // Where threats are known to be
        let mut threat_rooms: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for m in self.npcs.iter().filter(|n| !n.health.is_dead()).filter_map(|n| n.mind.as_ref()) {
            for k in &m.known {
                threat_rooms.extend(self.visibility.player_rooms(&self.level, k.at, None));
            }
        }
        for (i, n) in self.npcs.iter_mut().enumerate().filter(|(_, n)| !n.health.is_dead()) {
            let Some(m) = n.mind.as_mut() else { continue };
            let zones = m.orders.clone();
            let in_zone = |z: u32| zones.zones.get(&z).is_some_and(|rooms| rooms.iter().any(|r| threat_rooms.contains(r)));
            if let Some(k) = m.react(teams.get(&m.team).copied().unwrap_or_default(), in_zone) {
                let r = &m.orders.reactions[k];
                ev.push(super::Event::Log(format!("t={now:.2} character {i} turns to \"{}\": {:?} {:?}", r.name, r.plan.roe, r.plan.speed)));
            }
        }
        let mut searches = SEARCHES_PER_TICK;
        // How aware each makes its team, which it passes on when the tick is done
        let mut spread: Vec<(usize, u8)> = Vec::new();
        for i in 0..self.npcs.len() {
            let npc = &self.npcs[i];
            let Some(mut mind) = npc.mind.clone() else { continue };
            if npc.health.is_dead() {
                self.npcs[i].aiming = false;
                continue;
            }
            let kneel = kneels(npc.stance);
            let eye = [npc.pos[0], npc.floor + if kneel { kneel_eye } else { stand_eye }, npc.pos[2]];
            let rooms = self.rooms_at([npc.pos[0], npc.floor, npc.pos[2]], None);

            // The nearest noise it hears: in reach, and through walls only half as far, faintly.
            // Not its own.
            let hears = noises.iter().zip(&noise_rooms).filter_map(|(&(at, reach, noise), rooms_there)| {
                let d = super::dist3(eye, at);
                let across = ((at[0] - eye[0]).powi(2) + (at[2] - eye[2]).powi(2)).sqrt();
                if across <= 60.0 || d > reach {
                    return None;
                }
                let muffled = noise == Noise::Sound && d <= reach * MUFFLED;
                // A quiet sound means as much heard faintly or not
                let clear = !(muffled && reach < LOUD) && self.line_clear(eye, &rooms, at, rooms_there);
                (clear || muffled).then_some((at, d, reach, noise, !clear))
            }).min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((at, _, reach, noise, faint)) = hears {
                mind.heard = Some((at, now));
                let loud = reach >= LOUD;
                // What it makes of it: footsteps (0x6143e0), gunfire or a blast (0x6142f0), or a
                // quiet sound, as a door is (0x614640)
                let (aware, sign) = match noise {
                    Noise::Footstep => (1, Sign::Heard),
                    Noise::Sound if loud => (if faint { 1 } else { 2 }, Sign::Heard),
                    Noise::Sound => (1, Sign::Quiet),
                };
                mind.aware(aware, now);
                spread.push((mind.team, aware));
                mind.know(sign, None, at, now);
                mind.heard_gunfire |= loud;
                // With no one to fight, it goes to see if its plan says to: unless it is on its
                // way there already. Otherwise it stays put, its weapon up toward it.
                let going = mind.path.last().map(|w| w.0).or(mind.seek.map(|s| s.0)).is_some_and(|end| super::dist3(end, at) < 200.0);
                // Nor when it looked for a way just now, or toward there not long ago
                let asked = mind.asked.is_some_and(|(there, t)| {
                    t >= self.doors_moved && (now - t < ASK_AGAIN || (now - t < ASK_SAME && super::dist3(there, at) < SAME_PLACE))
                });
                if let (None, false, false, Some(gait)) = (mind.target, going, asked, mind.goes_to_see()) {
                    mind.asked = Some((at, now));
                    mind.seek = Some((at, gait));
                }
            }
            // Its turn to look for the way there
            if let Some((at, gait)) = mind.seek.filter(|_| mind.target.is_none() && mind.spent.is_none() && searches > 0) {
                searches -= 1;
                mind.seek = None;
                mind.gait = gait;
                // Off its plan's way for now: it comes back to the step it left
                mind.errand = None;
                mind.speed = None;
                let feet = [npc.pos[0], npc.floor, npc.pos[2]];
                let there = self.visibility.player_rooms(&self.level, at, None);
                mind.path = self.nav.as_ref().and_then(|g| g.path(&self.world, feet, &rooms, at, &there, &self.blocked_portals)).unwrap_or_default();
                if !mind.path.is_empty() {
                    ev.push(super::Event::Log(format!("t={now:.2} character {i} goes to see at ({:.0}, {:.0}, {:.0}), {} legs, {gait:?}", at[0], at[1], at[2], mind.path.len())));
                }
            }

            // Its own weapon empty: it draws its sidearm, or with none it gives up the fight
            let mut draw = None;
            if mind.ammo == 0 && mind.magazines == 0 && mind.reload_until.is_none() && mind.spent.is_none() {
                if let Some((weapon, gun, magazine, magazines)) = mind.sidearm.take() {
                    // Empty, so that drawing it ends with loading its first magazine
                    (mind.gun, mind.magazine, mind.magazines, mind.ammo) = (gun, magazine, magazines, 0);
                    mind.reload_until = Some(now + DRAW_SECONDS);
                    mind.reload_for = DRAW_SECONDS;
                    mind.aim = Aim::default();
                    draw = Some(weapon);
                    ev.push(super::Event::Log(format!("t={now:.2} character {i} draws its sidearm")));
                } else {
                    mind.spent = Some(Spent::Out);
                    mind.path.clear();
                    mind.seek = None;
                    mind.errand = None;
                    ev.push(super::Event::Log(format!("t={now:.2} character {i} has nothing left to fire")));
                }
            }

            // The players it sees: where it would aim at each, and how far that is
            let (_, range) = mind.view();
            let mut sees: Vec<(usize, V3, f32)> = Vec::new();
            let mut glimpsed = false;
            for (slot, p) in self.players.iter().enumerate() {
                if !p.active || !p.controlled || p.respawn_at.is_some() {
                    continue;
                }
                let feet = p.player.pos;
                let height = self.world.params().body_height(p.player.crouched);
                let at = |up: f32| [feet[0], feet[1] + up * height, feet[2]];
                // Its head if it shoots well, else the middle of its body (0x5a93e0)
                let (head, body) = (at(0.94), at(0.65));
                let aim_at = if mind.skill > 0.74 { head } else { body };
                let visible = if p.player.crouched { 0.7 } else { 1.0 } * if p.aim.pace == Pace::Stopped { 1.0 } else { 1.2 };
                let dist = super::dist3(eye, body);
                // The one it is after it keeps sight of whichever way it faces, out to a glimpse
                let looks = if mind.target == Some(slot) { dist <= CLOSE.max(range * visible * 1.75) } else { mind.could_see(eye, npc.yaw, body, visible) };
                // Only the rooms around the players are solid, so the line goes room to room
                let clear = |to: V3| self.line_clear(eye, &rooms, to, &player_rooms[slot]);
                if looks && (clear(body) || clear(head)) {
                    sees.push((slot, aim_at, dist));
                } else if !looks && mind.awareness < 2 && !glimpsed && mind.could_see(eye, npc.yaw, body, visible * 1.75) {
                    // Out to 1.75 times as far it glimpses them, which makes it suspicious
                    // (0x6142d0)
                    glimpsed = clear(body) || clear(head);
                }
            }
            // A dead terrorist it sees makes it aware (0x614240)
            let body_seen = mind.awareness < 2 && self.npcs.iter().enumerate().any(|(k, o)| {
                let body = [o.pos[0], o.floor + 20.0, o.pos[2]];
                k != i && o.mind.is_some() && o.health.is_dead() && mind.could_see(eye, npc.yaw, body, 1.0)
                    && self.line_clear(eye, &rooms, body, &self.rooms_at([o.pos[0], o.floor, o.pos[2]], None))
            });
            for (level, so) in [(1, glimpsed), (2, body_seen)] {
                if so {
                    mind.aware(level, now);
                    spread.push((mind.team, level));
                }
            }
            // The one it is after while it sees them, else the nearest; none when it cannot fight
            let seen = sees.iter().find(|s| mind.target == Some(s.0)).or_else(|| sees.iter().min_by(|a, b| a.2.total_cmp(&b.2))).copied();
            let threat_at = seen.map(|s| s.1).or(mind.target.and_then(|t| self.players.get(t)).map(|p| p.player.pos)).or(mind.heard.map(|h| h.0));
            let seen = seen.filter(|_| mind.spent.is_none());
            if mind.spent.is_some() {
                mind.target = None;
            }
            // Who its plan has it hunt, where they are, and whether it sees them to fire on,
            // which it does before anything else (0x603000)
            let hunt_step = mind.step;
            let hunted = match mind.plan().route.get(mind.step) {
                Some(Step::Hunt(name)) => self.npcs.iter().position(|o| o.name == *name && !o.health.is_dead())
                    .map(|v| (v, [self.npcs[v].pos[0], self.npcs[v].floor, self.npcs[v].pos[2]])),
                _ => None,
            };
            let prey = hunted.filter(|_| mind.engage_hostages && mind.spent.is_none()).and_then(|(v, feet)| {
                let body = [feet[0], feet[1] + 0.65 * self.world.params().body_height(kneels(self.npcs[v].stance)), feet[2]];
                let seen = mind.could_see(eye, npc.yaw, body, 1.0) && self.line_clear(eye, &rooms, body, &self.rooms_at(feet, None));
                seen.then(|| (v, body, super::dist3(eye, body)))
            });
            if let Some((v, _, _)) = prey {
                mind.path.clear();
                if mind.prey != Some(v) {
                    mind.aim = Aim::default();
                    mind.aim_dir = None;
                    mind.swing = 0.0;
                }
            }
            mind.prey = prey.map(|p| p.0);
            // How many of its team are left, to spread out in cover
            let team_size = if mind.covering {
                self.npcs.iter().filter(|o| !o.health.is_dead() && o.mind.as_ref().is_some_and(|m| m.team == mind.team)).count()
            } else {
                0
            };
            match seen {
                Some((slot, _, _)) => {
                    // It stops where it is to fight. Seeing them makes it and its team aware
                    // (0x614260).
                    mind.path.clear();
                    mind.know(Sign::Seen, Some(slot), self.players[slot].player.pos, now);
                    if mind.target != Some(slot) {
                        // A new target: its spread opens afresh, and its aim starts again from
                        // where it faces
                        mind.aim = Aim::default();
                        mind.aim_dir = None;
                        mind.swing = 0.0;
                        mind.quick = mind.alertness == Alertness::Alert;
                    }
                    mind.target = Some(slot);
                    mind.seen_at = now;
                    mind.aware(2, now);
                    spread.push((mind.team, 2));
                }
                None => {
                    let gone = mind.target.is_some_and(|t| self.players.get(t).is_none_or(|p| !p.active || p.respawn_at.is_some()));
                    if gone || now - mind.seen_at > FORGET {
                        mind.target = None;
                        mind.aim_dir = None;
                        mind.swing = 0.0;
                    }
                }
            }
            let players = &self.players;
            mind.forget(now, |p| players.get(p).is_none_or(|p| !p.active || p.respawn_at.is_some()));
            mind.calm(now);
            mind.threat = mind.rate([npc.pos[0], npc.floor, npc.pos[2]], now);
            // How long it takes to get its weapon up, from what it was doing
            let frames = |m: &str| self.characters.as_ref().map_or(RAISE_SECONDS * 30.0, |c| c.frames(m));
            let (_, raise_seconds) = raise(self.npcs[i].stance, frames);
            // Someone else stands on the next point of its way
            let width = 2.0 * self.world.params().radius;
            let on = |feet: V3, at: V3| (feet[1] - at[1]).abs() < 100.0 && ((feet[0] - at[0]).powi(2) + (feet[2] - at[2]).powi(2)).sqrt() < width;
            let taken = mind.path.first().is_some_and(|&(next, _)| {
                self.npcs.iter().enumerate().any(|(k, o)| k != i && !o.health.is_dead() && on([o.pos[0], o.floor, o.pos[2]], next))
                    || self.players.iter().any(|p| p.active && p.respawn_at.is_none() && on(p.player.pos, next))
            });
            // The others near enough to stand in its way
            let here = [npc.pos[0], npc.floor, npc.pos[2]];
            let others: Vec<V3> = self.npcs.iter().enumerate().filter(|(k, o)| *k != i && !o.health.is_dead()).map(|(_, o)| [o.pos[0], o.floor, o.pos[2]])
                .chain(self.players.iter().filter(|p| p.active && p.respawn_at.is_none()).map(|p| p.player.pos))
                .filter(|o| (o[1] - here[1]).abs() < 100.0 && ((o[0] - here[0]).powi(2) + (o[2] - here[2]).powi(2)).sqrt() < 4.0 * width)
                .collect();
            let npc = &mut self.npcs[i];
            if let Some(weapon) = draw {
                npc.weapon = Some(weapon);
            }
            npc.reload = mind.reload_until.map(|t| 1.0 - (t - now) / mind.reload_for.max(1e-3));
            // Its weapon up for someone it sees, or toward what it heard
            let listening = mind.heard.filter(|&(_, t)| now - t < HEARD_SECONDS && mind.spent.is_none());
            let aiming = mind.target.is_some() || listening.is_some() || prey.is_some();
            if aiming && !npc.aiming {
                npc.aim_since = now;
            }
            npc.aiming = aiming;
            let raised = now >= npc.aim_since + raise_seconds;
            // With nothing to fire, it runs from the fight, away from where the threat is, and
            // cowers where it gets to
            match mind.spent {
                Some(Spent::Out) if searches > 0 => {
                    searches -= 1;
                    let feet = [npc.pos[0], npc.floor, npc.pos[2]];
                    let threat = threat_at.unwrap_or(feet);
                    mind.path = self.nav.as_ref().and_then(|g| g.away(&self.world, feet, &rooms, threat, FLEE_REACH, &self.blocked_portals)).unwrap_or_default();
                    mind.gait = Gait::Run;
                    mind.spent = Some(if mind.path.is_empty() { Spent::Hiding } else { Spent::Running });
                    ev.push(super::Event::Log(format!("t={now:.2} character {i} runs, {} legs", mind.path.len())));
                }
                Some(Spent::Running) if mind.path.is_empty() => mind.spent = Some(Spent::Hiding),
                Some(Spent::Hiding) => npc.stance = Some(Stance::Cower),
                _ => {}
            }
            // With nothing else to do, it takes its plan's way, a step at a time
            if mind.target.is_none() && listening.is_none() && mind.path.is_empty() && mind.spent.is_none() && now >= mind.pause_until && searches > 0 {
                if mind.errand.take() == Some(mind.step) {
                    match mind.plan().route.get(mind.step) {
                        // Patrolling zones, it stays a while in each room, and goes on to another
                        Some(Step::PatrolZones) => mind.pause_until = now + ZONE_PAUSE,
                        // At cover it stays; hunting, it looks again in a while
                        Some(Step::Cover { .. }) => mind.covering = true,
                        Some(Step::Hunt(_)) => mind.pause_until = now + HUNT_PAUSE,
                        _ => mind.step += 1,
                    }
                }
                let pick = super::next_rng(&mut self.rng);
                let place = if now >= mind.pause_until { mind.next_place(pick) } else { None };
                // Who it hunts, as found for this step: else it looks next tick
                let hunted = hunted.filter(|_| mind.step == hunt_step);
                let to = place.and_then(|p| match p {
                    Place::At(at) => Some(at),
                    Place::Room(room) => self.visibility.center(room),
                    Place::Hunt => hunted.map(|(_, at)| at),
                });
                // Who it hunts is dead: on to the next step (0x603000)
                if place == Some(Place::Hunt) && mind.step == hunt_step && hunted.is_none() {
                    mind.step += 1;
                }
                if let Some(to) = to {
                    searches -= 1;
                    let feet = [npc.pos[0], npc.floor, npc.pos[2]];
                    let there = self.visibility.player_rooms(&self.level, to, None);
                    match self.nav.as_ref().and_then(|g| g.path(&self.world, feet, &rooms, to, &there, &self.blocked_portals)) {
                        Some(path) => {
                            mind.path = path;
                            mind.errand = Some(mind.step);
                            mind.gait = mind.plan_gait().0;
                        }
                        // No way there: on to the next step, or another room in a while. With no
                        // way to its cover it takes cover where it is.
                        None if matches!(place, Some(Place::Room(_) | Place::Hunt)) => mind.pause_until = now + ASK_AGAIN,
                        None if matches!(mind.plan().route.get(mind.step), Some(Step::Cover { .. })) => mind.covering = true,
                        None => mind.step += 1,
                    }
                }
            }
            if mind.reload_until.is_some_and(|t| now >= t) {
                mind.ammo = mind.magazine;
                mind.magazines -= 1;
                mind.reload_until = None;
            }
            // A flinch stops it
            if !matches!(npc.health.state, crate::combat::NpcState::Idle) {
                npc.mind = Some(mind);
                continue;
            }
            // Without a target, walk its way to where it heard something, then turn toward it
            let mut at_door = false;
            if seen.is_none() && !mind.path.is_empty() {
                let (next, doorway) = mind.path[0];
                let (dx, dz) = (next[0] - npc.pos[0], next[2] - npc.pos[2]);
                let left = (dx * dx + dz * dz).sqrt();
                // A door on the way it opens, and waits for, as the original's OpenDoor step does
                // (0x5fed60)
                let door = doorway.and_then(|p| self.doors.iter().position(|d| d.portal == Some(p as usize)));
                if let Some(k) = door.filter(|&k| self.door_states[k].open < 1.0) {
                    at_door = true;
                    if !self.door_states[k].opening {
                        self.door_states[k].opening = true;
                        let d = &self.doors[k];
                        if !d.open_sound.eq_ignore_ascii_case("none") {
                            ev.push(super::Event::Sound { name: d.open_sound.clone(), at: Some(d.center(self.door_states[k].open)), gain: 1.0 });
                        }
                        ev.push(super::Event::Log(format!("t={now:.2} character {i} opens door {}", d.name)));
                    }
                    npc.yaw = turn_toward(npc.yaw, dx.atan2(dz).to_degrees(), TURN_RATE * dt);
                } else if left < ARRIVE || (taken && left < width + ARRIVE) {
                    mind.path.remove(0);
                } else {
                    npc.yaw = turn_toward(npc.yaw, dx.atan2(dz).to_degrees(), TURN_RATE * dt);
                    let speed = match (mind.errand, mind.gait) {
                        (Some(_), _) => mind.plan_gait().1,
                        (None, Gait::Creep) => CREEP_SPEED,
                        (None, Gait::Walk) => WALK_SPEED,
                        (None, Gait::Run) => RUN_SPEED,
                    };
                    let step = (speed * dt).min(left);
                    let (x, z) = (npc.pos[0] + dx / left * step, npc.pos[2] + dz / left * step);
                    // Not into someone else: closer than a body's width, and nearer than it was
                    let gap = |o: &V3, x: f32, z: f32| ((o[0] - x).powi(2) + (o[2] - z).powi(2)).sqrt();
                    let blocked = others.iter().any(|o| gap(o, x, z) < width && gap(o, x, z) < gap(o, npc.pos[0], npc.pos[2]));
                    if blocked {
                        // At the end of its way, or held up a while, it goes on without that point
                        let since = *mind.blocked_since.get_or_insert(now);
                        if mind.path.len() == 1 || now - since > BLOCKED_SECONDS {
                            mind.path.remove(0);
                            mind.blocked_since = None;
                        }
                    } else {
                        mind.blocked_since = None;
                        npc.pos[0] = x;
                        npc.pos[2] = z;
                        let floor = self.world.ground_at(npc.pos[0], npc.pos[2], npc.floor + STEP).unwrap_or(npc.floor);
                        npc.pos[1] += floor - npc.floor;
                        npc.floor = floor;
                    }
                    // On its feet to walk
                    npc.stance = Some(Stance::Stand);
                }
            } else if let (None, Some((at, _))) = (seen, listening) {
                if ((at[0] - eye[0]).powi(2) + (at[2] - eye[2]).powi(2)).sqrt() > 60.0 {
                    let want = (at[0] - eye[0]).atan2(at[2] - eye[2]).to_degrees();
                    npc.yaw = turn_toward(npc.yaw, want, TURN_RATE * dt);
                }
            } else if let (None, Some(Step::Cover { watch, at })) = (seen, mind.plan().route.get(mind.step).filter(|_| mind.covering)) {
                // In cover, it faces what it watches
                npc.yaw = turn_toward(npc.yaw, cover_yaw(*watch, *at, mind.member, team_size), TURN_RATE * dt);
            }
            npc.walking = seen.is_none() && !mind.path.is_empty() && !at_door;
            npc.gait = mind.gait;
            // Turn toward the target, swing the aim onto them, and fire when the spread lets it
            let mut fire = None;
            if let Some((to, dist)) = prey.map(|p| (p.1, p.2)).or(seen.map(|s| (s.1, s.2))) {
                let want = (to[0] - eye[0]).atan2(to[2] - eye[2]).to_degrees();
                npc.yaw = turn_toward(npc.yaw, want, if mind.quick { ALERT_TURN_RATE } else { TURN_RATE } * dt);
                let facing = [npc.yaw.to_radians().sin(), 0.0, npc.yaw.to_radians().cos()];
                let d = super::dist3(eye, to).max(1e-3);
                let on_target = mind.swing_aim(facing, [0, 1, 2].map(|k| (to[k] - eye[k]) / d), dt);
                let ready = raised && mind.ammo > 0 && mind.reload_until.is_none();
                fire = (ready && on_target && mind.would_fire(dist)).then_some(to);
            }
            let wounded = npc.health.hp < crate::combat::Health::new().hp;
            mind.step_aim(kneel, wounded, fire.is_some(), npc.yaw, dt);
            let shot = fire.filter(|_| now >= mind.next_shot).map(|to| {
                let weapon = npc.weapon.and_then(|w| self.characters.as_ref()?.weapons.get(w)).map(|(_, w)| w);
                mind.ammo -= 1;
                mind.next_shot = now + weapon.map_or(0.1, |w| w.interval());
                // As long as its weapon's reload motion, else as long as a player's reload
                if mind.ammo == 0 && mind.magazines > 0 {
                    mind.reload_for = self.characters.as_ref().and_then(|c| c.reload_seconds(npc.weapon, kneel)).unwrap_or(super::RELOAD_SECONDS);
                    mind.reload_until = Some(now + mind.reload_for);
                }
                npc.last_shot = now;
                (to, weapon.and_then(|w| w.sound.clone()), weapon.map_or((1, 0.0), |w| (w.pellets.max(1), w.pellet_spread)))
            });
            let npc_pos = npc.pos;
            if let Some((to, sound, (pellets, spread))) = shot {
                let mut u = [0.0; 3];
                for v in &mut u {
                    *v = (super::next_rng(&mut self.rng) >> 40) as f32 / (1u64 << 24) as f32;
                }
                let (off, round) = mind.aim.throw(u);
                mind.aim.fired();
                // Along their aim, which is on the target
                let d = super::dist3(eye, to).max(1e-3);
                let aimed = mind.aim_dir.unwrap_or([0, 1, 2].map(|k| (to[k] - eye[k]) / d));
                let dir = super::deflect(aimed, off, round);
                if let Some(name) = sound {
                    ev.push(super::Event::Sound { name, at: Some(npc_pos), gain: 1.0 });
                }
                let at = match (mind.prey, mind.target) {
                    (Some(v), _) => format!("character {v}"),
                    (None, t) => format!("player {}", t.map_or("?".to_string(), |t| t.to_string())),
                };
                ev.push(super::Event::Log(format!("t={now:.2} character {i} fires at {at} from ({:.0}, {:.0}, {:.0}) along ({:.2}, {:.2}, {:.2})",
                    eye[0], eye[1], eye[2], dir[0], dir[1], dir[2])));
                self.npcs[i].mind = Some(mind);
                // A shotgun fires its pellets round the shot
                for k in 0..pellets {
                    let way = if pellets > 1 {
                        let mut u = [0.0; 2];
                        for v in &mut u {
                            *v = (super::next_rng(&mut self.rng) >> 40) as f32 / (1u64 << 24) as f32;
                        }
                        pellet(dir, k, spread, u)
                    } else {
                        dir
                    };
                    self.bullet(eye, way, super::Shooter::Character(i), ev);
                }
            } else {
                self.npcs[i].mind = Some(mind);
            }
        }
        for (team, level) in spread {
            self.make_aware(team, level);
        }
        self.keep_apart(dt);
    }

    /// Makes a team as aware as `level`, as one of it passes on what it knows (0x61e530)
    pub(super) fn make_aware(&mut self, team: usize, level: u8) {
        let now = self.time;
        for m in self.npcs.iter_mut().filter(|n| !n.health.is_dead()).filter_map(|n| n.mind.as_mut()).filter(|m| m.team == team) {
            m.aware(level, now);
        }
    }

    /// Pushes apart terrorists that stand closer than two bodies' width. Teammates walk the
    /// same way, and everyone sent to see a noise walks to the same point. Players and other
    /// characters stay put and push them away too. A push that would take a terrorist into a
    /// wall or off its floor does not happen.
    fn keep_apart(&mut self, dt: f32) {
        let width = 2.0 * self.world.params().radius;
        // Every body: where its feet are, and whether it can be pushed
        let mut bodies: Vec<(V3, bool)> = self.npcs.iter().filter(|n| !n.health.is_dead())
            .map(|n| ([n.pos[0], n.floor, n.pos[2]], n.mind.is_some())).collect();
        let movers: Vec<usize> = self.npcs.iter().enumerate().filter(|(_, n)| !n.health.is_dead()).map(|(i, _)| i).collect();
        bodies.extend(self.players.iter().filter(|p| p.active && p.respawn_at.is_none()).map(|p| (p.player.pos, false)));
        let mut push = vec![[0.0f32; 2]; movers.len()];
        for a in 0..bodies.len() {
            for b in a + 1..bodies.len() {
                let ((p, a_moves), (q, b_moves)) = (bodies[a], bodies[b]);
                if (p[1] - q[1]).abs() > 100.0 || !(a_moves || b_moves) {
                    continue;
                }
                let (dx, dz) = (p[0] - q[0], p[2] - q[2]);
                let d = (dx * dx + dz * dz).sqrt();
                if d >= width {
                    continue;
                }
                // Right on top of each other, apart a way of their own
                let (ux, uz) = if d > 1e-3 { (dx / d, dz / d) } else { let t = a as f32 * 2.4; (t.cos(), t.sin()) };
                let share = if a_moves && b_moves { 0.5 } else { 1.0 };
                let by = ((width - d) * share).min(PUSH_SPEED * dt);
                if a_moves {
                    push[a][0] += ux * by;
                    push[a][1] += uz * by;
                }
                if b_moves {
                    push[b][0] -= ux * by;
                    push[b][1] -= uz * by;
                }
            }
        }
        for (k, &i) in movers.iter().enumerate() {
            let [px, pz] = push[k];
            if px == 0.0 && pz == 0.0 {
                continue;
            }
            let n = &self.npcs[i];
            let feet = [n.pos[0] + px, n.floor, n.pos[2] + pz];
            let Some(floor) = self.world.ground_at(feet[0], feet[2], n.floor + STEP).filter(|f| (f - n.floor).abs() <= STEP) else { continue };
            // Not into a wall, nor nearer one it is already in, among the rooms of either spot
            let (from, to) = ([n.pos[0], n.floor, n.pos[2]], [feet[0], floor, feet[2]]);
            let mut rooms = self.rooms_at(from, None);
            rooms.extend(self.rooms_at(to, None));
            let clearance = |at: V3| rooms.iter().map(|&r| self.world.wall_clearance_in(&crate::collision::Player::new(at), r)).fold(f32::INFINITY, f32::min);
            if clearance(to) < clearance(from).min(0.0) {
                continue;
            }
            let n = &mut self.npcs[i];
            n.pos[0] = to[0];
            n.pos[2] = to[2];
            n.pos[1] += floor - n.floor;
            n.floor = floor;
        }
    }
}

impl super::Sim {
    /// A line about each terrorist for the debug dump: where it is and faces, what it knows and
    /// is doing, and whether it could see player `slot` now
    pub fn terrorist_report(&self, slot: usize) -> Vec<String> {
        let now = self.time;
        let params = self.world.params();
        let player = self.players.get(slot).filter(|p| p.active);
        let theirs = player.map(|p| self.rooms_at(p.player.pos, p.current_room)).unwrap_or_default();
        self.npcs.iter().enumerate().filter_map(|(i, n)| {
            let m = n.mind.as_ref()?;
            let feet = [n.pos[0], n.floor, n.pos[2]];
            let rooms = self.rooms_at(feet, None);
            let eye = [n.pos[0], n.floor + if kneels(n.stance) { params.crouch_eye_height } else { params.eye_height }, n.pos[2]];
            let mut t = format!("#{i} team {}: ", m.team);
            if n.health.is_dead() {
                t += &format!("dead at ({:.0}, {:.0}, {:.0})", feet[0], feet[1], feet[2]);
                return Some(t);
            }
            let (s, c) = n.yaw.to_radians().sin_cos();
            t += &format!("at ({:.0}, {:.0}, {:.0}) rooms {rooms:?}, yaw {:.0} facing ({s:.2}, 0, {c:.2}), {:?}, hp {:.0} {:?}",
                feet[0], feet[1], feet[2], n.yaw, n.stance, n.health.hp, n.health.state);
            if let Some(p) = player {
                let body = [p.player.pos[0], p.player.pos[1] + 0.65 * params.body_height(p.player.crouched), p.player.pos[2]];
                let head = [p.player.pos[0], p.player.pos[1] + 0.94 * params.body_height(p.player.crouched), p.player.pos[2]];
                let clear = |to| self.line_clear(eye, &rooms, to, &theirs);
                t += &format!("\n    you: {:.0} units away, in its view {}, clear line to body {} head {}",
                    super::dist3(eye, body), m.could_see(eye, n.yaw, body, 1.0), clear(body), clear(head));
            }
            let active = m.active.and_then(|k| m.orders.reactions.get(k)).map_or("its plan", |r| r.name.as_str());
            t += &format!("\n    {:?}, awareness {}, threat {:?}, following {active} ({:?}, {:?}), step {}",
                m.alertness, m.awareness, m.threat, m.plan().roe, m.plan().speed, m.step);
            for k in &m.known {
                t += &format!("\n    knows of {:?} {:?} at ({:.0}, {:.0}, {:.0}) {:.1}s ago", k.sign, k.who, k.at[0], k.at[1], k.at[2], now - k.since);
            }
            t += &format!("\n    target {:?}, last seen {:.1}s ago, aiming {}, ammo {}/{} and {} magazines{}",
                m.target, now - m.seen_at, n.aiming, m.ammo, m.magazine, m.magazines, if m.reload_until.is_some() { ", reloading" } else { "" });
            if m.sidearm.is_some() {
                t += ", and a sidearm";
            }
            if let Some(spent) = m.spent {
                t += &format!(", nothing left to fire ({spent:?})");
            }
            if let Some((at, when)) = m.heard {
                t += &format!("\n    heard something at ({:.0}, {:.0}, {:.0}) {:.1}s ago", at[0], at[1], at[2], now - when);
            }
            if let Some(&(next, door)) = m.path.first() {
                let end = m.path.last().map_or(next, |w| w.0);
                t += &format!("\n    walking ({:?}) {} points to ({:.0}, {:.0}, {:.0}), next ({:.0}, {:.0}, {:.0}){}",
                    m.gait, m.path.len(), end[0], end[1], end[2], next[0], next[1], next[2],
                    door.map_or(String::new(), |p| format!(" through doorway {}", self.level.portals.portals[p as usize].name)));
            }
            Some(t)
        }).collect()
    }
}

/// How a terrorist waiting in `stance` raises its weapon to aim: with the original's motion for
/// standing up with it, from leaning or slouching, or else easing into aiming over
/// `RAISE_SECONDS`; and how long that takes, by `frames` of each motion
pub fn raise(stance: Option<Stance>, frames: impl Fn(&str) -> f32) -> (Option<&'static str>, f32) {
    let motion = match stance {
        Some(Stance::Lean) => Some(RAISE_MOTIONS[0]),
        Some(Stance::Slouch) => Some(RAISE_MOTIONS[1]),
        _ => None,
    };
    (motion, motion.map_or(RAISE_SECONDS, |m| frames(m) / 30.0))
}

/// Whether a stance aims kneeling
pub fn kneels(stance: Option<Stance>) -> bool {
    matches!(stance, Some(Stance::Squat | Stance::Kneel))
}

/// Turn `yaw` toward `want` (degrees) by at most `step`, the long way never
pub fn turn_toward(yaw: f32, want: f32, step: f32) -> f32 {
    let off = (want - yaw + 540.0).rem_euclid(360.0) - 180.0;
    (yaw + off.clamp(-step, step)).rem_euclid(360.0)
}

/// Whether `yaw` is within `AIM_CONE` of `want`, both in degrees
pub fn facing(yaw: f32, want: f32) -> bool {
    ((want - yaw + 540.0).rem_euclid(360.0) - 180.0).abs() <= AIM_CONE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::aim::MP5K;

    fn mind(alertness: Alertness) -> Mind {
        Mind::new(alertness, 0.25, 0.3, 4000.0, MP5K, 30, 5)
    }

    #[test]
    fn a_bored_terrorist_sees_a_narrower_cone_but_further() {
        let (bored, alert) = (mind(Alertness::Bored).view(), mind(Alertness::Alert).view());
        assert!(bored.0 < alert.0 && bored.1 > alert.1, "{bored:?} {alert:?}");
        assert!((alert.1 - 4000.0).abs() < 1.0);
        // Ahead within range, but not behind unless close
        let m = mind(Alertness::Alert);
        assert!(m.could_see([0.0; 3], 0.0, [0.0, 0.0, 3000.0], 1.0));
        assert!(!m.could_see([0.0; 3], 0.0, [0.0, 0.0, -3000.0], 1.0));
        assert!(m.could_see([0.0; 3], 0.0, [0.0, 0.0, -150.0], 1.0));
        // Crouched players are seen less far
        assert!(!m.could_see([0.0; 3], 0.0, [0.0, 0.0, 3000.0], 0.7));
    }

    #[test]
    fn they_hold_fire_until_the_spread_closes_on_the_target() {
        let mut m = mind(Alertness::Alert);
        m.step_aim(false, false, false, 0.0, 1.0 / 60.0);
        assert!(!m.would_fire(1000.0));
        for _ in 0..60 {
            m.step_aim(false, false, false, 0.0, 1.0 / 60.0);
        }
        assert!(m.would_fire(1000.0));
        // Not at all at the far end of a hall, the spread being too wide there
        assert!(!m.would_fire(40000.0));
    }

    /// A terrorist in the box room at `at`, facing `yaw`, and a player standing still across it
    #[test]
    fn a_threat_is_deadly_while_seen_and_fades_as_the_original_forgets_it() {
        let mut m = mind(Alertness::Bored);
        let feet = [0.0, 0.0, 0.0];
        m.know(Sign::Seen, Some(0), [500.0, 0.0, 0.0], 1.0);
        assert_eq!(m.rate(feet, 1.0), Threat::Deadly);
        // Seen a while ago: Immediate, and forgotten after 40 seconds
        assert_eq!(m.rate(feet, 2.0), Threat::Immediate);
        m.forget(41.0, |_| false);
        assert_eq!(m.rate(feet, 41.0), Threat::Immediate);
        m.forget(41.5, |_| false);
        assert_eq!(m.rate(feet, 41.5), Threat::None);
        // Once the player is gone it is forgotten at once
        m.know(Sign::Seen, Some(0), [500.0, 0.0, 0.0], 50.0);
        m.forget(50.1, |p| p == 0);
        assert!(m.known.is_empty());
        // Heard: Immediate within 2000 units of where it stands, else Distant
        m.know(Sign::Heard, None, [2500.0, 0.0, 0.0], 60.0);
        assert_eq!(m.rate(feet, 60.0), Threat::Distant);
        assert_eq!(m.rate([1000.0, 0.0, 0.0], 60.0), Threat::Immediate);
        // A quiet one is Suspicious, and forgotten after 5 seconds
        let mut m = mind(Alertness::Bored);
        m.know(Sign::Quiet, None, [100.0, 0.0, 0.0], 0.0);
        assert_eq!(m.rate(feet, 0.0), Threat::Suspicious);
        m.forget(5.5, |_| false);
        assert_eq!(m.rate(feet, 5.5), Threat::None);
        // The same threat a little further on is the same one; it keeps five at most
        for k in 0..8 {
            m.know(Sign::Heard, None, [k as f32 * 30.0, 0.0, 0.0], k as f32);
        }
        m.know(Sign::Seen, Some(1), [0.0, 0.0, 900.0], 9.0);
        assert_eq!(m.known.len(), 2);
        for k in 0..6 {
            m.know(Sign::Heard, None, [k as f32 * 1000.0, 0.0, 5000.0], 10.0 + k as f32);
        }
        assert_eq!(m.known.len(), 5);
        assert!(m.known.iter().all(|k| k.since >= 11.0));
    }

    #[test]
    fn awareness_spreads_through_a_team_and_a_suspicious_one_calms_down() {
        let mut sim = standoff([0.0, 0.0, 300.0], 0.0);
        let mate = sim.npcs[0].clone();
        let other = crate::sim::characters::Npc { mind: mate.mind.clone().map(|m| m.with_orders(1, Rc::new(Orders::default()))), ..mate.clone() };
        sim.npcs.push(mate);
        sim.npcs.push(other);
        sim.make_aware(0, 1);
        let state = |sim: &crate::sim::Sim, k: usize| sim.npcs[k].mind.as_ref().map(|m| (m.awareness, m.alertness)).unwrap();
        assert_eq!([state(&sim, 0), state(&sim, 1), state(&sim, 2)], [(1, Alertness::Ready), (1, Alertness::Ready), (0, Alertness::Bored)]);
        // With nothing new for 30 seconds, it is bored again, though still suspicious
        let m = sim.npcs[0].mind.as_mut().unwrap();
        m.calm(20.0);
        assert_eq!(m.alertness, Alertness::Ready);
        m.calm(31.0);
        assert_eq!((m.awareness, m.alertness), (1, Alertness::Bored));
        // Aware, it is alert for good
        m.aware(2, 40.0);
        m.calm(100.0);
        assert_eq!((m.awareness, m.alertness), (2, Alertness::Alert));
        // One the mission makes ready never falls below it
        let mut m = mind(Alertness::Ready);
        m.aware(1, 0.0);
        m.calm(100.0);
        assert_eq!(m.alertness, Alertness::Ready);
    }

    #[test]
    fn a_threat_trigger_goes_off_only_at_its_own_level() {
        let when = |c: Condition| Reaction { name: "go".into(), priority: 0, only: false, when: vec![c], plan: Plan::default() };
        let mut m = mind(Alertness::Bored).with_orders(0, Rc::new(Orders { reactions: vec![when(Condition::ImmediateThreat)], ..Orders::default() }));
        let mut team = TeamState::default();
        team.threats[Threat::Deadly as usize] = true;
        assert_eq!(m.react(team, |_| false), None);
        team.threats[Threat::Immediate as usize] = true;
        assert_eq!(m.react(team, |_| false), Some(0));
        let mut m = mind(Alertness::Bored).with_orders(0, Rc::new(Orders { reactions: vec![when(Condition::GloballySuspicious)], ..Orders::default() }));
        let mut team = TeamState::default();
        team.awareness[2] = true;
        assert_eq!(m.react(team, |_| false), None);
        team.awareness[1] = true;
        assert_eq!(m.react(team, |_| false), Some(0));
    }

    #[test]
    fn a_terrorist_that_sees_a_player_is_sure_of_a_deadly_threat_and_its_team_aware() {
        // Facing the player across the room
        let mut sim = standoff([0.0, 0.0, 300.0], 180.0);
        let mate = crate::sim::characters::Npc { pos: [300.0, 0.0, 300.0], yaw: 0.0, ..sim.npcs[0].clone() };
        sim.npcs.push(mate);
        wait(&mut sim, 2.0 * crate::sim::TICK);
        let m = sim.npcs[0].mind.as_ref().unwrap();
        assert_eq!((m.threat, m.awareness, m.alertness, m.target), (Threat::Deadly, 2, Alertness::Alert, Some(0)));
        let mate = sim.npcs[1].mind.as_ref().unwrap();
        assert_eq!((mate.awareness, mate.alertness), (2, Alertness::Alert));
    }

    #[test]
    fn a_terrorist_glimpses_a_player_too_far_to_see_and_grows_suspicious() {
        // It sees 300 units: bored, its view reaches 590, and it glimpses out to 1.75 times that
        let mut sim = standoff([0.0, 0.0, 300.0], 180.0);
        sim.npcs[0].mind.as_mut().unwrap().sight = 300.0;
        wait(&mut sim, 0.5);
        let m = sim.npcs[0].mind.as_ref().unwrap();
        assert_eq!((m.awareness, m.alertness, m.target, m.threat), (1, Alertness::Ready, None, Threat::None));
    }

    #[test]
    fn a_terrorist_that_sees_a_dead_one_becomes_aware() {
        let mut sim = standoff([0.0, 0.0, 300.0], 90.0);
        let dead = crate::sim::characters::Npc { pos: [400.0, 0.0, 300.0], ..sim.npcs[0].clone() };
        sim.npcs.push(dead);
        sim.npcs[1].health.state = crate::combat::NpcState::Dead { motion: "thdu_1", start: 0.0 };
        wait(&mut sim, 2.0 * crate::sim::TICK);
        let m = sim.npcs[0].mind.as_ref().unwrap();
        assert_eq!((m.awareness, m.alertness, m.threat), (2, Alertness::Alert, Threat::None));
    }

    fn standoff(at: V3, yaw: f32) -> crate::sim::Sim {
        use crate::collision::{CollisionWorld, Params};
        let level = Rc::new(crate::demo::map());
        let world = CollisionWorld::from_map(&level, Params::default());
        let config = crate::sim::Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        let npc = crate::sim::characters::Npc {
            pos: at, floor: at[1], yaw, stance: Some(Stance::Stand),
            mind: Some(Mind::new(Alertness::Bored, 0.4, 0.3, 4000.0, MP5K, 30, 3)),
            ..Default::default()
        };
        let mut sim = crate::sim::Sim::new(level, world, config, None, vec![npc]);
        sim.add_player([0.0, 0.0, -350.0]);
        sim
    }

    /// Ticks for `seconds` with the player standing still, and when each shot at them was fired
    fn wait(sim: &mut crate::sim::Sim, seconds: f32) -> Vec<f32> {
        let still = crate::sim::Input { run: true, ..Default::default() };
        let mut shots = Vec::new();
        for _ in 0..(seconds / crate::sim::TICK) as usize {
            for e in sim.tick(&[still], crate::sim::TICK) {
                if matches!(e, crate::sim::Event::CharacterShot { .. }) {
                    shots.push(sim.time);
                }
            }
        }
        shots
    }

    #[test]
    fn a_terrorist_facing_a_player_waits_for_its_aim_then_shoots_them() {
        let mut sim = standoff([0.0, 0.0, 300.0], 180.0);
        let shots = wait(&mut sim, 3.0);
        // Not at once: it raises its weapon, and its spread opens wide when it first sees them
        assert!(shots.first().is_some_and(|&t| t >= RAISE_SECONDS), "{shots:?}");
        let died = sim.players[0].respawn_at.expect("the player still stands") - crate::sim::RESPAWN_SECONDS;
        // And not at the dead
        assert!(shots.iter().all(|&t| t <= died + crate::sim::TICK), "{shots:?} died at {died}");
    }

    #[test]
    fn a_terrorist_facing_away_does_not_see_a_player_across_the_room() {
        let mut sim = standoff([0.0, 0.0, 300.0], 0.0);
        assert!(wait(&mut sim, 3.0).is_empty());
        assert!(!sim.npcs[0].aiming);
        assert_eq!(sim.players[0].hp, crate::sim::PLAYER_HP);
    }

    #[test]
    fn a_terrorist_turns_round_on_a_player_close_behind_it() {
        let mut sim = standoff([0.0, 0.0, -230.0], 0.0);
        let shots = wait(&mut sim, 3.0);
        assert!(facing(sim.npcs[0].yaw, 180.0), "{}", sim.npcs[0].yaw);
        // A half turn at its rate, near enough, before it takes aim and fires
        assert!(shots.first().is_some_and(|&t| t >= (180.0 - AIM_CONE) / TURN_RATE), "{shots:?}");
    }

    #[test]
    fn a_terrorist_does_not_see_through_the_walls_of_rooms_far_from_the_players() {
        // rm05: a guard in the cabin's room 308 faces the spawn in room 314 through a wall.
        // Only the rooms around the player are solid, so it saw through it and shot them.
        use crate::collision::{CollisionWorld, Params};
        let Ok(level) = crate::map::read(std::path::Path::new("data/map/rm05/rm05.map")) else { return };
        let level = Rc::new(level);
        let world = CollisionWorld::from_map(&level, Params::default());
        let floor = world.ground_at(37789.0, 34346.0, 32877.0).unwrap();
        let npc = crate::sim::characters::Npc {
            pos: [37789.0, floor, 34346.0], floor, yaw: 179.0, stance: Some(Stance::Stand),
            mind: Some(Mind::new(Alertness::Alert, 0.4, 0.3, 4000.0, MP5K, 30, 3)),
            ..Default::default()
        };
        let config = crate::sim::Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        let mut sim = crate::sim::Sim::new(level, world, config, None, vec![npc]);
        sim.add_player([37813.4, 32767.0, 33285.3]);
        assert!(wait(&mut sim, 3.0).is_empty());
        assert!(!sim.npcs[0].aiming);
    }

    #[test]
    fn a_terrorist_hears_a_player_running_behind_it_and_turns_on_them() {
        let moving = |run: bool| {
            let mut i = crate::sim::Input { run, forward: true, ..Default::default() };
            i.set_look(90.0, 0.0);
            i
        };
        // Walking across the room is not heard
        let mut sim = standoff([0.0, 0.0, 300.0], 0.0);
        for _ in 0..30 {
            sim.tick(&[moving(false)], crate::sim::TICK);
        }
        assert!(sim.npcs[0].mind.as_ref().unwrap().heard.is_none());
        // Running near behind it is
        let mut sim = standoff([0.0, 0.0, 50.0], 0.0);
        for _ in 0..30 {
            sim.tick(&[moving(true)], crate::sim::TICK);
        }
        // Footsteps make it suspicious, and ready (0x6143e0)
        let m = sim.npcs[0].mind.as_ref().unwrap();
        assert!(m.heard.is_some());
        assert_eq!((m.alertness, m.awareness, m.threat), (Alertness::Ready, 1, Threat::Immediate));
        // It turns round, sees them and fires
        let shots = wait(&mut sim, 3.0);
        assert!(!shots.is_empty());
    }

    #[test]
    fn a_terrorist_hears_a_shot_and_turns_toward_it() {
        let mut sim = standoff([0.0, 0.0, 300.0], 0.0);
        let mut fire = crate::sim::Input { run: true, fire: true, ..Default::default() };
        fire.set_look(180.0, 0.0);
        sim.tick(&[fire], crate::sim::TICK);
        assert!(sim.npcs[0].mind.as_ref().unwrap().heard.is_some());
        let shots = wait(&mut sim, 3.0);
        assert!(sim.npcs[0].aiming && !shots.is_empty());
    }

    #[test]
    fn a_terrorist_walks_to_what_it_heard_and_stops_there() {
        let mut sim = standoff([0.0, 0.0, 300.0], 0.0);
        let orders = Orders { plan: Plan { roe: Roe::Investigate, ..Plan::default() }, ..Orders::default() };
        let mind = sim.npcs[0].mind.take().unwrap().with_orders(0, Rc::new(orders));
        sim.npcs[0].mind = Some(mind);
        sim.noises.push(([200.0, 50.0, 300.0], 1000.0, Noise::Sound));
        let mut walked = false;
        for _ in 0..60 {
            wait(&mut sim, crate::sim::TICK);
            walked |= sim.npcs[0].walking;
        }
        let n = &sim.npcs[0];
        assert!(walked && !n.walking, "still walking at {:?}", n.pos);
        assert!((n.pos[0] - 200.0).abs() <= 30.0 + 1.0 && (n.pos[2] - 300.0).abs() < 1.0, "{:?}", n.pos);
        assert_eq!(n.stance, Some(Stance::Stand));
    }

    #[test]
    fn a_defending_terrorist_spooks_but_stays_put_and_its_trigger_plan_sends_it_running() {
        // Defending, it turns its weapon toward a sound but stays where it is
        let mut sim = standoff([0.0, 0.0, 300.0], 0.0);
        sim.noises.push(([200.0, 50.0, 300.0], 1000.0, Noise::Sound));
        wait(&mut sim, 0.5);
        let n = &sim.npcs[0];
        assert!(n.aiming && !n.walking && n.pos == [0.0, 0.0, 300.0]);
        assert_eq!(n.mind.as_ref().unwrap().threat, Threat::Suspicious);
        // Its team's plan says to advance at a run on gunfire near by: it does
        let mut sim = standoff([0.0, 0.0, 300.0], 0.0);
        let rush = Reaction {
            name: "Rush".into(), priority: 0, only: false, when: vec![Condition::ImmediateThreat],
            plan: Plan { roe: Roe::Advance, speed: Speed::Blitz, ..Plan::default() },
        };
        let mind = sim.npcs[0].mind.take().unwrap().with_orders(0, Rc::new(Orders { reactions: vec![rush], ..Orders::default() }));
        sim.npcs[0].mind = Some(mind);
        sim.noises.push(([200.0, 50.0, 300.0], 10000.0, Noise::Sound));
        wait(&mut sim, 2.0 * crate::sim::TICK);
        sim.noises.push(([200.0, 50.0, 300.0], 10000.0, Noise::Sound));
        wait(&mut sim, crate::sim::TICK);
        let (n, m) = (&sim.npcs[0], sim.npcs[0].mind.as_ref().unwrap());
        assert_eq!((m.active, m.plan().roe, m.threat), (Some(0), Roe::Advance, Threat::Immediate));
        assert!(n.walking && n.gait == Gait::Run, "{:?}", n.pos);
    }

    #[test]
    fn a_team_takes_cover_in_a_wedge_behind_its_first() {
        // Watching along +z: the others stand back along -z and aside, in pairs
        let (watch, at) = ([0.0, 0.0, 1000.0], [0.0, 0.0, 0.0]);
        let spots: Vec<V3> = (0..5).map(|k| cover_spot(watch, at, k)).collect();
        assert_eq!(spots, [[0.0, 0.0, 0.0], [-150.0, 0.0, -150.0], [150.0, 0.0, -150.0], [-300.0, 0.0, -300.0], [300.0, 0.0, -300.0]]);
        // Facing what they watch, fanned out
        assert_eq!(cover_yaw(watch, at, 0, 1), 0.0);
        assert_eq!([0, 1, 2].map(|k| cover_yaw(watch, at, k, 3)), [0.0, 45.0, -45.0]);
        assert_eq!([0, 1, 2, 3].map(|k| cover_yaw(watch, at, k, 4)), [-18.0, 18.0, -18.0, 36.0]);
        // In the game, they go there and stay, facing it
        let mut sim = standoff([0.0, 0.0, 300.0], 0.0);
        let plan = Plan { route: vec![Step::Cover { watch: [-400.0, 0.0, -300.0], at: [-200.0, 0.0, 200.0] }, Step::Move([300.0, 0.0, 300.0])], ..Plan::default() };
        let mind = sim.npcs[0].mind.take().unwrap().with_orders(0, Rc::new(Orders { plan, ..Orders::default() }));
        sim.npcs[0].mind = Some(mind);
        away(&mut sim, 4.0);
        let n = &sim.npcs[0];
        assert!(crate::sim::dist3([n.pos[0], 0.0, n.pos[2]], [-200.0, 0.0, 200.0]) <= 31.0 && !n.walking, "{:?}", n.pos);
        let toward = (-200.0f32).atan2(-500.0).to_degrees();
        assert!((n.yaw - toward).rem_euclid(360.0).min((toward - n.yaw).rem_euclid(360.0)) < 1.0, "{} not {toward}", n.yaw);
        assert!(n.mind.as_ref().unwrap().covering);
    }

    /// Ticks for `seconds` with the player out of the game, flying free, and the log
    fn away(sim: &mut crate::sim::Sim, seconds: f32) -> Vec<String> {
        let free = crate::sim::Input { observer: Some([0.0, 0.0, -350.0]), ..Default::default() };
        let mut log = Vec::new();
        for _ in 0..(seconds / crate::sim::TICK) as usize {
            log.extend(sim.tick(&[free], crate::sim::TICK).into_iter().filter_map(|e| match e {
                crate::sim::Event::Log(l) => Some(l),
                _ => None,
            }));
        }
        log
    }

    /// A hostage named Hostage1 across the room, and a terrorist that hunts them, firing on them
    /// if `engage`. None without the characters' models.
    fn hunt(engage: bool) -> Option<crate::sim::Sim> {
        let mut chars = crate::sim::characters::Characters::load(std::path::Path::new("data/character/team_hrt/blue_M_a.CRP"), "up_none_cyc")?;
        let mut sim = standoff([0.0, 0.0, 300.0], 0.0);
        let hostage = crate::sim::characters::Npc { pos: [300.0, 0.0, -300.0], stance: Some(Stance::Stand), name: "Hostage1".into(), ..Default::default() };
        let feet_to_origin = -chars.body(&hostage).0.bounds().0[1];
        sim.npcs.push(crate::sim::characters::Npc { pos: [300.0, feet_to_origin, -300.0], ..hostage });
        sim.npcs[0].weapon = chars.add_weapon("tw_ak47.wpn");
        sim.characters = Some(chars);
        let route = vec![Step::EngageHostages(engage), Step::Hunt("Hostage1".into()), Step::Move([-300.0, 0.0, 300.0])];
        let mind = sim.npcs[0].mind.take().unwrap().with_orders(0, Rc::new(Orders { plan: Plan { route, ..Plan::default() }, ..Orders::default() }));
        sim.npcs[0].mind = Some(mind);
        Some(sim)
    }

    #[test]
    fn a_terrorist_hunts_down_a_hostage_and_goes_on_once_they_are_dead() {
        let Some(mut sim) = hunt(true) else { return };
        let fired = away(&mut sim, 10.0).iter().any(|l| l.contains("fires at character 1"));
        assert!(fired && sim.npcs[1].health.is_dead(), "hostage hp {}", sim.npcs[1].health.hp);
        // Then on to its next step, which it walks
        let (n, m) = (&sim.npcs[0], sim.npcs[0].mind.as_ref().unwrap());
        assert!(m.step >= 2 && crate::sim::dist3([n.pos[0], 0.0, n.pos[2]], [-300.0, 0.0, 300.0]) <= 31.0, "step {} at {:?}", m.step, n.pos);
    }

    #[test]
    fn pellets_spread_round_the_shot_a_quarter_each() {
        let dir = [0.0, 0.0, 1.0];
        assert_eq!(pellet(dir, 0, 2.16, [0.0, 0.0]), dir);
        let ways: Vec<V3> = (0..4).map(|k| pellet(dir, k, 2.16, [1.0, 1.0])).collect();
        // Across is +x for a shot along +z, up is +y: one pellet in each quarter round it
        let quarters: Vec<(bool, bool)> = ways.iter().map(|w| (w[0] > 0.0, w[1] > 0.0)).collect();
        assert_eq!(quarters, [(true, true), (false, true), (false, false), (true, false)]);
        for w in &ways {
            assert!(((w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt() - 1.0).abs() < 1e-5);
            // No further off than the corner of the spread
            let off = w[2].acos().to_degrees();
            assert!(off <= (2.16f32 * 2f32.sqrt() / 100.0).atan().to_degrees() + 1e-3, "{off}");
        }
        // Straight down, it still spreads
        let down = pellet([0.0, -1.0, 0.0], 1, 3.0, [0.5, 0.5]);
        assert!(down[1] < -0.99 && down[0] != 0.0 && down[2] != 0.0, "{down:?}");
    }

    #[test]
    fn a_terrorist_with_a_shotgun_fires_its_pellets() {
        let Some(mut chars) = crate::sim::characters::Characters::load(std::path::Path::new("data/character/team_hrt/blue_M_a.CRP"), "up_none_cyc") else { return };
        let mut sim = standoff([0.0, 0.0, 300.0], 180.0);
        sim.npcs[0].weapon = chars.add_weapon("pw_shotgun.wpn");
        sim.characters = Some(chars);
        let still = crate::sim::Input { run: true, ..Default::default() };
        let mut volleys = Vec::new();
        for _ in 0..(3.0 / crate::sim::TICK) as usize {
            let ev = sim.tick(&[still], crate::sim::TICK);
            if ev.iter().any(|e| matches!(e, crate::sim::Event::Log(l) if l.contains("fires at"))) {
                volleys.push(ev.iter().filter(|e| matches!(e, crate::sim::Event::CharacterShot { .. })).count());
            }
        }
        assert!(!volleys.is_empty() && volleys.iter().all(|&n| n == 4), "{volleys:?}");
    }

    #[test]
    fn a_kneeling_hostage_flinches_and_falls_from_kneeling() {
        let Some(mut sim) = hunt(true) else { return };
        sim.npcs[1].stance = Some(Stance::Kneel);
        let log = away(&mut sim, 10.0);
        let hits: Vec<&String> = log.iter().filter(|l| l.contains("hit character 1")).collect();
        assert!(!hits.is_empty() && sim.npcs[1].health.is_dead(), "{log:?}");
        // Resting in its stance, as a hostage always is, it falls out of it
        let crate::combat::NpcState::Dead { motion, .. } = sim.npcs[1].health.state else { unreachable!() };
        assert!(motion.starts_with("rhd_cr"), "{motion}");
    }

    #[test]
    fn a_terrorist_not_to_fire_on_hostages_goes_to_one_and_waits() {
        let Some(mut sim) = hunt(false) else { return };
        let log = away(&mut sim, 6.0);
        assert!(!log.iter().any(|l| l.contains("fires")) && !sim.npcs[1].health.is_dead(), "{log:?}");
        let (n, h) = (&sim.npcs[0], &sim.npcs[1]);
        assert!(crate::sim::dist3([n.pos[0], 0.0, n.pos[2]], [h.pos[0], 0.0, h.pos[2]]) < 100.0, "{:?}", n.pos);
    }

    #[test]
    fn a_patrol_walks_its_loop_round_and_round_and_a_guard_stays_put() {
        let mut sim = standoff([0.0, 0.0, 300.0], 0.0);
        let route = vec![Step::Loop, Step::Move([200.0, 0.0, 300.0]), Step::Move([-200.0, 0.0, 300.0]), Step::EndLoop];
        let plan = Plan { roe: Roe::Investigate, route, ..Plan::default() };
        let mind = sim.npcs[0].mind.take().unwrap().with_orders(0, Rc::new(Orders { plan, ..Orders::default() }));
        sim.npcs[0].mind = Some(mind);
        // Out one way, back the other, and out again
        let (mut east, mut west, mut turns) = (0.0f32, 0.0f32, 0);
        let mut last = sim.npcs[0].pos[0];
        let mut heading = 0.0f32;
        for _ in 0..(3.0 / crate::sim::TICK) as usize {
            wait(&mut sim, crate::sim::TICK);
            let x = sim.npcs[0].pos[0];
            let now = (x - last).signum();
            if now != 0.0 && heading != 0.0 && now != heading {
                turns += 1;
            }
            heading = if now != 0.0 { now } else { heading };
            (east, west, last) = (east.max(x), west.min(x), x);
        }
        assert!(east > 160.0 && west < -160.0 && turns >= 3, "{east} {west} {turns}");
        assert!(sim.npcs[0].walking && !sim.npcs[0].aiming);
        // A guard goes to its spot, and stays
        let mut sim = standoff([0.0, 0.0, 300.0], 0.0);
        let plan = Plan { route: vec![Step::Move([200.0, 0.0, 300.0]), Step::Guard, Step::Move([-200.0, 0.0, 300.0])], ..Plan::default() };
        let mind = sim.npcs[0].mind.take().unwrap().with_orders(0, Rc::new(Orders { plan, ..Orders::default() }));
        sim.npcs[0].mind = Some(mind);
        wait(&mut sim, 3.0);
        assert!((sim.npcs[0].pos[0] - 200.0).abs() <= 31.0 && !sim.npcs[0].walking, "{:?}", sim.npcs[0].pos);
    }

    #[test]
    fn a_zone_patrol_goes_to_the_rooms_of_its_zones_and_a_threat_in_a_zone_sets_off_its_plan() {
        let zones = [(5, vec![10, 11]), (1, vec![7]), (9, vec![99])].into_iter().collect();
        let plan = Plan { route: vec![Step::AddZone(5), Step::AddZone(1), Step::PatrolZones], ..Plan::default() };
        let search = |priority, only, zone| Reaction {
            name: format!("search {zone}"), priority, only, when: vec![Condition::ThreatInZone(zone)],
            plan: Plan { roe: Roe::Investigate, speed: Speed::Blitz, route: vec![Step::ClearZones, Step::AddZone(zone), Step::PatrolZones], ..Plan::default() },
        };
        let orders = Orders { plan, reactions: vec![search(0, true, 9), search(0, false, 1)], zones };
        let mut m = mind(Alertness::Bored).with_orders(0, Rc::new(orders));
        let rooms: std::collections::HashSet<u32> = (0..20).filter_map(|pick| match m.next_place(pick) {
            Some(Place::Room(r)) => Some(r),
            _ => None,
        }).collect();
        assert_eq!(rooms, [10, 11, 7].into_iter().collect());
        // A threat in zone 9 sets off its search, which rules out the other of its priority
        assert_eq!(m.react(TeamState::default(), |z| z == 9), Some(0));
        assert_eq!(m.next_place(3), Some(Place::Room(99)));
        assert_eq!(m.react(TeamState::default(), |z| z == 1), None);
    }

    #[test]
    fn a_way_across_a_level_goes_through_doorways_and_up_its_stairs() {
        let Ok(level) = crate::map::read(std::path::Path::new("data/map/rm05/rm05.map")) else { return };
        let vis = crate::visibility::Visibility::new(&level);
        // From the spawn in the cabin to the upper deck at the front
        let (a, b) = ([37813.4, 32817.0, 33285.3], [37607.0, 33170.0, 36478.0]);
        let (ra, rb) = (vis.player_rooms(&level, a, None), vis.player_rooms(&level, b, None));
        let way = vis.path(a, &ra, b, &rb, &[]).expect("a way there");
        assert!(way.len() > 3 && way.last() == Some(&b));
        assert!(way.iter().any(|p| p[1] > 33000.0), "{way:?}");
        // A closed door on the way, and there is none
        let blocked: Vec<bool> = (0..level.portals.portals.len()).map(|_| true).collect();
        assert!(vis.path(a, &ra, b, &rb, &blocked).is_none());
    }

    #[test]
    fn the_walking_grid_takes_a_way_round_the_ships_walls_to_the_deck() {
        // rm02: from the hostage guards' room in the ship's superstructure out to the deck,
        // where a straight line across the deck goes through the superstructure
        use crate::collision::{CollisionWorld, Params};
        let Ok(level) = crate::map::read(std::path::Path::new("data/map/rm02/rm02.map")) else { return };
        let world = CollisionWorld::from_map(&level, Params::default());
        let vis = crate::visibility::Visibility::new(&level);
        let rooms = |p: V3| vis.player_rooms(&level, [p[0], p[1] + 50.0, p[2]], None);
        let (from, to) = ([28906.0, 34567.0, 31008.0], [39295.4, 32767.0, 32056.6]);
        let grid = super::super::nav::Grid::new(&world, &level, &vis, &[(from, rooms(from))]);
        assert!(grid.len() > 1000 && grid.len() < 200_000, "{} places", grid.len());
        let way = grid.path(&world, from, &rooms(from), to, &rooms(to), &[]).expect("a way to the deck");
        // Every leg clear of the walls of the rooms it is in, but doors, which open
        let mut prev = from;
        for &(p, _) in &way {
            let (a, b) = ([prev[0], prev[1] + 80.0, prev[2]], [p[0], p[1] + 80.0, p[2]]);
            let d = super::super::dist3(a, b);
            if d > 1.0 {
                let dir = [0, 1, 2].map(|k| (b[k] - a[k]) / d);
                let legs_rooms: Vec<u32> = [prev, p].iter().flat_map(|&q| rooms(q)).collect();
                if let Some(h) = world.bullet_hit_in(a, dir, d - 1.0, &legs_rooms) {
                    let mat = h.material.and_then(|m| level.materials.materials.get(m)).map_or("?", |m| m.filename.as_str());
                    assert!(mat.to_ascii_lowercase().contains("door"), "{prev:?} -> {p:?} through {mat}");
                }
            }
            prev = p;
        }
    }

    #[test]
    fn a_terrorist_outside_does_not_see_into_a_closed_room_of_the_house() {
        // rm16: the grounds (room 1) hold the whole house, so a player in its room 205 is in
        // room 1 too, and the room's walls once went unseen from outside
        use crate::collision::{CollisionWorld, Params};
        let Ok(level) = crate::map::read(std::path::Path::new("data/map/rm16/rm16.map")) else { return };
        let level = Rc::new(level);
        let world = CollisionWorld::from_map(&level, Params::default());
        let floor = world.ground_at(30098.0, 34276.0, 32979.0).unwrap();
        let npc = crate::sim::characters::Npc {
            pos: [30098.0, floor, 34276.0], floor, yaw: 45.0, stance: Some(Stance::Stand),
            mind: Some(Mind::new(Alertness::Alert, 0.4, 0.3, 4000.0, MP5K, 30, 3)),
            ..Default::default()
        };
        let config = crate::sim::Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        let mut sim = crate::sim::Sim::new(level, world, config, None, vec![npc]);
        sim.add_player([30828.8, 33167.0, 35006.4]);
        assert!(wait(&mut sim, 3.0).is_empty());
        assert!(sim.npcs[0].mind.as_ref().unwrap().target.is_none());
    }

    #[test]
    fn a_way_into_a_room_with_its_doors_shut_goes_through_a_door() {
        // rm16: room 205, whose doors are shut, from the grounds outside the house
        use crate::collision::{CollisionWorld, Params};
        let Ok(level) = crate::map::read(std::path::Path::new("data/map/rm16/rm16.map")) else { return };
        let world = CollisionWorld::from_map(&level, Params::default());
        let vis = crate::visibility::Visibility::new(&level);
        let rooms = |p: V3| vis.player_rooms(&level, [p[0], p[1] + 50.0, p[2]], None);
        let outside = [30098.0, world.ground_at(30098.0, 34276.0, 32979.0).unwrap(), 34276.0];
        let inside = [30828.8, 33167.0, 35006.4];
        let grid = super::super::nav::Grid::new(&world, &level, &vis, &[(outside, rooms(outside)), (inside, rooms(inside))]);
        let shut: Vec<bool> = level.portals.portals.iter().map(|p| p.name.contains("205")).collect();
        let started = std::time::Instant::now();
        let way = grid.path(&world, outside, &rooms(outside), inside, &rooms(inside), &shut).expect("a way in");
        assert!(started.elapsed().as_millis() < 100, "{:?} over {} places", started.elapsed(), grid.len());
        let doorways: Vec<&str> = way.iter().filter_map(|w| w.1).map(|p| level.portals.portals[p as usize].name.as_str()).collect();
        assert!(doorways.iter().any(|n| n.contains("205")), "{doorways:?}");
        // Nowhere near the grid, there is none
        assert!(grid.path(&world, outside, &rooms(outside), [outside[0] + 100_000.0, outside[1], outside[2]], &[], &shut).is_none());
    }

    /// A terrorist outside rm16's house that goes to see what it hears, facing away from room 205
    fn outside_205() -> Option<crate::sim::Sim> {
        use crate::collision::{CollisionWorld, Params};
        let level = Rc::new(crate::map::read(std::path::Path::new("data/map/rm16/rm16.map")).ok()?);
        let world = CollisionWorld::from_map(&level, Params::default());
        let floor = world.ground_at(30098.0, 34276.0, 32979.0).unwrap();
        let orders = Orders { plan: Plan { roe: Roe::Investigate, ..Plan::default() }, ..Orders::default() };
        let npc = crate::sim::characters::Npc {
            pos: [30098.0, floor, 34276.0], floor, yaw: 225.0, stance: Some(Stance::Stand),
            mind: Some(Mind::new(Alertness::Alert, 0.4, 0.3, 4000.0, MP5K, 30, 3).with_orders(0, Rc::new(orders))),
            ..Default::default()
        };
        let config = crate::sim::Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        let mut sim = crate::sim::Sim::new(level, world, config, None, vec![npc]);
        sim.add_player([30828.8, 33167.0, 35006.4]);
        // The doors settle shut
        wait(&mut sim, 0.1);
        Some(sim)
    }

    #[test]
    fn a_terrorist_remembers_it_found_no_way_to_shots_and_does_not_look_at_each() {
        let Some(mut sim) = outside_205() else { return };
        // Somewhere no way goes
        let shot = [130_000.0, 33332.0, 35006.4];
        let asked = |sim: &crate::sim::Sim| sim.npcs[0].mind.as_ref().unwrap().asked.map(|(_, t)| t);
        let mut ev = Vec::new();
        sim.terrorists(0.1, &[(shot, 1e6, Noise::Sound)], &mut ev);
        let first = asked(&sim).expect("looked for a way");
        assert!(sim.npcs[0].mind.as_ref().unwrap().path.is_empty());
        // Ten shots a second from the same place: it does not look again for a while
        for _ in 0..40 {
            sim.time += 0.1;
            sim.terrorists(0.1, &[(shot, 1e6, Noise::Sound)], &mut ev);
        }
        assert_eq!(asked(&sim), Some(first));
        // Later, it does
        sim.time += ASK_SAME;
        sim.terrorists(0.1, &[(shot, 1e6, Noise::Sound)], &mut ev);
        assert!(asked(&sim) > Some(first));
    }

    #[test]
    fn the_debug_dump_says_where_each_terrorist_is_and_whether_it_sees_the_player() {
        let Some(sim) = outside_205() else { return };
        let report = sim.terrorist_report(0);
        assert_eq!(report.len(), 1);
        assert!(report[0].starts_with("#0 team 0: at (30098, "), "{}", report[0]);
        assert!(report[0].contains("clear line to body false head false"), "{}", report[0]);
        assert!(report[0].contains("Alert, awareness 0, threat None"), "{}", report[0]);
        // And what it knows of
        let Some(mut sim) = outside_205() else { return };
        sim.npcs[0].mind.as_mut().unwrap().know(Sign::Heard, None, [1.0, 2.0, 3.0], 0.0);
        sim.time = 1.5;
        let report = sim.terrorist_report(0);
        assert!(report[0].contains("knows of Heard None at (1, 2, 3) 1.5s ago"), "{}", report[0]);
    }

    #[test]
    fn a_terrorist_opens_a_shut_door_to_go_and_see_a_shot() {
        let Some(mut sim) = outside_205() else { return };
        let shut = |sim: &crate::sim::Sim| sim.doors.iter().zip(&sim.door_states)
            .filter(|(d, _)| d.portal.is_some_and(|p| sim.level.portals.portals[p].name.contains("205")))
            .all(|(_, s)| s.closed());
        assert!(shut(&sim));
        let mut ev = Vec::new();
        sim.terrorists(0.1, &[([30828.8, 33332.0, 35006.4], 10_000.0, Noise::Sound)], &mut ev);
        assert!(!sim.npcs[0].mind.as_ref().unwrap().path.is_empty());
        let mut opened = false;
        for _ in 0..(20.0 / crate::sim::TICK) as usize {
            wait(&mut sim, crate::sim::TICK);
            opened |= !shut(&sim);
            if sim.npcs[0].mind.as_ref().unwrap().target.is_some() {
                break;
            }
        }
        assert!(opened, "no door of 205 opened; at {:?}", sim.npcs[0].pos);
        assert_eq!(sim.npcs[0].mind.as_ref().unwrap().target, Some(0), "never saw the player; at {:?}", sim.npcs[0].pos);
    }

    #[test]
    fn none_of_rm16s_terrorists_outside_sees_into_the_house() {
        // Well inside the grounds (room 1) but near the wall of room 205, the player was once
        // only in room 1, and three terrorists outside saw them through 205's walls. In room
        // 201, the line from outside stayed in room 1, and missed the walls of the rooms between.
        use crate::collision::{CollisionWorld, Params};
        use crate::mission::{self, Difficulty};
        let map_path = std::path::Path::new("data/map/rm16/rm16.map");
        let Some((file, teams)) = mission::teams(map_path, Difficulty::Easy) else { return };
        let Some(mut chars) = crate::sim::characters::Characters::load(std::path::Path::new("data/character/team_hrt/blue_M_a.CRP"), "up_none_cyc") else { return };
        let level = Rc::new(crate::map::read(map_path).unwrap());
        let world = CollisionWorld::from_map(&level, Params::default());
        let npcs = crate::sim::characters::place_terrorists(&teams, 7, &mission::setting(&file), &world, &mut chars);
        let config = crate::sim::Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        let sim = crate::sim::Sim::new(level, world, config, None, npcs);
        for (feet, room) in [([30580.2, 33167.0, 34953.0], 205), ([29784.1, 33169.0, 35005.5], 201), ([29810.4, 33169.0, 35125.2], 201)] {
            let theirs = sim.rooms_at(feet, None);
            assert!(theirs.contains(&room), "{theirs:?}");
            for n in sim.npcs.iter().filter(|n| n.mind.is_some()) {
                let (eye, rooms) = ([n.pos[0], n.floor + 160.0, n.pos[2]], sim.rooms_at([n.pos[0], n.floor, n.pos[2]], None));
                for up in [0.65, 0.94] {
                    assert!(!sim.line_clear(eye, &rooms, [feet[0], feet[1] + up * 180.0, feet[2]], &theirs), "{feet:?} seen from {:?}", n.pos);
                }
            }
        }
    }

    #[test]
    fn rm16s_terrorists_ways_into_room_205_keep_clear_of_the_walls() {
        // A terrorist cut across the corner of a wall in room 205: a straight leg between
        // places where a body fits passed too near it. And a leg across rm16's grounds went
        // through the wall of a room inside them, whose walls the grounds' places did not heed.
        use crate::collision::{CollisionWorld, Params};
        use crate::mission::{self, Difficulty};
        let map_path = std::path::Path::new("data/map/rm16/rm16.map");
        let Some((file, teams)) = mission::teams(map_path, Difficulty::Easy) else { return };
        let Some(mut chars) = crate::sim::characters::Characters::load(std::path::Path::new("data/character/team_hrt/blue_M_a.CRP"), "up_none_cyc") else { return };
        let level = Rc::new(crate::map::read(map_path).unwrap());
        let world = CollisionWorld::from_map(&level, Params::default());
        let npcs = crate::sim::characters::place_terrorists(&teams, 7, &mission::setting(&file), &world, &mut chars);
        let config = crate::sim::Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        let mut sim = crate::sim::Sim::new(level, world, config, None, npcs);
        let to = [30537.0, 33167.0, 34967.0];
        sim.add_player(to);
        let started = std::time::Instant::now();
        sim.terrorists(0.01, &[], &mut Vec::new());
        eprintln!("the grid took {:?}", started.elapsed());
        let grid = sim.nav.as_ref().unwrap();
        let there = sim.rooms_at(to, None);
        let head = sim.world.params().height;
        for (i, n) in sim.npcs.iter().enumerate() {
            let feet = [n.pos[0], n.floor, n.pos[2]];
            let way = grid.path(&sim.world, feet, &sim.rooms_at(feet, None), to, &there, &sim.blocked_portals).expect("a way");
            // Between the points of the way: the first is near its feet, and the last is the
            // player's, where a player stands
            for leg in way[..way.len() - 1].windows(2) {
                let ((a, _), (b, door)) = (leg[0], leg[1]);
                let n = (crate::sim::dist3(a, b) / 5.0).ceil() as usize;
                for k in 1..n {
                    let t = k as f32 / n as f32;
                    let at = [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * t);
                    let Some(y) = sim.world.ground_at(at[0], at[2], at[1] + 60.0) else { continue };
                    let foot = [at[0], y, at[2]];
                    for r in sim.rooms_at(foot, None) {
                        let c = sim.world.wall_clearance_in(&crate::collision::Player::new(foot), r);
                        assert!(door.is_some() || c > -3.0, "#{i}: {a:?} -> {b:?} into room {r}'s wall by {:.0} at {foot:?}", -c);
                        // Nothing to walk through overhead, as a table's top
                        let over = sim.world.floors_in(r, foot[0], foot[2]).into_iter().find(|&f| f > foot[1] + 45.0 && f < foot[1] + head);
                        assert!(over.is_none(), "#{i}: {a:?} -> {b:?} under a top of room {r} at {over:?} at {foot:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_shot_many_hear_sets_them_off_one_search_a_tick() {
        // The first shot in rm16 once had five of them look for their ways in the same tick
        use crate::collision::{CollisionWorld, Params};
        use crate::mission::{self, Difficulty};
        let map_path = std::path::Path::new("data/map/rm16/rm16.map");
        let Some((file, teams)) = mission::teams(map_path, Difficulty::Easy) else { return };
        let Some(mut chars) = crate::sim::characters::Characters::load(std::path::Path::new("data/character/team_hrt/blue_M_a.CRP"), "up_none_cyc") else { return };
        let level = Rc::new(crate::map::read(map_path).unwrap());
        let world = CollisionWorld::from_map(&level, Params::default());
        let npcs = crate::sim::characters::place_terrorists(&teams, 7, &mission::setting(&file), &world, &mut chars);
        let config = crate::sim::Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        let mut sim = crate::sim::Sim::new(level, world, config, None, npcs);
        let at = [30537.0, 33167.0, 34967.0];
        sim.add_player(at);
        sim.terrorists(crate::sim::TICK, &[], &mut Vec::new());
        let mut went = Vec::new();
        for k in 0..30 {
            let mut ev = Vec::new();
            let shot = [([at[0], at[1] + 150.0, at[2]], 6000.0, Noise::Sound)];
            sim.terrorists(crate::sim::TICK, if k == 0 { &shot } else { &[] }, &mut ev);
            sim.time += crate::sim::TICK;
            let now = ev.iter().filter(|e| matches!(e, crate::sim::Event::Log(l) if l.contains("goes to see"))).count();
            assert!(now <= SEARCHES_PER_TICK, "{now} in tick {k}");
            went.extend(std::iter::repeat_n(k, now));
        }
        assert!(went.len() >= 5, "{went:?}");
    }

    /// The distance across the floor between two characters
    fn apart(sim: &crate::sim::Sim, a: usize, b: usize) -> f32 {
        let (p, q) = (sim.npcs[a].pos, sim.npcs[b].pos);
        ((p[0] - q[0]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
    }

    #[test]
    fn two_terrorists_on_one_spot_step_apart() {
        let mut sim = standoff([0.0, 0.0, 300.0], 180.0);
        sim.npcs.push(sim.npcs[0].clone());
        wait(&mut sim, 1.0);
        assert!(apart(&sim, 0, 1) >= 59.0, "{:?} {:?}", sim.npcs[0].pos, sim.npcs[1].pos);
    }

    #[test]
    fn a_terrorist_on_a_player_steps_off_and_the_player_stays() {
        let mut sim = standoff([0.0, 0.0, -340.0], 180.0);
        sim.god = true;
        wait(&mut sim, 1.0);
        let (n, p) = (sim.npcs[0].pos, sim.players[0].player.pos);
        assert_eq!(p, [0.0, 0.0, -350.0]);
        assert!(((n[0] - p[0]).powi(2) + (n[2] - p[2]).powi(2)).sqrt() >= 59.0, "{n:?}");
    }

    #[test]
    fn terrorists_against_a_wall_step_apart_along_it() {
        let x = crate::demo::WIDTH / 2.0 - 31.0;
        let mut sim = standoff([x, 0.0, 0.0], 270.0);
        sim.npcs.push(sim.npcs[0].clone());
        wait(&mut sim, 1.0);
        assert!(apart(&sim, 0, 1) >= 59.0, "{:?} {:?}", sim.npcs[0].pos, sim.npcs[1].pos);
        for n in &sim.npcs {
            assert!(sim.world.wall_clearance(&crate::collision::Player::new(n.pos)) >= -1.0, "in the wall at {:?}", n.pos);
        }
    }

    #[test]
    fn terrorists_sent_to_one_noise_stand_apart_there() {
        // rm16: the first shot once had ten of them stand inside each other where it was heard
        use crate::collision::{CollisionWorld, Params};
        use crate::mission::{self, Difficulty};
        let map_path = std::path::Path::new("data/map/rm16/rm16.map");
        let Some((file, teams)) = mission::teams(map_path, Difficulty::Easy) else { return };
        let Some(mut chars) = crate::sim::characters::Characters::load(std::path::Path::new("data/character/team_hrt/blue_M_a.CRP"), "up_none_cyc") else { return };
        let level = Rc::new(crate::map::read(map_path).unwrap());
        let world = CollisionWorld::from_map(&level, Params::default());
        let npcs = crate::sim::characters::place_terrorists(&teams, 7, &mission::setting(&file), &world, &mut chars);
        let config = crate::sim::Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        let mut sim = crate::sim::Sim::new(level, world, config, None, npcs);
        let at = [30537.0, 33167.0, 34967.0];
        sim.add_player(at);
        sim.god = true;
        wait(&mut sim, 0.1);
        sim.noises.push(([at[0], at[1] + 150.0, at[2]], 6000.0, Noise::Sound));
        wait(&mut sim, 20.0);
        let near: Vec<usize> = (0..sim.npcs.len()).filter(|&i| {
            let p = sim.npcs[i].pos;
            !sim.npcs[i].health.is_dead() && ((p[0] - at[0]).powi(2) + (p[2] - at[2]).powi(2)).sqrt() < 600.0
        }).collect();
        assert!(near.len() >= 4, "only {near:?} came");
        for &i in &near {
            let n = &sim.npcs[i];
            let feet = [n.pos[0], n.floor, n.pos[2]];
            for r in sim.rooms_at(feet, None) {
                let c = sim.world.wall_clearance_in(&crate::collision::Player::new(feet), r);
                assert!(c >= -3.0, "#{i} in room {r}'s wall by {:.0} at {feet:?}", -c);
            }
        }
        for (k, &a) in near.iter().enumerate() {
            for &b in near[k + 1..].iter().filter(|&&b| (sim.npcs[a].floor - sim.npcs[b].floor).abs() < 100.0) {
                assert!(apart(&sim, a, b) >= 55.0, "#{a} and #{b}: {:?} {:?}
{}
{}", sim.npcs[a].pos, sim.npcs[b].pos, sim.terrorist_report(0)[a], sim.terrorist_report(0)[b]);
            }
        }
    }

    #[test]
    fn a_terrorist_out_of_ammo_draws_its_sidearm_and_fights_on() {
        let mut sim = standoff([0.0, 0.0, 300.0], 180.0);
        sim.god = true;
        let m = sim.npcs[0].mind.as_mut().unwrap();
        (m.ammo, m.magazines) = (1, 0);
        *m = m.clone().with_sidearm(7, MP5K, 7, 2);
        let shots = wait(&mut sim, 8.0);
        let m = sim.npcs[0].mind.as_ref().unwrap();
        assert_eq!(sim.npcs[0].weapon, Some(7));
        assert!(m.sidearm.is_none());
        // Its last round, then the sidearm's, a draw later
        assert!(shots.len() >= 3, "{shots:?}");
        assert!(shots[1] - shots[0] >= DRAW_SECONDS, "{shots:?}");
    }

    #[test]
    fn a_terrorist_with_nothing_left_runs_from_the_fight_and_cowers() {
        let mut sim = standoff([0.0, 0.0, 0.0], 180.0);
        sim.god = true;
        let m = sim.npcs[0].mind.as_mut().unwrap();
        (m.ammo, m.magazines) = (1, 0);
        let shots = wait(&mut sim, 8.0);
        assert_eq!(shots.len(), 1, "{shots:?}");
        let (n, p) = (&sim.npcs[0], sim.players[0].player.pos);
        assert_eq!(n.mind.as_ref().unwrap().spent, Some(Spent::Hiding));
        assert_eq!(n.stance, Some(Stance::Cower));
        assert!(!n.aiming);
        // From 350 away to the far side of the room
        assert!(((n.pos[0] - p[0]).powi(2) + (n.pos[2] - p[2]).powi(2)).sqrt() > 700.0, "{:?}", n.pos);
    }

    #[test]
    fn turning_takes_the_short_way_round() {
        assert_eq!(turn_toward(350.0, 10.0, 5.0), 355.0);
        assert_eq!(turn_toward(10.0, 350.0, 5.0), 5.0);
        assert_eq!(turn_toward(10.0, 12.0, 5.0), 12.0);
        assert!(facing(359.0, 2.0) && !facing(0.0, 90.0));
    }
}





