//! What the other players' bodies are doing. A snapshot says where a player is and whether they
//! are firing, reloading or throwing, but not how they walk: that is told from how they are seen
//! to move, which way they face, and how fast.

use std::collections::HashMap;

use crate::sim::movement::{Movement, forward, ground_speed, right};

type V3 = [f32; 3];

/// How quickly the velocity seen follows the movement, per second
const SMOOTHING: f32 = 12.0;

/// Slower than this (units a second, at normal speed) is standing still
const STILL: f32 = 60.0;

/// Further than this in a frame is not walking but being put somewhere else
const TELEPORT: f32 = 250.0;

/// What a player is doing besides moving, as the snapshots say. Progress is from 0 to 1.
#[derive(Clone, Copy, Debug, Default)]
pub struct Doing {
    pub throwing: Option<f32>,
    pub reloading: Option<f32>,
    pub firing: bool,
}

struct State {
    last: Option<V3>,
    /// Ground velocity (map x and z), smoothed
    velocity: [f32; 2],
    movement: Movement,
    frame: f32,
}

/// The animation of each player in view
#[derive(Default)]
pub struct Animators {
    players: HashMap<usize, State>,
}

impl Animators {
    /// Forget a player, so that they start again from standing when they next appear
    pub fn forget(&mut self, slot: usize) {
        self.players.remove(&slot);
    }

    /// Advance a player's animation by `dt` seconds, given where they are seen now and where they
    /// face (`yaw`, degrees, as a camera's). `frames` says how long each movement's animation is,
    /// `speed_scale` how fast players move compared to normal and `pace` how fast the animation
    /// plays compared to the feet. Returns the movement and the frame of it to show.
    pub fn update(&mut self, slot: usize, pos: V3, yaw: f32, dt: f32, doing: Doing, frames: impl Fn(Movement) -> f32, speed_scale: f32, pace: f32) -> (Movement, f32) {
        let s = self.players.entry(slot).or_insert(State { last: None, velocity: [0.0; 2], movement: Movement::Idle, frame: 0.0 });
        if let (Some(last), true) = (s.last, dt > 0.0) {
            let moved = [pos[0] - last[0], pos[2] - last[2]];
            if moved[0].hypot(moved[1]) > TELEPORT {
                s.velocity = [0.0; 2];
            } else {
                let k = 1.0 - (-dt * SMOOTHING).exp();
                for i in 0..2 {
                    s.velocity[i] += (moved[i] / dt - s.velocity[i]) * k;
                }
            }
        }
        s.last = Some(pos);

        let walking = locomotion(s.velocity, yaw, speed_scale);
        let now = match (walking, doing) {
            (m, _) if m != Movement::Idle => m,
            (_, Doing { throwing: Some(_), .. }) => Movement::Throw,
            (_, Doing { reloading: Some(_), .. }) => Movement::Reload,
            (_, Doing { firing: true, .. }) => Movement::Fire,
            _ => Movement::Idle,
        };
        if now != s.movement {
            s.movement = now;
            s.frame = 0.0;
        }
        let n = frames(now).max(1.0);
        s.frame = match (now, doing) {
            (Movement::Throw, Doing { throwing: Some(p), .. }) | (Movement::Reload, Doing { reloading: Some(p), .. }) => p.clamp(0.0, 1.0) * (n - 1.0),
            (Movement::Idle | Movement::Fire | Movement::Reload | Movement::Throw, _) => (s.frame + 30.0 * dt) % n,
            _ => {
                let speed = s.velocity[0].hypot(s.velocity[1]) / speed_scale;
                (s.frame + (speed / ground_speed(now)).clamp(0.5, 3.0) * pace * 30.0 * dt) % n
            }
        };
        (now, s.frame)
    }
}

/// The way of walking that matches a ground velocity, for a player facing `yaw`
fn locomotion(velocity: [f32; 2], yaw: f32, speed_scale: f32) -> Movement {
    let speed = velocity[0].hypot(velocity[1]) / speed_scale;
    if speed < STILL {
        return Movement::Idle;
    }
    // The camera's Z is the opposite of the map's
    let (f, r) = (forward(yaw, 0.0), right(yaw));
    let ahead = velocity[0] * f[0] - velocity[1] * f[2];
    let beside = velocity[0] * r[0] - velocity[1] * r[2];
    if ahead.abs() >= beside.abs() {
        if ahead < 0.0 {
            Movement::Back
        } else if speed > (ground_speed(Movement::Walk) + ground_speed(Movement::Run)) / 2.0 {
            Movement::Run
        } else {
            Movement::Walk
        }
    } else if beside > 0.0 {
        Movement::StrafeRight
    } else {
        Movement::StrafeLeft
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    /// Walk a player for `seconds` along `dir` (a camera-space unit vector) at `speed`, and say
    /// what they ended up doing
    fn watch(yaw: f32, dir: V3, speed: f32, seconds: f32) -> Movement {
        let mut a = Animators::default();
        let mut pos = [0.0; 3];
        let mut last = (Movement::Idle, 0.0);
        for _ in 0..(seconds / DT) as usize {
            pos = [pos[0] + dir[0] * speed * DT, 0.0, pos[2] - dir[2] * speed * DT];
            last = a.update(0, pos, yaw, DT, Doing::default(), |_| 30.0, 1.0, 1.5);
        }
        last.0
    }

    #[test]
    fn a_player_is_animated_by_how_they_are_seen_to_move() {
        for yaw in [0.0, 90.0, 200.0] {
            let (f, r) = (forward(yaw, 0.0), right(yaw));
            let back = f.map(|c| -c);
            assert_eq!(watch(yaw, f, 514.0, 1.0), Movement::Walk, "yaw {yaw}");
            assert_eq!(watch(yaw, f, 735.0, 1.0), Movement::Run, "yaw {yaw}");
            assert_eq!(watch(yaw, back, 378.0, 1.0), Movement::Back, "yaw {yaw}");
            assert_eq!(watch(yaw, r, 348.0, 1.0), Movement::StrafeRight, "yaw {yaw}");
            assert_eq!(watch(yaw, r.map(|c| -c), 348.0, 1.0), Movement::StrafeLeft, "yaw {yaw}");
            assert_eq!(watch(yaw, f, 0.0, 1.0), Movement::Idle, "yaw {yaw}");
        }
    }

    #[test]
    fn standing_players_show_what_they_are_doing() {
        let mut a = Animators::default();
        let still = |a: &mut Animators, doing| a.update(1, [5.0, 0.0, 5.0], 0.0, DT, doing, |m| if m == Movement::Reload { 91.0 } else { 30.0 }, 1.0, 1.5);
        assert_eq!(still(&mut a, Doing { firing: true, ..Doing::default() }).0, Movement::Fire);
        // A reload plays across the time it takes, whatever the frame rate
        let (m, frame) = still(&mut a, Doing { reloading: Some(0.5), ..Doing::default() });
        assert_eq!((m, frame), (Movement::Reload, 45.0));
        assert_eq!(still(&mut a, Doing { reloading: Some(0.5), throwing: Some(1.0), firing: true }).0, Movement::Throw);
        assert_eq!(still(&mut a, Doing::default()).0, Movement::Idle);
    }

    #[test]
    fn being_put_somewhere_else_is_not_running() {
        let mut a = Animators::default();
        a.update(2, [0.0; 3], 0.0, DT, Doing::default(), |_| 30.0, 1.0, 1.5);
        let (m, _) = a.update(2, [3000.0, 0.0, 0.0], 0.0, DT, Doing::default(), |_| 30.0, 1.0, 1.5);
        assert_eq!(m, Movement::Idle);
    }
}
