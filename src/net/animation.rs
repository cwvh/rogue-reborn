//! What the other players' bodies are doing. A snapshot says where a player is and whether they
//! are firing, reloading or throwing, but not how they walk: that is told from how they are seen
//! to move, which way they face, and how fast.

use std::collections::HashMap;

use crate::sim::movement::{Movement, Reloading, crouched_speed, forward, ground_speed, hip_turn, right};

type V3 = [f32; 3];

/// How quickly the velocity seen follows the movement, per second
const SMOOTHING: f32 = 12.0;

/// Slower than this (units a second, at normal speed) is standing still
const STILL: f32 = 60.0;

/// Going more than this far to the side for each unit forward or back (about 11 degrees off) is
/// going to the side as well
const SIDEWAYS: f32 = 0.2;

/// Further than this in a frame (or between two snapshots) is not walking but being put somewhere else
pub const TELEPORT: f32 = 250.0;

/// What a player is doing besides moving, as the snapshots say. Progress is from 0 to 1.
#[derive(Clone, Copy, Debug, Default)]
pub struct Doing {
    pub throwing: Option<f32>,
    pub reloading: Option<Reloading>,
    pub firing: bool,
    /// Crouched, which moves at other paces
    pub crouched: bool,
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
    /// `speed_scale` how fast players move compared to normal and `anim_pace` how fast the animation
    /// plays compared to the feet. Returns the movement and the frame of it to show (but for a
    /// reload, whose frame follows its progress and which motion is drawn for it).
    pub fn update(&mut self, slot: usize, pos: V3, yaw: f32, dt: f32, doing: Doing, frames: impl Fn(Movement) -> f32, speed_scale: f32, anim_pace: f32) -> (Movement, f32) {
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

        let pace = |m: Movement| if doing.crouched { crouched_speed(m) } else { ground_speed(m) };
        let walking = locomotion(s.velocity, yaw, speed_scale, pace);
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
            (Movement::Throw, Doing { throwing: Some(p), .. }) => p.clamp(0.0, 1.0) * (n - 1.0),
            (Movement::Idle | Movement::Fire | Movement::Reload | Movement::Throw, _) => (s.frame + 30.0 * dt) % n,
            _ => {
                let speed = s.velocity[0].hypot(s.velocity[1]) / speed_scale;
                (s.frame + (speed / pace(now)).clamp(0.5, 3.0) * anim_pace * 30.0 * dt) % n
            }
        };
        (now, s.frame)
    }

    /// How far a player's hips turn as they are seen to move, facing `yaw` (see
    /// `movement::hip_turn`): when walking or running forward or back, and to one side as well
    pub fn hip_turn(&self, slot: usize, yaw: f32) -> f32 {
        let Some(s) = self.players.get(&slot).filter(|s| matches!(s.movement, Movement::Walk | Movement::Run | Movement::Back)) else {
            return 0.0;
        };
        let (ahead, beside) = along(s.velocity, yaw);
        let aside = beside.abs() > ahead.abs() * SIDEWAYS;
        hip_turn(ahead > 0.0, ahead < 0.0, aside && beside < 0.0, aside && beside > 0.0)
    }
}

/// How fast a ground velocity goes ahead and to the right of a player facing `yaw`
fn along(velocity: [f32; 2], yaw: f32) -> (f32, f32) {
    // The camera's Z is the opposite of the map's
    let (f, r) = (forward(yaw, 0.0), right(yaw));
    (velocity[0] * f[0] - velocity[1] * f[2], velocity[0] * r[0] - velocity[1] * r[2])
}

