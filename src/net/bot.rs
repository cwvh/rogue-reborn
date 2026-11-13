//! A computer player that explores the level and attacks opponents it sees. It fires in
//! bursts, keeps its distance and occasionally throws a grenade. It sees what a person would
//! on the same computer: the other players as the client
//! shows them (a little in the past), only within its field of view, and not through walls. It
//! does not cheat on its aim: it turns at a limited rate, and its aim drifts off and settles.
//!
//! `rrbot --fight` plays one over a real network.

use crate::net::client::{Client, Frame, Seen};
use crate::net::proto::LobbyCommand;
use crate::net::round::{Phase, RoundState};
use crate::net::tactics::{Navigation, Navigator};
use crate::sim::{Input, Sim, TICK};

type V3 = [f32; 3];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Behavior {
    Legacy,
    #[default]
    Navigate,
}

/// Perception variants used to isolate changes in the fixed lab workload.
#[derive(Clone, Copy, Debug, Default)]
pub enum Perception {
    Historical,
    Frame,
    Sight,
    #[default]
    Current,
}

#[derive(Clone, Debug, Default)]
pub struct PerceptionStats {
    pub frame_rejects: usize,
    pub dead_rejects: usize,
    pub identity_rejects: usize,
    pub live_rejects: usize,
    pub room_rejects: usize,
    pub world_rejects: usize,
}

impl Behavior {
    pub fn named(name: &str) -> Option<Self> {
        match name { "legacy" => Some(Self::Legacy), "navigate" => Some(Self::Navigate), _ => None }
    }
}

/// Where on a player the bot aims: the chest, below the eyes (165) and above the belly
pub const CHEST: f32 = 120.0;
/// How high its feelers for walls are, above its feet
const WAIST: f32 = 90.0;
/// How close a wall ahead makes it turn
const TOO_CLOSE: f32 = 140.0;
/// The distance it keeps from someone it fights: it closes in past the first, backs off within
/// the second
const FAR: f32 = 1100.0;
const NEAR: f32 = 350.0;
const CLOSE: f32 = 600.0;
const TAP_RANGE: f32 = 900.0;
/// Burst and pause lengths in seconds, each as (shortest, longest)
const CLOSE_BURST: ((f32, f32), (f32, f32)) = ((1.0, 2.0), (0.15, 0.4));
const MID_BURST: ((f32, f32), (f32, f32)) = ((0.15, 0.5), (0.2, 0.6));
const TAP_BURST: ((f32, f32), (f32, f32)) = ((0.08, 0.2), (0.3, 0.7));
/// Past CLOSE, a burst stops once the aim is this many times the start angle off
const HOLD_ANGLE: f32 = 2.0;
/// Degrees
const RECOIL_CAP: f32 = 8.0;
/// Degrees a second
const RECOIL_WANDER: f32 = 6.0;
/// How long it remembers where it last saw someone, and goes looking there
const MEMORY: f32 = 4.0;
/// Keep a local escape direction long enough to turn and move out of a tight corner.
pub const RECOVER_FOR: f32 = 1.5;
const RECOVER_DISTANCE: f32 = 80.0;
const RECOVER_STEP: f32 = 10.0;
const RECOVER_DROP: f32 = 90.0;
const RECOVER_SETTLE_TICKS: usize = 30;
const RECOVER_ANGLE: f32 = 5.0;
const RECOVER_USE_AGAIN: f32 = 0.6;
/// How far ahead it looks for the ground, and how far it will drop: farther is a ledge, like a
/// wall (rm08's docks end in nothing, and a player who walks off falls for ever)
const STEP_AHEAD: f32 = 80.0;
const MAX_DROP: f32 = 200.0;
/// Penned in (no farther than `PENNED` from where it was `BORED` seconds ago), it will drop as
/// far as `BORED_DROP` to get out, as a person would jump down from a platform
const PENNED: f32 = 500.0;
const BORED: f32 = 10.0;
const BORED_DROP: f32 = 1500.0;
/// It goes to open a closed door it sees within `DOOR_SIGHT`, presses use from within
/// `DOOR_PRESS` (the game reaches 220), and leaves a door it tried alone for `DOOR_AGAIN` seconds
const DOOR_SIGHT: f32 = 600.0;
const DOOR_PRESS: f32 = 170.0;
const DOOR_AGAIN: f32 = 30.0;

/// A bot is always ready: it says so in each lobby, and continues from each round's results at
/// once, so that no one need wait for it
pub fn say_ready(client: &mut Client) {
    let ready = client.ready.contains(&client.slot());
    if client.round.phase == Phase::Lobby && !ready && client.lobby_waiting().next().is_none() {
        client.lobby(LobbyCommand::Ready(true));
    }
    if client.may_continue() {
        client.lobby(LobbyCommand::Continue);
    }
}

/// How well a bot plays
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skill {
    /// Seconds from seeing someone to firing at them
    pub reaction: f32,
    /// How far off its aim wanders, in degrees, once it has settled on a target
    pub aim_error: f32,
    /// How fast it turns, in degrees a second
    pub turn_rate: f32,
    /// How far it sees, and how far to either side of where it looks (degrees)
    pub sight: f32,
    pub field: f32,
    /// The least time between two of its grenades, in seconds
    pub grenade_every: f32,
    /// Degrees a second while the trigger is held
    pub recoil_climb: f32,
    /// Decay rate a second once it lets go
    pub recoil_recover: f32,
}

impl Skill {
    pub fn easy() -> Skill {
        Skill { reaction: 0.8, aim_error: 6.0, turn_rate: 180.0, sight: 2500.0, field: 55.0, grenade_every: 40.0, recoil_climb: 4.5, recoil_recover: 4.0 }
    }

    pub fn normal() -> Skill {
        Skill { reaction: 0.45, aim_error: 3.0, turn_rate: 360.0, sight: 3500.0, field: 70.0, grenade_every: 20.0, recoil_climb: 3.0, recoil_recover: 8.0 }
    }

    pub fn hard() -> Skill {
        Skill { reaction: 0.25, aim_error: 1.2, turn_rate: 720.0, sight: 5000.0, field: 90.0, grenade_every: 12.0, recoil_climb: 1.5, recoil_recover: 14.0 }
    }

    /// A skill by name: easy, normal or hard
    pub fn named(name: &str) -> Option<Skill> {
        match name {
            "easy" => Some(Skill::easy()),
            "normal" => Some(Skill::normal()),
            "hard" => Some(Skill::hard()),
            _ => None,
        }
    }
}

