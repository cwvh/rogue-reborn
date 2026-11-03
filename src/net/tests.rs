//! The server and its clients, run against each other on a pretend network and a pretend clock.

use crate::net::bot::Skill;
use crate::net::client::Status;
use crate::net::lab::{Game, Peer, demo_setup, spawn, team_setup};
use crate::net::link::Conditions;
use crate::net::proto::Setup;
use crate::net::record::{ClientRecording, ServerRecording};
use crate::sim::{self, Event, Input, MAX_PLAYERS, Side, TICK};

fn setup() -> Setup {
    demo_setup()
}

fn run_key() -> Input {
    Input { run: true, ..Input::default() }
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
    let mut game = Game::new(Conditions { latency: 0.05, jitter: 0.03, loss: 0.35, duplicate: 0.35, ..Conditions::NONE });
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
    // Snapshots go in one datagram, but for a few in full as players joined
    let split = game.server.snapshots_split;
    assert!(split <= MAX_PLAYERS as u64, "{split} of {} snapshots went in parts", game.server.snapshots_sent);
    // The grenades went off, and everyone heard
    for p in &game.peers {
        assert!(p.events.iter().any(|e| matches!(e, Event::Blast { .. })));
    }
}

#[test]
fn a_snapshot_too_big_for_a_datagram_reaches_the_clients_in_parts() {
    // Everyone throws grenade after grenade, more than one datagram can tell of
    let mut game = Game::with_setup(Conditions::poor(), Setup { grenades: 40, ..setup() });
    for i in 0..MAX_PLAYERS {
        game.join(move |t| {
            let mut input = Input { grenade: t == 10, fire: t > 20 && t % 6 < 3, ..Input::default() };
            input.set_look(i as f32 * 22.5, 30.0);
            input
        });
    }
    // The most grenades in the air at once, on the server and as each client saw
    let mut most = 0;
    let mut seen = vec![0; MAX_PLAYERS];
    for _ in 0..(6.0 / TICK) as usize {
        game.step();
        most = most.max(game.server.sim.thrown.len());
        for (p, seen) in game.peers.iter().zip(seen.iter_mut()) {
            *seen = (*seen).max(p.sim.as_ref().map_or(0, |s| s.thrown.len()));
        }
    }
    assert!(game.server.snapshots_split > 100, "only {} snapshots went in parts", game.server.snapshots_split);
    assert!(seen.iter().all(|&n| n + 5 >= most), "{most} grenades at most, but the clients saw {seen:?}");
}

/// A match on a poor connection with players coming and going, recorded by the server and by
/// one of the players
fn recorded_match() -> (Game, usize) {
    // A team game, so that the recording has sides to play back too
    let mut game = Game::with_setup(Conditions::poor(), team_setup());
    game.server.record();
    game.join_bot(Skill::hard(), 3);
    let watcher = game.join_recorded(|t| {
        let mut i = Input { forward: (t / 90) % 2 == 0, fire: t % 70 < 20, run: true, ..Input::default() };
        i.set_look((t as f32 * 0.7) % 360.0, 0.0);
        i
    });
    game.run(3.0);
    let late = game.join_bot(Skill::normal(), 5);
    game.run(4.0);
    game.peers[late].on = false;
    game.run(8.0);
    game.join_bot(Skill::easy(), 7);
    game.run(5.0);
    (game, watcher)
}

#[test]
fn a_match_recorded_by_the_server_plays_back_the_same() {
    let (game, _) = recorded_match();
    let rec = game.server.recording().unwrap();
    let back = ServerRecording::from_bytes(&rec.to_bytes()).unwrap();
    assert_eq!(back, rec);
    let size = rec.to_bytes().len() as f32 / rec.seconds();
    assert!(size < 1500.0, "{size:.0} bytes a second for four players");
    // Players came and went, and shots were judged against the past
    let changes: usize = rec.ticks.iter().map(|t| t.changes.len()).sum();
    assert_eq!(changes, 5, "four joins and one leave");
    assert!(rec.ticks.iter().any(|t| t.rewind.iter().any(|&r| r > 5.0)));
    let sim = back.replay().unwrap();
    assert_eq!(sim.checksum(), game.server.sim.checksum());
    assert!(sim.players.iter().any(|p| p.deaths > 0), "nobody died, so the test shows little");
}

