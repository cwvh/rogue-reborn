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

type V3 = [f32; 3];

/// Where one other player was in a snapshot: their slot, position and look
pub type Seen = (usize, V3, (u16, u16));

/// A grenade in the air: who threw it, where it is and how far it has turned
pub type Flying = (u8, V3, f32);

/// What moves in the game, as it was in a snapshot or as it is shown: the other players, the
/// characters (position and facing in degrees) and the grenades
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub players: Vec<Seen>,
    pub npcs: Vec<(V3, f32)>,
    pub grenades: Vec<Flying>,
}

/// How many snapshot intervals behind the newest snapshot other players are shown, so that
/// there is nearly always a later snapshot to be moving toward
const VIEW_DELAY: f64 = 2.0;

/// How many snapshots are kept to be the difference from. It is more than the server keeps, so
/// that whichever it picks is here.
const REMEMBERED: usize = 96;

/// The most snapshots kept to look back through
const HISTORY: usize = 24;

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
    /// The other players in the latest snapshots, by server tick
    history: VecDeque<(u32, Frame)>,
    /// The latest snapshots received, without their events: what the server may say a snapshot
    /// is the difference from
    received: VecDeque<Snapshot>,
    /// When (client time) the newest snapshot arrived
    arrived: f64,
    /// The server tick that other players are shown at, which runs at the pace of the clock
    view_tick: Option<f64>,
    view_at: f64,
}

