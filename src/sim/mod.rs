//! The game itself, apart from how it looks and sounds: the players, their weapons and
//! grenades, the characters, doors and glass, and what a tick of time does to them.
//!
//! The viewer (or a server) feeds the sim an `Input` for every player on every `TICK` and
//! gets back `Event`s: things that happened, which it draws, plays sounds for and prints. The
//! sim never reads a clock, a window or a sound device, and takes all its randomness from its
//! own generator, so the same inputs always give the same game. That is what lets a recorded
//! run be played again, and lets a server and its clients agree.

pub mod characters;
pub mod movement;

use std::collections::VecDeque;
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

/// A player's health at the start, and how long a dead player waits to come back
pub const PLAYER_HP: f32 = 100.0;
pub const RESPAWN_SECONDS: f32 = 3.0;

/// How wide a player is to a bullet
const HIT_RADIUS: f32 = 25.0;

/// How many ticks of where players have been are kept, to aim at where they were
const PAST_TICKS: usize = 40;

/// A grenade throw: from the start of the swing to the end, and how far into it the grenade
/// leaves the hand (0..1)
pub const THROW_SECONDS: f32 = 0.85;
pub const THROW_RELEASE: f32 = 0.35;

/// How far away the door being opened or closed can be
const DOOR_REACH: f32 = 220.0;

/// The most players in a game
pub const MAX_PLAYERS: usize = 8;

/// A grenade throw under way
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Throw {
    pub start: f32,
    /// How long the throw was held for before letting go (0..1)
    pub charge: f32,
    /// Whether the grenade has left the hand
    pub thrown: bool,
}

/// What a player is doing during one tick
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// Where the player looks, in 65536ths of a turn (see `set_look`)
    pub yaw: u16,
    pub pitch: u16,
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

/// An angle in degrees, in 65536ths of a turn (about 0.0055 degrees), wrapping around a full turn
pub fn quantize(degrees: f32) -> u16 {
    ((degrees / 360.0).rem_euclid(1.0) * 65536.0).round() as u32 as u16
}

/// The angle in degrees, from 0 up to a full turn. Exact, since 360/65536 is a short binary fraction.
pub fn dequantize(angle: u16) -> f32 {
    angle as f32 * (360.0 / 65536.0)
}

impl Input {
    /// Look this way (degrees). The sim only ever sees the angles to the nearest 65536th of a
    /// turn, so a recorded or sent input is exactly the one the sim used.
    pub fn set_look(&mut self, yaw: f32, pitch: f32) {
        self.yaw = quantize(yaw);
        self.pitch = quantize(pitch);
    }

    /// Where the player looks, in degrees: yaw from 0 to 360, and pitch from -180 to 180
    pub fn look(&self) -> (f32, f32) {
        (dequantize(self.yaw), dequantize(self.pitch) - if self.pitch >= 0x8000 { 360.0 } else { 0.0 })
    }

    /// The same input a tick later: what is held stays held, and the presses are over
    pub fn held(&self) -> Input {
        Input { use_key: false, reload: false, gun: false, grenade: false, toggle_culling: false, ..*self }
    }
}

/// What killed a player
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cause {
    Shot,
    HeadShot,
    Grenade,
}

/// Something that happened in a tick
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A sound: at a point in the level, or with no place (the player's own)
    Sound { name: String, at: Option<V3>, gain: f32 },
    /// A line for the log
    Log(String),
    /// A player fired: where the shot left and where it ended, and whether it struck a character
    Shot { player: usize, start: V3, end: V3, hit_character: bool },
    /// A bullet struck the level. `seed` is for the effects it makes, `dist` how far it flew.
    BulletHit { point: V3, normal: V3, dir: V3, surface: Surface, material: String, dist: f32, seed: u64 },
    /// A pane broke where `at` is, struck along `dir`
    Glass { pane: usize, at: V3, dir: V3, seed: u64 },
    /// A grenade went off. `on_surface` is false in mid-air, where there is nothing to scorch.
    Blast { point: V3, normal: V3, surface: Surface, on_surface: bool, seed: u64 },
    /// What a blast did to a player: how hard it shook them, and how white it made their view
    PlayerBlast { player: usize, shake: f32, veil: f32 },
    /// A player died: who killed them (none if they did it to themselves) and how
    Kill { killer: Option<usize>, victim: usize, cause: Cause },
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
    /// Rounds in each player's magazine at the start
    pub ammo: u32,
    pub reserve: u32,
    pub grenades: u32,
    /// Scales the players' speed
    pub speed_scale: f32,
    pub seed: u64,
    /// Only a player's room and its neighbors are solid to them (the viewer's V key)
    pub culling: bool,
}

/// A grenade in flight, and who threw it
#[derive(Clone, Debug)]
pub struct Thrown {
    pub grenade: Grenade,
    pub owner: usize,
}

/// One player: their body, weapon, grenades and where they are in the level
pub struct PlayerState {
    /// Whether anyone is playing this slot
    pub active: bool,
    pub player: Player,
    /// Where the player looked in the last tick, as an `Input` says
    pub look: (u16, u16),
    /// Where the view was last tick if the player was not in control
    pub last_observer: Option<V3>,
    /// Whether the player is in control, as of the last tick
    pub controlled: bool,
    /// How fast the player moved in the last tick, in units per second
    pub moved_speed: f32,

    // Weapon
    pub ammo: u32,
    pub reserve: u32,
    pub reload_until: Option<f32>,
    pub next_shot: f32,
    /// The muzzle flashes until this time
    pub flash_until: f32,
    /// Whether the trigger is held with the gun out, as of the last tick
    pub gun_trigger: bool,
    pub kills: u32,

    // Health, when players can hurt each other (see `Sim::pvp`)
    pub hp: f32,
    /// When a dead player comes back, or none if they are alive
    pub respawn_at: Option<f32>,
    pub deaths: u32,

