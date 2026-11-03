//! Rounds, played out by clients and a server on a pretend network

use crate::net::client::{Frame, Status};
use crate::net::lab::{Game, demo_setup, team_setup};
use crate::net::link::Conditions;
use crate::net::proto::Setup;
use crate::net::record::Change;
use crate::net::round::{Outcome, Phase};
use crate::net::server::{LOAD_TIMEOUT, RELEASE_AFTER, Seat};
use crate::net::watch::{Pose, Watch, WatchContext};
use crate::sim::{Cause, Event, Input, PLAYER_HP, Side, Sim, TICK};

type Sees = Box<dyn FnMut(&Sim, usize, &Frame, u32) -> Input>;

fn in_rounds(setup: Setup) -> Game {
    let mut game = Game::with_setup(Conditions::lan(), setup);
    game.server.play_rounds();
    game
}

/// Two players 200 apart in the box room, Blue to the south facing Gold to the north
fn face_off() -> Setup {
    Setup { bases: [vec![[0.0, 0.0, -100.0]], vec![[0.0, 0.0, 100.0]]], ..team_setup() }
}

/// Facing the other side in `face_off`
fn facing(side: Side) -> f32 {
    if side == Side::Blue { 0.0 } else { 180.0 }
}

fn join(game: &mut Game, side: Side, sees: Sees) -> usize {
    let peer = game.join_seeing(sees);
    game.peers[peer].client.ask_side(Some(side));
    game.run(0.2);
    peer
}

/// Stands facing the other side, firing whenever `fires` says
fn shooter(side: Side, mut fires: impl FnMut(&Sim) -> bool + 'static) -> Sees {
    Box::new(move |sim, _, _, _| {
        let mut i = Input { fire: fires(sim), ..Input::default() };
        i.set_look(facing(side), 0.0);
        i
    })
}

fn idle(side: Side) -> Sees {
    shooter(side, |_| false)
}

/// Once let go into a round, drops a grenade at their feet, and nothing else
fn thrower(side: Side) -> Sees {
    let (mut held, mut live) = (false, 0u32);
    Box::new(move |sim, _, _, _| {
        // Counting from the end of the hold at a round's start
        held |= sim.frozen;
        live += (held && !sim.frozen) as u32;
        let mut i = match live {
            1 => Input { grenade: true, fire: true, ..Input::default() },
            2 => Input { fire: true, ..Input::default() },
            _ => Input::default(),
        };
        i.set_look(facing(side), -85.0);
        i
    })
}

/// Run until the server is in `phase`, for at most `seconds`. Says whether it got there.
fn run_until(game: &mut Game, phase: Phase, seconds: f32) -> bool {
    for _ in 0..(seconds / TICK) as usize {
        if game.server.round().phase == phase {
            return true;
        }
        game.step();
    }
    game.server.round().phase == phase
}

fn slot(game: &Game, peer: usize) -> usize {
    game.peers[peer].client.slot()
}

fn kills(game: &Game, peer: usize) -> Vec<(Option<usize>, usize, Cause)> {
    game.peers[peer].events.iter().filter_map(|e| match *e {
        Event::Kill { killer, victim, cause } => Some((killer, victim, cause)),
        _ => None,
    }).collect()
}

#[test]
fn a_round_is_won_by_the_last_side_standing_and_everyone_sees_it() {
    let mut game = in_rounds(face_off());
    // Blue holds the trigger throughout. In the lobby players are free, so Gold is hurt
    // already, and the round starts them over.
    let blue = join(&mut game, Side::Blue, shooter(Side::Blue, |_| true));
    let gold = join(&mut game, Side::Gold, idle(Side::Gold));
    game.run(1.0);
    assert_eq!(game.server.round().phase, Phase::Lobby);
    assert!(game.server.sim.players[slot(&game, gold)].hp < PLAYER_HP);
    assert!(game.server.start_countdown());
    assert!(run_until(&mut game, Phase::Hold, 10.0));
    assert_eq!(game.server.sim.players[slot(&game, gold)].hp, PLAYER_HP);
    assert!(run_until(&mut game, Phase::Results, 20.0));
    assert_eq!(game.server.round().last, Some(Outcome::Won(Side::Blue)));
    assert_eq!(game.server.round().wins, [1, 0]);
    game.run(0.5);
    for p in [blue, gold] {
        let round = game.peers[p].client.round;
        assert_eq!((round.phase, round.number, round.last, round.wins), (Phase::Results, 1, Some(Outcome::Won(Side::Blue)), [1, 0]));
    }
    // No one comes back within a round, and the fall plays once: it began when it began, however
    // long they lie there. Then it is the lobby again.
    let g = slot(&game, gold);
    let fell = game.sim_of(blue).players[g].hit.map(|h| h.start);
    assert!(fell.is_some());
    game.run(5.0);
    assert!(game.server.sim.players[g].respawn_at.is_some());
    assert_eq!(game.sim_of(blue).players[g].hit.map(|h| h.start), fell);
    assert!(run_until(&mut game, Phase::Lobby, 10.0));
}

