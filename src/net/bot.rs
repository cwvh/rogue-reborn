//! A player for the computer to play, for trying the game alone: it wanders the level, and turns
//! on whoever it sees, fires at them in bursts, keeps its distance and now and then throws a
//! grenade. It sees what a person would on the same computer: the other players as the client
//! shows them (a little in the past), only within its field of view, and not through walls. It
//! does not cheat on its aim: it turns at a limited rate, and its aim drifts off and settles.
//!
//! `rrbot --fight` plays one over a real network.

use crate::net::client::Frame;
use crate::sim::{Input, Sim, TICK};

type V3 = [f32; 3];

/// Where on a player the bot aims: the chest, below the eyes (165) and above the belly
const CHEST: f32 = 120.0;
/// How high its feelers for walls are, above its feet
const WAIST: f32 = 90.0;
/// How close a wall ahead makes it turn
const TOO_CLOSE: f32 = 140.0;
/// The distance it keeps from someone it fights: it closes in past the first, backs off within
/// the second
const FAR: f32 = 1100.0;
const NEAR: f32 = 350.0;
/// How long it remembers where it last saw someone, and goes looking there
const MEMORY: f32 = 4.0;
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
}

impl Skill {
    pub fn easy() -> Skill {
        Skill { reaction: 0.8, aim_error: 6.0, turn_rate: 180.0, sight: 2500.0, field: 55.0, grenade_every: 40.0 }
    }

    pub fn normal() -> Skill {
        Skill { reaction: 0.45, aim_error: 3.0, turn_rate: 360.0, sight: 3500.0, field: 70.0, grenade_every: 20.0 }
    }

    pub fn hard() -> Skill {
        Skill { reaction: 0.25, aim_error: 1.2, turn_rate: 720.0, sight: 5000.0, field: 90.0, grenade_every: 12.0 }
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

/// A grenade being thrown: when it was taken out
#[derive(Clone, Copy, Debug)]
struct Throwing {
    from: f32,
}

/// Seconds after taking out a grenade that the bot holds the trigger, lets go (it flies), and
/// takes the gun out again
const CHARGE: f32 = 0.5;
const GUN_AGAIN: f32 = 1.0;

pub struct Bot {
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
    pub fn think(&mut self, sim: &Sim, me: usize, view: &Frame) -> Input {
        self.time += TICK;
        let Some(state) = sim.players.get(me) else { return Input::default() };
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

        // Who it can see: alive, near enough, within its field of view (or already its target),
        // and with nothing in between
        let mut best: Option<(f32, usize, V3)> = None;
        for &(slot, pos, ..) in &view.players {
            let Some(other) = sim.players.get(slot) else { continue };
            if slot == me || !other.active || other.respawn_at.is_some() || other.hp <= 0.0 {
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
            let dir = d.map(|c| c / dist);
            // Seen through whatever bullets go through, such as a chain fence
            if sim.world.bullet_hit(eye, dir, dist).is_some_and(|wall| wall.dist < dist - 30.0) {
                continue;
            }
            if best.is_none_or(|b| dist < b.0) {
                best = Some((dist, slot, aim));
            }
        }

        let mut input = Input { run: true, ..Input::default() };
        match best {
            Some((dist, slot, aim)) => self.fight(sim, me, eye, slot, dist, aim, &mut input),
            None => {
                self.target = None;
                self.wander(sim, me, feet, &mut input);
            }
        }
        self.reload(sim, me, &mut input);
        input.set_look(self.yaw, self.pitch);
        input
    }

    #[allow(clippy::too_many_arguments)]
    fn fight(&mut self, sim: &Sim, me: usize, eye: V3, slot: usize, dist: f32, aim: V3, input: &mut Input) {
        if self.target != Some(slot) {
            // A new target: its first aim is well off, and settles
            self.target = Some(slot);
            self.seen_since = self.time;
            let e = 4.0 * self.skill.aim_error.max(1.0);
            self.error = (self.between(-e, e), self.between(-e, e) * 0.5);
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
        self.turn_toward(true_yaw + self.error.0, true_pitch + self.error.1 + lob, self.skill.turn_rate);

        let p = &sim.players[me];
        let ready = self.time - self.seen_since >= self.skill.reaction;
        if let Some(t) = self.throwing {
            let since = self.time - t.from;
            input.fire = since < CHARGE;
            if since >= GUN_AGAIN {
                input.gun = true;
                self.throwing = None;
            }
        } else if ready && p.grenades_left > 0 && self.time >= self.next_grenade && (500.0..1500.0).contains(&dist) && self.time - self.seen_since > 1.0 {
            self.throwing = Some(Throwing { from: self.time });
            input.grenade = true;
            input.fire = true;
            self.next_grenade = self.time + self.skill.grenade_every * self.between(1.0, 1.5);
        } else if ready {
            // On target, near enough: in bursts
            let off = angle_between(self.yaw, true_yaw).abs().max((self.pitch - true_pitch).abs());
            if off < 3.0 + 1.5 * self.skill.aim_error {
                if self.time < self.burst_until {
                    input.fire = true;
                } else if self.time >= self.pause_until {
                    self.burst_until = self.time + self.between(0.15, 0.5);
                    self.pause_until = self.burst_until + self.between(0.2, 0.6);
                    input.fire = true;
                }
            }
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
            sim.world.raycast(waist, horizontal(yaw), 2000.0).unwrap_or(2000.0)
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
