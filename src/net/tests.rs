//! The server and its clients, run against each other on a pretend network and a pretend clock.

use crate::net::client::{Client, Status};
use crate::net::link::{Conditions, FakeLink, FakeNet};
use crate::net::proto::Setup;
use crate::net::server::Server;
use crate::net::setup::build_sim;
use crate::sim::{Event, Input, MAX_PLAYERS, Sim, TICK};

const BUILD: &str = "test";

fn setup() -> Setup {
    Setup {
        map: "demo".to_string(),
        model: "no-such-model.CRP".to_string(),
        idle: "up_none_cyc".to_string(),
        magazine: 30,
        ammo: 30,
        reserve: 90,
        grenades: 3,
        speed_scale: 1.0,
        seed: 7,
        culling: true,
        doors_open: None,
        npcs: Vec::new(),
    }
}

/// One player at a computer: a client, its copy of the game, and what the player does
struct Peer {
    client: Client,
    link: FakeLink,
    sim: Option<Sim>,
    script: Box<dyn FnMut(u32) -> Input>,
    /// Ticks this client has run
    ticks: u32,
    /// Whether the client is running: a client that is not is a computer that has been switched off
    on: bool,
    /// What the client has been told happened
    events: Vec<Event>,
}

struct Game {
    net: FakeNet,
    server: Server,
    server_link: FakeLink,
    peers: Vec<Peer>,
    steps: u64,
}

fn run_key() -> Input {
    Input { run: true, ..Input::default() }
}

impl Game {
    fn new(conditions: Conditions) -> Game {
        let net = FakeNet::new(11, conditions);
        let server_link = net.endpoint();
        let setup = setup();
        let sim = build_sim(&setup).unwrap();
        let spawns = (0..MAX_PLAYERS).map(|i| [-300.0 + i as f32 * 80.0, 0.0, -350.0]).collect();
        let server = Server::new(sim, setup, BUILD, spawns);
        Game { net, server, server_link, peers: Vec::new(), steps: 0 }
    }

    fn join_with(&mut self, build: &str, script: impl FnMut(u32) -> Input + 'static) -> usize {
        let link = self.net.endpoint();
        let client = Client::new(self.server_link.addr(), 1000 + self.peers.len() as u32, build);
        self.peers.push(Peer { client, link, sim: None, script: Box::new(script), ticks: 0, on: true, events: Vec::new() });
        self.peers.len() - 1
    }

    fn join(&mut self, script: impl FnMut(u32) -> Input + 'static) -> usize {
        self.join_with(BUILD, script)
    }

    /// Everyone's computer runs a tick, then the server looks at what has come in
    fn step(&mut self) {
        self.steps += 1;
        let now = self.steps as f64 * TICK as f64;
        self.net.set_time(now);
        for p in self.peers.iter_mut().filter(|p| p.on) {
            if p.sim.is_none() {
                if let Some(setup) = p.client.connect(now, &mut p.link).cloned() {
                    p.sim = Some(build_sim(&setup).unwrap());
                }
            }
            if let Some(sim) = p.sim.as_mut() {
                let input = (p.script)(p.ticks);
                p.ticks += 1;
                let news = p.client.tick(now, &mut p.link, sim, input);
                p.events.extend(news);
            }
        }
        self.server.update(now, &mut self.server_link);
    }

    fn run(&mut self, seconds: f32) {
        for _ in 0..(seconds / TICK).round() as usize {
            self.step();
        }
    }

    fn sim_of(&self, peer: usize) -> &Sim {
        self.peers[peer].sim.as_ref().expect("the client has not joined")
    }

    fn me(&self, peer: usize) -> [f32; 3] {
        self.sim_of(peer).players[self.peers[peer].client.slot()].player.pos
    }

