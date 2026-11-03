//! Recorded network games.
//!
//! The server's game is deterministic, so a match is fully described by how it was set up, who
//! joined and left between which ticks, and what each player did every tick. Playing that back
//! gives the whole match again, every player's side of it.
//!
//! A client's recording is what one player saw: every datagram the server sent it, with when it
//! came, and what the player did. Playing that back through the client code gives what was on
//! their screen. A tester can send theirs, and marks they made show where to look.
//!
//! Both keep a checksum of the game every second, as single-player recordings do, so playing
//! back can tell the moment it stops matching.

use std::collections::VecDeque;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

use crate::bytes::{Before, Reader, Writer};
use crate::net::client::{Client, Frame};
use crate::net::link::{Addr, Link};
use crate::net::proto::{Setup, read_setup, side_code, side_from, write_setup};
use crate::net::setup::build_sim;
use crate::recording::{CHECK_EVERY, Desync};
use crate::sim::{Input, Side, Sim, TICK};

type V3 = [f32; 3];

const SERVER_MAGIC: &[u8; 8] = b"RRSERV\0\x02";
const CLIENT_MAGIC: &[u8; 8] = b"RRCLNT\0\x01";

/// A change in who plays, between two ticks
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Joined { slot: u8, spawn: V3, side: Option<Side> },
    Left { slot: u8 },
}

/// One tick of the server's game
#[derive(Clone, Debug, PartialEq)]
pub struct ServerTick {
    /// Who joined and left before it ran
    pub changes: Vec<Change>,
    /// Each slot's input, and how far back its shots were judged (`Sim::rewind`)
    pub inputs: Vec<Input>,
    pub rewind: Vec<f32>,
}

/// A whole match, as the server ran it
#[derive(Clone, Debug, PartialEq)]
pub struct ServerRecording {
    pub build: String,
    pub setup: Setup,
    /// Where players start, in turn
    pub spawns: Vec<V3>,
    pub ticks: Vec<ServerTick>,
    /// (ticks run, checksum of the game after them)
    pub checks: Vec<(u32, u64)>,
}

impl ServerRecording {
    /// The server's game as the match began
    pub fn start(&self) -> Result<Sim> {
        let mut sim = build_sim(&self.setup)?;
        sim.spawns = self.spawns.clone();
        Ok(sim)
    }

    /// Run tick `n` of the match
    pub fn step(&self, sim: &mut Sim, n: usize) -> Result<()> {
        let t = &self.ticks[n];
        for change in &t.changes {
            match *change {
                Change::Joined { slot, spawn, side } => {
                    let got = sim.add_player(spawn);
                    ensure!(got == Some(slot as usize), "player {slot} joined at tick {n}, but the game put them in {got:?}");
                    sim.players[slot as usize].side = side;
                }
                Change::Left { slot } => sim.remove_player(slot as usize),
            }
        }
        sim.rewind = t.rewind.clone();
        sim.tick(&t.inputs, TICK);
        Ok(())
    }

    /// Play the whole match. Fails with a `Desync` where it first stops matching.
    pub fn replay(&self) -> Result<Sim> {
        let mut sim = self.start()?;
        let mut checks = self.checks.iter().peekable();
        for n in 0..self.ticks.len() {
            self.step(&mut sim, n)?;
            check(&mut checks, n as u32 + 1, &sim)?;
        }
        Ok(sim)
    }