#[test]
fn the_time_limit_ends_a_round_in_a_draw() {
    let mut game = in_rounds(team_setup());
    assert!(game.server.set_time_limit(Some(3.0)));
    join(&mut game, Side::Blue, idle(Side::Blue));
    join(&mut game, Side::Gold, idle(Side::Gold));
    game.server.start_countdown();
    assert!(run_until(&mut game, Phase::Live, 10.0));
    let start = game.steps;
    assert!(run_until(&mut game, Phase::Results, 10.0));
    let lasted = (game.steps - start) as f32 * TICK;
    assert!((lasted - 3.0).abs() < 0.05, "{lasted} s");
    assert_eq!(game.server.round().last, Some(Outcome::Draw));
    assert_eq!(game.server.round().wins, [0, 0]);
}

#[test]
fn one_grenade_that_kills_the_last_of_both_sides_is_a_draw() {
    let mut game = in_rounds(face_off());
    let blue = join(&mut game, Side::Blue, thrower(Side::Blue));
    let gold = join(&mut game, Side::Gold, idle(Side::Gold));
    game.server.start_countdown();
    assert!(run_until(&mut game, Phase::Results, 20.0));
    assert_eq!(game.server.round().last, Some(Outcome::Draw));
    game.run(0.5);
    let (b, g) = (slot(&game, blue), slot(&game, gold));
    let mut dead = kills(&game, blue).iter().map(|k| k.1).collect::<Vec<_>>();
    dead.sort();
    assert_eq!(dead, [b.min(g), b.max(g)]);
}

#[test]
fn the_last_throw_of_a_player_shot_dead_still_counts() {
    let mut game = in_rounds(face_off());
    let blue = join(&mut game, Side::Blue, thrower(Side::Blue));
    // Gold fires once the grenade is in the air, and kills Blue before it goes off
    let gold = join(&mut game, Side::Gold, shooter(Side::Gold, |sim| !sim.thrown.is_empty()));
    game.server.start_countdown();
    assert!(run_until(&mut game, Phase::Results, 20.0));
    game.run(0.5);
    let (b, g) = (slot(&game, blue), slot(&game, gold));
    let k = kills(&game, gold);
    assert_eq!(k.len(), 2, "{k:?}");
    assert_eq!((k[0].0, k[0].1), (Some(g), b), "Blue is shot first: {k:?}");
    assert_eq!(k[1], (Some(b), g, Cause::Grenade), "then Blue's grenade kills Gold: {k:?}");
    // Gold was alone for a moment, but the grenade was waited for
    assert_eq!(game.server.round().last, Some(Outcome::Draw));
}

#[test]
fn the_session_is_locked_once_the_countdown_starts() {
    let mut game = in_rounds(team_setup());
    join(&mut game, Side::Blue, idle(Side::Blue));
    join(&mut game, Side::Gold, idle(Side::Gold));
    assert!(game.server.set_time_limit(Some(60.0)));
    assert!(game.server.start_countdown());
    game.run(1.0);
    assert!(!game.server.set_time_limit(Some(10.0)));
    assert!(!game.server.set_friendly_fire(false));
    assert!(!game.server.start_countdown());
    // The host can stop it, and change things then
    assert!(game.server.stop_countdown());
    assert_eq!(game.server.round().phase, Phase::Lobby);
    assert!(game.server.set_time_limit(Some(10.0)));
    assert!(game.server.set_friendly_fire(false));
    assert!(game.server.start_countdown());
    assert!(run_until(&mut game, Phase::Live, 10.0));
    assert!(!game.server.set_time_limit(Some(60.0)));
    let ends = game.server.round().ends.unwrap();
    assert!((ends - game.server.sim.time - 10.0).abs() < 0.1, "the round ends at {ends}, at {}", game.server.sim.time);
}

