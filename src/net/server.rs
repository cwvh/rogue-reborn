//! The server: it owns the game. Clients send what their players do; the server runs the sim
//! with those inputs, a tick at a time, and sends back snapshots of the result.
//!
//! It is driven by the time it is given and the link it is given, never a clock or a socket of
//! its own, so a test can run it against a pretend network.

use std::collections::{BTreeMap, VecDeque};

use crate::net::link::{Addr, Link};
use crate::net::proto::{Setup, Snapshot, ToClient, ToServer, split};
use crate::net::record::{ServerRecorder, ServerRecording};
use crate::sim::{Event, Input, Side, Sim, TICK};

type V3 = [f32; 3];

/// Snapshots are sent every this many ticks unless the server is told otherwise: 60 a second.
/// With `client::VIEW_DELAY` this shows others 25 ms in the past, about as Quake Live does,
/// and halves the peeker's advantage on the LAN against 30 a second (see `rrlab --tune`).
pub const SNAPSHOT_EVERY: u32 = 1;

/// How many snapshots sent to each client are kept to be the difference from (about two
/// seconds). A client whose latest news is older than this is sent a snapshot in full.
const REMEMBERED: usize = 128;

/// A client that has said nothing for this long (seconds) is gone
const TIMEOUT: f64 = 5.0;

/// How long (in ticks) events are kept for clients that have not confirmed them yet
const EVENT_MEMORY: u32 = 180;

/// A client's inputs arrive in a bumpy stream: they are held in a queue, which gives the server
/// a few to spend when one is late. Beyond this many, the oldest are dropped to catch up.
const MAX_QUEUE: usize = 6;

/// If an input is missing but this many later ones are waiting, the missing one is given up on
const GIVE_UP_AFTER: usize = 3;

/// A client's clock is never quite the server's. One that runs fast sends inputs a little faster
/// than they are used, and they would wait longer and longer in the queue. Every `SETTLE`
/// ticks, a client that had `SPARE` inputs waiting even at the emptiest moment is steadily
/// ahead, and one of its inputs is skipped.
const SETTLE: u32 = 60;
const SPARE: usize = 2;

/// With no input coming, the last one is held this many ticks, to bridge a short loss, and then
/// let go: a client that has stopped, stuck or put aside, does not run on and fire by itself
const HOLD: u32 = 15;

/// The furthest back (seconds) that a shot is judged against where the others were, unless
/// told otherwise. Less than half a second fails players far away; three quarters carries a
/// shooter on Wi-Fi through most stalls, at no cost the lab can see to players on the LAN.
pub const MAX_REWIND: f32 = 0.75;

/// The most ticks run in one update, if the server has fallen behind
const MAX_CATCH_UP: u32 = 10;

struct Remote {
    addr: Addr,
    token: u32,
    slot: usize,
    /// Inputs that have arrived and are not used yet, by number
    queue: BTreeMap<u32, (Input, u32)>,
    /// The number of the next input to use, once the first has arrived
    next_seq: Option<u32>,
    /// What the player did last, which goes on for a while if their inputs stop coming
    last_input: Input,
    /// Ticks since an input came to use
    starved: u32,
    /// The moment the client showed as it sent the last input used (`ToServer::Input::view`)
    last_view: u32,
    /// The fewest inputs waiting beyond the one used, and ticks, since the queue was last looked at
    low: usize,
    looked: u32,
    /// The number of the latest input used
    applied_seq: u32,
    /// Events up to this number are known to have arrived
    event_ack: u32,
    /// Snapshots sent and not confirmed yet: the tick, and the last event in each
    in_flight: VecDeque<(u32, u32)>,
    heard: f64,
    /// The tick of the latest snapshot the client says it has
    acked: u32,
    /// The latest snapshots sent, without their events: what a later one can be the difference from
    sent: VecDeque<Snapshot>,
}

