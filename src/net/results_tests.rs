//! Names and the round's results, played out by clients and a server on a pretend network: who
//! is called what, who played a round, what they did, and the results screen's wait

use crate::net::lab::{Game, team_setup};
use crate::net::link::{Conditions, Link};
use crate::net::proto::{LobbyCommand, ToServer};
use crate::net::record::{ClientRecording, ServerRecording};
use crate::net::round::{Outcome, Phase};
use crate::net::server::DEBRIEF_WAIT;
use crate::sim::{Ending, Event, Input, Side, TICK, Thrown};

const KEY: u64 = 0x0be7_a11e_d0c5_ba5e;

/// A team game in rounds whose host joins with `KEY`
fn hosted() -> Game {
    let mut game = Game::with_setup(Conditions::lan(), team_setup());
    game.server.set_host_key(KEY, false);
    game.server.play_rounds();
    game
}

/// Someone joins on `side` with `key` and asks for `name`, and is in after a moment. They
/// continue from the results only when told.
fn join(game: &mut Game, side: Side, key: u64, name: &str) -> usize {
    let peer = game.join_on(side, |_| Input::default());
    game.peers[peer].client.set_key(key);
    game.peers[peer].client.set_name(name);
    game.peers[peer].continues = false;
    game.run(0.3);
    peer
}

fn slot(game: &Game, peer: usize) -> usize {
    game.peers[peer].client.slot()
}

fn run_until(game: &mut Game, phase: Phase, seconds: f32) -> bool {
    for _ in 0..(seconds / TICK) as usize {
        if game.server.round().phase == phase {
            return true;
        }
        game.step();
    }
    game.server.round().phase == phase
}

fn leave(game: &mut Game, peer: usize) {
    let p = &mut game.peers[peer];
    p.client.leave(&mut p.link);
    p.on = false;
}

/// A lobby command sent as is, past the client's queue
fn send(game: &mut Game, peer: usize, seq: u32, generation: u32, command: LobbyCommand) {
    let bytes = ToServer::Lobby { token: 1000 + peer as u32, seq, generation, command }.encode();
    let to = game.server_link.addr();
    game.peers[peer].link.send(to, &bytes);
}

/// The host starts the round, and Gold falls at once, so that Blue wins it
fn play_a_round(game: &mut Game, host: usize) {
    game.lobby(host, LobbyCommand::Start);
    assert!(run_until(game, Phase::Live, 20.0));
    game.run(0.5);
    let now = game.server.sim.time;
    for p in game.server.sim.players.iter_mut().filter(|p| p.active && p.side == Some(Side::Gold)) {
        p.respawn_at = Some(now);
    }
    assert!(run_until(game, Phase::Debrief, 10.0));
    game.run(0.3);
}

fn continued(game: &Game, peer: usize) -> bool {
    let me = (slot(game, peer), game.peers[peer].client.names.iter().find(|n| n.0 == slot(game, peer)).map_or(0, |n| n.1));
    game.peers[peer].client.continued.contains(&me)
}

#[test]
fn names_are_cleaned_made_unique_and_known_to_everyone_late_joiners_too() {
    let mut game = hosted();
    let a = join(&mut game, Side::Blue, KEY, "  [RAINBOW] Pie4Foo\u{7} ");
    let b = join(&mut game, Side::Gold, 0, "[rainbow] pie4foo");
    let c = join(&mut game, Side::Gold, 0, "");
    game.run(0.5);
    let want = [(slot(&game, a), "[RAINBOW] Pie4Foo".to_string()), (slot(&game, b), "[rainbow] pie4foo 2".to_string()), (slot(&game, c), format!("PLAYER {}", slot(&game, c) + 1))];
    let late = join(&mut game, Side::Blue, 0, "Villico");
    game.run(0.5);
    for p in [a, b, c, late] {
        for (s, name) in &want {
            assert_eq!(game.peers[p].client.name_of(*s), *name, "peer {p}");
        }
        assert_eq!(game.peers[p].client.name_of(slot(&game, late)), "Villico");
    }
    // A bot's is made the same way
    let bot = game.join_bot(crate::net::bot::Skill::normal(), 1);
    game.peers[bot].client.set_name("BOT");
    let bot2 = game.join_bot(crate::net::bot::Skill::normal(), 2);
    game.peers[bot2].client.set_name("BOT");
    game.run(0.5);
    // They join in the same tick, so either may be first
    let mut bots = [game.peers[a].client.name_of(slot(&game, bot)), game.peers[a].client.name_of(slot(&game, bot2))];
    bots.sort();
    assert_eq!(bots, ["BOT", "BOT 2"]);
}