#[test]
fn a_server_recording_that_does_not_match_says_where() {
    let (game, _) = recorded_match();
    let mut rec = game.server.recording().unwrap();
    // The watcher turns back for a moment at 9 s
    for t in &mut rec.ticks[540..560] {
        t.inputs[1].forward = !t.inputs[1].forward;
    }
    let Err(err) = rec.replay() else { panic!("it played back the same") };
    let d = err.downcast_ref::<crate::recording::Desync>().expect("a desync");
    assert_eq!(d.tick, 600);
}

#[test]
fn a_player_records_what_they_saw_and_it_plays_back_the_same() {
    let (mut game, watcher) = recorded_match();
    let now = game.steps as f64 * TICK as f64;
    let p = &mut game.peers[watcher];
    p.recorder.as_mut().unwrap().mark(now - 2.0);
    let rec = p.recorder.as_ref().unwrap().recording(p.sim.as_ref().unwrap());
    let back = ClientRecording::from_bytes(&rec.to_bytes()).unwrap();
    // Small enough for a tester to send: about eight megabytes for ten minutes
    let size = rec.to_bytes().len() as f64 / rec.seconds();
    assert!(size < 16000.0, "{size:.0} bytes a second");
    assert_eq!(back, rec);
    assert_eq!(back.marks.len(), 1);
    let mut frames = Vec::new();
    let sim = back.replay(|_, frame, _, _| frames.push(frame.clone())).unwrap();
    assert_eq!(sim.checksum(), p.sim.as_ref().unwrap().checksum());
    // Every frame as it was shown, the others moving and all
    assert_eq!(frames.len(), p.frames.len());
    assert!(frames == p.frames, "the frames differ");
    assert!(frames.iter().any(|f| f.players.len() >= 3));
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

#[test]
fn a_player_shot_dead_is_dead_for_everyone_and_comes_back_where_the_server_says() {
    let mut game = Game::new(Conditions::broadband());
    game.server.sim.pvp = true;
    // The shooter looks along the row the players start in, at the other one's head
    let shooter = game.join(|t| {
        let mut i = Input { fire: t > 30 && t < 40, run: true, ..Input::default() };
        i.set_look(90.0, 0.0);
        i
    });
    // The other stands still until they have been shot, and then tries to walk away
    let target = game.join(|t| Input { forward: t > 60, run: true, ..Input::default() });
    game.run(1.5);
    let (a, b) = (game.peers[shooter].client.slot(), game.peers[target].client.slot());
    assert_eq!(game.server.sim.players[b].deaths, 1, "{:?}", game.server.sim.players.iter().map(|p| (p.player.pos, p.look, p.ammo, p.hp)).collect::<Vec<_>>());
    assert_eq!(game.server.sim.players[a].kills, 1, "a={a} b={b} {:?}", game.server.sim.players.iter().map(|p| (p.kills, p.deaths, p.hp)).collect::<Vec<_>>());
    // Everyone's copy of the game says so, with the fall the server picked from when it began,
    // and the dead player did not walk on in their own
    let fall = game.server.sim.players[b].hit.expect("a fall");
    for peer in [shooter, target] {
        let seen = &game.sim_of(peer).players[b];
        assert!(seen.respawn_at.is_some() && seen.hp == 0.0, "peer {peer}: {:?} {}", seen.respawn_at, seen.hp);
        let hit = seen.hit.expect("a fall");
        assert!(hit.motion == fall.motion && (hit.start - fall.start).abs() < 0.001, "peer {peer}: {hit:?} against {fall:?}");
    }
    assert!(distance(game.me(target), game.server_pos(b)) < 1.0);
    // And everyone was told who killed whom
    for peer in [shooter, target] {
        let told = game.peers[peer].events.iter().filter(|e| matches!(e, Event::Kill { killer: Some(k), victim, .. } if (*k, *victim) == (a, b))).count();
        assert_eq!(told, 1, "peer {peer}");
    }
    game.run(sim::RESPAWN_SECONDS + 1.0);
    let server = &game.server.sim.players[b];
    assert!(server.respawn_at.is_none() && server.hp == sim::PLAYER_HP);
    assert_eq!(game.sim_of(target).players[b].hp, sim::PLAYER_HP);
    assert!(distance(game.me(target), game.server_pos(b)) < 1.0);
    assert_eq!(game.sim_of(shooter).players[a].kills, 1);
}

#[test]
fn a_bot_finds_a_player_standing_in_the_room_and_shoots_them_dead() {
    let mut game = Game::new(Conditions::broadband());
    game.server.sim.pvp = true;
    let bot = game.join_bot(Skill::normal(), 3);
    let target = game.join(|_| run_key());
    game.run(20.0);
    let (a, b) = (game.peers[bot].client.slot(), game.peers[target].client.slot());
    let server = &game.server.sim.players;
    assert!(server[a].kills >= 1 && server[b].deaths >= 1, "kills {} deaths {}", server[a].kills, server[b].deaths);
    // And the target never hurt it back
    assert_eq!(server[a].deaths, 0);
}

#[test]
fn a_bot_alone_keeps_walking_around_the_room() {
    let mut game = Game::new(Conditions::broadband());
    let bot = game.join_bot(Skill::normal(), 5);
    game.run(1.0);
    let start = game.me(bot);
    let (mut farthest, mut last) = (0.0f32, start);
    let mut still = 0;
    for _ in 0..30 {
        game.run(1.0);
        let here = game.me(bot);
        farthest = farthest.max(distance(here, start));
        if distance(here, last) < 30.0 {
            still += 1;
        }
        last = here;
    }
    assert!(farthest > 300.0, "it stayed within {farthest} of where it began");
    assert!(still <= 3, "it stood still for {still} of 30 seconds");
}

/// The lag meter's figures for a player walking about for ten seconds on a connection like this
fn metered(conditions: Conditions) -> crate::net::meter::Figures {
    let mut game = Game::new(conditions);
    let a = game.join(|t| Input { forward: (t / 60) % 2 == 0, run: true, ..Input::default() });
    for _ in 0..600 {
        game.step();
        let now = game.steps as f64 * TICK as f64;
        game.peers[a].client.view(now);
    }
    game.peers[a].client.meter.figures()
}

#[test]
fn the_lag_meter_reads_a_clean_connection_as_clean() {
    let f = metered(Conditions::NONE);
    // Nothing on the wire, so only the server's input queue and the wait for the next snapshot
    assert!(f.ping < 0.07, "{f:?}");
    assert!(f.jitter < 0.005, "{f:?}");
    assert_eq!(f.loss, 0.0, "{f:?}");
    assert_eq!(f.fixes, 0, "{f:?}");
    assert!(f.bytes_in > 0.0 && f.bytes_out > 0.0, "{f:?}");
}

#[test]
fn the_lag_meter_sees_the_latency_jitter_and_loss_of_a_bad_connection() {
    let clean = metered(Conditions::NONE);
    let f = metered(Conditions { latency: 0.08, jitter: 0.04, loss: 0.08, duplicate: 0.0, ..Conditions::NONE });
    // 80 ms each way, 20 more each way on average from the jitter, and the inputs the server
    // holds back so that a late one does not leave it with nothing (that lag is felt too)
    let added = f.ping - clean.ping;
    assert!((0.2..0.3).contains(&added), "{added} added to the ping: {f:?}");
    assert!((0.005..0.03).contains(&f.jitter), "{f:?}");
    assert!((0.03..0.15).contains(&f.loss), "{f:?}");
    // Other players are shown later than on a clean connection by at least the trip
    assert!(f.view - clean.view > 0.08, "{f:?} against {clean:?}");
}

#[test]
fn a_player_far_away_does_not_slow_the_ones_on_the_lan() {
    // Three players on the LAN with the server, and one over a poor internet connection
    let mut game = Game::new(Conditions::lan());
    let walk = |t: u32| Input { forward: (t / 60) % 2 == 0, run: true, ..Input::default() };
    let lan: Vec<usize> = (0..3).map(|_| game.join(walk)).collect();
    let far = game.join_over(Conditions::poor(), walk);
    for _ in 0..(10.0 / TICK) as usize {
        game.step();
        let now = game.steps as f64 * TICK as f64;
        for p in game.peers.iter_mut() {
            p.client.view(now);
        }
    }
    let far = game.peers[far].client.meter.figures();
    for &p in &lan {
        let f = game.peers[p].client.meter.figures();
        assert!(f.ping < 0.07, "{f:?}");
        assert_eq!(f.fixes, 0, "{f:?}");
        assert!(far.ping > f.ping + 0.2, "{far:?} against {f:?}");
    }
    assert!(far.loss > 0.02, "{far:?}");
}

#[test]
fn others_see_a_dead_player_fall_and_then_appear_where_they_come_back() {
    let mut game = Game::new(Conditions::broadband());
    game.server.sim.pvp = true;
    let bot = game.join_bot(Skill::hard(), 3);
    // Pacing to and fro, so that where they die is never a spawn point
    let target = game.join(|t| Input { forward: (t / 40) % 2 == 0, back: (t / 40) % 2 == 1, ..run_key() });
    let mut last: Option<([f32; 3], bool)> = None;
    let (mut died_at, mut comebacks) = (None, 0);
    for _ in 0..(25.0 / TICK) as usize {
        game.step();
        let now = game.steps as f64 * TICK as f64;
        let slot = game.peers[target].client.slot();
        let view = game.peers[bot].client.view(now);
        let Some(&crate::net::client::Seen { feet: pos, dead, .. }) = view.players.iter().find(|p| p.slot == slot) else { continue };
        match (last, dead.is_some()) {
            (Some((_, false)), true) => died_at = Some(pos),
            (Some((_, true)), false) => {
                // Not standing up where they fell, and not slid there from it
                let fell = died_at.expect("seen to die first");
                assert!(distance(pos, fell) > 1.0, "came back at {pos:?}, where they died");
                assert!((0..MAX_PLAYERS).any(|i| distance(pos, spawn(i)) < 30.0), "came back at {pos:?}, which is no spawn point");
                comebacks += 1;
            }
            (Some((before, false)), false) => assert!(distance(pos, before) < 50.0, "slid from {before:?} to {pos:?}"),
            _ => {}
        }
        last = Some((pos, dead.is_some()));
    }
    assert!(comebacks >= 1, "never seen to die and come back");
}

#[test]
fn the_lab_sees_the_server_agree_with_the_shooter_only_when_it_looks_back() {
    use crate::net::lab::{Tuning, internet, peek_with};
    // The shooter far away, the one they shoot at on the LAN
    let far = internet(0.15);
    let judged = peek_with(Tuning::default(), far, Conditions::lan(), 15.0);
    let now = peek_with(Tuning { lag_compensation: false, ..Tuning::default() }, far, Conditions::lan(), 15.0);
    assert!(judged.holder.shots > 50 && now.holder.shots > 50, "{judged:?}");
    // Judged where the shooter saw them, the hits on screen are hits
    assert!(judged.holder.agreement() > 0.95, "{judged:?}");
    // Judged where they are now, the peeker has moved on from where the shooter saw them
    assert!(now.holder.agreement() < 0.8, "{:?}", now.holder.agreement());
}

#[test]
fn the_one_peeking_sees_first_and_more_so_with_lag() {
    use crate::net::lab::{internet, peek};
    let lan = peek(Conditions::lan(), Conditions::lan(), 12.0);
    let far = peek(Conditions::lan(), internet(0.15), 12.0);
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    assert!(lan.advantage.len() >= 4 && far.advantage.len() >= 4);
    // Even on the LAN: the holder sees the others a little in the past
    assert!(mean(&lan.advantage) > 0.03, "{:?}", lan.advantage);
    assert!(mean(&far.advantage) > mean(&lan.advantage) + 0.05, "{:?} against {:?}", far.advantage, lan.advantage);
    // Shot as the holder saw them, the peeker is sometimes back behind the corner on their own screen
    assert!(far.peeker.behind > 0, "{far:?}");
}

#[test]
fn a_lab_fight_measures_players_on_the_lan_and_far_away() {
    use crate::net::lab::{fight, internet};
    let f = fight(4, 2, internet(0.1), 15.0);
    assert!(f.lan.shots > 0 && f.remote.shots > 0, "{f:?}");
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    // Far away, the news of a shot takes a round trip longer
    assert!(mean(&f.remote.shot_delays) > mean(&f.lan.shot_delays) + 0.08, "{f:?}");
    assert!(f.upload > 1000.0 && f.meters.len() == 6);
}

#[test]
fn a_player_whose_computer_freezes_stops_on_the_server_and_plays_on_after() {
    let mut game = Game::new(Conditions::lan());
    // Running forward and back, the trigger held
    let a = game.join(|t| Input { forward: (t / 60) % 2 == 0, back: (t / 60) % 2 == 1, fire: true, run: true, ..Input::default() });
    game.run(2.2);
    let slot = game.peers[a].client.slot();
    let (before, ammo) = (game.server_pos(slot), game.server.sim.players[slot].ammo + game.server.sim.players[slot].reserve);
    game.peers[a].frozen = Some((2.2, 4.2));
    game.run(1.9);
    // A moment's running and firing, and then nothing
    let moved = distance(game.server_pos(slot), before);
    let fired = ammo - game.server.sim.players[slot].ammo - game.server.sim.players[slot].reserve;
    assert!(moved < 260.0, "ran on {moved} units");
    assert!(fired <= 6, "fired {fired} rounds");
    game.run(2.0);
    assert_eq!(game.peers[a].client.status, Status::Connected);
    let after = game.server_pos(slot);
    game.run(0.5);
    assert!(distance(game.server_pos(slot), after) > 100.0, "not moving again");
}

/// A player walking about on broadband for `seconds`, with a clock that runs at `rate`: their
/// ping at the end, and the fixes to their prediction all along
fn drifting(rate: f64, seconds: f32) -> (f32, u32) {
    let mut game = Game::new(Conditions::broadband());
    let a = game.join(|t| Input { forward: (t / 50) % 2 == 0, right: (t / 70) % 2 == 0, run: true, ..Input::default() });
    game.peers[a].clock = rate;
    let mut fixes = 0;
    for _ in 0..(seconds / TICK) as usize {
        game.step();
        fixes += (game.peers[a].client.correction > 1.0) as u32;
    }
    (game.peers[a].client.meter.figures().ping, fixes)
}

#[test]
fn a_clock_that_runs_fast_does_not_pile_up_inputs_on_the_server() {
    let (true_ping, _) = drifting(1.0, 60.0);
    // A tenth of a millisecond a second fast, for ten minutes, and a clock that is far off
    for (rate, seconds) in [(1.0001, 600.0), (1.005, 60.0), (0.995, 60.0)] {
        let (ping, fixes) = drifting(rate, seconds);
        assert!(ping < true_ping + 0.012, "{ping} against {true_ping} at {rate}");
        assert!(fixes as f32 <= seconds / 10.0, "{fixes} fixes at {rate}");
    }
}

#[test]
fn players_on_wifi_keep_playing_through_its_bursts_and_stalls() {
    use crate::net::lab::fight;
    let f = fight(4, 2, Conditions::wifi(), 40.0);
    assert!(f.remote.shots > 10, "{f:?}");
    // Shots from a held trigger are judged as the shooter would have seen them; what is left is
    // a stall longer than the server looks back, and the shooter's own place mispredicted
    assert!(f.remote.agreement() > 0.9, "{:?}", f.remote.agreement());
    assert!(f.lan.agreement() > 0.97, "{:?}", f.lan.agreement());
}

/// Where a side's players start in the box room
fn starts(side: Side) -> Vec<[f32; 3]> {
    team_setup().bases[side.index()].clone()
}

#[test]
fn players_join_on_alternate_sides_and_start_in_their_own_base() {
    let mut game = Game::with_setup(Conditions::lan(), team_setup());
    for _ in 0..4 {
        game.join(|_| Input::default());
        game.run(0.3);
    }
    game.run(0.5);
    let sides: Vec<Option<Side>> = (0..4).map(|i| game.server.sim.players[i].side).collect();
    assert_eq!(sides, [Some(Side::Blue), Some(Side::Gold), Some(Side::Blue), Some(Side::Gold)]);
    // A side's first two at its base's opposite corners
    for (slot, side, k) in [(0, Side::Blue, 0), (1, Side::Gold, 0), (2, Side::Blue, 1), (3, Side::Gold, 1)] {
        assert!(distance(game.server_pos(slot), starts(side)[k]) < 1.0, "player {slot} at {:?}", game.server_pos(slot));
    }
    // And every client knows who is on which side
    for p in 0..4 {
        let seen: Vec<Option<Side>> = (0..4).map(|i| game.sim_of(p).players[i].side).collect();
        assert_eq!(seen, sides);
    }
}

#[test]
fn a_player_gets_the_side_they_ask_for_unless_it_has_more() {
    let mut game = Game::with_setup(Conditions::lan(), team_setup());
    for side in [Side::Gold, Side::Gold, Side::Gold, Side::Blue] {
        game.join_on(side, |_| Input::default());
        game.run(0.3);
    }
    let sides: Vec<Option<Side>> = (0..4).map(|i| game.server.sim.players[i].side).collect();
    // The second to ask for Gold is turned to Blue, which has fewer; then Gold is level again
    assert_eq!(sides, [Some(Side::Gold), Some(Side::Blue), Some(Side::Gold), Some(Side::Blue)]);
}

#[test]
fn without_friendly_fire_a_side_cannot_hurt_its_own() {
    for friendly_fire in [true, false] {
        let mut game = Game::with_setup(Conditions::lan(), Setup { friendly_fire, ..team_setup() });
        // Two on Blue at their base's opposite corners, one firing at the other's head, and
        // one on Gold so that sides stay even
        let (from, to) = (starts(Side::Blue)[0], starts(Side::Blue)[1]);
        let yaw = (-(to[0] - from[0])).atan2(to[2] - from[2]).to_degrees();
        let shooter = game.join_on(Side::Blue, move |t| {
            let mut i = Input { fire: t > 60 && t < 70, ..Input::default() };
            i.set_look(yaw, 0.0);
            i
        });
        game.run(0.3);
        game.join_on(Side::Gold, |_| Input::default());
        game.run(0.3);
        let target = game.join_on(Side::Blue, |_| Input::default());
        game.run(1.5);
        assert_eq!(game.server.sim.players[game.peers[target].client.slot()].side, Some(Side::Blue));
        let slot = game.peers[target].client.slot();
        let hurt = game.server.sim.players[slot].hp < sim::PLAYER_HP || game.server.sim.players[slot].deaths > 0;
        assert_eq!(hurt, friendly_fire, "friendly fire {friendly_fire}: {:?} shot at {:?}", game.server_pos(game.peers[shooter].client.slot()), game.server_pos(slot));
    }
}

#[test]
fn bots_fight_only_the_other_side() {
    let mut game = Game::with_setup(Conditions::lan(), Setup { friendly_fire: false, reserve: 3000, ..team_setup() });
    for i in 0..4 {
        game.join_bot(crate::net::bot::Skill::hard(), 50 + i);
        game.run(0.2);
    }
    game.run(30.0);
    let side = |i: usize| game.server.sim.players[i].side;
    let kills: Vec<(usize, usize)> = game.peers[0].events.iter()
        .filter_map(|e| match *e { Event::Kill { killer: Some(k), victim, .. } => Some((k, victim)), _ => None }).collect();
    assert!(!kills.is_empty(), "nobody was killed");
    assert!(kills.iter().all(|&(k, v)| side(k) != side(v)), "{kills:?}");
}