impl Remote {
    /// The input for this tick
    fn take(&mut self) -> (Input, u32) {
        let Some(mut next) = self.next_seq else { return (self.last_input.held(), 0) };
        // The last input again, as at the moment the client would have shown by now, so that a
        // shot from a held trigger is judged as the shooter would have seen it
        let missing = |r: &mut Remote| {
            r.starved += 1;
            let view = if r.last_view == 0 { 0 } else { r.last_view + 16 * r.starved };
            (if r.starved > HOLD { r.last_input.let_go() } else { r.last_input.held() }, view)
        };
        // Too far behind: drop the oldest and catch up
        while self.queue.len() > MAX_QUEUE {
            self.queue.pop_first();
            next = self.queue.keys().next().copied().unwrap_or(next);
        }
        // Steadily ahead: skip one, keeping what it pressed
        self.low = self.low.min(self.queue.len().saturating_sub(1));
        self.looked += 1;
        if self.looked >= SETTLE {
            if self.low >= SPARE {
                if let Some((skipped, _)) = self.queue.remove(&next) {
                    next = self.queue.keys().next().copied().unwrap_or(next + 1);
                    if let Some((input, _)) = self.queue.get_mut(&next) {
                        input.use_key |= skipped.use_key;
                        input.reload |= skipped.reload;
                        input.gun |= skipped.gun;
                        input.grenade |= skipped.grenade;
                        input.toggle_culling |= skipped.toggle_culling;
                    }
                }
            }
            (self.low, self.looked) = (usize::MAX, 0);
        }
        let seq = if self.queue.contains_key(&next) {
            Some(next)
        } else if self.queue.len() >= GIVE_UP_AFTER {
            // The one we want is not coming
            self.queue.keys().next().copied()
        } else {
            None
        };
        match seq.and_then(|s| self.queue.remove(&s).map(|i| (s, i))) {
            Some((s, (input, view))) => {
                self.next_seq = Some(s + 1);
                self.applied_seq = s;
                self.last_input = input;
                self.last_view = view;
                self.starved = 0;
                (input, view)
            }
            None => {
                self.next_seq = Some(next);
                missing(self)
            }
        }
    }
}

pub struct Server {
    pub sim: Sim,
    setup: Setup,
    build: String,
    /// Where new players start, in turn
    spawns: Vec<V3>,
    spawned: usize,
    remotes: Vec<Remote>,
    pub tick: u32,
    next_tick_at: Option<f64>,
    /// Events not everyone has confirmed: the tick, the number, what
    events: VecDeque<(u32, u32, Event)>,
    next_event: u32,
    snapshot_every: u32,
    /// Whether a shot is judged against where the others were when the shooter saw them, and
    /// how far back at most (ticks)
    lag_compensation: bool,
    max_rewind: f32,
    /// How many snapshots have been sent, and how many bytes they came to
    pub snapshots_sent: u64,
    pub snapshot_bytes: u64,
    /// Snapshots too big for a datagram, sent in parts
    pub snapshots_split: u64,
    recorder: Option<ServerRecorder>,
}

impl Server {
    pub fn new(sim: Sim, setup: Setup, build: &str, spawns: Vec<V3>) -> Server {
        assert!(!spawns.is_empty(), "players need somewhere to start");
        let mut sim = sim;
        sim.spawns = spawns.clone();
        Server { sim, setup, build: build.to_string(), spawns, spawned: 0, remotes: Vec::new(), tick: 0, next_tick_at: None, events: VecDeque::new(), next_event: 1, snapshot_every: SNAPSHOT_EVERY, lag_compensation: true, max_rewind: MAX_REWIND / TICK, snapshots_sent: 0, snapshot_bytes: 0, snapshots_split: 0, recorder: None }
    }

    /// Send snapshots this many times a second (the nearest the 60 ticks a second allow)
    pub fn set_snapshot_rate(&mut self, hz: f32) {
        self.snapshot_every = ((1.0 / TICK) / hz.max(1.0)).round().max(1.0) as u32;
    }

    /// Judge shots against where the others were when the shooter saw them (the default), or
    /// where they are now
    pub fn set_lag_compensation(&mut self, on: bool) {
        self.lag_compensation = on;
    }

    /// Record the match from here, which must be before it has begun
    pub fn record(&mut self) {
        assert!(self.tick == 0 && self.remotes.is_empty(), "a match is recorded from its start");
        self.recorder = Some(ServerRecorder::new(&self.build, self.setup.clone(), self.spawns.clone()));
    }

    /// The match so far, if it is being recorded
    pub fn recording(&self) -> Option<ServerRecording> {
        self.recorder.as_ref().map(|r| r.recording(&self.sim))
    }

