//! What the server and clients say to each other.
//!
//! A client sends what its player does (`Input`, a few of the latest every time, so a lost
//! datagram costs nothing) and which snapshot it has seen last. The server sends snapshots: the
//! state of everything a client draws, with the events since the client last confirmed one.
//! Datagrams may be lost, repeated or reordered, and every message is meant to survive that.

use std::collections::VecDeque;

use anyhow::{Result, bail, ensure};

use crate::bytes::{Before, Reader, Writer};
use crate::combat::NpcState;
use crate::grenade::Grenade;
use crate::impact::Surface;
use crate::net::link::MAX_PACKET;
use crate::sim::characters::REACTION_MOTIONS;
use crate::sim::aim::{Aim, Pace};
use crate::sim::peek::Peek;
use crate::net::round::{Outcome, Phase, RoundState};
use crate::sim::{Cause, Ending, Event, Input, MAX_PLAYERS, RoundStats, Side, Sim, Struck, Throw};

type V3 = [f32; 3];

/// Bumped whenever the messages change, so that a client and server that disagree notice
pub const PROTOCOL: u8 = 16;

const MAGIC: u16 = 0x5252;

/// The most inputs sent in one datagram: the newest and the ones before it, in case those were lost
pub const INPUTS_PER_PACKET: usize = 8;

/// A character as the game starts: where it stands, and who it is
#[derive(Clone, Debug, PartialEq)]
pub struct NpcSetup {
    pub pos: V3,
    pub floor: f32,
    pub yaw: f32,
    pub scale: V3,
    pub stagger: f32,
    /// Which team member's face it wears
    pub actor: Option<u16>,
}

/// How the game was set up, which a client needs to set up the same level
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    /// The level's MAP file, or "demo" for the built-in box room
    pub map: String,
    pub model: String,
    /// The motion characters idle with
    pub idle: String,
    pub magazine: u32,
    pub ammo: u32,
    pub reserve: u32,
    pub grenades: u32,
    pub speed_scale: f32,
    pub seed: u64,
    pub culling: bool,
    /// Whether players can hurt each other, and their own side
    pub pvp: bool,
    pub friendly_fire: bool,
    /// Whether the dead come back, or wait for the next round
    pub respawn: bool,
    /// In a team game, where each side's players start: Blue's and Gold's (`Sim::bases`), and
    /// the uniform each wears (a model of the players' body)
    pub bases: [Vec<V3>; 2],
    pub uniforms: [String; 2],
    /// Every pair of bases the host can choose from, the one in `bases` among them. Empty if
    /// there is no choice.
    pub base_pairs: Vec<BasePairSetup>,
    pub doors_open: Option<f32>,
    /// The mission whose terrorists stand in the level, placed with `seed` the same way by
    /// everyone, in place of `npcs`
    pub mission: Option<String>,
    pub npcs: Vec<NpcSetup>,
}

/// A pair of bases a team game can start from: its name in the game's text
/// (`ZONE_CSL_GROUP_1_DESC`), and where Blue's and Gold's players start
#[derive(Clone, Debug, PartialEq)]
pub struct BasePairSetup {
    pub name: String,
    pub bases: [Vec<V3>; 2],
}

/// The session's settings, which the host changes in the lobby
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Session {
    /// Goes up with each new lobby, countdown or results screen. Lobby commands made for an
    /// older one are dropped. An automatic countdown keeps the lobby's.
    pub generation: u32,
    pub host: Option<u8>,
    /// A round's time limit (seconds), or none
    pub time_limit: Option<u16>,
    pub friendly_fire: bool,
    /// Which of `Setup::base_pairs` the sides start from
    pub base_pair: u8,
    /// Whether the countdown began by itself. Players can cancel it by changing ready or side.
    pub auto_countdown: bool,
}

/// What a player asks for in the lobby. The last five are the host's alone.
#[derive(Clone, Debug, PartialEq)]
pub enum LobbyCommand {
    Side(Side),
    Ready(bool),
    /// A line for everyone's chat (`chat_text`), in any phase but Live
    Say(String),
    /// Done with the round's results (Debrief), once
    Continue,
    /// The host takes everyone from the results to the lobby, once they have continued
    ToLobby,
    Start,
    Stop,
    TimeLimit(Option<u16>),
    FriendlyFire(bool),
    BasePair(u8),
}

/// The most characters a line of chat has
pub const CHAT_LENGTH: usize = 120;

/// A line of chat as it may be said: what the game's fonts can draw (printable Latin-1), with
/// no space at either end and no more than `CHAT_LENGTH` characters. None if nothing is left.
pub fn chat_text(text: &str) -> Option<String> {
    let kept: String = text.chars().filter(|c| matches!(c, ' '..='~' | '\u{a0}'..='\u{ff}')).collect();
    let kept: String = kept.trim().chars().take(CHAT_LENGTH).collect();
    let kept = kept.trim_end().to_string();
    (!kept.is_empty()).then_some(kept)
}

/// The most characters a player's name has
pub const NAME_LENGTH: usize = 24;

/// The name a player gets: what they asked for, as chat would keep it, trimmed and cut to
/// `NAME_LENGTH`, or "PLAYER n" if nothing is left. If a name in `taken` is the same, ignoring
/// case, " 2", " 3" and so on follow, with the name cut to make room.
pub fn player_name(asked: &str, slot: usize, taken: &[&str]) -> String {
    let kept: String = asked.chars().filter(|c| matches!(c, ' '..='~' | '\u{a0}'..='\u{ff}')).collect();
    let cut = |s: &str, n: usize| s.trim().chars().take(n).collect::<String>().trim_end().to_string();
    let base = match cut(&kept, NAME_LENGTH) {
        b if b.is_empty() => format!("PLAYER {}", slot + 1),
        b => b,
    };
    let fold = |s: &str| s.chars().flat_map(char::to_lowercase).collect::<String>();
    let free = |name: &str| !taken.iter().any(|t| fold(t) == fold(name));
    if free(&base) {
        return base;
    }
    (2..).map(|k| {
        let suffix = format!(" {k}");
        format!("{}{suffix}", cut(&base, NAME_LENGTH - suffix.len()))
    }).find(|n| free(n)).unwrap_or(base)
}

/// What a client sends
#[derive(Clone, Debug, PartialEq)]
pub enum ToServer {
    /// Ask to join. `token` is picked by the client and goes on everything it sends, so that a
    /// stranger cannot speak for it. `build` says which version of the game it is.
    /// `side` is the one the player would like, in a team game. `key` makes them the host if
    /// it is the server's host key, and is 0 for none.
    Connect { token: u32, build: String, side: Option<Side>, key: u64, name: String },
    /// Player inputs starting at `first_seq`, ordered oldest to newest. `ack_snapshot` acknowledges
    /// the latest received snapshot tick. `view` is the server tick at which other players were
    /// shown when the newest input was sampled, in sixteenths of a tick. Zero means unknown. Lag
    /// compensation judges shots against other players' positions at that time. `round` identifies
    /// the round loaded by the client and acknowledges its setup.
    Input { token: u32, ack_snapshot: u32, first_seq: u32, view: u32, round: u16, inputs: Vec<Input> },
    /// Leave
    Leave { token: u32 },
    /// A lobby command, the `seq`th from this client, made in lobby `generation`. It is sent
    /// until a snapshot's `lobby_ack` reaches it.
    Lobby { token: u32, seq: u32, generation: u32, command: LobbyCommand },
}

/// What the server sends
#[derive(Clone, Debug, PartialEq)]
pub enum ToClient {
    /// You are in: this is your slot and how the game is set up
    Accept { token: u32, slot: u8, tick: u32, setup: Setup },
    /// You are not: why
    Refuse { token: u32, reason: String },
    Snapshot(Snapshot),
    /// The server is closing the connection, and why
    Goodbye { token: u32, reason: String },
    /// One of the parts of a message too big for a datagram (see `split`)
    Part { tick: u32, index: u8, count: u8, bytes: Vec<u8> },
}

/// The most of a message one part carries: a datagram less the part's own header
const PART: usize = MAX_PACKET - 10;

/// The most parts a message may be split into
pub const MAX_PARTS: usize = 16;

/// A message as datagrams: itself if it fits in one, or else in parts. A part lost loses the
/// whole, so only what must is split: a snapshot in full, or with many characters in it.
pub fn split(tick: u32, bytes: Vec<u8>) -> Vec<Vec<u8>> {
    if bytes.len() <= MAX_PACKET {
        return vec![bytes];
    }
    let count = bytes.len().div_ceil(PART);
    assert!(count <= MAX_PARTS, "a message of {} bytes is too big even in parts", bytes.len());
    bytes.chunks(PART).enumerate().map(|(index, chunk)| {
        let mut w = Writer::new();
        header(&mut w, 4);
        w.u32(tick);
        w.u8(index as u8);
        w.u8(count as u8);
        w.0.extend_from_slice(chunk);
        w.0
    }).collect()
}

/// The parts of messages that have come so far, put back together once all have
#[derive(Default)]
pub struct Parts {
    /// For a few ticks, the parts of that tick's message
    pending: VecDeque<(u32, Vec<Option<Vec<u8>>>)>,
}

impl Parts {
    /// Keep a part. Returns the whole message once it is the last to come.
    pub fn add(&mut self, tick: u32, index: u8, count: u8, bytes: Vec<u8>) -> Option<Vec<u8>> {
        let (index, count) = (index as usize, count as usize);
        if index >= count || count > MAX_PARTS {
            return None;
        }
        let at = match self.pending.iter().position(|(t, _)| *t == tick) {
            Some(at) => at,
            None => {
                // Only the newest few are waited for: an older one would be out of date
                if self.pending.len() == 4 {
                    self.pending.pop_front();
                }
                self.pending.push_back((tick, vec![None; count]));
                self.pending.len() - 1
            }
        };
        let parts = &mut self.pending[at].1;
        if parts.len() != count {
            return None;
        }
        parts[index] = Some(bytes);
        if parts.iter().any(Option::is_none) {
            return None;
        }
        let (_, parts) = self.pending.remove(at)?;
        Some(parts.into_iter().flatten().flatten().collect())
    }
}

