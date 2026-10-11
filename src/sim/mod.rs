//! The game itself, apart from how it looks and sounds: the player, their weapon and
//! grenades, the characters, doors and glass, and what a tick of time does to them.
//!
//! The viewer feeds the sim an `Input` for every `TICK` and gets back `Event`s: things that
//! happened, which it draws, plays sounds for and prints. The sim never reads a clock, a
//! window or a sound device, and takes all its randomness from its own generator, so the same
//! inputs always give the same game. That is what lets a recorded run be played again, and
//! lets a server and its clients agree.

pub mod characters;
pub mod movement;

use std::sync::Arc;

use crate::collision::{CollisionWorld, Player, SolidRoom};
use crate::combat::{self, HitResult};
use crate::door::{Door, DoorState};
use crate::glass::{self, Pane};
use crate::grenade::{self, Grenade};
use crate::impact::Surface;
use crate::map::Map;
use crate::util::{dist3, mul3, next_rng, unit3};
use crate::visibility::Visibility;

use characters::{Characters, Npc};
use movement::{forward, ground_speed, right};

type V3 = [f32; 3];

/// The length of a tick, in seconds. The sim is stepped at this rate, whatever the frame rate.
pub const TICK: f32 = 1.0 / 60.0;

/// How long a reload takes
pub const RELOAD_SECONDS: f32 = 2.2;

/// A grenade throw: from the start of the swing to the end, and how far into it the grenade
/// leaves the hand (0..1)
pub const THROW_SECONDS: f32 = 0.85;
pub const THROW_RELEASE: f32 = 0.35;

/// How far away the door being opened or closed can be
const DOOR_REACH: f32 = 220.0;

/// A grenade throw under way
#[derive(Clone, Copy)]
pub struct Throw {
    pub start: f32,
    /// How long the throw was held for before letting go (0..1)
    pub charge: f32,
    /// Whether the grenade has left the hand
    pub thrown: bool,
}

/// What the player is doing during one tick
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// Where the player looks, in degrees
    pub yaw: f32,
    pub pitch: f32,
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    /// Moving at a run (the default) rather than a walk
    pub run: bool,
    /// The trigger is held
    pub fire: bool,
    /// Pressed since the last tick: use (open or close a door), reload, take out the gun,
    /// take out a grenade
    pub use_key: bool,
    pub reload: bool,
    pub gun: bool,
    pub grenade: bool,
    /// Pressed since the last tick: switch between only the player's room and its neighbors
    /// being solid, and all of the level (the viewer's V key)
    pub toggle_culling: bool,
    /// Set when the player is not in control, because the view is flying free: the point in
    /// map space it is at. When control returns, the player drops in there.
    pub observer: Option<V3>,
}

/// Something that happened in a tick
#[derive(Clone, Debug)]
pub enum Event {
    /// A sound: at a point in the level, or with no place (the player's own)
    Sound { name: String, at: Option<V3>, gain: f32 },
    /// A line for the log
    Log(String),
    /// A shot was fired: where it left and where it ended, and whether it struck a character
    Shot { start: V3, end: V3, hit_character: bool },
    /// A bullet struck the level. `seed` is for the effects it makes, `dist` how far it flew.
    BulletHit { point: V3, normal: V3, dir: V3, surface: Surface, material: String, dist: f32, seed: u64 },
    /// A pane broke where `at` is, struck along `dir`
    Glass { pane: usize, at: V3, dir: V3, seed: u64 },
    /// A grenade went off. `on_surface` is false in mid-air, where there is nothing to scorch.
    Blast { point: V3, normal: V3, surface: Surface, on_surface: bool, seed: u64 },
    /// What a blast did to the player: how hard it shook them, and how white it made their view
    PlayerBlast { shake: f32, veil: f32 },
}

/// FNV-1a, a simple hash that gives the same answer on every machine
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf29ce484222325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 = (self.0 ^ x as u64).wrapping_mul(0x100000001b3);
        }
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    fn opt_f32(&mut self, v: Option<f32>) {
        match v {
            None => self.u32(0),
            Some(v) => { self.u32(1); self.f32(v); }
        }
    }
    fn v3(&mut self, v: V3) {
        for c in v {
            self.f32(c);
        }
    }
}

/// Settings a sim starts with
pub struct Config {
    pub magazine: u32,
    /// Rounds in the magazine at the start
    pub ammo: u32,
    pub reserve: u32,
    pub grenades: u32,
    /// Scales the player's speed
    pub speed_scale: f32,
    pub seed: u64,
    /// Only the player's room and its neighbors are solid (the viewer's V key)
    pub culling: bool,
}

/// The whole game
pub struct Sim {
    pub level: Arc<Map>,
    pub world: CollisionWorld,
    pub visibility: Visibility,
    /// Seconds since the start
    pub time: f32,
    pub rng: u64,
    pub culling: bool,
    speed_scale: f32,
    eye_height: f32,

