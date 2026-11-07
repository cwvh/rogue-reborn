//! The lobby, played out by clients and a server on a pretend network: the host, the commands
//! everyone sends, and the settings they change

use crate::net::client::Status;
use crate::net::lab::{Game, team_setup};
use crate::net::link::{Conditions, Link};
use crate::net::proto::{CHAT_LENGTH, LobbyCommand, ToServer};
use crate::net::record::ServerRecording;
use crate::net::round::Phase;
use crate::net::server::HOST_WAIT;
use crate::sim::{Event, Input, Side, TICK};

const KEY: u64 = 0x5ec2_e7ab_cdef_0123;

/// A team game in rounds whose host joins with `KEY`, which ends without them if `hosted`
fn hosted(hosted: bool) -> Game {
    let mut game = Game::with_setup(Conditions::lan(), team_setup());
    game.server.set_host_key(KEY, hosted);
    game.server.play_rounds();
    game
}

/// Someone joins on `side`, with `key`, and is in after a moment
fn join(game: &mut Game, side: Side, key: u64) -> usize {
    let peer = game.join_on(side, |_| Input::default());
    game.peers[peer].client.set_key(key);
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

/// A lobby command sent as is, past the client's queue
fn send(game: &mut Game, peer: usize, seq: u32, generation: u32, command: LobbyCommand) {
    let bytes = ToServer::Lobby { token: 1000 + peer as u32, seq, generation, command }.encode();
    let to = game.server_link.addr();
    game.peers[peer].link.send(to, &bytes);
}

fn ready(game: &Game, peer: usize) -> bool {
    game.peers[peer].client.ready.contains(&slot(game, peer))
}

#[test]
fn the_host_starts_and_stops_the_countdown_and_no_one_else_can() {
    let mut game = hosted(false);
    let host = join(&mut game, Side::Blue, KEY);
    let other = join(&mut game, Side::Gold, 0);
    game.run(1.0);
    // A hosted server waits for its host, though both sides are there
    assert_eq!(game.server.round().phase, Phase::Lobby);
    for p in [host, other] {
        assert_eq!(game.peers[p].client.session.host, Some(slot(&game, host) as u8));
    }
    game.lobby(other, LobbyCommand::Start);
    game.run(0.5);
    assert_eq!(game.server.round().phase, Phase::Lobby, "a player who is not the host started it");
    assert!(game.peers[other].client.lobby_waiting().next().is_none(), "a refused command is still waiting");
    game.lobby(host, LobbyCommand::Start);
    game.run(0.5);
    assert_eq!(game.server.round().phase, Phase::Countdown);
    game.lobby(other, LobbyCommand::Stop);
    game.run(0.5);
    assert_eq!(game.server.round().phase, Phase::Countdown, "a player who is not the host stopped it");
    // The host's Stop is made in the countdown, which is a lobby generation of its own
    game.lobby(host, LobbyCommand::Stop);
    game.run(0.5);
    assert_eq!(game.server.round().phase, Phase::Lobby);
}

#[test]
fn a_host_needs_a_key_other_than_zero_and_there_is_only_one() {
    // No key on the server: joining with none makes no one the host
    let mut game = Game::with_setup(Conditions::lan(), team_setup());
    game.server.set_host_key(0, true);
    game.server.play_rounds();
    let a = join(&mut game, Side::Blue, 0);
    assert_eq!(game.peers[a].client.session.host, None);
    // With a key, the first to join with it is the host, and a second is a player like any
    let mut game = hosted(false);
    let first = join(&mut game, Side::Blue, KEY);
    let second = join(&mut game, Side::Gold, KEY);
    game.run(0.5);
    assert_eq!(game.peers[second].client.session.host, Some(slot(&game, first) as u8));
    game.lobby(second, LobbyCommand::FriendlyFire(false));
    game.run(0.5);
    assert!(game.server.sim.friendly_fire);
}

#[test]
fn the_host_starts_with_players_not_ready_but_not_with_a_side_empty() {
    let mut game = hosted(false);
    let host = join(&mut game, Side::Blue, KEY);
    game.lobby(host, LobbyCommand::Start);
    game.run(0.5);
    assert_eq!(game.server.round().phase, Phase::Lobby, "started with no one on Gold");
    let gold = join(&mut game, Side::Gold, 0);
    assert!(!ready(&game, gold));
    game.lobby(host, LobbyCommand::Start);
    game.run(0.5);
    assert_eq!(game.server.round().phase, Phase::Countdown);
}

#[test]
fn players_change_side_and_say_ready_in_the_lobby_and_each_round_asks_again() {
    let mut game = hosted(false);
    let host = join(&mut game, Side::Blue, KEY);
    let p = join(&mut game, Side::Gold, 0);
    let s = slot(&game, p);
    game.lobby(p, LobbyCommand::Ready(true));
    game.run(0.5);
    assert!(ready(&game, p));
    assert!(!ready(&game, host));
    // A side change is the lobby's, and leaves them to say ready again
    game.lobby(p, LobbyCommand::Side(Side::Blue));
    game.run(0.5);
    assert_eq!(game.server.sim.players[s].side, Some(Side::Blue));
    assert_eq!(game.sim_of(p).players[s].side, Some(Side::Blue));
    assert!(!ready(&game, p));
    game.lobby(p, LobbyCommand::Ready(true));
    // Both on Blue, and the host may not start with Gold empty
    game.lobby(host, LobbyCommand::Start);
    game.run(0.5);
    assert_eq!(game.server.round().phase, Phase::Lobby);
    game.lobby(host, LobbyCommand::Side(Side::Gold));
    game.lobby(host, LobbyCommand::Start);
    game.run(0.5);
    assert_eq!(game.server.round().phase, Phase::Countdown);
    // Locked once the countdown runs
    game.lobby(p, LobbyCommand::Ready(false));
    game.lobby(p, LobbyCommand::Side(Side::Gold));
    game.run(0.5);
    assert!(ready(&game, p));
    assert_eq!(game.server.sim.players[s].side, Some(Side::Blue));
    // Once the round begins no one is ready, so the next lobby asks again
    assert!(run_until(&mut game, Phase::Live, 10.0));
    assert!(!ready(&game, p));
}

#[test]
fn a_bot_says_ready_in_every_lobby() {
    let mut game = hosted(false);
    let host = join(&mut game, Side::Blue, KEY);
    let bot = game.join_bot(crate::net::bot::Skill::easy(), 3);
    game.peers[bot].client.ask_side(Some(Side::Gold));
    game.run(1.0);
    assert!(ready(&game, bot));
    // A short round, which the time ends if no one wins it
    game.lobby(host, LobbyCommand::TimeLimit(Some(2)));
    game.lobby(host, LobbyCommand::Start);
    assert!(run_until(&mut game, Phase::Live, 10.0));
    assert!(!ready(&game, bot));
    assert!(run_until(&mut game, Phase::Lobby, 20.0));
    game.run(1.0);
    assert!(ready(&game, bot));
}

#[test]
fn commands_are_taken_once_each_in_order_whatever_the_network_does() {
    let mut game = hosted(false);
    let host = join(&mut game, Side::Blue, KEY);
    let other = join(&mut game, Side::Gold, 0);
    let g = game.server.session().generation;
    // The second comes before the first, and is not taken
    send(&mut game, host, 2, g, LobbyCommand::FriendlyFire(false));
    game.run(0.2);
    assert!(game.server.sim.friendly_fire);
    send(&mut game, host, 1, g, LobbyCommand::TimeLimit(Some(60)));
    game.run(0.2);
    assert_eq!(game.server.session().time_limit, Some(60));
    assert!(game.server.sim.friendly_fire);
    send(&mut game, host, 2, g, LobbyCommand::FriendlyFire(false));
    game.run(0.2);
    assert!(!game.server.sim.friendly_fire);
    // The first again, late: it is not taken twice
    send(&mut game, host, 3, g, LobbyCommand::TimeLimit(Some(90)));
    send(&mut game, host, 1, g, LobbyCommand::TimeLimit(Some(60)));
    game.run(0.2);
    assert_eq!(game.server.session().time_limit, Some(90));
    // Each player is told only of their own
    game.run(0.2);
    let snaps = |p: usize| game.peers[p].client.lobby_waiting().count();
    assert_eq!(snaps(other), 0);

    // Over a bad network, several in a row all come through, in order
    let mut game = hosted(false);
    let host = join(&mut game, Side::Blue, KEY);
    join(&mut game, Side::Gold, 0);
    game.net.set_conditions(Conditions { loss: 0.4, duplicate: 0.2, jitter: 0.03, latency: 0.02, ..Conditions::NONE });
    for t in [60, 120, 30] {
        game.lobby(host, LobbyCommand::TimeLimit(Some(t)));
    }
    game.lobby(host, LobbyCommand::FriendlyFire(false));
    game.lobby(host, LobbyCommand::FriendlyFire(true));
    game.lobby(host, LobbyCommand::FriendlyFire(false));
    game.run(8.0);
    assert!(game.peers[host].client.lobby_waiting().next().is_none());
    assert_eq!(game.server.session().time_limit, Some(30));
    assert!(!game.server.sim.friendly_fire);
}

/// What a peer has been told players said, in order
fn heard(game: &Game, peer: usize) -> Vec<(usize, String)> {
    game.peers[peer].events.iter().filter_map(|e| match e {
        Event::Chat { player, text, .. } => Some((*player, text.clone())),
        _ => None,
    }).collect()
}

#[test]
fn what_players_say_reaches_everyone_once_in_order_but_not_while_a_round_is_played() {
    let mut game = hosted(false);
    let host = join(&mut game, Side::Blue, KEY);
    let other = join(&mut game, Side::Gold, 0);
    let (h, o) = (slot(&game, host), slot(&game, other));
    game.net.set_conditions(Conditions { loss: 0.3, duplicate: 0.2, jitter: 0.03, latency: 0.02, ..Conditions::NONE });
    // Cleaned up as it may be said, and nothing is not said
    game.lobby(other, LobbyCommand::Say("  hello\u{7} there  ".to_string()));
    game.lobby(host, LobbyCommand::Say("   ".to_string()));
    game.lobby(host, LobbyCommand::Say("hi".to_string()));
    game.lobby(other, LobbyCommand::Say("x".repeat(CHAT_LENGTH + 10)));
    game.run(6.0);
    let long = "x".repeat(CHAT_LENGTH);
    for p in [host, other] {
        let mut said = heard(&game, p);
        // The two players' lines may cross on the way
        said.sort_by_key(|(who, _)| *who);
        let mut want = vec![(o, "hello there".to_string()), (o, long.clone()), (h, "hi".to_string())];
        want.sort_by_key(|(who, _)| *who);
        assert_eq!(said, want, "peer {p}");
    }

    // Not while the round is played: players use their voices then
    game.net.set_conditions(Conditions::lan());
    game.peers[other].continues = false;
    game.lobby(host, LobbyCommand::Start);
    assert!(run_until(&mut game, Phase::Live, 20.0));
    send(&mut game, other, 3, 0, LobbyCommand::Say("go go".to_string()));
    game.run(0.5);
    for p in [host, other] {
        assert!(!heard(&game, p).iter().any(|(_, t)| t == "go go"), "said in the round");
    }
    // On the results screen, said, though numbered for a lobby long gone
    let now = game.server.sim.time;
    game.server.sim.players[h].respawn_at = Some(now);
    assert!(run_until(&mut game, Phase::Debrief, 10.0));
    send(&mut game, other, 4, 0, LobbyCommand::Say("gg".to_string()));
    game.run(0.5);
    for p in [host, other] {
        assert_eq!(heard(&game, p).last(), Some(&(o, "gg".to_string())));
    }
}

#[test]
fn a_command_made_for_another_lobby_or_countdown_is_refused_and_the_next_still_works() {
    let mut game = hosted(false);
    assert!(game.server.set_time_limit(Some(1.0)));
    let host = join(&mut game, Side::Blue, KEY);
    join(&mut game, Side::Gold, 0);
    let lobby = game.server.session().generation;
    send(&mut game, host, 1, lobby, LobbyCommand::Start);
    game.run(0.2);
    assert_eq!(game.server.round().phase, Phase::Countdown);
    let countdown = game.server.session().generation;
    send(&mut game, host, 2, countdown, LobbyCommand::Stop);
    game.run(0.2);
    assert_eq!(game.server.round().phase, Phase::Lobby);
    // A Start held back from the first lobby, and a Stop for the countdown that is gone
    send(&mut game, host, 3, lobby, LobbyCommand::Start);
    game.run(0.2);
    assert_eq!(game.server.round().phase, Phase::Lobby);
    let again = game.server.session().generation;
    send(&mut game, host, 4, again, LobbyCommand::Start);
    game.run(0.2);
    assert_eq!(game.server.round().phase, Phase::Countdown);
    send(&mut game, host, 5, countdown, LobbyCommand::Stop);
    game.run(0.2);
    assert_eq!(game.server.round().phase, Phase::Countdown);
    // A Ready from the first lobby, arriving in the next one
    // Its own Continue, numbered by the client, is out of turn here, so the results wait their time
    assert!(run_until(&mut game, Phase::Lobby, 60.0));
    send(&mut game, host, 6, lobby, LobbyCommand::Ready(true));
    game.run(0.2);
    assert!(!ready(&game, host));
    let now = game.server.session().generation;
    send(&mut game, host, 7, now, LobbyCommand::Ready(true));
    game.run(0.2);
    assert!(ready(&game, host));
}

#[test]
fn a_new_pair_of_bases_starts_the_next_round_there_for_server_and_client() {
    let mut game = hosted(false);
    let host = join(&mut game, Side::Blue, KEY);
    let gold = join(&mut game, Side::Gold, 0);
    let setup = team_setup();
    // Two changes in a row: the second holds
    game.lobby(host, LobbyCommand::BasePair(1));
    game.lobby(host, LobbyCommand::BasePair(0));
    game.lobby(host, LobbyCommand::BasePair(1));
    game.run(1.0);
    assert_eq!(game.server.session().base_pair, 1);
    let swapped = &setup.base_pairs[1].bases;
    assert_eq!(&game.server.sim.bases, swapped);
    for p in [host, gold] {
        assert_eq!(&game.sim_of(p).bases, swapped);
    }
    // A pair the level lacks is refused
    game.lobby(host, LobbyCommand::BasePair(2));
    game.run(0.5);
    assert_eq!(game.server.session().base_pair, 1);
    game.lobby(host, LobbyCommand::Start);
    assert!(run_until(&mut game, Phase::Hold, 10.0));
    // Blue starts where Gold did
    let (b, g) = (slot(&game, host), slot(&game, gold));
    assert!(swapped[0].contains(&game.server_pos(b)), "Blue is not at the new pair's base");
    assert!(swapped[1].contains(&game.server_pos(g)));
    game.run(0.5);
    assert_eq!(game.me(host), game.server_pos(b));
}

#[test]
fn friendly_fire_set_in_the_lobby_reaches_everyone_and_late_joiners() {
    let mut game = hosted(false);
    let host = join(&mut game, Side::Blue, KEY);
    let gold = join(&mut game, Side::Gold, 0);
    game.lobby(host, LobbyCommand::FriendlyFire(false));
    game.lobby(host, LobbyCommand::BasePair(1));
    // Someone joins while the host changes things
    let late = game.join_on(Side::Blue, |_| Input::default());
    game.run(1.0);
    for p in [host, gold, late] {
        assert!(!game.sim_of(p).friendly_fire);
        assert_eq!(game.sim_of(p).bases, game.server.sim.bases);
    }
    game.lobby(host, LobbyCommand::FriendlyFire(true));
    game.run(0.5);
    for p in [host, gold, late] {
        assert!(game.sim_of(p).friendly_fire);
    }
}

#[test]
fn a_hosted_game_ends_when_its_host_leaves_or_goes_quiet_or_never_comes() {
    let mut game = hosted(true);
    let host = join(&mut game, Side::Blue, KEY);
    let other = join(&mut game, Side::Gold, 0);
    let p = &mut game.peers[host];
    p.client.leave(&mut p.link);
    game.peers[host].on = false;
    game.run(0.5);
    assert_eq!(game.server.ended(), Some("the host left"));
    assert_eq!(game.peers[other].client.status, Status::SentAway("the host left".to_string()));

    let mut game = hosted(true);
    let host = join(&mut game, Side::Blue, KEY);
    join(&mut game, Side::Gold, 0);
    game.peers[host].on = false;
    game.run(4.0);
    assert_eq!(game.server.ended(), None);
    game.run(2.0);
    assert_eq!(game.server.ended(), Some("the host left"));

    let mut game = hosted(true);
    join(&mut game, Side::Gold, 0);
    game.run(HOST_WAIT as f32 - 1.0);
    assert_eq!(game.server.ended(), None);
    game.run(1.5);
    assert_eq!(game.server.ended(), Some("the host never came"));
}

#[test]
fn a_dedicated_server_with_a_key_runs_on_without_its_host_until_another_comes() {
    let mut game = hosted(false);
    let host = join(&mut game, Side::Blue, KEY);
    let other = join(&mut game, Side::Gold, 0);
    let p = &mut game.peers[host];
    p.client.leave(&mut p.link);
    game.peers[host].on = false;
    game.run(HOST_WAIT as f32 + 1.0);
    assert_eq!(game.server.ended(), None);
    assert_eq!(game.peers[other].client.session.host, None);
    // A new player in the old host's slot is not the host for it
    let plain = join(&mut game, Side::Blue, 0);
    assert_eq!(slot(&game, plain), slot(&game, host));
    assert_eq!(game.server.session().host, None);
    let next = join(&mut game, Side::Blue, KEY);
    assert_eq!(game.server.session().host, Some(slot(&game, next) as u8));
}

#[test]
fn no_one_moves_or_fires_behind_the_lobby_and_their_look_is_kept() {
    let mut game = hosted(false);
    assert!(game.server.set_time_limit(Some(1.0)));
    let host = join(&mut game, Side::Blue, KEY);
    // Gold turns and walks and fires all the time
    let gold = game.join_on(Side::Gold, |t| {
        let mut i = Input { forward: true, run: true, fire: true, ..Input::default() };
        i.set_look(t as f32, 0.0);
        i
    });
    game.run(1.0);
    let g = slot(&game, gold);
    let (pos, look) = (game.server_pos(g), game.server.sim.players[g].look);
    game.lobby(host, LobbyCommand::Start);
    game.run(2.0);
    assert_eq!(game.server.round().phase, Phase::Countdown);
    assert_eq!((game.server_pos(g), game.server.sim.players[g].look), (pos, look));
    assert_eq!(game.server.sim.players[g].ammo, team_setup().ammo, "fired behind the lobby");
    assert_eq!(game.me(gold), pos);
    // In the round they move, and after it, back in the lobby, they are held where they were
    assert!(run_until(&mut game, Phase::Live, 10.0));
    game.run(1.0);
    assert_ne!(game.server.sim.players[g].look, look);
    assert!(run_until(&mut game, Phase::Lobby, 20.0));
    let (pos, look) = (game.server_pos(g), game.server.sim.players[g].look);
    assert_ne!(look, (0, 0));
    game.run(1.0);
    assert_eq!((game.server_pos(g), game.server.sim.players[g].look), (pos, look));
}

#[test]
fn a_match_with_lobby_commands_plays_back_the_same_from_the_server_and_a_client() {
    let mut game = hosted(false);
    game.server.record();
    let host = game.join_recorded(|_| Input::default());
    game.peers[host].client.set_key(KEY);
    game.peers[host].client.ask_side(Some(Side::Blue));
    game.run(0.5);
    let gold = game.join_on(Side::Gold, |t| Input { fire: t % 30 < 10, ..Input::default() });
    let blue = game.join_on(Side::Blue, |_| Input::default());
    game.run(0.5);
    game.lobby(blue, LobbyCommand::Side(Side::Gold));
    game.lobby(host, LobbyCommand::BasePair(1));
    game.lobby(host, LobbyCommand::FriendlyFire(false));
    // Refused: Gold is not the host
    game.lobby(gold, LobbyCommand::TimeLimit(Some(5)));
    game.run(0.5);
    game.lobby(host, LobbyCommand::TimeLimit(Some(3)));
    game.lobby(host, LobbyCommand::Start);
    assert!(run_until(&mut game, Phase::Live, 10.0));
    assert!(run_until(&mut game, Phase::Lobby, 30.0));
    // The host sees the lobby before they click
    game.run(0.5);
    game.lobby(host, LobbyCommand::FriendlyFire(true));
    game.lobby(host, LobbyCommand::BasePair(0));
    game.lobby(host, LobbyCommand::Start);
    assert!(run_until(&mut game, Phase::Live, 10.0));
    assert!(run_until(&mut game, Phase::Lobby, 30.0));
    game.run(0.5);
    assert_eq!(game.server.records.len(), 2);

    let rec = game.server.recording().unwrap();
    let back = ServerRecording::from_bytes(&rec.to_bytes()).unwrap();
    assert_eq!(back, rec);
    // The recording starts as the match did, whatever the host changed since
    assert_eq!(rec.setup.bases, team_setup().bases);
    let sim = back.replay().unwrap();
    assert_eq!(sim.checksum(), game.server.sim.checksum());
    assert_eq!(sim.players[slot(&game, blue)].side, Some(Side::Gold));

    let seen = game.peers[host].recorder.as_ref().unwrap().recording(game.peers[host].sim.as_ref().unwrap());
    assert!(seen.steps.iter().any(|s| matches!(s.call, crate::net::record::Call::Lobby(LobbyCommand::Start))));
    let back = crate::net::record::ClientRecording::from_bytes(&seen.to_bytes()).unwrap();
    assert_eq!(back, seen);
    back.replay(|_, _, _, _| {}).unwrap();
}

#[test]
fn an_accept_too_big_for_a_datagram_comes_in_parts() {
    // Many pairs of bases, as a level with five has, and more
    let mut setup = team_setup();
    let pair = setup.base_pairs[0].clone();
    setup.base_pairs = (0..12).map(|i| crate::net::proto::BasePairSetup { name: format!("ZONE_LONG_LEVEL_NAME_GROUP_{i}_DESC"), ..pair.clone() }).collect();
    let bytes = crate::net::proto::ToClient::Accept { token: 1, slot: 0, tick: 0, setup: setup.clone() }.encode().0;
    assert!(bytes.len() > crate::net::link::MAX_PACKET);
    let mut game = Game::with_setup(Conditions::lan(), setup.clone());
    game.server.play_rounds();
    let p = join(&mut game, Side::Blue, 0);
    assert_eq!(game.peers[p].client.status, Status::Connected);
    assert_eq!(game.peers[p].client.setup().unwrap().base_pairs, setup.base_pairs);
}

#[test]
fn every_levels_accept_with_all_its_pairs_of_bases_comes_through() {
    let Ok(dirs) = std::fs::read_dir("data/map") else { return };
    let mut maps: Vec<_> = dirs.flatten().flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().flatten()).map(|f| f.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("map"))).collect();
    maps.sort();
    let mut largest = (0, String::new());
    for map in maps {
        let pairs = crate::levels::find_bases(&map);
        if pairs.is_empty() {
            continue;
        }
        let base_pairs: Vec<_> = pairs.iter().map(|p| crate::net::proto::BasePairSetup { name: p.name.clone(), bases: [&p.blue, &p.gold].map(|b| b.starts().to_vec()) }).collect();
        let setup = crate::net::proto::Setup { map: map.display().to_string(), bases: base_pairs[0].bases.clone(), base_pairs, ..team_setup() };
        let msg = crate::net::proto::ToClient::Accept { token: 1, slot: 7, tick: 123_456, setup };
        let bytes = msg.encode().0;
        assert_eq!(crate::net::proto::ToClient::decode(&bytes).unwrap(), msg);
        assert!(bytes.len().div_ceil(crate::net::link::MAX_PACKET) <= crate::net::proto::MAX_PARTS);
        if bytes.len() > largest.0 {
            largest = (bytes.len(), map.display().to_string());
        }
    }
    println!("the largest Accept is {} bytes, for {} ({} a datagram)", largest.0, largest.1, crate::net::link::MAX_PACKET);
}