#[test]
fn no_one_moves_or_looks_around_until_everyone_has_loaded_and_two_more_seconds_pass() {
    let mut game = in_rounds(team_setup());
    // Blue walks and turns all along
    let blue = join(&mut game, Side::Blue, Box::new(|_, _, _, t| {
        let mut i = Input { forward: true, run: true, ..Input::default() };
        i.set_look(t as f32, 0.0);
        i
    }));
    let gold = join(&mut game, Side::Gold, idle(Side::Gold));
    let b = slot(&game, blue);
    // Gold's computer takes four seconds to load the round
    game.peers[gold].client.loading = true;
    game.server.start_countdown();
    assert!(run_until(&mut game, Phase::Loading, 10.0));
    game.run(0.5);
    let (pos, look) = (game.server_pos(b), game.server.sim.players[b].look);
    game.run(3.5);
    assert_eq!(game.server.round().phase, Phase::Loading);
    assert_eq!(game.server.round().ends.is_some(), true);
    game.peers[gold].client.loading = false;
    assert!(run_until(&mut game, Phase::Hold, 1.0));
    game.run(RELEASE_AFTER - 0.1);
    assert_eq!(game.server.round().phase, Phase::Hold);
    assert_eq!((game.server_pos(b), game.server.sim.players[b].look), (pos, look), "Blue moved before being let go");
    // Their own screen held them too, looking where they were when the hold began
    assert_eq!(game.me(blue), pos);
    assert_ne!(look, (0, 0));
    assert_eq!(game.sim_of(blue).players[b].look, look);
    assert!(run_until(&mut game, Phase::Live, 0.5));
    game.run(0.5);
    assert!((game.server_pos(b)[2] - pos[2]).abs() > 50.0, "Blue is not moving once let go");
    assert_ne!(game.server.sim.players[b].look, look);
}

#[test]
fn a_player_who_takes_too_long_to_load_is_dropped_and_the_round_goes_on() {
    let mut game = in_rounds(team_setup());
    join(&mut game, Side::Blue, idle(Side::Blue));
    join(&mut game, Side::Gold, idle(Side::Gold));
    let slow = join(&mut game, Side::Blue, idle(Side::Blue));
    let s = slot(&game, slow);
    game.peers[slow].client.loading = true;
    game.server.start_countdown();
    assert!(run_until(&mut game, Phase::Loading, 10.0));
    let start = game.steps;
    assert!(run_until(&mut game, Phase::Live, LOAD_TIMEOUT + RELEASE_AFTER + 1.0));
    let waited = (game.steps - start) as f32 * TICK;
    assert!(waited >= LOAD_TIMEOUT + RELEASE_AFTER - 0.1, "{waited} s");
    game.run(0.5);
    assert_eq!(game.peers[slow].client.status, Status::Lost);
    assert_eq!(game.server.players(), 2);
    assert!(!game.server.sim.players[s].active);
}

#[test]
fn a_side_whose_only_player_is_dropped_while_loading_calls_the_round_off() {
    let mut game = in_rounds(team_setup());
    let a = join(&mut game, Side::Blue, idle(Side::Blue));
    let gold = join(&mut game, Side::Gold, idle(Side::Gold));
    let c = join(&mut game, Side::Blue, idle(Side::Blue));
    game.peers[gold].client.loading = true;
    game.server.start_countdown();
    assert!(run_until(&mut game, Phase::Results, LOAD_TIMEOUT + 15.0));
    assert_eq!(game.server.round().last, Some(Outcome::CalledOff));
    assert_eq!(game.server.round().wins, [0, 0]);
    let record = game.server.records.last().unwrap();
    assert_eq!(record.outcome, Outcome::CalledOff);
    assert!(record.why.contains("Gold"), "{}", record.why);
    let mut seats = record.seats.clone();
    seats.sort_by_key(|s| s.slot);
    let seat = |peer: usize, side: Side, present: bool| Seat { slot: slot(&game, peer), side: Some(side), present };
    let mut want = vec![seat(a, Side::Blue, true), seat(gold, Side::Gold, false), seat(c, Side::Blue, true)];
    want.sort_by_key(|s| s.slot);
    assert_eq!(seats, want);
    // The players still there see it called off
    game.run(0.5);
    assert_eq!(game.peers[a].client.round.last, Some(Outcome::CalledOff));
}

