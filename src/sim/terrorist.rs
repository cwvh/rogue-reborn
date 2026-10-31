//! What a mission's terrorists do about the players: see them, turn to them and fire, through
//! the same spread as players (sim/aim.rs). How they see and when they pull the trigger are the
//! original's (docs/terrorist-ai.md); the values it sets at run time, and how fast they turn,
//! are guesses here.

use crate::mission::{Alertness, Stance};
use crate::sim::aim::{Aim, Gun, Pace};

type V3 = [f32; 3];

/// The normal view angle, the full width (RogueSpear.exe 0x8169f8, set at run time: a guess)
pub const NORMAL_VIEW: f32 = 90.0;
/// Within this they see a player whichever way they face (0x8169a0, set at run time: a guess)
pub const CLOSE: f32 = 200.0;
/// Degrees a second they turn toward a target (not read yet: a guess)
pub const TURN_RATE: f32 = 180.0;
/// Seconds they keep to a target they no longer see
pub const FORGET: f32 = 5.0;
/// How near their facing must be to the target before they fire, in degrees
const FACING: f32 = 5.0;

/// What a terrorist knows and holds, which only the game's own copy keeps
#[derive(Clone, Debug)]
pub struct Mind {
    pub alertness: Alertness,
    /// Their Assault skill, out of 1
    pub skill: f32,
    /// The mission's Max LOS Distance
    pub sight: f32,
    /// The player they are after, and when they last saw them
    pub target: Option<usize>,
    pub seen_at: f32,
    pub aim: Aim,
    pub gun: Gun,
    pub ammo: u32,
    /// Full magazines left
    pub magazines: u32,
    pub magazine: u32,
    pub reload_until: Option<f32>,
    pub next_shot: f32,
}