/// A player as the others need to see them, and as they themselves need to know
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerSnap {
    pub slot: u8,
    /// Occupant's join number, copied from `PlayerState::joined`.
    pub joined: u32,
    pub side: Option<Side>,
    pub pos: V3,
    pub vel_y: f32,
    pub on_ground: bool,
    pub crouched: bool,
    pub look: (u16, u16),
    pub controlled: bool,
    pub ammo: u32,
    pub reserve: u32,
    pub reload_until: Option<f32>,
    pub flash_until: f32,
    pub gun_trigger: bool,
    pub kills: u32,
    pub deaths: u32,
    pub hp: f32,
    pub respawn_at: Option<f32>,
    /// What the latest hit set the body playing (see `PlayerState::hit`), and when it began.
    /// A fall lasts until the next round, so the whole time is sent; it changes only with a hit.
    pub hit: Option<(&'static str, f32)>,
    pub equipped_grenade: bool,
    pub grenades_left: u32,
    pub charge_from: Option<f32>,
    pub throw: Option<Throw>,
    /// What the receiver sees on screen, only for their own player
    pub message: Option<(String, f32)>,
    pub target_hp: Option<f32>,
    pub peek: Option<Peek>,
    /// The spread of the receiver's gun, only for their own player
    pub aim: Option<Aim>,
    /// Said ready in the lobby
    pub ready: bool,
    /// Their name (`player_name`), which the server fills in
    pub name: String,
    /// Done with the round's results (Debrief)
    pub continued: bool,
}

/// A player's line in a round's results
#[derive(Clone, Debug, PartialEq)]
pub struct ResultRow {
    pub slot: u8,
    pub joined: u32,
    pub name: String,
    pub side: Option<Side>,
    /// They left before the round was decided
    pub left: bool,
    pub end: Ending,
    pub stats: RoundStats,
    /// The row of the participant who killed them
    pub killed_by: Option<u8>,
}

/// How a round went, made once when it is decided, from who played it (`Sim::round`)
#[derive(Clone, Debug, PartialEq)]
pub struct RoundResults {
    pub number: u16,
    pub outcome: Outcome,
    /// The row of the last one standing, in Survival
    pub winner: Option<u8>,
    /// Whole seconds from being let go to the decision, 0 if they never were
    pub played: u16,
    pub rows: Vec<ResultRow>,
}

/// Round `number`'s results, from who played it, as the sim has them now that it is decided
pub fn round_results(sim: &Sim, number: u16, outcome: Outcome) -> RoundResults {
    let row_of = |who: crate::sim::Occupant| sim.round.iter().position(|p| p.who == who).map(|i| i as u8);
    let rows = sim.round.iter().map(|p| ResultRow {
        slot: p.who.slot as u8,
        joined: p.who.joined,
        name: sim.name_of(p.who),
        side: p.side,
        left: p.left,
        end: p.end,
        stats: p.stats,
        killed_by: p.killed_by.and_then(row_of),
    }).collect();
    // The last one standing, as they were then, before anyone else can take their slot
    let winner = match outcome {
        Outcome::Survived(slot) => sim.round.iter().position(|p| p.who.slot == slot as usize && !p.left).map(|i| i as u8),
        _ => None,
    };
    let played = sim.opened.map_or(0, |t| (sim.time - t).max(0.0).round().min(u16::MAX as f32) as u16);
    RoundResults { number, outcome, winner, played, rows }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NpcSnap {
    pub pos: V3,
    pub yaw: f32,
    pub hp: f32,
    pub state: NpcState,
    /// Aiming at a player
    pub aiming: bool,
    /// Whether it is walking and how, and its stance
    pub walking: bool,
    pub gait: crate::sim::characters::Gait,
    pub stance: Option<crate::mission::Stance>,
}

/// A grenade, as clients draw it. Only the server moves grenades and sets them off.
#[derive(Clone, Debug, PartialEq)]
pub struct ThrownSnap {
    pub owner: u8,
    pub pos: V3,
    pub spin: f32,
}

/// The state of the game at one tick
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub tick: u32,
    pub time: f32,
    /// The latest input of the receiver that the game has used
    pub ack_seq: u32,
    /// The latest of the receiver's lobby commands that the server has taken
    pub lobby_ack: u32,
    pub culling: bool,
    pub players: Vec<PlayerSnap>,
    pub npcs: Vec<NpcSnap>,
    pub thrown: Vec<ThrownSnap>,
    /// How open each door is, and which way it is going
    pub doors: Vec<(f32, bool)>,
    /// The panes that have broken, and when
    pub broken: Vec<(u16, f32)>,
    /// Where the rounds are, and the session's settings
    pub round: RoundState,
    pub session: Session,
    /// The last round's results, from its end until the lobby
    pub results: Option<RoundResults>,
    /// Events, numbered in the order they happened
    pub events: Vec<(u32, Event)>,
}

impl Snapshot {
    /// The game as it stands, for the player in `receiver`. The events are the caller's to add.
    pub fn capture(sim: &Sim, tick: u32, receiver: usize, ack_seq: u32) -> Snapshot {
        // Another player is sent without what only their own screen shows, and their trigger
        // only while it fires
        let players = sim.players.iter().enumerate().filter(|(_, p)| p.active).map(|(i, p)| (i, p, i == receiver)).map(|(i, p, mine)| PlayerSnap {
            slot: i as u8,
            joined: p.joined,
            pos: p.player.pos,
            vel_y: if mine { p.player.vel_y } else { 0.0 },
            on_ground: p.player.on_ground,
            crouched: p.player.crouched,
            look: p.look,
            controlled: p.controlled,
            side: p.side,
            ammo: if mine { p.ammo } else { 0 },
            reserve: if mine { p.reserve } else { 0 },
            reload_until: p.reload_until,
            flash_until: p.flash_until,
            gun_trigger: p.gun_trigger && (mine || p.ammo > 0),
            kills: p.kills,
            deaths: p.deaths,
            hp: p.hp,
            respawn_at: p.respawn_at,
            hit: p.hit.map(|h| (h.motion, h.start)),
            equipped_grenade: p.equipped_grenade,
            grenades_left: if mine { p.grenades_left } else { 0 },
            charge_from: if mine { p.charge_from } else { None },
            throw: p.throw,
            message: (mine && p.message.1 > sim.time).then(|| p.message.clone()),
            target_hp: if mine { p.target_hp } else { None },
            peek: p.peek,
            aim: mine.then_some(p.aim),
            ready: false,
            name: String::new(),
            continued: false,
        }).collect();
        Snapshot {
            tick,
            time: sim.time,
            ack_seq,
            lobby_ack: 0,
            culling: sim.culling,
            players,
            npcs: sim.npcs.iter().map(|n| NpcSnap { pos: n.pos, yaw: n.yaw, hp: n.health.hp, state: n.health.state, aiming: n.aiming, walking: n.walking, gait: n.gait, stance: n.stance }).collect(),
            thrown: sim.thrown.iter().map(|t| ThrownSnap { owner: t.owner as u8, pos: t.grenade.pos, spin: t.grenade.spin }).collect(),
            doors: sim.door_states.iter().map(|s| (s.open, s.opening)).collect(),
            broken: sim.broken_at.iter().enumerate().filter_map(|(i, t)| t.map(|t| (i as u16, t))).collect(),
            round: RoundState::default(),
            session: Session::default(),
            results: None,
            events: Vec::new(),
        }
    }

    /// Make a copy of the game (a `Sim` built from the same setup) match this snapshot. The
    /// player in `me` is not moved: the client predicts where they are.
    pub fn apply(&self, sim: &mut Sim, me: usize) {
        sim.time = self.time;
        sim.culling = self.culling;
        sim.frozen = self.round.phase.frozen();
        // The host's settings, for the next round
        sim.friendly_fire = self.session.friendly_fire;
        if let Some(pair) = sim.base_pairs.get(self.session.base_pair as usize) {
            if sim.bases != *pair {
                sim.bases = pair.clone();
            }
        }

        for p in sim.players.iter_mut() {
            p.active = false;
        }
        for s in &self.players {
            let slot = s.slot as usize;
            sim.activate_slot(slot, s.pos);
            let p = &mut sim.players[slot];
            // The player's own look too: their inputs not yet used turn them on from it, and
            // held, it is where they stay
            p.look = s.look;
            if slot != me {
                p.player.pos = s.pos;
                p.player.on_ground = s.on_ground;
                p.player.crouched = s.crouched;
            } else {
                p.ammo = s.ammo;
                p.reserve = s.reserve;
                p.grenades_left = s.grenades_left;
                p.charge_from = s.charge_from;
            }
            p.controlled = s.controlled;
            p.joined = s.joined;
            p.side = s.side;
            p.reload_until = s.reload_until;
            p.flash_until = s.flash_until;
            p.gun_trigger = s.gun_trigger;
            p.kills = s.kills;
            p.deaths = s.deaths;
            p.hp = s.hp;
            p.respawn_at = s.respawn_at;
            p.hit = s.hit.map(|(motion, start)| Struck { motion, start });
            p.equipped_grenade = s.equipped_grenade;
            p.throw = s.throw;
            if slot != me {
                p.peek = s.peek;
            }
            if slot == me {
                if let Some(m) = &s.message {
                    p.message = m.clone();
                }
                p.target_hp = s.target_hp;
            }
        }

        for (npc, s) in sim.npcs.iter_mut().zip(&self.npcs) {
            // Its feet stay as far below its middle as they were placed, up and down stairs
            let above = npc.pos[1] - npc.floor;
            npc.pos = s.pos;
            npc.floor = s.pos[1] - above;
            npc.walking = s.walking;
            npc.gait = s.gait;
            npc.stance = s.stance;
            npc.yaw = s.yaw;
            npc.health.hp = s.hp;
            npc.health.state = s.state;
            // Taking aim, it raises its weapon from here
            if s.aiming && !npc.aiming {
                npc.aim_since = sim.time;
            }
            npc.aiming = s.aiming;
        }

        // Only the server sets grenades off, so who threw them is only drawn here
        sim.thrown = self.thrown.iter().map(|t| crate::sim::Thrown {
            grenade: Grenade { pos: t.pos, vel: [0.0; 3], fuse: crate::grenade::FUSE, spin: t.spin, at_rest: false },
            owner: t.owner as usize,
            by: sim.occupant(t.owner as usize),
            side: sim.players.get(t.owner as usize).and_then(|p| p.side),
        }).collect();

        for (i, &(open, opening)) in self.doors.iter().enumerate().take(sim.door_states.len()) {
            let state = &mut sim.door_states[i];
            if state.open != open {
                sim.world.set_door_open(i, open);
            }
            state.open = open;
            state.opening = opening;
            if let Some(portal) = sim.doors[i].portal {
                sim.blocked_portals[portal] = state.closed();
            }
        }

        // A new round puts the panes back
        for i in 0..sim.broken_at.len() {
            if sim.broken_at[i].is_some() && !self.broken.iter().any(|b| b.0 as usize == i) {
                sim.broken_at[i] = None;
                let name = sim.panes[i].name.clone();
                sim.world.switch_on(&name);
            }
        }
        for &(i, at) in &self.broken {
            let i = i as usize;
            if i < sim.broken_at.len() && sim.broken_at[i].is_none() {
                sim.broken_at[i] = Some(at);
                let name = sim.panes[i].name.clone();
                sim.world.switch_off(&name);
            }
        }
    }
}

// ------------------------------------------------------------------ writing

fn surface_byte(s: Surface) -> u8 {
    match s {
        Surface::Concrete => 0,
        Surface::Wood => 1,
        Surface::Metal => 2,
        Surface::Earth => 3,
        Surface::Glass => 4,
        Surface::Soft => 5,
        Surface::Scorch => 6,
    }
}

fn surface_from(b: u8) -> Result<Surface> {
    Ok(match b {
        0 => Surface::Concrete,
        1 => Surface::Wood,
        2 => Surface::Metal,
        3 => Surface::Earth,
        4 => Surface::Glass,
        5 => Surface::Soft,
        6 => Surface::Scorch,
        _ => bail!("no such surface as {b}"),
    })
}

fn motion_byte(name: &str) -> u8 {
    REACTION_MOTIONS.iter().position(|m| *m == name).unwrap_or(0) as u8
}

fn motion_from(b: u8) -> Result<&'static str> {
    REACTION_MOTIONS.get(b as usize).copied().ok_or_else(|| anyhow::anyhow!("no such motion as {b}"))
}