    pub player: Player,
    /// Where the view was last tick if the player was not in control
    last_observer: Option<V3>,
    /// Whether the player is in control, as of the last tick
    pub controlled: bool,
    /// How fast the player moved in the last tick, in units per second
    pub moved_speed: f32,

    // Weapon
    pub magazine: u32,
    pub ammo: u32,
    pub reserve: u32,
    pub reload_until: Option<f32>,
    next_shot: f32,
    /// The muzzle flashes until this time
    pub flash_until: f32,
    /// Whether the trigger is held with the gun out, as of the last tick
    pub gun_trigger: bool,
    pub kills: u32,

    // Grenades
    pub equipped_grenade: bool,
    pub grenades_left: u32,
    pub charge_from: Option<f32>,
    pub throw: Option<Throw>,
    pub thrown: Vec<Grenade>,

    pub characters: Option<Characters>,
    pub npcs: Vec<Npc>,

    pub doors: Vec<Door>,
    pub door_states: Vec<DoorState>,
    /// Whether each portal is shut by a closed door
    pub blocked_portals: Vec<bool>,
    pub panes: Vec<Pane>,
    /// When each pane broke
    pub broken_at: Vec<Option<f32>>,

    /// A line for the screen and when it goes away
    pub message: (String, f32),
    /// The health of the character last hit
    pub target_hp: Option<f32>,

    // Where the player is in the level
    pub current_room: Option<usize>,
    /// The rooms that count around the player, and the room drawing starts from
    pub player_rooms: Vec<u32>,
    pub render_rooms: Vec<u32>,
    /// The player's feet and eyes (or the free view's point twice), for portals they are standing in
    pub body: [V3; 2],
    pub collision_rooms: Option<Vec<SolidRoom>>,
    last_render_rooms: Vec<u32>,
}

impl Sim {
    pub fn new(level: Arc<Map>, world: CollisionWorld, config: Config, characters: Option<Characters>, npcs: Vec<Npc>, player_at: V3) -> Sim {
        let visibility = Visibility::new(&level);
        let doors: Vec<Door> = world.doors().cloned().collect();
        let panes = glass::panes(&level);
        let eye_height = world.params().eye_height;
        Sim {
            visibility,
            door_states: vec![DoorState::default(); doors.len()],
            blocked_portals: vec![false; level.portals.portals.len()],
            doors,
            broken_at: vec![None; panes.len()],
            panes,
            level,
            world,
            time: 0.0,
            rng: config.seed | 1,
            culling: config.culling,
            speed_scale: config.speed_scale,
            eye_height,
            player: Player::new(player_at),
            last_observer: None,
            controlled: true,
            moved_speed: 0.0,
            magazine: config.magazine,
            ammo: config.ammo,
            reserve: config.reserve,
            reload_until: None,
            next_shot: 0.0,
            flash_until: 0.0,
            gun_trigger: false,
            kills: 0,
            equipped_grenade: false,
            grenades_left: config.grenades,
            charge_from: None,
            throw: None,
            thrown: Vec::new(),
            characters,
            npcs,
            message: (String::new(), 0.0),
            target_hp: None,
            current_room: None,
            player_rooms: Vec::new(),
            render_rooms: Vec::new(),
            body: [[0.0; 3]; 2],
            collision_rooms: None,
            last_render_rooms: Vec::new(),
        }
    }

    /// Where the player's eyes are
    pub fn eye(&self) -> V3 {
        [self.player.pos[0], self.player.pos[1] + self.eye_height, self.player.pos[2]]
    }

    /// Set every door this far open (0 shut, 1 open)
    pub fn set_doors_open(&mut self, open: f32) {
        for (i, s) in self.door_states.iter_mut().enumerate() {
            s.open = open.clamp(0.0, 1.0);
            s.opening = open > 0.5;
            self.world.set_door_open(i, s.open);
        }
    }

    /// Whether the full-auto firing sound should be running
    pub fn firing(&self) -> bool {
        self.controlled && self.gun_trigger && self.reload_until.is_none() && self.ammo > 0
    }

    /// A fingerprint of everything that decides what happens next, to compare two runs of the
    /// game: equal inputs must leave equal fingerprints
    pub fn checksum(&self) -> u64 {
        let mut h = Fnv::new();
        h.f32(self.time);
        h.u64(self.rng);
        h.u32(self.culling as u32);
        h.v3(self.player.pos);
        h.f32(self.player.vel_y);
        h.u32(self.player.on_ground as u32);
        h.u32(self.ammo);
        h.u32(self.reserve);
        h.opt_f32(self.reload_until);
        h.f32(self.next_shot);
        h.f32(self.flash_until);
        h.u32(self.kills);
        h.u32(self.equipped_grenade as u32);
        h.u32(self.grenades_left);
        h.opt_f32(self.charge_from);
        match self.throw {
            None => h.u32(0),
            Some(t) => { h.u32(1); h.f32(t.start); h.f32(t.charge); h.u32(t.thrown as u32); }
        }
        for g in &self.thrown {
            h.v3(g.pos);
            h.v3(g.vel);
            h.f32(g.fuse);
            h.f32(g.spin);
            h.u32(g.at_rest as u32);
        }
        for n in &self.npcs {
            h.v3(n.pos);
            h.f32(n.floor);
            h.f32(n.yaw);
            h.f32(n.health.hp);
            match n.health.state {
                combat::NpcState::Idle => h.u32(0),
                combat::NpcState::Reacting { motion, start } => { h.u32(1); h.bytes(motion.as_bytes()); h.f32(start); }
                combat::NpcState::Dead { motion, start } => { h.u32(2); h.bytes(motion.as_bytes()); h.f32(start); }
            }
        }
        for s in &self.door_states {
            h.f32(s.open);
            h.u32(s.opening as u32);
        }
        for b in &self.broken_at {
            h.opt_f32(*b);
        }
        h.u32(self.current_room.map_or(u32::MAX, |r| r as u32));
        h.0
    }