    pub fn seconds(&self) -> f32 {
        self.ticks.len() as f32 * TICK
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.0.extend_from_slice(SERVER_MAGIC);
        w.string(&self.build);
        write_setup(&mut w, &self.setup);
        w.u32(self.spawns.len() as u32);
        for &s in &self.spawns {
            w.v3(s);
        }
        w.u32(self.ticks.len() as u32);
        let mut befores: Vec<Before> = Vec::new();
        let mut rewound: Vec<f32> = Vec::new();
        for t in &self.ticks {
            w.varint(t.changes.len() as u32);
            for change in &t.changes {
                match *change {
                    Change::Joined { slot, spawn, side } => {
                        w.u8(0);
                        w.u8(slot);
                        w.v3(spawn);
                        w.u8(side_code(side));
                    }
                    Change::Left { slot } => {
                        w.u8(1);
                        w.u8(slot);
                    }
                }
            }
            let n = t.inputs.len();
            w.u8(n as u8);
            befores.resize_with(n.max(befores.len()), Before::default);
            for (input, before) in t.inputs.iter().zip(befores.iter_mut()) {
                w.input(input, before);
            }
            // A rewind stays the same while the client's connection does, so only changes are kept
            rewound.resize(n.max(rewound.len()), 0.0);
            let changed: Vec<bool> = t.rewind.iter().zip(&rewound).map(|(a, b)| a.to_bits() != b.to_bits()).collect();
            for chunk in changed.chunks(8) {
                w.u8(chunk.iter().enumerate().fold(0, |acc, (i, &c)| acc | (c as u8) << i));
            }
            for (i, &r) in t.rewind.iter().enumerate().filter(|&(i, _)| changed[i]) {
                w.f32(r);
                rewound[i] = r;
            }
        }
        write_checks(&mut w, &self.checks);
        w.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<ServerRecording> {
        let mut r = Reader::new(bytes);
        ensure!(r.take(SERVER_MAGIC.len())? == SERVER_MAGIC, "not a server recording (or one from another version)");
        let build = r.string()?;
        let setup = read_setup(&mut r)?;
        let spawns = (0..r.u32()?).map(|_| r.v3()).collect::<Result<Vec<_>>>()?;
        let n = r.u32()? as usize;
        let mut ticks = Vec::with_capacity(n.min(1 << 20));
        let mut befores: Vec<Before> = Vec::new();
        let mut rewound: Vec<f32> = Vec::new();
        for _ in 0..n {
            let mut changes = Vec::new();
            for _ in 0..r.varint()? {
                changes.push(match r.u8()? {
                    0 => Change::Joined { slot: r.u8()?, spawn: r.v3()?, side: side_from(r.u8()?)? },
                    1 => Change::Left { slot: r.u8()? },
                    k => bail!("no such change as {k}"),
                });
            }
            let slots = r.u8()? as usize;
            befores.resize_with(slots.max(befores.len()), Before::default);
            let inputs = befores[..slots].iter_mut().map(|b| r.input(b)).collect::<Result<Vec<_>>>()?;
            rewound.resize(slots.max(rewound.len()), 0.0);
            let mut changed = Vec::with_capacity(slots);
            for _ in 0..slots.div_ceil(8) {
                let byte = r.u8()?;
                changed.extend((0..8).map(|i| byte >> i & 1 != 0));
            }
            for i in 0..slots {
                if changed[i] {
                    rewound[i] = r.f32()?;
                }
            }
            ticks.push(ServerTick { changes, inputs, rewind: rewound[..slots].to_vec() });
        }
        let checks = read_checks(&mut r)?;
        ensure!(r.left() == 0, "{} bytes left over after the recording", r.left());
        Ok(ServerRecording { build, setup, spawns, ticks, checks })
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_bytes()).with_context(|| format!("could not write {}", path.display()))
    }

    pub fn read(path: &Path) -> Result<ServerRecording> {
        let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
        ServerRecording::from_bytes(&bytes).with_context(|| format!("{}", path.display()))
    }
}

/// Makes a server's recording as the match is played
pub struct ServerRecorder {
    rec: ServerRecording,
    changes: Vec<Change>,
}

impl ServerRecorder {
    pub fn new(build: &str, setup: Setup, spawns: Vec<V3>) -> ServerRecorder {
        ServerRecorder { rec: ServerRecording { build: build.to_string(), setup, spawns, ticks: Vec::new(), checks: Vec::new() }, changes: Vec::new() }
    }

    pub fn joined(&mut self, slot: usize, spawn: V3, side: Option<Side>) {
        self.changes.push(Change::Joined { slot: slot as u8, spawn, side });
    }

    pub fn left(&mut self, slot: usize) {
        self.changes.push(Change::Left { slot: slot as u8 });
    }

    /// Note a tick that has just run
    pub fn ticked(&mut self, inputs: &[Input], rewind: &[f32], sim: &Sim) {
        self.rec.ticks.push(ServerTick { changes: std::mem::take(&mut self.changes), inputs: inputs.to_vec(), rewind: rewind.to_vec() });
        let ticks = self.rec.ticks.len() as u32;
        if ticks % CHECK_EVERY == 0 {
            self.rec.checks.push((ticks, sim.checksum()));
        }
    }

    /// The recording so far, with a last checksum where it ends. The match can go on.
    pub fn recording(&self, sim: &Sim) -> ServerRecording {
        let mut rec = self.rec.clone();
        let ticks = rec.ticks.len() as u32;
        if ticks % CHECK_EVERY != 0 {
            rec.checks.push((ticks, sim.checksum()));
        }
        rec
    }
}

