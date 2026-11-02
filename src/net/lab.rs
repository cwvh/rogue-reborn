//! The network lab: whole games played on a pretend network and a pretend clock, in a moment,
//! to measure how fair and smooth play is over given connections. `rrlab` prints its tables,
//! and the tests run their games the same way.
//!
//! What it measures, each player taken as on the LAN or over the internet:
//!
//! - Hit agreement. Each shot the server fired is checked twice with the same ray: against where
//!   the server judged the others to be, and against where the shooter's screen showed them when
//!   they pressed the trigger. Both use plain cylinders for bodies, so only the netcode differs.
//! - The gap: for each hit, how far the victim, on their own screen, was from where they were
//!   judged to be when the shot landed. Past a corner, that is a shot behind cover.
//! - Delays: from sending the input that fired until the shooter is told of the shot, and from a
//!   death on the server until the victim is told.
//! - Fixes and bytes, from each client's lag meter, and the server's upload.
//! - Peeker's advantage (`peek`): one player steps past a corner toward another who holds still.
//!   How much sooner the one moving sees the other than they are seen.

use std::collections::VecDeque;

use crate::net::bot::{Bot, Skill};
use crate::net::client::{Client, Frame};
use crate::net::link::{Conditions, FakeLink, FakeNet};
use crate::net::proto::Setup;
use crate::net::record::ClientRecorder;
use crate::net::server::Server;
use crate::net::setup::build_sim;
use crate::sim::{Event, Input, MAX_PLAYERS, Sim, TICK};

type V3 = [f32; 3];

pub const BUILD: &str = "test";

/// The box room with nobody in it
pub fn demo_setup() -> Setup {
    Setup {
        map: "demo".to_string(),
        model: String::new(),
        idle: "up_none_cyc".to_string(),
        magazine: 30,
        ammo: 30,
        reserve: 90,
        grenades: 3,
        speed_scale: 1.0,
        seed: 7,
        culling: true,
        pvp: false,
        doors_open: None,
        mission: None,
        npcs: Vec::new(),
    }
}

/// Where player `i` starts in the box room: in rows of eight along the south wall
pub fn spawn(i: usize) -> V3 {
    [-280.0 + (i % 8) as f32 * 80.0, 0.0, -350.0 + (i / 8) as f32 * 100.0]
}

/// Where a recorded player stands until the server says
pub const START: V3 = [0.0, 0.0, 300.0];

/// A player who plays from what their screen shows: their copy of the game, their slot, the
/// frame drawn, and the tick
pub type Sees = Box<dyn FnMut(&Sim, usize, &Frame, u32) -> Input>;

/// One player at a computer: a client, its copy of the game, and what the player does
pub struct Peer {
    pub client: Client,
    pub link: FakeLink,
    pub sim: Option<Sim>,
    pub script: Box<dyn FnMut(u32) -> Input>,
    /// A computer player, which plays instead of the script
    pub bot: Option<Bot>,
    /// Plays instead of the script, from what is on screen
    pub sees: Option<Sees>,
    /// Ticks this client has run
    pub ticks: u32,
    /// Whether the client is running: a client that is not is a computer that has been switched off
    pub on: bool,
    /// What the client has been told happened
    pub events: Vec<Event>,
    /// Recording what the player saw, and the frames they were shown
    pub recorder: Option<ClientRecorder>,
    pub frames: Vec<Frame>,
    /// What the screen shows now, when anything looks at it
    pub view: Option<Frame>,
    /// Over the internet rather than on the LAN
    pub remote: bool,
    /// For the lab: what was on screen as each of the latest inputs was sent
    shown: VecDeque<Shown>,
    /// For the lab: how many of their own shots and deaths they have been told of
    told_shots: usize,
    told_deaths: usize,
}

/// What a player's screen showed as they sent an input
struct Shown {
    seq: u32,
    sent: f64,
    eye: V3,
    /// The others: slot, feet, and whether crouched
    bodies: Vec<(usize, V3, bool)>,
}

/// How long what was on screen is kept, in ticks: longer than any input waits to be used
const SHOWN: usize = 240;

