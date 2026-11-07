//! The server: it owns the game. Clients send what their players do; the server runs the sim
//! with those inputs, a tick at a time, and sends back snapshots of the result.
//!
//! It is driven by the time it is given and the link it is given, never a clock or a socket of
//! its own, so a test can run it against a pretend network.

use std::collections::{BTreeMap, VecDeque};

use crate::net::link::{Addr, Link};
use crate::net::proto::{LobbyCommand, RoundResults, Session, Setup, Snapshot, ToClient, ToServer, chat_text, player_name, round_results, split};
use crate::net::record::{Change, ServerRecorder, ServerRecording};
use crate::net::round::{Outcome, Phase, RoundState, judge, teams};
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

/// How long (seconds) the countdown to a round runs, as in the original
pub const COUNTDOWN: f32 = 5.0;

/// How long (seconds) a player has to load the round before they are dropped
pub const LOAD_TIMEOUT: f32 = 30.0;

/// How long (seconds) everyone waits, loaded, before they are let go
pub const RELEASE_AFTER: f32 = 2.0;

/// How long (seconds) the round's end is seen in the level, before the results screen
pub const RESULTS: f32 = 3.0;

/// How long (seconds) the results screen waits for everyone to continue
pub const DEBRIEF_WAIT: f32 = 30.0;

/// A round's time limit (seconds) unless told otherwise: the original's 3:00
pub const ROUND_TIME: f32 = 180.0;

/// How long (seconds) a hosted game waits for its host to join before it ends
pub const HOST_WAIT: f64 = 30.0;

/// What a message in parts is numbered when it is an `Accept`, so that its parts are not
/// taken for a snapshot's
const ACCEPT_PARTS: u32 = u32::MAX;

/// A player in a round, as the server keeps it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seat {
    pub slot: usize,
    pub side: Option<Side>,
    /// Whether they were still there when the round was decided
    pub present: bool,
}

/// How a round went: its result, why, and who played it. A round called off keeps who was
/// there and who was dropped, so a win by abandonment can be judged later.
#[derive(Clone, Debug, PartialEq)]
pub struct RoundRecord {
    pub number: u16,
    pub outcome: Outcome,
    pub why: String,
    pub seats: Vec<Seat>,
}

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
    /// The round the client's game is in (`ToServer::Input::round`)
    round: u16,
    /// Joined with the host key
    host: bool,
    /// Said ready in the lobby
    ready: bool,
    /// The number of the latest lobby command taken, refused or not
    lobby_seq: u32,
    /// The occupant's join number (`PlayerState::joined`), and their name (`player_name`)
    joined: u32,
    name: String,
    /// Done with the round's results
    continued: bool,
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
    /// Where the rounds are, and when (tick) the phase ends by the clock
    round: RoundState,
    until: Option<u32>,
    /// A round's time limit (seconds), or none
    time_limit: Option<f32>,
    /// Start the countdown whenever there are two sides to play, as a host would
    auto_start: bool,
    /// Who is playing the round under way
    seats: Vec<Seat>,
    /// Every round played
    pub records: Vec<RoundRecord>,
    /// Lines for the server's log from the rounds
    news: Vec<Event>,
    /// What players have said since the last tick, to go out with the next one's events
    chat: Vec<Event>,
    /// The key that makes whoever joins with it the host, if there is one. A hosted game is
    /// the host's: it ends without them.
    host_key: Option<u64>,
    hosted: bool,
    /// Whether the host has joined yet, and when (seconds) the server began
    host_came: bool,
    began: Option<f64>,
    /// How long (seconds) a hosted game waits for its host (`HOST_WAIT`)
    host_wait: f64,
    /// Why the game is over, once it is
    ended: Option<String>,
    /// Counts the times the phase has entered Lobby or Countdown (`Session::generation`)
    generation: u32,
    /// Which of the setup's base pairs the sides start from
    base_pair: u8,
    /// The session as last recorded
    recorded: Option<Session>,
    /// The last round's results, from its end until the lobby
    results: Option<RoundResults>,
}