/// What a client was doing when datagrams came
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Call {
    /// Asking to join (`Client::connect`)
    Connect,
    /// A tick, with what the player did (`Client::tick`)
    Tick(Input),
    /// Drawing a frame (`Client::view`)
    View,
}

/// One call into the client, at a time, with the datagrams it read
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub now: f64,
    pub call: Call,
    pub received: Vec<Vec<u8>>,
}

/// What one player saw of a match
#[derive(Clone, Debug, PartialEq)]
pub struct ClientRecording {
    pub build: String,
    pub token: u32,
    /// Where the player stood before the server said, and whether they took no harm (--god)
    pub start: V3,
    pub god: bool,
    pub steps: Vec<Step>,
    /// (ticks run, checksum of the client's game after them), from 0, as it was set up
    pub checks: Vec<(u32, u64)>,
    /// Moments the player marked: the tick, and the time
    pub marks: Vec<(u32, f64)>,
}

/// The address a replayed client takes its server to have
const SERVER: Addr = 1;

/// A link that hands over the datagrams it is given, as from the server, and sends nothing
#[derive(Default)]
struct Tape {
    incoming: VecDeque<Vec<u8>>,
}

impl Link for Tape {
    fn send(&mut self, _to: Addr, _bytes: &[u8]) {}

    fn recv(&mut self) -> Option<(Addr, Vec<u8>)> {
        self.incoming.pop_front().map(|b| (SERVER, b))
    }
}

impl ClientRecording {
    /// Play it back through the client code. `viewed` is given each frame as it was shown: the
    /// time, what moved, the client's game and the player's slot. Fails with a `Desync` where
    /// the client's game first stops matching.
    pub fn replay(&self, mut viewed: impl FnMut(f64, &Frame, &Sim, usize)) -> Result<Sim> {
        let mut client = Client::new(SERVER, self.token, &self.build);
        let mut link = Tape::default();
        let mut sim: Option<Sim> = None;
        let mut ticks = 0u32;
        let mut checks = self.checks.iter().peekable();
        for step in &self.steps {
            link.incoming = step.received.iter().cloned().collect();
            match step.call {
                Call::Connect => {
                    if let Some(setup) = client.connect(step.now, &mut link).cloned() {
                        if sim.is_none() {
                            let mut s = build_sim(&setup)?;
                            s.activate_slot(client.slot(), self.start);
                            s.god = self.god;
                            check(&mut checks, 0, &s)?;
                            sim = Some(s);
                        }
                    }
                }
                Call::Tick(input) => {
                    let Some(sim) = sim.as_mut() else { bail!("a tick before the server let the player in") };
                    client.tick(step.now, &mut link, sim, input);
                    ticks += 1;
                    check(&mut checks, ticks, sim)?;
                }
                Call::View => {
                    let frame = client.view(step.now);
                    if let Some(sim) = &sim {
                        viewed(step.now, &frame, sim, client.slot());
                    }
                }
            }
        }
        sim.context("the server never let the player in")
    }

