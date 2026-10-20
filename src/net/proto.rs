//! What the server and clients say to each other.
//!
//! A client sends what its player does (`Input`, a few of the latest every time, so a lost
//! datagram costs nothing) and which snapshot it has seen last. The server sends snapshots: the
//! state of everything a client draws, with the events since the client last confirmed one.
//! Datagrams may be lost, repeated or reordered, and every message is meant to survive that.

use anyhow::{Result, bail, ensure};

use crate::bytes::{Before, Reader, Writer};
use crate::combat::NpcState;
use crate::grenade::Grenade;
use crate::impact::Surface;
use crate::net::link::MAX_PACKET;
use crate::sim::characters::REACTION_MOTIONS;
use crate::sim::{Cause, Event, Input, MAX_PLAYERS, Sim, Throw};

type V3 = [f32; 3];

/// Bumped whenever the messages change, so that a client and server that disagree notice
pub const PROTOCOL: u8 = 4;

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
    /// Whether players can hurt each other
    pub pvp: bool,
    pub doors_open: Option<f32>,
    pub npcs: Vec<NpcSetup>,
}

/// What a client sends
#[derive(Clone, Debug, PartialEq)]
pub enum ToServer {
    /// Ask to join. `token` is picked by the client and goes on everything it sends, so that a
    /// stranger cannot speak for it. `build` says which version of the game it is.
    Connect { token: u32, build: String },
    /// What the player did. `inputs` are those numbered `first_seq` and on, newest last.
    /// `ack_snapshot` is the tick of the latest snapshot seen. `view` is the server tick, in
    /// sixteenths, that the newest input's player was looking at the others as of, or 0 if
    /// that is not known: a shot is judged against where the others were then.
    Input { token: u32, ack_snapshot: u32, first_seq: u32, view: u32, inputs: Vec<Input> },
    /// Leave
    Leave { token: u32 },
}

/// What the server sends
#[derive(Clone, Debug, PartialEq)]
pub enum ToClient {
    /// You are in: this is your slot and how the game is set up
    Accept { token: u32, slot: u8, tick: u32, setup: Setup },
    /// You are not: why
    Refuse { token: u32, reason: String },
    Snapshot(Snapshot),
    /// The server is closing the connection
    Goodbye { token: u32 },
}

