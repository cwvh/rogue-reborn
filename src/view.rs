use rogue_reborn::concussion::Concussion;
use rogue_reborn::{sim, sound};
use rogue_reborn::net::client::{Frame, Status};
use rogue_reborn::net::watch::{Pose, Watch, WatchContext, WatchMode};
use rogue_reborn::sim::movement::Movement;
use crate::audio::Audio;
use crate::camera::Camera;
use crate::session::Session;
use crate::state::{Timing, Toggles};

// The player's own point of view: the camera and the body it follows, what the keys and the
// mouse hold, and what the player's senses go through (a blast, a death, watching others)
pub struct PlayerView {
    pub camera: Camera,
    // The view's height over the feet, which eases down and up as the player crouches and stands
    pub eye_now: f32,
    // Left Ctrl switches between standing and crouched
    pub crouch_wanted: bool,
    pub movement: Movement,
    pub player_frame: f32,
    pub player_hip_turn: f32,
    // Where the player stood a tick ago, to draw between ticks
    pub prev_feet: [f32; 3],
    pub was_free: bool,
    pub trigger: bool,
    // Right mouse button held: walk instead of the default run
    pub walk_held: bool,
    // A blast leaves the player shaken (the view), dazzled (a white veil) and deafened (muffled sound)
    pub shake: f32,
    pub veil: f32,
    pub deaf: f32,
    // ...and what they hear is dull: how much, and seconds since it began (see `sound::stunned_after`)
    pub stun: (f32, f32),
    // ...and flinches, and sees a grey tunnel for a moment
    pub concussion: Concussion,
    pub hit_marker_until: f32,
    // How this player died: by whom, and where they were looking before
    pub killed_by: Option<Option<usize>>,
    pub alive_look: Option<(f32, f32)>,
    // Dead in a round: whom the player watches (see net::watch), the keys pressed for it, whom
    // the camera is on and the room they were in last, and the eye height eased between the
    // watched one's stances
    pub watch: Watch,
    pub watch_next: bool,
    pub watch_previous: bool,
    pub watch_mode: bool,
    pub camera_on: Option<usize>,
    pub camera_room: Option<usize>,
    pub watch_eye: Option<(Option<usize>, f32)>,
}

impl PlayerView {
    pub fn new(camera: Camera, eye_height: f32, feet: [f32; 3]) -> PlayerView {
        PlayerView {
            camera,
            eye_now: eye_height,
            crouch_wanted: false,
            movement: Movement::Idle,
            player_frame: 0.0,
            player_hip_turn: 0.0,
            prev_feet: feet,
            was_free: false,
            trigger: false,
            walk_held: false,
            shake: 0.0,
            veil: 0.0,
            deaf: 0.0,
            stun: (0.0, f32::MAX),
            concussion: Concussion::default(),
            hit_marker_until: 0.0,
            killed_by: None,
            alive_look: None,
            watch: Watch::default(),
            watch_next: false,
            watch_previous: false,
            watch_mode: false,
            camera_on: None,
            camera_room: None,
            watch_eye: None,
        }
    }
}

// What the view reads besides the structs: the game, the player's slot and the frame's time
pub struct ViewPass<'a> {
    pub sim: &'a sim::Sim,
    pub me: usize,
    pub game_time: f32,
    pub step_dt: f32,
    pub eye_height: f32,
    pub crouch_eye_height: f32,
    pub team_game: bool,
}

// Where the camera is and whom it is on this frame
pub struct Watched {
    pub watch_pose: Option<Pose>,
    pub watched_body: Option<usize>,
    // Where the camera's rooms are taken from
    pub camera_feet: [f32; 3],
}

impl PlayerView {
    // Puts the camera at the player's eyes and returns where the player's feet are drawn
    pub fn eyes(&mut self, pass: &ViewPass, toggles: &Toggles, timing: &Timing) -> [f32; 3] {
        let ViewPass { sim, me, step_dt, eye_height, crouch_eye_height, .. } = *pass;
        // The view is at the player's eyes, between where they were a tick ago and now
        if toggles.first_person {
            let alpha = if timing.fixed_step { 1.0 } else { (timing.tick_budget / sim::TICK).clamp(0.0, 1.0) };
            let (a, b) = (self.prev_feet, sim.players[me].player.pos);
            let feet = [a[0] * (1.0 - alpha) + b[0] * alpha, a[1] * (1.0 - alpha) + b[1] * alpha, a[2] * (1.0 - alpha) + b[2] * alpha];
            let eye_target = if sim.players[me].player.crouched { crouch_eye_height } else { eye_height };
            self.eye_now += (eye_target - self.eye_now) * (1.0 - (-step_dt * 15.0).exp());
            let peek = sim.peek_offset(me);
            self.camera.position = [feet[0] + peek[0], feet[1] + self.eye_now + peek[1], -(feet[2] + peek[2])];
            feet
        } else {
            sim.players[me].player.pos
        }
    }