/// The way of walking that matches a ground velocity, for a player facing `yaw` who moves at
/// the paces `pace` gives
fn locomotion(velocity: [f32; 2], yaw: f32, speed_scale: f32, pace: impl Fn(Movement) -> f32) -> Movement {
    let speed = velocity[0].hypot(velocity[1]) / speed_scale;
    if speed < STILL {
        return Movement::Idle;
    }
    let (ahead, beside) = along(velocity, yaw);
    if ahead.abs() >= beside.abs() {
        if ahead < 0.0 {
            Movement::Back
        } else if speed > (pace(Movement::Walk) + pace(Movement::Run)) / 2.0 {
            Movement::Run
        } else {
            Movement::Walk
        }
    } else {
        let run = speed > (pace(Movement::WalkRight) + pace(Movement::StrafeRight)) / 2.0;
        match (beside > 0.0, run) {
            (true, true) => Movement::StrafeRight,
            (true, false) => Movement::WalkRight,
            (false, true) => Movement::StrafeLeft,
            (false, false) => Movement::WalkLeft,
        }
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
    fn hips_turn_toward_a_side_step_taken_going_forward_or_back() {
        use crate::sim::movement::HIP_TURN;
        for yaw in [0.0, 90.0, 200.0] {
            let (f, r) = (forward(yaw, 0.0), right(yaw));
            let turn = |ahead: f32, beside: f32, speed: f32| {
                let dir = [f[0] * ahead + r[0] * beside, 0.0, f[2] * ahead + r[2] * beside];
                let mut a = Animators::default();
                let mut pos = [0.0; 3];
                for _ in 0..60 {
                    pos = [pos[0] + dir[0] * speed * DT, 0.0, pos[2] - dir[2] * speed * DT];
                    a.update(0, pos, yaw, DT, Doing::default(), |_| 30.0, 1.0, 1.5);
                }
                a.hip_turn(0, yaw)
            };
            // Running forward and to the right, as the run and a side step add up
            assert_eq!(turn(0.9, 0.43, 735.0), HIP_TURN, "yaw {yaw}");
            assert_eq!(turn(0.9, -0.43, 735.0), -HIP_TURN, "yaw {yaw}");
            // Backing up to the right leads with the back of the hips, so they turn left
            assert_eq!(turn(-0.74, 0.68, 378.0), -HIP_TURN, "yaw {yaw}");
            assert_eq!(turn(1.0, 0.0, 735.0), 0.0, "yaw {yaw}");
            assert_eq!(turn(0.0, 1.0, 348.0), 0.0, "yaw {yaw}");
        }
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
            assert_eq!(watch(yaw, r, 143.0, 1.0), Movement::WalkRight, "yaw {yaw}");
            assert_eq!(watch(yaw, r.map(|c| -c), 143.0, 1.0), Movement::WalkLeft, "yaw {yaw}");
            assert_eq!(watch(yaw, f, 0.0, 1.0), Movement::Idle, "yaw {yaw}");
        }
    }

    #[test]
    fn standing_players_show_what_they_are_doing() {
        let mut a = Animators::default();
        let still = |a: &mut Animators, doing| a.update(1, [5.0, 0.0, 5.0], 0.0, DT, doing, |_| 30.0, 1.0, 1.5);
        assert_eq!(still(&mut a, Doing { firing: true, ..Doing::default() }).0, Movement::Fire);
        let reloading = Some(Reloading { progress: 0.5, empty: false });
        assert_eq!(still(&mut a, Doing { reloading, ..Doing::default() }).0, Movement::Reload);
        assert_eq!(still(&mut a, Doing { reloading, throwing: Some(1.0), firing: true, ..Doing::default() }).0, Movement::Throw);
        assert_eq!(still(&mut a, Doing::default()).0, Movement::Idle);
    }

    #[test]
    fn a_crouched_player_is_seen_to_run_at_a_crouched_run() {
        let f = forward(0.0, 0.0);
        let mut a = Animators::default();
        let crouched = Doing { crouched: true, ..Doing::default() };
        let mut pos = [0.0; 3];
        let mut seen = Movement::Idle;
        let speed = crouched_speed(Movement::Run);
        for _ in 0..60 {
            pos = [pos[0] + f[0] * speed * DT, 0.0, pos[2] - f[2] * speed * DT];
            seen = a.update(0, pos, 0.0, DT, crouched, |_| 30.0, 1.0, 1.5).0;
        }
        // Standing, that pace would be a walk
        assert_eq!(seen, Movement::Run);
    }

    #[test]
    fn being_put_somewhere_else_is_not_running() {
        let mut a = Animators::default();
        a.update(2, [0.0; 3], 0.0, DT, Doing::default(), |_| 30.0, 1.0, 1.5);
        let (m, _) = a.update(2, [3000.0, 0.0, 0.0], 0.0, DT, Doing::default(), |_| 30.0, 1.0, 1.5);
        assert_eq!(m, Movement::Idle);
    }
}