#[test]
fn a_kill_is_named_as_it_was_though_the_watcher_missed_the_killer_and_the_slot_has_changed() {
    // Play with no rounds, so a kill is simply a kill
    let mut game = Game::with_setup(Conditions::lan(), team_setup());
    let watcher = join(&mut game, Side::Blue, 0, "Watcher");
    let target = join(&mut game, Side::Gold, 0, "Target");
    // The watcher hears nothing while the thrower comes, kills and goes
    game.connect_over(watcher, Conditions { loss: 1.0, ..Conditions::NONE });
    let thrower = join(&mut game, Side::Blue, 0, "Thrower");
    let (t, v) = (slot(&game, thrower), slot(&game, target));
    let sim = &mut game.server.sim;
    let by = sim.occupant(t);
    let at = sim.players[v].player.pos;
    sim.thrown.push(Thrown { grenade: crate::grenade::Grenade { pos: [at[0], at[1] + 100.0, at[2]], vel: [0.0; 3], fuse: 0.0, spin: 0.0, at_rest: true }, owner: t, by, side: Some(Side::Blue) });
    game.run(0.2);
    leave(&mut game, thrower);
    game.run(0.2);
    let newcomer = join(&mut game, Side::Gold, 0, "Newcomer");
    assert_eq!(slot(&game, newcomer), t, "the newcomer has the thrower's slot");
    // Back, and several snapshots at once after a stall
    game.connect_over(watcher, Conditions::lan());
    let now = game.steps as f64 * TICK as f64;
    game.peers[watcher].frozen = Some((now, now + 0.3));
    game.run(1.0);
    let kills: Vec<(String, String)> = game.peers[watcher].events.iter().filter_map(|e| match e {
        Event::Kill { killer: Some(k), victim, killer_name, victim_name, .. } if (*k, *victim) == (t, v) => Some((killer_name.clone(), victim_name.clone())),
        _ => None,
    }).collect();
    assert_eq!(kills, [("Thrower".to_string(), "Target".to_string())]);
    assert_eq!(game.peers[watcher].client.name_of(t), "Newcomer");
}

#[test]
fn the_results_screen_waits_for_everyone_who_played_and_is_still_here() {
    let mut game = hosted();
    let host = join(&mut game, Side::Blue, KEY, "Host");
    let blue = join(&mut game, Side::Blue, 0, "Blue");
    let gold = join(&mut game, Side::Gold, 0, "Gold");
    let goes = join(&mut game, Side::Gold, 0, "Goes");
    play_a_round(&mut game, host);
    // Everyone sees the table, and is held behind it
    let table = game.server.results().cloned().unwrap();
    assert_eq!(table.outcome, Outcome::Won(Side::Blue));
    for p in [host, blue, gold, goes] {
        assert_eq!(game.peers[p].client.results.as_ref(), Some(&table));
    }
    assert!(game.server.sim.frozen);
    // A Continue made for another screen is refused; then each continues once
    let stale = game.server.session().generation - 1;
    send(&mut game, blue, 1, stale, LobbyCommand::Continue);
    game.run(0.3);
    assert!(!continued(&game, blue));
    game.lobby(blue, LobbyCommand::Continue);
    game.lobby(blue, LobbyCommand::Continue);
    game.run(0.3);
    assert!(continued(&game, blue));
    // Only the host goes to the lobby, and only once they have continued
    game.lobby(blue, LobbyCommand::ToLobby);
    game.lobby(host, LobbyCommand::ToLobby);
    game.run(0.3);
    assert_eq!(game.server.round().phase, Phase::Debrief);
    // One who joins now is not waited for, and may speak; one who leaves is waited for no more
    let late = join(&mut game, Side::Blue, 0, "Late");
    game.lobby(late, LobbyCommand::Say("what did I miss".to_string()));
    leave(&mut game, goes);
    game.lobby(gold, LobbyCommand::Continue);
    game.run(0.3);
    assert!(game.peers[host].events.iter().any(|e| matches!(e, Event::Chat { name, .. } if name == "Late")));
    assert_eq!(game.server.round().phase, Phase::Debrief, "the host has not continued");
    game.lobby(host, LobbyCommand::Continue);
    game.run(0.3);
    assert_eq!(game.server.round().phase, Phase::Lobby);
    for p in [host, blue, gold, late] {
        assert!(game.peers[p].client.results.is_none(), "peer {p} still has the results");
    }
}

#[test]
fn the_hosts_second_click_takes_everyone_to_the_lobby_and_the_wait_has_an_end() {
    let mut game = hosted();
    let host = join(&mut game, Side::Blue, KEY, "Host");
    join(&mut game, Side::Gold, 0, "Gold");
    play_a_round(&mut game, host);
    game.lobby(host, LobbyCommand::Continue);
    game.run(0.3);
    assert_eq!(game.server.round().phase, Phase::Debrief);
    game.lobby(host, LobbyCommand::ToLobby);
    game.run(0.3);
    assert_eq!(game.server.round().phase, Phase::Lobby);
    // No one continues: the lobby comes all the same
    play_a_round(&mut game, host);
    game.run(DEBRIEF_WAIT - 1.5);
    assert_eq!(game.server.round().phase, Phase::Debrief);
    assert!(run_until(&mut game, Phase::Lobby, 2.0));
}