fn write_event(w: &mut Writer, e: &Event) -> bool {
    match e {
        // The server's own log is not for the players
        Event::Log(_) => return false,
        Event::Sound { name, at, gain } => {
            w.u8(0);
            w.string(name);
            w.opt_v3(*at);
            w.f32(*gain);
        }
        Event::Shot { player, start, end, hit_character } => {
            w.u8(1);
            w.u8(*player as u8);
            w.v3(*start);
            w.v3(*end);
            w.bool(*hit_character);
        }
        Event::CharacterShot { npc, start, end } => {
            w.u8(7);
            w.u16(*npc as u16);
            w.v3(*start);
            w.v3(*end);
        }
        Event::Chat { player, name, text, dead } => {
            w.u8(8);
            w.u8(*player as u8);
            w.string(name);
            w.string(text);
            w.bool(*dead);
        }
        Event::BulletHit { point, normal, dir, surface, material, dist, seed } => {
            w.u8(2);
            w.v3(*point);
            w.v3(*normal);
            w.v3(*dir);
            w.u8(surface_byte(*surface));
            w.string(material);
            w.f32(*dist);
            w.u64(*seed);
        }
        Event::Glass { pane, at, dir, seed } => {
            w.u8(3);
            w.u16(*pane as u16);
            w.v3(*at);
            w.v3(*dir);
            w.u64(*seed);
        }
        Event::Blast { point, normal, surface, on_surface, seed } => {
            w.u8(4);
            w.v3(*point);
            w.v3(*normal);
            w.u8(surface_byte(*surface));
            w.bool(*on_surface);
            w.u64(*seed);
        }
        Event::PlayerBlast { player, shake, veil } => {
            w.u8(5);
            w.u8(*player as u8);
            w.f32(*shake);
            w.f32(*veil);
        }
        Event::Kill { killer, victim, cause, killer_joined, victim_joined, killer_name, victim_name } => {
            w.u8(6);
            w.u8(killer.map_or(255, |k| k as u8));
            w.u8(*victim as u8);
            w.u8(match cause { Cause::Shot => 0, Cause::HeadShot => 1, Cause::Grenade => 2, Cause::Terrorist => 3 });
            w.varint(*killer_joined);
            w.varint(*victim_joined);
            w.string(killer_name);
            w.string(victim_name);
        }
    }
    true
}

fn read_event(r: &mut Reader) -> Result<Event> {
    Ok(match r.u8()? {
        0 => Event::Sound { name: r.string()?, at: r.opt_v3()?, gain: r.f32()? },
        1 => Event::Shot { player: r.u8()? as usize, start: r.v3()?, end: r.v3()?, hit_character: r.bool()? },
        2 => Event::BulletHit {
            point: r.v3()?, normal: r.v3()?, dir: r.v3()?, surface: surface_from(r.u8()?)?,
            material: r.string()?, dist: r.f32()?, seed: r.u64()?,
        },
        3 => Event::Glass { pane: r.u16()? as usize, at: r.v3()?, dir: r.v3()?, seed: r.u64()? },
        4 => Event::Blast { point: r.v3()?, normal: r.v3()?, surface: surface_from(r.u8()?)?, on_surface: r.bool()?, seed: r.u64()? },
        5 => Event::PlayerBlast { player: r.u8()? as usize, shake: r.f32()?, veil: r.f32()? },
        6 => Event::Kill {
            killer: Some(r.u8()?).filter(|&k| k != 255).map(|k| k as usize),
            victim: r.u8()? as usize,
            cause: match r.u8()? { 0 => Cause::Shot, 1 => Cause::HeadShot, 2 => Cause::Grenade, 3 => Cause::Terrorist, c => bail!("no such cause of death as {c}") },
            killer_joined: r.varint()?,
            victim_joined: r.varint()?,
            killer_name: r.string()?,
            victim_name: r.string()?,
        },
        7 => Event::CharacterShot { npc: r.u16()? as usize, start: r.v3()?, end: r.v3()? },
        8 => Event::Chat { player: r.u8()? as usize, name: r.string()?, text: r.string()?, dead: r.bool()? },
        t => bail!("no such event as {t}"),
    })
}

fn write_opt_f32(w: &mut Writer, v: Option<f32>) {
    w.opt_f32(v);
}

fn write_throw(w: &mut Writer, t: Option<Throw>) {
    match t {
        None => w.bool(false),
        Some(t) => {
            w.bool(true);
            w.f32(t.start);
            w.f32(t.charge);
            w.bool(t.thrown);
        }
    }
}

fn read_throw(r: &mut Reader) -> Result<Option<Throw>> {
    Ok(if r.bool()? { Some(Throw { start: r.f32()?, charge: r.f32()?, thrown: r.bool()? }) } else { None })
}

/// The widest a spread is sent as, in radians: 15 degrees, as wide as one can be
const AIM_WIDEST: f32 = 0.2617994;

/// The receiver's spread, as their screen shows it, in 7 bytes: the angles as fractions of
/// `AIM_WIDEST`, how long it has closed in hundredths of a second, and the growth above 1 in
/// hundredths. Their shots are the server's, so it need not be exact; what it closes to is
/// worked out again each tick, and which way they face is the player's look, so neither is sent.
fn write_aim(w: &mut Writer, a: &Aim) {
    w.u8((a.pace as u8) | (a.crouched as u8) << 3 | (a.started as u8) << 4);
    for v in [a.angle, a.from] {
        w.u16((v / AIM_WIDEST * 65535.0).round().clamp(0.0, 65535.0) as u16);
    }
    w.u8((a.since * 100.0).round().clamp(0.0, 255.0) as u8);
    w.u8(((a.growth - 1.0) * 100.0).round().clamp(0.0, 255.0) as u8);
}

fn read_aim(r: &mut Reader) -> Result<Aim> {
    let b = r.u8()?;
    ensure!(b < 32, "no such aim as {b:#x}");
    let pace = match b & 7 {
        0 => Pace::Stopped,
        1 => Pace::Shuffle,
        2 => Pace::Walk,
        3 => Pace::WalkFast,
        4 => Pace::Run,
        p => bail!("no such pace as {p}"),
    };
    let mut angle = || -> Result<f32> { Ok(r.u16()? as f32 / 65535.0 * AIM_WIDEST) };
    let (angle, from) = (angle()?, angle()?);
    let (since, growth) = (r.u8()? as f32 / 100.0, r.u8()? as f32 / 100.0 + 1.0);
    Ok(Aim { angle, from, to: 0.0, since, growth, pace, crouched: b & 8 != 0, started: b & 16 != 0, look: 0 })
}

fn write_opt_aim(w: &mut Writer, a: Option<Aim>) {
    w.bool(a.is_some());
    if let Some(a) = a {
        write_aim(w, &a);
    }
}

fn read_opt_aim(r: &mut Reader) -> Result<Option<Aim>> {
    Ok(if r.bool()? { Some(read_aim(r)?) } else { None })
}

fn write_peek(w: &mut Writer, peek: Option<Peek>) {
    match peek {
        None => w.u8(0),
        Some(p) => {
            w.u8(1 | (p.right as u8) << 1 | (p.crouched as u8) << 2 | (p.returning as u8) << 3);
            // A peek moves on by half frames (30 a second, ticks of 1/60), so this is exact
            w.u8((p.frame * 2.0).round() as u8);
        }
    }
}

fn read_peek(r: &mut Reader) -> Result<Option<Peek>> {
    let b = r.u8()?;
    ensure!(b < 16, "no such peek as {b:#x}");
    Ok(if b & 1 == 0 { None } else { Some(Peek { right: b & 2 != 0, crouched: b & 4 != 0, returning: b & 8 != 0, frame: r.u8()? as f32 / 2.0 }) })
}

/// The yes-or-no parts of a player, and their side in the top two bits
fn flag_byte(p: &PlayerSnap) -> u8 {
    [p.on_ground, p.controlled, p.gun_trigger, p.equipped_grenade, p.crouched, p.aim.is_some()].iter().enumerate().fold(0, |acc, (n, &b)| acc | (b as u8) << n)
        | side_code(p.side) << 6
}

fn write_message(w: &mut Writer, m: &Option<(String, f32)>) {
    w.bool(m.is_some());
    if let Some((text, until)) = m {
        w.string(text);
        w.f32(*until);
    }
}

fn read_message(r: &mut Reader) -> Result<Option<(String, f32)>> {
    Ok(if r.bool()? { Some((r.string()?, r.f32()?)) } else { None })
}