/// The figures for a player, or for players of a kind
#[derive(Clone, Debug, Default)]
pub struct Tally {
    /// Shots compared, and those the server and the shooter's screen agree on
    pub shots: usize,
    pub agree: usize,
    /// Hits on the shooter's screen, and of those, the ones the server gave them
    pub seen_hits: usize,
    pub honored: usize,
    /// Hits the server gave, and of those, the ones the shooter's screen showed as misses
    pub server_hits: usize,
    pub unseen: usize,
    /// For each hit on this player: how far they were, on their own screen, from where they
    /// were judged to be
    pub gaps: Vec<f32>,
    /// Hits on this player while they were past the corner on their own screen (`peek`)
    pub behind: usize,
    /// Seconds from sending the input that fired to being told of the shot
    pub shot_delays: Vec<f32>,
    /// Seconds from dying on the server to being told
    pub death_delays: Vec<f32>,
}

impl Tally {
    pub fn agreement(&self) -> f32 {
        self.agree as f32 / self.shots.max(1) as f32
    }

    pub fn add(&mut self, o: &Tally) {
        self.shots += o.shots;
        self.agree += o.agree;
        self.seen_hits += o.seen_hits;
        self.honored += o.honored;
        self.server_hits += o.server_hits;
        self.unseen += o.unseen;
        self.gaps.extend(&o.gaps);
        self.behind += o.behind;
        self.shot_delays.extend(&o.shot_delays);
        self.death_delays.extend(&o.death_delays);
    }
}

/// What the lab keeps as a game is played
#[derive(Default)]
pub struct Lab {
    /// Each slot's figures
    pub players: Vec<Tally>,
    /// Where the corner is: a player whose x is less is out of sight
    pub corner: Option<f32>,
    /// For each slot, when each input that fired a shot was sent, in order
    fired: Vec<Vec<f64>>,
    /// For each slot, when the server killed them, in order
    killed: Vec<Vec<f64>>,
}

impl Lab {
    pub fn tally(&mut self, slot: usize) -> &mut Tally {
        if self.players.len() <= slot {
            self.players.resize(slot + 1, Tally::default());
        }
        &mut self.players[slot]
    }
}

pub struct Game {
    pub net: FakeNet,
    pub server: Server,
    pub server_link: FakeLink,
    pub peers: Vec<Peer>,
    pub steps: u64,
    /// Measuring, when on
    pub lab: Option<Lab>,
}

impl Game {
    pub fn new(conditions: Conditions) -> Game {
        Game::with_setup(conditions, demo_setup())
    }

    pub fn with_setup(conditions: Conditions, setup: Setup) -> Game {
        Game::with_spawns(conditions, setup, (0..MAX_PLAYERS).map(spawn).collect())
    }

    pub fn with_spawns(conditions: Conditions, setup: Setup, spawns: Vec<V3>) -> Game {
        let net = FakeNet::new(11, conditions);
        let server_link = net.endpoint();
        let sim = build_sim(&setup).unwrap();
        let server = Server::new(sim, setup, BUILD, spawns);
        Game { net, server, server_link, peers: Vec::new(), steps: 0, lab: None }
    }

    /// Measure from now on
    pub fn measure(&mut self) {
        self.server.sim.fired = Some(Vec::new());
        self.lab = Some(Lab::default());
    }

    pub fn join_with(&mut self, build: &str, script: impl FnMut(u32) -> Input + 'static) -> usize {
        let link = self.net.endpoint();
        let client = Client::new(self.server_link.addr(), 1000 + self.peers.len() as u32, build);
        self.peers.push(Peer {
            client, link, sim: None, script: Box::new(script), bot: None, sees: None, ticks: 0, on: true, events: Vec::new(),
            recorder: None, frames: Vec::new(), view: None, remote: false, shown: VecDeque::new(), told_shots: 0, told_deaths: 0,
        });
        self.peers.len() - 1
    }

    pub fn join(&mut self, script: impl FnMut(u32) -> Input + 'static) -> usize {
        self.join_with(BUILD, script)
    }

    pub fn join_bot(&mut self, skill: Skill, seed: u64) -> usize {
        let peer = self.join(|_| Input::default());
        self.peers[peer].bot = Some(Bot::new(skill, seed));
        peer
    }