    fn announce(&mut self, text: &str, seconds: f32) {
        self.message = (text.to_string(), self.time + seconds);
    }

    fn surface_of(&self, material: Option<usize>) -> Surface {
        material.and_then(|m| self.level.materials.materials.get(m)).map_or(Surface::Concrete, |m| Surface::classify(&m.filename))
    }

    /// Run the game for one tick
    pub fn tick(&mut self, input: &Input, dt: f32) -> Vec<Event> {
        let mut ev: Vec<Event> = Vec::new();
        let controlled = input.observer.is_none();
        self.controlled = controlled;
        if controlled {
            // Control returns: the player drops in from wherever the view was
            if let Some(o) = self.last_observer.take() {
                self.player = Player::new([o[0], o[1] - self.eye_height, o[2]]);
            }
        } else {
            self.last_observer = input.observer;
        }
        if input.toggle_culling {
            self.culling = !self.culling;
        }

        // Keys pressed since the last tick
        if input.gun && self.throw.is_none() {
            // Back to the gun
            self.equipped_grenade = false;
            self.charge_from = None;
        }
        if input.grenade && controlled && self.throw.is_none() && self.reload_until.is_none() {
            self.equipped_grenade = true;
            if self.grenades_left == 0 {
                self.announce("NO GRENADES", 1.0);
            }
        }
        // Reload: takes time, then fills the magazine from the reserve
        if input.reload && controlled && !self.equipped_grenade && self.reload_until.is_none() && self.ammo < self.magazine && self.reserve > 0 {
            self.reload_until = Some(self.time + RELOAD_SECONDS);
        }

        // The trigger fires the gun, unless a grenade is out
        let gun_trigger = input.fire && !self.equipped_grenade;
        self.gun_trigger = gun_trigger;
        self.time += dt;

        let (forward_cam, right_cam) = (forward(input.yaw, input.pitch), right(input.yaw));
        if input.use_key && controlled {
            self.use_door(forward_cam, &mut ev);
        }

        // Doors move, and the collision and the portals follow
        for (i, (d, s)) in self.doors.iter().zip(self.door_states.iter_mut()).enumerate() {
            if s.advance(d, dt) {
                self.world.set_door_open(i, s.open);
            }
            if let Some(p) = d.portal {
                self.blocked_portals[p] = s.closed();
            }
        }
        // Finish a reload
        if self.reload_until.is_some_and(|t| self.time >= t) {
            let take = (self.magazine - self.ammo).min(self.reserve);
            self.ammo += take;
            self.reserve -= take;
            self.reload_until = None;
        }
        // Injury reactions end and the character goes back to idling
        if let Some(chars) = &self.characters {
            for npc in self.npcs.iter_mut() {
                if let combat::NpcState::Reacting { motion, start } = npc.health.state {
                    if (self.time - start) * 30.0 >= chars.frames(motion) {
                        npc.health.state = combat::NpcState::Idle;
                    }
                }
            }
        }

        if controlled {
            self.move_player(input, dt, right_cam);
            self.throw_grenade(input, forward_cam, right_cam, &mut ev);
            self.shoot(forward_cam, &mut ev);
        }

        self.update_grenades(dt, controlled, &mut ev);
        self.track_rooms(input.observer);
        ev
    }

