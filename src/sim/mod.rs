//! The game itself, apart from how it looks and sounds: the players, their weapons and
//! grenades, the characters, doors and glass, and what a tick of time does to them.
//!
//! The viewer (or a server) feeds the sim an `Input` for every player on every `TICK` and
//! gets back `Event`s: things that happened, which it draws, plays sounds for and prints. The
//! sim never reads a clock, a window or a sound device, and takes all its randomness from its
//! own generator, so the same inputs always give the same game. That is what lets a recorded
//! run be played again, and lets a server and its clients agree.

pub mod aim;
pub mod characters;
pub mod idle;
pub mod nav;
pub mod movement;
pub mod peek;
pub mod terrorist;

use std::collections::VecDeque;
use std::rc::Rc;

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
use movement::{Reloading, forward, right};

type V3 = [f32; 3];

/// The length of a tick, in seconds. The sim is stepped at this rate, whatever the frame rate.
pub const TICK: f32 = 1.0 / 60.0;

/// How long a reload takes
pub const RELOAD_SECONDS: f32 = 2.2;

/// A player's health at the start
pub const PLAYER_HP: f32 = 100.0;

/// A player's Stamina, out of 100, which says how many wounds they take: the original's
/// operatives' is mostly 75 to 95
pub const PLAYER_STAMINA: f32 = 85.0;

/// The ammunition the players' MP5K fires (pw_mp5k.wpn)
pub const PLAYER_AMMO: &str = "9mmP(FMJ)";

/// The sound of one shot of the players' MP5K, whose reach is how far it is heard
const PLAYER_SHOT: &str = "C_SB5KS.WAV";

/// How far from the feet, across the floor, a posed body can reach (a peek, an arm out)
const POSED_REACH: f32 = 120.0;

/// The radius of each part of a player's body that bullets hit (see `Sim::hit_parts`)
pub const HIT_RADIUS: f32 = 25.0;

/// How far along a shot from `eye` along `dir` enters a body part, an upright cylinder
fn through_part(eye: V3, dir: V3, part: &HitPart) -> Option<f32> {
    let a = dir[0] * dir[0] + dir[2] * dir[2];
    if a < 1e-9 {
        return None;
    }
    let at = part.bottom;
    let (ox, oz) = (eye[0] - at[0], eye[2] - at[2]);
    let b = 2.0 * (ox * dir[0] + oz * dir[2]);
    let c = ox * ox + oz * oz - HIT_RADIUS * HIT_RADIUS;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / (2.0 * a);
    let y = eye[1] + dir[1] * t;
    (t > 0.0 && (at[1]..=part.top).contains(&y)).then_some(t)
}

/// How far a peek moves the eye of a player facing `yaw` (an `Input`'s), in map space
fn lean(peek: Option<peek::Peek>, yaw: u16) -> V3 {
    let Some([side, up]) = peek.map(|k| k.eye()) else { return [0.0; 3] };
    let r = right(Input { yaw, ..Input::default() }.look().0);
    [r[0] * side, up, -r[2] * side]
}

/// A unit direction turned `off` radians away from `dir`, toward the side `round` radians round it
fn deflect(dir: V3, off: f32, round: f32) -> V3 {
    let up = if dir[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let side = unit3([up[1] * dir[2] - up[2] * dir[1], up[2] * dir[0] - up[0] * dir[2], up[0] * dir[1] - up[1] * dir[0]]);
    let other = [dir[1] * side[2] - dir[2] * side[1], dir[2] * side[0] - dir[0] * side[2], dir[0] * side[1] - dir[1] * side[0]];
    let (c, s) = (off.cos(), off.sin());
    let (rc, rs) = (round.cos(), round.sin());
    unit3([0, 1, 2].map(|k| dir[k] * c + (side[k] * rc + other[k] * rs) * s))
}

/// How players' bodies are animated as others see them: as fast against their feet as the
/// viewer's default (its --anim-scale)
const ANIM_PACE: f32 = 1.0;

/// What a living player's body was doing at the end of a tick, to shoot at it as the shooter
/// saw it then
#[derive(Clone, Copy, Debug)]
struct Seen {
    feet: V3,
    crouched: bool,
    limp: bool,
    /// How far a peek leaned them (see `Sim::peek_offset`)
    lean: V3,
    /// Which way they faced, as a camera's yaw in degrees
    yaw: f32,
    /// What their body played, as others are shown it (see `net::animation`)
    movement: movement::Movement,
    frame: f32,
    hip_turn: f32,
    reloading: Option<Reloading>,
    firing: bool,
    /// A wound's flinch, and how far through it
    flinch: Option<(&'static str, f32)>,
    peek: Option<peek::Peek>,
    time: f32,
}

/// A player's shot as the server judged it, for the network lab to compare with what the
/// shooter saw
#[derive(Clone, Debug)]
pub struct Fired {
    pub shooter: usize,
    pub eye: V3,
    pub dir: V3,
    /// Where the others were judged to be: their slot, feet, and whether crouched
    pub bodies: Vec<(usize, V3, bool)>,
    /// The number of the shooter's input that fired it, which the server fills in
    pub seq: u32,
    /// How many ticks back the others were judged (`Sim::rewind`)
    pub back: f32,
}

/// One of the parts of a player's body that bullets hit: an upright cylinder of `HIT_RADIUS`
/// from the middle of its bottom up to the height `top`. The legs are no part (`None`): which
/// one a bullet hits is the side it goes in.
#[derive(Clone, Copy, Debug)]
pub struct HitPart {
    pub bottom: V3,
    pub top: f32,
    pub part: Option<combat::BodyPart>,
}

/// How long a dead player waits to come back. The body they fell with stays where it fell.
pub const RESPAWN_SECONDS: f32 = 3.7;

/// How long a wounded player's flinch is kept: longer than any flinch plays (the longest, 51
/// frames, is 1.7 s), and short enough to send as a tick count in a byte (see net::proto)
pub const FLINCH_SECONDS: f32 = 2.0;

/// How many ticks of where players have been are kept, to aim at where they were
const PAST_TICKS: usize = 64;

/// A grenade throw: from the start of the swing to the end, and how far into it the grenade
/// leaves the hand (0..1)
pub const THROW_SECONDS: f32 = 0.85;
pub const THROW_RELEASE: f32 = 0.35;

/// How far away the door being opened or closed can be
const DOOR_REACH: f32 = 220.0;

/// The door a use press selects, with eye and look direction in map space.
pub fn door_target(eye: V3, look: V3, doors: &[Door], states: &[DoorState]) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, d) in doors.iter().enumerate() {
        // A press can aim at the doorway or the leaf wherever it has moved.
        for c in [d.center(0.0), d.center(states[i].open)] {
            let to = [c[0] - eye[0], c[1] - eye[1], c[2] - eye[2]];
            let dist = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
            if dist > DOOR_REACH || dist < 1.0 { continue; }
            let facing = (to[0] * look[0] + to[1] * look[1] + to[2] * look[2]) / dist;
            if facing < 0.5 { continue; }
            let score = dist * (2.0 - facing);
            if best.is_none_or(|(_, s)| score < s) { best = Some((i, score)); }
        }
    }
    best.map(|(i, _)| i)
}

/// The most players in a game
pub const MAX_PLAYERS: usize = 16;

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
    /// Wanting to be crouched. The player stands up again only where there is room.
    pub crouch: bool,
    /// Peeking round a corner to the left or right (see `peek`)
    pub peek_left: bool,
    pub peek_right: bool,
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

/// What a Rainbow operative, player or character, cries out when hurt or killed, picked by
/// `roll`. Dying is what the original calls being incapacitated, so there is no death cry of
/// its own; the re_/rr_ wounds and deaths are the terrorists'.
fn voice(killed: bool, roll: u32) -> String {
    let (name, n) = if killed { ("tm_inc", 3) } else { ("tm_wnd", 5) };
    format!("English/{name}{}.wav", 1 + roll % n)
}

/// The sound a shot makes where it left, for the player in slot `me` to hear. Their own shots
/// have none: the firing loop plays those. As the terrorists' shots do, each shot plays once,
/// so full auto is the shot overlapping itself.
pub fn shot_sound(event: &Event, me: usize) -> Option<(&'static str, V3)> {
    match *event {
        Event::Shot { player, start, .. } if player != me => Some((PLAYER_SHOT, start)),
        _ => None,
    }
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

    /// Hands off: nothing pressed or held, looking the same way, crouched or not as before
    pub fn let_go(&self) -> Input {
        Input { yaw: self.yaw, pitch: self.pitch, crouch: self.crouch, ..Input::default() }
    }
}

/// A motion a hit set playing on a player's body from `start`
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Struck {
    pub motion: &'static str,
    pub start: f32,
}

/// How a player was hurt: shot in a part of the body, or caught in a blast, from a side of them
#[derive(Clone, Copy, Debug, PartialEq)]
enum Wound {
    Shot(combat::BodyPart, combat::Facing),
    Blast(combat::Facing),
}

/// A side in a team game
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Blue,
    Gold,
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::Blue, Side::Gold];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn other(self) -> Side {
        match self { Side::Blue => Side::Gold, Side::Gold => Side::Blue }
    }

    /// "blue" or "gold", as a player names it
    pub fn named(name: &str) -> Option<Side> {
        match name.to_ascii_lowercase().as_str() {
            "blue" => Some(Side::Blue),
            "gold" => Some(Side::Gold),
            _ => None,
        }
    }
}

/// What killed a player
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cause {
    Shot,
    HeadShot,
    Grenade,
    /// A terrorist shot them
    Terrorist,
}

/// Who fired a bullet: a player, or one of the characters
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shooter {
    Player(usize),
    Character(usize),
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
    /// A character fired: where the shot left and where it ended
    CharacterShot { npc: usize, start: V3, end: V3 },
    /// A bullet struck the level. `seed` is for the effects it makes, `dist` how far it flew.
    BulletHit { point: V3, normal: V3, dir: V3, surface: Surface, material: String, dist: f32, seed: u64 },
    /// A pane broke where `at` is, struck along `dir`
    Glass { pane: usize, at: V3, dir: V3, seed: u64 },
    /// A grenade went off. `on_surface` is false in mid-air, where there is nothing to scorch.
    Blast { point: V3, normal: V3, surface: Surface, on_surface: bool, seed: u64 },
    /// What a blast did to a player: how hard it shook them, and how white it made their view
    PlayerBlast { player: usize, shake: f32, veil: f32 },
    /// A player died: who killed them (none if they did it to themselves) and how. Each is named
    /// as they were then, by join number and name, since the event may arrive after they have
    /// left and someone else has the slot.
    Kill { killer: Option<usize>, victim: usize, cause: Cause, killer_joined: u32, victim_joined: u32, killer_name: String, victim_name: String },
    /// A player said something. The server makes these, never the game itself. `dead` if they
    /// were dead in a round, when only the dead hear them, so that they cannot call out to the
    /// living.
    Chat { player: usize, name: String, text: String, dead: bool },
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

/// A grenade in flight, and who threw it: the slot, and the thrower as they were when they threw
/// it, since they may leave and someone else take the slot before it goes off
#[derive(Clone, Debug)]
pub struct Thrown {
    pub grenade: Grenade,
    pub owner: usize,
    pub by: Occupant,
    pub side: Option<Side>,
}

/// Whoever has a slot: the slot and their join number, which no one else in it has
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Occupant {
    pub slot: usize,
    pub joined: u32,
}

/// What a player did in a round, while counting is open (`Sim::open_round`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoundStats {
    pub kills: u32,
    /// Bullets fired
    pub fired: u32,
    /// Their bullets that struck someone living
    pub hits: u32,
    /// Bullets that struck them while alive
    pub taken: u32,
}

impl RoundStats {
    /// Hits as a percentage of rounds fired, rounded down
    pub fn hit_percent(&self) -> u32 {
        (self.hits as u64 * 100).checked_div(self.fired as u64).unwrap_or(0).min(100) as u32
    }
}

/// How a player ended a round. Wounded and incapacitated wait for the original's hurt rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ending {
    #[default]
    Alive,
    Killed,
}

/// Someone who played a round: everyone in the game when it began, whether they stay or not
#[derive(Clone, Debug, PartialEq)]
pub struct Participant {
    pub who: Occupant,
    pub side: Option<Side>,
    /// They left before the round was decided
    pub left: bool,
    pub end: Ending,
    pub stats: RoundStats,
    /// Who killed them, if a participant did
    pub killed_by: Option<Occupant>,
}