    // Grenades
    pub equipped_grenade: bool,
    pub grenades_left: u32,
    pub charge_from: Option<f32>,
    pub throw: Option<Throw>,

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
    /// What is solid to this player (see `Sim::culling`)
    pub collision_rooms: Option<Vec<SolidRoom>>,
    last_render_rooms: Vec<u32>,
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
    /// Whether players can hurt each other (a server's game: it is off when playing alone)
    pub pvp: bool,
    /// Where players come back after they die
    pub spawns: Vec<V3>,
    /// For each player, how many ticks back in time the others were when they took their shot,
    /// as that player saw them (a server sets it before each tick; none means no looking back)
    pub rewind: Vec<f32>,
    /// Where each living player was at the end of each of the latest ticks, oldest first
    past: VecDeque<Vec<Option<V3>>>,
    eye_height: f32,
    /// What a player starts with
    pub magazine: u32,
    loadout: (u32, u32, u32),

    pub players: Vec<PlayerState>,
    pub thrown: Vec<Thrown>,
    /// What is solid in the world right now (see `PlayerState::collision_rooms`)
    applied_rooms: Option<Vec<SolidRoom>>,

    pub characters: Option<Characters>,
    pub npcs: Vec<Npc>,

    pub doors: Vec<Door>,
    pub door_states: Vec<DoorState>,
    /// Whether each portal is shut by a closed door
    pub blocked_portals: Vec<bool>,
    pub panes: Vec<Pane>,
    /// When each pane broke
    pub broken_at: Vec<Option<f32>>,
}