    // Dead on a server the view sinks and turns, and in a round it is a teammate's
    pub fn dead_and_watching(&mut self, pass: &ViewPass, session: &Session, toggles: &Toggles, frame: &Option<Frame>) -> Watched {
        let ViewPass { sim, me, game_time, step_dt, team_game, .. } = *pass;
        // Dead on a server: the view sinks to the floor and turns to whoever did it, and is given
        // back the way it was when the player comes back
        match session.net.as_ref().and_then(|_| sim.players[me].respawn_at) {
            Some(until) if toggles.first_person => {
                let (yaw0, pitch0) = *self.alive_look.get_or_insert((self.camera.yaw, self.camera.pitch));
                let fall = ((game_time - (until - sim::RESPAWN_SECONDS)) / 0.6).clamp(0.0, 1.0);
                self.camera.position[1] -= (self.eye_now - 25.0) * fall * fall * (3.0 - 2.0 * fall);
                let (want_yaw, want_pitch) = match self.killed_by.flatten().filter(|&k| k != me && sim.players[k].active) {
                    Some(k) => {
                        let t = sim.players[k].player.pos;
                        let d = [t[0] - self.camera.position[0], t[1] + 100.0 - self.camera.position[1], -t[2] - self.camera.position[2]];
                        let flat = d[0].hypot(d[2]);
                        ((-d[0]).atan2(-d[2]).to_degrees(), d[1].atan2(flat).to_degrees())
                    }
                    None => (yaw0, pitch0),
                };
                let k = 1.0 - (-5.0 * step_dt).exp();
                self.camera.yaw += ((want_yaw - self.camera.yaw + 540.0).rem_euclid(360.0) - 180.0) * k;
                self.camera.pitch += (want_pitch - self.camera.pitch) * k;
            }
            Some(_) => {}
            None => {
                if let Some((yaw, pitch)) = self.alive_look.take() {
                    (self.camera.yaw, self.camera.pitch) = (yaw, pitch);
                }
                self.killed_by = None;
            }
        }

        // Dead in a round, the view is a teammate's (see net::watch)
        let watch_pose = match (&session.net, frame) {
            (Some((client, _)), Some(f)) => {
                let cx = WatchContext { sim: sim, frame: f, me, round: client.round, team_game, connected: client.status == Status::Connected };
                self.watch.update(&cx);
                if std::mem::take(&mut self.watch_next) {
                    self.watch.next(&cx);
                }
                if std::mem::take(&mut self.watch_previous) {
                    self.watch.previous(&cx);
                }
                if std::mem::take(&mut self.watch_mode) {
                    self.watch.toggle_mode();
                }
                self.watch.camera(&cx)
            }
            _ => None,
        };
        match watch_pose {
            Some(pose) => {
                // Through their eyes the height eases as they crouch and stand, as the player's
                // own does, and starts afresh with each one watched
                let target = self.watch.target().filter(|_| self.watch.mode == WatchMode::Eyes);
                let y = match self.watch_eye {
                    Some((t, y)) if t == target && target.is_some() => y + (pose.eye[1] - y) * (1.0 - (-step_dt * 15.0).exp()),
                    _ => pose.eye[1],
                };
                self.watch_eye = Some((target, y));
                self.camera.position = [pose.eye[0], y, -pose.eye[2]];
                (self.camera.yaw, self.camera.pitch) = (pose.yaw, pose.pitch);
            }
            None => self.watch_eye = None,
        }
        let watched_body = self.watch.target().filter(|_| watch_pose.is_some() && self.watch.mode == WatchMode::Eyes);
        // Dead, the view is drawn from the rooms of whoever the camera is on, the one watched (or
        // last watched, while their view is held) or the player's own body, where they stand
        let (on, camera_feet) = watch_pose.and(self.watch.standing()).unwrap_or((me, sim.players[me].player.pos));
        if self.camera_on != Some(on) {
            (self.camera_on, self.camera_room) = (Some(on), None);
        }
        Watched { watch_pose, watched_body, camera_feet }
    }

    pub fn fade_blast(&mut self, audio: &mut Audio, step_dt: f32) {
        // A blast leaves the view shaken and whited out, and the sound muffled, for a while
        self.shake = (self.shake - 9.0 * step_dt).max(0.0);
        self.veil = (self.veil - 5.0 * step_dt).max(0.0);
        self.deaf = (self.deaf - 0.2 * step_dt).max(0.0);
        self.concussion.update(step_dt);
        if let Some(snd) = audio.sound.as_mut() {
            snd.master = audio.master_volume * (1.0 - 0.5 * self.deaf.min(1.0));
            self.stun.1 += step_dt;
            snd.set_muffle(sound::stunned_after(self.stun.0, self.stun.1));
        }
    }
}