    /// Space or Backspace opens or closes the door the player is looking at
    fn use_door(&mut self, forward_cam: V3, ev: &mut Vec<Event>) {
        let eye = self.eye();
        let look = [forward_cam[0], forward_cam[1], -forward_cam[2]];
        let mut best: Option<(usize, f32)> = None;
        for (i, d) in self.doors.iter().enumerate() {
            // Aim at the doorway, or at the leaf wherever it has gone (a door swung open
            // to the side is still the one in front of you)
            for c in [d.center(0.0), d.center(self.door_states[i].open)] {
                let to = [c[0] - eye[0], c[1] - eye[1], c[2] - eye[2]];
                let dist = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
                if dist > DOOR_REACH || dist < 1.0 {
                    continue;
                }
                // Within 60 degrees of where they are looking
                let facing = (to[0] * look[0] + to[1] * look[1] + to[2] * look[2]) / dist;
                if facing < 0.5 {
                    continue;
                }
                let score = dist * (2.0 - facing);
                if best.is_none_or(|(_, s)| score < s) {
                    best = Some((i, score));
                }
            }
        }
        match best {
            Some((i, _)) => {
                self.door_states[i].opening = !self.door_states[i].opening;
                let opening = self.door_states[i].opening;
                let sound_name = if opening { &self.doors[i].open_sound } else { &self.doors[i].close_sound };
                if !sound_name.eq_ignore_ascii_case("none") {
                    ev.push(Event::Sound { name: sound_name.clone(), at: Some(self.doors[i].center(self.door_states[i].open)), gain: 1.0 });
                }
                self.announce(&format!("DOOR {}", if opening { "OPENING" } else { "CLOSING" }), 1.2);
                ev.push(Event::Log(format!("t={:.2} door {} {}", self.time, self.doors[i].name, if opening { "opening" } else { "closing" })));
            }
            None => self.announce("NO DOOR IN REACH", 1.0),
        }
    }

    /// Walk the player, at speeds that come from the animations so the feet do not slide
    fn move_player(&mut self, input: &Input, dt: f32, right_cam: V3) {
        let yaw_rad = input.yaw.to_radians();
        let fwd_flat = [-yaw_rad.sin(), 0.0, -yaw_rad.cos()];
        let (w, a, s_, d, run) = (input.forward, input.left, input.back, input.right, input.run);
        let scale = self.speed_scale;
        let fwd_speed = scale * if w == s_ { 0.0 }
            else if w { ground_speed(if run { movement::Movement::Run } else { movement::Movement::Walk }) }
            else { -ground_speed(movement::Movement::Back) };
        let side_speed = scale * if a == d { 0.0 }
            else if d { ground_speed(movement::Movement::StrafeRight) } else { -ground_speed(movement::Movement::StrafeLeft) };
        let mut vel = [
            fwd_flat[0] * fwd_speed + right_cam[0] * side_speed,
            fwd_flat[2] * fwd_speed + right_cam[2] * side_speed,
        ];
        // Moving diagonally is no faster than the faster of the two
        let (len, cap) = ((vel[0] * vel[0] + vel[1] * vel[1]).sqrt(), fwd_speed.abs().max(side_speed.abs()));
        if len > cap && len > 0.0 {
            vel = [vel[0] / len * cap, vel[1] / len * cap];
        }

        let before = self.player.pos;
        self.world.step(&mut self.player, [vel[0] * dt, -vel[1] * dt], dt);
        self.moved_speed = ((self.player.pos[0] - before[0]).powi(2) + (self.player.pos[2] - before[2]).powi(2)).sqrt() / dt.max(1e-4);
    }

    /// A grenade: hold the trigger to wind up, let go to throw. The swing plays for a moment,
    /// and the grenade leaves the hand part of the way in.
    fn throw_grenade(&mut self, input: &Input, forward_cam: V3, right_cam: V3, ev: &mut Vec<Event>) {
        if !self.equipped_grenade {
            return;
        }
        let now = self.time;
        let mut finished = false;
        match self.throw {
            None => {
                if input.fire && self.grenades_left > 0 {
                    self.charge_from.get_or_insert(now);
                } else if let Some(from) = self.charge_from.take() {
                    let charge = ((now - from) / grenade::CHARGE_SECONDS).clamp(0.0, 1.0);
                    self.throw = Some(Throw { start: now, charge, thrown: false });
                } else if input.fire && self.grenades_left == 0 && now >= self.message.1 {
                    self.announce("NO GRENADES", 0.8);
                }
            }
            Some(mut th) => {
                let progress = (now - th.start) / THROW_SECONDS;
                if !th.thrown && progress >= THROW_RELEASE {
                    th.thrown = true;
                    self.grenades_left -= 1;
                    let eye = self.eye();
                    let f = forward_cam;
                    // Thrown a little upward, as a lob
                    let aim = unit3([f[0], f[1] + 0.14, -f[2]]);
                    let side = [right_cam[0], right_cam[1], -right_cam[2]];
                    let want = [eye[0] + aim[0] * 28.0 + side[0] * 10.0, eye[1] + aim[1] * 28.0 - 12.0, eye[2] + aim[2] * 28.0 + side[2] * 10.0];
                    // Not through a wall that is right in front of the player
                    let to = unit3([want[0] - eye[0], want[1] - eye[1], want[2] - eye[2]]);
                    let reach = dist3(eye, want);
                    let from = match self.world.raycast(eye, to, reach + grenade::RADIUS) {
                        Some(d) => { let d = (d - grenade::RADIUS - 1.0).max(0.0); [eye[0] + to[0] * d, eye[1] + to[1] * d, eye[2] + to[2] * d] }
                        None => want,
                    };
                    self.thrown.push(Grenade::thrown(from, aim, th.charge, [0.0; 3]));
                    ev.push(Event::Log(format!("t={now:.2} threw a grenade at {:.0}% power", th.charge * 100.0)));
                }
                self.throw = Some(th);
                finished = progress >= 1.0;
            }
        }
        if finished {
            self.throw = None;
            if self.grenades_left == 0 {
                self.equipped_grenade = false;
                self.announce("OUT OF GRENADES", 1.2);
            }
        }
    }

