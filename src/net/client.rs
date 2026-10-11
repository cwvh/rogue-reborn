//! A client: it sends the server what its player does, and keeps its own copy of the game up
//! to date from the server's snapshots.
//!
//! The copy is a `Sim` built from the same setup. Everything in it but the client's own player
//! is overwritten by what the server says. The player is moved at once by their own input, as
//! the server will move them, so that the game feels immediate: when a snapshot says where the
//! server had them, the inputs it has not yet used are played again from there.

use std::collections::VecDeque;

use crate::collision::Player;
use crate::net::link::{Addr, Link};
use crate::net::proto::{INPUTS_PER_PACKET, Setup, Snapshot, ToClient, ToServer};
use crate::sim::{Event, Input, Sim, TICK};

/// How often (seconds) a client that is not in yet asks again
const RETRY: f64 = 0.25;

/// A server that has said nothing for this long (seconds) is lost
const TIMEOUT: f64 = 5.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Waiting for the server to let us in
    Connecting,
    Connected,
    /// The server said no, and why
    Refused(String),
    /// The server went quiet, or sent us away
    Lost,
}

pub struct Client {
    server: Addr,
    token: u32,
    build: String,
    pub status: Status,
    setup: Option<Setup>,
    slot: usize,
    /// The number of the next input
    seq: u32,
    /// The latest few inputs, sent again every time
    recent: VecDeque<(u32, Input)>,
    /// Inputs the server has not confirmed using
    pending: VecDeque<(u32, Input)>,
    /// The tick of the latest snapshot seen
    last_snapshot: u32,
    /// Events up to this number have been handled
    event_seen: u32,
    heard: f64,
    asked: f64,
    /// How far the last snapshot moved the player from where they were predicted to be
    pub correction: f32,
    /// Snapshots received
    pub snapshots: u32,
}

impl Client {
    pub fn new(server: Addr, token: u32, build: &str) -> Client {
        Client {
            server, token, build: build.to_string(), status: Status::Connecting, setup: None, slot: 0, seq: 1,
            recent: VecDeque::new(), pending: VecDeque::new(), last_snapshot: 0, event_seen: 0, heard: 0.0,
            asked: f64::NEG_INFINITY, correction: 0.0, snapshots: 0,
        }
    }

    /// How the game was set up, once the server has said
    pub fn setup(&self) -> Option<&Setup> {
        self.setup.as_ref()
    }

    /// Which player in the game this client is
    pub fn slot(&self) -> usize {
        self.slot
    }

    pub fn leave(&mut self, link: &mut impl Link) {
        link.send(self.server, &ToServer::Leave { token: self.token }.encode());
        self.status = Status::Lost;
    }

    /// Listen, until the server lets us in. Returns the setup once it has.
    pub fn connect(&mut self, now: f64, link: &mut impl Link) -> Option<&Setup> {
        if self.status == Status::Connecting && now - self.asked >= RETRY {
            self.asked = now;
            link.send(self.server, &ToServer::Connect { token: self.token, build: self.build.clone() }.encode());
        }
        self.listen(now, link);
        if self.status == Status::Connected { self.setup.as_ref() } else { None }
    }

    /// Everything that has arrived. Returns the newest snapshot, with the events of all of them
    /// that have not been seen.
    fn listen(&mut self, now: f64, link: &mut impl Link) -> Option<Snapshot> {
        let mut newest: Option<Snapshot> = None;
        let mut events: Vec<(u32, Event)> = Vec::new();
        while let Some((from, bytes)) = link.recv() {
            if from != self.server {
                continue;
            }
            let Ok(msg) = ToClient::decode(&bytes) else { continue };
            match msg {
                ToClient::Accept { token, slot, setup, .. } if token == self.token => {
                    self.heard = now;
                    if self.status == Status::Connecting {
                        self.slot = slot as usize;
                        self.setup = Some(setup);
                        self.status = Status::Connected;
                    }
                }
                ToClient::Refuse { token, reason } if token == self.token => {
                    self.status = Status::Refused(reason);
                }
                ToClient::Goodbye { token } if token == self.token => {
                    self.status = Status::Lost;
                }
                ToClient::Snapshot(s) if self.status == Status::Connected => {
                    self.heard = now;
                    events.extend(s.events.iter().cloned());
                    if s.tick > newest.as_ref().map_or(self.last_snapshot, |n| n.tick) {
                        newest = Some(s);
                    }
                }
                _ => {}
            }
        }
        events.sort_by_key(|(id, _)| *id);
        events.dedup_by_key(|(id, _)| *id);
        let fresh: Vec<(u32, Event)> = events.into_iter().filter(|(id, _)| *id > self.event_seen).collect();
        if let Some(&(last, _)) = fresh.last() {
            self.event_seen = last;
        }
        let mut snap = newest?;
        snap.events = fresh;
        Some(snap)
    }

    /// One tick of the client's game, with what the player did in it. Their player moves at once;
    /// the server is told; and whatever it has said is put into `sim`. Returns what has happened
    /// that is news.
    pub fn tick(&mut self, now: f64, link: &mut impl Link, sim: &mut Sim, input: Input) -> Vec<Event> {
        if self.status != Status::Connected {
            self.connect(now, link);
            return Vec::new();
        }

        let mut events = Vec::new();
        if let Some(snap) = self.listen(now, link) {
            self.last_snapshot = snap.tick;
            self.snapshots += 1;
            self.reconcile(sim, &snap);
            events = snap.events.into_iter().map(|(_, e)| e).collect();
        }
        if self.status != Status::Connected {
            return events;
        }
        if now - self.heard > TIMEOUT {
            self.status = Status::Lost;
            return events;
        }

        // The player's own input: move them now, and tell the server
        let seq = self.seq;
        self.seq += 1;
        self.recent.push_back((seq, input));
        while self.recent.len() > INPUTS_PER_PACKET {
            self.recent.pop_front();
        }
        self.pending.push_back((seq, input));
        sim.predict(self.slot, &input, TICK);
        sim.time += TICK;
        let msg = ToServer::Input {
            token: self.token,
            ack_snapshot: self.last_snapshot,
            first_seq: self.recent.front().map_or(seq, |(s, _)| *s),
            inputs: self.recent.iter().map(|(_, i)| *i).collect(),
        };
        link.send(self.server, &msg.encode());
        events
    }

    /// Put what the server says into the client's copy of the game, and move the player on
    /// from where the server had them by the inputs it has not used yet
    fn reconcile(&mut self, sim: &mut Sim, snap: &Snapshot) {
        let before = sim.players.get(self.slot).map(|p| p.player.pos);
        snap.apply(sim, self.slot);
        let Some(mine) = snap.players.iter().find(|p| p.slot as usize == self.slot) else { return };
        while self.pending.front().is_some_and(|&(s, _)| s <= snap.ack_seq) {
            self.pending.pop_front();
        }
        let p = &mut sim.players[self.slot];
        p.player = Player { pos: mine.pos, vel_y: mine.vel_y, on_ground: mine.on_ground };
        let inputs: Vec<Input> = self.pending.iter().map(|(_, i)| *i).collect();
        for input in &inputs {
            sim.predict(self.slot, input, TICK);
        }
        if let Some(b) = before {
            let a = sim.players[self.slot].player.pos;
            self.correction = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
        }
    }
}