#[test]
fn a_late_joiner_waits_for_the_next_round() {
    let mut game = in_rounds(team_setup());
    game.server.set_time_limit(Some(3.0));
    join(&mut game, Side::Blue, idle(Side::Blue));
    join(&mut game, Side::Gold, idle(Side::Gold));
    game.server.start_countdown();
    assert!(run_until(&mut game, Phase::Live, 10.0));
    let late = join(&mut game, Side::Gold, idle(Side::Gold));
    let l = slot(&game, late);
    game.run(0.5);
    assert!(game.server.sim.players[l].respawn_at.is_some(), "the late joiner is playing");
    assert!(game.sim_of(late).players[l].respawn_at.is_some());
    assert_eq!(game.server.round().phase, Phase::Live);
    // The round goes on to its end, and the next has them in it
    assert!(run_until(&mut game, Phase::Results, 5.0));
    assert_eq!(game.server.round().last, Some(Outcome::Draw));
    assert!(run_until(&mut game, Phase::Lobby, 10.0));
    game.server.start_countdown();
    assert!(run_until(&mut game, Phase::Live, 10.0));
    let p = &game.server.sim.players[l];
    assert!(p.respawn_at.is_none() && p.hp == PLAYER_HP);
    assert!(game.server.records.last().unwrap().seats.iter().all(|s| s.slot != l), "they were not in round 1");
}

#[test]
fn a_player_who_leaves_mid_round_counts_as_dead() {
    let mut game = in_rounds(team_setup());
    join(&mut game, Side::Blue, idle(Side::Blue));
    let gold = join(&mut game, Side::Gold, idle(Side::Gold));
    game.server.start_countdown();
    assert!(run_until(&mut game, Phase::Live, 10.0));
    game.run(1.0);
    let p = &mut game.peers[gold];
    p.client.leave(&mut p.link);
    p.on = false;
    assert!(run_until(&mut game, Phase::Results, 1.0));
    assert_eq!(game.server.round().last, Some(Outcome::Won(Side::Blue)));
    assert!(!game.server.records[0].seats.iter().find(|s| s.side == Some(Side::Gold)).unwrap().present);
}

#[test]
fn a_new_round_puts_everyone_back_alike_on_every_client() {
    let mut game = in_rounds(face_off());
    let blue = join(&mut game, Side::Blue, thrower(Side::Blue));
    let gold = join(&mut game, Side::Gold, idle(Side::Gold));
    game.server.set_auto_start(true);
    assert!(run_until(&mut game, Phase::Results, 20.0));
    assert!(game.server.sim.players.iter().all(|p| !p.active || p.respawn_at.is_some()));
    // The next round starts by itself, from the lobby
    assert!(run_until(&mut game, Phase::Hold, 20.0));
    game.run(0.5);
    assert_eq!(game.server.round().number, 2);
    let state = |sim: &Sim| sim.players.iter().filter(|p| p.active).map(|p| (p.player.pos, p.hp, p.respawn_at, p.side)).collect::<Vec<_>>();
    let server = state(&game.server.sim);
    for (i, side) in [(0, Side::Blue), (1, Side::Gold)] {
        assert_eq!((server[i].0, server[i].1, server[i].2, server[i].3), (face_off().bases[side.index()][0], PLAYER_HP, None, Some(side)));
    }
    for p in [blue, gold] {
        assert_eq!(state(game.sim_of(p)), server, "peer {p}");
        assert!(game.sim_of(p).thrown.is_empty());
        assert_eq!(game.peers[p].client.round.number, 2);
    }
    // With their grenades back, and their scores kept
    let b = &game.server.sim.players[slot(&game, blue)];
    assert_eq!((b.grenades_left, b.deaths), (team_setup().grenades, 1));
}

#[test]
fn without_sides_the_last_one_standing_wins() {
    let mut game = in_rounds(Setup { pvp: true, ..demo_setup() });
    // The first two start 80 apart along the south wall; the first faces the second
    let a = game.join_seeing(|sim: &Sim, _, _, _| {
        let mut i = Input { fire: !sim.frozen, ..Input::default() };
        i.set_look(-90.0, 0.0);
        i
    });
    game.run(0.2);
    game.join(|_| Input::default());
    game.run(0.2);
    game.server.start_countdown();
    assert!(run_until(&mut game, Phase::Results, 20.0));
    assert_eq!(game.server.round().last, Some(Outcome::Survived(slot(&game, a) as u8)));
}