    /// Shooting: a ray from the eye along the view, stopped by walls
    fn shoot(&mut self, forward_cam: V3, ev: &mut Vec<Event>) {
        let gun_trigger = self.gun_trigger;
        if gun_trigger && self.reload_until.is_none() && self.ammo == 0 && self.reserve > 0 {
            self.reload_until = Some(self.time + RELOAD_SECONDS);
        }
        if !(gun_trigger && self.reload_until.is_none() && self.ammo > 0 && self.time >= self.next_shot) {
            return;
        }
        self.ammo -= 1;
        self.next_shot = self.time + 60.0 / 800.0; // the MP5K fires about 800 rounds a minute
        self.flash_until = self.time + 0.05;
        let now = self.time;

        let eye = self.eye();
        let f = forward_cam;
        let dir = [f[0], f[1], -f[2]]; // camera space to MAP space
        let mut wall = self.world.raycast(eye, dir, 20000.0);

        let mut best: Option<(usize, combat::Hit)> = None;
        if let Some(chars) = &self.characters {
            for (i, npc) in self.npcs.iter().enumerate() {
                if npc.health.is_dead() {
                    continue;
                }
                if let Some(h) = chars.raycast(npc, now, eye, dir) {
                    if best.map_or(true, |(_, b)| h.distance < b.distance) {
                        best = Some((i, h));
                    }
                }
            }
        }

        // Glass in the way breaks and lets the bullet through
        loop {
            let glass_hit = self.panes.iter().enumerate()
                .filter(|(i, _)| self.broken_at[*i].is_none())
                .filter_map(|(i, p)| p.raycast(eye, dir).map(|d| (i, d)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((gi, gd)) = glass_hit else { break };
            // Something solid, or a character, is in front of the glass
            if wall.is_some_and(|w| w < gd - 0.5) || best.as_ref().is_some_and(|(_, h)| h.distance < gd) {
                break;
            }
            self.broken_at[gi] = Some(now);
            self.world.switch_off(&self.panes[gi].name);
            next_rng(&mut self.rng);
            let at = [eye[0] + dir[0] * gd, eye[1] + dir[1] * gd, eye[2] + dir[2] * gd];
            ev.push(Event::Glass { pane: gi, at, dir, seed: self.rng });
            ev.push(Event::Log(format!("t={now:.2} shot broke glass {}", self.panes[gi].name)));
            ev.push(Event::Sound { name: "C_GBRKMD.WAV".to_string(), at: Some(self.panes[gi].center()), gain: 1.0 });
            wall = self.world.raycast(eye, dir, 20000.0);
        }

        let start = [eye[0] + dir[0] * 30.0, eye[1] + dir[1] * 30.0 - 8.0, eye[2] + dir[2] * 30.0];
        match best {
            Some((i, hit)) if wall.map_or(true, |w| hit.distance <= w) => {
                next_rng(&mut self.rng);
                let result = self.npcs[i].health.apply_hit(&hit, now, (self.rng >> 33) as u32);
                self.target_hp = Some(self.npcs[i].health.hp);
                ev.push(Event::Log(format!("t={now:.2} shot hit character {i}: {:?} from the {:?}, {:?}, hp now {}", hit.part, hit.facing, result, self.npcs[i].health.hp)));
                let text = match result {
                    HitResult::Killed => {
                        self.kills += 1;
                        if hit.part == combat::BodyPart::Head { "HEAD SHOT KILL".to_string() } else { "KILLED".to_string() }
                    }
                    _ => format!("HIT {:?}", hit.part).to_uppercase(),
                };
                self.announce(&text, 1.2);
                ev.push(Event::Sound { name: "C_BHTHT.WAV".to_string(), at: Some(hit.point), gain: 1.0 });
                // Voices: a grunt when hurt, a cry when killed
                let (name, n) = if matches!(result, HitResult::Killed) { ("re_ded", 2) } else { ("re_wnd", 3) };
                let pick = 1 + (self.rng >> 40) as u32 % n;
                ev.push(Event::Sound { name: format!("English/{name}{pick}.wav"), at: Some(self.npcs[i].pos), gain: 1.0 });
                ev.push(Event::Shot { start, end: hit.point, hit_character: true });
            }
            _ => {
                // The bullet struck the level
                if let Some(hit) = wall.and_then(|_| self.world.raycast_hit(eye, dir, 20000.0)) {
                    let point = [eye[0] + dir[0] * hit.dist, eye[1] + dir[1] * hit.dist, eye[2] + dir[2] * hit.dist];
                    let surface = self.surface_of(hit.material);
                    let material = hit.material.and_then(|m| self.level.materials.materials.get(m)).map_or("?", |m| m.filename.as_str()).to_string();
                    next_rng(&mut self.rng);
                    ev.push(Event::BulletHit { point, normal: hit.normal, dir, surface, material, dist: hit.dist, seed: self.rng });
                }
                let d = wall.unwrap_or(5000.0);
                ev.push(Event::Shot { start, end: [eye[0] + dir[0] * d, eye[1] + dir[1] * d, eye[2] + dir[2] * d], hit_character: false });
            }
        }
    }

    /// Grenades in the air: they bounce off the level and go off when the fuse runs out
    fn update_grenades(&mut self, dt: f32, controlled: bool, ev: &mut Vec<Event>) {
        let now = self.time;
        let mut blasts: Vec<V3> = Vec::new();
        let mut thrown = std::mem::take(&mut self.thrown);
        for g in thrown.iter_mut() {
            for b in g.update(dt, &self.world) {
                // Glass: it goes through, shattering the pane, slowed and knocked off line
                let heading = unit3(b.incoming);
                let behind = [b.point[0] - heading[0] * 20.0, b.point[1] - heading[1] * 20.0, b.point[2] - heading[2] * 20.0];
                let pane = self.panes.iter().enumerate()
                    .filter(|(i, _)| self.broken_at[*i].is_none())
                    .filter_map(|(i, p)| p.raycast(behind, heading).filter(|d| *d < 45.0).map(|d| (i, d)))
                    .min_by(|a, c| a.1.total_cmp(&c.1));
                if let Some((gi, _)) = pane {
                    self.broken_at[gi] = Some(now);
                    self.world.switch_off(&self.panes[gi].name);
                    next_rng(&mut self.rng);
                    let rng = self.rng;
                    ev.push(Event::Glass { pane: gi, at: b.point, dir: heading, seed: rng });
                    let wobble = |bits: u64| ((bits & 0xFFFF) as f32 / 32768.0 - 1.0) * 0.18;
                    let (sx, sy, sz) = (wobble(rng >> 8), wobble(rng >> 24), wobble(rng >> 40));
                    let speed = (b.incoming[0].powi(2) + b.incoming[1].powi(2) + b.incoming[2].powi(2)).sqrt() * 0.6;
                    g.vel = mul3(unit3([heading[0] + sx, heading[1] + sy, heading[2] + sz]), speed);
                    g.pos = [b.point[0] + heading[0] * 12.0, b.point[1] + heading[1] * 12.0, b.point[2] + heading[2] * 12.0];
                    g.at_rest = false;
                    ev.push(Event::Sound { name: "C_GBRKMD.WAV".to_string(), at: Some(b.point), gain: 1.0 });
                    continue;
                }
                let name = match self.surface_of(b.material) {
                    Surface::Metal => "C_GRMET.WAV",
                    Surface::Wood => "C_GRWOD.WAV",
                    Surface::Earth | Surface::Soft => "C_GREAR.WAV",
                    _ => "C_GRCON.WAV",
                };
                ev.push(Event::Sound { name: name.to_string(), at: Some(b.point), gain: (b.speed / 500.0).clamp(0.25, 1.0) });
            }
            if g.goes_off() {
                blasts.push(g.pos);
            }
        }
        thrown.retain(|g| !g.goes_off());
        self.thrown = thrown;
        for c in blasts {
            self.blast(c, controlled, ev);
        }
    }

    /// A grenade goes off at `c`
    fn blast(&mut self, c: V3, controlled: bool, ev: &mut Vec<Event>) {
        let now = self.time;
        // The surface it went off against, the nearest of the six directions
        let mut near: Option<(f32, V3, V3, Option<usize>)> = None;
        for d in [[0.0, -1.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]] {
            if let Some(h) = self.world.raycast_hit(c, d, 80.0) {
                if near.is_none_or(|n| h.dist < n.0) {
                    near = Some((h.dist, [c[0] + d[0] * h.dist, c[1] + d[1] * h.dist, c[2] + d[2] * h.dist], h.normal, h.material));
                }
            }
        }
        let (point, normal, material) = near.map_or((c, [0.0, 1.0, 0.0], None), |n| (n.1, n.2, n.3));
        let surface = self.surface_of(material);
        next_rng(&mut self.rng);
        // In mid-air (nothing within a meter) there is no ground to scorch or throw dust along
        let on_surface = near.is_some_and(|n| n.0 < 100.0);
        ev.push(Event::Blast { point, normal, surface, on_surface, seed: self.rng });
        ev.push(Event::Log(format!("t={now:.2} grenade went off at ({:.0}, {:.0}, {:.0}) on {surface:?}", c[0], c[1], c[2])));
        ev.push(Event::Sound { name: "C_GRFRAG.WAV".to_string(), at: Some(c), gain: 1.0 });

        // Characters: killed close in, hurt further out, and shielded by walls
        for i in 0..self.npcs.len() {
            if self.npcs[i].health.is_dead() {
                continue;
            }
            let Some(chars) = &self.characters else { break };
            let place = chars.placement(&self.npcs[i]);
            let chest = [place.pos[0], place.pos[1] + 100.0, place.pos[2]];
            let head = [place.pos[0], place.pos[1] + 160.0, place.pos[2]];
            let strength = grenade::blast_strength(dist3(c, chest));
            if strength < 0.02 || !(grenade::clear_between(&self.world, c, chest) || grenade::clear_between(&self.world, c, head)) {
                continue;
            }
            let from = place.facing_of_shot(unit3([chest[0] - c[0], chest[1] - c[1], chest[2] - c[2]]));
            next_rng(&mut self.rng);
            let result = self.npcs[i].health.apply_blast(100.0 * strength, from, now, (self.rng >> 33) as u32);
            self.target_hp = Some(self.npcs[i].health.hp);
            ev.push(Event::Log(format!("t={now:.2} grenade hit character {i} at {:.0} units: {result:?}, hp now {}", dist3(c, chest), self.npcs[i].health.hp)));
            if matches!(result, HitResult::Killed) {
                self.kills += 1;
                self.announce("GRENADE KILL", 1.2);
            }
            let (name, n) = if matches!(result, HitResult::Killed) { ("re_ded", 2) } else { ("re_wnd", 3) };
            ev.push(Event::Sound { name: format!("English/{name}{}.wav", 1 + (self.rng >> 40) as u32 % n), at: Some(self.npcs[i].pos), gain: 1.0 });
        }

        // Glass in reach breaks, the nearest panes first
        let mut panes_near: Vec<(usize, f32)> = self.panes.iter().enumerate()
            .filter(|(i, _)| self.broken_at[*i].is_none())
            .map(|(i, p)| (i, p.distance_to(c)))
            .filter(|(_, d)| *d <= grenade::GLASS_REACH)
            .collect();
        panes_near.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut heard = 0;
        for (gi, _) in panes_near {
            let at = self.panes[gi].center();
            if !grenade::clear_between(&self.world, c, at) {
                continue;
            }
            self.broken_at[gi] = Some(now);
            self.world.switch_off(&self.panes[gi].name);
            next_rng(&mut self.rng);
            ev.push(Event::Glass { pane: gi, at, dir: unit3([at[0] - c[0], at[1] - c[1], at[2] - c[2]]), seed: self.rng });
            if heard < 3 {
                heard += 1;
                ev.push(Event::Sound { name: "C_GBRKMD.WAV".to_string(), at: Some(at), gain: 1.0 });
            }
        }

        // The player: shaken and dazzled more the nearer, and less behind a wall
        if controlled {
            let eye = self.eye();
            let d = dist3(c, eye);
            let open = grenade::clear_between(&self.world, c, eye);
            let shake = grenade::shake(d, open);
            let veil = if open { ((900.0 - d) / 900.0).clamp(0.0, 1.0) } else { 0.0 };
            ev.push(Event::PlayerBlast { shake, veil });
        }
    }

    /// Work out which rooms the player is in and make only those solid. The view is the
    /// player's, or `observer` when it is flying free.
    fn track_rooms(&mut self, observer: Option<V3>) {
        // The room the player is in: the point just above the feet (the view when flying),
        // low enough to land inside the thin slabs that stairs and ramps use. Where no room
        // claims it, the last room stays.
        let (probe, eye) = match observer {
            Some(o) => (o, o),
            None => {
                let eye_y = self.player.pos[1] + self.eye_height;
                ([self.player.pos[0], eye_y - (self.eye_height - 10.0), self.player.pos[2]], [self.player.pos[0], eye_y, self.player.pos[2]])
            }
        };
        if let Some(i) = self.level.rooms.locate(probe, self.current_room) {
            self.current_room = Some(i);
        }
        // Where the player's feet and eyes are, for portals they are standing in
        self.body = [probe, eye];
        // Only the player's room and the rooms its portals lead to can stop movement or
        // bullets: the level overlaps itself, and the exterior of a plane must not block
        // anyone inside it
        self.player_rooms = self.visibility.player_rooms(&self.level, probe, self.current_room);
        // The room drawing starts from: the one whose floor the player stands on, as
        // overlapping rooms (a stairwell beside a hall, an upstairs over a downstairs)
        // all claim the same spot. Airborne, the last one stays; flying, the smallest
        // room around the view.
        let controlled = observer.is_none();
        let floor_room = if controlled {
            self.world.floor_room([probe[0], probe[1] - 10.0, probe[2]]).filter(|&n| self.visibility.is_room(n))
        } else {
            None
        };
        let render_rooms: Vec<u32> = match floor_room {
            Some(n) => vec![n],
            None if controlled && !self.last_render_rooms.is_empty() => self.last_render_rooms.clone(),
            None => self.current_room.and_then(|i| self.visibility.number_of(i)).map(|n| vec![n]).unwrap_or_else(|| self.player_rooms.clone()),
        };
        self.last_render_rooms = render_rooms.clone();
        // What stops the player: their room, the rooms through its portals but only
        // beyond them, and the other rooms they are well inside (they can hold the floor)
        let wanted: Option<Vec<SolidRoom>> = if self.culling && !render_rooms.is_empty() {
            Some(self.visibility.solid_rooms(&self.level, &render_rooms, &self.player_rooms, &self.body))
        } else {
            None
        };
        self.render_rooms = render_rooms;
        if wanted != self.collision_rooms {
            self.world.set_solid_rooms(wanted.as_deref());
            self.collision_rooms = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::Params;

    /// A sim of the demo room with nobody in it but the player
    fn demo() -> Sim {
        let level = Arc::new(crate::demo::map());
        let world = CollisionWorld::from_map(&level, Params::default());
        let config = Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        Sim::new(level, world, config, None, Vec::new(), [0.0, 0.0, -350.0])
    }

    fn holding() -> Input {
        Input { run: true, ..Input::default() }
    }

    /// Run `seconds` of ticks with the same input, keeping every event
    fn run(sim: &mut Sim, input: &Input, seconds: f32) -> Vec<Event> {
        (0..(seconds / TICK).round() as usize).flat_map(|_| sim.tick(input, TICK)).collect()
    }

    #[test]
    fn the_trigger_fires_at_the_guns_rate_until_the_magazine_is_empty() {
        let mut sim = demo();
        let fire = Input { fire: true, ..holding() };
        run(&mut sim, &fire, 1.0);
        // About 800 rounds a minute
        assert!((12..=15).contains(&(30 - sim.ammo)), "{} rounds left", sim.ammo);
        run(&mut sim, &fire, 2.0);
        // Dry after 2.25 s, so the reload has begun and no round is fired during it
        assert!(sim.reload_until.is_some());
        assert_eq!(sim.ammo, 0);
    }

    #[test]
    fn a_reload_takes_its_time_then_fills_the_magazine_from_the_reserve() {
        let mut sim = demo();
        sim.ammo = 10;
        sim.tick(&Input { reload: true, ..holding() }, TICK);
        assert!(sim.reload_until.is_some());
        run(&mut sim, &holding(), RELOAD_SECONDS - 0.2);
        assert_eq!(sim.ammo, 10);
        run(&mut sim, &holding(), 0.4);
        assert_eq!((sim.ammo, sim.reserve, sim.reload_until), (30, 70, None));
    }

    #[test]
    fn a_thrown_grenade_goes_off_once_when_its_fuse_runs_out() {
        let mut sim = demo();
        let mut events = sim.tick(&Input { grenade: true, fire: true, ..holding() }, TICK);
        assert!(sim.equipped_grenade);
        // Wind up for half a second, let go, and wait out the throw and the fuse
        events.extend(run(&mut sim, &Input { fire: true, ..holding() }, 0.5));
        events.extend(run(&mut sim, &holding(), 5.0));
        let blasts = events.iter().filter(|e| matches!(e, Event::Blast { .. })).count();
        assert_eq!(blasts, 1);
        assert_eq!(sim.grenades_left, 2);
        assert!(sim.thrown.is_empty());
    }

    #[test]
    fn the_player_walks_and_stops_at_a_wall() {
        let mut sim = demo();
        // Facing +Z (yaw 0), the far wall is 850 units off
        run(&mut sim, &Input { forward: true, ..holding() }, 4.0);
        assert!(sim.player.pos[2] > 300.0 && sim.player.pos[2] < 500.0, "{:?}", sim.player.pos);
        assert!(sim.moved_speed < 1.0, "still moving at {}", sim.moved_speed);
    }

    /// Everything a replay would have to reproduce, to the bit
    fn fingerprint(sim: &Sim) -> Vec<u32> {
        let mut v: Vec<u32> = sim.player.pos.iter().map(|f| f.to_bits()).collect();
        v.push(sim.time.to_bits());
        v.push(sim.ammo);
        v.push(sim.reserve);
        v.push(sim.grenades_left);
        v.push(sim.rng as u32);
        v.push((sim.rng >> 32) as u32);
        for g in &sim.thrown {
            v.extend(g.pos.iter().chain(g.vel.iter()).map(|f| f.to_bits()));
        }
        v
    }

    #[test]
    fn the_same_inputs_give_the_same_game() {
        let script = |sim: &mut Sim| -> Vec<String> {
            let mut log = Vec::new();
            for tick in 0..900 {
                let input = Input {
                    yaw: (tick as f32 * 0.7).sin() * 90.0,
                    pitch: (tick as f32 * 0.3).sin() * 20.0,
                    forward: tick % 200 < 120,
                    left: tick % 90 < 20,
                    run: tick % 300 < 200,
                    fire: tick % 150 < 60,
                    grenade: tick == 300,
                    reload: tick % 400 == 399,
                    ..Input::default()
                };
                log.extend(sim.tick(&input, TICK).iter().map(|e| format!("{e:?}")));
            }
            log
        };
        let (mut a, mut b) = (demo(), demo());
        let (log_a, log_b) = (script(&mut a), script(&mut b));
        assert!(!log_a.is_empty());
        assert_eq!(log_a, log_b);
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }
}