/// A grenade being thrown, followed by a request to return to the gun.
#[derive(Clone, Copy, Debug)]
struct Throwing {
    from: f32,
    returning: bool,
}

struct Recovery {
    from: V3,
    yaw: f32,
    until: f32,
}

/// Seconds to hold the trigger before releasing a grenade.
const CHARGE: f32 = 0.5;

pub struct Bot {
    pub behavior: Behavior,
    pub navigation: Option<Navigator>,
    /// The life whose gameplay commitments are current, and whether it had died.
    life: Option<(crate::net::tactics::Life, bool)>,
    /// Seed-derived tie breaker for equally useful exploration destinations.
    preference: usize,
    /// Preserve the fixed bot workload used by the network fairness experiment.
    perception: Perception,
    pub perception_stats: Option<PerceptionStats>,
    /// A short escape commitment while the current feet cannot attach to a walking path.
    recovery: Option<Recovery>,
    /// Last use press to close a leaf pinning the body, on the bot's gameplay clock.
    recovery_use: f32,
    skill: Skill,
    rng: u64,
    /// Its own clock, in seconds
    time: f32,
    /// Where it looks, and where it means to walk when it has no one to fight (degrees)
    yaw: f32,
    pitch: f32,
    heading: f32,
    /// Who it fights, since when it has seen them, and where and when it last did
    target: Option<usize>,
    seen_since: f32,
    last_seen: Option<(f32, V3)>,
    /// How far off its aim is now, and where that is drifting to (degrees of yaw and pitch)
    error: (f32, f32),
    error_to: (f32, f32),
    /// Upward pitch from recoil, in degrees
    climb: f32,
    next_drift: f32,
    /// Firing in bursts: the trigger is held until the first, and not again before the second
    burst_until: f32,
    pause_until: f32,
    /// Which way it sidesteps, and until when
    strafe_left: bool,
    strafe_until: f32,
    /// When it next turns somewhere new on a whim
    next_whim: f32,
    /// Where it was at the last check for being stuck, and when that was
    stuck_check: (f32, V3),
    use_door: bool,
    /// Where it was when it last checked for being penned in, and when that was; and whether
    /// it is (then it drops from ledges)
    pen_check: (f32, V3),
    penned: bool,
    /// The doors it tried to open, and when
    doors_tried: Vec<(usize, f32)>,
    throwing: Option<Throwing>,
    next_grenade: f32,
    reloading: bool,
}

impl Bot {
    pub fn new(skill: Skill, seed: u64) -> Bot {
        let mut bot = Bot {
            behavior: Behavior::Navigate,
            navigation: None,
            life: None,
            preference: seed as usize,
            perception: Perception::Current,
            perception_stats: None,
            recovery: None,
            recovery_use: -RECOVER_USE_AGAIN,
            skill,
            rng: seed.wrapping_mul(0x9e3779b97f4a7c15) | 1,
            time: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            heading: 0.0,
            target: None,
            seen_since: 0.0,
            last_seen: None,
            error: (0.0, 0.0),
            error_to: (0.0, 0.0),
            climb: 0.0,
            next_drift: 0.0,
            burst_until: 0.0,
            pause_until: 0.0,
            strafe_left: false,
            strafe_until: 0.0,
            next_whim: 0.0,
            stuck_check: (0.0, [0.0; 3]),
            pen_check: (0.0, [0.0; 3]),
            penned: false,
            doors_tried: Vec::new(),
            use_door: false,
            throwing: None,
            next_grenade: 0.0,
            reloading: false,
        };
        bot.heading = bot.between(0.0, 360.0);
        bot.yaw = bot.heading;
        bot.next_grenade = bot.between(0.5, 1.0) * skill.grenade_every;
        bot
    }

    /// Who the bot is fighting, if anyone
    pub fn target(&self) -> Option<usize> {
        self.target
    }

    /// Use the fixed historical bot workload for the network fairness experiment.
    pub fn fairness_baseline(skill: Skill, seed: u64) -> Self {
        let mut bot = Self::new(skill, seed);
        bot.behavior = Behavior::Legacy;
        bot.perception = Perception::Historical;
        bot
    }

    pub fn measure_perception(&mut self, perception: Perception) {
        self.perception = perception;
        self.perception_stats = Some(PerceptionStats::default());
    }

    pub fn trace(&self) -> String {
        if self.behavior == Behavior::Legacy { return "legacy wandering".to_string(); }
        self.navigation.as_ref().map_or_else(|| "navigate fallback: no floor grid".to_string(), |n| format!(
            "{:?} goal={:?} visited={}/{} replans={} blocked={:.1}s {}{} next={:?} recovery_yaw={:?}", self.behavior,
            n.goal, n.visited.iter().filter(|v| v.is_some()).count(), n.patch_count, n.replans,
            n.blocked_seconds, if !n.has_grid { "legacy fallback: " } else if n.fallback { "local recovery: " } else { "" },
            if !n.has_grid { "empty floor grid".to_string() } else { n.reason.to_string() }, n.next_point(), self.recovery.as_ref().map(|r| r.yaw)))
    }