    fn server_pos(&self, slot: usize) -> [f32; 3] {
        self.server.sim.players[slot].player.pos
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn a_client_asks_to_join_and_is_given_a_place() {
    let mut game = Game::new(Conditions::NONE);
    let a = game.join(|_| run_key());
    game.run(1.0);
    assert_eq!(game.peers[a].client.status, Status::Connected);
    assert_eq!(game.peers[a].client.slot(), 0);
    assert_eq!(game.server.players(), 1);
    assert!(game.server.sim.players[0].active);
    // The client's copy of the game has them too
    assert!(game.sim_of(a).players[0].active);
    assert!(game.peers[a].client.snapshots > 10);
}

#[test]
fn a_client_that_asks_again_is_not_let_in_twice() {
    let mut game = Game::new(Conditions { loss: 0.6, ..Conditions::broadband() });
    let a = game.join(|_| run_key());
    game.run(10.0);
    assert_eq!(game.peers[a].client.status, Status::Connected);
    assert_eq!(game.server.players(), 1);
    assert_eq!(game.server.sim.players.iter().filter(|p| p.active).count(), 1);
}

#[test]
fn prediction_agrees_with_the_server_when_nothing_goes_wrong() {
    let mut game = Game::new(Conditions::NONE);
    let a = game.join(|t| {
        let mut i = Input { forward: (t / 90) % 2 == 0, left: (t / 50) % 3 == 0, run: true, ..Input::default() };
        i.set_look((t as f32 * 0.9) % 360.0, 0.0);
        i
    });
    let mut worst = 0.0f32;
    for _ in 0..600 {
        game.step();
        if game.peers[a].client.snapshots > 5 {
            worst = worst.max(game.peers[a].client.correction);
        }
    }
    // The server was never surprised by what the player did, so it never had to move them
    assert!(worst < 0.001, "the server moved the player by {worst} after prediction");
    // And it was not a standing player: they went somewhere
    assert!(distance(game.me(a), [-300.0, 0.0, -350.0]) > 100.0);
}

#[test]
fn prediction_survives_a_poor_connection() {
    let mut game = Game::new(Conditions::poor());
    let a = game.join(|t| {
        let mut i = Input { forward: (t / 70) % 2 == 0, right: (t / 40) % 3 == 0, run: true, ..Input::default() };
        i.set_look((t as f32 * 0.5) % 360.0, 0.0);
        i
    });
    game.run(30.0);
    // Then the player stops, and the last of the datagrams arrive
    game.peers[a].script = Box::new(|_| run_key());
    game.run(3.0);
    let (mine, theirs) = (game.me(a), game.server_pos(0));
    assert!(distance(mine, theirs) < 1.0, "the client thinks {mine:?} and the server {theirs:?}");
    assert_eq!(game.peers[a].client.status, Status::Connected);
}

#[test]
fn shooting_is_the_servers_to_decide() {
    let mut game = Game::new(Conditions::broadband());
    let a = game.join(|t| Input { fire: t > 60 && t < 140, run: true, ..Input::default() });
    game.run(1.0);
    assert_eq!(game.sim_of(a).players[0].ammo, 30);
    game.run(3.0);
    // The server fired 13 or so rounds and so, as far as anyone can see, did the player
    let fired = 30 - game.server.sim.players[0].ammo;
    assert!((9..=16).contains(&fired), "{fired} rounds fired");
    assert_eq!(game.sim_of(a).players[0].ammo, game.server.sim.players[0].ammo);
    // The bullets hit the wall, and the client heard so
    let hits = game.peers[a].events.iter().filter(|e| matches!(e, Event::BulletHit { .. })).count();
    assert_eq!(hits as u32, fired);
}

#[test]
fn players_see_one_another_move() {
    let mut game = Game::new(Conditions::broadband());
    let a = game.join(|t| Input { forward: t > 30 && t < 130, run: true, ..Input::default() });
    let b = game.join(|_| run_key());
    game.run(0.5);
    // While A runs, B's copy of the game follows along, a little behind
    let slot_a = game.peers[a].client.slot();
    let mut lag = 0.0f32;
    for _ in 0..90 {
        game.step();
        let seen = game.sim_of(b).players[slot_a].player.pos;
        lag = lag.max(distance(seen, game.server_pos(slot_a)));
    }
    assert!(lag > 1.0 && lag < 150.0, "B sees A {lag} units behind at worst");
    // Once A has stopped, B sees them where they are
    game.run(2.0);
    let seen = game.sim_of(b).players[slot_a].player.pos;
    assert!(distance(seen, game.server_pos(slot_a)) < 0.01, "B sees A at {seen:?}, who is at {:?}", game.server_pos(slot_a));
    assert!(distance(seen, [-300.0, 0.0, -350.0]) > 300.0, "A has not gone anywhere");
    assert!(game.sim_of(b).players[game.peers[b].client.slot()].active);
}

#[test]
fn an_event_is_heard_once_even_when_the_network_loses_and_repeats_datagrams() {
    let mut game = Game::new(Conditions { latency: 0.05, jitter: 0.03, loss: 0.35, duplicate: 0.35 });
    let a = game.join(|t| match t {
        20 => Input { grenade: true, fire: true, run: true, ..Input::default() },
        21..=50 => Input { fire: true, run: true, ..Input::default() },
        _ => run_key(),
    });
    game.run(10.0);
    let blasts = game.peers[a].events.iter().filter(|e| matches!(e, Event::Blast { .. })).count();
    let shakes = game.peers[a].events.iter().filter(|e| matches!(e, Event::PlayerBlast { .. })).count();
    assert_eq!((blasts, shakes), (1, 1), "events: {:?}", game.peers[a].events.iter().filter(|e| !matches!(e, Event::Sound { .. })).collect::<Vec<_>>());
}

#[test]
fn a_computer_switched_off_is_dropped_and_its_place_is_free_again() {
    let mut game = Game::new(Conditions::NONE);
    let a = game.join(|_| run_key());
    game.run(1.0);
    assert_eq!(game.server.players(), 1);
    game.peers[a].on = false;
    game.run(6.0);
    assert_eq!(game.server.players(), 0);
    assert!(!game.server.sim.players[0].active);
    let b = game.join(|_| run_key());
    game.run(1.0);
    assert_eq!(game.peers[b].client.slot(), 0);
    assert_eq!(game.server.players(), 1);
}

#[test]
fn a_client_that_says_goodbye_is_dropped_at_once() {
    let mut game = Game::new(Conditions::NONE);
    let a = game.join(|_| run_key());
    game.run(1.0);
    let peer = &mut game.peers[a];
    peer.client.leave(&mut peer.link);
    game.peers[a].on = false;
    game.run(0.1);
    assert_eq!(game.server.players(), 0);
}

#[test]
fn a_full_game_turns_the_next_player_away() {
    let mut game = Game::new(Conditions::NONE);
    let peers: Vec<usize> = (0..MAX_PLAYERS + 1).map(|_| game.join(|_| run_key())).collect();
    game.run(2.0);
    assert_eq!(game.server.players(), MAX_PLAYERS);
    let refused: Vec<&Peer> = game.peers.iter().filter(|p| matches!(p.client.status, Status::Refused(_))).collect();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].client.status, Status::Refused("the game is full".to_string()));
    let _ = peers;
}