    pub fn seconds(&self) -> f64 {
        match (self.steps.first(), self.steps.last()) {
            (Some(a), Some(b)) => b.now - a.now,
            _ => 0.0,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.0.extend_from_slice(CLIENT_MAGIC);
        w.string(&self.build);
        w.u32(self.token);
        w.v3(self.start);
        w.bool(self.god);
        w.u32(self.steps.len() as u32);
        let mut before = Before::default();
        for step in &self.steps {
            match step.call {
                Call::Connect => w.u8(0),
                Call::Tick(input) => {
                    w.u8(1);
                    w.input(&input, &mut before);
                }
                Call::View => w.u8(2),
            }
            w.u64(step.now.to_bits());
            w.varint(step.received.len() as u32);
            for d in &step.received {
                w.varint(d.len() as u32);
                w.0.extend_from_slice(d);
            }
        }
        write_checks(&mut w, &self.checks);
        w.u32(self.marks.len() as u32);
        for &(tick, now) in &self.marks {
            w.u32(tick);
            w.u64(now.to_bits());
        }
        w.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<ClientRecording> {
        let mut r = Reader::new(bytes);
        ensure!(r.take(CLIENT_MAGIC.len())? == CLIENT_MAGIC, "not a client recording (or one from another version)");
        let (build, token, start, god) = (r.string()?, r.u32()?, r.v3()?, r.bool()?);
        let n = r.u32()? as usize;
        let mut steps = Vec::with_capacity(n.min(1 << 20));
        let mut before = Before::default();
        for _ in 0..n {
            let call = match r.u8()? {
                0 => Call::Connect,
                1 => Call::Tick(r.input(&mut before)?),
                2 => Call::View,
                k => bail!("no such call as {k}"),
            };
            let now = f64::from_bits(r.u64()?);
            let received = (0..r.varint()?).map(|_| {
                let len = r.varint()? as usize;
                Ok(r.take(len)?.to_vec())
            }).collect::<Result<Vec<_>>>()?;
            steps.push(Step { now, call, received });
        }
        let checks = read_checks(&mut r)?;
        let marks = (0..r.u32()?).map(|_| Ok((r.u32()?, f64::from_bits(r.u64()?)))).collect::<Result<Vec<_>>>()?;
        ensure!(r.left() == 0, "{} bytes left over after the recording", r.left());
        Ok(ClientRecording { build, token, start, god, steps, checks, marks })
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_bytes()).with_context(|| format!("could not write {}", path.display()))
    }

    pub fn read(path: &Path) -> Result<ClientRecording> {
        let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
        ClientRecording::from_bytes(&bytes).with_context(|| format!("{}", path.display()))
    }
}

/// Makes a client's recording as the player plays. The client must keep what it receives
/// (`Client::start_tape`) from before it asks to join.
pub struct ClientRecorder {
    rec: ClientRecording,
    ticks: u32,
}

impl ClientRecorder {
    pub fn new(build: &str, token: u32) -> ClientRecorder {
        ClientRecorder { rec: ClientRecording { build: build.to_string(), token, start: [0.0; 3], god: false, steps: Vec::new(), checks: Vec::new(), marks: Vec::new() }, ticks: 0 }
    }

    /// After `Client::connect` at `now`
    pub fn connected(&mut self, now: f64, client: &mut Client) {
        self.rec.steps.push(Step { now, call: Call::Connect, received: client.take_tape() });
    }

    /// The game is set up from what the server said: the player stands at `start` until the
    /// server says where they are
    pub fn began(&mut self, start: V3, god: bool, sim: &Sim) {
        (self.rec.start, self.rec.god) = (start, god);
        self.rec.checks.push((0, sim.checksum()));
    }

    /// After `Client::tick` at `now` with `input`
    pub fn ticked(&mut self, now: f64, input: Input, client: &mut Client, sim: &Sim) {
        self.rec.steps.push(Step { now, call: Call::Tick(input), received: client.take_tape() });
        self.ticks += 1;
        if self.ticks % CHECK_EVERY == 0 {
            self.rec.checks.push((self.ticks, sim.checksum()));
        }
    }

    /// After `Client::view` at `now`
    pub fn viewed(&mut self, now: f64) {
        self.rec.steps.push(Step { now, call: Call::View, received: Vec::new() });
    }

    /// The player marks this moment, to find it later
    pub fn mark(&mut self, now: f64) {
        self.rec.marks.push((self.ticks, now));
    }

    /// The recording so far, with a last checksum where it ends
    pub fn recording(&self, sim: &Sim) -> ClientRecording {
        let mut rec = self.rec.clone();
        if self.ticks % CHECK_EVERY != 0 {
            rec.checks.push((self.ticks, sim.checksum()));
        }
        rec
    }
}

fn write_checks(w: &mut Writer, checks: &[(u32, u64)]) {
    w.u32(checks.len() as u32);
    for &(tick, sum) in checks {
        w.u32(tick);
        w.u64(sum);
    }
}

fn read_checks(r: &mut Reader) -> Result<Vec<(u32, u64)>> {
    (0..r.u32()?).map(|_| Ok((r.u32()?, r.u64()?))).collect()
}

/// Compare the game after `ticks` with the recorded checksum for then, if there is one
fn check<'a>(checks: &mut std::iter::Peekable<impl Iterator<Item = &'a (u32, u64)>>, ticks: u32, sim: &Sim) -> Result<(), Desync> {
    while let Some(&&(tick, sum)) = checks.peek() {
        if tick > ticks {
            break;
        }
        checks.next();
        if tick == ticks && sum != sim.checksum() {
            return Err(Desync { tick, recorded: sum, found: sim.checksum() });
        }
    }
    Ok(())
}