    fn random(&mut self) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545f4914f6cdd1d) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.random()
    }

    /// What the bot does this tick, as player `me` in `sim` (the client's copy of the game),
    /// with the other players where `view` shows them
    pub fn think(&mut self, sim: &Sim, nav: Option<&Navigation>, me: usize, view: &Frame, round: RoundState) -> Input {
        let Some(state) = sim.players.get(me) else { return Input::default() };
        let dead = state.respawn_at.is_some() || state.hp <= 0.0;
        let life = (crate::net::tactics::Life { joined: state.joined, round: round.number }, dead);
        if self.behavior != Behavior::Legacy {
            if let Some(map) = nav {
                if let Some(n) = &mut self.navigation { n.sync(map); }
                else { self.navigation = Some(Navigator::new(map)); }
            } else { self.navigation = None; }
        }
        if sim.frozen || round.phase.frozen() {
            if let Some(n) = self.navigation.as_mut() { n.cancel_search(); }
            let mut input = Input::default();
            input.set_look(self.yaw, self.pitch);
            return input;
        }
        self.time += TICK;
        if self.life != Some(life) {
            self.life = Some(life);
            self.target = None;
            self.last_seen = None;
            self.throwing = None;
            self.recovery = None;
            self.navigation = nav.filter(|_| self.behavior != Behavior::Legacy).map(Navigator::new);
        }
        if state.respawn_at.is_some() || state.hp <= 0.0 {
            // Dead: start afresh where it comes back
            self.target = None;
            self.last_seen = None;
            self.throwing = None;
            let mut input = Input::default();
            input.set_look(self.yaw, 0.0);
            return input;
        }
        let feet = state.player.pos;
        let eye = sim.eye(me);

        // Who it can see: the other side's, alive, near enough, within its field of view (or
        // already its target), and with nothing in between
        let mut best: Option<(f32, usize, V3)> = None;
        let side = sim.players.get(me).and_then(|p| p.side);
        for &Seen { slot, feet: pos, joined, dead, .. } in &view.players {
            let Some(other) = sim.players.get(slot) else { continue };
            if slot == me || !other.active {
                continue;
            }
            if side.is_some() && other.side == side {
                continue;
            }
            let aim = [pos[0], pos[1] + CHEST, pos[2]];
            let d = sub(aim, eye);
            let dist = length(d);
            if dist > self.skill.sight || dist < 1.0 {
                continue;
            }
            let off = angle_between(self.yaw, yaw_of(d)).abs();
            if off > self.skill.field && self.target != Some(slot) {
                continue;
            }
            let frame_gone = other.joined != joined || dead.is_some();
            let frame = matches!(self.perception, Perception::Frame | Perception::Current);
            let room = matches!(self.perception, Perception::Sight | Perception::Current);
            let live_gone = if !frame || self.perception_stats.is_some() { other.respawn_at.is_some() || other.hp <= 0.0 } else { false };
            if let Some(stats) = &mut self.perception_stats {
                stats.frame_rejects += usize::from(frame_gone && !live_gone);
                stats.dead_rejects += usize::from(dead.is_some() && !live_gone);
                stats.identity_rejects += usize::from(other.joined != joined && !live_gone);
                stats.live_rejects += usize::from(live_gone && !frame_gone);
                if !frame_gone && !live_gone {
                    let old = sim.world.bullet_hit(eye, d.map(|v| v / dist), dist).is_some_and(|h| h.dist < dist - 30.0);
                    let new = sim.bot_sight_reach(eye, aim) < dist - 30.0;
                    stats.room_rejects += usize::from(new && !old);
                    stats.world_rejects += usize::from(old && !new);
                }
            }
            if if frame { frame_gone } else { live_gone } { continue; }
            // Seen through whatever bullets go through, such as a chain fence
            let blocked = if !room {
                sim.world.bullet_hit(eye, d.map(|v| v / dist), dist).is_some_and(|h| h.dist < dist - 30.0)
            } else { sim.bot_sight_reach(eye, aim) < dist - 30.0 };
            if blocked {
                continue;
            }
            if best.is_none_or(|b| dist < b.0) {
                best = Some((dist, slot, aim));
            }
        }

        let mut input = Input { run: true, ..Input::default() };
        let mut door_to_use = None;
        let mut closing_door = false;
        match best {
            Some((dist, slot, aim)) => {
                self.recovery = None;
                if let Some(n) = self.navigation.as_mut() { n.interrupt(); }
                self.fight(sim, me, eye, slot, dist, aim, &mut input);
            }
            None => {
                self.target = None;
                if self.behavior == Behavior::Legacy {
                    self.wander(sim, me, feet, &mut input);
                } else {
                    let pursuit = self.last_seen.filter(|&(t, _)| self.time - t < MEMORY)
                        .map(|(_, p)| [p[0], p[1] - CHEST, p[2]]);
                    let recovering = self.recovery.as_ref().is_some_and(|r| self.time < r.until && length(sub(feet, r.from)) < RECOVER_DISTANCE)
                        && !self.navigation.as_ref().is_some_and(|n| n.recovery_finished(self.time));
                    let steering = if recovering { None } else { self.navigation.as_mut().zip(nav).and_then(|(n, map)| {
                        n.sight = self.skill.sight;
                        n.field = self.skill.field;
                        n.steer(map, sim, me, view, self.time, self.preference, pursuit)
                    }) };
                    if let Some(s) = steering {
                        door_to_use = s.use_door;
                        self.recovery = None;
                        let d = sub(s.look, if input.use_key || s.toward.is_none() { eye } else { feet });
                        self.heading = yaw_of(d);
                        self.turn_toward(self.heading, if input.use_key || s.toward.is_none() { pitch_of(d) } else { 0.0 }, self.skill.turn_rate * 0.5);
                        if let Some(to) = s.toward {
                            input.forward = angle_between(self.yaw, yaw_of(sub(to, feet))).abs() < 40.0;
                        }
                    } else if self.navigation.as_ref().is_none_or(|n| !n.has_grid) {
                        self.wander(sim, me, feet, &mut input);
                    } else if recovering || self.navigation.as_ref().is_some_and(|n| n.fallback) {
                        door_to_use = self.recover(sim, nav, me, feet, &mut input);
                        closing_door = true;
                    }
                }
            }
        }
        self.finish_grenade(sim, me, &mut input);
        self.reload(sim, me, &mut input);
        input.set_look(self.yaw, self.pitch);
        if let Some(k) = door_to_use {
            let look = crate::sim::movement::forward(input.look().0, input.look().1);
            if crate::sim::door_target(eye, [look[0], look[1], -look[2]], &sim.doors, &sim.door_states) == Some(k) {
                if closing_door {
                    if self.time - self.recovery_use >= RECOVER_USE_AGAIN {
                        input.use_key = true;
                        self.recovery_use = self.time;
                    }
                } else if self.navigation.as_mut().is_some_and(|n| n.press_door(k, self.time)) {
                    input.use_key = true;
                }
            }
        }
        input
    }

    fn recover(&mut self, sim: &Sim, nav: Option<&Navigation>, me: usize, feet: V3, input: &mut Input) -> Option<usize> {
        if self.recovery.as_ref().is_none_or(|r| self.time >= r.until || length(sub(feet, r.from)) >= RECOVER_DISTANCE) {
            let mut best = None;
            for k in 0..8 {
                let yaw = self.yaw + k as f32 * 45.0;
                let dir = horizontal(yaw);
                // Probe short moves with a copy of the body.
                let mut body = sim.players[me].player;
                for _ in 0..(RECOVER_DISTANCE / RECOVER_STEP) as usize {
                    sim.world.step(&mut body, [dir[0] * RECOVER_STEP, dir[2] * RECOVER_STEP], TICK);
                }
                // Finish a small descent before judging its landing. Reject a probe into a void.
                for _ in 0..RECOVER_SETTLE_TICKS {
                    if body.on_ground || feet[1] - body.pos[1] > RECOVER_DROP { break; }
                    sim.world.step(&mut body, [0.0; 2], TICK);
                }
                let ahead = (body.pos[0] - feet[0]) * dir[0] + (body.pos[2] - feet[2]) * dir[2];
                let attached = nav.is_some_and(|map| map.grid.attached(&sim.world, body.pos, &sim.standing_rooms(body.pos, None).0));
                if body.on_ground && body.pos[1] - feet[1] <= sim.world.params().step_up && feet[1] - body.pos[1] <= RECOVER_DROP
                    && ahead > sim.world.params().radius && best.is_none_or(|(fits, score, _)| (attached, ahead) > (fits, score)) {
                    best = Some((attached, ahead, yaw));
                }
            }
            self.recovery = best.map(|(_, _, yaw)| Recovery { from: feet, yaw, until: self.time + RECOVER_FOR });
        }
        if let Some(r) = &self.recovery {
            let yaw = r.yaw;
            self.turn_toward(yaw, 0.0, self.skill.turn_rate * 0.5);
            input.forward = angle_between(self.yaw, yaw).abs() < RECOVER_ANGLE;
        } else {
            // An open leaf can pin the body against a wall. Close the touching leaf to get free.
            let eye = sim.eye(me);
            let door = sim.doors.iter().enumerate().filter_map(|(k, d)| {
                let at = d.center(sim.door_states[k].open);
                let distance = length(sub(at, eye));
                (sim.door_states[k].open > 0.5 && distance <= DOOR_PRESS && sim.world.door_clearance(&sim.players[me].player, k) <= 1.0).then_some((k, at, distance))
            }).min_by(|a, b| a.2.total_cmp(&b.2));
            if let Some((k, at, _)) = door {
                let d = sub(at, eye);
                let yaw = yaw_of(d);
                self.turn_toward(yaw, pitch_of(d), self.skill.turn_rate * 0.5);
                if angle_between(self.yaw, yaw).abs() < 30.0 && sim.door_states[k].opening && self.time - self.recovery_use >= RECOVER_USE_AGAIN {
                    return Some(k);
                }
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn fight(&mut self, sim: &Sim, me: usize, eye: V3, slot: usize, dist: f32, aim: V3, input: &mut Input) {
        if self.target != Some(slot) {
            // A new target: its first aim is well off, and settles
            self.target = Some(slot);
            self.seen_since = self.time;
            let e = 4.0 * self.skill.aim_error.max(1.0);
            self.error = (self.between(-e, e), self.between(-e, e) * 0.5);
            self.climb = 0.0;
            self.next_drift = self.time;
        }
        self.last_seen = Some((self.time, aim));
        if self.time >= self.next_drift {
            let e = self.skill.aim_error;
            self.error_to = (self.between(-e, e), self.between(-e, e) * 0.6);
            self.next_drift = self.time + self.between(0.2, 0.5);
        }
        let settle = 1.0 - (-TICK * 3.0f32).exp();
        self.error.0 += (self.error_to.0 - self.error.0) * settle;
        self.error.1 += (self.error_to.1 - self.error.1) * settle;

        let d = sub(aim, eye);
        let (true_yaw, true_pitch) = (yaw_of(d), pitch_of(d));
        // A grenade lobbed a little high; otherwise the gun, aimed at the chest
        let lob = if self.throwing.is_some() { 8.0 + dist / 200.0 } else { 0.0 };
        self.turn_toward(true_yaw + self.error.0, true_pitch + self.error.1 + self.climb + lob, self.skill.turn_rate);

        let p = &sim.players[me];
        let ready = self.time - self.seen_since >= self.skill.reaction;
        if self.throwing.is_none() {
            if ready && p.reload_until.is_none() && p.throw.is_none() && p.grenades_left > 0 && self.time >= self.next_grenade && (500.0..1500.0).contains(&dist) && self.time - self.seen_since > 1.0 {
                self.throwing = Some(Throwing { from: self.time, returning: false });
                input.grenade = true;
                self.next_grenade = self.time + self.skill.grenade_every * self.between(1.0, 1.5);
            } else if ready {
                // On target, near enough: in bursts
                let off = angle_between(self.yaw, true_yaw).abs().max((self.pitch - true_pitch).abs());
                let start = 3.0 + 1.5 * self.skill.aim_error;
                if self.time < self.burst_until {
                    // Up close a burst sprays to its end. Farther out it outlasts a target crossing.
                    input.fire = dist < CLOSE || off < start * HOLD_ANGLE;
                } else if off < start && self.time >= self.pause_until {
                    let (burst, pause) = if dist < CLOSE { CLOSE_BURST } else if dist > TAP_RANGE { TAP_BURST } else { MID_BURST };
                    self.burst_until = self.time + self.between(burst.0, burst.1);
                    self.pause_until = self.burst_until + self.between(pause.0, pause.1);
                    input.fire = true;
                }
            }
        }

        if input.fire && self.time < self.burst_until {
            self.climb = (self.climb + self.skill.recoil_climb * TICK).min(RECOIL_CAP);
            self.error.0 += self.between(-1.0, 1.0) * RECOIL_WANDER * TICK;
        } else {
            self.climb *= (-self.skill.recoil_recover * TICK).exp();
        }

        // Sidestep, and keep a middling distance, but not off a ledge
        if self.time >= self.strafe_until {
            self.strafe_left = self.random() < 0.5;
            self.strafe_until = self.time + self.between(0.6, 1.5);
        }
        let feet = p.player.pos;
        let side = |left: bool| self.yaw + if left { 90.0 } else { -90.0 };
        let drop = self.drop();
        if !ground_ahead(sim, feet, side(self.strafe_left), drop) {
            self.strafe_left = !self.strafe_left;
        }
        let can_strafe = ground_ahead(sim, feet, side(self.strafe_left), drop);
        input.left = can_strafe && self.strafe_left;
        input.right = can_strafe && !self.strafe_left;
        input.forward = dist > FAR && ground_ahead(sim, feet, self.yaw, drop);
        input.back = dist < NEAR && ground_ahead(sim, feet, self.yaw + 180.0, drop);
        self.heading = self.yaw;
    }

    fn wander(&mut self, sim: &Sim, me: usize, feet: V3, input: &mut Input) {
        let waist = [feet[0], feet[1] + WAIST, feet[2]];
        // Someone seen a moment ago: go and look where they were
        let mut to_door = None;
        let recent = self.last_seen.filter(|&(when, _)| self.time - when < MEMORY);
        if let Some((_, at)) = recent {
            let d = sub(at, waist);
            if length([d[0], 0.0, d[2]]) < 60.0 {
                self.last_seen = None;
            } else {
                self.heading = yaw_of(d);
            }
        } else if let Some(door) = self.go_to_door(sim, me, input) {
            to_door = Some(door);
        } else if self.time >= self.next_whim {
            self.heading += self.between(-100.0, 100.0);
            self.next_whim = self.time + self.between(2.0, 6.0);
        }
        // Feelers: turn away from a wall or a ledge ahead, toward the side with more room
        self.check_penned(feet);
        let drop = self.drop();
        let room = |yaw: f32| {
            if !ground_ahead(sim, feet, yaw, drop) {
                return 0.0;
            }
            sim.world.body_raycast(waist, horizontal(yaw), 2000.0).unwrap_or(2000.0)
        };
        // (not walking up to a door: the door is the wall ahead)
        if to_door.is_none() && room(self.heading) < TOO_CLOSE {
            let (left, right) = (room(self.heading + 60.0), room(self.heading - 60.0));
            let turn = self.between(60.0, 130.0);
            self.heading += if left > right { turn } else { -turn };
        }
        // Stuck (a corner, a closed door): try the door, and turn around
        self.use_door = false;
        if self.time - self.stuck_check.0 >= 1.0 {
            let moved = length(sub(feet, self.stuck_check.1));
            if moved < 30.0 && self.stuck_check.0 > 0.0 {
                self.heading += 180.0 + self.between(-60.0, 60.0);
                self.use_door = true;
                // A door it cannot get to (behind a railing, a hatch overhead): leave it be
                if let Some(door) = to_door {
                    self.doors_tried.push((door, self.time));
                }
            }
            self.stuck_check = (self.time, feet);
        }
        input.use_key |= self.use_door && sim.players[me].controlled;
        self.heading = self.heading.rem_euclid(360.0);
        self.turn_toward(self.heading, 0.0, self.skill.turn_rate * 0.5);
        input.forward = angle_between(self.yaw, self.heading).abs() < 45.0 && ground_ahead(sim, feet, self.yaw, drop);
    }

    /// Head for a closed door in sight, and open it once there. Which door, if any.
    fn go_to_door(&mut self, sim: &Sim, me: usize, input: &mut Input) -> Option<usize> {
        self.last_seen = None;
        let eye = sim.eye(me);
        let now = self.time;
        self.doors_tried.retain(|&(_, when)| now - when < DOOR_AGAIN);
        let mut best: Option<(f32, usize, V3)> = None;
        for (i, door) in sim.doors.iter().enumerate() {
            if !sim.door_states.get(i).is_some_and(|s| s.closed()) || self.doors_tried.iter().any(|&(d, _)| d == i) {
                continue;
            }
            let c = door.center(0.0);
            let d = sub(c, eye);
            let dist = length(d);
            if dist > DOOR_SIGHT || dist < 1.0 {
                continue;
            }
            // In sight: nothing in the way short of the door itself
            if sim.world.raycast(eye, d.map(|x| x / dist), dist).is_some_and(|wall| wall < dist - 60.0) {
                continue;
            }
            if best.is_none_or(|b| dist < b.0) {
                best = Some((dist, i, c));
            }
        }
        let (dist, i, c) = best?;
        let d = sub(c, eye);
        self.heading = yaw_of(d);
        if dist < DOOR_PRESS && angle_between(self.yaw, self.heading).abs() < 30.0 {
            input.use_key = true;
            self.doors_tried.push((i, now));
        }
        Some(i)
    }

    /// How far it will drop from a ledge
    fn drop(&self) -> f32 {
        if self.penned { BORED_DROP } else { MAX_DROP }
    }

    fn check_penned(&mut self, feet: V3) {
        if self.time - self.pen_check.0 < BORED {
            if self.penned && length(sub(feet, self.pen_check.1)) > PENNED {
                self.penned = false;
            }
            return;
        }
        self.penned = self.pen_check.0 > 0.0 && length(sub(feet, self.pen_check.1)) < PENNED;
        self.pen_check = (self.time, feet);
    }

    fn finish_grenade(&mut self, sim: &Sim, me: usize, input: &mut Input) {
        let Some(t) = &mut self.throwing else { return };
        let p = &sim.players[me];
        // A gun request before the swing starts cancels the release. Once the swing is seen,
        // keep requesting the gun until our equipment confirms that the switch succeeded.
        t.returning |= p.throw.is_some();
        if t.returning {
            input.fire = false;
            input.gun = p.equipped_grenade;
            if !p.equipped_grenade { self.throwing = None; }
        } else {
            input.fire = self.time - t.from < CHARGE;
        }
    }

    /// Reload when empty, or when there is time and the magazine is low
    fn reload(&mut self, sim: &Sim, me: usize, input: &mut Input) {
        let p = &sim.players[me];
        if p.reload_until.is_some() {
            self.reloading = false;
            return;
        }
        let low = p.ammo == 0 || (self.target.is_none() && p.ammo < sim.magazine / 3);
        if low && p.reserve > 0 && !self.reloading && self.throwing.is_none() {
            input.reload = true;
            self.reloading = true;
        } else if !low {
            self.reloading = false;
        }
    }

    fn turn_toward(&mut self, yaw: f32, pitch: f32, rate: f32) {
        let step = rate * TICK;
        let dy = angle_between(self.yaw, yaw);
        self.yaw = (self.yaw + dy.clamp(-step, step)).rem_euclid(360.0);
        let dp = pitch.clamp(-80.0, 80.0) - self.pitch;
        self.pitch += dp.clamp(-step, step);
    }
}

/// Whether there is ground a step ahead along `yaw`, no more than `drop` below the feet
fn ground_ahead(sim: &Sim, feet: V3, yaw: f32, drop: f32) -> bool {
    let d = horizontal(yaw);
    let above = [feet[0] + d[0] * STEP_AHEAD, feet[1] + WAIST, feet[2] + d[2] * STEP_AHEAD];
    sim.world.raycast(above, [0.0, -1.0, 0.0], WAIST + drop).is_some()
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(a: V3) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

/// The look (`Input::set_look`) that faces along a direction in map space: a shot goes along
/// (-sin yaw cos pitch, sin pitch, cos yaw cos pitch)
fn yaw_of(d: V3) -> f32 {
    (-d[0]).atan2(d[2]).to_degrees().rem_euclid(360.0)
}

fn pitch_of(d: V3) -> f32 {
    (d[1] / length(d).max(1e-6)).clamp(-1.0, 1.0).asin().to_degrees()
}

/// The direction along the ground a look of this yaw faces
fn horizontal(yaw: f32) -> V3 {
    let y = yaw.to_radians();
    [-y.sin(), 0.0, y.cos()]
}

/// The turn from one yaw to another, the short way round (-180 to 180)
fn angle_between(from: f32, to: f32) -> f32 {
    (to - from + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bot_sees_and_shoots_an_opponent_through_intact_glass() {
        if !std::path::Path::new("data/map/rm06/rm06.map").exists() { return; }
        let setup = crate::net::proto::Setup { map: "data/map/rm06/rm06.map".to_string(), grenades: 0, pvp: true,
            ..crate::net::lab::demo_setup() };
        let mut sim = crate::net::setup::build_sim(&setup).unwrap();
        let (from, to) = ([33227.0, 32767.0, 31457.0], [33227.0, 32767.0, 31617.0]);
        sim.add_player(from);
        sim.add_player(to);
        sim.tick(&[Input::default(); 2], TICK);
        let pane = sim.panes.iter().position(|p| p.name == "15_glass02").unwrap();
        let eye = sim.eye(0);
        let aim = [to[0], to[1] + CHEST, to[2]];
        let distance = length(sub(aim, eye));
        assert!(sim.line_reach(eye, aim) < distance - 30.0, "the watch camera stops at glass");
        assert!(sim.bot_sight_reach(eye, aim) >= distance - 1.0);
        assert!(sim.broken_at[pane].is_none());
        let mut bot = Bot::new(Skill { reaction: 0.0, aim_error: 0.0, ..Skill::hard() }, 42);
        let mut baseline = Bot::fairness_baseline(Skill::hard(), 42);
        bot.yaw = yaw_of(sub(aim, eye));
        baseline.yaw = bot.yaw;
        let view = Frame { players: vec![Seen { slot: 1, feet: to, look: (0, 0), dead: None,
            crouched: false, peek: None, joined: sim.players[1].joined }], ..Frame::default() };
        baseline.think(&sim, None, 0, &view, RoundState::default());
        assert_eq!(baseline.target(), None);
        let mut shattered = false;
        let mut hit = false;
        let mut shots = Vec::new();
        let hp = sim.players[1].hp;
        for _ in 0..120 {
            let mut input = bot.think(&sim, None, 0, &view, RoundState::default());
            assert_eq!(bot.target(), Some(1));
            input.forward = false;
            input.back = false;
            input.left = false;
            input.right = false;
            for event in sim.tick(&[input, Input::default()], TICK) {
                shattered |= matches!(event, crate::sim::Event::Glass { pane: p, .. } if p == pane);
                hit |= matches!(event, crate::sim::Event::Shot { player: 0, hit_character: true, .. });
                if matches!(event, crate::sim::Event::Shot { .. }) { shots.push(event); }
            }
            if shattered && hit { break; }
        }
        assert!(shattered && hit && sim.players[1].hp < hp, "shattered={shattered} hit={hit} hp={} shots={shots:?}", sim.players[1].hp);
        assert!(sim.broken_at[pane].is_some());
        assert!(sim.line_reach(eye, aim) >= distance - 1.0);
        let forward = Input { forward: true, ..Input::default() };
        for _ in 0..120 {
            sim.tick(&[forward, Input::default()], TICK);
            if sim.players[0].player.pos[2] >= 31580.0 { break; }
        }
        assert!(sim.players[0].player.pos[2] >= 31580.0, "the broken low window must be walkable");
    }

    fn ahead(eye: V3, dist: f32) -> V3 {
        let h = horizontal(0.0);
        [eye[0] + h[0] * dist, eye[1] + h[1] * dist, eye[2] + h[2] * dist]
    }

    fn trigger_runs(dist: f32, seconds: f32, seed: u64) -> (usize, usize) {
        let mut sim = crate::net::setup::build_sim(&crate::net::lab::demo_setup()).unwrap();
        sim.add_player([0.0, 0.0, 0.0]);
        sim.add_player([0.0, 0.0, 400.0]);
        sim.tick(&[Input::default(); 2], TICK);
        let mut bot = Bot::new(Skill { reaction: 0.0, aim_error: 0.0, ..Skill::hard() }, seed);
        bot.yaw = 0.0;
        let eye = sim.eye(0);
        let aim = ahead(eye, dist);
        let (mut run, mut longest, mut held) = (0, 0, 0);
        for _ in 0..(seconds / TICK) as usize {
            bot.time += TICK;
            let mut input = Input::default();
            bot.fight(&sim, 0, eye, 1, dist, aim, &mut input);
            if input.fire { run += 1; held += 1; longest = longest.max(run); } else { run = 0; }
        }
        (longest, held)
    }

    #[test]
    fn up_close_a_bot_holds_the_trigger_in_long_bursts() {
        for seed in 1..6 {
            let (longest, _) = trigger_runs(400.0, 10.0, seed);
            assert!(longest as f32 * TICK >= 1.0 - 2.0 * TICK, "seed {seed}: longest burst {longest} ticks");
        }
    }

    #[test]
    fn up_close_a_burst_sprays_on_far_off_target_but_farther_out_it_stops() {
        for (dist, holds) in [(400.0, true), (700.0, false)] {
            let mut sim = crate::net::setup::build_sim(&crate::net::lab::demo_setup()).unwrap();
            sim.add_player([0.0, 0.0, 0.0]);
            sim.add_player([0.0, 0.0, 400.0]);
            sim.tick(&[Input::default(); 2], TICK);
            let mut bot = Bot::new(Skill { reaction: 0.0, aim_error: 0.0, ..Skill::hard() }, 7);
            let eye = sim.eye(0);
            let on = ahead(eye, dist);
            bot.time = 1.0;
            bot.target = Some(1);
            bot.yaw = 0.0;
            bot.next_drift = 100.0;
            let mut input = Input::default();
            bot.fight(&sim, 0, eye, 1, dist, on, &mut input);
            assert!(input.fire);
            // Far outside the hold angle
            bot.yaw = 30.0;
            bot.skill.turn_rate = 0.0;
            bot.time += TICK;
            let mut input = Input::default();
            bot.fight(&sim, 0, eye, 1, dist, on, &mut input);
            assert_eq!(input.fire, holds, "at {dist}");
        }
    }

    #[test]
    fn far_away_a_bot_only_taps() {
        for seed in 1..6 {
            let (longest, held) = trigger_runs(1000.0, 10.0, seed);
            assert!(longest as f32 * TICK <= 0.2 + 2.0 * TICK, "seed {seed}: longest burst {longest} ticks");
            assert!(held > 0);
        }
    }

    #[test]
    fn a_burst_survives_a_target_crossing_but_does_not_start_off_target() {
        let mut sim = crate::net::setup::build_sim(&crate::net::lab::demo_setup()).unwrap();
        sim.add_player([0.0, 0.0, 0.0]);
        sim.add_player([0.0, 0.0, 400.0]);
        sim.tick(&[Input::default(); 2], TICK);
        let mut bot = Bot::new(Skill { reaction: 0.0, aim_error: 0.0, ..Skill::hard() }, 7);
        let eye = sim.eye(0);
        let on = ahead(eye, 400.0);
        let mut input = Input::default();
        bot.time = 1.0;
        bot.target = Some(1);
        bot.yaw = 0.0;
        bot.next_drift = 100.0;
        bot.fight(&sim, 0, eye, 1, 400.0, on, &mut input);
        assert!(input.fire);
        // Between the start angle and the hold angle
        bot.yaw = 5.0;
        bot.skill.turn_rate = 0.0;
        bot.time += TICK;
        let mut input = Input::default();
        bot.fight(&sim, 0, eye, 1, 400.0, on, &mut input);
        assert!(input.fire, "the burst should hold");
        bot.burst_until = 0.0;
        bot.pause_until = 0.0;
        let mut input = Input::default();
        bot.fight(&sim, 0, eye, 1, 400.0, on, &mut input);
        assert!(!input.fire, "a new burst should not start 5 degrees off");
    }

    fn burst_pitch(skill: Skill, seconds: f32) -> (f32, f32) {
        let mut sim = crate::net::setup::build_sim(&crate::net::lab::demo_setup()).unwrap();
        sim.add_player([0.0, 0.0, 0.0]);
        sim.add_player([0.0, 0.0, 400.0]);
        sim.tick(&[Input::default(); 2], TICK);
        let mut bot = Bot::new(Skill { reaction: 0.0, aim_error: 0.0, ..skill }, 7);
        let eye = sim.eye(0);
        let on = ahead(eye, 400.0);
        bot.time = 1.0;
        bot.target = Some(1);
        bot.yaw = 0.0;
        bot.next_drift = 1000.0;
        let mut input = Input::default();
        bot.fight(&sim, 0, eye, 1, 400.0, on, &mut input);
        assert!(input.fire);
        let start = bot.pitch;
        bot.burst_until = bot.time + seconds;
        bot.pause_until = bot.burst_until + 0.3;
        let (burst_end, pause_end) = (bot.burst_until, bot.pause_until);
        while bot.time < burst_end {
            bot.time += TICK;
            bot.fight(&sim, 0, eye, 1, 400.0, on, &mut Input::default());
        }
        let climbed = bot.pitch - start;
        while bot.time < pause_end - TICK {
            bot.time += TICK;
            bot.fight(&sim, 0, eye, 1, 400.0, on, &mut Input::default());
        }
        (climbed, bot.pitch - start)
    }

    #[test]
    fn a_long_burst_climbs_and_the_pause_brings_the_aim_back() {
        let (climbed, settled) = burst_pitch(Skill::normal(), 1.5);
        assert!((3.0..6.0).contains(&climbed), "climbed {climbed}");
        assert!(settled.abs() < 1.0, "settled {settled}");
    }

    #[test]
    fn a_hard_bot_climbs_less_than_an_easy_one() {
        let (easy, _) = burst_pitch(Skill::easy(), 1.5);
        let (hard, _) = burst_pitch(Skill::hard(), 1.5);
        assert!(hard < easy, "hard {hard}, easy {easy}");
    }

    fn grenade_returns_to_gun(behavior: Behavior, lose_during: Option<bool>, reloading: bool) {
        let mut sim = crate::net::setup::build_sim(&crate::net::lab::demo_setup()).unwrap();
        sim.add_player([0.0, 0.0, -400.0]);
        sim.add_player([0.0, 0.0, 400.0]);
        sim.tick(&[Input::default(); 2], TICK);
        if reloading { sim.players[0].reload_until = Some(sim.time + 2.0); }
        let mut bot = Bot::new(Skill::normal(), 42);
        bot.behavior = behavior;
        bot.skill.field = 180.0;
        bot.next_grenade = 0.0;
        bot.yaw = 0.0;
        let (mut lost, mut throws, mut shots_after, mut retry_after_swing) = (false, 0, 0, false);
        for tick in 0..600 {
            let own = &sim.players[0];
            if let Some(swing) = lose_during {
                lost |= if swing { own.throw.is_some() } else { own.charge_from.is_some() };
            }
            let view = if lost && tick < 300 { Frame::default() } else {
                Frame { players: vec![Seen { slot: 1, feet: sim.players[1].player.pos, look: (0, 0),
                    dead: None, crouched: false, peek: None, joined: sim.players[1].joined }], ..Frame::default() }
            };
            let mut input = bot.think(&sim, None, 0, &view, RoundState::default());
            assert!(!(input.gun && own.charge_from.is_some() && own.throw.is_none()), "gun canceled the release");
            retry_after_swing |= input.gun && own.throw.is_none() && own.equipped_grenade;
            // Hold both players in place to exercise combat at 800 units for the full ten seconds.
            input.forward = false;
            input.back = false;
            input.left = false;
            input.right = false;
            let events = sim.tick(&[input, Input::default()], TICK);
            for event in events {
                match event {
                    crate::sim::Event::Log(line) if line.contains("threw a grenade") => throws += 1,
                    crate::sim::Event::Shot { player: 0, .. } if throws > 0 => shots_after += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(throws, 1, "{behavior:?}, lost during={lose_during:?}");
        assert_eq!(sim.players[0].grenades_left, 2);
        assert!(!sim.players[0].equipped_grenade);
        assert!(bot.throwing.is_none());
        assert!(retry_after_swing && shots_after > 0);
        assert!(lose_during.is_none() || lost);
    }

    #[test]
    fn a_grenade_returns_to_the_gun_before_the_next_burst() {
        for behavior in [Behavior::Navigate, Behavior::Legacy] { grenade_returns_to_gun(behavior, None, false); }
    }

    #[test]
    fn a_grenade_finishes_after_losing_the_target_during_charge_or_swing() {
        for behavior in [Behavior::Navigate, Behavior::Legacy] {
            for swing in [false, true] { grenade_returns_to_gun(behavior, Some(swing), false); }
        }
    }

    #[test]
    fn a_grenade_waits_until_reloading_finishes() {
        for behavior in [Behavior::Navigate, Behavior::Legacy] { grenade_returns_to_gun(behavior, None, true); }
    }

    #[test]
    fn the_first_held_tick_accepts_navigation_without_advancing_gameplay() {
        let mut sim = crate::net::setup::build_sim(&crate::net::lab::demo_setup()).unwrap();
        sim.add_player([0.0; 3]);
        let map = Navigation::for_map(&sim, "demo", None);
        let mut bot = Bot::new(Skill::normal(), 42);
        let mut expected = Input::default();
        expected.set_look(bot.yaw, bot.pitch);
        assert_eq!(bot.think(&sim, Some(&map), 0, &Frame::default(), RoundState { phase: Phase::Hold, ..RoundState::default() }), expected);
        assert_eq!(bot.time, 0.0);
        let nav = bot.navigation.as_ref().unwrap();
        assert!(nav.has_grid);
        assert!(nav.visited.iter().all(Option::is_none));
        assert!(nav.goal.is_none());
    }

    #[test]
    fn hidden_snapshot_changes_do_not_change_decisions() {
        let mut a = crate::net::setup::build_sim(&crate::net::lab::demo_setup()).unwrap();
        let mut b = crate::net::setup::build_sim(&crate::net::lab::demo_setup()).unwrap();
        for sim in [&mut a, &mut b] {
            sim.add_player([0.0; 3]);
            sim.add_player([0.0, 0.0, -300.0]);
            sim.tick(&[Input::default(); 2], TICK);
        }
        let seen = |sim: &Sim, feet| Frame { players: vec![Seen { slot: 1, feet, look: (0, 0),
            dead: None, crouched: false, peek: None, joined: sim.players[1].joined }], ..Frame::default() };
        let va = seen(&a, [0.0, 0.0, -300.0]);
        let vb = seen(&b, [100.0, 0.0, -300.0]);
        b.players[1].player.pos = [9999.0, 400.0, 9999.0];
        b.players[1].hp = 0.0;
        let (mut ba, mut bb) = (Bot::new(Skill::normal(), 73), Bot::new(Skill::normal(), 73));
        // These delayed bodies remain behind the bot's field throughout this decision.
        ba.yaw = 0.0;
        bb.yaw = 0.0;
        assert_eq!(ba.think(&a, None, 0, &va, RoundState::default()), bb.think(&b, None, 0, &vb, RoundState::default()));
        assert_eq!(ba.rng, bb.rng);
        assert_eq!(ba.trace(), bb.trace());
        let visible = seen(&a, [0.0, 0.0, 300.0]);
        let (mut ba, mut bb) = (Bot::new(Skill::normal(), 73), Bot::new(Skill::normal(), 73));
        ba.yaw = 0.0;
        bb.yaw = 0.0;
        assert_eq!(ba.think(&a, None, 0, &visible, RoundState::default()), bb.think(&b, None, 0, &visible, RoundState::default()));
        assert_eq!(ba.target(), Some(1));
        assert_eq!(bb.target(), Some(1));
    }

    #[test]
    fn a_live_respawn_does_not_make_its_displayed_dead_body_a_target() {
        let mut sim = crate::net::setup::build_sim(&crate::net::lab::demo_setup()).unwrap();
        sim.add_player([0.0; 3]);
        sim.add_player([0.0, 0.0, 300.0]);
        for dead in [Some(1.0), None] {
            let shown = Seen { slot: 1, feet: [0.0, 0.0, 300.0], look: (0, 0), dead,
                crouched: false, peek: None, joined: sim.players[1].joined + u32::from(dead.is_none()) };
            let view = Frame { players: vec![shown], ..Frame::default() };
            let mut current = Bot::new(Skill::normal(), 73);
            let mut historical = Bot::fairness_baseline(Skill::normal(), 73);
            current.yaw = 0.0;
            historical.yaw = 0.0;
            current.think(&sim, None, 0, &view, RoundState::default());
            historical.think(&sim, None, 0, &view, RoundState::default());
            assert_eq!(current.target(), None);
            assert_eq!(historical.target(), Some(1));
        }
    }

    #[test]
    fn holding_pauses_the_clock_and_life_boundaries_reset_navigation() {
        let mut sim = crate::net::setup::build_sim(&crate::net::lab::demo_setup()).unwrap();
        sim.add_player([0.0; 3]);
        sim.tick(&[Input::default()], TICK);
        let map = Navigation::for_map(&sim, "demo", None);
        let mut bot = Bot::new(Skill::normal(), 42);
        bot.think(&sim, Some(&map), 0, &Frame::default(), RoundState::default());
        bot.navigation.as_mut().unwrap().visited.fill(Some(0.0));
        let time = bot.time;
        sim.frozen = true;
        let mut held = Input::default();
        held.set_look(bot.yaw, bot.pitch);
        for _ in 0..180 { assert_eq!(bot.think(&sim, Some(&map), 0, &Frame::default(), RoundState::default()), held); }
        assert_eq!(bot.time, time);
        sim.frozen = false;
        for boundary in 0..3 {
            match boundary {
                0 => sim.players[0].hp = 0.0,
                1 => sim.players[0].hp = 100.0,
                _ => sim.players[0].joined += 1,
            }
            bot.think(&sim, Some(&map), 0, &Frame::default(), RoundState::default());
            assert!(bot.navigation.as_ref().unwrap().visited.iter().filter(|v| v.is_some()).count() <= 1);
            bot.navigation.as_mut().unwrap().visited.fill(Some(0.0));
        }
        bot.think(&sim, Some(&map), 0, &Frame::default(), RoundState { number: 2, phase: Phase::Live, ..RoundState::default() });
        assert!(bot.navigation.as_ref().unwrap().visited.iter().filter(|v| v.is_some()).count() <= 1);
    }

    #[test]
    fn a_look_toward_a_direction_shoots_along_it() {
        for d in [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [-3.0, 1.0, -2.0], [0.5, -0.5, 2.0]] {
            let (yaw, pitch) = (yaw_of(d), pitch_of(d));
            let mut input = Input::default();
            input.set_look(yaw, pitch);
            let (y, p) = input.look();
            let f = crate::sim::movement::forward(y, p);
            let shot = [f[0], f[1], -f[2]]; // as Sim::shoot turns it into map space
            let n = length(d);
            for k in 0..3 {
                assert!((shot[k] - d[k] / n).abs() < 1e-3, "{d:?}: {shot:?}");
            }
        }
    }

    #[test]
    fn turning_takes_the_short_way_round() {
        assert_eq!(angle_between(350.0, 10.0), 20.0);
        assert_eq!(angle_between(10.0, 350.0), -20.0);
    }
}