fn write_hit(w: &mut Writer, hit: Option<(&'static str, f32)>) {
    match hit {
        None => w.u8(u8::MAX),
        Some((motion, start)) => { w.u8(motion_byte(motion)); w.f32(start); }
    }
}

fn read_hit(r: &mut Reader) -> Result<Option<(&'static str, f32)>> {
    Ok(match r.u8()? {
        u8::MAX => None,
        b => Some((motion_from(b)?, r.f32()?)),
    })
}

/// A player in full, after their slot
/// A player in full. Only a player with an aim, the receiver's own, carries what is on their
/// screen: their fall speed, ammo, grenades, throw charge and messages.
fn write_player(w: &mut Writer, p: &PlayerSnap) {
    w.u8(flag_byte(p));
    let detailed = p.aim.is_some();
    w.v3(p.pos);
    w.u16(p.look.0);
    w.u16(p.look.1);
    write_hit(w, p.hit);
    write_opt_f32(w, p.reload_until);
    w.f32(p.flash_until);
    w.u16(p.kills as u16);
    write_throw(w, p.throw);
    w.u16(p.deaths as u16);
    w.f32(p.hp);
    write_opt_f32(w, p.respawn_at);
    write_peek(w, p.peek);
    w.varint(p.joined);
    w.bool(p.ready);
    w.string(&p.name);
    w.bool(p.continued);
    if detailed {
        w.f32(p.vel_y);
        w.u16(p.ammo as u16);
        w.u16(p.reserve as u16);
        w.u8(p.grenades_left as u8);
        write_opt_f32(w, p.charge_from);
        write_message(w, &p.message);
        write_opt_f32(w, p.target_hp);
    }
    if let Some(a) = &p.aim {
        write_aim(w, a);
    }
}

fn read_player(r: &mut Reader, slot: u8) -> Result<PlayerSnap> {
    let flags = r.u8()?;
    let on = |n: u8| flags >> n & 1 != 0;
    let pos = r.v3()?;
    let look = (r.u16()?, r.u16()?);
    let hit = read_hit(r)?;
    let reload_until = r.opt_f32()?;
    let flash_until = r.f32()?;
    let kills = r.u16()? as u32;
    let throw = read_throw(r)?;
    let (deaths, hp, respawn_at) = (r.u16()? as u32, r.f32()?, r.opt_f32()?);
    let peek = read_peek(r)?;
    let joined = r.varint()?;
    let ready = r.bool()?;
    let (name, continued) = (r.string()?, r.bool()?);
    let (mut vel_y, mut ammo, mut reserve, mut grenades_left, mut charge_from, mut message, mut target_hp) = (0.0, 0, 0, 0, None, None, None);
    if on(5) {
        (vel_y, ammo, reserve, grenades_left) = (r.f32()?, r.u16()? as u32, r.u16()? as u32, r.u8()? as u32);
        (charge_from, message, target_hp) = (r.opt_f32()?, read_message(r)?, r.opt_f32()?);
    }
    let aim = if on(5) { Some(read_aim(r)?) } else { None };
    Ok(PlayerSnap {
        slot, joined, side: side_from(flags >> 6)?, pos, vel_y, on_ground: on(0), crouched: on(4), look, controlled: on(1), ammo, reserve, reload_until,
        flash_until, gun_trigger: on(2), kills, deaths, hp, respawn_at, hit, equipped_grenade: on(3), grenades_left, charge_from,
        throw, message, target_hp, peek, aim, ready, name, continued,
    })
}

/// Returns the float's bit representation for exact comparisons.
fn bits(v: f32) -> u32 {
    v.to_bits()
}

fn opt_bits(v: Option<f32>) -> Option<u32> {
    v.map(f32::to_bits)
}

/// Mask value indicating a full player record instead of a delta.
const FULL: u32 = 1 << 31;

/// A player as it differs from `base`: a mask of the parts that changed, then those parts
fn write_player_delta(w: &mut Writer, p: &PlayerSnap, base: &PlayerSnap) {
    let changed = [
        p.pos.map(bits) != base.pos.map(bits),
        bits(p.vel_y) != bits(base.vel_y),
        p.look != base.look,
        p.hit != base.hit,
        p.ammo as u16 != base.ammo as u16,
        p.reserve as u16 != base.reserve as u16,
        opt_bits(p.reload_until) != opt_bits(base.reload_until),
        bits(p.flash_until) != bits(base.flash_until),
        p.kills as u16 != base.kills as u16,
        p.grenades_left as u8 != base.grenades_left as u8,
        opt_bits(p.charge_from) != opt_bits(base.charge_from),
        p.throw != base.throw,
        flag_byte(p) != flag_byte(base),
        p.message != base.message,
        opt_bits(p.target_hp) != opt_bits(base.target_hp),
        p.deaths as u16 != base.deaths as u16,
        bits(p.hp) != bits(base.hp),
        opt_bits(p.respawn_at) != opt_bits(base.respawn_at),
        p.peek.map(|k| (k.right, k.crouched, k.returning, bits(k.frame))) != base.peek.map(|k| (k.right, k.crouched, k.returning, bits(k.frame))),
        p.aim.map(|a| (a.pace, a.crouched, a.started, [a.angle, a.from, a.to, a.since, a.growth].map(bits))) != base.aim.map(|a| (a.pace, a.crouched, a.started, [a.angle, a.from, a.to, a.since, a.growth].map(bits))),
        p.joined != base.joined,
        p.ready != base.ready,
        p.name != base.name,
        p.continued != base.continued,
    ];
    w.varint(changed.iter().enumerate().fold(0, |acc, (n, &c)| acc | (c as u32) << n));
    if changed[0] { w.v3(p.pos); }
    if changed[1] { w.f32(p.vel_y); }
    if changed[2] { w.u16(p.look.0); w.u16(p.look.1); }
    if changed[3] { write_hit(w, p.hit); }
    if changed[4] { w.u16(p.ammo as u16); }
    if changed[5] { w.u16(p.reserve as u16); }
    if changed[6] { write_opt_f32(w, p.reload_until); }
    if changed[7] { w.f32(p.flash_until); }
    if changed[8] { w.u16(p.kills as u16); }
    if changed[9] { w.u8(p.grenades_left as u8); }
    if changed[10] { write_opt_f32(w, p.charge_from); }
    if changed[11] { write_throw(w, p.throw); }
    if changed[12] { w.u8(flag_byte(p)); }
    if changed[13] { write_message(w, &p.message); }
    if changed[14] { write_opt_f32(w, p.target_hp); }
    if changed[15] { w.u16(p.deaths as u16); }
    if changed[16] { w.f32(p.hp); }
    if changed[17] { write_opt_f32(w, p.respawn_at); }
    if changed[18] { write_peek(w, p.peek); }
    if changed[19] { write_opt_aim(w, p.aim); }
    if changed[20] { w.varint(p.joined); }
    if changed[21] { w.bool(p.ready); }
    if changed[22] { w.string(&p.name); }
    if changed[23] { w.bool(p.continued); }
}

/// The player `base` was, changed as the mask in the message says
fn read_player_delta(r: &mut Reader, mask: u32, base: &PlayerSnap) -> Result<PlayerSnap> {
    ensure!(mask < 1 << 24, "no such parts of a player as {mask:#x}");
    let on = |n: u32| mask >> n & 1 != 0;
    let mut p = base.clone();
    if on(0) { p.pos = r.v3()?; }
    if on(1) { p.vel_y = r.f32()?; }
    if on(2) { p.look = (r.u16()?, r.u16()?); }
    if on(3) { p.hit = read_hit(r)?; }
    if on(4) { p.ammo = r.u16()? as u32; }
    if on(5) { p.reserve = r.u16()? as u32; }
    if on(6) { p.reload_until = r.opt_f32()?; }
    if on(7) { p.flash_until = r.f32()?; }
    if on(8) { p.kills = r.u16()? as u32; }
    if on(9) { p.grenades_left = r.u8()? as u32; }
    if on(10) { p.charge_from = r.opt_f32()?; }
    if on(11) { p.throw = read_throw(r)?; }
    if on(12) {
        let flags = r.u8()?;
        let f = |n: u8| flags >> n & 1 != 0;
        (p.on_ground, p.controlled, p.gun_trigger, p.equipped_grenade, p.crouched) = (f(0), f(1), f(2), f(3), f(4));
        p.side = side_from(flags >> 6)?;
    }
    if on(13) { p.message = read_message(r)?; }
    if on(14) { p.target_hp = r.opt_f32()?; }
    if on(15) { p.deaths = r.u16()? as u32; }
    if on(16) { p.hp = r.f32()?; }
    if on(17) { p.respawn_at = r.opt_f32()?; }
    if on(18) { p.peek = read_peek(r)?; }
    if on(19) { p.aim = read_opt_aim(r)?; }
    if on(20) { p.joined = r.varint()?; }
    if on(21) { p.ready = r.bool()?; }
    if on(22) { p.name = r.string()?; }
    if on(23) { p.continued = r.bool()?; }
    Ok(p)
}

/// One bit for each of `flags`, eight to a byte
fn write_bits(w: &mut Writer, flags: &[bool]) {
    for chunk in flags.chunks(8) {
        w.u8(chunk.iter().enumerate().fold(0, |acc, (n, &b)| acc | (b as u8) << n));
    }
}

fn read_bits(r: &mut Reader, n: usize) -> Result<Vec<bool>> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n.div_ceil(8) {
        let byte = r.u8()?;
        out.extend((0..8).map(|i| byte >> i & 1 != 0));
    }
    out.truncate(n);
    Ok(out)
}

/// A list where the things that are the same as in `base` are only marked as such. The reader
/// has to know how many there are, and has to have the same `base`.
fn write_changed<T: PartialEq>(w: &mut Writer, now: &[T], base: Option<&[T]>, mut one: impl FnMut(&mut Writer, &T)) {
    let base = base.filter(|b| b.len() == now.len());
    let changed: Vec<bool> = now.iter().enumerate().map(|(i, t)| base.is_none_or(|b| b[i] != *t)).collect();
    if base.is_some() {
        write_bits(w, &changed);
    }
    for (t, _) in now.iter().zip(&changed).filter(|(_, c)| **c) {
        one(w, t);
    }
}

fn read_changed<T: Clone>(r: &mut Reader, n: usize, base: Option<&[T]>, mut one: impl FnMut(&mut Reader) -> Result<T>) -> Result<Vec<T>> {
    let base = base.filter(|b| b.len() == n);
    let changed = if base.is_some() { read_bits(r, n)? } else { vec![true; n] };
    (0..n).map(|i| match base {
        Some(b) if !changed[i] => Ok(b[i].clone()),
        _ => one(r),
    }).collect()
}

fn write_npc(w: &mut Writer, n: &NpcSnap) {
    w.v3(n.pos);
    w.f32(n.yaw);
    // Health is shown in whole points, rounded up so the living are never shown as dead
    w.u8(n.hp.ceil().clamp(0.0, 255.0) as u8);
    // The state's top bit is aiming, the next walking, and the four below them the stance
    let stance = n.stance.map_or(0, |s| 1 + crate::mission::Stance::ALL.iter().position(|&t| t == s).unwrap_or(0) as u8);
    let aiming = if n.aiming { 0x80 } else { 0 } | if n.walking { 0x40 } else { 0 } | stance << 2;
    match n.state {
        NpcState::Idle => w.u8(aiming),
        NpcState::Reacting { motion, start } => { w.u8(1 | aiming); w.u8(motion_byte(motion)); w.f32(start); }
        NpcState::Dead { motion, start } => { w.u8(2 | aiming); w.u8(motion_byte(motion)); w.f32(start); }
    }
    // How it walks, while it does
    if n.walking {
        w.u8(n.gait as u8);
    }
}

fn read_npc(r: &mut Reader) -> Result<NpcSnap> {
    let (pos, yaw, hp) = (r.v3()?, r.f32()?, r.u8()? as f32);
    let byte = r.u8()?;
    let (aiming, walking) = (byte & 0x80 != 0, byte & 0x40 != 0);
    let stance = match (byte >> 2) & 0xf {
        0 => None,
        s => crate::mission::Stance::ALL.get(s as usize - 1).copied(),
    };
    let state = match byte & 0x3 {
        0 => NpcState::Idle,
        1 => NpcState::Reacting { motion: motion_from(r.u8()?)?, start: r.f32()? },
        2 => NpcState::Dead { motion: motion_from(r.u8()?)?, start: r.f32()? },
        s => bail!("no such state as {s}"),
    };
    use crate::sim::characters::Gait;
    let gait = match (walking, if walking { r.u8()? } else { 1 }) {
        (_, 0) => Gait::Creep,
        (_, 1) => Gait::Walk,
        (_, 2) => Gait::Run,
        (_, g) => bail!("no such gait as {g}"),
    };
    Ok(NpcSnap { pos, yaw, hp, state, aiming, walking, gait, stance })
}