    /// Judge shots against where the others were at most this many seconds back
    pub fn set_max_rewind(&mut self, seconds: f32) {
        self.max_rewind = seconds / TICK;
    }

    pub fn players(&self) -> usize {
        self.remotes.len()
    }

    /// Everything that has arrived is dealt with, and the game runs for as many ticks as `now`
    /// has brought. Returns what happened in them, for the server's own log.
    pub fn update(&mut self, now: f64, link: &mut impl Link) -> Vec<Event> {
        while let Some((from, bytes)) = link.recv() {
            if let Ok(msg) = ToServer::decode(&bytes) {
                self.receive(now, from, msg, link);
            }
        }

        let mut log = Vec::new();
        let next = self.next_tick_at.get_or_insert(now);
        // A long stall is not made up for
        if now - *next > TICK as f64 * MAX_CATCH_UP as f64 {
            *next = now;
        }
        let mut ran = 0;
        while now >= self.next_tick_at.unwrap() && ran < MAX_CATCH_UP {
            self.next_tick_at = Some(self.next_tick_at.unwrap() + TICK as f64);
            log.extend(self.step(now, link));
            ran += 1;
        }
        log
    }

    fn receive(&mut self, now: f64, from: Addr, msg: ToServer, link: &mut impl Link) {
        match msg {
            ToServer::Connect { token, build, side: wish } => {
                if let Some(r) = self.remotes.iter().find(|r| r.addr == from) {
                    // It did not hear us the first time
                    if r.token == token {
                        self.accept(link, from, token, r.slot);
                    }
                    return;
                }
                if build != self.build {
                    let reason = format!("this server runs build {}, and you have {build}", self.build);
                    link.send(from, &ToClient::Refuse { token, reason }.encode().0);
                    return;
                }
                // In a team game, their side and its base
                let side = self.side_for(wish);
                let spawn = side.and_then(|s| self.sim.start_for(s)).unwrap_or(self.spawns[self.spawned % self.spawns.len()]);
                let Some(slot) = self.sim.add_player(spawn) else {
                    link.send(from, &ToClient::Refuse { token, reason: "the game is full".to_string() }.encode().0);
                    return;
                };
                self.sim.players[slot].side = side;
                self.spawned += 1;
                if let Some(rec) = self.recorder.as_mut() {
                    rec.joined(slot, spawn, side);
                }
                self.remotes.push(Remote {
                    addr: from, token, slot, queue: BTreeMap::new(), next_seq: None, last_input: Input { run: true, ..Input::default() }, starved: 0, last_view: 0, low: usize::MAX, looked: 0,
                    applied_seq: 0, event_ack: self.next_event - 1, in_flight: VecDeque::new(), heard: now, acked: 0, sent: VecDeque::new(),
                });
                self.accept(link, from, token, slot);
            }
            ToServer::Input { token, ack_snapshot, first_seq, view, inputs } => {
                let Some(r) = self.remotes.iter_mut().find(|r| r.addr == from && r.token == token) else { return };
                r.heard = now;
                r.acked = r.acked.max(ack_snapshot);
                let count = inputs.len() as u32;
                for (i, input) in inputs.into_iter().enumerate() {
                    let seq = first_seq.wrapping_add(i as u32);
                    // The view moved on a tick for each input
                    let view = if view == 0 { 0 } else { view.saturating_sub((count - 1 - i as u32) * 16) };
                    let next = *r.next_seq.get_or_insert(seq);
                    // Old news, or nonsense from far in the future
                    if seq.wrapping_sub(next) < 1 << 16 {
                        r.queue.insert(seq, (input, view));
                    }
                }
                while r.in_flight.front().is_some_and(|&(tick, _)| tick <= ack_snapshot) {
                    let (_, last) = r.in_flight.pop_front().unwrap();
                    r.event_ack = r.event_ack.max(last);
                }
            }
            ToServer::Leave { token } => {
                if let Some(i) = self.remotes.iter().position(|r| r.addr == from && r.token == token) {
                    let r = self.remotes.remove(i);
                    self.drop_player(r.slot);
                }
            }
        }
    }