/// One player: their body, weapon, grenades and where they are in the level
pub struct PlayerState {
    /// Whether anyone is playing this slot
    pub active: bool,
    /// Join number identifying this occupant. Changes when someone new takes the slot.
    pub joined: u32,
    /// Their side in a team game
    pub side: Option<Side>,
    pub player: Player,
    /// Where the player looked in the last tick, as an `Input` says
    pub look: (u16, u16),
    /// Where the view was last tick if the player was not in control
    pub last_observer: Option<V3>,
    /// Whether the player is in control, as of the last tick
    pub controlled: bool,
    /// How fast the player moved in the last tick, in units per second
    pub moved_speed: f32,
    pub peek: Option<peek::Peek>,
    /// The spread the gun's shots leave in
    pub aim: aim::Aim,

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
    /// Counts surviving bullet wounds and the subset that hit a leg.
    /// Respawning or starting a new round clears both counts.
    pub wounds: u8,
    pub leg_wounds: u8,
    /// When a dead player comes back, or none if they are alive
    pub respawn_at: Option<f32>,
    pub deaths: u32,
    /// What the latest hit set the body playing: the flinch from a wound, for `FLINCH_SECONDS`,
    /// or the fall a dead player died with, until they come back
    pub hit: Option<Struck>,

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

impl PlayerState {
    /// The reload under way at `now`, if any. The magazine keeps its rounds until the reload
    /// is done, so it says throughout whether it was empty.
    pub fn reloading(&self, now: f32) -> Option<Reloading> {
        self.reload_until.map(|t| Reloading { progress: 1.0 - (t - now) / RELOAD_SECONDS, empty: self.ammo == 0 })
    }
}

/// The whole game
pub struct Sim {
    pub level: Rc<Map>,
    pub world: CollisionWorld,
    pub visibility: Visibility,
    /// Seconds since the start
    pub time: f32,
    pub rng: u64,
    pub culling: bool,
    speed_scale: f32,
    /// Whether players can hurt each other (a server's game: it is off when playing alone)
    pub pvp: bool,
    /// Whether players can hurt their own side (the original's FF, on in competitive play)
    pub friendly_fire: bool,
    /// Where each side's players start in a team game, Blue's and Gold's (`levels::Base::starts`)
    pub bases: [Vec<V3>; 2],
    /// The pairs of bases the host can choose `bases` from (`Setup::base_pairs`)
    pub base_pairs: Vec<[Vec<V3>; 2]>,
    /// Whether the dead come back after `RESPAWN_SECONDS`. In rounds they wait for the next.
    pub respawn: bool,
    /// No one moves, looks around, fires or changes weapons: before a round, and after it
    pub frozen: bool,
    /// How the doors and characters were at the start, for a new round to put back
    start: Option<(Vec<DoorState>, Vec<Npc>)>,
    /// Players who have joined so far
    joins: u32,
    /// Players take no harm, but flinch as they would (--god)
    pub god: bool,
    /// Where players come back after they die
    pub spawns: Vec<V3>,
    /// For each player, how many ticks back in time the others were when they took their shot,
    /// as that player saw them (a server sets it before each tick; none means no looking back)
    pub rewind: Vec<f32>,
    /// What each living player's body was doing at the end of each of the latest ticks, oldest first
    past: VecDeque<Vec<Option<Seen>>>,
    /// Players' shots as they were judged, kept only when this is on (the network lab)
    pub fired: Option<Vec<Fired>>,
    /// How each player's body is animated as the others see it, kept when players can be hit
    animators: crate::net::animation::Animators,
    eye_height: f32,
    /// What a player starts with
    pub magazine: u32,
    loadout: (u32, u32, u32),

    pub players: Vec<PlayerState>,
    pub thrown: Vec<Thrown>,
    /// Who played the round under way, whether what they do counts yet, and when (sim time)
    /// counting opened, which is when the round's players were let go
    pub round: Vec<Participant>,
    pub counting: bool,
    pub opened: Option<f32>,
    /// Players' names, the ones who left this round too, as the server gives them. Only for
    /// what the game says on screen and in its events, so not in the checksum.
    pub names: Vec<(Occupant, String)>,
    /// What is solid in the world right now (see `PlayerState::collision_rooms`)
    applied_rooms: Option<Vec<SolidRoom>>,

    pub characters: Option<Characters>,
    pub npcs: Vec<Npc>,
    /// What bullets do (data/kit/gfcm.txt), or without it each part of the body takes so much
    pub gunfire: Option<crate::gfcm::Gunfire>,

    pub doors: Vec<Door>,
    pub door_states: Vec<DoorState>,
    /// Whether each portal is shut by a closed door, and when one last opened or shut
    pub blocked_portals: Vec<bool>,
    pub doors_moved: f32,
    pub panes: Vec<Pane>,
    /// When each pane broke
    pub broken_at: Vec<Option<f32>>,
    /// How far each sound carries (data/sound/soundvol.txt), by its lowercase name
    reaches: std::collections::HashMap<String, f32>,
    /// Where characters can walk, made the first time one has somewhere to go
    nav: Option<nav::Grid>,
    /// What was made to be heard this tick besides the sounds it sends: where, and how far it
    /// carries
    noises: Vec<(V3, f32, terrorist::Noise)>,
}

impl Sim {
    /// A game with nobody in it yet: see `add_player`
    pub fn new(level: Rc<Map>, world: CollisionWorld, config: Config, characters: Option<Characters>, npcs: Vec<Npc>) -> Sim {
        let visibility = Visibility::new(&level);
        let doors: Vec<Door> = world.doors().cloned().collect();
        let panes = glass::panes(&level);
        let eye_height = world.params().eye_height;
        Sim {
            visibility,
            door_states: vec![DoorState::default(); doors.len()],
            blocked_portals: vec![false; level.portals.portals.len()],
            doors_moved: 0.0,
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
            friendly_fire: true,
            bases: [Vec::new(), Vec::new()],
            base_pairs: Vec::new(),
            respawn: true,
            frozen: false,
            start: None,
            joins: 0,
            god: false,
            rewind: Vec::new(),
            past: VecDeque::new(),
            fired: None,
            animators: Default::default(),
            spawns: Vec::new(),
            eye_height,
            magazine: config.magazine,
            loadout: (config.ammo, config.reserve, config.grenades),
            players: Vec::new(),
            thrown: Vec::new(),
            round: Vec::new(),
            counting: false,
            opened: None,
            names: Vec::new(),
            applied_rooms: None,
            characters,
            npcs,
            gunfire: crate::gfcm::read(std::path::Path::new("data/kit/gfcm.txt")).ok(),
            reaches: std::fs::read_to_string("data/sound/soundvol.txt").map(|t| crate::sound::reaches(&t)).unwrap_or_default(),
            noises: Vec::new(),
            nav: None,
        }
    }