fn write_thrown(w: &mut Writer, t: &ThrownSnap) {
    w.u8(t.owner);
    w.v3(t.pos);
    w.f32(t.spin);
}

fn read_thrown(r: &mut Reader) -> Result<ThrownSnap> {
    Ok(ThrownSnap { owner: r.u8()?, pos: r.v3()?, spin: r.f32()? })
}

/// Everything in a snapshot but its events, as it differs from `base` if there is one
fn write_state(w: &mut Writer, s: &Snapshot, base: Option<&Snapshot>) {
    w.u32(s.tick);
    w.f32(s.time);
    w.u32(s.ack_seq);
    w.varint(s.lobby_ack);
    w.bool(s.culling);
    w.u8(s.players.len() as u8);
    for p in &s.players {
        w.u8(p.slot);
        match base.and_then(|b| b.players.iter().find(|q| q.slot == p.slot)) {
            Some(q) => write_player_delta(w, p, q),
            None => {
                w.varint(FULL);
                write_player(w, p);
            }
        }
    }
    w.u16(s.npcs.len() as u16);
    write_changed(w, &s.npcs, base.map(|b| &b.npcs[..]), write_npc);
    w.u8(s.thrown.len() as u8);
    write_changed(w, &s.thrown, base.map(|b| &b.thrown[..]), write_thrown);
    w.u16(s.doors.len() as u16);
    write_changed(w, &s.doors, base.map(|b| &b.doors[..]), |w, &(open, opening)| {
        w.f32(open);
        w.bool(opening);
    });
    // Panes only ever break within a round, and a base is always of the same round, so only
    // the ones the base does not know of are sent
    let new: Vec<&(u16, f32)> = s.broken.iter().filter(|b| base.is_none_or(|base| !base.broken.contains(b))).collect();
    w.u16(new.len() as u16);
    for &&(i, at) in &new {
        w.u16(i);
        w.f32(at);
    }
    write_round(w, &s.round);
    // The session only when it has changed
    match base.filter(|b| b.session == s.session) {
        Some(_) => w.bool(false),
        None => {
            w.bool(true);
            write_session(w, &s.session);
        }
    }
    // The results: 0 as before (none, with no snapshot before), 1 new ones, 2 none now
    match (base.map(|b| &b.results), &s.results) {
        (Some(b), now) if b == now => w.u8(0),
        (None, None) => w.u8(0),
        (_, Some(results)) => {
            w.u8(1);
            write_results(w, results);
        }
        (_, None) => w.u8(2),
    }
}

pub fn write_results(w: &mut Writer, r: &RoundResults) {
    w.u16(r.number);
    w.u8(outcome_code(Some(r.outcome)).0);
    w.u8(r.winner.unwrap_or(u8::MAX));
    w.u16(r.played);
    w.u8(r.rows.len() as u8);
    for row in &r.rows {
        w.u8(row.slot);
        w.varint(row.joined);
        w.string(&row.name);
        w.u8(side_code(row.side) | (row.left as u8) << 2 | ((row.end == Ending::Killed) as u8) << 3);
        for v in [row.stats.kills, row.stats.fired, row.stats.hits, row.stats.taken] {
            w.varint(v);
        }
        w.u8(row.killed_by.unwrap_or(u8::MAX));
    }
}

pub fn read_results(r: &mut Reader) -> Result<RoundResults> {
    let (number, code, winner, played, n) = (r.u16()?, r.u8()?, r.u8()?, r.u16()?, r.u8()? as usize);
    ensure!(n <= MAX_PLAYERS, "too many players in the results");
    let mut rows = Vec::with_capacity(n);
    for _ in 0..n {
        let (slot, joined, name, flags) = (r.u8()?, r.varint()?, r.string()?, r.u8()?);
        ensure!(flags < 1 << 4, "no such result flags as {flags:#x}");
        let stats = RoundStats { kills: r.varint()?, fired: r.varint()?, hits: r.varint()?, taken: r.varint()? };
        let killed_by = Some(r.u8()?).filter(|&k| k != u8::MAX);
        let end = if flags >> 3 & 1 != 0 { Ending::Killed } else { Ending::Alive };
        rows.push(ResultRow { slot, joined, name, side: side_from(flags & 3)?, left: flags >> 2 & 1 != 0, end, stats, killed_by });
    }
    // Every row named must be one of them
    ensure!(rows.iter().all(|row| row.killed_by.is_none_or(|k| (k as usize) < n)), "a killer who is not in the results");
    let winner = Some(winner).filter(|&w| w != u8::MAX);
    ensure!(winner.is_none_or(|w| (w as usize) < n), "a winner who is not in the results");
    let Some(outcome) = outcome_from(code, winner.map_or(0, |w| rows[w as usize].slot))? else { bail!("results with no outcome") };
    Ok(RoundResults { number, outcome, winner, played, rows })
}

pub fn write_session(w: &mut Writer, s: &Session) {
    w.varint(s.generation);
    w.u8(s.host.unwrap_or(u8::MAX));
    // No round is 0 seconds long, so 0 is no limit
    w.u16(s.time_limit.unwrap_or(0));
    w.bool(s.friendly_fire);
    w.u8(s.base_pair);
    w.bool(s.auto_countdown);
}

pub fn read_session(r: &mut Reader) -> Result<Session> {
    let (generation, host, time_limit) = (r.varint()?, r.u8()?, r.u16()?);
    Ok(Session { generation, host: (host != u8::MAX).then_some(host), time_limit: (time_limit != 0).then_some(time_limit), friendly_fire: r.bool()?, base_pair: r.u8()?, auto_countdown: r.bool()? })
}

fn outcome_code(o: Option<Outcome>) -> (u8, u8) {
    match o {
        None => (0, 0),
        Some(Outcome::Won(s)) => (1 + s.index() as u8, 0),
        Some(Outcome::Draw) => (3, 0),
        Some(Outcome::CalledOff) => (4, 0),
        Some(Outcome::Survived(p)) => (5, p),
    }
}

fn outcome_from(code: u8, slot: u8) -> Result<Option<Outcome>> {
    Ok(match code {
        0 => None,
        1 => Some(Outcome::Won(Side::Blue)),
        2 => Some(Outcome::Won(Side::Gold)),
        3 => Some(Outcome::Draw),
        4 => Some(Outcome::CalledOff),
        5 => Some(Outcome::Survived(slot)),
        c => bail!("no such outcome as {c}"),
    })
}

fn write_round(w: &mut Writer, r: &RoundState) {
    w.u8(r.phase.code());
    if r.phase == Phase::Open {
        return;
    }
    w.u16(r.number);
    w.opt_f32(r.ends);
    w.u16(r.wins[0]);
    w.u16(r.wins[1]);
    let (code, slot) = outcome_code(r.last);
    w.u8(code);
    w.u8(slot);
}

fn read_round(r: &mut Reader) -> Result<RoundState> {
    let Some(phase) = Phase::from_code(r.u8()?) else { bail!("no such phase") };
    if phase == Phase::Open {
        return Ok(RoundState::default());
    }
    let (number, ends, wins) = (r.u16()?, r.opt_f32()?, [r.u16()?, r.u16()?]);
    let last = outcome_from(r.u8()?, r.u8()?)?;
    Ok(RoundState { phase, number, ends, wins, last })
}

fn read_state(r: &mut Reader, base: Option<&Snapshot>) -> Result<Snapshot> {
    let (tick, time, ack_seq, lobby_ack, culling) = (r.u32()?, r.f32()?, r.u32()?, r.varint()?, r.bool()?);
    let n = r.u8()? as usize;
    ensure!(n <= MAX_PLAYERS, "too many players");
    let mut players = Vec::with_capacity(n);
    for _ in 0..n {
        let slot = r.u8()?;
        let mask = r.varint()?;
        players.push(if mask == FULL {
            read_player(r, slot)?
        } else {
            let Some(q) = base.and_then(|b| b.players.iter().find(|q| q.slot == slot)) else { bail!("a change to player {slot}, who was not there") };
            read_player_delta(r, mask, q)?
        });
    }
    let n = r.u16()? as usize;
    let npcs = read_changed(r, n, base.map(|b| &b.npcs[..]), read_npc)?;
    let n = r.u8()? as usize;
    let thrown = read_changed(r, n, base.map(|b| &b.thrown[..]), read_thrown)?;
    let n = r.u16()? as usize;
    let doors = read_changed(r, n, base.map(|b| &b.doors[..]), |r| Ok((r.f32()?, r.bool()?)))?;
    let mut broken = base.map_or(Vec::new(), |b| b.broken.clone());
    for _ in 0..r.u16()? {
        let (i, at) = (r.u16()?, r.f32()?);
        match broken.iter_mut().find(|b| b.0 == i) {
            Some(b) => b.1 = at,
            None => broken.push((i, at)),
        }
    }
    broken.sort_by_key(|b| b.0);
    let round = read_round(r)?;
    let session = match (r.bool()?, base) {
        (true, _) => read_session(r)?,
        (false, Some(b)) => b.session,
        (false, None) => bail!("a session unchanged from no snapshot"),
    };
    let results = match r.u8()? {
        0 => base.and_then(|b| b.results.clone()),
        1 => Some(read_results(r)?),
        2 => None,
        t => bail!("no such results tag as {t}"),
    };
    Ok(Snapshot { tick, time, ack_seq, lobby_ack, culling, players, npcs, thrown, doors, broken, round, session, results, events: Vec::new() })
}

pub fn write_setup(w: &mut Writer, s: &Setup) {
    w.string(&s.map);
    w.string(&s.model);
    w.string(&s.idle);
    w.u32(s.magazine);
    w.u32(s.ammo);
    w.u32(s.reserve);
    w.u32(s.grenades);
    w.f32(s.speed_scale);
    w.u64(s.seed);
    w.bool(s.culling);
    w.bool(s.pvp);
    w.bool(s.friendly_fire);
    w.bool(s.respawn);
    for base in &s.bases {
        w.u8(base.len() as u8);
        for &p in base {
            w.v3(p);
        }
    }
    for u in &s.uniforms {
        w.string(u);
    }
    w.u8(s.base_pairs.len() as u8);
    for pair in &s.base_pairs {
        w.string(&pair.name);
        for base in &pair.bases {
            w.u8(base.len() as u8);
            for &p in base {
                w.v3(p);
            }
        }
    }
    w.opt_f32(s.doors_open);
    w.string(s.mission.as_deref().unwrap_or(""));
    w.u16(s.npcs.len() as u16);
    for n in &s.npcs {
        w.v3(n.pos);
        w.f32(n.floor);
        w.f32(n.yaw);
        w.v3(n.scale);
        w.f32(n.stagger);
        w.u16(n.actor.map_or(u16::MAX, |a| a));
    }
}