impl Mind {
    pub fn new(alertness: Alertness, skill: f32, sight: f32, gun: Gun, magazine: u32, magazines: u32) -> Mind {
        Mind {
            alertness, skill, sight, target: None, seen_at: 0.0, aim: Aim::default(), gun,
            ammo: magazine, magazines: magazines.saturating_sub(1), magazine, reload_until: None, next_shot: 0.0,
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
    /// there is at most `15 - 7s` units across, or it has nearly closed, and never over 180.
    /// Not before the spread has opened at all.
    pub fn would_fire(&self, dist: f32) -> bool {
        if !self.aim.started {
            return false;
        }
        let across = self.aim.angle * dist;
        let time = self.gun.close * 0.5;
        let left = 1.0 - (self.aim.since / time).min(1.0);
        (across <= 15.0 - 7.0 * self.skill || left * left <= 0.15 - 0.14 * self.skill) && across <= 180.0
    }

    /// Their spread, `dt` on, holding the trigger or not, facing `yaw` degrees
    pub fn step_aim(&mut self, crouched: bool, wounded: bool, trigger: bool, yaw: f32, dt: f32) {
        let gun = self.gun;
        self.aim.step(&gun, self.skill, Pace::Stopped, crouched, wounded, trigger, super::quantize(yaw), dt);
    }
}

impl super::Sim {
    /// Each armed terrorist sees the players it can, turns toward the one it is after, and
    /// fires when its spread lets it; it stops to flinch and reloads when its magazine is empty
    pub(super) fn terrorists(&mut self, dt: f32, ev: &mut Vec<super::Event>) {
        let now = self.time;
        let params = self.world.params();
        let (stand_eye, kneel_eye) = (params.eye_height, params.crouch_eye_height);
        for i in 0..self.npcs.len() {
            let npc = &self.npcs[i];
            let Some(mut mind) = npc.mind.clone() else { continue };
            if npc.health.is_dead() {
                self.npcs[i].aiming = false;
                continue;
            }
            let kneel = kneels(npc.stance);
            let eye = [npc.pos[0], npc.floor + if kneel { kneel_eye } else { stand_eye }, npc.pos[2]];

            // The players it sees: where it would aim at each, and how far that is
            let (_, range) = mind.view();
            let mut sees: Vec<(usize, V3, f32)> = Vec::new();
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
                let clear = |to: V3| {
                    let d = super::dist3(eye, to);
                    let dir = [(to[0] - eye[0]) / d, (to[1] - eye[1]) / d, (to[2] - eye[2]) / d];
                    self.world.bullet_hit(eye, dir, d).is_none_or(|h| h.dist >= d - 1.0)
                };
                if looks && (clear(body) || clear(head)) {
                    sees.push((slot, aim_at, dist));
                }
            }
            // The one it is after while it sees them, else the nearest
            let seen = sees.iter().find(|s| mind.target == Some(s.0)).or_else(|| sees.iter().min_by(|a, b| a.2.total_cmp(&b.2))).copied();
            match seen {
                Some((slot, _, _)) => {
                    if mind.target != Some(slot) {
                        // A new target: its aim opens afresh
                        mind.aim = Aim::default();
                    }
                    mind.target = Some(slot);
                    mind.seen_at = now;
                    mind.alertness = Alertness::Alert;
                }
                None => {
                    let gone = mind.target.is_some_and(|t| self.players.get(t).is_none_or(|p| !p.active || p.respawn_at.is_some()));
                    if gone || now - mind.seen_at > FORGET {
                        mind.target = None;
                    }
                }
            }
            let npc = &mut self.npcs[i];
            npc.aiming = mind.target.is_some();
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
            // Turn toward the target, and fire when the spread lets it
            let mut fire = None;
            if let Some((_, to, dist)) = seen {
                let want = (to[0] - eye[0]).atan2(to[2] - eye[2]).to_degrees();
                npc.yaw = turn_toward(npc.yaw, want, TURN_RATE * dt);
                let ready = mind.ammo > 0 && mind.reload_until.is_none();
                fire = (ready && facing(npc.yaw, want) && mind.would_fire(dist)).then_some(to);
            }
            let wounded = npc.health.hp < crate::combat::Health::new().hp;
            mind.step_aim(kneel, wounded, fire.is_some(), npc.yaw, dt);
            let shot = fire.filter(|_| now >= mind.next_shot).map(|to| {
                let weapon = npc.weapon.and_then(|w| self.characters.as_ref()?.weapons.get(w)).map(|(_, w)| w);
                mind.ammo -= 1;
                mind.next_shot = now + weapon.map_or(0.1, |w| w.interval());
                if mind.ammo == 0 && mind.magazines > 0 {
                    mind.reload_until = Some(now + super::RELOAD_SECONDS);
                }
                npc.last_shot = now;
                (to, weapon.and_then(|w| w.sound.clone()))
            });
            let npc_pos = npc.pos;
            if let Some((to, sound)) = shot {
                let mut u = [0.0; 3];
                for v in &mut u {
                    *v = (super::next_rng(&mut self.rng) >> 40) as f32 / (1u64 << 24) as f32;
                }
                let (off, round) = mind.aim.throw(u);
                mind.aim.fired();
                let d = super::dist3(eye, to).max(1e-3);
                let dir = super::deflect([(to[0] - eye[0]) / d, (to[1] - eye[1]) / d, (to[2] - eye[2]) / d], off, round);
                if let Some(name) = sound {
                    ev.push(super::Event::Sound { name, at: Some(npc_pos), gain: 1.0 });
                }
                self.npcs[i].mind = Some(mind);
                self.bullet(eye, dir, super::Shooter::Character(i), ev);
            } else {
                self.npcs[i].mind = Some(mind);
            }
        }
    }
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

/// Whether `yaw` is within a few degrees of `want`
pub fn facing(yaw: f32, want: f32) -> bool {
    ((want - yaw + 540.0).rem_euclid(360.0) - 180.0).abs() <= FACING
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::aim::MP5K;

    fn mind(alertness: Alertness) -> Mind {
        Mind::new(alertness, 0.25, 4000.0, MP5K, 30, 5)
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
    fn standoff(at: V3, yaw: f32) -> crate::sim::Sim {
        use crate::collision::{CollisionWorld, Params};
        let level = std::sync::Arc::new(crate::demo::map());
        let world = CollisionWorld::from_map(&level, Params::default());
        let config = crate::sim::Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        let npc = crate::sim::characters::Npc {
            pos: at, floor: at[1], yaw, stance: Some(Stance::Stand),
            mind: Some(Mind::new(Alertness::Bored, 0.4, 4000.0, MP5K, 30, 3)),
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
        // Not at once: the spread opens wide when it first sees them, and closes
        assert!(shots.first().is_some_and(|&t| t > 0.2), "{shots:?}");
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
        // A half turn at its rate before the first shot
        assert!(shots.first().is_some_and(|&t| t >= (180.0 - FACING) / TURN_RATE), "{shots:?}");
    }

    #[test]
    fn turning_takes_the_short_way_round() {
        assert_eq!(turn_toward(350.0, 10.0, 5.0), 355.0);
        assert_eq!(turn_toward(10.0, 350.0, 5.0), 5.0);
        assert_eq!(turn_toward(10.0, 12.0, 5.0), 12.0);
        assert!(facing(359.0, 2.0) && !facing(0.0, 90.0));
    }
}