#[test]
fn a_lone_host_with_bots_goes_on_with_one_click() {
    let mut game = hosted();
    let host = join(&mut game, Side::Blue, KEY, "Host");
    for seed in 0..3 {
        let bot = game.join_bot(crate::net::bot::Skill::normal(), seed);
        game.peers[bot].client.ask_side(Some(if seed == 0 { Side::Blue } else { Side::Gold }));
    }
    game.run(0.5);
    play_a_round(&mut game, host);
    game.run(0.5);
    assert_eq!(game.server.round().phase, Phase::Debrief, "the bots have continued, the host has not");
    game.lobby(host, LobbyCommand::Continue);
    game.run(0.3);
    assert_eq!(game.server.round().phase, Phase::Lobby);
}

#[test]
fn the_table_keeps_who_played_though_they_left_and_someone_else_took_their_slot() {
    let mut game = hosted();
    game.server.record();
    let host = game.join_recorded(|_| Input::default());
    game.peers[host].client.set_key(KEY);
    game.peers[host].client.set_name("Host");
    game.peers[host].client.ask_side(Some(Side::Blue));
    game.peers[host].continues = false;
    game.run(0.3);
    let gone = join(&mut game, Side::Blue, 0, "Gone");
    let gold = join(&mut game, Side::Gold, 0, "Gold");
    game.lobby(host, LobbyCommand::Start);
    assert!(run_until(&mut game, Phase::Live, 20.0));
    game.run(0.5);
    let old = slot(&game, gone);
    leave(&mut game, gone);
    game.run(0.3);
    // The newcomer takes the slot, on the other side, and waits for the next round
    let newcomer = join(&mut game, Side::Gold, 0, "Newcomer");
    assert_eq!(slot(&game, newcomer), old);
    // Gold goes too, which leaves Blue the winner. A recording keeps that, as it would not keep
    // a death made by hand.
    leave(&mut game, gold);
    assert!(run_until(&mut game, Phase::Debrief, 10.0));
    game.run(0.3);
    let table = game.peers[newcomer].client.results.clone().unwrap();
    let names: Vec<(&str, bool, Ending)> = table.rows.iter().map(|r| (r.name.as_str(), r.left, r.end)).collect();
    assert_eq!(names.len(), 3);
    for want in [("Host", false, Ending::Alive), ("Gone", true, Ending::Alive), ("Gold", true, Ending::Alive)] {
        assert!(names.contains(&want), "{want:?} in {names:?}");
    }
    // The newcomer did not play it, though they have the slot of one who did
    let row = table.rows.iter().find(|r| r.slot as usize == old).unwrap();
    assert_ne!(row.joined, game.peers[newcomer].client.names.iter().find(|n| n.0 == old).unwrap().1);
    // Nor are they waited for
    game.lobby(host, LobbyCommand::Continue);
    game.run(0.3);
    assert_eq!(game.server.round().phase, Phase::Lobby);

    // The server's recording makes the same table again, and the host's plays back
    let rec = game.server.recording().unwrap();
    let back = ServerRecording::from_bytes(&rec.to_bytes()).unwrap();
    assert_eq!(back, rec);
    let sim = back.replay().unwrap();
    assert_eq!(sim.checksum(), game.server.sim.checksum());
    let seen = game.peers[host].recorder.as_ref().unwrap().recording(game.peers[host].sim.as_ref().unwrap());
    ClientRecording::from_bytes(&seen.to_bytes()).unwrap().replay(|_, _, _, _| {}).unwrap();
}

#[test]
fn a_round_called_off_before_it_is_played_is_shown_with_nothing_counted() {
    let mut game = hosted();
    let host = join(&mut game, Side::Blue, KEY, "Host");
    let gold = join(&mut game, Side::Gold, 0, "Gold");
    game.lobby(host, LobbyCommand::Start);
    assert!(run_until(&mut game, Phase::Loading, 10.0));
    // Gold goes before the round is let go, so it cannot be played
    leave(&mut game, gold);
    assert!(run_until(&mut game, Phase::Debrief, 10.0));
    let table = game.server.results().cloned().unwrap();
    assert_eq!(table.outcome, Outcome::CalledOff);
    assert_eq!(table.played, 0);
    assert!(table.rows.iter().all(|r| r.stats == Default::default()));
    assert!(table.rows.iter().any(|r| r.name == "Gold" && r.left));
}