impl Client {
    pub fn new(server: Addr, token: u32, build: &str) -> Client {
        Client {
            server, token, build: build.to_string(), status: Status::Connecting, setup: None, slot: 0, seq: 1,
            recent: VecDeque::new(), pending: VecDeque::new(), last_snapshot: 0, event_seen: 0, heard: 0.0,
            asked: f64::NEG_INFINITY, correction: 0.0, snapshots: 0,
            history: VecDeque::new(), received: VecDeque::new(), arrived: 0.0, view_tick: None, view_at: 0.0,
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
            let received = &self.received;
            let Ok(msg) = ToClient::decode_with(&bytes, &|tick| received.iter().find(|s| s.tick == tick)) else { continue };
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
                ToClient::Snapshot(mut s) if self.status == Status::Connected => {
                    self.heard = now;
                    let announced = std::mem::take(&mut s.events);
                    self.received.push_back(s.clone());
                    while self.received.len() > REMEMBERED {
                        self.received.pop_front();
                    }
                    s.events = announced;
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
            self.remember(now, &snap);
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
            view: (self.view_tick.unwrap_or(self.last_snapshot as f64 - 4.0).max(1.0) * 16.0) as u32,
            inputs: self.recent.iter().map(|(_, i)| *i).collect(),
        };
        link.send(self.server, &msg.encode());
        events
    }

    fn remember(&mut self, now: f64, snap: &Snapshot) {
        self.arrived = now;
        self.history.push_back((snap.tick, Frame {
            players: snap.players.iter().map(|p| (p.slot as usize, p.pos, p.look)).collect(),
            npcs: snap.npcs.iter().map(|n| (n.pos, n.yaw)).collect(),
            grenades: snap.thrown.iter().map(|t| (t.owner, t.pos, t.spin)).collect(),
        }));
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
    }

    /// What moves, as it is shown at time `now`: a little behind the newest snapshot,
    /// moving smoothly between the two that surround that moment, however unevenly they came.
    /// Call it once a frame.
    pub fn view(&mut self, now: f64) -> Frame {
        let Some(newest) = self.history.back().map(|h| h.0) else { return Frame::default() };
        let interval = match self.history.len() {
            0 | 1 => 3.0,
            n => (newest - self.history[n - 2].0).max(1) as f64,
        };
        // Where the server is now, as far as can be told, less the delay
        let target = newest as f64 + (now - self.arrived) / TICK as f64 - interval * VIEW_DELAY;
        let t = match self.view_tick {
            Some(t) if (target - t).abs() < 15.0 => {
                // Run with the clock, and drift toward the target so a late packet does not jerk the view
                let dt = now - self.view_at;
                let advanced = t + dt / TICK as f64;
                advanced + (target - advanced) * (1.0 - (-dt * 4.0).exp())
            }
            _ => target,
        };
        let t = t.min(newest as f64);
        self.view_tick = Some(t);
        self.view_at = now;
        blend(&self.history, t)
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

/// The moving parts of the game at server tick `t`, between the two snapshots that surround it
fn blend(history: &VecDeque<(u32, Frame)>, t: f64) -> Frame {
    let Some(first) = history.front() else { return Frame::default() };
    let (a, b) = match history.iter().position(|h| h.0 as f64 >= t) {
        Some(0) => (first, first),
        Some(i) => (&history[i - 1], &history[i]),
        None => {
            let last = history.back().unwrap();
            (last, last)
        }
    };
    let span = (b.0 - a.0) as f64;
    let f = if span > 0.0 { ((t - a.0 as f64) / span).clamp(0.0, 1.0) as f32 } else { 1.0 };
    let (a, b) = (&a.1, &b.1);
    let lerp = |from: V3, to: V3| [0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * f);

    let players = b.players.iter().map(|&(slot, pos, look)| match a.players.iter().find(|s| s.0 == slot) {
        // Just arrived in the game: there is nothing earlier to move from
        None => (slot, pos, look),
        Some(&(_, from, from_look)) => (slot, lerp(from, pos), (turn_between(from_look.0, look.0, f), turn_between(from_look.1, look.1, f))),
    }).collect();

    let npcs = if a.npcs.len() == b.npcs.len() {
        a.npcs.iter().zip(&b.npcs).map(|(&(from, from_yaw), &(to, to_yaw))| (lerp(from, to), degrees_between(from_yaw, to_yaw, f))).collect()
    } else {
        b.npcs.clone()
    };

    // A grenade is the same one in both if it is the same thrower's first, second, and so on
    let nth = |list: &[Flying], owner: u8, n: usize| list.iter().filter(|g| g.0 == owner).nth(n).copied();
    let mut counted = [0usize; 256];
    let grenades = b.grenades.iter().map(|&(owner, pos, spin)| {
        let n = counted[owner as usize];
        counted[owner as usize] += 1;
        match nth(&a.grenades, owner, n) {
            Some((_, from, from_spin)) => (owner, lerp(from, pos), from_spin + (spin - from_spin) * f),
            None => (owner, pos, spin),
        }
    }).collect();
    Frame { players, npcs, grenades }
}

/// A quantized angle part of the way from `a` to `b`, by the shorter way round
fn turn_between(a: u16, b: u16, f: f32) -> u16 {
    let delta = b.wrapping_sub(a) as i16;
    (a as i32 + (delta as f32 * f).round() as i32) as u16
}

/// An angle in degrees part of the way from `a` to `b`, by the shorter way round
fn degrees_between(a: f32, b: f32, f: f32) -> f32 {
    a + ((b - a + 540.0).rem_euclid(360.0) - 180.0) * f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(points: &[(u32, f32, u16)]) -> VecDeque<(u32, Frame)> {
        points.iter().map(|&(t, x, yaw)| (t, Frame {
            players: vec![(1, [x, 0.0, 0.0], (yaw, 0))],
            npcs: vec![([x, 0.0, 0.0], yaw as f32)],
            grenades: vec![(2, [x, 1.0, 0.0], x * 10.0)],
        })).collect()
    }

    #[test]
    fn blends_between_snapshots_and_holds_at_the_ends() {
        let h = history(&[(10, 0.0, 0), (12, 2.0, 100), (14, 6.0, 200)]);
        assert_eq!(blend(&h, 11.0).players[0].1[0], 1.0);
        assert_eq!(blend(&h, 13.0).players[0].1[0], 4.0);
        assert_eq!(blend(&h, 11.0).players[0].2.0, 50);
        assert_eq!(blend(&h, 5.0).players[0].1[0], 0.0);
        assert_eq!(blend(&h, 99.0).players[0].1[0], 6.0);
    }

    #[test]
    fn characters_and_grenades_blend_too() {
        let h = history(&[(10, 0.0, 0), (12, 2.0, 100)]);
        let f = blend(&h, 11.0);
        assert_eq!(f.npcs[0], ([1.0, 0.0, 0.0], 50.0));
        assert_eq!(f.grenades[0], (2, [1.0, 1.0, 0.0], 10.0));
        // A grenade with nothing earlier to match is where it is; one that has gone is not drawn
        let mut h = h;
        h[1].1.grenades.push((2, [9.0; 3], 0.0));
        assert_eq!(blend(&h, 11.0).grenades[1], (2, [9.0; 3], 0.0));
        h[1].1.grenades.clear();
        assert!(blend(&h, 11.0).grenades.is_empty());
    }

    #[test]
    fn turns_the_short_way_round() {
        assert_eq!(turn_between(65000, 500, 0.5), 65518);
        assert_eq!(turn_between(500, 65000, 0.5), 65518);
        assert_eq!(degrees_between(350.0, 10.0, 0.5), 360.0);
        assert_eq!(degrees_between(10.0, 350.0, 0.5), 0.0);
    }
}