    /// A player who plays from what their screen shows
    pub fn join_seeing(&mut self, sees: impl FnMut(&Sim, usize, &Frame, u32) -> Input + 'static) -> usize {
        let peer = self.join(|_| Input::default());
        self.peers[peer].sees = Some(Box::new(sees));
        peer
    }

    /// A player whose computer records what they see, a frame each tick
    pub fn join_recorded(&mut self, script: impl FnMut(u32) -> Input + 'static) -> usize {
        let peer = self.join(script);
        let p = &mut self.peers[peer];
        p.client.start_tape();
        p.recorder = Some(ClientRecorder::new(BUILD, 1000 + peer as u32));
        peer
    }

    /// A player with a connection of their own, the same both ways
    pub fn join_over(&mut self, conditions: Conditions, script: impl FnMut(u32) -> Input + 'static) -> usize {
        let peer = self.join(script);
        self.connect_over(peer, conditions);
        peer
    }

    /// Give a player a connection of their own, the same both ways. One with any delay is
    /// counted as over the internet.
    pub fn connect_over(&mut self, peer: usize, conditions: Conditions) {
        let (me, server) = (self.peers[peer].link.addr(), self.server_link.addr());
        self.net.set_conditions_between(me, server, conditions);
        self.net.set_conditions_between(server, me, conditions);
        self.peers[peer].remote = conditions.latency > 0.005;
    }

    /// Everyone's computer runs a tick, then the server looks at what has come in
    pub fn step(&mut self) {
        self.steps += 1;
        let now = self.steps as f64 * TICK as f64;
        self.net.set_time(now);
        for p in self.peers.iter_mut().filter(|p| p.on) {
            if p.sim.is_none() {
                let setup = p.client.connect(now, &mut p.link).cloned();
                if let Some(rec) = p.recorder.as_mut() {
                    rec.connected(now, &mut p.client);
                }
                if let Some(setup) = setup {
                    let mut sim = build_sim(&setup).unwrap();
                    if let Some(rec) = p.recorder.as_mut() {
                        // As the game's client does
                        sim.activate_slot(p.client.slot(), START);
                        rec.began(START, false, &sim);
                    }
                    p.sim = Some(sim);
                }
            }
            let Some(sim) = p.sim.as_mut() else { continue };
            let slot = p.client.slot();
            let looks = p.bot.is_some() || p.sees.is_some() || p.recorder.is_some() || self.lab.is_some();
            let view = looks.then(|| p.client.view(now));
            p.view = view.clone();
            if let (Some(rec), Some(view)) = (p.recorder.as_mut(), &view) {
                rec.viewed(now);
                p.frames.push(view.clone());
            }
            let input = match (p.bot.as_mut(), p.sees.as_mut(), &view) {
                (Some(bot), _, Some(view)) => bot.think(sim, slot, view),
                (_, Some(sees), Some(view)) => sees(sim, slot, view, p.ticks),
                _ => (p.script)(p.ticks),
            };
            p.ticks += 1;
            let news = p.client.tick(now, &mut p.link, sim, input);
            if let Some(rec) = p.recorder.as_mut() {
                rec.ticked(now, input, &mut p.client, sim);
            }
            if let (Some(lab), Some(view), true) = (self.lab.as_mut(), &view, slot < sim.players.len()) {
                let bodies = view.players.iter().filter(|s| s.0 != slot && s.3.is_none())
                    .map(|&(i, feet, _, _)| (i, feet, sim.players.get(i).is_some_and(|q| q.player.crouched))).collect();
                p.shown.push_back(Shown { seq: p.client.last_seq(), sent: now, eye: sim.eye(slot), bodies });
                while p.shown.len() > SHOWN {
                    p.shown.pop_front();
                }
                for e in &news {
                    match *e {
                        Event::Shot { player, .. } if player == slot => {
                            if let Some(&sent) = lab.fired.get(slot).and_then(|f| f.get(p.told_shots)) {
                                lab.tally(slot).shot_delays.push((now - sent) as f32);
                            }
                            p.told_shots += 1;
                        }
                        Event::Kill { victim, .. } if victim == slot => {
                            if let Some(&died) = lab.killed.get(slot).and_then(|k| k.get(p.told_deaths)) {
                                lab.tally(slot).death_delays.push((now - died) as f32);
                            }
                            p.told_deaths += 1;
                        }
                        _ => {}
                    }
                }
            }
            p.events.extend(news);
        }
        let happened = self.server.update(now, &mut self.server_link);
        if self.lab.is_some() {
            self.judge(now, &happened);
        }
    }