impl Server {
    pub fn new(sim: Sim, setup: Setup, build: &str, spawns: Vec<V3>) -> Server {
        assert!(!spawns.is_empty(), "players need somewhere to start");
        let mut sim = sim;
        sim.spawns = spawns.clone();
        let base_pair = setup.base_pairs.iter().position(|p| p.bases == setup.bases).unwrap_or(0) as u8;
        Server {
            sim, setup, build: build.to_string(), spawns, spawned: 0, remotes: Vec::new(), tick: 0, next_tick_at: None, events: VecDeque::new(), next_event: 1,
            snapshot_every: SNAPSHOT_EVERY, lag_compensation: true, max_rewind: MAX_REWIND / TICK, snapshots_sent: 0, snapshot_bytes: 0, snapshots_split: 0, recorder: None,
            round: RoundState::default(), until: None, time_limit: Some(ROUND_TIME), auto_start: false, seats: Vec::new(), records: Vec::new(), news: Vec::new(), chat: Vec::new(),
            host_key: None, hosted: false, host_came: false, began: None, host_wait: HOST_WAIT, ended: None, generation: 0, base_pair, recorded: None,
            results: None,
        }
    }

    /// Whoever joins with `key` is the host, unless it is 0. With `hosted`, the game is theirs:
    /// it ends when they leave, or if they have not come `HOST_WAIT` seconds after it began.
    pub fn set_host_key(&mut self, key: u64, hosted: bool) {
        self.host_key = (key != 0).then_some(key);
        self.hosted = hosted && key != 0;
    }

    /// Wait this long (seconds) for the host of a hosted game, not `HOST_WAIT`
    pub fn set_host_wait(&mut self, seconds: f64) {
        self.host_wait = seconds;
    }

    /// Why the game is over, once it is: the server has sent everyone away
    pub fn ended(&self) -> Option<&str> {
        self.ended.as_deref()
    }

    /// The session's settings, as every snapshot says
    pub fn session(&self) -> Session {
        Session {
            generation: self.generation,
            host: self.remotes.iter().find(|r| r.host).map(|r| r.slot as u8),
            time_limit: self.time_limit.map(|t| t.round().clamp(1.0, u16::MAX as f32) as u16),
            friendly_fire: self.sim.friendly_fire,
            base_pair: self.base_pair,
        }
    }

    /// Play in rounds, each player with one life a round, from the lobby. Before anyone joins,
    /// and before `record`.
    pub fn play_rounds(&mut self) {
        assert!(self.remotes.is_empty() && self.recorder.is_none(), "rounds are set before the game begins");
        self.setup.respawn = false;
        self.sim.respawn = false;
        self.set_phase(Phase::Lobby, None);
    }

    /// Where the rounds are
    pub fn round(&self) -> RoundState {
        self.round
    }

    /// Start the countdown whenever there are two sides to play, and stop it if one empties
    pub fn set_auto_start(&mut self, on: bool) {
        self.auto_start = on;
    }

    /// Change the time limit (seconds, or none). Only in the lobby: once the countdown starts,
    /// the session is locked. Says whether it changed.
    pub fn set_time_limit(&mut self, seconds: Option<f32>) -> bool {
        let open = matches!(self.round.phase, Phase::Open | Phase::Lobby);
        if open {
            self.time_limit = seconds;
        }
        open
    }

    /// Turn friendly fire on or off, only in the lobby as `set_time_limit`
    pub fn set_friendly_fire(&mut self, on: bool) -> bool {
        let open = matches!(self.round.phase, Phase::Open | Phase::Lobby);
        if open && self.sim.friendly_fire != on {
            self.setup.friendly_fire = on;
            self.sim.friendly_fire = on;
            if let Some(rec) = self.recorder.as_mut() {
                rec.changed(Change::FriendlyFire(on));
            }
        }
        open
    }

    /// Start the sides from pair `i` of the setup's base pairs, from the next round. Only in the
    /// lobby, as `set_time_limit`.
    pub fn set_base_pair(&mut self, i: u8) -> bool {
        let open = matches!(self.round.phase, Phase::Open | Phase::Lobby);
        let Some(pair) = self.setup.base_pairs.get(i as usize).filter(|_| open) else { return false };
        if i != self.base_pair {
            self.setup.bases = pair.bases.clone();
            self.sim.bases = pair.bases.clone();
            self.base_pair = i;
            if let Some(rec) = self.recorder.as_mut() {
                rec.changed(Change::Bases(i));
            }
        }
        true
    }