impl Sim {
    /// A game with nobody in it yet: see `add_player`
    pub fn new(level: Arc<Map>, world: CollisionWorld, config: Config, characters: Option<Characters>, npcs: Vec<Npc>) -> Sim {
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
            pvp: false,
            rewind: Vec::new(),
            past: VecDeque::new(),
            spawns: Vec::new(),
            eye_height,
            magazine: config.magazine,
            loadout: (config.ammo, config.reserve, config.grenades),
            players: Vec::new(),
            thrown: Vec::new(),
            applied_rooms: None,
            characters,
            npcs,
        }
    }

    /// A player with the starting loadout, on their feet at `feet`
    fn new_player(&self, feet: V3) -> PlayerState {
        let (ammo, reserve, grenades) = self.loadout;
        PlayerState {
            active: true,
            player: Player::new(feet),
            look: (0, 0),
            last_observer: None,
            controlled: true,
            moved_speed: 0.0,
            ammo,
            reserve,
            reload_until: None,
            next_shot: 0.0,
            flash_until: 0.0,
            gun_trigger: false,
            kills: 0,
            hp: PLAYER_HP,
            respawn_at: None,
            deaths: 0,
            equipped_grenade: false,
            grenades_left: grenades,
            charge_from: None,
            throw: None,
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

    /// Put a new player on their feet at `feet`. Returns their slot, or none when the game is full.
    pub fn add_player(&mut self, feet: V3) -> Option<usize> {
        let fresh = self.new_player(feet);
        if let Some(i) = self.players.iter().position(|p| !p.active) {
            self.players[i] = fresh;
            return Some(i);
        }
        if self.players.len() >= MAX_PLAYERS {
            return None;
        }
        self.players.push(fresh);
        Some(self.players.len() - 1)
    }

    /// Make sure `slot` is a player, standing at `feet` if it was not one. For a copy of a
    /// game that someone else runs: the slots are theirs to choose.
    pub fn activate_slot(&mut self, slot: usize, feet: V3) {
        assert!(slot < MAX_PLAYERS, "there is no slot {slot}");
        while self.players.len() <= slot {
            let vacant = PlayerState { active: false, ..self.new_player(feet) };
            self.players.push(vacant);
        }
        if !self.players[slot].active {
            self.players[slot] = self.new_player(feet);
        }
    }

    /// Move a player as a tick would, and nothing else: what a client does with its own player's
    /// input to see the result before the server has answered
    pub fn predict(&mut self, slot: usize, input: &Input, dt: f32) {
        if !self.players.get(slot).is_some_and(|p| p.active) {
            return;
        }
        let yaw = input.look().0;
        self.players[slot].look = (input.yaw, input.pitch);
        if self.players[slot].respawn_at.is_some() {
            return;
        }
        self.players[slot].controlled = input.observer.is_none();
        self.use_rooms_of(slot);
        self.move_player(slot, input, dt, right(yaw));
        self.track_rooms(slot, input.observer);
    }

    /// A player leaves. Their slot is free for the next to join, and their grenades stay in the air.
    pub fn remove_player(&mut self, slot: usize) {
        if let Some(p) = self.players.get_mut(slot) {
            p.active = false;
        }
        self.apply_union_rooms();
    }

    /// Where a player's eyes are
    pub fn eye(&self, slot: usize) -> V3 {
        let p = self.players[slot].player.pos;
        [p[0], p[1] + self.eye_height, p[2]]
    }

    /// Set every door this far open (0 shut, 1 open)
    pub fn set_doors_open(&mut self, open: f32) {
        for (i, s) in self.door_states.iter_mut().enumerate() {
            s.open = open.clamp(0.0, 1.0);
            s.opening = open > 0.5;
            self.world.set_door_open(i, s.open);
        }
    }

    /// Whether the full-auto firing sound should be running for a player
    pub fn firing(&self, slot: usize) -> bool {
        let p = &self.players[slot];
        p.controlled && p.gun_trigger && p.reload_until.is_none() && p.ammo > 0
    }

    /// A fingerprint of everything that decides what happens next, to compare two runs of the
    /// game: equal inputs must leave equal fingerprints
    pub fn checksum(&self) -> u64 {
        let mut h = Fnv::new();
        h.f32(self.time);
        h.u64(self.rng);
        h.u32(self.culling as u32);
        for p in self.players.iter().filter(|p| p.active) {
            h.v3(p.player.pos);
            h.f32(p.player.vel_y);
            h.u32(p.player.on_ground as u32);
            h.u32(p.ammo);
            h.u32(p.reserve);
            h.opt_f32(p.reload_until);
            h.f32(p.next_shot);
            h.f32(p.flash_until);
            h.u32(p.kills);
            h.f32(p.hp);
            h.opt_f32(p.respawn_at);
            h.u32(p.deaths);
            h.u32(p.equipped_grenade as u32);
            h.u32(p.grenades_left);
            h.opt_f32(p.charge_from);
            match p.throw {
                None => h.u32(0),
                Some(t) => { h.u32(1); h.f32(t.start); h.f32(t.charge); h.u32(t.thrown as u32); }
            }
            h.u32(p.current_room.map_or(u32::MAX, |r| r as u32));
        }
        for t in &self.thrown {
            let g = &t.grenade;
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
        h.0
    }

    fn announce(&mut self, slot: usize, text: &str, seconds: f32) {
        self.players[slot].message = (text.to_string(), self.time + seconds);
    }

    fn surface_of(&self, material: Option<usize>) -> Surface {
        material.and_then(|m| self.level.materials.materials.get(m)).map_or(Surface::Concrete, |m| Surface::classify(&m.filename))
    }

    /// Make this what is solid in the world, if it is not already
    fn set_applied_rooms(&mut self, wanted: Option<Vec<SolidRoom>>) {
        if wanted != self.applied_rooms {
            self.world.set_solid_rooms(wanted.as_deref());
            self.applied_rooms = wanted;
        }
    }

    /// What is solid to one player is what moves and shoots them
    fn use_rooms_of(&mut self, slot: usize) {
        if self.players[slot].collision_rooms != self.applied_rooms {
            let wanted = self.players[slot].collision_rooms.clone();
            self.set_applied_rooms(wanted);
        }
    }

    /// What everyone's rooms add up to, for what belongs to no one: grenades in the air. When
    /// anyone has the whole level solid, so has it.
    fn apply_union_rooms(&mut self) {
        let mut all: Vec<SolidRoom> = Vec::new();
        let mut any = false;
        let mut whole = false;
        for p in self.players.iter().filter(|p| p.active) {
            any = true;
            match &p.collision_rooms {
                None => whole = true,
                Some(rooms) => all.extend(rooms.iter().cloned()),
            }
        }
        let wanted = if !any || whole { None } else { Some(all) };
        self.set_applied_rooms(wanted);
    }

    /// Run the game for one tick. `inputs` has what each player slot did; a slot with no input
    /// does nothing.
    pub fn tick(&mut self, inputs: &[Input], dt: f32) -> Vec<Event> {
        let mut ev: Vec<Event> = Vec::new();
        let none = Input::default();
        let slots: Vec<usize> = (0..self.players.len()).filter(|&i| self.players[i].active).collect();
        // A dead player does nothing but look where they were looking
        let mut given: Vec<Input> = (0..self.players.len()).map(|i| inputs.get(i).copied().unwrap_or(none)).collect();
        for &i in &slots {
            let p = &self.players[i];
            if p.respawn_at.is_some() {
                given[i] = Input { yaw: p.look.0, pitch: p.look.1, ..none };
            }
        }
        let input_of = |i: usize| given[i];

        // Keys pressed since the last tick
        for &i in &slots {
            let input = input_of(i);
            let controlled = input.observer.is_none();
            let p = &mut self.players[i];
            p.controlled = controlled;
            p.look = (input.yaw, input.pitch);
            if controlled {
                // Control returns: the player drops in from wherever the view was
                if let Some(o) = p.last_observer.take() {
                    p.player = Player::new([o[0], o[1] - self.eye_height, o[2]]);
                }
            } else {
                p.last_observer = input.observer;
            }
            if input.toggle_culling {
                self.culling = !self.culling;
            }
            let p = &mut self.players[i];
            if input.gun && p.throw.is_none() {
                // Back to the gun
                p.equipped_grenade = false;
                p.charge_from = None;
            }
            if input.grenade && controlled && p.throw.is_none() && p.reload_until.is_none() {
                p.equipped_grenade = true;
                if p.grenades_left == 0 {
                    self.announce(i, "NO GRENADES", 1.0);
                }
            }
            let p = &mut self.players[i];
            // Reload: takes time, then fills the magazine from the reserve
            if input.reload && controlled && !p.equipped_grenade && p.reload_until.is_none() && p.ammo < self.magazine && p.reserve > 0 {
                p.reload_until = Some(self.time + RELOAD_SECONDS);
            }
            // The trigger fires the gun, unless a grenade is out
            p.gun_trigger = input.fire && !p.equipped_grenade;
        }
        self.time += dt;

        for &i in &slots {
            if self.players[i].respawn_at.is_some_and(|t| self.time >= t) {
                self.respawn(i);
            }
        }

        for &i in &slots {
            let input = input_of(i);
            if input.use_key && input.observer.is_none() {
                self.use_door(i, forward(input.look().0, input.look().1), &mut ev);
            }
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
        for &i in &slots {
            let p = &mut self.players[i];
            if p.reload_until.is_some_and(|t| self.time >= t) {
                let take = (self.magazine - p.ammo).min(p.reserve);
                p.ammo += take;
                p.reserve -= take;
                p.reload_until = None;
            }
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

        for &i in &slots {
            let input = input_of(i);
            if input.observer.is_none() {
                let (yaw, pitch) = input.look();
                let (forward_cam, right_cam) = (forward(yaw, pitch), right(yaw));
                self.use_rooms_of(i);
                self.move_player(i, &input, dt, right_cam);
                if self.players[i].respawn_at.is_none() {
                    self.throw_grenade(i, &input, forward_cam, right_cam, &mut ev);
                    self.shoot(i, forward_cam, &mut ev);
                }
            }
        }

        self.apply_union_rooms();
        self.update_grenades(dt, &mut ev);
        for &i in &slots {
            self.track_rooms(i, input_of(i).observer);
        }
        self.apply_union_rooms();
        if self.pvp {
            self.past.push_back(self.players.iter().map(|p| (p.active && p.respawn_at.is_none()).then_some(p.player.pos)).collect());
            while self.past.len() > PAST_TICKS {
                self.past.pop_front();
            }
        }
        ev
    }

    /// Where a player was `back` ticks ago (between ticks, if it is not a whole number), or
    /// where they are if `back` is nothing. None if they were not standing then.
    fn position_then(&self, slot: usize, back: f32) -> Option<V3> {
        let now = self.players[slot].player.pos;
        if back <= 0.0 || self.past.is_empty() {
            return Some(now);
        }
        // The newest tick in `past` is the one just finished, which is as good as now
        let back = back.min((self.past.len() - 1) as f32);
        let newer = self.past.len() - 1 - back.floor() as usize;
        let at = |i: usize| self.past[i].get(slot).copied().flatten();
        let (newer_pos, older_pos) = (at(newer), newer.checked_sub(1).and_then(at));
        match (newer_pos, older_pos) {
            (Some(n), Some(o)) => {
                let f = back.fract();
                Some([0, 1, 2].map(|k| n[k] + (o[k] - n[k]) * f))
            }
            (Some(n), None) if back.fract() < 0.5 => Some(n),
            (None, Some(o)) if back.fract() >= 0.5 => Some(o),
            _ => None,
        }
    }

    /// Space or Backspace opens or closes the door the player is looking at
    fn use_door(&mut self, slot: usize, forward_cam: V3, ev: &mut Vec<Event>) {
        let eye = self.eye(slot);
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
                self.announce(slot, &format!("DOOR {}", if opening { "OPENING" } else { "CLOSING" }), 1.2);
                ev.push(Event::Log(format!("t={:.2} door {} {}", self.time, self.doors[i].name, if opening { "opening" } else { "closing" })));
            }
            None => self.announce(slot, "NO DOOR IN REACH", 1.0),
        }
    }

    /// Walk the player, at speeds that come from the animations so the feet do not slide
    fn move_player(&mut self, slot: usize, input: &Input, dt: f32, right_cam: V3) {
        let yaw_rad = input.look().0.to_radians();
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

        let before = self.players[slot].player.pos;
        self.world.step(&mut self.players[slot].player, [vel[0] * dt, -vel[1] * dt], dt);
        let after = self.players[slot].player.pos;
        self.players[slot].moved_speed = ((after[0] - before[0]).powi(2) + (after[2] - before[2]).powi(2)).sqrt() / dt.max(1e-4);
    }

    /// A grenade: hold the trigger to wind up, let go to throw. The swing plays for a moment,
    /// and the grenade leaves the hand part of the way in.
    fn throw_grenade(&mut self, slot: usize, input: &Input, forward_cam: V3, right_cam: V3, ev: &mut Vec<Event>) {
        if !self.players[slot].equipped_grenade {
            return;
        }
        let now = self.time;
        let mut finished = false;
        match self.players[slot].throw {
            None => {
                let p = &mut self.players[slot];
                if input.fire && p.grenades_left > 0 {
                    p.charge_from.get_or_insert(now);
                } else if let Some(from) = p.charge_from.take() {
                    let charge = ((now - from) / grenade::CHARGE_SECONDS).clamp(0.0, 1.0);
                    p.throw = Some(Throw { start: now, charge, thrown: false });
                } else if input.fire && p.grenades_left == 0 && now >= p.message.1 {
                    self.announce(slot, "NO GRENADES", 0.8);
                }
            }
            Some(mut th) => {
                let progress = (now - th.start) / THROW_SECONDS;
                if !th.thrown && progress >= THROW_RELEASE {
                    th.thrown = true;
                    self.players[slot].grenades_left -= 1;
                    let eye = self.eye(slot);
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
                    self.thrown.push(Thrown { grenade: Grenade::thrown(from, aim, th.charge, [0.0; 3]), owner: slot });
                    ev.push(Event::Log(format!("t={now:.2} threw a grenade at {:.0}% power", th.charge * 100.0)));
                }
                self.players[slot].throw = Some(th);
                finished = progress >= 1.0;
            }
        }
        if finished {
            self.players[slot].throw = None;
            if self.players[slot].grenades_left == 0 {
                self.players[slot].equipped_grenade = false;
                self.announce(slot, "OUT OF GRENADES", 1.2);
            }
        }
    }

    /// Shooting: a ray from the eye along the view, stopped by walls
    fn shoot(&mut self, slot: usize, forward_cam: V3, ev: &mut Vec<Event>) {
        let gun_trigger = self.players[slot].gun_trigger;
        if gun_trigger && self.players[slot].reload_until.is_none() && self.players[slot].ammo == 0 && self.players[slot].reserve > 0 {
            self.players[slot].reload_until = Some(self.time + RELOAD_SECONDS);
        }
        let p = &self.players[slot];
        if !(gun_trigger && p.reload_until.is_none() && p.ammo > 0 && self.time >= p.next_shot) {
            return;
        }
        let now = self.time;
        let p = &mut self.players[slot];
        p.ammo -= 1;
        p.next_shot = now + 60.0 / 800.0; // the MP5K fires about 800 rounds a minute
        p.flash_until = now + 0.05;

        let eye = self.eye(slot);
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

        let hit_player = self.player_hit(slot, eye, dir);

        // Glass in the way breaks and lets the bullet through
        loop {
            let glass_hit = self.panes.iter().enumerate()
                .filter(|(i, _)| self.broken_at[*i].is_none())
                .filter_map(|(i, p)| p.raycast(eye, dir).map(|d| (i, d)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((gi, gd)) = glass_hit else { break };
            // Something solid, or a character, is in front of the glass
            if wall.is_some_and(|w| w < gd - 0.5) || best.as_ref().is_some_and(|(_, h)| h.distance < gd) || hit_player.is_some_and(|h| h.1 < gd) {
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
        // Another player in the way, nearer than whatever else was hit
        let hit_player = hit_player.filter(|h| wall.is_none_or(|w| h.1 <= w) && best.as_ref().is_none_or(|(_, b)| h.1 < b.distance));
        if let Some((victim, dist, part)) = hit_player {
            let point = [eye[0] + dir[0] * dist, eye[1] + dir[1] * dist, eye[2] + dir[2] * dist];
            self.hurt_player(victim, Some(slot), combat::damage(part), point, if part == combat::BodyPart::Head { Cause::HeadShot } else { Cause::Shot }, ev);
            ev.push(Event::Shot { player: slot, start, end: point, hit_character: true });
            return;
        }
        match best {
            Some((i, hit)) if wall.map_or(true, |w| hit.distance <= w) => {
                next_rng(&mut self.rng);
                let result = self.npcs[i].health.apply_hit(&hit, now, (self.rng >> 33) as u32);
                self.players[slot].target_hp = Some(self.npcs[i].health.hp);
                ev.push(Event::Log(format!("t={now:.2} shot hit character {i}: {:?} from the {:?}, {:?}, hp now {}", hit.part, hit.facing, result, self.npcs[i].health.hp)));
                let text = match result {
                    HitResult::Killed => {
                        self.players[slot].kills += 1;
                        if hit.part == combat::BodyPart::Head { "HEAD SHOT KILL".to_string() } else { "KILLED".to_string() }
                    }
                    _ => format!("HIT {:?}", hit.part).to_uppercase(),
                };
                self.announce(slot, &text, 1.2);
                ev.push(Event::Sound { name: "C_BHTHT.WAV".to_string(), at: Some(hit.point), gain: 1.0 });
                // Voices: a grunt when hurt, a cry when killed
                let (name, n) = if matches!(result, HitResult::Killed) { ("re_ded", 2) } else { ("re_wnd", 3) };
                let pick = 1 + (self.rng >> 40) as u32 % n;
                ev.push(Event::Sound { name: format!("English/{name}{pick}.wav"), at: Some(self.npcs[i].pos), gain: 1.0 });
                ev.push(Event::Shot { player: slot, start, end: hit.point, hit_character: true });
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
                ev.push(Event::Shot { player: slot, start, end: [eye[0] + dir[0] * d, eye[1] + dir[1] * d, eye[2] + dir[2] * d], hit_character: false });
            }
        }
    }

    /// Grenades in the air: they bounce off the level and go off when the fuse runs out
    fn update_grenades(&mut self, dt: f32, ev: &mut Vec<Event>) {
        let now = self.time;
        let mut blasts: Vec<(V3, usize)> = Vec::new();
        let mut thrown = std::mem::take(&mut self.thrown);
        for t in thrown.iter_mut() {
            let g = &mut t.grenade;
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
                blasts.push((g.pos, t.owner));
            }
        }
        thrown.retain(|t| !t.grenade.goes_off());
        self.thrown = thrown;
        for (c, owner) in blasts {
            self.blast(c, owner, ev);
        }
    }

    /// A grenade thrown by `owner` goes off at `c`
    fn blast(&mut self, c: V3, owner: usize, ev: &mut Vec<Event>) {
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
            if let Some(p) = self.players.get_mut(owner) {
                p.target_hp = Some(self.npcs[i].health.hp);
            }
            ev.push(Event::Log(format!("t={now:.2} grenade hit character {i} at {:.0} units: {result:?}, hp now {}", dist3(c, chest), self.npcs[i].health.hp)));
            if matches!(result, HitResult::Killed) {
                if let Some(p) = self.players.get_mut(owner) {
                    p.kills += 1;
                }
                if owner < self.players.len() {
                    self.announce(owner, "GRENADE KILL", 1.2);
                }
            }
            let (name, n) = if matches!(result, HitResult::Killed) { ("re_ded", 2) } else { ("re_wnd", 3) };
            ev.push(Event::Sound { name: format!("English/{name}{}.wav", 1 + (self.rng >> 40) as u32 % n), at: Some(self.npcs[i].pos), gain: 1.0 });
        }

        // Players are hurt as characters are, if they can be
        if self.pvp {
            for i in 0..self.players.len() {
                if !self.players[i].active || self.players[i].respawn_at.is_some() {
                    continue;
                }
                let feet = self.players[i].player.pos;
                let chest = [feet[0], feet[1] + 100.0, feet[2]];
                let head = [feet[0], feet[1] + 160.0, feet[2]];
                let strength = grenade::blast_strength(dist3(c, chest));
                if strength < 0.02 || !(grenade::clear_between(&self.world, c, chest) || grenade::clear_between(&self.world, c, head)) {
                    continue;
                }
                self.hurt_player(i, Some(owner), PLAYER_HP * strength, chest, Cause::Grenade, ev);
            }
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

        // The players: shaken and dazzled more the nearer, and less behind a wall
        for i in 0..self.players.len() {
            if !self.players[i].active || !self.players[i].controlled {
                continue;
            }
            let eye = self.eye(i);
            let d = dist3(c, eye);
            let open = grenade::clear_between(&self.world, c, eye);
            let shake = grenade::shake(d, open);
            let veil = if open { ((900.0 - d) / 900.0).clamp(0.0, 1.0) } else { 0.0 };
            ev.push(Event::PlayerBlast { player: i, shake, veil });
        }
    }

    /// The nearest other player that a bullet from `eye` along `dir` hits, if players can be
    /// hit: who, how far along, and where on the body
    fn player_hit(&self, shooter: usize, eye: V3, dir: V3) -> Option<(usize, f32, combat::BodyPart)> {
        if !self.pvp {
            return None;
        }
        let height = self.world.params().height;
        let a = dir[0] * dir[0] + dir[2] * dir[2];
        if a < 1e-9 {
            return None;
        }
        let back = self.rewind.get(shooter).copied().unwrap_or(0.0);
        (0..self.players.len())
            .filter(|&i| i != shooter && self.players[i].active && self.players[i].respawn_at.is_none())
            .filter_map(|i| {
                // The body is an upright cylinder, where the shooter saw it
                let feet = self.position_then(i, back)?;
                let (ox, oz) = (eye[0] - feet[0], eye[2] - feet[2]);
                let b = 2.0 * (ox * dir[0] + oz * dir[2]);
                let c = ox * ox + oz * oz - HIT_RADIUS * HIT_RADIUS;
                let disc = b * b - 4.0 * a * c;
                if disc < 0.0 {
                    return None;
                }
                let t = (-b - disc.sqrt()) / (2.0 * a);
                let up = (eye[1] + dir[1] * t - feet[1]) / height;
                if t <= 0.0 || !(0.0..=1.0).contains(&up) {
                    return None;
                }
                let part = match up {
                    u if u >= 0.88 => combat::BodyPart::Head,
                    u if u >= 0.55 => combat::BodyPart::Torso,
                    u if u >= 0.40 => combat::BodyPart::Abdomen,
                    _ => combat::BodyPart::LeftLeg,
                };
                Some((i, t, part))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// Take health from a player. If it is all of it they die, and `attacker` (who is not the
    /// victim) is credited, with the kill on their screen.
    fn hurt_player(&mut self, victim: usize, attacker: Option<usize>, amount: f32, at: V3, cause: Cause, ev: &mut Vec<Event>) {
        let now = self.time;
        let p = &mut self.players[victim];
        if p.respawn_at.is_some() {
            return;
        }
        p.hp = (p.hp - amount).max(0.0);
        let killed = p.hp <= 0.0;
        let hp = p.hp;
        let pos = p.player.pos;
        if let Some(a) = attacker.filter(|&a| a != victim) {
            self.players[a].target_hp = Some(hp);
        }
        next_rng(&mut self.rng);
        let roll = (self.rng >> 40) as u32;
        ev.push(Event::Log(format!("t={now:.2} player {victim} hurt by {amount:.0}{}, hp now {hp:.0}", attacker.map_or(String::new(), |a| format!(" from player {a}")))));
        ev.push(Event::Sound { name: "C_BHTHT.WAV".to_string(), at: Some(at), gain: 1.0 });
        let (name, n) = if killed { ("re_ded", 2) } else { ("re_wnd", 3) };
        ev.push(Event::Sound { name: format!("English/{name}{}.wav", 1 + roll % n), at: Some(pos), gain: 1.0 });
        if !killed {
            return;
        }
        let p = &mut self.players[victim];
        p.respawn_at = Some(now + RESPAWN_SECONDS);
        p.deaths += 1;
        p.equipped_grenade = false;
        p.charge_from = None;
        p.throw = None;
        p.reload_until = None;
        p.gun_trigger = false;
        match attacker.filter(|&a| a != victim) {
            Some(a) => {
                self.players[a].kills += 1;
                let text = match cause {
                    Cause::HeadShot => "HEAD SHOT KILL",
                    Cause::Shot => "KILLED",
                    Cause::Grenade => "GRENADE KILL",
                };
                self.announce(a, text, 1.2);
                self.announce(victim, &format!("KILLED BY PLAYER {}", a + 1), RESPAWN_SECONDS);
            }
            None => self.announce(victim, "YOU KILLED YOURSELF", RESPAWN_SECONDS),
        }
        ev.push(Event::Log(format!("t={now:.2} player {victim} died")));
        ev.push(Event::Kill { killer: attacker.filter(|&a| a != victim), victim, cause });
    }

    /// A dead player comes back at the spawn point farthest from the players still standing, with
    /// the starting loadout and their score
    fn respawn(&mut self, slot: usize) {
        let others: Vec<V3> = (0..self.players.len())
            .filter(|&i| i != slot && self.players[i].active && self.players[i].respawn_at.is_none())
            .map(|i| self.players[i].player.pos)
            .collect();
        let nearest = |s: &V3| others.iter().map(|o| dist3(*s, *o)).fold(f32::INFINITY, f32::min);
        let here = self.players[slot].player.pos;
        let spawn = self.spawns.iter().copied().fold(None, |best: Option<(V3, f32)>, s| {
            let d = nearest(&s);
            if best.is_none_or(|b| d > b.1) { Some((s, d)) } else { best }
        }).map_or(here, |b| b.0);
        let (kills, deaths) = (self.players[slot].kills, self.players[slot].deaths);
        let look = self.players[slot].look;
        self.players[slot] = self.new_player(spawn);
        let p = &mut self.players[slot];
        (p.kills, p.deaths, p.look) = (kills, deaths, look);
    }

    /// Work out which rooms a player is in and which are solid to them. The view is the
    /// player's, or `observer` when it is flying free.
    fn track_rooms(&mut self, slot: usize, observer: Option<V3>) {
        // The room the player is in: the point just above the feet (the view when flying),
        // low enough to land inside the thin slabs that stairs and ramps use. Where no room
        // claims it, the last room stays.
        let pos = self.players[slot].player.pos;
        let (probe, eye) = match observer {
            Some(o) => (o, o),
            None => {
                let eye_y = pos[1] + self.eye_height;
                ([pos[0], eye_y - (self.eye_height - 10.0), pos[2]], [pos[0], eye_y, pos[2]])
            }
        };
        if let Some(i) = self.level.rooms.locate(probe, self.players[slot].current_room) {
            self.players[slot].current_room = Some(i);
        }
        // Where the player's feet and eyes are, for portals they are standing in
        self.players[slot].body = [probe, eye];
        // Only the player's room and the rooms its portals lead to can stop movement or
        // bullets: the level overlaps itself, and the exterior of a plane must not block
        // anyone inside it
        let player_rooms = self.visibility.player_rooms(&self.level, probe, self.players[slot].current_room);
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
        let p = &self.players[slot];
        let render_rooms: Vec<u32> = match floor_room {
            Some(n) => vec![n],
            None if controlled && !p.last_render_rooms.is_empty() => p.last_render_rooms.clone(),
            None => p.current_room.and_then(|i| self.visibility.number_of(i)).map(|n| vec![n]).unwrap_or_else(|| player_rooms.clone()),
        };
        // What stops the player: their room, the rooms through its portals but only
        // beyond them, and the other rooms they are well inside (they can hold the floor)
        let wanted: Option<Vec<SolidRoom>> = if self.culling && !render_rooms.is_empty() {
            Some(self.visibility.solid_rooms(&self.level, &render_rooms, &player_rooms, &self.players[slot].body))
        } else {
            None
        };
        let p = &mut self.players[slot];
        p.last_render_rooms = render_rooms.clone();
        p.player_rooms = player_rooms;
        p.render_rooms = render_rooms;
        p.collision_rooms = wanted;
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
        let mut sim = Sim::new(level, world, config, None, Vec::new());
        sim.add_player([0.0, 0.0, -350.0]);
        sim
    }

    fn holding() -> Input {
        Input { run: true, ..Input::default() }
    }

    /// Run `seconds` of ticks with the same input, keeping every event
    fn run(sim: &mut Sim, input: &Input, seconds: f32) -> Vec<Event> {
        (0..(seconds / TICK).round() as usize).flat_map(|_| sim.tick(&[*input], TICK)).collect()
    }

    #[test]
    fn the_trigger_fires_at_the_guns_rate_until_the_magazine_is_empty() {
        let mut sim = demo();
        let fire = Input { fire: true, ..holding() };
        run(&mut sim, &fire, 1.0);
        // About 800 rounds a minute
        let ammo = sim.players[0].ammo;
        assert!((12..=15).contains(&(30 - ammo)), "{ammo} rounds left");
        run(&mut sim, &fire, 2.0);
        // Dry after 2.25 s, so the reload has begun and no round is fired during it
        assert!(sim.players[0].reload_until.is_some());
        assert_eq!(sim.players[0].ammo, 0);
    }

    #[test]
    fn a_reload_takes_its_time_then_fills_the_magazine_from_the_reserve() {
        let mut sim = demo();
        sim.players[0].ammo = 10;
        sim.tick(&[Input { reload: true, ..holding() }], TICK);
        assert!(sim.players[0].reload_until.is_some());
        run(&mut sim, &holding(), RELOAD_SECONDS - 0.2);
        assert_eq!(sim.players[0].ammo, 10);
        run(&mut sim, &holding(), 0.4);
        let p = &sim.players[0];
        assert_eq!((p.ammo, p.reserve, p.reload_until), (30, 70, None));
    }

    #[test]
    fn a_thrown_grenade_goes_off_once_when_its_fuse_runs_out() {
        let mut sim = demo();
        let mut events = sim.tick(&[Input { grenade: true, fire: true, ..holding() }], TICK);
        assert!(sim.players[0].equipped_grenade);
        // Wind up for half a second, let go, and wait out the throw and the fuse
        events.extend(run(&mut sim, &Input { fire: true, ..holding() }, 0.5));
        events.extend(run(&mut sim, &holding(), 5.0));
        let blasts = events.iter().filter(|e| matches!(e, Event::Blast { .. })).count();
        assert_eq!(blasts, 1);
        assert_eq!(sim.players[0].grenades_left, 2);
        assert!(sim.thrown.is_empty());
    }

    #[test]
    fn the_player_walks_and_stops_at_a_wall() {
        let mut sim = demo();
        // Facing +Z (yaw 0), the far wall is 850 units off
        run(&mut sim, &Input { forward: true, ..holding() }, 4.0);
        let p = &sim.players[0];
        assert!(p.player.pos[2] > 300.0 && p.player.pos[2] < 500.0, "{:?}", p.player.pos);
        assert!(p.moved_speed < 1.0, "still moving at {}", p.moved_speed);
    }

    /// Everything a replay would have to reproduce, to the bit
    fn fingerprint(sim: &Sim) -> Vec<u32> {
        let p = &sim.players[0];
        let mut v: Vec<u32> = p.player.pos.iter().map(|f| f.to_bits()).collect();
        v.push(sim.time.to_bits());
        v.push(p.ammo);
        v.push(p.reserve);
        v.push(p.grenades_left);
        v.push(sim.rng as u32);
        v.push((sim.rng >> 32) as u32);
        for t in &sim.thrown {
            v.extend(t.grenade.pos.iter().chain(t.grenade.vel.iter()).map(|f| f.to_bits()));
        }
        v
    }

    #[test]
    fn the_same_inputs_give_the_same_game() {
        let script = |sim: &mut Sim| -> Vec<String> {
            let mut log = Vec::new();
            for tick in 0..900 {
                let mut input = Input {
                    forward: tick % 200 < 120,
                    left: tick % 90 < 20,
                    run: tick % 300 < 200,
                    fire: tick % 150 < 60,
                    grenade: tick == 300,
                    reload: tick % 400 == 399,
                    ..Input::default()
                };
                input.set_look((tick as f32 * 0.7).sin() * 90.0, (tick as f32 * 0.3).sin() * 20.0);
                log.extend(sim.tick(&[input], TICK).iter().map(|e| format!("{e:?}")));
            }
            log
        };
        let (mut a, mut b) = (demo(), demo());
        let (log_a, log_b) = (script(&mut a), script(&mut b));
        assert!(!log_a.is_empty());
        assert_eq!(log_a, log_b);
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn two_players_move_and_fire_on_their_own() {
        let mut sim = demo();
        let second = sim.add_player([200.0, 0.0, -350.0]).unwrap();
        assert_eq!(second, 1);
        let walker = Input { forward: true, ..holding() };
        let shooter = Input { fire: true, ..holding() };
        for _ in 0..60 {
            sim.tick(&[walker, shooter], TICK);
        }
        let (a, b) = (&sim.players[0], &sim.players[1]);
        assert!(a.player.pos[2] > -350.0 + 300.0 && a.ammo == 30, "{:?} {}", a.player.pos, a.ammo);
        assert!(b.player.pos[2] == -350.0 && b.ammo < 30);
    }

    #[test]
    fn a_player_who_leaves_frees_their_slot() {
        let mut sim = demo();
        sim.add_player([100.0, 0.0, -350.0]);
        sim.remove_player(0);
        sim.tick(&[holding(), holding()], TICK);
        // The slot is empty, so what it was fed does nothing, and the next to join takes it
        assert_eq!(sim.add_player([0.0, 0.0, 0.0]), Some(0));
        let full = (0..MAX_PLAYERS).filter_map(|_| sim.add_player([0.0; 3])).count();
        assert_eq!(full, MAX_PLAYERS - 2);
        assert_eq!(sim.add_player([0.0; 3]), None);
    }

    #[test]
    fn a_blast_shakes_every_player_whoever_threw_the_grenade() {
        let mut sim = demo();
        sim.add_player([300.0, 0.0, -350.0]);
        run(&mut sim, &holding(), 0.1);
        let mut events = sim.tick(&[holding(), Input { grenade: true, fire: true, ..holding() }], TICK);
        events.extend((0..30).flat_map(|_| sim.tick(&[holding(), Input { fire: true, ..holding() }], TICK)));
        events.extend((0..400).flat_map(|_| sim.tick(&[holding(), holding()], TICK)));
        assert_eq!(sim.players[0].grenades_left, 3);
        assert_eq!(sim.players[1].grenades_left, 2);
        // And the blast shook both, whoever threw it
        let shaken: Vec<usize> = events.iter().filter_map(|e| match e { Event::PlayerBlast { player, .. } => Some(*player), _ => None }).collect();
        assert_eq!(shaken, vec![0, 1]);
    }

    /// Two players facing each other across the box room, 250 units apart, with players able to hurt each other
    fn duel() -> Sim {
        let mut sim = demo();
        sim.pvp = true;
        sim.spawns = vec![[0.0, 0.0, -350.0], [0.0, 0.0, 350.0]];
        sim.add_player([0.0, 0.0, -100.0]);
        sim
    }

    fn aim(pitch: f32) -> Input {
        let mut i = Input { fire: true, ..holding() };
        i.set_look(0.0, pitch);
        i
    }

    #[test]
    fn a_bullet_hurts_the_player_it_hits_by_where_it_hits() {
        // Level with the eyes is the head, and a head shot kills
        let mut sim = duel();
        let events = sim.tick(&[aim(0.0), holding()], TICK);
        assert!(events.contains(&Event::Kill { killer: Some(0), victim: 1, cause: Cause::HeadShot }));
        assert_eq!(sim.players[1].hp, 0.0);
        assert!(sim.players[1].respawn_at.is_some());
        assert_eq!((sim.players[0].kills, sim.players[1].deaths), (1, 1));
        assert!(sim.players[1].message.0.contains("KILLED BY PLAYER 1"));

        // Aimed at the chest, it takes a third
        let mut sim = duel();
        sim.tick(&[aim(-14.6), holding()], TICK);
        assert_eq!(sim.players[1].hp, 100.0 - combat::damage(combat::BodyPart::Torso));
        assert!(sim.players[1].respawn_at.is_none());
        assert_eq!(sim.players[0].target_hp, Some(sim.players[1].hp));

        // Over their head and into the wall, it hurts nobody
        let mut sim = duel();
        sim.tick(&[aim(8.0), holding()], TICK);
        assert_eq!(sim.players[1].hp, 100.0);
    }

    #[test]
    fn nothing_hurts_a_player_when_players_cannot_be_hurt() {
        let mut sim = duel();
        sim.pvp = false;
        sim.tick(&[aim(0.0), holding()], TICK);
        assert_eq!(sim.players[1].hp, 100.0);
    }

    #[test]
    fn a_dead_player_is_still_until_they_come_back_at_a_spawn_with_everything_restored() {
        let mut sim = duel();
        sim.tick(&[aim(0.0), holding()], TICK);
        let walk = Input { forward: true, fire: true, grenade: true, ..holding() };
        for _ in 0..60 {
            sim.tick(&[holding(), walk], TICK);
        }
        let dead = &sim.players[1];
        assert_eq!((dead.player.pos[0], dead.player.pos[2]), (0.0, -100.0));
        assert_eq!(dead.ammo, 30);
        assert!(!dead.equipped_grenade);
        run(&mut sim, &holding(), RESPAWN_SECONDS);
        let back = &sim.players[1];
        assert!(back.respawn_at.is_none());
        assert_eq!(back.hp, PLAYER_HP);
        // The spawn point farthest from the player still standing at (0, -350)
        assert_eq!((back.player.pos[0], back.player.pos[2]), (0.0, 350.0));
        assert_eq!((back.kills, back.deaths), (0, 1));
    }

    #[test]
    fn a_shot_is_judged_against_where_the_target_was_when_the_shooter_saw_them() {
        // The target strafes out of the line of fire; the shooter fires at where they were
        let strafe = |rewind: f32| {
            let mut sim = duel();
            let step = Input { right: true, ..holding() };
            for _ in 0..12 {
                sim.tick(&[holding(), step], TICK);
            }
            assert!(sim.players[1].player.pos[0].abs() > 60.0, "the target has moved clear: {:?}", sim.players[1].player.pos);
            sim.rewind = vec![rewind, 0.0];
            sim.tick(&[aim(0.0), holding()], TICK);
            sim.players[1].hp
        };
        assert_eq!(strafe(0.0), PLAYER_HP, "now, they are out of the way");
        assert_eq!(strafe(12.0), 0.0, "twelve ticks ago, they were in the way");
        // Asking for more than is remembered gets the oldest, when they were still in the way
        assert_eq!(strafe(500.0), 0.0);
    }

    #[test]
    fn a_grenade_hurts_the_players_it_reaches_and_credits_the_thrower() {
        let mut sim = duel();
        sim.thrown.push(Thrown { grenade: Grenade { pos: [0.0, 100.0, -90.0], vel: [0.0; 3], fuse: 0.0, spin: 0.0, at_rest: true }, owner: 0 });
        let events = sim.tick(&[holding(), holding()], TICK);
        assert!(events.iter().any(|e| matches!(e, Event::Blast { .. })));
        assert_eq!(sim.players[1].hp, 0.0);
        assert_eq!(sim.players[0].kills, 1);
        assert!(events.contains(&Event::Kill { killer: Some(0), victim: 1, cause: Cause::Grenade }));
        assert!(events.contains(&Event::Kill { killer: None, victim: 0, cause: Cause::Grenade }));
        // So close that the thrower is caught too: that is a death, but not a kill
        assert_eq!((sim.players[0].kills, sim.players[0].deaths), (1, 1));
    }
}