    /// Compare each shot the server fired with what the shooter's screen showed
    fn judge(&mut self, now: f64, happened: &[Event]) {
        let lab = self.lab.as_mut().unwrap();
        for e in happened {
            if let Event::Kill { victim, .. } = *e {
                grow(&mut lab.killed, victim).push(now);
            }
        }
        let fired = std::mem::take(self.server.sim.fired.as_mut().unwrap());
        for f in fired {
            let Some(shooter) = self.peers.iter().find(|p| p.sim.is_some() && p.client.slot() == f.shooter) else { continue };
            let Some(shown) = shooter.shown.iter().find(|s| s.seq == f.seq) else { continue };
            grow(&mut lab.fired, f.shooter).push(shown.sent);
            let sim = &self.server.sim;
            let judged = sim.body_along(f.eye, f.dir, &f.bodies);
            let seen = sim.body_along(shown.eye, f.dir, &shown.bodies);
            let t = lab.tally(f.shooter);
            t.shots += 1;
            t.agree += (judged == seen) as usize;
            t.seen_hits += seen.is_some() as usize;
            t.honored += (seen.is_some() && judged == seen) as usize;
            t.server_hits += judged.is_some() as usize;
            t.unseen += (judged.is_some() && seen.is_none()) as usize;
            // How far the victim, on their own screen, was from where the shot found them
            let Some(victim) = judged else { continue };
            let Some(v) = self.peers.iter().find(|p| p.client.slot() == victim) else { continue };
            let Some(own) = v.sim.as_ref().map(|s| s.players[victim].player.pos) else { continue };
            let Some(&(_, at, _)) = f.bodies.iter().find(|b| b.0 == victim) else { continue };
            let corner = lab.corner;
            let t = lab.tally(victim);
            t.gaps.push(((own[0] - at[0]).powi(2) + (own[2] - at[2]).powi(2)).sqrt());
            t.behind += corner.is_some_and(|c| own[0] < c) as usize;
        }
    }

    pub fn run(&mut self, seconds: f32) {
        for _ in 0..(seconds / TICK).round() as usize {
            self.step();
        }
    }

    pub fn sim_of(&self, peer: usize) -> &Sim {
        self.peers[peer].sim.as_ref().expect("the client has not joined")
    }

    pub fn me(&self, peer: usize) -> V3 {
        self.sim_of(peer).players[self.peers[peer].client.slot()].player.pos
    }

    pub fn server_pos(&self, slot: usize) -> V3 {
        self.server.sim.players[slot].player.pos
    }
}

fn grow(list: &mut Vec<Vec<f64>>, i: usize) -> &mut Vec<f64> {
    if list.len() <= i {
        list.resize(i + 1, Vec::new());
    }
    &mut list[i]
}

/// A connection over the internet with this round trip (seconds), a little uneven and lossy
pub fn internet(ping: f64) -> Conditions {
    Conditions { latency: ping / 2.0, jitter: ping / 10.0, loss: 0.01, duplicate: 0.0 }
}

/// What a fight comes to
#[derive(Clone, Debug)]
pub struct Fight {
    /// The players on the LAN together, and those over the internet
    pub lan: Tally,
    pub remote: Tally,
    /// Each player's lag meter at the end, and whether they were remote
    pub meters: Vec<(bool, crate::net::meter::Figures)>,
    /// Bytes a second the server sent, all clients together
    pub upload: f32,
}

/// The box room for measuring: players hurt each other, and do not run out of rounds
fn lab_setup() -> Setup {
    Setup { pvp: true, reserve: 30000, ..demo_setup() }
}