    /// Start the countdown to the next round, from the lobby
    pub fn start_countdown(&mut self) -> bool {
        if self.round.phase != Phase::Lobby {
            return false;
        }
        self.say(format!("starting round {} in {COUNTDOWN:.0} seconds", self.round.number + 1));
        self.set_phase(Phase::Countdown, Some(COUNTDOWN));
        true
    }

    /// Stop the countdown, back to the lobby
    pub fn stop_countdown(&mut self) -> bool {
        if self.round.phase != Phase::Countdown {
            return false;
        }
        self.say("the countdown was stopped".to_string());
        self.set_phase(Phase::Lobby, None);
        true
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
        let mut rec = ServerRecorder::new(&self.build, self.setup.clone(), self.spawns.clone());
        // Waiting in the lobby already
        if self.sim.frozen {
            rec.changed(Change::Frozen(true));
        }
        self.recorder = Some(rec);
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
        if self.ended.is_some() {
            return std::mem::take(&mut self.news);
        }
        self.began.get_or_insert(now);
        while let Some((from, bytes)) = link.recv() {
            if let Ok(msg) = ToServer::decode(&bytes) {
                self.receive(now, from, msg, link);
            }
        }

        let mut log = std::mem::take(&mut self.news);
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
            ToServer::Connect { token, build, side: wish, key, name } => {
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
                // Their name, unlike any other here
                let taken: Vec<&str> = self.remotes.iter().map(|r| r.name.as_str()).collect();
                let name = player_name(&name, slot, &taken);
                let who = self.sim.occupant(slot);
                self.sim.names.push((who, name.clone()));
                if let Some(rec) = self.recorder.as_mut() {
                    rec.joined(slot, spawn, side, &name);
                }
                self.say(format!("player {slot} is {name}"));
                // Too late for the round under way: they wait for the next
                if matches!(self.round.phase, Phase::Loading | Phase::Hold | Phase::Live | Phase::Results | Phase::Debrief) {
                    self.sim.sit_out(slot);
                    if let Some(rec) = self.recorder.as_mut() {
                        rec.changed(Change::SatOut { slot: slot as u8 });
                    }
                }
                // One host at a time, and never without a key
                let host = self.host_key.is_some_and(|k| k == key) && !self.remotes.iter().any(|r| r.host);
                if host {
                    self.host_came = true;
                    self.say(format!("player {slot} is the host"));
                }
                self.remotes.push(Remote {
                    addr: from, token, slot, queue: BTreeMap::new(), next_seq: None, last_input: Input { run: true, ..Input::default() }, starved: 0, last_view: 0, low: usize::MAX, looked: 0,
                    applied_seq: 0, event_ack: self.next_event - 1, in_flight: VecDeque::new(), heard: now, acked: 0, sent: VecDeque::new(), round: 0,
                    host, ready: false, lobby_seq: 0, joined: who.joined, name, continued: false,
                });
                self.accept(link, from, token, slot);
            }
            ToServer::Lobby { token, seq, generation, command } => {
                let Some(i) = self.remotes.iter().position(|r| r.addr == from && r.token == token) else { return };
                self.remotes[i].heard = now;
                // Each is taken once, in order: the next after the last, and no other
                if self.remotes[i].lobby_seq.checked_add(1) != Some(seq) {
                    return;
                }
                self.remotes[i].lobby_seq = seq;
                let slot = self.remotes[i].slot;
                let took = self.lobby(i, generation, command.clone());
                let refused = if took { "" } else { ", refused" };
                self.say(format!("player {slot} asked for {command:?}{refused}"));
            }
            ToServer::Input { token, ack_snapshot, first_seq, view, round, inputs } => {
                let Some(r) = self.remotes.iter_mut().find(|r| r.addr == from && r.token == token) else { return };
                r.heard = now;
                r.round = round;
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

    /// Do what a player asks for in the lobby, if they may. Says whether it was done.
    fn lobby(&mut self, i: usize, generation: u32, command: LobbyCommand) -> bool {
        let (slot, host) = (self.remotes[i].slot, self.remotes[i].host);
        let lobby = self.round.phase == Phase::Lobby;
        // Made for another lobby, countdown or results than these. What is said is said whenever.
        if generation != self.generation && !matches!(command, LobbyCommand::Say(_)) {
            return false;
        }
        let debrief = self.round.phase == Phase::Debrief;
        match command {
            // Not while a round is played: players use their voices then
            LobbyCommand::Say(_) if self.round.phase == Phase::Live => false,
            LobbyCommand::Say(text) => match chat_text(&text) {
                Some(text) => {
                    let dead = self.in_round() && self.sim.players[slot].respawn_at.is_some();
                    let name = self.sim.name_of(self.sim.occupant(slot));
                    self.chat.push(Event::Chat { player: slot, name, text, dead });
                    true
                }
                None => false,
            },
            LobbyCommand::Continue => {
                let first = debrief && !self.remotes[i].continued;
                self.remotes[i].continued |= first;
                first
            }
            LobbyCommand::ToLobby if host => {
                let ready = debrief && self.remotes[i].continued;
                if ready {
                    self.say("the host took everyone to the lobby".to_string());
                    self.set_phase(Phase::Lobby, None);
                }
                ready
            }
            LobbyCommand::Side(side) => {
                if !lobby || self.sim.bases.iter().any(Vec::is_empty) {
                    return false;
                }
                if self.sim.players[slot].side != Some(side) {
                    self.sim.players[slot].side = Some(side);
                    self.remotes[i].ready = false;
                    if let Some(rec) = self.recorder.as_mut() {
                        rec.changed(Change::Side { slot: slot as u8, side: Some(side) });
                    }
                }
                true
            }
            LobbyCommand::Ready(on) => {
                if lobby {
                    self.remotes[i].ready = on;
                }
                lobby
            }
            LobbyCommand::Start if host => lobby && teams(&self.sim, false).len() >= 2 && self.start_countdown(),
            LobbyCommand::Stop if host => self.stop_countdown(),
            LobbyCommand::TimeLimit(t) if host => lobby && self.set_time_limit(t.map(f32::from)),
            LobbyCommand::FriendlyFire(on) if host => lobby && self.set_friendly_fire(on),
            LobbyCommand::BasePair(n) if host => lobby && self.set_base_pair(n),
            _ => false,
        }
    }

    /// Whether a round is being played, from its loading to its end
    fn in_round(&self) -> bool {
        matches!(self.round.phase, Phase::Loading | Phase::Hold | Phase::Live)
    }

    /// The game is over: everyone is sent away, with why (`ended`)
    fn end(&mut self, why: &str, link: &mut impl Link) {
        for r in std::mem::take(&mut self.remotes) {
            link.send(r.addr, &ToClient::Goodbye { token: r.token, reason: why.to_string() }.encode().0);
            self.drop_player(r.slot);
        }
        self.ended = Some(why.to_string());
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

    /// A player leaves. In a round they count as dead from now.
    fn drop_player(&mut self, slot: usize) {
        self.sim.remove_player(slot);
        if let Some(rec) = self.recorder.as_mut() {
            rec.left(slot);
        }
        for seat in self.seats.iter_mut().filter(|s| s.slot == slot) {
            seat.present = false;
        }
    }

    fn say(&mut self, line: String) {
        self.news.push(Event::Log(format!("t={:.2} {line}", self.sim.time)));
    }

    /// Move to `phase`, for `seconds` if the clock ends it. Players are held unless the round
    /// is being played.
    fn set_phase(&mut self, phase: Phase, seconds: Option<f32>) {
        self.round.phase = phase;
        // A new lobby, countdown or results screen: commands made for the last are stale
        if matches!(phase, Phase::Lobby | Phase::Countdown | Phase::Debrief) {
            match self.generation.checked_add(1) {
                Some(g) => self.generation = g,
                None => self.ended = Some("the session ran out of numbers".to_string()),
            }
        }
        // The lobby has no results, and asks everyone to continue again next time
        if phase == Phase::Lobby {
            self.results = None;
            for r in &mut self.remotes {
                r.continued = false;
            }
        }
        self.until = seconds.map(|s| self.tick + (s / TICK).round() as u32);
        self.round.ends = seconds.map(|s| self.sim.time + s);
        if self.sim.frozen != phase.frozen() {
            self.sim.frozen = phase.frozen();
            if let Some(rec) = self.recorder.as_mut() {
                rec.changed(Change::Frozen(phase.frozen()));
            }
        }
    }

    fn timed_out(&self) -> bool {
        self.until.is_some_and(|t| self.tick >= t)
    }

    /// The rounds move on, once a tick
    fn run_rounds(&mut self, link: &mut impl Link) {
        let sides = teams(&self.sim, false).len();
        match self.round.phase {
            Phase::Open => {}
            Phase::Lobby => {
                if self.auto_start && sides >= 2 {
                    self.start_countdown();
                }
            }
            Phase::Countdown if self.auto_start && sides < 2 => {
                self.stop_countdown();
            }
            Phase::Countdown => {
                if self.timed_out() {
                    self.begin_round();
                }
            }
            Phase::Loading => {
                let number = self.round.number;
                let seated = |r: &Remote, seats: &[Seat]| seats.iter().any(|s| s.slot == r.slot && s.present);
                if self.timed_out() {
                    // Those still loading are dropped, and the round goes on without them
                    let slow: Vec<usize> = (0..self.remotes.len()).rev().filter(|&i| seated(&self.remotes[i], &self.seats) && self.remotes[i].round != number).collect();
                    for i in slow {
                        let r = self.remotes.remove(i);
                        link.send(r.addr, &ToClient::Goodbye { token: r.token, reason: "you took too long to load the round".to_string() }.encode().0);
                        self.say(format!("player {} took too long to load and was dropped", r.slot));
                        self.drop_player(r.slot);
                    }
                }
                if self.remotes.iter().all(|r| !seated(r, &self.seats) || r.round == number) {
                    self.set_phase(Phase::Hold, Some(RELEASE_AFTER));
                }
            }
            Phase::Hold => {
                if self.timed_out() {
                    self.release();
                }
            }
            Phase::Live => {
                if self.timed_out() {
                    self.end_round(Outcome::Draw, "the time ran out".to_string());
                } else if let Some(outcome) = judge(&self.sim) {
                    let why = match outcome {
                        Outcome::Won(s) => format!("{s:?} is the last side standing"),
                        Outcome::Survived(p) => format!("player {p} is the last one standing"),
                        _ => "no one is left".to_string(),
                    };
                    self.end_round(outcome, why);
                }
            }
            Phase::Results => {
                if self.timed_out() {
                    self.set_phase(Phase::Debrief, Some(DEBRIEF_WAIT));
                }
            }
            Phase::Debrief => {
                // Waiting for those who played the round and are still here
                let waited = |r: &Remote| self.sim.round.iter().any(|p| !p.left && p.who.slot == r.slot && p.who.joined == r.joined);
                let done = self.remotes.iter().filter(|r| waited(r)).all(|r| r.continued);
                if done || self.timed_out() {
                    self.say(if done { "everyone has continued" } else { "not everyone continued in time" }.to_string());
                    self.set_phase(Phase::Lobby, None);
                }
            }
        }
    }

    /// A new round: everyone back at the start, held until they have it
    fn begin_round(&mut self) {
        self.round.number += 1;
        let seed = self.setup.seed ^ (self.round.number as u64).wrapping_mul(0x9e3779b97f4a7c15);
        self.sim.new_round(seed);
        if let Some(rec) = self.recorder.as_mut() {
            rec.changed(Change::Round { seed });
        }
        self.seats = self.sim.players.iter().enumerate().filter(|(_, p)| p.active).map(|(slot, p)| Seat { slot, side: p.side, present: true }).collect();
        // Each round asks again
        for r in &mut self.remotes {
            r.ready = false;
        }
        self.say(format!("round {} is loading", self.round.number));
        self.set_phase(Phase::Loading, Some(LOAD_TIMEOUT));
    }

    /// Everyone has the round: let them go, unless a side has no one left to play it
    fn release(&mut self) {
        if teams(&self.sim, false).len() < 2 {
            let why = match self.sim.bases.iter().all(|b| !b.is_empty()) {
                true => Side::BOTH.iter().filter(|&&s| !self.seats.iter().any(|seat| seat.present && seat.side == Some(s))).map(|s| format!("{s:?}")).collect::<Vec<_>>().join(" and ") + " had no one",
                false => "there was no one to play against".to_string(),
            };
            self.end_round(Outcome::CalledOff, why);
            return;
        }
        self.say(format!("round {} is under way", self.round.number));
        self.set_phase(Phase::Live, self.time_limit);
        // What players do counts from now
        self.sim.open_round();
        if let Some(rec) = self.recorder.as_mut() {
            rec.changed(Change::Counting(true));
        }
    }

    fn end_round(&mut self, outcome: Outcome, why: String) {
        if let Outcome::Won(s) = outcome {
            self.round.wins[s.index()] += 1;
        }
        self.round.last = Some(outcome);
        let verdict = match outcome {
            Outcome::Won(s) => format!("{s:?} wins"),
            Outcome::Survived(p) => format!("player {p} wins"),
            Outcome::Draw => "everybody lost".to_string(),
            Outcome::CalledOff => "it is called off".to_string(),
        };
        self.say(format!("round {}: {verdict}, since {why}", self.round.number));
        self.records.push(RoundRecord { number: self.round.number, outcome, why, seats: self.seats.clone() });
        // Nothing counts from now, and the results are made once
        self.sim.close_round();
        let results = round_results(&self.sim, self.round.number, outcome);
        if let Some(rec) = self.recorder.as_mut() {
            rec.changed(Change::Counting(false));
            rec.changed(Change::Results(results.clone()));
        }
        self.results = Some(results);
        self.set_phase(Phase::Results, Some(RESULTS));
    }

    /// The last round's results, while there are any
    pub fn results(&self) -> Option<&RoundResults> {
        self.results.as_ref()
    }

    fn accept(&self, link: &mut impl Link, to: Addr, token: u32, slot: usize) {
        let msg = ToClient::Accept { token, slot: slot as u8, tick: self.tick, setup: self.setup.clone() };
        // A level with many pairs of bases can make it too big for one datagram
        for d in split(ACCEPT_PARTS, msg.encode().0) {
            link.send(to, &d);
        }
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
        let mut events = self.sim.tick(&inputs, TICK);
        events.append(&mut self.chat);
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
        self.run_rounds(link);
        // A hosted game is over when its host has gone, or never came
        if self.hosted && self.ended.is_none() {
            if self.host_came && !self.remotes.iter().any(|r| r.host) {
                self.end("the host left", link);
            } else if !self.host_came && self.began.is_some_and(|t| now - t > self.host_wait) {
                self.end("the host never came", link);
            }
        }
        let session = self.session();
        if let Some(rec) = self.recorder.as_mut().filter(|_| self.recorded != Some(session)) {
            rec.changed(Change::Session(session));
            self.recorded = Some(session);
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
        snap.round = self.round;
        snap.session = self.session();
        snap.lobby_ack = r.lobby_seq;
        snap.results = self.results.clone();
        for p in &mut snap.players {
            p.ready = self.remotes.iter().any(|q| q.slot == p.slot as usize && q.ready);
            p.continued = self.remotes.iter().any(|q| q.slot == p.slot as usize && q.continued);
            p.name = self.sim.name_of(crate::sim::Occupant { slot: p.slot as usize, joined: p.joined });
        }
        let stored = snap.clone();
        // The living in a round do not hear the dead
        let deaf = self.in_round() && self.sim.players[r.slot].respawn_at.is_none();
        snap.events = self.events.iter()
            .filter(|(_, id, e)| *id > r.event_ack && !(deaf && matches!(e, Event::Chat { dead: true, .. })))
            .map(|(_, id, e)| (*id, e.clone()))
            .collect();
        let ids: Vec<u32> = snap.events.iter().map(|(id, _)| *id).collect();
        // What the client is known to have is what this one is told as the difference from,
        // if it is of the same round: a new round puts back what only ever changes one way
        let base = r.sent.iter().find(|s| r.acked > 0 && s.tick == r.acked && s.round.number == snap.round.number);
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