    /// A player with the starting loadout, on their feet at `feet`
    fn new_player(&self, feet: V3) -> PlayerState {
        let (ammo, reserve, grenades) = self.loadout;
        PlayerState {
            active: true,
            joined: 0,
            side: None,
            player: Player::new(feet),
            look: (0, 0),
            last_observer: None,
            controlled: true,
            moved_speed: 0.0,
            peek: None,
            aim: aim::Aim::default(),
            ammo,
            reserve,
            reload_until: None,
            next_shot: 0.0,
            flash_until: 0.0,
            gun_trigger: false,
            kills: 0,
            hp: PLAYER_HP,
            wounds: 0,
            leg_wounds: 0,
            respawn_at: None,
            deaths: 0,
            hit: None,
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
        self.joins += 1;
        let fresh = PlayerState { joined: self.joins, ..self.new_player(feet) };
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
        if self.frozen {
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
        if !self.players.get(slot).is_some_and(|p| p.active) {
            return;
        }
        // They stay in the round's table, as they were when they went
        let who = self.occupant(slot);
        let end = self.ending(slot);
        if let Some(p) = self.round.iter_mut().find(|p| p.who == who && !p.left) {
            (p.left, p.end) = (true, end);
        }
        self.players[slot].active = false;
        self.apply_union_rooms();
    }

    pub fn occupant(&self, slot: usize) -> Occupant {
        Occupant { slot, joined: self.players.get(slot).map_or(0, |p| p.joined) }
    }

    /// Whether `who` still has their slot
    fn present(&self, who: Occupant) -> bool {
        self.players.get(who.slot).is_some_and(|p| p.active && p.joined == who.joined)
    }

    fn ending(&self, slot: usize) -> Ending {
        if self.players[slot].respawn_at.is_some() { Ending::Killed } else { Ending::Alive }
    }

    /// A player's name, as the server gave it, or "PLAYER n"
    pub fn name_of(&self, who: Occupant) -> String {
        self.names.iter().find(|(o, _)| *o == who).map_or_else(|| format!("PLAYER {}", who.slot + 1), |(_, n)| n.clone())
    }

    /// From now on, what the round's players do counts
    pub fn open_round(&mut self) {
        self.counting = true;
        self.opened = Some(self.time);
    }

    /// The round is decided: nothing counts any more, and how each player ended is kept
    pub fn close_round(&mut self) {
        self.counting = false;
        for i in 0..self.round.len() {
            let who = self.round[i].who;
            if !self.round[i].left && self.present(who) {
                self.round[i].end = self.ending(who.slot);
            }
        }
    }

    /// Change what `who` did in this round, if they played it and counting is open
    fn count(&mut self, who: Occupant, change: impl FnOnce(&mut Participant)) {
        if !self.counting {
            return;
        }
        if let Some(p) = self.round.iter_mut().find(|p| p.who == who) {
            change(p);
        }
    }

    /// Where a player on `side` starts: the next of their base's places, by how many are on
    /// that side already, as the original fills a base
    pub fn start_for(&self, side: Side) -> Option<V3> {
        let places = &self.bases[side.index()];
        let taken = self.players.iter().filter(|p| p.active && p.side == Some(side)).count();
        (!places.is_empty()).then(|| places[taken % places.len()])
    }

    /// Keep how the doors and characters are now, for `new_round` to put back
    pub fn mark_start(&mut self) {
        self.start = Some((self.door_states.clone(), self.npcs.clone()));
    }

    /// A new round from `seed`: the doors, panes and characters as they were at the start (see
    /// `mark_start`), no grenades, and every player fresh in their base's places in turn, or at
    /// the spawn points without a side. Scores and sides stay.
    pub fn new_round(&mut self, seed: u64) {
        self.rng = seed | 1;
        if let Some((doors, npcs)) = self.start.clone() {
            for (i, d) in doors.into_iter().enumerate() {
                self.door_states[i] = d;
                self.world.set_door_open(i, d.open);
                if let Some(portal) = self.doors[i].portal {
                    self.blocked_portals[portal] = d.closed();
                }
            }
            self.npcs = npcs;
        }
        for i in 0..self.panes.len() {
            if self.broken_at[i].take().is_some() {
                let name = self.panes[i].name.clone();
                self.world.switch_on(&name);
            }
        }
        self.thrown.clear();
        self.noises.clear();
        self.past.clear();
        self.animators = Default::default();
        let mut taken = [0usize; 3];
        for i in 0..self.players.len() {
            if !self.players[i].active {
                continue;
            }
            let p = &self.players[i];
            let (side, look, kills, deaths, joined) = (p.side, p.look, p.kills, p.deaths, p.joined);
            let (k, places) = match side {
                Some(s) if !self.bases[s.index()].is_empty() => (s.index(), &self.bases[s.index()]),
                _ => (2, &self.spawns),
            };
            let at = if places.is_empty() { p.player.pos } else { places[taken[k] % places.len()] };
            taken[k] += 1;
            self.players[i] = self.new_player(at);
            let p = &mut self.players[i];
            (p.side, p.look, p.kills, p.deaths, p.joined) = (side, look, kills, deaths, joined);
        }
        // Who plays this round: everyone here now. Counting waits for them to be let go.
        self.round = (0..self.players.len()).filter(|&i| self.players[i].active)
            .map(|i| Participant { who: self.occupant(i), side: self.players[i].side, left: false, end: Ending::Alive, stats: RoundStats::default(), killed_by: None })
            .collect();
        self.counting = false;
        self.opened = None;
        // The names of those who left are needed no more: their grenades went with the last round
        let here: Vec<bool> = self.names.iter().map(|(o, _)| self.present(*o)).collect();
        let mut keep = here.into_iter();
        self.names.retain(|_| keep.next().unwrap_or(false));
        self.apply_union_rooms();
    }

    /// A player out of the round from now, as the dead are: one who joined once it had begun
    pub fn sit_out(&mut self, slot: usize) {
        let now = self.time;
        let p = &mut self.players[slot];
        p.respawn_at = Some(now);
        p.hit = None;
        p.equipped_grenade = false;
        p.gun_trigger = false;
        self.announce(slot, "WAITING FOR THE NEXT ROUND", 5.0);
    }

    /// Whether `by` can hurt `victim`: themselves always, and their own side only with friendly fire
    fn may_hurt(&self, by: usize, victim: usize) -> bool {
        let side = |i: usize| self.players.get(i).and_then(|p| p.side);
        by == victim || self.friendly_fire || side(by).is_none() || side(by) != side(victim)
    }

    /// Where a player's eyes are
    pub fn eye(&self, slot: usize) -> V3 {
        let p = &self.players[slot];
        self.eye_at(p.player.pos, p.player.crouched, p.peek, p.look.0)
    }

    /// Where the eyes are of a player standing at `feet`, crouched or not, peeking as `peek`
    /// says, facing `yaw` (an `Input`'s): for a player as a client's view shows them
    pub fn eye_at(&self, feet: V3, crouched: bool, peek: Option<peek::Peek>, yaw: u16) -> V3 {
        let lean = lean(peek, yaw);
        [feet[0] + lean[0], feet[1] + self.world.params().eye(crouched) + lean[1], feet[2] + lean[2]]
    }

    /// How far a peek moves the player's eye, in map space
    pub fn peek_offset(&self, slot: usize) -> V3 {
        let p = &self.players[slot];
        lean(p.peek, p.look.0)
    }

    /// How far from `a` toward `b` nothing stops a bullet: the faces of the rooms around `a`
    /// and of those the line passes through, and unbroken glass. For a camera, which must not
    /// end up in or behind a wall, whatever rooms are solid around the players.
    pub fn line_reach(&self, a: V3, b: V3) -> f32 {
        self.sight_reach(a, b, true)
    }

    pub fn bot_sight_reach(&self, a: V3, b: V3) -> f32 {
        self.sight_reach(a, b, false)
    }

    fn sight_reach(&self, a: V3, b: V3, glass: bool) -> f32 {
        let length = dist3(a, b);
        if length < 1e-3 {
            return length;
        }
        let dir = [(b[0] - a[0]) / length, (b[1] - a[1]) / length, (b[2] - a[2]) / length];
        let (mut rooms, room) = self.view_rooms(a, None);
        rooms.extend(self.visibility.player_rooms(&self.level, a, room));
        rooms.extend(self.rooms_crossing(a, b));
        if !glass {
            return self.world.sight_hit_in(a, dir, length, &rooms).map_or(length, |h| h.dist);
        }
        let wall = self.world.bullet_hit_in(a, dir, length, &rooms).map_or(length, |h| h.dist);
        let glass = self.panes.iter().enumerate()
            .filter(|(i, _)| self.broken_at[*i].is_none())
            .filter_map(|(_, p)| p.raycast(a, dir))
            .fold(length, f32::min);
        wall.min(glass)
    }

    /// The rooms drawing starts from for a view at `at`, as for a view flying free (see
    /// `track_rooms`), and the room it is in, from `hint` (the one it was in last). Nothing
    /// is changed: for a camera that is no player's, such as a dead player watching another.
    pub fn view_rooms(&self, at: V3, hint: Option<usize>) -> (Vec<u32>, Option<usize>) {
        let room = self.level.rooms.locate(at, hint).or(hint);
        let rooms = match room.and_then(|i| self.visibility.number_of(i)) {
            Some(n) => vec![n],
            None => self.visibility.player_rooms(&self.level, at, room),
        };
        (rooms, room)
    }

    /// The rooms a view from someone standing at `feet` is drawn from, as their own is: those
    /// whose floor they stand on, else the room around their feet (`hint` the last one)
    pub fn standing_rooms(&self, feet: V3, hint: Option<usize>) -> (Vec<u32>, Option<usize>) {
        let probe = [feet[0], feet[1] + 10.0, feet[2]];
        let room = self.level.rooms.locate(probe, hint).or(hint);
        let floor: Vec<u32> = self.world.floor_rooms(feet).into_iter().filter(|&n| self.visibility.is_room(n)).collect();
        if !floor.is_empty() {
            return (floor, room);
        }
        let rooms = room.and_then(|i| self.visibility.number_of(i)).map(|n| vec![n]).unwrap_or_else(|| self.visibility.player_rooms(&self.level, probe, room));
        (rooms, room)
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
        h.u32(self.culling as u32 + 2 * self.frozen as u32);
        for p in self.players.iter().filter(|p| p.active) {
            h.v3(p.player.pos);
            h.f32(p.player.vel_y);
            h.u32(p.player.on_ground as u32);
            h.u32(p.player.crouched as u32);
            match p.peek {
                None => h.u32(0),
                Some(k) => {
                    h.u32(1 + k.right as u32 + 2 * k.crouched as u32 + 4 * k.returning as u32);
                    h.f32(k.frame);
                }
            }
            for v in [p.aim.angle, p.aim.from, p.aim.to, p.aim.since, p.aim.growth] {
                h.f32(v);
            }
            h.u32(p.aim.pace as u32 + 8 * p.aim.crouched as u32 + 16 * p.aim.started as u32 + ((p.aim.look as u32) << 16));
            h.u32(p.ammo);
            h.u32(p.reserve);
            h.opt_f32(p.reload_until);
            h.f32(p.next_shot);
            h.f32(p.flash_until);
            h.u32(p.kills);
            h.u32(p.side.map_or(0, |s| 1 + s.index() as u32));
            h.u32(p.joined);
            h.f32(p.hp);
            h.opt_f32(p.respawn_at);
            h.u32(p.deaths);
            match p.hit {
                None => h.u32(0),
                Some(s) => { h.u32(1); h.bytes(s.motion.as_bytes()); h.f32(s.start); }
            }
            h.u32(p.equipped_grenade as u32);
            h.u32(p.grenades_left);
            h.opt_f32(p.charge_from);
            match p.throw {
                None => h.u32(0),
                Some(t) => { h.u32(1); h.f32(t.start); h.f32(t.charge); h.u32(t.thrown as u32); }
            }
            h.u32(p.current_room.map_or(u32::MAX, |r| r as u32));
        }
        h.u32(self.counting as u32);
        h.opt_f32(self.opened);
        for p in &self.round {
            h.u32(p.who.slot as u32);
            h.u32(p.who.joined);
            h.u32(p.side.map_or(0, |s| 1 + s.index() as u32) + 4 * p.left as u32 + 8 * (p.end == Ending::Killed) as u32);
            for v in [p.stats.kills, p.stats.fired, p.stats.hits, p.stats.taken] {
                h.u32(v);
            }
            match p.killed_by {
                None => h.u32(u32::MAX),
                Some(o) => { h.u32(o.slot as u32); h.u32(o.joined); }
            }
        }
        for t in &self.thrown {
            let g = &t.grenade;
            h.u32(t.by.slot as u32);
            h.u32(t.by.joined);
            // The side it was thrown from says whom it may hurt
            h.u32(t.side.map_or(0, |s| 1 + s.index() as u32));
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
            h.u32(n.aiming as u32 + 2 * n.walking as u32);
            h.f32(n.last_shot);
            if let Some(m) = &n.mind {
                h.u32(m.target.map_or(u32::MAX, |t| t as u32));
                h.f32(m.seen_at);
                h.f32(m.aim.angle);
                h.u32(m.ammo + (m.magazines << 16));
                h.opt_f32(m.reload_until);
                h.f32(m.next_shot);
                h.u32(m.path.len() as u32);
                h.u32(m.threat as u32 + 8 * m.active.map_or(0, |k| k as u32 + 1));
            }
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
        // A dead or frozen player does nothing but look where they were looking
        let mut given: Vec<Input> = (0..self.players.len()).map(|i| inputs.get(i).copied().unwrap_or(none)).collect();
        for &i in &slots {
            let p = &self.players[i];
            if p.respawn_at.is_some() || self.frozen {
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
            if self.respawn && self.players[i].respawn_at.is_some_and(|t| self.time >= t) {
                self.respawn(i);
            }
            let p = &mut self.players[i];
            if p.respawn_at.is_none() && p.hit.is_some_and(|h| self.time - h.start >= FLINCH_SECONDS) {
                p.hit = None;
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
                if self.blocked_portals[p] != s.closed() {
                    self.doors_moved = self.time;
                }
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
        // The terrorists see the players where they have moved to, and fire
        // What the terrorists hear: the sounds of this tick, the players' shots and footsteps
        for i in 0..self.players.len() {
            let p = &self.players[i];
            if p.active && p.controlled && p.respawn_at.is_none() {
                // Four steps a second, each as loud as the pace makes it
                let reach = terrorist::footstep_reach(p.aim.pace);
                if reach > 0.0 && (self.time * 4.0).floor() != ((self.time - dt) * 4.0).floor() {
                    self.noises.push((p.player.pos, reach, terrorist::Noise::Footstep));
                }
            }
        }
        let mut noises = std::mem::take(&mut self.noises);
        noises.extend(ev.iter().filter_map(|e| match e {
            Event::Sound { name, at: Some(at), .. } => Some((*at, self.reach_of(name), terrorist::Noise::Sound)),
            _ => None,
        }));
        self.terrorists(dt, &noises, &mut ev);

        self.apply_union_rooms();
        self.update_grenades(dt, &mut ev);
        for &i in &slots {
            self.track_rooms(i, input_of(i).observer);
        }
        self.apply_union_rooms();
        if self.pvp {
            let past = (0..self.players.len()).map(|i| self.seen(i, dt)).collect();
            self.past.push_back(past);
            while self.past.len() > PAST_TICKS {
                self.past.pop_front();
            }
        }
        ev
    }

    /// What a living player's body is doing now, advancing its animation by `dt`. The
    /// animation is the one the others are shown (`net::animation`), from how the body moves.
    fn seen(&mut self, slot: usize, dt: f32) -> Option<Seen> {
        let now = self.time;
        let p = &self.players[slot];
        if !p.active || p.respawn_at.is_some() {
            self.animators.forget(slot);
            return None;
        }
        let (pos, crouched, yaw) = (p.player.pos, p.player.crouched, dequantize(p.look.0));
        let doing = crate::net::animation::Doing {
            throwing: p.throw.map(|t| (now - t.start) / THROW_SECONDS),
            reloading: p.reloading(now),
            firing: p.gun_trigger && p.ammo > 0,
            crouched,
            wounds: p.wounds,
            leg_wounds: p.leg_wounds,
        };
        let limp = p.leg_wounds > 0;
        let flinch = p.hit.map(|h| (h.motion, (now - h.start) * 30.0));
        let peek = p.peek;
        let lean = self.peek_offset(slot);
        let player = self.characters.as_ref().map(|c| &c.player);
        let frames = |m: movement::Movement| player.and_then(|pm| pm.get(m, crouched, limp)).map_or(1, |m| m.frames.max(1)) as f32;
        let (movement, frame) = self.animators.update(slot, pos, yaw, dt, doing, frames, self.speed_scale, ANIM_PACE);
        let hip_turn = self.animators.hip_turn(slot, yaw);
        let flinch = flinch.filter(|&(m, f)| f < self.characters.as_ref().map_or(0.0, |c| c.frames(m)));
        Some(Seen { feet: pos, crouched, limp, lean, yaw, movement, frame, hip_turn, reloading: doing.reloading, firing: doing.firing, flinch, peek, time: now })
    }

    /// What a player's body was doing `back` ticks ago (between ticks, if it is not a whole
    /// number: where they were and how far they leaned, between the two, and the rest as at the
    /// nearer tick), or what it does now if `back` is nothing. None if they were not standing then.
    fn seen_then(&self, slot: usize, back: f32) -> Option<Seen> {
        if back <= 0.0 || self.past.is_empty() {
            // Where they are now, animated as at the end of the last tick
            let now = self.seen_now(slot)?;
            let last = self.past.back().and_then(|t| t.get(slot).copied().flatten());
            return Some(last.map_or(now, |a| Seen { movement: a.movement, frame: a.frame, hip_turn: a.hip_turn, flinch: a.flinch, ..now }));
        }
        // The newest tick in `past` is the one just finished, which is as good as now
        let back = back.min((self.past.len() - 1) as f32);
        let newer = self.past.len() - 1 - back.floor() as usize;
        let at = |i: usize| self.past[i].get(slot).copied().flatten();
        let (newer_body, older_body) = (at(newer), newer.checked_sub(1).and_then(at));
        match (newer_body, older_body) {
            (Some(n), Some(o)) => {
                let f = back.fract();
                let mix = |a: V3, b: V3| [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * f);
                Some(Seen { feet: mix(n.feet, o.feet), lean: mix(n.lean, o.lean), ..if f < 0.5 { n } else { o } })
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
        match door_target(eye, look, &self.doors, &self.door_states) {
            Some(i) => {
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
        // Crouching is at once; standing up waits for the room to
        let body = &mut self.players[slot].player;
        if input.crouch {
            body.crouched = true;
        } else if body.crouched && self.world.room_to_stand(body) {
            body.crouched = false;
        }
        let crouched = body.crouched;
        let (wounds, leg_wounds) = (self.players[slot].wounds, self.players[slot].leg_wounds);
        let speed = |m| movement::pace(m, crouched, wounds, leg_wounds);
        let alive = self.players[slot].respawn_at.is_none();
        // The body moved to the side as far as the head goes has to be clear of the walls
        let (body, side) = (self.players[slot].player, right(input.look().0));
        let world = &self.world;
        let clearance = |by: f32| world.wall_clearance(&Player { pos: [body.pos[0] + side[0] * by, body.pos[1], body.pos[2] - side[2] * by], ..body });
        let peek = &mut self.players[slot].peek;
        // Holding a peek key stands the player still. Let go, a direction moves them at once,
        // and the lean plays back the faster.
        let leaning = input.peek_left || input.peek_right;
        let off = !leaning && (input.forward || input.left || input.back || input.right);
        let dt_peek = if off { dt * peek::HURRY } else { dt };
        *peek = peek::Peek::step(*peek, input.peek_left && alive, input.peek_right && alive, crouched, dt_peek, clearance);
        let still = peek.is_some() && !off;

        let yaw_rad = input.look().0.to_radians();
        let fwd_flat = [-yaw_rad.sin(), 0.0, -yaw_rad.cos()];
        let (w, a, s_, d, run) = (input.forward && !still, input.left && !still, input.back && !still, input.right && !still, input.run);
        // The spread follows the motion the keys play: running forward, or else at a shuffle
        // (the walk is the original's slow walk), as it does for a diagonal
        let pace = if w && run && !s_ { aim::Pace::Run } else if w || a || s_ || d { aim::Pace::Shuffle } else { aim::Pace::Stopped };
        let p = &mut self.players[slot];
        let trigger = input.fire && !p.equipped_grenade;
        let wounded = p.hp < PLAYER_HP;
        p.aim.step(&aim::MP5K, aim::PLAYER_SKILL, pace, crouched, wounded, trigger, input.yaw, dt);
        let scale = self.speed_scale;
        let fwd_speed = scale * if w == s_ { 0.0 }
            else if w { speed(if run { movement::Movement::Run } else { movement::Movement::Walk }) }
            else { -speed(movement::Movement::Back) };
        let side = |right: bool| speed(movement::movement_from_keys(false, false, !right, right, run));
        let side_speed = scale * if a == d { 0.0 } else if d { side(true) } else { -side(false) };
        // Forward or back with a side key, the original keeps the forward or back pace and turns
        // it toward the side by the hips' 27 degrees
        let (ahead, aside) = if fwd_speed != 0.0 && side_speed != 0.0 {
            (fwd_speed * movement::HIP_TURN.cos(), fwd_speed.abs() * side_speed.signum() * movement::HIP_TURN.sin())
        } else {
            (fwd_speed, side_speed)
        };
        let vel = [fwd_flat[0] * ahead + right_cam[0] * aside, fwd_flat[2] * ahead + right_cam[2] * aside];

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
                    let from = match self.world.grenade_hit(eye, to, reach + grenade::RADIUS).map(|h| h.dist) {
                        Some(d) => { let d = (d - grenade::RADIUS - 1.0).max(0.0); [eye[0] + to[0] * d, eye[1] + to[1] * d, eye[2] + to[2] * d] }
                        None => want,
                    };
                    let (by, side) = (self.occupant(slot), self.players[slot].side);
                    self.thrown.push(Thrown { grenade: Grenade::thrown(from, aim, th.charge, [0.0; 3]), owner: slot, by, side });
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
        let noise = (p.player.pos, self.reach_of(PLAYER_SHOT), terrorist::Noise::Sound);
        self.noises.push(noise);
        let who = self.occupant(slot);
        self.count(who, |p| p.stats.fired += 1);

        let eye = self.eye(slot);
        let f = forward_cam;
        let aimed = [f[0], f[1], -f[2]]; // camera space to MAP space
        // Somewhere within the spread, which the shot then opens
        let mut u = [0.0; 3];
        for v in &mut u {
            *v = (next_rng(&mut self.rng) >> 40) as f32 / (1u64 << 24) as f32;
        }
        let (off, round) = self.players[slot].aim.throw(u);
        self.players[slot].aim.fired();
        let dir = deflect(aimed, off, round);
        if self.fired.is_some() {
            let back = self.rewind.get(slot).copied().unwrap_or(0.0);
            let bodies = (0..self.players.len()).filter(|&i| i != slot)
                .filter_map(|i| self.seen_then(i, back).map(|s| (i, s.feet, s.crouched))).collect();
            self.fired.as_mut().unwrap().push(Fired { shooter: slot, eye, dir, bodies, seq: 0, back });
        }
        self.bullet(eye, dir, Shooter::Player(slot), ev);
    }

    /// The nearest of `bodies` (slot, feet, crouched) that a shot from `eye` along the unit
    /// vector `dir` passes through before a wall, the bodies taken as upright cylinders
    pub fn body_along(&self, eye: V3, dir: V3, bodies: &[(usize, V3, bool)]) -> Option<usize> {
        let wall = self.world.bullet_hit(eye, dir, 20000.0).map_or(f32::INFINITY, |h| h.dist);
        bodies.iter()
            .filter_map(|&(i, feet, crouched)| {
                self.hit_parts(feet, crouched, [0.0; 3]).into_iter().filter_map(|p| through_part(eye, dir, &p)).reduce(f32::min).map(|t| (i, t))
            })
            .filter(|&(_, t)| t <= wall)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// A bullet from `eye` along the unit vector `dir`, and what it strikes: a player, a
    /// character (a player's bullet), glass, which it breaks and goes on through, or the level
    fn bullet(&mut self, eye: V3, dir: V3, by: Shooter, ev: &mut Vec<Event>) {
        let now = self.time;
        // Where it flies, room to room, and what of those rooms' it strikes. Only the rooms
        // around the players are solid, so this alone says what stops a character's bullet,
        // and whether a player's reaches someone far off.
        let start_rooms = match by {
            // And the rooms around them and that they see from: standing where rooms overlap,
            // past the plane of the portal between them, a bullet crosses no portal into the
            // room they see into (citystreet_large's killhouse)
            Shooter::Player(s) => {
                let p = &self.players[s];
                let mut rooms = self.rooms_at(p.player.pos, p.current_room);
                for &r in p.player_rooms.iter().chain(&p.render_rooms) {
                    if !rooms.contains(&r) {
                        rooms.push(r);
                    }
                }
                rooms
            }
            Shooter::Character(i) => self.rooms_at([self.npcs[i].pos[0], self.npcs[i].floor, self.npcs[i].pos[2]], None),
        };
        let crossed = self.flight(eye, dir, &start_rooms);
        let far_wall = |sim: &Sim| sim.world.bullet_hit_in(eye, dir, 20000.0, &crossed).map(|h| h.dist);
        // Through the faces bullets pass, such as a chain fence
        let near_wall = |sim: &Sim| match by {
            Shooter::Player(_) => sim.world.bullet_hit(eye, dir, 20000.0).map(|h| h.dist),
            Shooter::Character(_) => far_wall(sim),
        };
        let mut wall = near_wall(self);
        let flown = far_wall(self);
        // Reaching a body in `rooms`: among the walls of the rooms flown through, and of its own
        let reaches = |rooms: &[u32], d: f32| {
            let mut walls: Vec<u32> = crossed.iter().chain(rooms).copied().collect();
            walls.extend(self.rooms_crossing(eye, [0, 1, 2].map(|k| eye[k] + dir[k] * d)));
            rooms.iter().any(|r| crossed.contains(r)) && flown.is_none_or(|w| w >= d - 1.0)
                && self.world.bullet_hit_in(eye, dir, d, &walls).is_none_or(|h| h.dist >= d - 1.0)
        };

        let mut best: Option<(usize, combat::Hit)> = None;
        if let Some(chars) = &self.characters {
            for (i, npc) in self.npcs.iter().enumerate() {
                // A terrorist's bullets hit hostages, not other terrorists
                if npc.health.is_dead() || matches!(by, Shooter::Character(s) if s == i || npc.mind.is_some()) {
                    continue;
                }
                let h = chars.raycast(npc, now, eye, dir).filter(|h| reaches(&self.rooms_at([npc.pos[0], npc.floor, npc.pos[2]], None), h.distance));
                if let Some(h) = h {
                    if best.map_or(true, |(_, b)| h.distance < b.distance) {
                        best = Some((i, h));
                    }
                }
            }
        }

        let hit_player = self.player_hit(by, eye, dir)
            .filter(|&(v, d, _)| reaches(&self.rooms_at(self.players[v].player.pos, self.players[v].current_room), d));

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
            wall = near_wall(self);
        }

        let start = [eye[0] + dir[0] * 30.0, eye[1] + dir[1] * 30.0 - 8.0, eye[2] + dir[2] * 30.0];
        // Another player in the way, nearer than whatever else was hit
        let hit_player = hit_player.filter(|h| wall.is_none_or(|w| h.1 <= w) && best.as_ref().is_none_or(|(_, b)| h.1 < b.distance));
        // What it does to a person it struck `dist` away, rolled
        let ammo = match by {
            Shooter::Player(_) => Some(PLAYER_AMMO.to_string()),
            Shooter::Character(i) => self.npcs[i].weapon.and_then(|w| self.characters.as_ref()?.weapons.get(w)?.1.ammo.clone()),
        };
        let shot = |end: V3, hit_character: bool| match by {
            Shooter::Player(player) => Event::Shot { player, start, end, hit_character },
            Shooter::Character(npc) => Event::CharacterShot { npc, start, end },
        };
        if let Some((victim, dist, part)) = hit_player {
            let point = [eye[0] + dir[0] * dist, eye[1] + dir[1] * dist, eye[2] + dir[2] * dist];
            let from = self.placement(victim).facing_of_shot(dir);
            let (attacker, cause) = match by {
                Shooter::Player(a) => (Some(self.occupant(a)), if part == combat::BodyPart::Head { Cause::HeadShot } else { Cause::Shot }),
                Shooter::Character(_) => (None, Cause::Terrorist),
            };
            if let Some(a) = attacker {
                self.count(a, |p| p.stats.hits += 1);
            }
            let struck = self.occupant(victim);
            self.count(struck, |p| p.stats.taken += 1);
            let effect = self.bullet_effect(ammo.as_deref(), part, dist);
            let hp = self.players[victim].hp;
            let amount = combat::bullet_damage(effect, part, hp, PLAYER_HP / crate::gfcm::wounds_to_fall(PLAYER_STAMINA) as f32);
            self.hurt_player(victim, attacker, amount, point, Wound::Shot(part, from), cause, ev);
            ev.push(shot(point, true));
            return;
        }
        if let (Shooter::Character(shooter), Some((i, hit))) = (by, best.clone()) {
            if wall.is_none_or(|w| hit.distance <= w) {
                let effect = self.bullet_effect(ammo.as_deref(), hit.part, hit.distance);
                next_rng(&mut self.rng);
                let how = self.npcs[i].posture();
                let result = self.npcs[i].health.apply_hit(&hit, effect, now, (self.rng >> 33) as u32, how);
                ev.push(Event::Log(format!("t={now:.2} character {shooter}'s shot hit character {i}: {:?} from the {:?}, {:?}, hp now {}", hit.part, hit.facing, result, self.npcs[i].health.hp)));
                ev.push(Event::Sound { name: "C_BHTHT.WAV".to_string(), at: Some(hit.point), gain: 1.0 });
                let name = voice(matches!(result, HitResult::Killed), (self.rng >> 40) as u32);
                ev.push(Event::Sound { name, at: Some(self.npcs[i].pos), gain: 1.0 });
                ev.push(shot(hit.point, true));
                return;
            }
        }
        let Shooter::Player(slot) = by else {
            self.strike_level(eye, dir, wall, Some(&crossed), ev);
            let d = wall.unwrap_or(5000.0);
            ev.push(shot([eye[0] + dir[0] * d, eye[1] + dir[1] * d, eye[2] + dir[2] * d], false));
            return;
        };
        match best {
            Some((i, hit)) if wall.map_or(true, |w| hit.distance <= w) => {
                // Shot, they turn on whoever shot them, and their team is aware
                let from = self.players[slot].player.pos;
                if let Some(mind) = self.npcs[i].mind.as_mut() {
                    mind.target = Some(slot);
                    mind.seen_at = now;
                    mind.know(terrorist::Sign::Shot, Some(slot), from, now);
                    let team = mind.team;
                    self.make_aware(team, 2);
                }
                let effect = self.bullet_effect(ammo.as_deref(), hit.part, hit.distance);
                next_rng(&mut self.rng);
                let how = self.npcs[i].posture();
                let result = self.npcs[i].health.apply_hit(&hit, effect, now, (self.rng >> 33) as u32, how);
                self.players[slot].target_hp = Some(self.npcs[i].health.hp);
                ev.push(Event::Log(format!("t={now:.2} shot hit character {i}: {:?} from the {:?}, {:?}, hp now {}", hit.part, hit.facing, result, self.npcs[i].health.hp)));
                let who = self.occupant(slot);
                self.count(who, |p| p.stats.hits += 1);
                let text = match result {
                    HitResult::Killed => {
                        self.players[slot].kills += 1;
                        self.count(who, |p| p.stats.kills += 1);
                        if hit.part == combat::BodyPart::Head { "HEAD SHOT KILL".to_string() } else { "KILLED".to_string() }
                    }
                    _ => format!("HIT {:?}", hit.part).to_uppercase(),
                };
                self.announce(slot, &text, 1.2);
                ev.push(Event::Sound { name: "C_BHTHT.WAV".to_string(), at: Some(hit.point), gain: 1.0 });
                // Voices: a grunt when hurt, a cry when killed
                let name = voice(matches!(result, HitResult::Killed), (self.rng >> 40) as u32);
                ev.push(Event::Sound { name, at: Some(self.npcs[i].pos), gain: 1.0 });
                ev.push(Event::Shot { player: slot, start, end: hit.point, hit_character: true });
            }
            _ => {
                self.strike_level(eye, dir, wall, None, ev);
                let d = wall.unwrap_or(5000.0);
                ev.push(shot([eye[0] + dir[0] * d, eye[1] + dir[1] * d, eye[2] + dir[2] * d], false));
            }
        }
    }

    /// What a bullet of `ammo` does to the part of a person it struck `dist` away, rolled: none
    /// without the game's chart or the ammunition in it
    fn bullet_effect(&mut self, ammo: Option<&str>, part: combat::BodyPart, dist: f32) -> Option<crate::gfcm::Effect> {
        let roll = 1 + (next_rng(&mut self.rng) >> 33) as u32 % 100;
        let gunfire = self.gunfire.as_ref()?;
        Some(gunfire.effect(gunfire.ammo(ammo?)?, part, dist / 100.0, roll))
    }

    /// How far a sound carries, none when soundvol.txt does not have it
    fn reach_of(&self, name: &str) -> f32 {
        self.reaches.get(&name.to_ascii_lowercase()).copied().unwrap_or(0.0)
    }

    /// The rooms a person standing at `feet` is in (`hint` is the one they were last in), every
    /// one whose walls have to stop sight and bullets
    fn rooms_at(&self, feet: V3, hint: Option<usize>) -> Vec<u32> {
        self.visibility.rooms_holding(&self.level, [feet[0], feet[1] + 50.0, feet[2]], hint)
    }

    /// The rooms whose volumes the segment from `a` to `b` passes through
    fn rooms_crossing(&self, a: V3, b: V3) -> Vec<u32> {
        self.level.rooms.crossing(a, b).into_iter().filter_map(|i| self.level.rooms.rooms[i].section_name.parse().ok()).collect()
    }

    /// The rooms a bullet from `eye` along `dir`, starting in the rooms `from`, flies into
    /// through the portals it crosses
    fn flight(&self, eye: V3, dir: V3, from: &[u32]) -> Vec<u32> {
        let far = [eye[0] + dir[0] * 20000.0, eye[1] + dir[1] * 20000.0, eye[2] + dir[2] * 20000.0];
        self.visibility.rooms_along(from, &[], eye, far, &self.blocked_portals).unwrap_or_default()
    }

    /// Whether nothing stops a bullet between `a`, in the rooms `from`, and `b`, in the rooms
    /// `to`: the line goes from room to room through portals to get there, and strikes none of
    /// those rooms' faces on the way
    fn line_clear(&self, a: V3, from: &[u32], b: V3, to: &[u32]) -> bool {
        let d = dist3(a, b);
        let Some(mut rooms) = self.visibility.rooms_along(from, to, a, b, &self.blocked_portals) else { return false };
        // Both ends' rooms' walls too, and those of every room the line passes through: in a
        // room that holds the whole level (rm16's grounds) the line goes from room to room
        // without leaving it, and would leave out the walls of the rooms inside
        rooms.extend(from.iter().chain(to));
        rooms.extend(self.rooms_crossing(a, b));
        let dir = [(b[0] - a[0]) / d, (b[1] - a[1]) / d, (b[2] - a[2]) / d];
        self.world.bullet_hit_in(a, dir, d, &rooms).is_none_or(|h| h.dist >= d - 1.0)
    }

    /// A bullet striking the level, if `wall` says it does: among the faces of `rooms` if
    /// given, else what is solid around the players
    fn strike_level(&mut self, eye: V3, dir: V3, wall: Option<f32>, rooms: Option<&[u32]>, ev: &mut Vec<Event>) {
        let hit = match rooms {
            Some(rooms) => self.world.bullet_hit_in(eye, dir, 20000.0, rooms),
            None => self.world.bullet_hit(eye, dir, 20000.0),
        };
        if let Some(hit) = wall.and(hit) {
            let point = [eye[0] + dir[0] * hit.dist, eye[1] + dir[1] * hit.dist, eye[2] + dir[2] * hit.dist];
            let surface = self.surface_of(hit.material);
            let material = hit.material.and_then(|m| self.level.materials.materials.get(m)).map_or("?", |m| m.filename.as_str()).to_string();
            next_rng(&mut self.rng);
            ev.push(Event::BulletHit { point, normal: hit.normal, dir, surface, material, dist: hit.dist, seed: self.rng });
        }
    }

    /// Grenades in the air: they bounce off the level and go off when the fuse runs out
    fn update_grenades(&mut self, dt: f32, ev: &mut Vec<Event>) {
        let now = self.time;
        let mut blasts: Vec<(V3, Occupant, Option<Side>)> = Vec::new();
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
                blasts.push((g.pos, t.by, t.side));
            }
        }
        thrown.retain(|t| !t.grenade.goes_off());
        self.thrown = thrown;
        for (c, by, side) in blasts {
            self.blast(c, by, side, ev);
        }
    }

    /// A grenade thrown by `owner` goes off at `c`
    fn blast(&mut self, c: V3, by: Occupant, side: Option<Side>, ev: &mut Vec<Event>) {
        // The thrower may have left, and someone else taken the slot
        let owner = by.slot;
        let theirs = self.present(by);
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
            let how = self.npcs[i].posture();
            let result = self.npcs[i].health.apply_blast(100.0 * strength, from, now, how);
            if theirs {
                self.players[owner].target_hp = Some(self.npcs[i].health.hp);
            }
            ev.push(Event::Log(format!("t={now:.2} grenade hit character {i} at {:.0} units: {result:?}, hp now {}", dist3(c, chest), self.npcs[i].health.hp)));
            if matches!(result, HitResult::Killed) {
                self.count(by, |p| p.stats.kills += 1);
                if theirs {
                    self.players[owner].kills += 1;
                    self.announce(owner, "GRENADE KILL", 1.2);
                }
            }
            let name = voice(matches!(result, HitResult::Killed), (self.rng >> 40) as u32);
            ev.push(Event::Sound { name, at: Some(self.npcs[i].pos), gain: 1.0 });
        }

        // Players are hurt as characters are, if they can be
        if self.pvp {
            for i in 0..self.players.len() {
                // The thrower is always hurt by their own. Anyone else, the newcomer in their
                // slot included, by the friendly fire rule for the side they threw from.
                let hurts = self.occupant(i) == by || self.friendly_fire || side.is_none() || side != self.players[i].side;
                if !self.players[i].active || self.players[i].respawn_at.is_some() || !hurts {
                    continue;
                }
                let feet = self.players[i].player.pos;
                let chest = [feet[0], feet[1] + 100.0, feet[2]];
                let head = [feet[0], feet[1] + 160.0, feet[2]];
                let strength = grenade::blast_strength(dist3(c, chest));
                if strength < 0.02 || !(grenade::clear_between(&self.world, c, chest) || grenade::clear_between(&self.world, c, head)) {
                    continue;
                }
                let from = self.placement(i).facing_of_shot(unit3([chest[0] - c[0], chest[1] - c[1], chest[2] - c[2]]));
                self.hurt_player(i, Some(by), PLAYER_HP * strength, chest, Wound::Blast(from), Cause::Grenade, ev);
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
    fn player_hit(&self, by: Shooter, eye: V3, dir: V3) -> Option<(usize, f32, combat::BodyPart)> {
        // Players hurt each other only when the game says they can; characters always can, and
        // see players where they are, with no looking back
        let (shooter, back) = match by {
            Shooter::Player(_) if !self.pvp => return None,
            Shooter::Player(s) => (Some(s), self.rewind.get(s).copied().unwrap_or(0.0)),
            Shooter::Character(_) => (None, 0.0),
        };
        let a = dir[0] * dir[0] + dir[2] * dir[2];
        if a < 1e-9 {
            return None;
        }
        (0..self.players.len())
            .filter(|&i| Some(i) != shooter && self.players[i].active && self.players[i].respawn_at.is_none())
            .filter(|&i| shooter.is_none_or(|s| self.may_hurt(s, i)))
            .filter_map(|i| {
                // Where the shooter saw the body
                let seen = self.seen_then(i, back)?;
                let place = combat::Placement { pos: seen.feet, yaw_deg: -seen.yaw, scale: [1.0; 3] };
                if let Some(posed) = self.posed_hit(&seen, &place, eye, dir) {
                    return posed.map(|h| (i, h.distance, h.part));
                }
                let right = place.to_world([1.0, 0.0, 0.0]);
                self.hit_parts(seen.feet, seen.crouched, seen.lean).into_iter().filter_map(|hit_part| {
                    let t = through_part(eye, dir, &hit_part)?;
                    let HitPart { bottom: at, part, .. } = hit_part;
                    let (ox, oz) = (eye[0] - at[0], eye[2] - at[2]);
                    // Below the belly, the leg on the side of the body the bullet goes in
                    let across = (ox + dir[0] * t) * (right[0] - place.pos[0]) + (oz + dir[2] * t) * (right[2] - place.pos[2]);
                    let part = part.unwrap_or(if across > 0.0 { combat::BodyPart::RightLeg } else { combat::BodyPart::LeftLeg });
                    Some((i, t, part))
                }).min_by(|a, b| a.1.total_cmp(&b.1))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// A living player's body as it is now, without moving its animation on
    fn seen_now(&self, slot: usize) -> Option<Seen> {
        let p = &self.players[slot];
        (p.active && p.respawn_at.is_none()).then(|| Seen {
            feet: p.player.pos, crouched: p.player.crouched, limp: p.leg_wounds > 0, lean: self.peek_offset(slot), yaw: dequantize(p.look.0),
            movement: movement::Movement::Idle, frame: 0.0, hip_turn: 0.0, reloading: p.reloading(self.time),
            firing: p.gun_trigger && p.ammo > 0, flinch: None, peek: p.peek, time: self.time,
        })
    }

    /// The body posed as it was `seen`, and where on it a bullet from `eye` along the unit vector
    /// `dir` strikes it: Some(None) where it goes past, None without the model to pose
    fn posed_hit(&self, seen: &Seen, place: &combat::Placement, eye: V3, dir: V3) -> Option<Option<combat::Hit>> {
        let chars = self.characters.as_ref()?;
        let rig = chars.rig.as_ref()?;
        // Nowhere near the body is a miss, without posing it
        let reach = POSED_REACH;
        let (ox, oz) = (seen.feet[0] - eye[0], seen.feet[2] - eye[2]);
        let along = (ox * dir[0] + oz * dir[2]) / (dir[0] * dir[0] + dir[2] * dir[2]).max(1e-9);
        let (cx, cz) = (eye[0] + dir[0] * along - seen.feet[0], eye[2] + dir[2] * along - seen.feet[2]);
        if along < -reach || cx * cx + cz * cz > reach * reach {
            return Some(None);
        }
        let flinch = seen.flinch.and_then(|(m, f)| chars.motions.get(m).map(|m| (m, f)));
        let doing = crate::body::Doing {
            movement: seen.movement, frame: seen.frame, crouched: seen.crouched, limp: seen.limp, reloading: seen.reloading, firing: seen.firing,
            flinch, peek: seen.peek, hip_turn: seen.hip_turn, changing: None, shuffle: None, clock: seen.time,
        };
        let pose = crate::body::pose(&chars.player, &rig.skeleton, &doing)?;
        let verts = rig.skin.apply(&rig.skeleton.pose([0.0, pose.root[1], 0.0], &pose.rotations));
        Some(combat::raycast_character(&chars.model, &rig.skeleton, &rig.skin, &verts, place, eye, dir))
    }

    /// What bullets hit of a player standing at `feet`: an upright cylinder of `HIT_RADIUS`, cut
    /// by height into the legs, belly, chest and head. A peek's `lean` (see `peek_offset`) moves
    /// them toward where the head has gone: the legs stay, and each part above goes so much of
    /// the way.
    pub fn hit_parts(&self, feet: V3, crouched: bool, lean: V3) -> [HitPart; 4] {
        let height = self.world.params().body_height(crouched);
        [(0.0, 0.40, 0.0, None), (0.40, 0.55, 0.25, Some(combat::BodyPart::Abdomen)),
            (0.55, 0.88, 0.6, Some(combat::BodyPart::Torso)), (0.88, 1.0, 1.0, Some(combat::BodyPart::Head))]
            .map(|(lo, hi, follows, part)| {
                let at = [0, 1, 2].map(|k| feet[k] + lean[k] * follows);
                HitPart { bottom: [at[0], at[1] + lo * height, at[2]], top: at[1] + hi * height, part }
            })
    }

    /// Where a player stands and the way they face, as their body is drawn
    fn placement(&self, slot: usize) -> combat::Placement {
        let p = &self.players[slot];
        combat::Placement { pos: p.player.pos, yaw_deg: -dequantize(p.look.0), scale: [1.0; 3] }
    }

    /// Take health from a player, who flinches with the `wound`. If it is all of it they die and
    /// fall for it instead, and `attacker` (who is not the victim) is credited, with the kill on
    /// their screen.
    fn hurt_player(&mut self, victim: usize, attacker: Option<Occupant>, amount: f32, at: V3, wound: Wound, cause: Cause, ev: &mut Vec<Event>) {
        let now = self.time;
        let struck = self.occupant(victim);
        // Who is credited: someone else, and they may have left since they threw
        let attacker = attacker.filter(|&a| a != struck);
        let theirs = attacker.filter(|&a| self.present(a)).map(|a| a.slot);
        let p = &mut self.players[victim];
        if p.respawn_at.is_some() {
            return;
        }
        if !self.god {
            p.hp = (p.hp - amount).max(0.0);
        }
        let killed = p.hp <= 0.0;
        let hp = p.hp;
        let pos = p.player.pos;
        if let Some(a) = theirs {
            self.players[a].target_hp = Some(hp);
        }
        next_rng(&mut self.rng);
        let roll = (self.rng >> 40) as u32;
        ev.push(Event::Log(format!("t={now:.2} player {victim} hurt by {amount:.0}{}, hp now {hp:.0}", attacker.map_or(String::new(), |a| format!(" from player {}", a.slot)))));
        ev.push(Event::Sound { name: "C_BHTHT.WAV".to_string(), at: Some(at), gain: 1.0 });
        ev.push(Event::Sound { name: voice(killed, roll), at: Some(pos), gain: 1.0 });
        let p = &mut self.players[victim];
        // Players flinch and fall as those on their feet do
        let up = combat::Posture::default();
        let motion = match (wound, killed) {
            (Wound::Shot(part, from), false) => combat::hit_reaction(part, from, up).unwrap_or("thwu_bf"),
            (Wound::Blast(from), false) => combat::hit_reaction(combat::BodyPart::Torso, from, up).unwrap_or("thwu_bf"),
            (Wound::Shot(part, from), true) => combat::death_motion(part, from, roll >> 8, up),
            (Wound::Blast(from), true) => combat::blast_death_motion(from, up),
        };
        if let (Wound::Shot(part, _), false, true) = (wound, killed, amount > 0.0 && !self.god) {
            p.wounds = p.wounds.saturating_add(1);
            if matches!(part, combat::BodyPart::LeftLeg | combat::BodyPart::RightLeg) {
                p.leg_wounds = p.leg_wounds.saturating_add(1);
            }
        }
        p.hit = Some(Struck { motion, start: now });
        ev.push(Event::Log(format!("t={now:.2} player {victim} {} with {motion}", if killed { "falls" } else { "flinches" })));
        if !killed {
            return;
        }
        p.respawn_at = Some(now + RESPAWN_SECONDS);
        p.deaths += 1;
        p.equipped_grenade = false;
        p.charge_from = None;
        p.throw = None;
        p.reload_until = None;
        p.gun_trigger = false;
        match attacker {
            Some(a) => {
                self.count(a, |p| p.stats.kills += 1);
                self.count(struck, |p| p.killed_by = Some(a));
                if let Some(slot) = theirs {
                    self.players[slot].kills += 1;
                    let text = match cause {
                        Cause::HeadShot => "HEAD SHOT KILL",
                        Cause::Shot | Cause::Terrorist => "KILLED",
                        Cause::Grenade => "GRENADE KILL",
                    };
                    self.announce(slot, text, 1.2);
                }
                let by = self.name_of(a);
                self.announce(victim, &format!("KILLED BY {by}"), RESPAWN_SECONDS);
            }
            None if cause == Cause::Terrorist => self.announce(victim, "KILLED BY A TERRORIST", RESPAWN_SECONDS),
            None => self.announce(victim, "YOU KILLED YOURSELF", RESPAWN_SECONDS),
        }
        ev.push(Event::Log(format!("t={now:.2} player {victim} died")));
        ev.push(Event::Kill {
            killer: attacker.map(|a| a.slot),
            victim,
            cause,
            killer_joined: attacker.map_or(0, |a| a.joined),
            victim_joined: struck.joined,
            killer_name: attacker.map_or(String::new(), |a| self.name_of(a)),
            victim_name: self.name_of(struck),
        });
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
        // Their own base, in a team game
        let spawns = match self.players[slot].side {
            Some(side) if !self.bases[side.index()].is_empty() => &self.bases[side.index()],
            _ => &self.spawns,
        };
        let spawn = spawns.iter().copied().fold(None, |best: Option<(V3, f32)>, s| {
            let d = nearest(&s);
            if best.is_none_or(|b| d > b.1) { Some((s, d)) } else { best }
        }).map_or(here, |b| b.0);
        let (kills, deaths) = (self.players[slot].kills, self.players[slot].deaths);
        let (look, side, joined) = (self.players[slot].look, self.players[slot].side, self.players[slot].joined);
        self.players[slot] = self.new_player(spawn);
        let p = &mut self.players[slot];
        (p.kills, p.deaths, p.look, p.side, p.joined) = (kills, deaths, look, side, joined);
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
            None => ([pos[0], pos[1] + 10.0, pos[2]], self.eye(slot)),
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
        // The rooms drawing starts from: those whose floor the player stands on, as
        // overlapping rooms (a stairwell beside a hall, an upstairs over a downstairs)
        // all claim the same spot. Usually one; two where rooms share the floor (rm02's
        // deck). Airborne, the last ones stay; flying, the smallest room around the view.
        let controlled = observer.is_none();
        let floor_rooms: Vec<u32> = if controlled {
            self.world.floor_rooms([probe[0], probe[1] - 10.0, probe[2]]).into_iter().filter(|&n| self.visibility.is_room(n)).collect()
        } else {
            Vec::new()
        };
        let p = &self.players[slot];
        let render_rooms: Vec<u32> = match floor_rooms {
            rooms if !rooms.is_empty() => rooms,
            _ if controlled && !p.last_render_rooms.is_empty() => p.last_render_rooms.clone(),
            _ => p.current_room.and_then(|i| self.visibility.number_of(i)).map(|n| vec![n]).unwrap_or_else(|| player_rooms.clone()),
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
    use movement::{crouched_speed, ground_speed, limp_speed};

    /// Whether `killer` killed `victim` this way, whatever they were called
    fn killed(events: &[Event], killer: Option<usize>, victim: usize, cause: Cause) -> bool {
        events.iter().any(|e| matches!(e, Event::Kill { killer: k, victim: v, cause: c, .. } if (*k, *v, *c) == (killer, victim, cause)))
    }
    use crate::collision::Params;

    /// A sim of the demo room with nobody in it but the player
    fn demo() -> Sim {
        let level = Rc::new(crate::demo::map());
        let world = CollisionWorld::from_map(&level, Params::default());
        let config = Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        let mut sim = Sim::new(level, world, config, None, Vec::new());
        sim.add_player([0.0, 0.0, -350.0]);
        sim
    }

    fn holding() -> Input {
        Input { run: true, ..Input::default() }
    }

    #[test]
    fn a_god_takes_no_harm_but_flinches() {
        let mut sim = demo();
        sim.god = true;
        let mut ev = Vec::new();
        let wound = Wound::Shot(combat::BodyPart::Torso, combat::Facing::Front);
        sim.hurt_player(0, None, PLAYER_HP * 2.0, [0.0, 100.0, -350.0], wound, Cause::Terrorist, &mut ev);
        let p = &sim.players[0];
        assert_eq!(p.hp, PLAYER_HP);
        assert!(p.respawn_at.is_none() && p.hit.is_some());
    }

    fn shot_at(sim: &mut Sim, part: combat::BodyPart) {
        let wound = Wound::Shot(part, combat::Facing::Front);
        sim.hurt_player(0, None, 30.0, [0.0, 50.0, -350.0], wound, Cause::Terrorist, &mut Vec::new());
    }

    #[test]
    fn a_bullet_that_wounds_counts_and_one_in_a_leg_counts_as_a_leg_wound() {
        let mut sim = demo();
        shot_at(&mut sim, combat::BodyPart::Torso);
        assert_eq!((sim.players[0].wounds, sim.players[0].leg_wounds), (1, 0));
        shot_at(&mut sim, combat::BodyPart::RightLeg);
        assert_eq!((sim.players[0].wounds, sim.players[0].leg_wounds), (2, 1));
        let wound = Wound::Shot(combat::BodyPart::LeftLeg, combat::Facing::Front);
        sim.hurt_player(0, None, PLAYER_HP, [0.0, 50.0, -350.0], wound, Cause::Terrorist, &mut Vec::new());
        assert_eq!((sim.players[0].wounds, sim.players[0].leg_wounds), (2, 1));
        let mut sim = demo();
        let wound = Wound::Shot(combat::BodyPart::LeftLeg, combat::Facing::Front);
        sim.hurt_player(0, None, 0.0, [0.0, 50.0, -350.0], wound, Cause::Terrorist, &mut Vec::new());
        assert_eq!((sim.players[0].wounds, sim.players[0].leg_wounds), (0, 0));
    }

    #[test]
    fn a_blast_does_not_make_a_player_limp() {
        let mut sim = demo();
        sim.hurt_player(0, None, 30.0, [0.0, 50.0, -350.0], Wound::Blast(combat::Facing::Front), Cause::Grenade, &mut Vec::new());
        assert!(sim.players[0].hp < PLAYER_HP);
        assert_eq!((sim.players[0].wounds, sim.players[0].leg_wounds), (0, 0));
    }

    #[test]
    fn wounds_last_until_the_player_is_back() {
        let mut sim = demo();
        shot_at(&mut sim, combat::BodyPart::LeftLeg);
        sim.respawn(0);
        assert_eq!((sim.players[0].wounds, sim.players[0].leg_wounds), (0, 0));
        shot_at(&mut sim, combat::BodyPart::LeftLeg);
        sim.new_round(1);
        assert_eq!((sim.players[0].wounds, sim.players[0].leg_wounds), (0, 0));
    }

    #[test]
    fn a_leg_wound_slows_a_walk_to_the_wounded_motions_speed_and_never_speeds_a_strafe_up() {
        use movement::Movement::*;
        assert_eq!(movement::pace(Walk, false, 0, 1), limp_speed(Walk, false));
        assert!((movement::pace(Walk, false, 1, 1) - limp_speed(Walk, false) / 1.05).abs() < 1e-3);
        assert_eq!(movement::pace(Run, false, 0, 1), limp_speed(Run, false));
        for m in [WalkLeft, WalkRight, StrafeLeft, StrafeRight, Back] {
            assert!(movement::pace(m, false, 0, 1) <= ground_speed(m), "{m:?}");
        }
        assert_eq!(movement::pace(WalkLeft, false, 0, 1), ground_speed(WalkLeft));
        assert_eq!(movement::pace(Run, true, 0, 1), limp_speed(Run, true));
        assert_eq!(movement::pace(Walk, true, 0, 1), crouched_speed(Walk));
        assert!((movement::pace(Run, false, 2, 0) - ground_speed(Run) / 1.1).abs() < 1e-3);
        assert!((movement::pace(Run, false, 20, 0) - ground_speed(Run) / 1.4).abs() < 1e-3);

        let mut sim = demo();
        sim.players[0].leg_wounds = 1;
        sim.players[0].wounds = 1;
        let z = sim.players[0].player.pos[2];
        run(&mut sim, &Input { forward: true, run: false, ..holding() }, 0.5);
        let gone = (sim.players[0].player.pos[2] - z).abs();
        let expected = limp_speed(movement::Movement::Walk, false) / 1.05 * 0.5;
        assert!((gone - expected).abs() < 5.0, "{gone} against {expected}");
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
    fn every_wound_and_death_cry_is_a_sound_in_the_game() {
        let dir = std::path::Path::new("data/sound");
        if !dir.exists() {
            eprintln!("skipping: {} not found", dir.display());
            return;
        }
        for killed in [false, true] {
            for roll in 0..60 {
                let name = voice(killed, roll);
                assert!(dir.join(&name).exists(), "no such sound as {name}");
            }
        }
    }

    /// A sim of rm11 with the player at `feet`, or None without the game data
    fn rm11(feet: V3) -> Option<Sim> {
        let path = std::path::Path::new("data/map/rm11/rm11.map");
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return None;
        }
        let level = Rc::new(crate::map::read(path).expect("rm11 loads"));
        let world = CollisionWorld::from_map(&level, Params::default());
        let config = Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        let mut sim = Sim::new(level, world, config, None, Vec::new());
        sim.add_player(feet);
        Some(sim)
    }

    // On rm11 the stairs of room 121 climb from its landing (y 32987, z past 40167) toward -z,
    // through the back wall (z 40117) and the ceiling (y 33127) of room 119, to the floor
    // above. 119's wall and ceiling used to stop the player on the way down, and its ceiling
    // to be taken for the floor on the way up, so the stairs were not drawn.
    #[test]
    fn the_player_climbs_the_stairs_through_another_room_and_comes_back_down() {
        let Some(mut sim) = rm11([29863.0, 32990.0, 40320.0]) else { return };
        let on_stairs = |p: V3| p[0] > 29757.0 && p[0] < 30027.0 && p[2] > 39907.0 && p[2] < 40167.0 && p[1] > 32990.0 && p[1] < 33150.0;
        let walk = |sim: &mut Sim, yaw: f32, seconds: f32| {
            let mut input = Input { forward: true, ..Input::default() };
            input.set_look(yaw, 0.0);
            for _ in 0..(seconds / TICK).round() as usize {
                sim.tick(&[input], TICK);
                let p = &sim.players[0];
                if on_stairs(p.player.pos) {
                    assert_eq!(p.render_rooms, vec![121], "on the stairs at {:?}, drawing starts from 121", p.player.pos);
                }
            }
            sim.players[0].player.pos
        };
        // Up, toward -z, and on across the floor above
        let top = walk(&mut sim, 180.0, 3.0);
        assert!(top[1] > 33140.0 && top[2] < 39950.0, "reached the top: {top:?}");
        // And back down to the landing
        let bottom = walk(&mut sim, 0.0, 3.0);
        assert!(bottom[1] < 33000.0 && bottom[2] > 40200.0, "back on the landing: {bottom:?}");
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
    fn another_players_shots_are_heard_where_they_left_and_ones_own_are_not() {
        let mut sim = demo();
        sim.add_player([200.0, 0.0, -350.0]).unwrap();
        let events: Vec<Event> = (0..30).flat_map(|_| sim.tick(&[holding(), Input { fire: true, ..holding() }], TICK)).collect();
        let shots: Vec<V3> = events.iter().filter_map(|e| match e { Event::Shot { start, .. } => Some(*start), _ => None }).collect();
        assert!(shots.len() >= 5, "{} shots", shots.len());
        // Each shot is heard once by the other player, from where it left
        let heard: Vec<(&str, V3)> = events.iter().filter_map(|e| shot_sound(e, 0)).collect();
        assert_eq!(heard, shots.iter().map(|&s| (PLAYER_SHOT, s)).collect::<Vec<_>>());
        // The shooter hears their own through the firing loop instead
        assert!(events.iter().all(|e| shot_sound(e, 1).is_none()));
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
        // Each part of the body takes so much, so where a bullet lands says what it does
        sim.gunfire = None;
        sim.spawns = vec![[0.0, 0.0, -350.0], [0.0, 0.0, 350.0]];
        sim.add_player([0.0, 0.0, -100.0]);
        // Standing still a moment closes the spread their shots leave in
        for _ in 0..30 {
            sim.tick(&[holding(), holding()], TICK);
        }
        sim
    }

    fn aim(pitch: f32) -> Input {
        let mut i = Input { fire: true, ..holding() };
        i.set_look(0.0, pitch);
        i
    }

    #[test]
    fn standing_a_walk_steps_to_the_side_more_slowly_than_a_run() {
        let mut sim = demo();
        for (run_key, pace) in [(true, ground_speed(movement::Movement::StrafeRight)), (false, ground_speed(movement::Movement::WalkRight))] {
            let x = sim.players[0].player.pos[0];
            run(&mut sim, &Input { right: true, run: run_key, ..holding() }, 0.5);
            let side = sim.players[0].player.pos[0] - x;
            assert!((side.abs() - pace * 0.5).abs() < 10.0, "{side} against {pace}");
        }
    }

    #[test]
    fn a_diagonal_goes_27_degrees_off_at_the_forward_or_back_pace() {
        for (input, pace) in [
            (Input { forward: true, right: true, ..holding() }, ground_speed(movement::Movement::Run)),
            (Input { forward: true, left: true, run: false, ..holding() }, ground_speed(movement::Movement::Walk)),
            (Input { back: true, right: true, ..holding() }, ground_speed(movement::Movement::Back)),
        ] {
            let mut sim = demo();
            let start = sim.players[0].player.pos;
            run(&mut sim, &input, 0.25);
            let end = sim.players[0].player.pos;
            let (dx, dz) = (end[0] - start[0], end[2] - start[2]);
            let gone = dx.hypot(dz);
            assert!((gone - pace * 0.25).abs() < 5.0, "{gone} against {pace}");
            let off = dx.abs().atan2(dz.abs()).to_degrees();
            assert!((off - 27.0).abs() < 0.5, "{off} degrees off");
            // To the side the key says, ahead or behind as the other one does
            assert_eq!(dx > 0.0, input.right, "{dx}");
        }
    }

    #[test]
    fn a_peek_moves_the_eye_to_the_side_and_moving_off_ends_it_at_once() {
        let mut sim = demo();
        let (feet, eye) = (sim.players[0].player.pos, sim.eye(0));
        run(&mut sim, &Input { peek_left: true, ..holding() }, 1.0);
        let peeked = sim.eye(0);
        // Facing along -z, the left is -x
        assert!(peeked[0] - eye[0] < -50.0 && peeked[1] < eye[1], "{peeked:?} from {eye:?}");
        run(&mut sim, &holding(), 1.0);
        assert_eq!(sim.eye(0), eye);
        // Holding the key, the player stays where they are
        run(&mut sim, &Input { peek_left: true, forward: true, ..holding() }, 1.0);
        assert_eq!(sim.players[0].player.pos, feet, "moved while peeking");
        // Let go, they walk off straight away, and the lean is soon gone
        let walk = Input { forward: true, ..holding() };
        sim.tick(&[walk], TICK);
        assert_ne!(sim.players[0].player.pos, feet, "held by the lean");
        run(&mut sim, &walk, 0.3);
        assert!(sim.players[0].peek.is_none());
    }

    #[test]
    fn a_peek_stops_short_at_a_wall() {
        let mut sim = demo();
        // 40 from the west wall, which leaves 10 past the body's 30
        let x = -crate::demo::WIDTH / 2.0 + 40.0;
        sim.players[0].player.pos[0] = x;
        run(&mut sim, &holding(), 0.1);
        let eye = sim.eye(0);
        run(&mut sim, &Input { peek_left: true, ..holding() }, 1.0);
        let moved = sim.eye(0)[0] - eye[0];
        assert!((-11.0..-8.0).contains(&moved), "leaned {moved} into the wall");
        run(&mut sim, &holding(), 1.0);
        run(&mut sim, &Input { peek_right: true, ..holding() }, 1.0);
        assert!(sim.eye(0)[0] - eye[0] > 45.0, "{:?}", sim.eye(0));
    }

    #[test]
    fn crouching_lowers_the_eyes_and_slows_the_player_until_they_stand() {
        let mut sim = demo();
        let params = *sim.world.params();
        let crouch = Input { crouch: true, ..holding() };
        sim.tick(&[crouch], TICK);
        assert!(sim.players[0].player.crouched);
        assert_eq!(sim.eye(0)[1] - sim.players[0].player.pos[1], params.crouch_eye_height);
        // A second's crouched run covers its own pace, well short of a standing run
        let z = sim.players[0].player.pos[2];
        run(&mut sim, &Input { forward: true, ..crouch }, 1.0);
        let crouched = sim.players[0].player.pos[2] - z;
        assert!((crouched.abs() - crouched_speed(movement::Movement::Run)).abs() < 10.0, "{crouched}");
        // Crouched, stepping to the side goes at the one pace, walking or not
        for run_key in [true, false] {
            let x = sim.players[0].player.pos[0];
            run(&mut sim, &Input { right: true, run: run_key, ..crouch }, 1.0);
            let side = sim.players[0].player.pos[0] - x;
            assert!((side.abs() - crouched_speed(movement::Movement::StrafeRight)).abs() < 10.0, "{side}");
        }
        // Let go of crouching in the open, and they are standing again
        sim.tick(&[holding()], TICK);
        assert!(!sim.players[0].player.crouched);
        assert_eq!(sim.eye(0)[1] - sim.players[0].player.pos[1], params.eye_height);
    }

    #[test]
    fn a_level_shot_passes_over_a_crouched_player() {
        let mut sim = duel();
        sim.tick(&[Input::default(), Input { crouch: true, ..holding() }], TICK);
        sim.tick(&[aim(0.0), Input { crouch: true, ..holding() }], TICK);
        assert_eq!(sim.players[1].hp, 100.0);
    }

    #[test]
    fn a_bullet_hurts_the_player_it_hits_by_where_it_hits() {
        // Level with the eyes is the head, and a head shot kills
        let mut sim = duel();
        let events = sim.tick(&[aim(0.0), holding()], TICK);
        assert!(killed(&events, Some(0), 1, Cause::HeadShot));
        // With a Rainbow operative's cry, not a terrorist's
        assert!(events.iter().any(|e| matches!(e, Event::Sound { name, .. } if name.starts_with("English/tm_inc"))));
        assert!(!events.iter().any(|e| matches!(e, Event::Sound { name, .. } if name.starts_with("English/re_"))));
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
    fn a_peeking_player_is_hit_where_their_head_has_gone() {
        let peeking = Input { peek_right: true, ..holding() };
        let leaned = || {
            let mut sim = duel();
            run_both(&mut sim, &holding(), &peeking, 1.0);
            sim
        };
        // Where the head was, the bullet goes past
        let mut sim = leaned();
        sim.tick(&[aim(0.0), peeking], TICK);
        assert_eq!(sim.players[1].hp, 100.0);
        // Where it has gone, it is a head shot
        let mut sim = leaned();
        let target = sim.eye(1);
        sim.players[0].player.pos[0] = target[0];
        let from = sim.eye(0);
        let pitch = (target[1] - from[1]).atan2((target[2] - from[2]).abs()).to_degrees();
        let events = sim.tick(&[aim(pitch), peeking], TICK);
        assert!(killed(&events, Some(0), 1, Cause::HeadShot), "{events:?}");
    }

    /// A duel between two players with the game's model, to be hit where the posed body is.
    /// None without the game's data.
    fn posed_duel() -> Option<Sim> {
        let mut sim = duel();
        sim.characters = Some(Characters::load(std::path::Path::new("data/character/team_hrt/blue_M_a.CRP"), "up_none_cyc")?);
        sim.characters.as_ref()?.rig.as_ref()?;
        Some(sim)
    }

    /// What a level shot from player 0 at `height` above player 1's feet, `aside` to the
    /// shooter's right of their middle, hits of them
    fn level_shot(sim: &Sim, height: f32, aside: f32) -> Option<combat::BodyPart> {
        let target = sim.players[1].player.pos;
        let from = [target[0] + aside, target[1] + height, sim.players[0].player.pos[2]];
        let dir = unit3([0.0, 0.0, target[2] - from[2]]);
        sim.player_hit(Shooter::Player(0), from, dir).map(|h| h.2)
    }

    #[test]
    fn a_player_with_a_model_is_hit_where_their_posed_body_is() {
        let Some(mut sim) = posed_duel() else { return };
        run_both(&mut sim, &holding(), &holding(), 0.5);
        let standing = *sim.world.params();
        assert_eq!(level_shot(&sim, standing.eye_height, 0.0), Some(combat::BodyPart::Head));
        assert_eq!(level_shot(&sim, 130.0, 0.0), Some(combat::BodyPart::Torso));
        // Facing away from the shooter, their right leg is on the shooter's right, and a shot
        // between the legs goes through
        assert_eq!(level_shot(&sim, 40.0, 20.0), Some(combat::BodyPart::RightLeg));
        assert_eq!(level_shot(&sim, 40.0, -20.0), Some(combat::BodyPart::LeftLeg));
        assert_eq!(level_shot(&sim, 40.0, 0.0), None);
        // Between the body and the shooter's side, somewhere an arm holds the gun
        let arms = (-60..=60).step_by(4).flat_map(|x| (80..=160).step_by(8).map(move |y| (x, y)))
            .filter_map(|(x, y)| level_shot(&sim, y as f32, x as f32))
            .filter(|p| matches!(p, combat::BodyPart::LeftArm | combat::BodyPart::RightArm)).count();
        assert!(arms > 0, "no arm anywhere");
        // Past the body's side, nothing
        assert_eq!(level_shot(&sim, 130.0, 70.0), None);
    }

    #[test]
    fn a_peeking_player_with_a_model_is_hit_where_the_lean_has_put_their_head() {
        let Some(mut sim) = posed_duel() else { return };
        let peeking = Input { peek_right: true, ..holding() };
        run_both(&mut sim, &holding(), &peeking, 1.0);
        let lean = sim.peek_offset(1);
        // The eye is at the top of the head, which has gone right and down with it. Player 1
        // faces the same way as player 0, so their right is the shooter's right.
        let head = sim.world.params().eye_height + lean[1] - 12.0;
        assert_eq!(level_shot(&sim, head, lean[0] + 8.0), Some(combat::BodyPart::Head));
        assert_ne!(level_shot(&sim, sim.world.params().eye_height, 0.0), Some(combat::BodyPart::Head));
    }

    fn run_both(sim: &mut Sim, first: &Input, second: &Input, seconds: f32) {
        for _ in 0..(seconds / TICK).round() as usize {
            sim.tick(&[*first, *second], TICK);
        }
    }

    #[test]
    fn a_bullet_does_what_the_gunfire_chart_rolls_for_it() {
        let Ok(gunfire) = crate::gfcm::read(std::path::Path::new("data/kit/gfcm.txt")) else { return };
        // A chest shot close up: wounds, or kills outright
        let (mut wounded, mut killed) = (0, 0);
        for seed in 0..40 {
            let mut sim = duel();
            sim.gunfire = Some(gunfire.clone());
            sim.rng = seed * 7919 + 1;
            sim.tick(&[aim(-14.6), holding()], TICK);
            match sim.players[1].hp {
                hp if hp == 0.0 => killed += 1,
                hp => {
                    // A wound is a fifth, as a player takes five
                    assert!((hp - (PLAYER_HP - PLAYER_HP / 5.0)).abs() < 1e-3, "{hp}");
                    wounded += 1;
                }
            }
        }
        assert!(wounded > 5 && killed > 5, "{wounded} wounded, {killed} killed");
    }

    #[test]
    fn nothing_hurts_a_player_when_players_cannot_be_hurt() {
        let mut sim = duel();
        sim.pvp = false;
        sim.tick(&[aim(0.0), holding()], TICK);
        assert_eq!(sim.players[1].hp, 100.0);
    }

    #[test]
    fn a_wounded_player_flinches_for_the_side_the_shot_came_from() {
        // Player 1 faces away from the shooter, who hits them in the chest from behind
        let mut sim = duel();
        sim.tick(&[aim(-14.6), holding()], TICK);
        assert_eq!(sim.players[1].hit, Some(Struck { motion: "thwu_bb", start: sim.time }));
        run(&mut sim, &holding(), FLINCH_SECONDS + TICK);
        assert!(sim.players[1].hit.is_none());
    }

    #[test]
    fn a_shot_low_on_the_body_hits_the_leg_on_its_side() {
        // Aimed at the knees a little to either side of the middle; player 1's right is +X
        for (yaw, motion) in [(2.3, "thwu_ll"), (-2.3, "thwu_rl")] {
            let mut sim = duel();
            let mut shot = aim(-26.6);
            shot.set_look(yaw, -26.6);
            sim.tick(&[shot, holding()], TICK);
            assert_eq!(sim.players[1].hp, 100.0 - combat::damage(combat::BodyPart::LeftLeg), "yaw {yaw}");
            assert_eq!(sim.players[1].hit.map(|f| f.motion), Some(motion), "yaw {yaw}");
        }
    }

    #[test]
    fn a_dead_player_falls_for_where_and_from_where_the_shot_came_until_they_come_back() {
        let mut sim = duel();
        sim.tick(&[aim(-14.6), holding()], TICK);
        while sim.players[1].respawn_at.is_none() {
            sim.tick(&[aim(0.0), holding()], TICK);
        }
        let fall = sim.players[1].hit.expect("a fall");
        assert_eq!(fall.start, sim.time);
        // A head shot from behind
        assert!(["thiu_hb", "hw_bk_up2", "hw_bk_up2_g", "thdu_3"].contains(&fall.motion), "{}", fall.motion);
        // Kept past a flinch's time, for as long as they are dead
        run(&mut sim, &holding(), FLINCH_SECONDS + 0.1);
        assert_eq!(sim.players[1].hit, Some(fall));
        run(&mut sim, &holding(), RESPAWN_SECONDS - FLINCH_SECONDS);
        assert!(sim.players[1].respawn_at.is_none());
        assert!(sim.players[1].hit.is_none());
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
            for _ in 0..16 {
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
        let by = sim.occupant(0);
        sim.thrown.push(Thrown { grenade: Grenade { pos: [0.0, 100.0, -90.0], vel: [0.0; 3], fuse: 0.0, spin: 0.0, at_rest: true }, owner: 0, by, side: None });
        let events = sim.tick(&[holding(), holding()], TICK);
        assert!(events.iter().any(|e| matches!(e, Event::Blast { .. })));
        assert_eq!(sim.players[1].hp, 0.0);
        assert_eq!(sim.players[0].kills, 1);
        assert!(killed(&events, Some(0), 1, Cause::Grenade));
        assert!(killed(&events, None, 0, Cause::Grenade));
        // So close that the thrower is caught too: that is a death, but not a kill
        assert_eq!((sim.players[0].kills, sim.players[0].deaths), (1, 1));
    }

    /// Everyone here plays a round, and what they do counts from now
    fn counting(sim: &mut Sim) {
        sim.round = (0..sim.players.len()).filter(|&i| sim.players[i].active)
            .map(|i| Participant { who: sim.occupant(i), side: sim.players[i].side, left: false, end: Ending::Alive, stats: RoundStats::default(), killed_by: None })
            .collect();
        sim.open_round();
    }

    fn stats(sim: &Sim, i: usize) -> RoundStats {
        sim.round[i].stats
    }

    /// One shot at `pitch` from player 0, then long enough for the gun to fire again
    fn shoot_once(sim: &mut Sim, pitch: f32) {
        sim.tick(&[aim(pitch), holding()], TICK);
        for _ in 0..10 {
            sim.tick(&[holding(), holding()], TICK);
        }
    }

    #[test]
    fn a_round_counts_rounds_fired_hits_taken_and_kills_only_while_counting_is_open() {
        let mut sim = duel();
        // Before counting opens, nothing counts
        shoot_once(&mut sim, 8.0);
        assert!(sim.round.is_empty());
        counting(&mut sim);
        // Over their head: fired, not a hit
        shoot_once(&mut sim, 8.0);
        assert_eq!(stats(&sim, 0), RoundStats { fired: 1, ..RoundStats::default() });
        // In the chest: a hit for one, a bullet taken for the other
        shoot_once(&mut sim, -14.6);
        assert_eq!(stats(&sim, 0), RoundStats { fired: 2, hits: 1, ..RoundStats::default() });
        assert_eq!(stats(&sim, 1), RoundStats { taken: 1, ..RoundStats::default() });
        // In the head: a kill, and who did it
        shoot_once(&mut sim, 0.0);
        assert_eq!(stats(&sim, 0), RoundStats { kills: 1, fired: 3, hits: 2, taken: 0 });
        assert_eq!(sim.round[1].killed_by, Some(sim.occupant(0)));
        assert_eq!(stats(&sim, 0).hit_percent(), 66);
        // The body takes no more: the bullet goes past it
        shoot_once(&mut sim, 0.0);
        assert_eq!((stats(&sim, 0).hits, stats(&sim, 1).taken), (2, 2));
        // Decided: how they ended is kept, and nothing counts after
        sim.close_round();
        assert_eq!((sim.round[0].end, sim.round[1].end), (Ending::Alive, Ending::Killed));
        shoot_once(&mut sim, 8.0);
        assert_eq!(stats(&sim, 0).fired, 4);
    }

    #[test]
    fn a_hit_on_a_teammate_counts_only_if_it_could_hurt_them_and_god_mode_still_counts() {
        // Friendly fire off: the bullet passes through a teammate, a miss
        let mut sim = duel();
        sim.players[0].side = Some(Side::Blue);
        sim.players[1].side = Some(Side::Blue);
        sim.friendly_fire = false;
        counting(&mut sim);
        shoot_once(&mut sim, -14.6);
        assert_eq!((stats(&sim, 0).fired, stats(&sim, 0).hits, stats(&sim, 1).taken), (1, 0, 0));
        // On, it is a hit, and a teammate killed counts as a kill
        sim.friendly_fire = true;
        shoot_once(&mut sim, 0.0);
        assert_eq!((stats(&sim, 0).hits, stats(&sim, 0).kills, stats(&sim, 1).taken), (1, 1, 1));
        // In god mode no one is hurt, but the bullet struck
        let mut sim = duel();
        sim.god = true;
        counting(&mut sim);
        shoot_once(&mut sim, 0.0);
        assert_eq!(sim.players[1].hp, PLAYER_HP);
        assert_eq!((stats(&sim, 0).hits, stats(&sim, 1).taken), (1, 1));
    }

    #[test]
    fn a_grenade_counts_its_kills_but_no_rounds_and_not_the_thrower_killing_themselves() {
        let mut sim = duel();
        counting(&mut sim);
        let by = sim.occupant(0);
        sim.thrown.push(Thrown { grenade: Grenade { pos: [0.0, 100.0, -90.0], vel: [0.0; 3], fuse: 0.0, spin: 0.0, at_rest: true }, owner: 0, by, side: None });
        sim.tick(&[holding(), holding()], TICK);
        assert_eq!(stats(&sim, 0), RoundStats { kills: 1, ..RoundStats::default() });
        assert_eq!(stats(&sim, 1), RoundStats::default());
        assert_eq!((sim.round[0].killed_by, sim.round[1].killed_by), (None, Some(by)));
    }

    /// The thrower in slot 0 of a duel throws, leaves, and a newcomer on `side` takes the slot
    /// where the grenade will go off. Player 1 is on Gold.
    fn thrower_gone(friendly_fire: bool, side: Side) -> (Sim, Occupant) {
        let mut sim = duel();
        sim.friendly_fire = friendly_fire;
        (sim.players[0].side, sim.players[1].side) = (Some(Side::Blue), Some(Side::Gold));
        counting(&mut sim);
        let by = sim.occupant(0);
        sim.names = vec![(by, "Thrower".to_string()), (sim.occupant(1), "Target".to_string())];
        sim.thrown.push(Thrown { grenade: Grenade { pos: [0.0, 100.0, -90.0], vel: [0.0; 3], fuse: 0.0, spin: 0.0, at_rest: true }, owner: 0, by, side: Some(Side::Blue) });
        sim.remove_player(0);
        assert_eq!(sim.add_player([0.0, 0.0, -100.0]), Some(0));
        sim.players[0].side = Some(side);
        (sim, by)
    }

    #[test]
    fn the_side_a_grenade_was_thrown_from_is_part_of_the_game_state() {
        // Alike but for that side, and with friendly fire off it decides who is hurt
        let checksums: Vec<u64> = [None, Some(Side::Blue), Some(Side::Gold)].into_iter().map(|side| {
            let mut sim = duel();
            let by = sim.occupant(0);
            sim.thrown.push(Thrown { grenade: Grenade { pos: [0.0, 100.0, -90.0], vel: [0.0; 3], fuse: 1.0, spin: 0.0, at_rest: true }, owner: 0, by, side });
            sim.checksum()
        }).collect();
        assert_ne!(checksums[0], checksums[1]);
        assert_ne!(checksums[1], checksums[2]);
        assert_ne!(checksums[0], checksums[2]);
    }

    #[test]
    fn a_grenade_thrown_before_its_owner_left_credits_them_and_not_whoever_has_the_slot() {
        let (mut sim, by) = thrower_gone(true, Side::Gold);
        let events = sim.tick(&[holding(), holding()], TICK);
        // Their kills are theirs, though they have gone
        assert!(sim.round[0].left);
        assert_eq!(sim.round[0].stats.kills, 2, "the target and the newcomer");
        assert_eq!(sim.round[1].killed_by, Some(by));
        assert_eq!(sim.players[0].kills, 0, "the newcomer is credited");
        let named: Vec<(&str, &str)> = events.iter().filter_map(|e| match e {
            Event::Kill { killer_name, victim_name, .. } => Some((killer_name.as_str(), victim_name.as_str())),
            _ => None,
        }).collect();
        assert!(named.contains(&("Thrower", "Target")), "{named:?}");
        // The newcomer is not the thrower: friendly fire off, one on the thrower's side is spared,
        // as the thrower would not have been
        let (mut sim, _) = thrower_gone(false, Side::Blue);
        sim.tick(&[holding(), holding()], TICK);
        assert_eq!(sim.players[0].hp, PLAYER_HP);
        assert_eq!(sim.players[1].hp, 0.0);
        // and one on the other side is not
        let (mut sim, _) = thrower_gone(false, Side::Gold);
        sim.tick(&[holding(), holding()], TICK);
        assert_eq!(sim.players[0].hp, 0.0);
    }

    #[test]
    fn a_bullet_reaches_a_player_in_the_open_from_where_rooms_overlap() {
        // Standing in citystreet_large's killhouse where rooms 1 and 3 overlap, past the plane of
        // the portal between them, firing into room 3 where the other player stands
        let setup = crate::net::proto::Setup {
            map: "data/map/m00/citystreet_large.map".to_string(), model: "data/character/team_hrt/blue_M_a.CRP".to_string(), idle: "up_none_cyc".to_string(),
            magazine: 30, ammo: 30, reserve: 900, grenades: 3, speed_scale: 1.0, seed: 99, culling: true, pvp: true,
            friendly_fire: true, respawn: true, bases: [Vec::new(), Vec::new()], uniforms: crate::net::proto::UNIFORMS.map(String::from), base_pairs: Vec::new(), doors_open: None, mission: None, npcs: Vec::new(),
        };
        let Ok(base) = crate::net::setup::build_sim(&setup) else { return };
        drop(base);
        let from = [33533.0, 32887.0, 34907.4];
        for yaw in [313.3f32, 330.0, 341.0, 355.0, 10.6, 19.6] {
            for d in [300.0f32, 450.0] {
                let mut sim = crate::net::setup::build_sim(&setup).unwrap();
                sim.gunfire = None;
                let (dx, dz) = (-yaw.to_radians().sin(), yaw.to_radians().cos());
                let to = [from[0] + dx * d, from[1], from[2] + dz * d];
                let (a, b) = (sim.add_player(from).unwrap(), sim.add_player(to).unwrap());
                // Standing still a moment closes the spread
                run(&mut sim, &Input::default(), 0.5);
                let eye = sim.eye(a);
                let chest = [to[0], to[1] + 0.7 * sim.world.params().body_height(false), to[2]];
                let v = [chest[0] - eye[0], chest[1] - eye[1], chest[2] - eye[2]];
                let flat = v[0].hypot(v[2]);
                let wall = sim.world.bullet_hit(eye, unit3(v), 20000.0).map_or(f32::INFINITY, |h| h.dist);
                assert!(wall > d + 50.0, "a wall in the way at {yaw} degrees, {d} units");
                let mut aim = Input { fire: true, ..Input::default() };
                aim.set_look((-v[0]).atan2(v[2]).to_degrees(), v[1].atan2(flat).to_degrees());
                let events = sim.tick(&[aim, Input::default()], TICK);
                assert!(events.iter().any(|e| matches!(e, Event::Shot { hit_character: true, .. })), "missed at {yaw} degrees, {d} units");
                assert!(sim.players[b].hp < PLAYER_HP);
            }
        }
    }

    #[test]
    fn a_camera_is_drawn_from_the_rooms_a_view_flying_there_would_be() {
        let setup = crate::net::proto::Setup { map: "data/map/m00/citystreet_large.map".to_string(), ..crate::net::lab::demo_setup() };
        let Ok(mut sim) = crate::net::setup::build_sim(&setup) else { return };
        let slot = sim.add_player([33533.0, 32887.0, 34907.4]).unwrap();
        // Flying from the killhouse out over the level, room to room
        for k in 0..40 {
            let at = [33533.0 - k as f32 * 60.0, 33052.0 - k as f32 * 10.0, 34907.4 - k as f32 * 120.0];
            let hint = sim.players[slot].current_room;
            sim.predict(slot, &Input { observer: Some(at), ..Input::default() }, TICK);
            let (rooms, room) = sim.view_rooms(at, hint);
            assert_eq!((rooms, room), (sim.players[slot].render_rooms.clone(), sim.players[slot].current_room), "at {at:?}");
        }
    }

    #[test]
    fn a_player_watched_upstairs_is_drawn_from_their_floor_wherever_the_camera_was() {
        // citystreet_large's upper floor is a thin room: the eye is in no room, and the
        // camera's last room (the basement, say) must not be drawn in its place
        let path = std::path::Path::new("data/map/m00/citystreet_large.map");
        let setup = crate::net::proto::Setup { map: path.display().to_string(), ..crate::net::lab::demo_setup() };
        let Ok(mut sim) = crate::net::setup::build_sim(&setup) else { return };
        let bases = crate::levels::find_bases(path);
        for q in bases[0].blue.starts() {
            let feet = [q[0], sim.world.ground_at(q[0], q[2], q[1] + 100.0).unwrap(), q[2]];
            let slot = sim.add_player(feet).unwrap();
            run(&mut sim, &Input::default(), 0.5);
            let (feet, standing) = (sim.players[slot].player.pos, sim.players[slot].render_rooms.clone());
            assert_eq!(sim.view_rooms(sim.eye(slot), None).0, Vec::<u32>::new());
            for hint in std::iter::once(None).chain((0..sim.level.rooms.rooms.len()).map(Some)) {
                assert_eq!(sim.standing_rooms(feet, hint).0, standing, "at {feet:?} after room {hint:?}");
            }
            sim.remove_player(slot);
        }
    }
}