/// `lan` bots on the LAN and `remote` over the internet with this round trip fight for `seconds`
pub fn fight(lan: usize, remote: usize, ping: f64, seconds: f32) -> Fight {
    let mut game = Game::with_setup(Conditions::lan(), lab_setup());
    game.measure();
    let skills = [Skill::hard(), Skill::normal(), Skill::easy()];
    for i in 0..lan + remote {
        let p = game.join_bot(skills[i % 3], 100 + i as u64);
        if i >= lan {
            game.connect_over(p, internet(ping));
        }
    }
    game.run(seconds);
    let mut lab = game.lab.take().unwrap();
    let (mut lan, mut far) = (Tally::default(), Tally::default());
    for p in &game.peers {
        let t = lab.tally(p.client.slot()).clone();
        if p.remote { far.add(&t) } else { lan.add(&t) }
    }
    let meters = game.peers.iter().map(|p| (p.remote, p.client.meter.figures())).collect();
    let upload = game.server.snapshot_bytes as f32 / seconds;
    Fight { lan, remote: far, meters, upload }
}

/// Where the corner is in `peek`: a player at a smaller x is out of sight of the one holding
pub const CORNER: f32 = -150.0;

/// What peeking comes to
#[derive(Clone, Debug)]
pub struct Peeking {
    /// Seconds by which the one moving saw the other first, each time they stepped out
    pub advantage: Vec<f32>,
    /// The holder's shots and hits, as the server and their screen saw them; the peeker's gaps
    /// and hits behind cover
    pub holder: Tally,
    pub peeker: Tally,
}

/// One player holds still, watching the corner; another steps out past it and back, again and
/// again, over these connections. The holder fires whenever their screen shows the other.
pub fn peek(holder: Conditions, peeker: Conditions, seconds: f32) -> Peeking {
    peek_judged(holder, peeker, seconds, true)
}

/// `peek`, with shots judged against where the others were when the shooter saw them, or not
pub fn peek_judged(holder: Conditions, peeker: Conditions, seconds: f32, lag_compensation: bool) -> Peeking {
    // The holder across the room, facing the corner; the peeker behind it
    let spawns = vec![[0.0, 0.0, 300.0], [-400.0, 0.0, -200.0]];
    let mut game = Game::with_spawns(Conditions::lan(), lab_setup(), spawns);
    game.server.sim.god = true;
    game.server.set_lag_compensation(lag_compensation);
    game.measure();
    game.lab.as_mut().unwrap().corner = Some(CORNER);
    let h = game.join_seeing(|sim, me, view, _| {
        let mut input = Input::default();
        input.set_look(180.0, 0.0);
        if me >= sim.players.len() {
            return input;
        }
        let eye = sim.eye(me);
        if let Some(&(_, feet, _, _)) = view.players.iter().find(|s| s.0 != me && s.3.is_none() && s.1[0] >= CORNER) {
            let d = [feet[0] - eye[0], feet[1] + 120.0 - eye[1], feet[2] - eye[2]];
            let flat = d[0].hypot(d[2]);
            input.set_look((-d[0]).atan2(d[2]).to_degrees(), d[1].atan2(flat).to_degrees());
            input.fire = true;
        }
        input
    });
    game.connect_over(h, holder);
    // The holder first, to take the first place to start
    game.run(1.0);
    // Out to the right for a second, a moment in view, back, and a rest
    let p = game.join(|t| {
        let phase = t % 160;
        Input { right: phase < 60, left: (80..140).contains(&phase), run: true, ..Input::default() }
    });
    game.connect_over(p, peeker);
    game.run(1.0);
    let mut advantage = Vec::new();
    // When the peeker last came into sight on their own screen, and on the holder's
    let (mut out, mut seen): (Option<f64>, bool) = (None, false);
    for _ in 0..(seconds / TICK) as usize {
        game.step();
        let now = game.steps as f64 * TICK as f64;
        let ps = game.peers[p].client.slot();
        let own = game.me(p)[0] >= CORNER;
        let shown = game.peers[h].view.as_ref().is_some_and(|v| v.players.iter().any(|s| s.0 == ps && s.1[0] >= CORNER));
        match (out, own) {
            (None, true) => (out, seen) = (Some(now), false),
            (Some(_), false) if !shown => out = None,
            _ => {}
        }
        if let (Some(t), true, false) = (out, shown, seen) {
            advantage.push((now - t) as f32);
            seen = true;
        }
    }
    let mut lab = game.lab.take().unwrap();
    let holder = lab.tally(game.peers[h].client.slot()).clone();
    let peeker = lab.tally(game.peers[p].client.slot()).clone();
    Peeking { advantage, holder, peeker }
}