pub fn read_setup(r: &mut Reader) -> Result<Setup> {
    let (map, model, idle) = (r.string()?, r.string()?, r.string()?);
    let (magazine, ammo, reserve, grenades) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
    let speed_scale = r.f32()?;
    let seed = r.u64()?;
    let culling = r.bool()?;
    let (pvp, friendly_fire, respawn) = (r.bool()?, r.bool()?, r.bool()?);
    let base = |r: &mut Reader| (0..r.u8()?).map(|_| r.v3()).collect::<Result<Vec<_>>>();
    let bases = [base(r)?, base(r)?];
    let uniforms = [r.string()?, r.string()?];
    let base_pairs = (0..r.u8()?).map(|_| Ok(BasePairSetup { name: r.string()?, bases: [base(r)?, base(r)?] })).collect::<Result<Vec<_>>>()?;
    let doors_open = r.opt_f32()?;
    let mission = Some(r.string()?).filter(|m| !m.is_empty());
    let n = r.u16()? as usize;
    let mut npcs = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        let (pos, floor, yaw, scale, stagger) = (r.v3()?, r.f32()?, r.f32()?, r.v3()?, r.f32()?);
        let actor = r.u16()?;
        npcs.push(NpcSetup { pos, floor, yaw, scale, stagger, actor: (actor != u16::MAX).then_some(actor) });
    }
    Ok(Setup { map, model, idle, magazine, ammo, reserve, grenades, speed_scale, seed, culling, pvp, friendly_fire, respawn, bases, uniforms, base_pairs, doors_open, mission, npcs })
}

/// What each side wears unless the server says otherwise: "Blue Medium" and "Desert 1 Medium",
/// as ladder play had it
pub const UNIFORMS: [&str; 2] = ["data/character/team_hrt/blue_M_a.CRP", "data/character/team_camo/desert1_M_a.CRP"];

/// A side as a number: 0 for none, then Blue and Gold
pub fn side_code(side: Option<Side>) -> u8 {
    side.map_or(0, |s| 1 + s.index() as u8)
}

pub fn side_from(code: u8) -> Result<Option<Side>> {
    match code {
        0 => Ok(None),
        1 => Ok(Some(Side::Blue)),
        2 => Ok(Some(Side::Gold)),
        c => bail!("no such side as {c}"),
    }
}

fn header(w: &mut Writer, kind: u8) {
    w.u16(MAGIC);
    w.u8(PROTOCOL);
    w.u8(kind);
}

fn check_header(r: &mut Reader) -> Result<u8> {
    ensure!(r.u16()? == MAGIC, "not one of ours");
    let protocol = r.u8()?;
    ensure!(protocol == PROTOCOL, "protocol {protocol}, not {PROTOCOL}");
    r.u8()
}

impl ToServer {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            ToServer::Connect { token, build, side, key, name } => {
                header(&mut w, 0);
                w.u32(*token);
                w.string(build);
                w.u8(side_code(*side));
                w.u64(*key);
                w.string(name);
            }
            ToServer::Input { token, ack_snapshot, first_seq, view, round, inputs } => {
                header(&mut w, 1);
                w.u32(*token);
                w.u32(*ack_snapshot);
                w.u32(*first_seq);
                w.u32(*view);
                w.u16(*round);
                w.u8(inputs.len() as u8);
                let mut before = Before::default();
                for i in inputs {
                    w.input(i, &mut before);
                }
            }
            ToServer::Leave { token } => {
                header(&mut w, 2);
                w.u32(*token);
            }
            ToServer::Lobby { token, seq, generation, command } => {
                header(&mut w, 3);
                w.u32(*token);
                w.u32(*seq);
                w.u32(*generation);
                match command {
                    LobbyCommand::Side(s) => { w.u8(0); w.u8(side_code(Some(*s))); }
                    LobbyCommand::Ready(on) => { w.u8(1); w.bool(*on); }
                    LobbyCommand::Start => w.u8(2),
                    LobbyCommand::Stop => w.u8(3),
                    LobbyCommand::TimeLimit(t) => { w.u8(4); w.u16(t.unwrap_or(0)); }
                    LobbyCommand::FriendlyFire(on) => { w.u8(5); w.bool(*on); }
                    LobbyCommand::BasePair(i) => { w.u8(6); w.u8(*i); }
                    LobbyCommand::Say(text) => { w.u8(7); w.string(text); }
                    LobbyCommand::Continue => w.u8(8),
                    LobbyCommand::ToLobby => w.u8(9),
                }
            }
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Result<ToServer> {
        let mut r = Reader::new(bytes);
        let kind = check_header(&mut r)?;
        let msg = match kind {
            0 => ToServer::Connect { token: r.u32()?, build: r.string()?, side: side_from(r.u8()?)?, key: r.u64()?, name: r.string()? },
            1 => {
                let (token, ack_snapshot, first_seq, view, round) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u16()?);
                let n = r.u8()? as usize;
                ensure!(n <= 32, "too many inputs");
                let mut before = Before::default();
                let mut inputs = Vec::with_capacity(n);
                for _ in 0..n {
                    inputs.push(r.input(&mut before)?);
                }
                ToServer::Input { token, ack_snapshot, first_seq, view, round, inputs }
            }
            2 => ToServer::Leave { token: r.u32()? },
            3 => {
                let (token, seq, generation) = (r.u32()?, r.u32()?, r.u32()?);
                let command = match r.u8()? {
                    0 => LobbyCommand::Side(side_from(r.u8()?)?.ok_or_else(|| anyhow::anyhow!("no side to change to"))?),
                    1 => LobbyCommand::Ready(r.bool()?),
                    2 => LobbyCommand::Start,
                    3 => LobbyCommand::Stop,
                    4 => LobbyCommand::TimeLimit(Some(r.u16()?).filter(|&t| t != 0)),
                    5 => LobbyCommand::FriendlyFire(r.bool()?),
                    6 => LobbyCommand::BasePair(r.u8()?),
                    7 => LobbyCommand::Say(r.string()?),
                    8 => LobbyCommand::Continue,
                    9 => LobbyCommand::ToLobby,
                    c => bail!("no such lobby command as {c}"),
                };
                ToServer::Lobby { token, seq, generation, command }
            }
            k => bail!("no such message as {k}"),
        };
        ensure!(r.left() == 0, "{} bytes left over", r.left());
        Ok(msg)
    }
}

impl ToClient {
    /// The message as bytes. A snapshot leaves out the events that do not fit in a datagram, or
    /// for one too big for a datagram, in the last of its parts; `sent_events` says how many
    /// were put in.
    pub fn encode(&self) -> (Vec<u8>, usize) {
        self.encode_against(None)
    }

    /// The same, but a snapshot is sent as what has changed since `base`, a snapshot that the
    /// receiver is known to have. Only a receiver that has `base` can make sense of it.
    pub fn encode_against(&self, base: Option<&Snapshot>) -> (Vec<u8>, usize) {
        let mut w = Writer::new();
        let mut sent = 0;
        match self {
            ToClient::Accept { token, slot, tick, setup } => {
                header(&mut w, 0);
                w.u32(*token);
                w.u8(*slot);
                w.u32(*tick);
                write_setup(&mut w, setup);
            }
            ToClient::Refuse { token, reason } => {
                header(&mut w, 1);
                w.u32(*token);
                w.string(reason);
            }
            ToClient::Snapshot(s) => {
                header(&mut w, 2);
                // Which snapshot the rest is the difference from: one more than its tick, or 0 for none
                w.u32(base.map_or(0, |b| b.tick + 1));
                write_state(&mut w, s, base);
                // Events go in as long as they fit
                let room = if w.0.len() < MAX_PACKET { MAX_PACKET } else { (w.0.len() + 1).div_ceil(PART) * PART };
                let mut body = Writer::new();
                for (id, e) in &s.events {
                    let mut one = Writer::new();
                    if !write_event(&mut one, e) {
                        // Not sent, but counted as handled so that it is not tried again
                        sent += 1;
                        continue;
                    }
                    if w.0.len() + 1 + body.0.len() + 4 + one.0.len() > room {
                        break;
                    }
                    body.u32(*id);
                    body.0.extend_from_slice(&one.0);
                    sent += 1;
                }
                let kept = s.events.iter().take(sent).filter(|(_, e)| !matches!(e, Event::Log(_))).count();
                w.u8(kept as u8);
                w.0.extend_from_slice(&body.0);
            }
            ToClient::Goodbye { token, reason } => {
                header(&mut w, 3);
                w.u32(*token);
                w.string(reason);
            }
            ToClient::Part { tick, index, count, bytes } => {
                header(&mut w, 4);
                w.u32(*tick);
                w.u8(*index);
                w.u8(*count);
                w.0.extend_from_slice(bytes);
            }
        }
        (w.0, sent)
    }

    pub fn decode(bytes: &[u8]) -> Result<ToClient> {
        Self::decode_with(bytes, &|_| None)
    }