#[test]
fn a_different_build_is_turned_away() {
    let mut game = Game::new(Conditions::NONE);
    let a = game.join_with("some other build", |_| run_key());
    game.run(1.0);
    match &game.peers[a].client.status {
        Status::Refused(why) => assert!(why.contains("build"), "{why}"),
        s => panic!("{s:?}"),
    }
    assert_eq!(game.server.players(), 0);
}

#[test]
fn a_server_that_goes_quiet_is_noticed() {
    let mut game = Game::new(Conditions::NONE);
    let a = game.join(|_| run_key());
    game.run(1.0);
    // The server stops hearing from anyone, and so stops answering: pull the plug on its link
    game.net.set_conditions(Conditions { loss: 1.0, ..Conditions::NONE });
    game.run(6.0);
    assert_eq!(game.peers[a].client.status, Status::Lost);
}

#[test]
fn a_full_game_of_busy_players_fits_in_the_datagrams() {
    // FakeLink refuses a datagram over the limit, so this fails if one is ever too big
    let mut game = Game::new(Conditions::broadband());
    for i in 0..MAX_PLAYERS {
        game.join(move |t| {
            let mut input = Input {
                forward: (t / 40 + i as u32) % 2 == 0, fire: (t / 25 + i as u32) % 3 == 0,
                grenade: t % 300 == i as u32 * 7, reload: t % 200 == 199, run: true, ..Input::default()
            };
            input.set_look((t as f32 * (1.0 + i as f32 * 0.2)) % 360.0, 0.0);
            input
        });
    }
    game.run(20.0);
    assert_eq!(game.server.players(), MAX_PLAYERS);
    // The grenades went off, and everyone heard
    for p in &game.peers {
        assert!(p.events.iter().any(|e| matches!(e, Event::Blast { .. })));
    }
}

#[test]
fn the_whole_thing_is_repeatable() {
    let run = || {
        let mut game = Game::new(Conditions::poor());
        let a = game.join(|t| Input { forward: t % 100 < 60, fire: t % 70 < 20, grenade: t == 30, run: true, ..Input::default() });
        game.run(15.0);
        (game.server.sim.checksum(), game.me(a).map(f32::to_bits), game.peers[a].events.len())
    };
    assert_eq!(run(), run());
}