/// A player as the others need to see them, and as they themselves need to know
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerSnap {
    pub slot: u8,
    pub pos: V3,
    pub vel_y: f32,
    pub on_ground: bool,
    pub crouched: bool,
    pub look: (u16, u16),
    pub controlled: bool,
    pub moved_speed: f32,
    pub ammo: u32,
    pub reserve: u32,
    pub reload_until: Option<f32>,
    pub flash_until: f32,
    pub gun_trigger: bool,
    pub kills: u32,
    pub deaths: u32,
    pub hp: f32,
    pub respawn_at: Option<f32>,
    pub equipped_grenade: bool,
    pub grenades_left: u32,
    pub charge_from: Option<f32>,
    pub throw: Option<Throw>,
    /// What the receiver sees on screen, only for their own player
    pub message: Option<(String, f32)>,
    pub target_hp: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NpcSnap {
    pub pos: V3,
    pub yaw: f32,
    pub hp: f32,
    pub state: NpcState,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThrownSnap {
    pub owner: u8,
    pub pos: V3,
    pub vel: V3,
    pub fuse: f32,
    pub spin: f32,
    pub at_rest: bool,
}

/// The state of the game at one tick
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub tick: u32,
    pub time: f32,
    /// The latest input of the receiver that the game has used
    pub ack_seq: u32,
    pub culling: bool,
    pub players: Vec<PlayerSnap>,
    pub npcs: Vec<NpcSnap>,
    pub thrown: Vec<ThrownSnap>,
    /// How open each door is, and which way it is going
    pub doors: Vec<(f32, bool)>,
    /// The panes that have broken, and when
    pub broken: Vec<(u16, f32)>,
    /// Events, numbered in the order they happened
    pub events: Vec<(u32, Event)>,
}

impl Snapshot {
    /// The game as it stands, for the player in `receiver`. The events are the caller's to add.
    pub fn capture(sim: &Sim, tick: u32, receiver: usize, ack_seq: u32) -> Snapshot {
        let players = sim.players.iter().enumerate().filter(|(_, p)| p.active).map(|(i, p)| PlayerSnap {
            slot: i as u8,
            pos: p.player.pos,
            vel_y: p.player.vel_y,
            on_ground: p.player.on_ground,
            crouched: p.player.crouched,
            look: p.look,
            controlled: p.controlled,
            moved_speed: p.moved_speed,
            ammo: p.ammo,
            reserve: p.reserve,
            reload_until: p.reload_until,
            flash_until: p.flash_until,
            gun_trigger: p.gun_trigger,
            kills: p.kills,
            deaths: p.deaths,
            hp: p.hp,
            respawn_at: p.respawn_at,
            equipped_grenade: p.equipped_grenade,
            grenades_left: p.grenades_left,
            charge_from: p.charge_from,
            throw: p.throw,
            message: (i == receiver && p.message.1 > sim.time).then(|| p.message.clone()),
            target_hp: if i == receiver { p.target_hp } else { None },
        }).collect();
        Snapshot {
            tick,
            time: sim.time,
            ack_seq,
            culling: sim.culling,
            players,
            npcs: sim.npcs.iter().map(|n| NpcSnap { pos: n.pos, yaw: n.yaw, hp: n.health.hp, state: n.health.state }).collect(),
            thrown: sim.thrown.iter().map(|t| ThrownSnap {
                owner: t.owner as u8,
                pos: t.grenade.pos,
                vel: t.grenade.vel,
                fuse: t.grenade.fuse,
                spin: t.grenade.spin,
                at_rest: t.grenade.at_rest,
            }).collect(),
            doors: sim.door_states.iter().map(|s| (s.open, s.opening)).collect(),
            broken: sim.broken_at.iter().enumerate().filter_map(|(i, t)| t.map(|t| (i as u16, t))).collect(),
            events: Vec::new(),
        }
    }

    /// Make a copy of the game (a `Sim` built from the same setup) match this snapshot. The
    /// player in `me` is not moved: the client predicts where they are.
    pub fn apply(&self, sim: &mut Sim, me: usize) {
        sim.time = self.time;
        sim.culling = self.culling;

        for p in sim.players.iter_mut() {
            p.active = false;
        }
        for s in &self.players {
            let slot = s.slot as usize;
            sim.activate_slot(slot, s.pos);
            let p = &mut sim.players[slot];
            if slot != me {
                p.player.pos = s.pos;
                p.player.vel_y = s.vel_y;
                p.player.on_ground = s.on_ground;
                p.player.crouched = s.crouched;
                p.moved_speed = s.moved_speed;
                p.look = s.look;
            }
            p.controlled = s.controlled;
            p.ammo = s.ammo;
            p.reserve = s.reserve;
            p.reload_until = s.reload_until;
            p.flash_until = s.flash_until;
            p.gun_trigger = s.gun_trigger;
            p.kills = s.kills;
            p.deaths = s.deaths;
            p.hp = s.hp;
            p.respawn_at = s.respawn_at;
            p.equipped_grenade = s.equipped_grenade;
            p.grenades_left = s.grenades_left;
            p.charge_from = s.charge_from;
            p.throw = s.throw;
            if slot == me {
                if let Some(m) = &s.message {
                    p.message = m.clone();
                }
                p.target_hp = s.target_hp;
            }
        }

        for (npc, s) in sim.npcs.iter_mut().zip(&self.npcs) {
            npc.pos[0] = s.pos[0];
            npc.pos[2] = s.pos[2];
            npc.yaw = s.yaw;
            npc.health.hp = s.hp;
            npc.health.state = s.state;
        }

        sim.thrown = self.thrown.iter().map(|t| crate::sim::Thrown {
            grenade: Grenade { pos: t.pos, vel: t.vel, fuse: t.fuse, spin: t.spin, at_rest: t.at_rest },
            owner: t.owner as usize,
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
        Event::Kill { killer, victim, cause } => {
            w.u8(6);
            w.u8(killer.map_or(255, |k| k as u8));
            w.u8(*victim as u8);
            w.u8(match cause { Cause::Shot => 0, Cause::HeadShot => 1, Cause::Grenade => 2 });
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
            cause: match r.u8()? { 0 => Cause::Shot, 1 => Cause::HeadShot, 2 => Cause::Grenade, c => bail!("no such cause of death as {c}") },
        },
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

fn flag_byte(p: &PlayerSnap) -> u8 {
    [p.on_ground, p.controlled, p.gun_trigger, p.equipped_grenade, p.crouched].iter().enumerate().fold(0, |acc, (n, &b)| acc | (b as u8) << n)
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

/// A player in full, after their slot
fn write_player(w: &mut Writer, p: &PlayerSnap) {
    w.v3(p.pos);
    w.f32(p.vel_y);
    w.u16(p.look.0);
    w.u16(p.look.1);
    w.f32(p.moved_speed);
    w.u16(p.ammo as u16);
    w.u16(p.reserve as u16);
    write_opt_f32(w, p.reload_until);
    w.f32(p.flash_until);
    w.u16(p.kills as u16);
    w.u8(p.grenades_left as u8);
    write_opt_f32(w, p.charge_from);
    write_throw(w, p.throw);
    w.u8(flag_byte(p));
    write_message(w, &p.message);
    write_opt_f32(w, p.target_hp);
    w.u16(p.deaths as u16);
    w.f32(p.hp);
    write_opt_f32(w, p.respawn_at);
}

fn read_player(r: &mut Reader, slot: u8) -> Result<PlayerSnap> {
    let pos = r.v3()?;
    let vel_y = r.f32()?;
    let look = (r.u16()?, r.u16()?);
    let moved_speed = r.f32()?;
    let ammo = r.u16()? as u32;
    let reserve = r.u16()? as u32;
    let reload_until = r.opt_f32()?;
    let flash_until = r.f32()?;
    let kills = r.u16()? as u32;
    let grenades_left = r.u8()? as u32;
    let charge_from = r.opt_f32()?;
    let throw = read_throw(r)?;
    let flags = r.u8()?;
    let on = |n: u8| flags >> n & 1 != 0;
    let message = read_message(r)?;
    let target_hp = r.opt_f32()?;
    let (deaths, hp, respawn_at) = (r.u16()? as u32, r.f32()?, r.opt_f32()?);
    Ok(PlayerSnap {
        slot, pos, vel_y, on_ground: on(0), crouched: on(4), look, controlled: on(1), moved_speed, ammo, reserve, reload_until,
        flash_until, gun_trigger: on(2), kills, deaths, hp, respawn_at, equipped_grenade: on(3), grenades_left, charge_from,
        throw, message, target_hp,
    })
}

/// A float as the bits that make it up, so that two are the same only if nothing about them differs
fn bits(v: f32) -> u32 {
    v.to_bits()
}

fn opt_bits(v: Option<f32>) -> Option<u32> {
    v.map(f32::to_bits)
}

/// A player's mask of what changed is this when they are sent in full instead
const FULL: u32 = 1 << 31;

/// A player as it differs from `base`: a mask of the parts that changed, then those parts
fn write_player_delta(w: &mut Writer, p: &PlayerSnap, base: &PlayerSnap) {
    let changed = [
        p.pos.map(bits) != base.pos.map(bits),
        bits(p.vel_y) != bits(base.vel_y),
        p.look != base.look,
        bits(p.moved_speed) != bits(base.moved_speed),
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
    ];
    w.varint(changed.iter().enumerate().fold(0, |acc, (n, &c)| acc | (c as u32) << n));
    if changed[0] { w.v3(p.pos); }
    if changed[1] { w.f32(p.vel_y); }
    if changed[2] { w.u16(p.look.0); w.u16(p.look.1); }
    if changed[3] { w.f32(p.moved_speed); }
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
}

/// The player `base` was, changed as the mask in the message says
fn read_player_delta(r: &mut Reader, mask: u32, base: &PlayerSnap) -> Result<PlayerSnap> {
    ensure!(mask < 1 << 18, "no such parts of a player as {mask:#x}");
    let on = |n: u32| mask >> n & 1 != 0;
    let mut p = base.clone();
    if on(0) { p.pos = r.v3()?; }
    if on(1) { p.vel_y = r.f32()?; }
    if on(2) { p.look = (r.u16()?, r.u16()?); }
    if on(3) { p.moved_speed = r.f32()?; }
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
    }
    if on(13) { p.message = read_message(r)?; }
    if on(14) { p.target_hp = r.opt_f32()?; }
    if on(15) { p.deaths = r.u16()? as u32; }
    if on(16) { p.hp = r.f32()?; }
    if on(17) { p.respawn_at = r.opt_f32()?; }
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
    w.f32(n.hp);
    match n.state {
        NpcState::Idle => w.u8(0),
        NpcState::Reacting { motion, start } => { w.u8(1); w.u8(motion_byte(motion)); w.f32(start); }
        NpcState::Dead { motion, start } => { w.u8(2); w.u8(motion_byte(motion)); w.f32(start); }
    }
}

fn read_npc(r: &mut Reader) -> Result<NpcSnap> {
    let (pos, yaw, hp) = (r.v3()?, r.f32()?, r.f32()?);
    let state = match r.u8()? {
        0 => NpcState::Idle,
        1 => NpcState::Reacting { motion: motion_from(r.u8()?)?, start: r.f32()? },
        2 => NpcState::Dead { motion: motion_from(r.u8()?)?, start: r.f32()? },
        s => bail!("no such state as {s}"),
    };
    Ok(NpcSnap { pos, yaw, hp, state })
}

fn write_thrown(w: &mut Writer, t: &ThrownSnap) {
    w.u8(t.owner);
    w.v3(t.pos);
    w.v3(t.vel);
    w.f32(t.fuse);
    w.f32(t.spin);
    w.bool(t.at_rest);
}

fn read_thrown(r: &mut Reader) -> Result<ThrownSnap> {
    Ok(ThrownSnap { owner: r.u8()?, pos: r.v3()?, vel: r.v3()?, fuse: r.f32()?, spin: r.f32()?, at_rest: r.bool()? })
}

/// Everything in a snapshot but its events, as it differs from `base` if there is one
fn write_state(w: &mut Writer, s: &Snapshot, base: Option<&Snapshot>) {
    w.u32(s.tick);
    w.f32(s.time);
    w.u32(s.ack_seq);
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
    // Panes only ever break, so only the ones the base does not know of are sent
    let new: Vec<&(u16, f32)> = s.broken.iter().filter(|b| base.is_none_or(|base| !base.broken.contains(b))).collect();
    w.u16(new.len() as u16);
    for &&(i, at) in &new {
        w.u16(i);
        w.f32(at);
    }
}

fn read_state(r: &mut Reader, base: Option<&Snapshot>) -> Result<Snapshot> {
    let (tick, time, ack_seq, culling) = (r.u32()?, r.f32()?, r.u32()?, r.bool()?);
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
    Ok(Snapshot { tick, time, ack_seq, culling, players, npcs, thrown, doors, broken, events: Vec::new() })
}

fn write_setup(w: &mut Writer, s: &Setup) {
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
    w.opt_f32(s.doors_open);
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

fn read_setup(r: &mut Reader) -> Result<Setup> {
    let (map, model, idle) = (r.string()?, r.string()?, r.string()?);
    let (magazine, ammo, reserve, grenades) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
    let speed_scale = r.f32()?;
    let seed = r.u64()?;
    let culling = r.bool()?;
    let pvp = r.bool()?;
    let doors_open = r.opt_f32()?;
    let n = r.u16()? as usize;
    let mut npcs = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        let (pos, floor, yaw, scale, stagger) = (r.v3()?, r.f32()?, r.f32()?, r.v3()?, r.f32()?);
        let actor = r.u16()?;
        npcs.push(NpcSetup { pos, floor, yaw, scale, stagger, actor: (actor != u16::MAX).then_some(actor) });
    }
    Ok(Setup { map, model, idle, magazine, ammo, reserve, grenades, speed_scale, seed, culling, pvp, doors_open, npcs })
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
            ToServer::Connect { token, build } => {
                header(&mut w, 0);
                w.u32(*token);
                w.string(build);
            }
            ToServer::Input { token, ack_snapshot, first_seq, view, inputs } => {
                header(&mut w, 1);
                w.u32(*token);
                w.u32(*ack_snapshot);
                w.u32(*first_seq);
                w.u32(*view);
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
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Result<ToServer> {
        let mut r = Reader::new(bytes);
        let kind = check_header(&mut r)?;
        let msg = match kind {
            0 => ToServer::Connect { token: r.u32()?, build: r.string()? },
            1 => {
                let (token, ack_snapshot, first_seq, view) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                let n = r.u8()? as usize;
                ensure!(n <= 32, "too many inputs");
                let mut before = Before::default();
                let mut inputs = Vec::with_capacity(n);
                for _ in 0..n {
                    inputs.push(r.input(&mut before)?);
                }
                ToServer::Input { token, ack_snapshot, first_seq, view, inputs }
            }
            2 => ToServer::Leave { token: r.u32()? },
            k => bail!("no such message as {k}"),
        };
        ensure!(r.left() == 0, "{} bytes left over", r.left());
        Ok(msg)
    }
}

impl ToClient {
    /// The message as bytes. A snapshot leaves out the events that do not fit in a datagram;
    /// `sent_events` says how many were put in.
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
                let mut body = Writer::new();
                for (id, e) in &s.events {
                    let mut one = Writer::new();
                    if !write_event(&mut one, e) {
                        // Not sent, but counted as handled so that it is not tried again
                        sent += 1;
                        continue;
                    }
                    if w.0.len() + 1 + body.0.len() + 4 + one.0.len() > MAX_PACKET {
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
            ToClient::Goodbye { token } => {
                header(&mut w, 3);
                w.u32(*token);
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
            3 => ToClient::Goodbye { token: r.u32()? },
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
            seed: 0x5eed, culling: true, pvp: true, doors_open: Some(0.0),
            npcs: (0..8).map(|i| NpcSetup {
                pos: [i as f32 * 110.0, 0.0, 12.0], floor: 0.0, yaw: 90.0 + i as f32, scale: [1.0, 1.0, 1.0],
                stagger: i as f32 * 0.73, actor: (i % 3 != 0).then_some(i as u16),
            }).collect(),
        }
    }

    fn player(slot: u8) -> PlayerSnap {
        PlayerSnap {
            slot, pos: [1.0, 2.0, 3.0], vel_y: -4.0, on_ground: true, crouched: true, look: (123, 65000), controlled: true, moved_speed: 7.5,
            ammo: 12, reserve: 400, reload_until: Some(3.5), flash_until: 1.25, gun_trigger: true, kills: 3, deaths: 1, hp: 62.5,
            respawn_at: Some(20.0),
            equipped_grenade: true, grenades_left: 2, charge_from: None,
            throw: Some(Throw { start: 1.0, charge: 0.5, thrown: false }),
            message: Some(("KILLED".to_string(), 9.0)), target_hp: Some(33.0),
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            tick: 900, time: 15.0, ack_seq: 777, culling: true,
            players: vec![player(0), PlayerSnap { message: None, target_hp: None, ..player(3) }],
            npcs: vec![
                NpcSnap { pos: [1.0, 0.0, 2.0], yaw: 30.0, hp: 100.0, state: NpcState::Idle },
                NpcSnap { pos: [2.0, 0.0, 2.0], yaw: 60.0, hp: 40.0, state: NpcState::Reacting { motion: "thwu_bf", start: 14.5 } },
                NpcSnap { pos: [3.0, 0.0, 2.0], yaw: 90.0, hp: 0.0, state: NpcState::Dead { motion: "thdh_2", start: 12.0 } },
            ],
            thrown: vec![ThrownSnap { owner: 1, pos: [5.0; 3], vel: [1.0, 2.0, 3.0], fuse: 1.5, spin: 40.0, at_rest: false }],
            doors: vec![(0.0, false), (0.5, true), (1.0, true)],
            broken: vec![(3, 10.5), (200, 11.0)],
            events: vec![
                (10, Event::Sound { name: "C_GRFRAG.WAV".to_string(), at: Some([1.0, 2.0, 3.0]), gain: 1.0 }),
                (11, Event::Shot { player: 2, start: [0.0; 3], end: [9.0, 8.0, 7.0], hit_character: true }),
                (12, Event::BulletHit { point: [1.0; 3], normal: [0.0, 1.0, 0.0], dir: [0.0, -1.0, 0.0], surface: Surface::Metal, material: "x.bmp".to_string(), dist: 55.0, seed: 99 }),
                (13, Event::Glass { pane: 4, at: [1.0; 3], dir: [0.0, 0.0, 1.0], seed: 5 }),
                (14, Event::Blast { point: [1.0; 3], normal: [0.0, 1.0, 0.0], surface: Surface::Glass, on_surface: false, seed: 6 }),
                (15, Event::PlayerBlast { player: 1, shake: 0.5, veil: 0.25 }),
                (16, Event::Kill { killer: Some(0), victim: 1, cause: Cause::HeadShot }),
                (17, Event::Kill { killer: None, victim: 1, cause: Cause::Grenade }),
            ],
        }
    }

    #[test]
    fn every_message_survives_the_trip() {
        let messages = [
            ToServer::Connect { token: 42, build: "abc1234".to_string() },
            ToServer::Input { token: 42, ack_snapshot: 897, first_seq: 5000, view: 14400, inputs: (0..5).map(|i| {
                let mut x = Input { forward: i % 2 == 0, fire: i > 2, run: true, ..Input::default() };
                x.set_look(i as f32 * 10.0, -5.0);
                x
            }).collect() },
            ToServer::Leave { token: 42 },
        ];
        for m in messages {
            assert_eq!(ToServer::decode(&m.encode()).unwrap(), m);
        }
        let mut snap = snapshot();
        let (bytes, sent) = ToClient::Snapshot(snap.clone()).encode();
        assert_eq!(sent, 8);
        assert_eq!(ToClient::decode(&bytes).unwrap(), ToClient::Snapshot(snap.clone()));
        // The server's own log is not sent, and does not count against what was not
        snap.events.insert(1, (99, Event::Log("t=1.00 door opening".to_string())));
        let (bytes, sent) = ToClient::Snapshot(snap.clone()).encode();
        assert_eq!(sent, 9);
        snap.events.remove(1);
        assert_eq!(ToClient::decode(&bytes).unwrap(), ToClient::Snapshot(snap));
        for m in [
            ToClient::Accept { token: 42, slot: 2, tick: 1234, setup: setup() },
            ToClient::Refuse { token: 42, reason: "the game is full".to_string() },
            ToClient::Goodbye { token: 42 },
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

    #[test]
    fn a_full_snapshot_fits_in_a_datagram() {
        let mut snap = snapshot();
        // Only the receiver's own player carries what is on their screen
        snap.players = (0..MAX_PLAYERS as u8).map(|i| if i == 0 { player(0) } else { PlayerSnap { message: None, target_hp: None, ..player(i) } }).collect();
        snap.npcs = (0..16).map(|_| NpcSnap { pos: [1.0; 3], yaw: 1.0, hp: 1.0, state: NpcState::Reacting { motion: "thwu_bb", start: 1.0 } }).collect();
        snap.events.clear();
        let (bytes, _) = ToClient::Snapshot(snap).encode();
        assert!(bytes.len() < MAX_PACKET - 40, "{} bytes before any events", bytes.len());
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
        next.npcs[1].state = NpcState::Dead { motion: "thdh_2", start: 15.0 };
        next.npcs[1].hp = 0.0;
        next.thrown.push(ThrownSnap { owner: 0, pos: [1.0; 3], vel: [0.0; 3], fuse: 2.0, spin: 0.0, at_rest: true });
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
}