    /// Decode, with `earlier` to find the snapshots that a snapshot may be the difference from
    pub fn decode_with<'a>(bytes: &[u8], earlier: &dyn Fn(u32) -> Option<&'a Snapshot>) -> Result<ToClient> {
        let mut r = Reader::new(bytes);
        let kind = check_header(&mut r)?;
        let msg = match kind {
            0 => ToClient::Accept { token: r.u32()?, slot: r.u8()?, tick: r.u32()?, setup: read_setup(&mut r)? },
            1 => ToClient::Refuse { token: r.u32()?, reason: r.string()? },
            2 => {
                let base = match r.u32()? {
                    0 => None,
                    t => Some(earlier(t - 1).ok_or_else(|| anyhow::anyhow!("a difference from snapshot {}, which is not here", t - 1))?),
                };
                let mut snap = read_state(&mut r, base)?;
                let n = r.u8()? as usize;
                let events = (0..n).map(|_| Ok((r.u32()?, read_event(&mut r)?))).collect::<Result<Vec<_>>>()?;
                snap.events = events;
                ToClient::Snapshot(snap)
            }
            3 => ToClient::Goodbye { token: r.u32()?, reason: r.string()? },
            4 => {
                let (tick, index, count) = (r.u32()?, r.u8()?, r.u8()?);
                ToClient::Part { tick, index, count, bytes: r.rest().to_vec() }
            }
            k => bail!("no such message as {k}"),
        };
        ensure!(r.left() == 0, "{} bytes left over", r.left());
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Setup {
        Setup {
            map: "data/map/rm11/rm11.map".to_string(), model: "data/character/team_hrt/blue_M_a.CRP".to_string(),
            idle: "up_none_cyc".to_string(), magazine: 30, ammo: 30, reserve: 900, grenades: 3, speed_scale: 1.0,
            seed: 0x5eed, culling: true, pvp: true, friendly_fire: false, respawn: false,
            bases: [vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]], vec![[7.0, 8.0, 9.0]]], uniforms: UNIFORMS.map(String::from), base_pairs: vec![
                BasePairSetup { name: "ZONE_CSL_GROUP_1_DESC".to_string(), bases: [vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]], vec![[7.0, 8.0, 9.0]]] },
                BasePairSetup { name: "ZONE_CSL_GROUP_2_DESC".to_string(), bases: [vec![[-1.0, 2.0, 3.0]], vec![[7.0, -8.0, 9.0], [0.5; 3]]] },
            ], doors_open: Some(0.0), mission: Some("data/mission/RM11Med.mdf".to_string()),
            npcs: (0..8).map(|i| NpcSetup {
                pos: [i as f32 * 110.0, 0.0, 12.0], floor: 0.0, yaw: 90.0 + i as f32, scale: [1.0, 1.0, 1.0],
                stagger: i as f32 * 0.73, actor: (i % 3 != 0).then_some(i as u16),
            }).collect(),
        }
    }

    fn player(slot: u8) -> PlayerSnap {
        PlayerSnap {
            slot, joined: 300 + slot as u32, side: Some(Side::Gold), pos: [1.0, 2.0, 3.0], vel_y: -4.0, on_ground: true, crouched: true, look: (123, 65000), controlled: true,
            ammo: 12, reserve: 400, reload_until: Some(3.5), flash_until: 1.25, gun_trigger: true, kills: 3, deaths: 1, hp: 62.5,
            respawn_at: Some(20.0),
            hit: Some(("thiu_la", 14.25)),
            equipped_grenade: true, grenades_left: 2, charge_from: None,
            throw: Some(Throw { start: 1.0, charge: 0.5, thrown: false }),
            message: Some(("KILLED".to_string(), 9.0)), target_hp: Some(33.0),
            peek: Some(Peek { right: true, crouched: false, frame: 12.5, returning: true }),
            aim: Some(Aim { angle: 0.0, from: AIM_WIDEST, to: 0.0, since: 0.25, growth: 1.25, pace: Pace::Run, crouched: true, started: true, look: 0 }),
            ready: true,
            name: format!("[RAINBOW] Jöhn {slot}"),
            continued: true,
        }
    }

    /// A round's results with a row who left and a killer, up to `rows` long
    fn results(rows: usize) -> RoundResults {
        RoundResults {
            number: 7, outcome: Outcome::Won(Side::Gold), winner: None, played: 116,
            rows: (0..rows).map(|i| ResultRow {
                slot: i as u8, joined: 1000 + i as u32, name: format!("Pie4Foo of Pie's Tác{i:03}"), side: Some(if i % 2 == 0 { Side::Blue } else { Side::Gold }),
                left: i == 2, end: if i % 3 == 0 { Ending::Killed } else { Ending::Alive },
                stats: RoundStats { kills: i as u32, fired: 1000 + i as u32, hits: 300, taken: 7 }, killed_by: (i % 3 == 0).then_some(((i + 1) % rows) as u8),
            }).collect(),
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            tick: 900, time: 15.0, ack_seq: 777, lobby_ack: 12, culling: true,
            players: vec![player(0), PlayerSnap { message: None, target_hp: None, ..player(3) }],
            npcs: vec![
                NpcSnap { pos: [1.0, 0.0, 2.0], yaw: 30.0, hp: 100.0, state: NpcState::Idle, aiming: true, walking: true, gait: crate::sim::characters::Gait::Run, stance: Some(crate::mission::Stance::Stand) },
                NpcSnap { pos: [2.0, 0.0, 2.0], yaw: 60.0, hp: 40.0, state: NpcState::Reacting { motion: "thwu_bf", start: 14.5 }, aiming: true, walking: false, gait: crate::sim::characters::Gait::Walk, stance: Some(crate::mission::Stance::SitInChair) },
                NpcSnap { pos: [3.0, 0.0, 2.0], yaw: 90.0, hp: 0.0, state: NpcState::Dead { motion: "hw_bk_up2", start: 12.0 }, aiming: false, walking: false, gait: crate::sim::characters::Gait::Walk, stance: None },
            ],
            thrown: vec![ThrownSnap { owner: 1, pos: [5.0; 3], spin: 40.0 }],
            doors: vec![(0.0, false), (0.5, true), (1.0, true)],
            broken: vec![(3, 10.5), (200, 11.0)],
            round: RoundState { phase: Phase::Results, number: 7, ends: Some(20.0), wins: [3, 2], last: Some(Outcome::Won(Side::Gold)) },
            session: Session { generation: 9, host: Some(3), time_limit: Some(240), friendly_fire: true, base_pair: 2, auto_countdown: true },
            results: Some(results(4)),
            events: vec![
                (10, Event::Sound { name: "C_GRFRAG.WAV".to_string(), at: Some([1.0, 2.0, 3.0]), gain: 1.0 }),
                (11, Event::Shot { player: 2, start: [0.0; 3], end: [9.0, 8.0, 7.0], hit_character: true }),
                (12, Event::BulletHit { point: [1.0; 3], normal: [0.0, 1.0, 0.0], dir: [0.0, -1.0, 0.0], surface: Surface::Metal, material: "x.bmp".to_string(), dist: 55.0, seed: 99 }),
                (13, Event::Glass { pane: 4, at: [1.0; 3], dir: [0.0, 0.0, 1.0], seed: 5 }),
                (14, Event::Blast { point: [1.0; 3], normal: [0.0, 1.0, 0.0], surface: Surface::Glass, on_surface: false, seed: 6 }),
                (15, Event::PlayerBlast { player: 1, shake: 0.5, veil: 0.25 }),
                (16, Event::Kill { killer: Some(0), victim: 1, cause: Cause::HeadShot, killer_joined: 5, victim_joined: 9, killer_name: "7ony".to_string(), victim_name: "Raithat".to_string() }),
                (17, Event::Kill { killer: None, victim: 1, cause: Cause::Grenade, killer_joined: 0, victim_joined: 9, killer_name: String::new(), victim_name: "Raithat".to_string() }),
                (18, Event::Kill { killer: None, victim: 0, cause: Cause::Terrorist, killer_joined: 0, victim_joined: 4, killer_name: String::new(), victim_name: "Villico".to_string() }),
                (19, Event::CharacterShot { npc: 300, start: [1.0; 3], end: [2.0; 3] }),
                (20, Event::Chat { player: 5, name: "BobbyBoyGaming".to_string(), text: "on my way".to_string(), dead: true }),
            ],
        }
    }

    #[test]
    fn every_message_survives_the_trip() {
        let messages = [
            ToServer::Connect { token: 42, build: "abc1234".to_string(), side: Some(Side::Gold), key: 0x0123_4567_89ab_cdef, name: "[RAINBOW] JOHN DOE".to_string() },
            ToServer::Input { token: 42, ack_snapshot: 897, first_seq: 5000, view: 14400, round: 3, inputs: (0..5).map(|i| {
                let mut x = Input { forward: i % 2 == 0, fire: i > 2, run: true, ..Input::default() };
                x.set_look(i as f32 * 10.0, -5.0);
                x
            }).collect() },
            ToServer::Leave { token: 42 },
        ];
        let commands = [
            LobbyCommand::Side(Side::Blue), LobbyCommand::Side(Side::Gold), LobbyCommand::Ready(true), LobbyCommand::Ready(false), LobbyCommand::Start, LobbyCommand::Stop,
            LobbyCommand::TimeLimit(Some(300)), LobbyCommand::TimeLimit(None), LobbyCommand::FriendlyFire(false), LobbyCommand::BasePair(4),
            LobbyCommand::Say("gg, één more".to_string()),
        ];
        let lobby = commands.into_iter().enumerate().map(|(i, command)| ToServer::Lobby { token: 42, seq: i as u32 + 1, generation: u32::MAX - i as u32, command });
        for m in messages.into_iter().chain(lobby) {
            assert_eq!(ToServer::decode(&m.encode()).unwrap(), m);
        }
        let mut snap = snapshot();
        let (bytes, sent) = ToClient::Snapshot(snap.clone()).encode();
        assert_eq!(sent, 11);
        assert_eq!(ToClient::decode(&bytes).unwrap(), ToClient::Snapshot(snap.clone()));
        // The server's own log is not sent, and does not count against what was not
        snap.events.insert(1, (99, Event::Log("t=1.00 door opening".to_string())));
        let (bytes, sent) = ToClient::Snapshot(snap.clone()).encode();
        assert_eq!(sent, 12);
        snap.events.remove(1);
        assert_eq!(ToClient::decode(&bytes).unwrap(), ToClient::Snapshot(snap));
        for m in [
            ToClient::Accept { token: 42, slot: 2, tick: 1234, setup: setup() },
            ToClient::Refuse { token: 42, reason: "the game is full".to_string() },
            ToClient::Goodbye { token: 42, reason: "the host left".to_string() },
        ] {
            let (bytes, _) = m.encode();
            assert_eq!(ToClient::decode(&bytes).unwrap(), m);
        }
    }

    #[test]
    fn a_snapshot_with_too_many_events_sends_what_fits_and_says_how_many() {
        let mut snap = snapshot();
        snap.events = (0..200).map(|i| (i, Event::Sound { name: "English/re_wnd1.wav".to_string(), at: Some([1.0; 3]), gain: 1.0 })).collect();
        let (bytes, sent) = ToClient::Snapshot(snap).encode();
        assert!(bytes.len() <= MAX_PACKET);
        assert!(sent > 10 && sent < 200, "{sent} events sent");
        let ToClient::Snapshot(back) = ToClient::decode(&bytes).unwrap() else { panic!() };
        assert_eq!(back.events.len(), sent);
        assert_eq!(back.events.last().unwrap().0 as usize, sent - 1);
    }

    #[test]
    fn nonsense_is_refused_not_trusted() {
        assert!(ToServer::decode(b"").is_err());
        assert!(ToServer::decode(b"GET / HTTP/1.1\r\n").is_err());
        assert!(ToClient::decode(&[0x52, 0x52, PROTOCOL + 1, 0]).is_err());
        let (good, _) = ToClient::Snapshot(snapshot()).encode();
        for cut in [3, 10, good.len() / 2, good.len() - 1] {
            assert!(ToClient::decode(&good[..cut]).is_err(), "cut at {cut}");
        }
        let mut longer = good.clone();
        longer.push(7);
        assert!(ToClient::decode(&longer).is_err());
        // Whatever the bytes, decoding never panics
        let mut junk = good;
        for i in 0..junk.len() {
            let saved = junk[i];
            junk[i] = junk[i].wrapping_add(0x55);
            let _ = ToClient::decode(&junk);
            junk[i] = saved;
        }
    }

    /// Another player as the receiver is sent them: without what only their own screen shows
    fn other(slot: u8) -> PlayerSnap {
        PlayerSnap { vel_y: 0.0, ammo: 0, reserve: 0, grenades_left: 0, charge_from: None, message: None, target_hp: None, aim: None, ..player(slot) }
    }

    /// A snapshot of a full game: every player, and `npcs` characters
    fn crowded(npcs: usize) -> Snapshot {
        let mut snap = snapshot();
        snap.results = None;
        snap.players = (0..MAX_PLAYERS as u8).map(|i| if i == 0 { player(0) } else { other(i) }).collect();
        snap.npcs = (0..npcs).map(|i| NpcSnap { pos: [i as f32; 3], yaw: 1.0, hp: 1.0, state: NpcState::Reacting { motion: "thwu_bb", start: 1.0 }, aiming: true, walking: true, gait: crate::sim::characters::Gait::Creep, stance: Some(crate::mission::Stance::Slouch) }).collect();
        snap
    }

    #[test]
    fn a_full_game_on_the_move_fits_in_a_datagram() {
        let mut base = crowded(16);
        base.events.clear();
        // Everyone moves, looks round and fires, and every character moves
        let mut next = base.clone();
        next.tick += 2;
        for p in next.players.iter_mut() {
            p.pos[0] += 3.0;
            p.look.0 += 50;
            p.gun_trigger = !p.gun_trigger;
            p.flash_until += 0.1;
        }
        for n in next.npcs.iter_mut() {
            n.pos[2] += 2.0;
            n.yaw += 4.0;
        }
        let (bytes, _) = ToClient::Snapshot(next).encode_against(Some(&base));
        // With room left for a busy tick's events
        assert!(bytes.len() < MAX_PACKET - 300, "{} bytes before any events", bytes.len());
    }

    #[test]
    fn a_snapshot_too_big_for_a_datagram_comes_through_in_parts() {
        // A full game in full, with a hunt's worth of terrorists and a level of doors
        let mut snap = crowded(40);
        snap.doors = (0..150).map(|i| (i as f32 / 150.0, i % 2 == 0)).collect();
        let (bytes, sent) = ToClient::Snapshot(snap.clone()).encode();
        assert_eq!(sent, snap.events.len(), "the events fit in the last part");
        let datagrams = split(snap.tick, bytes.clone());
        assert!(datagrams.len() > 1);
        assert!(datagrams.iter().all(|d| d.len() <= MAX_PACKET));

        let part = |d: &Vec<u8>| match ToClient::decode(d).unwrap() {
            ToClient::Part { tick, index, count, bytes } => (tick, index, count, bytes),
            m => panic!("{m:?}"),
        };
        // Out of order, and one twice
        let mut parts = Parts::default();
        let mut whole = None;
        for d in datagrams.iter().rev().chain(&datagrams[..1]) {
            let (tick, index, count, bytes) = part(d);
            if let Some(w) = parts.add(tick, index, count, bytes) {
                assert!(whole.is_none(), "put together twice");
                whole = Some(w);
            }
        }
        assert_eq!(whole.as_ref(), Some(&bytes));
        assert_eq!(ToClient::decode(&bytes).unwrap(), ToClient::Snapshot(snap.clone()));

        // One lost, and the rest are never a snapshot
        let mut parts = Parts::default();
        for d in &datagrams[1..] {
            let (tick, index, count, bytes) = part(d);
            assert!(parts.add(tick, index, count, bytes).is_none());
        }
    }

    #[test]
    fn a_full_table_of_results_comes_through_with_a_full_game() {
        let mut snap = crowded(40);
        let mut table = results(MAX_PLAYERS);
        // Every name as long as it may be, in letters that take two bytes each
        for row in &mut table.rows {
            row.name = "Ñ".repeat(NAME_LENGTH);
        }
        snap.results = Some(table);
        let (bytes, _) = ToClient::Snapshot(snap.clone()).encode();
        let datagrams = split(snap.tick, bytes.clone());
        assert!(datagrams.iter().all(|d| d.len() <= MAX_PACKET));
        let mut parts = Parts::default();
        let whole = datagrams.iter().find_map(|d| match ToClient::decode(d).unwrap() {
            ToClient::Part { tick, index, count, bytes } => parts.add(tick, index, count, bytes),
            m => panic!("{m:?}"),
        });
        let ToClient::Snapshot(got) = ToClient::decode(&whole.unwrap()).unwrap() else { panic!() };
        assert_eq!(got.results, snap.results);
    }

    #[test]
    fn results_are_sent_once_cleared_when_gone_and_checked_as_read() {
        let mut base = snapshot();
        let mut next = base.clone();
        next.tick += 2;
        // Unchanged: nothing of them is sent, and they are as they were
        let (same, _) = delta_trip(&base, &next);
        assert_eq!(same.results, base.results);
        let size = |s: &Snapshot, b: &Snapshot| ToClient::Snapshot(s.clone()).encode_against(Some(b)).0.len();
        let without = { let mut n = next.clone(); n.results = None; n };
        assert!(size(&next, &base) < size(&next, &without), "a table that has not changed is sent again");
        // Gone: cleared, from a base and from none
        let (cleared, _) = delta_trip(&base, &without);
        assert_eq!(cleared.results, None);
        let (bytes, _) = ToClient::Snapshot(without.clone()).encode();
        let ToClient::Snapshot(full) = ToClient::decode(&bytes).unwrap() else { panic!() };
        assert_eq!(full.results, None);
        // A row that names a killer or winner beyond the table is refused
        base.results = None;
        for bad in [
            RoundResults { rows: vec![ResultRow { killed_by: Some(4), ..results(1).rows[0].clone() }], ..results(1) },
            RoundResults { outcome: Outcome::Survived(0), winner: Some(1), ..results(1) },
        ] {
            let mut w = Writer::new();
            write_results(&mut w, &bad);
            assert!(read_results(&mut Reader::new(&w.0)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_name_is_cleaned_cut_and_made_unique() {
        assert_eq!(player_name("  [RAINBOW] John Doe\u{7}  ", 0, &[]), "[RAINBOW] John Doe");
        assert_eq!(player_name("", 2, &[]), "PLAYER 3");
        assert_eq!(player_name("\u{1}\u{2603}", 2, &[]), "PLAYER 3");
        assert_eq!(player_name(&"x".repeat(40), 0, &[]).len(), NAME_LENGTH);
        // The same ignoring case, accents included, gets a number
        assert_eq!(player_name("bot", 1, &["BOT"]), "bot 2");
        assert_eq!(player_name("BOT", 1, &["BOT", "Bot 2"]), "BOT 3");
        assert_eq!(player_name("ÉLAN", 1, &["élan"]), "ÉLAN 2");
        // Cut to make room for it, and checked again
        let long = "y".repeat(NAME_LENGTH);
        let second = player_name(&long, 1, &[&long]);
        assert_eq!(second, format!("{} 2", "y".repeat(NAME_LENGTH - 2)));
        assert_eq!(player_name(&long, 1, &[&long, &second]), format!("{} 3", "y".repeat(NAME_LENGTH - 2)));
        // A made-up name is no different
        assert_eq!(player_name("", 2, &["player 3"]), "PLAYER 3 2");
    }

    fn delta_trip(base: &Snapshot, next: &Snapshot) -> (Snapshot, usize) {
        let (bytes, _) = ToClient::Snapshot(next.clone()).encode_against(Some(base));
        let ToClient::Snapshot(back) = ToClient::decode_with(&bytes, &|t| (t == base.tick).then_some(base)).unwrap() else { panic!() };
        (back, bytes.len())
    }

    #[test]
    fn a_snapshot_sent_as_a_difference_comes_out_the_same() {
        let mut base = snapshot();
        base.events.clear();
        // Nothing happened, and it is almost nothing
        let mut next = base.clone();
        next.tick += 2;
        next.time += 0.033;
        let (back, small) = delta_trip(&base, &next);
        assert_eq!(back, next);
        let (full, _) = ToClient::Snapshot(next.clone()).encode();
        assert!(small * 3 < full.len(), "{small} bytes against {} in full", full.len());

        // Every kind of change at once
        next.players[0].pos = [9.0, 8.0, 7.0];
        next.players[0].look.0 = 7;
        next.players[0].ammo = 11;
        next.players[0].reload_until = None;
        next.players[0].throw = None;
        next.players[0].message = None;
        next.players[0].on_ground = false;
        next.players[1].target_hp = Some(5.0);
        next.players[1].message = Some(("HELLO".to_string(), 3.0));
        next.players[1].charge_from = Some(2.0);
        next.players[1].hp = 0.0;
        next.players[1].respawn_at = None;
        next.players[0].deaths = 2;
        next.players[0].hit = None;
        next.players[1].hit = Some(("thwu_ll", 14.9));
        // Someone new in the slot
        next.players[0].joined = 9001;
        next.npcs[1].state = NpcState::Dead { motion: "hw_bk_up2", start: 15.0 };
        next.npcs[1].hp = 0.0;
        next.thrown.push(ThrownSnap { owner: 0, pos: [1.0; 3], spin: 0.0 });
        next.doors[2] = (0.25, false);
        next.broken.push((50, 16.0));
        next.broken.sort_by_key(|b| b.0);
        let (back, _) = delta_trip(&base, &next);
        assert_eq!(back, next);

        // Players come and go
        next.players.remove(0);
        next.players.push(player(5));
        let (back, _) = delta_trip(&base, &next);
        assert_eq!(back, next);
        let (back, _) = delta_trip(&base, &base);
        assert_eq!(back, base);
    }

    #[test]
    fn a_difference_from_a_snapshot_the_receiver_lacks_is_refused() {
        let base = snapshot();
        let mut next = base.clone();
        next.tick += 2;
        let (bytes, _) = ToClient::Snapshot(next).encode_against(Some(&base));
        assert!(ToClient::decode(&bytes).is_err());
        let mut junk = bytes;
        for i in 0..junk.len() {
            let saved = junk[i];
            junk[i] = junk[i].wrapping_add(0x55);
            let _ = ToClient::decode_with(&junk, &|_| Some(&base));
            junk[i] = saved;
        }
    }

    #[test]
    fn each_uniform_fits_the_skeleton_and_follows_the_body_bullets_hit() {
        let read = |p: &str| crate::crp::read(std::path::Path::new(p)).ok();
        let Some(body) = read("data/character/team_hrt/blue_M_a.CRP") else { return };
        let skeleton = crate::skeleton::read(std::path::Path::new("data/character/team_hrt/blue_M_a.SKL")).unwrap();
        // The body's corners and the middles of its triangles
        let v = &body.vertices;
        let surface: Vec<[f32; 3]> = v.iter().copied().chain(body.parts.iter().flat_map(|p| &p.triangles).map(|t| {
            let [a, b, c] = t.map(|i| v[i as usize]);
            [0, 1, 2].map(|k| (a[k] + b[k] + c[k]) / 3.0)
        })).collect();
        for path in UNIFORMS {
            let uniform = read(path).unwrap();
            assert!(uniform.skin(&skeleton).is_some(), "{path}");
            let mut off: Vec<f32> = uniform.vertices.iter().map(|p| {
                surface.iter().map(|q| (0..3).map(|k| (p[k] - q[k]).powi(2)).sum::<f32>().sqrt()).fold(f32::MAX, f32::min)
            }).collect();
            off.sort_by(f32::total_cmp);
            let (median, p90) = (off[off.len() / 2], off[off.len() * 9 / 10]);
            // About 2 cm and 6 cm for Desert 1 Medium
            assert!(median < 4.0 && p90 < 8.0, "{path}: median {median}, p90 {p90}");
        }
    }
}
