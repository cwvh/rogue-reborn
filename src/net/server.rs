//! The server: it owns the game. Clients send what their players do; the server runs the sim
//! with those inputs, a tick at a time, and sends back snapshots of the result.
//!
//! It is driven by the time it is given and the link it is given, never a clock or a socket of
//! its own, so a test can run it against a pretend network.

use std::collections::{BTreeMap, VecDeque};

use crate::net::link::{Addr, Link};
use crate::net::proto::{Setup, Snapshot, ToClient, ToServer};
use crate::sim::{Event, Input, Sim, TICK};

type V3 = [f32; 3];

/// Snapshots are sent every this many ticks unless the server is told otherwise: 30 a second
pub const SNAPSHOT_EVERY: u32 = 2;

/// How many snapshots sent to each client are kept to be the difference from. A client whose
/// latest news is older than this is sent a snapshot in full.
const REMEMBERED: usize = 64;

/// A client that has said nothing for this long (seconds) is gone
const TIMEOUT: f64 = 5.0;

/// How long (in ticks) events are kept for clients that have not confirmed them yet
const EVENT_MEMORY: u32 = 180;

/// A client's inputs arrive in a bumpy stream: they are held in a queue, which gives the server
/// a few to spend when one is late. Beyond this many, the oldest are dropped to catch up.
const MAX_QUEUE: usize = 6;

/// If an input is missing but this many later ones are waiting, the missing one is given up on
const GIVE_UP_AFTER: usize = 3;

/// The most ticks run in one update, if the server has fallen behind
const MAX_CATCH_UP: u32 = 10;

struct Remote {
    addr: Addr,
    token: u32,
    slot: usize,
    /// Inputs that have arrived and are not used yet, by number
    queue: BTreeMap<u32, Input>,
    /// The number of the next input to use, once the first has arrived
    next_seq: Option<u32>,
    /// What the player did last, which goes on if their inputs stop coming
    last_input: Input,
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
    fn take(&mut self) -> Input {
        let Some(mut next) = self.next_seq else { return self.last_input.held() };
        // Too far behind: drop the oldest and catch up
        while self.queue.len() > MAX_QUEUE {
            self.queue.pop_first();
            next = self.queue.keys().next().copied().unwrap_or(next);
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
            Some((s, input)) => {
                self.next_seq = Some(s + 1);
                self.applied_seq = s;
                self.last_input = input;
                input
            }
            None => {
                self.next_seq = Some(next);
                self.last_input.held()
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
    /// How many snapshots have been sent, and how many bytes they came to
    pub snapshots_sent: u64,
    pub snapshot_bytes: u64,
}

impl Server {
    pub fn new(sim: Sim, setup: Setup, build: &str, spawns: Vec<V3>) -> Server {
        assert!(!spawns.is_empty(), "players need somewhere to start");
        Server { sim, setup, build: build.to_string(), spawns, spawned: 0, remotes: Vec::new(), tick: 0, next_tick_at: None, events: VecDeque::new(), next_event: 1, snapshot_every: SNAPSHOT_EVERY, snapshots_sent: 0, snapshot_bytes: 0 }
    }

    /// Send snapshots this many times a second (the nearest the 60 ticks a second allow)
    pub fn set_snapshot_rate(&mut self, hz: f32) {
        self.snapshot_every = ((1.0 / TICK) / hz.max(1.0)).round().max(1.0) as u32;
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
            ToServer::Connect { token, build } => {
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
                let spawn = self.spawns[self.spawned % self.spawns.len()];
                let Some(slot) = self.sim.add_player(spawn) else {
                    link.send(from, &ToClient::Refuse { token, reason: "the game is full".to_string() }.encode().0);
                    return;
                };
                self.spawned += 1;
                self.remotes.push(Remote {
                    addr: from, token, slot, queue: BTreeMap::new(), next_seq: None, last_input: Input { run: true, ..Input::default() },
                    applied_seq: 0, event_ack: self.next_event - 1, in_flight: VecDeque::new(), heard: now, acked: 0, sent: VecDeque::new(),
                });
                self.accept(link, from, token, slot);
            }
            ToServer::Input { token, ack_snapshot, first_seq, inputs } => {
                let Some(r) = self.remotes.iter_mut().find(|r| r.addr == from && r.token == token) else { return };
                r.heard = now;
                r.acked = r.acked.max(ack_snapshot);
                for (i, input) in inputs.into_iter().enumerate() {
                    let seq = first_seq.wrapping_add(i as u32);
                    let next = *r.next_seq.get_or_insert(seq);
                    // Old news, or nonsense from far in the future
                    if seq.wrapping_sub(next) < 1 << 16 {
                        r.queue.insert(seq, input);
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
                    self.sim.remove_player(r.slot);
                }
            }
        }
    }

    fn accept(&self, link: &mut impl Link, to: Addr, token: u32, slot: usize) {
        let msg = ToClient::Accept { token, slot: slot as u8, tick: self.tick, setup: self.setup.clone() };
        link.send(to, &msg.encode().0);
    }

    /// One tick of the game
    fn step(&mut self, now: f64, link: &mut impl Link) -> Vec<Event> {
        let mut inputs = vec![Input::default(); self.sim.players.len()];
        for r in self.remotes.iter_mut() {
            inputs[r.slot] = r.take();
        }
        let events = self.sim.tick(&inputs, TICK);
        self.tick += 1;
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
            self.sim.remove_player(r.slot);
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
        link.send(r.addr, &bytes);
        self.snapshots_sent += 1;
        self.snapshot_bytes += bytes.len() as u64;
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