    /// The side a player joining a team game goes on: the one they asked for, unless it has
    /// more players than the other, and else the smaller, Blue if they are even
    fn side_for(&self, wish: Option<Side>) -> Option<Side> {
        if self.sim.bases.iter().any(Vec::is_empty) {
            return None;
        }
        let on = |s: Side| self.sim.players.iter().filter(|p| p.active && p.side == Some(s)).count();
        match wish {
            Some(w) if on(w) <= on(w.other()) => Some(w),
            _ if on(Side::Gold) < on(Side::Blue) => Some(Side::Gold),
            _ => Some(Side::Blue),
        }
    }

    fn drop_player(&mut self, slot: usize) {
        self.sim.remove_player(slot);
        if let Some(rec) = self.recorder.as_mut() {
            rec.left(slot);
        }
    }

    fn accept(&self, link: &mut impl Link, to: Addr, token: u32, slot: usize) {
        let msg = ToClient::Accept { token, slot: slot as u8, tick: self.tick, setup: self.setup.clone() };
        link.send(to, &msg.encode().0);
    }

    /// One tick of the game
    fn step(&mut self, now: f64, link: &mut impl Link) -> Vec<Event> {
        let mut inputs = vec![Input::default(); self.sim.players.len()];
        let mut rewind = vec![0.0; inputs.len()];
        for r in self.remotes.iter_mut() {
            let (input, view) = r.take();
            inputs[r.slot] = input;
            if self.lag_compensation && view > 0 {
                rewind[r.slot] = (self.tick as f32 - view as f32 / 16.0).clamp(0.0, self.max_rewind);
            }
        }
        self.sim.rewind = rewind.clone();
        let fired_before = self.sim.fired.as_ref().map_or(0, Vec::len);
        let events = self.sim.tick(&inputs, TICK);
        if let Some(fired) = self.sim.fired.as_mut() {
            for f in &mut fired[fired_before..] {
                // The input the client sent for this tick, though it may never have come
                f.seq = self.remotes.iter().find(|r| r.slot == f.shooter).map_or(0, |r| r.applied_seq + r.starved);
            }
        }
        self.tick += 1;
        if let Some(rec) = self.recorder.as_mut() {
            rec.ticked(&inputs, &rewind, &self.sim);
        }
        for e in &events {
            self.events.push_back((self.tick, self.next_event, e.clone()));
            self.next_event += 1;
        }
        while self.events.front().is_some_and(|(t, _, _)| self.tick - t > EVENT_MEMORY) {
            self.events.pop_front();
        }

        // Anyone who has gone quiet is gone
        let gone: Vec<usize> = self.remotes.iter().enumerate().filter(|(_, r)| now - r.heard > TIMEOUT).map(|(i, _)| i).collect();
        for i in gone.into_iter().rev() {
            let r = self.remotes.remove(i);
            self.drop_player(r.slot);
        }

        if self.tick % self.snapshot_every == 0 {
            for i in 0..self.remotes.len() {
                self.send_snapshot(i, link);
            }
        }
        events
    }

    fn send_snapshot(&mut self, i: usize, link: &mut impl Link) {
        let r = &self.remotes[i];
        let mut snap = Snapshot::capture(&self.sim, self.tick, r.slot, r.applied_seq);
        let stored = snap.clone();
        snap.events = self.events.iter().filter(|(_, id, _)| *id > r.event_ack).map(|(_, id, e)| (*id, e.clone())).collect();
        let ids: Vec<u32> = snap.events.iter().map(|(id, _)| *id).collect();
        // What the client is known to have is what this one is told as the difference from
        let base = r.sent.iter().find(|s| r.acked > 0 && s.tick == r.acked);
        let (bytes, sent) = ToClient::Snapshot(snap).encode_against(base);
        self.snapshots_sent += 1;
        self.snapshot_bytes += bytes.len() as u64;
        let datagrams = split(self.tick, bytes);
        if datagrams.len() > 1 {
            self.snapshots_split += 1;
        }
        for d in &datagrams {
            link.send(r.addr, d);
        }
        let r = &mut self.remotes[i];
        r.sent.push_back(stored);
        while r.sent.len() > REMEMBERED {
            r.sent.pop_front();
        }
        if sent > 0 {
            r.in_flight.push_back((self.tick, ids[sent - 1]));
            while r.in_flight.len() > 64 {
                r.in_flight.pop_front();
            }
        }
    }
}