#[test]
fn a_match_in_rounds_plays_back_the_same() {
    let mut game = Game::with_setup(Conditions::poor(), face_off());
    game.server.play_rounds();
    game.server.record();
    game.server.set_auto_start(true);
    join(&mut game, Side::Blue, thrower(Side::Blue));
    join(&mut game, Side::Gold, shooter(Side::Gold, |sim| !sim.thrown.is_empty()));
    assert!(run_until(&mut game, Phase::Results, 20.0));
    let late = join(&mut game, Side::Blue, idle(Side::Blue));
    assert!(run_until(&mut game, Phase::Live, 20.0));
    game.run(2.0);
    assert_eq!(game.server.round().number, 2);
    assert!(game.sim_of(late).players[slot(&game, late)].respawn_at.is_none());
    let rec = game.server.recording().unwrap();
    let rounds = rec.ticks.iter().flat_map(|t| &t.changes).filter(|c| matches!(c, Change::Round { .. })).count();
    assert_eq!(rounds, 2);
    rec.replay().unwrap();
}

/// A peer's watch, moved on by what their screen shows now; and where its camera is
fn watching(game: &Game, peer: usize, watch: &mut Watch) -> Option<Pose> {
    let p = &game.peers[peer];
    let (Some(sim), Some(view)) = (p.sim.as_ref(), p.view.as_ref()) else { return None };
    let cx = WatchContext { sim, frame: view, me: p.client.slot(), round: p.client.round, team_game: true, connected: p.client.status == Status::Connected };
    watch.update(&cx);
    watch.camera(&cx)
}

#[test]
fn a_dead_player_watches_a_living_teammate_and_never_the_other_side() {
    // Blue's first in line of Gold's first, Blue's second well aside
    let setup = Setup { bases: [vec![[-100.0, 0.0, -100.0], [100.0, 0.0, -300.0]], vec![[-100.0, 0.0, 100.0], [300.0, 0.0, 300.0]]], ..team_setup() };
    let mut game = in_rounds(setup);
    let shot = join(&mut game, Side::Blue, idle(Side::Blue));
    let gold = join(&mut game, Side::Gold, shooter(Side::Gold, |sim| !sim.frozen));
    let mate = join(&mut game, Side::Blue, idle(Side::Blue));
    let other = join(&mut game, Side::Gold, idle(Side::Gold));
    let (m, golds) = (slot(&game, mate), [slot(&game, gold), slot(&game, other)]);
    game.server.start_countdown();
    assert!(run_until(&mut game, Phase::Live, 10.0));
    let mut watch = Watch::default();
    let mut watched = Vec::new();
    for _ in 0..(5.0 / TICK) as usize {
        game.step();
        watching(&game, shot, &mut watch);
        watched.extend(watch.target());
    }
    assert!(game.server.sim.players[slot(&game, shot)].respawn_at.is_some(), "Blue's first was not shot");
    assert!(!watched.is_empty(), "no one was watched");
    assert!(watched.iter().all(|&s| s == m), "watched {watched:?}, not only {m}, never Gold's {golds:?}");
}

#[test]
fn a_late_joiner_while_loading_watches_at_once_held_until_let_go() {
    let mut game = in_rounds(team_setup());
    // Blue's first walks on all along, once free
    join(&mut game, Side::Blue, Box::new(|_, _, _, _| Input { forward: true, run: true, ..Input::default() }));
    let gold = join(&mut game, Side::Gold, idle(Side::Gold));
    // Gold takes a second to load, so that the late joiner comes during loading
    game.peers[gold].client.loading = true;
    game.server.start_countdown();
    assert!(run_until(&mut game, Phase::Loading, 10.0));
    let late = game.join_seeing(|_: &Sim, _, _, _| Input::default());
    game.peers[late].client.ask_side(Some(Side::Blue));
    game.run(0.5);
    assert_eq!(game.server.round().phase, Phase::Loading);
    assert!(game.server.sim.players[slot(&game, late)].respawn_at.is_some(), "the late joiner is in the round");
    game.peers[gold].client.loading = false;
    let mut watch = Watch::default();
    let mut poses = Vec::new();
    while game.server.round().phase != Phase::Live {
        game.step();
        poses.extend(watching(&game, late, &mut watch));
    }
    assert!(!poses.is_empty(), "not watching while held");
    assert!(poses.iter().all(|p| *p == poses[0]), "the held view moved");
    game.run(1.0);
    let moved = watching(&game, late, &mut watch).unwrap();
    assert!((moved.eye[2] - poses[0].eye[2]).abs() > 50.0, "the view did not follow once let go");
}


