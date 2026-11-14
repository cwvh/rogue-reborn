use super::*;
use crate::net::bot::{Bot, Skill};
use crate::net::lab::demo_setup;
use crate::net::round::RoundState;
use crate::net::setup::build_sim;
use crate::sim::{Input, TICK};

#[test]
#[ignore = "manual navigation size profile"]
fn profile_rm08_navigation() {
    let path = "data/map/rm08/rm08.map";
    let sim = build_sim(&crate::net::proto::Setup { map: path.to_string(), ..demo_setup() }).unwrap();
    let nav = Navigation::for_map(&sim, path, None);
    println!("rm08 build_ms={:.1} patches={} bytes={}", nav.built_ms, nav.patches.len(), nav.estimated_bytes());
    nav.grid.profile();
}

#[test]
fn rm06_seed_99_does_not_repeat_the_intact_window_route() {
    if !std::path::Path::new("data/map/rm06/rm06.map").exists() { return; }
    let reports = crate::net::lab::navigation::compare(std::path::Path::new("data/map/rm06/rm06.map"), 75.0, 99, false).unwrap();
    let report = reports.iter().find(|r| r.behavior == crate::net::bot::Behavior::Navigate).unwrap();
    assert!(report.blocked < 8.0, "blocked={:.2}, patches={:.1}, rooms={:.1}", report.blocked, report.patches, report.rooms);
    assert!(report.patches > 43.5, "patches={:.1}, rooms={:.1}", report.patches, report.rooms);
}

#[test]
fn relaxing_failed_doors_does_not_relax_intact_glass() {
    if !std::path::Path::new("data/map/rm06/rm06.map").exists() { return; }
    let path = "data/map/rm06/rm06.map";
    let sim = build_sim(&crate::net::proto::Setup { map: path.to_string(), ..demo_setup() }).unwrap();
    let map = Navigation::for_map(&sim, path, None);
    let mut nav = Navigator::new(&map);
    nav.relaxed_doors = true;
    assert!(nav.route(&map, &sim, [33220.0, 32767.0, 31460.0], [33220.0, 32767.0, 31620.0], 0.0));
    for i in 1..nav.path.len() { assert!(sim.world.pane_path_clear(nav.path[i - 1].0, nav.path[i].0)); }
}

#[test]
fn a_player_can_cross_the_low_window_only_after_it_breaks() {
    if !std::path::Path::new("data/map/rm06/rm06.map").exists() { return; }
    let path = "data/map/rm06/rm06.map";
    let mut sim = build_sim(&crate::net::proto::Setup { map: path.to_string(), ..demo_setup() }).unwrap();
    sim.add_player([33227.0, 32767.0, 31457.0]);
    let input = Input { forward: true, ..Input::default() };
    for _ in 0..120 { sim.tick(&[input], TICK); }
    assert!(sim.players[0].player.pos[2] <= 31508.0);
    let pane = sim.panes.iter().position(|p| p.name == "15_glass02").unwrap();
    sim.broken_at[pane] = Some(sim.time);
    sim.world.switch_off("15_glass02");
    for _ in 0..120 {
        sim.tick(&[input], TICK);
        if sim.players[0].player.pos[2] >= 31580.0 { break; }
    }
    assert!(sim.players[0].player.pos[2] >= 31580.0, "{:?}", sim.players[0].player);
}

#[test]
fn a_bot_leaves_the_rm06_spawn_without_waiting_for_door_avoidance_to_expire() {
    let path = "data/map/rm06/rm06.map";
    if !std::path::Path::new(path).exists() { return; }
    let mut failures = Vec::new();
    for variant in ["public", "own start", "prepared", "raw", "raw own"] {
        let mut sim = build_sim(&crate::net::proto::Setup { map: path.to_string(), ..demo_setup() }).unwrap();
        let starts = crate::levels::find_mission_spawns(std::path::Path::new(path));
        let p = starts[10 % starts.len()];
        let from = if variant.starts_with("raw") { p } else { [p[0], sim.world.ground_at(p[0], p[2], p[1] + 45.0).unwrap(), p[2]] };
        sim.add_player(from);
        let mut grid = Navigation::for_map(&sim, path, (variant == "own start" || variant == "raw own").then_some(from));
        if variant == "prepared" { grid.prepare_start(&sim, 0, RoundState::default(), &mut None); }
        let mut bot = Bot::new(Skill::normal(), 10);
        let mut left = None;
        let (mut recovery, mut longest) = (0.0f32, 0.0f32);
        let mut trace = String::new();
        for tick in 0..3600 {
            let input = bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
            sim.tick(&[input], TICK);
            let feet = sim.players[0].player.pos;
            if tick < 1200 && bot.navigation.as_ref().unwrap().fallback { recovery += TICK; longest = longest.max(recovery); }
            else { recovery = 0.0; }
            if flat(feet, from) >= 400.0 { left.get_or_insert(tick as f32 * TICK); }
            if tick % 60 == 0 { trace.push_str(&format!("\n{:.0}: at={feet:?} {}", tick as f32 * TICK, bot.trace())); }
        }
        if !left.is_some_and(|t| t < 10.0) || longest >= 5.0 {
            failures.push(format!("{variant}: from={from:?}, left={left:?}, longest recovery={longest:.2}{trace}"));
        }
        println!("G1 {variant}: from={from:?}, left={left:?}, longest recovery={longest:.2}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_bot_recovers_its_rm11_stair_attachment_without_a_long_stall() {
    let path = "data/map/rm11/rm11.map";
    if !std::path::Path::new(path).exists() { return; }
    let mut sim = build_sim(&crate::net::proto::Setup { map: path.to_string(), ..demo_setup() }).unwrap();
    let starts = crate::levels::find_mission_spawns(std::path::Path::new(path));
    let p = starts[8 % starts.len()];
    sim.add_player([p[0], sim.world.ground_at(p[0], p[2], p[1] + 45.0).unwrap(), p[2]]);
    let grid = Navigation::for_map(&sim, path, None);
    let mut bot = Bot::new(Skill::normal(), 8);
    let mut recent = VecDeque::new();
    let (mut stretch, mut longest) = (0.0f32, 0.0f32);
    let mut failure = String::new();
    let mut diagnosis = String::new();
    for tick in 0..3600 {
        let input = bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
        let body = sim.players[0].player;
        let nav = bot.navigation.as_ref().unwrap();
        recent.push_back(format!("{:.3}: body={body:?} {}", tick as f32 * TICK, bot.trace()));
        if recent.len() > 60 { recent.pop_front(); }
        if nav.reason == Reason::StartDetached {
            if stretch == 0.0 {
                let rooms = sim.standing_rooms(body.pos, None).0;
                let observed = Navigator::observed_leaves(&sim, body.pos);
                let all = nav.leaves(&sim, body.pos, tick as f32 * TICK);
                let attempt = |leaves: &[usize]| grid.grid.routes_past_doors(&sim.world, body.pos, &rooms, &[], leaves).map(|_| "ok");
                let floors: Vec<_> = rooms.iter().map(|&r| (r, sim.world.floors_in(r, body.pos[0], body.pos[2]), sim.world.wall_clearance_in(&body, r))).collect();
                let mut near: Vec<_> = grid.grid.points().filter(|(_, at, _)| flat(*at, body.pos) < 160.0 && (at[1] - body.pos[1]).abs() < 120.0).collect();
                near.sort_by(|a, b| flat(a.1, body.pos).total_cmp(&flat(b.1, body.pos)));
                let yaw = crate::sim::dequantize(sim.players[0].look.0);
                let probes: Vec<_> = (0..8).map(|k| {
                    let heading = (yaw + k as f32 * 45.0).to_radians();
                    let mut probe = body;
                    for _ in 0..8 { sim.world.step(&mut probe, [-heading.sin() * 10.0, heading.cos() * 10.0], TICK); }
                    let attaches = grid.grid.attached(&sim.world, probe.pos, &sim.standing_rooms(probe.pos, None).0);
                    (k, probe.pos, probe.on_ground, probe.pos[1] - body.pos[1], attaches)
                }).collect();
                diagnosis = format!("\nfirst failure at {:.3}, rooms={rooms:?}, floors/clearance={floors:?}\nno leaves={:?}, observed={observed:?} {:?}, all={all:?} {:?}\nnear={:?}\n{}",
                    tick as f32 * TICK, attempt(&[]), attempt(&observed), attempt(&all), &near[..near.len().min(20)], format!("probes={probes:?}\n{}", recent.iter().cloned().collect::<Vec<_>>().join("\n")));
            }
            stretch += TICK;
            if stretch > longest { longest = stretch; failure = diagnosis.clone(); }
        } else { stretch = 0.0; }
        sim.tick(&[input], TICK);
    }
    assert!(longest < 3.0, "longest={longest:.2}{failure}");
}

#[test]
fn a_recovery_probe_can_land_off_the_rm11_ledge_before_reattaching() {
    let path = "data/map/rm11/rm11.map";
    if !std::path::Path::new(path).exists() { return; }
    let mut sim = build_sim(&crate::net::proto::Setup { map: path.to_string(), ..demo_setup() }).unwrap();
    let from = [29218.904, 32713.0, 36547.0];
    sim.add_player(from);
    let probe = [from[0], from[1] + 10.0, from[2]];
    let (rooms, hint) = sim.standing_rooms(from, None);
    let around = sim.visibility.player_rooms(&sim.level, probe, hint);
    sim.players[0].collision_rooms = Some(sim.visibility.solid_rooms(&sim.level, &rooms, &around, &[probe, sim.eye(0)]));
    sim.tick(&[Input::default()], TICK);
    assert!(flat(sim.players[0].player.pos, from) < 1.0);
    assert_eq!(sim.players[0].player.pos[1], from[1]);
    let map = Navigation::for_map(&sim, path, None);
    assert!(!map.grid.attached(&sim.world, from, &rooms));
    let mut bot = Bot::new(Skill::normal(), 8);
    let mut landed = false;
    for _ in 0..180 {
        let input = bot.think(&sim, Some(&map), 0, &Frame::default(), RoundState::default());
        sim.tick(&[input], TICK);
        let p = sim.players[0].player;
        assert!(from[1] - p.pos[1] <= 90.0);
        if p.on_ground && from[1] - p.pos[1] >= 40.0 && map.grid.attached(&sim.world, p.pos, &sim.standing_rooms(p.pos, None).0) {
            landed = true;
            break;
        }
    }
    assert!(landed, "at={:?}, {}", sim.players[0].player.pos, bot.trace());
}

#[test]
fn an_avoided_spawn_exit_can_be_retried_when_no_other_destination_is_reachable() {
    let path = "data/map/rm06/rm06.map";
    if !std::path::Path::new(path).exists() { return; }
    let mut sim = build_sim(&crate::net::proto::Setup { map: path.to_string(), ..demo_setup() }).unwrap();
    let starts = crate::levels::find_mission_spawns(std::path::Path::new(path));
    let p = starts[10 % starts.len()];
    let from = [p[0], sim.world.ground_at(p[0], p[2], p[1] + 45.0).unwrap(), p[2]];
    sim.add_player(from);
    sim.tick(&[Input::default()], TICK);
    let map = Navigation::for_map(&sim, path, None);
    let from = sim.players[0].player.pos;
    let rooms = sim.standing_rooms(from, None).0;
    let open = map.grid.routes(&sim.world, from, &rooms, &[]).unwrap();
    let leaves: Vec<_> = (0..sim.doors.len()).collect();
    let shut = map.grid.routes_past_doors(&sim.world, from, &rooms, &[], &leaves).unwrap();
    let goal = map.patches.iter().position(|p| open.cost[p.node as usize].is_finite() && !shut.cost[p.node as usize].is_finite()).unwrap();
    let mut bot = Bot::new(Skill::normal(), 10);
    bot.think(&sim, Some(&map), 0, &Frame::default(), RoundState::default());
    let nav = bot.navigation.as_mut().unwrap();
    nav.path.clear();
    nav.goal = None;
    nav.cooldown.fill(100.0);
    nav.cooldown[goal] = 0.0;
    nav.avoided_doors = leaves.iter().map(|&door| FailedDoor { door, open: 0.0, until: 15.0 }).collect();
    nav.search(&map, &sim, from, TICK, 10);
    assert!(!nav.fallback, "{}", nav.reason);
    assert!(nav.relaxed_doors);
    assert!(!nav.path.is_empty(), "{}", nav.reason);
    assert_eq!(nav.goal, Some(goal));
    let mut pressed = None;
    for tick in 0..600 {
        let input = bot.think(&sim, Some(&map), 0, &Frame::default(), RoundState::default());
        if input.use_key { pressed.get_or_insert(tick as f32 * TICK); }
        sim.tick(&[input], TICK);
    }
    assert!(pressed.is_some(), "the fallback route never operated its door: {}", bot.trace());
}

#[test]
fn relaxing_failed_doors_keeps_observed_open_leaves_solid() {
    let path = "data/map/rm16/rm16.map";
    if !std::path::Path::new(path).exists() { return; }
    let mut sim = build_sim(&crate::net::proto::Setup { map: path.to_string(), ..demo_setup() }).unwrap();
    let from = [30098.0, sim.world.ground_at(30098.0, 34276.0, 32979.0).unwrap(), 34276.0];
    sim.add_player(from);
    sim.set_doors_open(1.0);
    let map = Navigation::for_map(&sim, path, None);
    let mut nav = Navigator::new(&map);
    let from = map.grid.points().map(|p| p.1).find(|&at| !Navigator::observed_leaves(&sim, at).is_empty()).unwrap();
    let observed = Navigator::observed_leaves(&sim, from);
    assert!(!observed.is_empty());
    nav.avoided_doors = observed.iter().map(|&door| FailedDoor { door, open: 1.0, until: 15.0 }).collect();
    nav.relaxed_doors = true;
    let leaves = nav.leaves(&sim, from, 1.0);
    assert_eq!(leaves, observed);
    let mut checked = false;
    for to in map.patches.iter().map(|p| p.at) {
        if nav.route(&map, &sim, from, to, 1.0) {
            let mut previous = from;
            for &(p, _) in &nav.path {
                assert!(observed.iter().all(|&k| sim.world.door_path_clear(previous, p, k)));
                previous = p;
            }
            checked = true;
            break;
        }
    }
    assert!(checked);
}

fn test_door(origin: V3) -> crate::door::Door {
    crate::door::Door {
        name: String::new(), model: String::new(), model_index: 0, origin,
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        motion: crate::door::Motion::Slide { dir: [1.0, 0.0, 0.0], distance: 100.0 },
        seconds: 0.5, open_sound: "none".to_string(), close_sound: "none".to_string(),
        portal: None, rooms: Vec::new(), center_local: [0.0; 3],
    }
}

#[test]
fn door_wait_ends_after_a_passage_without_a_long_walk_away() {
    let mut sim = build_sim(&demo_setup()).unwrap();
    sim.add_player([0.0, 0.0, 60.0]);
    sim.doors.push(test_door([0.0, 100.0, 0.0]));
    sim.door_states.push(crate::door::DoorState { open: 1.0, opening: true });
    let map = Navigation::for_map(&sim, "demo", None);
    let mut nav = Navigator::new(&map);
    nav.wait_for_door(0, [0.0, 0.0, -60.0], 0.0);
    nav.path.push_back(([0.0, 0.0, 300.0], None));
    assert!(nav.steer(&map, &sim, 0, &Frame::default(), 6.1, 42, None).is_some());
    assert!(nav.door_wait.is_none());
    assert!(nav.recover_until.is_none());
}

#[test]
fn door_wait_expiry_allows_search_after_a_fixed_recovery_even_without_movement() {
    let mut sim = build_sim(&demo_setup()).unwrap();
    sim.add_player([0.0; 3]);
    sim.doors.push(test_door([0.0, 100.0, 150.0]));
    sim.door_states.push(crate::door::DoorState::default());
    let map = Navigation::for_map(&sim, "demo", None);
    let mut nav = Navigator::new(&map);
    nav.wait_for_door(0, [0.0; 3], 0.0);
    assert!(nav.steer(&map, &sim, 0, &Frame::default(), 6.1, 42, None).is_none());
    assert!(nav.door_wait.is_none());
    assert!(nav.recover_until.is_some());
    assert!(nav.steer(&map, &sim, 0, &Frame::default(), 6.2, 42, None).is_none());
    nav.steer(&map, &sim, 0, &Frame::default(), 6.1 + ABANDON_AGAIN + TICK, 42, None);
    assert!(nav.recover_until.is_none());
    assert!(!nav.path.is_empty());
    assert_ne!(nav.reason, Reason::DoorWaitExpired);
}

#[test]
fn door_wait_recovery_ends_in_mission_rooms() {
    let mut failures = Vec::new();
    for (level, seed) in [("rm16", 5u64), ("rm06", 1), ("rm06", 2)] {
        let path = format!("data/map/{level}/{level}.map");
        if !std::path::Path::new(&path).exists() { continue; }
        let mut sim = build_sim(&crate::net::proto::Setup { map: path.clone(), ..demo_setup() }).unwrap();
        let starts = crate::levels::find_mission_spawns(std::path::Path::new(&path));
        let p = starts[seed as usize % starts.len()];
        let from = [p[0], sim.world.ground_at(p[0], p[2], p[1] + 45.0).unwrap(), p[2]];
        sim.add_player(from);
        let grid = Navigation::for_map(&sim, &path, None);
        let mut bot = Bot::new(Skill::normal(), seed);
        let (mut stretch, mut longest) = (0.0f32, 0.0f32);
        let mut feet_at_expiry = None;
        let mut resumed = false;
        for _ in 0..3600 {
            let input = bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
            let nav = bot.navigation.as_ref().unwrap();
            if nav.reason == Reason::DoorWaitExpired {
                feet_at_expiry.get_or_insert(sim.players[0].player.pos);
                stretch += TICK;
                longest = longest.max(stretch);
            } else { stretch = 0.0; }
            if feet_at_expiry.is_some_and(|p| flat(p, sim.players[0].player.pos) >= RECOVERED)
                && nav.goal.is_some() && !nav.fallback && !nav.path.is_empty() { resumed = true; }
            sim.tick(&[input], TICK);
        }
        if longest > crate::net::bot::RECOVER_FOR + 0.25 || (feet_at_expiry.is_some() && !resumed) {
            failures.push(format!("{level} seed={seed}, longest expiry recovery={longest:.2}s, resumed={resumed}, {}", bot.trace()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn door_wait_is_cancelled_by_combat() {
    let sim = build_sim(&demo_setup()).unwrap();
    let map = Navigation::for_map(&sim, "demo", None);
    let mut nav = Navigator::new(&map);
    nav.wait_for_door(0, [0.0; 3], 0.0);
    assert!(nav.press_door(0, 0.0));
    nav.recover_until = Some(6.0);
    nav.interrupt();
    assert!(nav.door_wait.is_none());
    assert!(nav.recover_until.is_none());
    assert_eq!(nav.door_attempts[0].count, 1);
}

#[test]
fn clients_stay_on_the_rm11_floor_during_exploration() {
    let map = "data/map/rm11/rm11.map";
    if !std::path::Path::new(map).exists() { return; }
    let mut setup = crate::net::proto::Setup { map: map.to_string(), grenades: 0, friendly_fire: false, ..demo_setup() };
    let sim = build_sim(&setup).unwrap();
    let pair = crate::levels::find_bases(std::path::Path::new(map)).remove(0);
    setup.bases = [pair.blue.starts(), pair.gold.starts()].map(|s| s.into_iter().map(|p|
        [p[0], sim.world.ground_at(p[0], p[2], p[1] + 45.0).unwrap(), p[2]]).collect());
    let spawns = setup.bases.iter().map(|s| s[0]).collect();
    let mut game = crate::net::lab::Game::with_spawns(crate::net::link::Conditions::lan(), setup, spawns);
    for seed in [99, 100] { game.join_bot(Skill::normal(), seed); }
    let mut trace = VecDeque::new();
    for tick in 0..3600 {
        game.step();
        let peer = &game.peers[1];
        if peer.client.snapshots == 0 { continue; }
        let sim = peer.sim.as_ref().unwrap();
        let p = &sim.players[peer.client.slot()];
        let doors: Vec<_> = sim.doors.iter().enumerate().filter(|(_, d)| flat(p.player.pos, d.center(0.0)) < 200.0)
            .map(|(k, _)| (k, sim.door_states[k].open, sim.door_states[k].opening, sim.world.door_sweep_clear(&p.player, k))).collect();
        trace.push_back(format!("{tick} at={:?} on_ground={} velocity={} rooms={:?} doors={doors:?} {}", p.player.pos, p.player.on_ground,
            p.player.vel_y, sim.standing_rooms(p.player.pos, None).0, peer.bot.as_ref().unwrap().trace()));
        if trace.len() > 120 { trace.pop_front(); }
        assert!(p.player.pos[1] > 32500.0, "{}", trace.iter().cloned().collect::<Vec<_>>().join("\n"));
    }
}

#[test]
fn a_nearer_open_door_wins_a_press_aimed_at_a_farther_closed_door() {
    let mut sim = build_sim(&demo_setup()).unwrap();
    sim.add_player([0.0; 3]);
    sim.doors = vec![test_door([20.0, sim.eye(0)[1], 50.0]), test_door([0.0, sim.eye(0)[1], 135.0])];
    sim.door_states = vec![crate::door::DoorState { open: 1.0, opening: true }, crate::door::DoorState::default()];
    let mut input = Input { use_key: true, ..Input::default() };
    input.set_look(0.0, 0.0);
    assert_eq!(crate::sim::door_target(sim.eye(0), [0.0, 0.0, 1.0], &sim.doors, &sim.door_states), Some(0));
    sim.tick(&[input], TICK);
    assert!(!sim.door_states[0].opening);
    assert!(!sim.door_states[1].opening);
}

#[test]
fn door_wait_survives_switches_and_replans_without_body_progress() {
    let mut sim = build_sim(&demo_setup()).unwrap();
    let from = [0.0; 3];
    sim.add_player(from);
    sim.doors = vec![test_door([0.0, 100.0, 150.0]), test_door([150.0, 100.0, 150.0])];
    sim.door_states = vec![crate::door::DoorState::default(); 2];
    let map = Navigation::for_map(&sim, "demo", None);
    let mut nav = Navigator::new(&map);
    nav.wait_for_door(0, from, 0.0);
    assert!(nav.press_door(0, 0.0));
    nav.wait_for_door(1, from, 1.0);
    assert!(nav.press_door(1, 1.0));
    assert_eq!(nav.door_wait.as_ref().unwrap().since, 1.0);
    nav.wait_for_door(0, from, 2.0);
    assert!(nav.press_door(0, 2.0));
    assert!(!nav.press_door(0, 3.0));
    assert!(nav.route(&map, &sim, from, [300.0, 0.0, 300.0], 5.0));
    nav.wait_for_door(0, from, 5.0);
    assert_eq!(nav.door_wait.as_ref().unwrap().since, 2.0);
    assert!(nav.steer(&map, &sim, 0, &Frame::default(), 8.1, 42, None).is_none());
    assert!(nav.fallback);
    assert_eq!(nav.reason, Reason::DoorWaitExpired);
    assert!(nav.path.is_empty());
}

#[test]
fn a_grid_extension_keeps_visits_but_a_new_map_resets_them() {
    let mut sim = build_sim(&demo_setup()).unwrap();
    sim.add_player([0.0; 3]);
    let mut map = Navigation::for_map(&sim, "demo", None);
    let mut nav = Navigator::new(&map);
    nav.visited.fill(Some(3.0));
    assert!(nav.route(&map, &sim, [0.0; 3], [300.0, 0.0, 300.0], 0.0));
    map.extend(&sim, [200.0, 0.0, 200.0]);
    nav.sync(&map);
    assert!(nav.path.is_empty());
    assert!(nav.visited.iter().all(|v| *v == Some(3.0)));
    let other = Navigation::for_map(&sim, "demo", None);
    assert_eq!(other.patches.len(), map.patches.len());
    nav.sync(&other);
    assert!(nav.visited.iter().all(Option::is_none));
    assert!(nav.goal.is_none());
}

#[test]
fn a_bot_passes_the_rm06_doors_without_repeated_toggles() {
    let map = "data/map/rm06/rm06.map";
    if !std::path::Path::new(map).exists() { return; }
    let mut failures = Vec::new();
    for seed in [4, 32] {
        let mut sim = build_sim(&crate::net::proto::Setup { map: map.to_string(), ..demo_setup() }).unwrap();
        let from = [35473.4, 32767.0, 33154.2];
        sim.add_player(from);
        let probe = [from[0], from[1] + 10.0, from[2]];
        let (rooms, hint) = sim.standing_rooms(from, None);
        let around = sim.visibility.player_rooms(&sim.level, probe, hint);
        sim.players[0].collision_rooms = Some(sim.visibility.solid_rooms(&sim.level, &rooms, &around, &[probe, sim.eye(0)]));
        sim.tick(&[Input::default()], TICK);
        assert!(flat(sim.players[0].player.pos, from) < 1.0, "unstable initial feet: {:?}", sim.players[0].player.pos);
        let mut bot = Bot::new(Skill::normal(), seed);
        let grid = Navigation::for_map(&sim, map, None);
        let mut toggles = vec![0; sim.doors.len()];
        let mut farthest = 0.0f32;
        for _ in 0..1200 {
            let input = bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
            let before: Vec<_> = sim.door_states.iter().map(|d| d.opening).collect();
            sim.tick(&[input], TICK);
            for (k, d) in sim.door_states.iter().enumerate() { toggles[k] += usize::from(d.opening != before[k]); }
            farthest = farthest.max(flat(sim.players[0].player.pos, from));
        }
        if farthest < 100.0 || toggles.iter().any(|&n| n > 2) {
            failures.push(format!("seed={seed}, moved={farthest:.1}, toggles={toggles:?}, {}", bot.trace()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_bad_start_does_not_cool_good_destinations() {
    let mut sim = build_sim(&demo_setup()).unwrap();
    sim.add_player([499.0, 0.0, 0.0]);
    let mut grid = Navigation::for_map(&sim, "demo", None);
    let mut checked = None;
    assert!(grid.prepare_start(&sim, 0, RoundState::default(), &mut checked));
    for _ in 0..3 { assert!(!grid.prepare_start(&sim, 0, RoundState::default(), &mut checked)); }
    assert_eq!(grid.extensions, 1);
    assert!(!grid.grid.attached(&sim.world, sim.players[0].player.pos, &[1]));
    let mut nav = Navigator::new(&grid);
    for tick in 0..3 {
        nav.search(&grid, &sim, sim.players[0].player.pos, tick as f32, 42);
        assert!(nav.fallback);
        assert_eq!(nav.reason, Reason::StartDetached);
        assert!(nav.cooldown.iter().all(|&t| t == 0.0));
    }
}

#[test]
fn actual_progress_resets_retries_but_a_replan_start_point_does_not() {
    let mut sim = build_sim(&demo_setup()).unwrap();
    sim.add_player([20.0, 0.0, 20.0]);
    sim.tick(&[Input::default()], TICK);
    let feet = sim.players[0].player.pos;
    let grid = Navigation::for_map(&sim, "demo", None);
    let mut nav = Navigator::new(&grid);
    assert!(nav.route(&grid, &sim, feet, [400.0, 0.0, 300.0], 0.0));
    nav.retries = 2;
    nav.replan_at = Some(feet);
    nav.steer(&grid, &sim, 0, &Frame::default(), 0.1, 42, None);
    assert_eq!(nav.retries, 2);
    sim.players[0].player.pos[0] += RECOVERED;
    nav.steer(&grid, &sim, 0, &Frame::default(), 0.2, 42, None);
    assert_eq!(nav.retries, 0);
}

#[test]
fn public_navigation_ignores_server_spawn_state_and_prepares_while_held() {
    let Some(mut server) = citystreet() else { return };
    let mut client = build_sim(&crate::net::proto::Setup { map: "data/map/m00/citystreet_large.map".to_string(), bases: server.bases.clone(), ..demo_setup() }).unwrap();
    server.spawns = vec![[12345.0, 0.0, 12345.0]];
    let path = "data/map/m00/citystreet_large.map";
    let a = Navigation::for_map(&server, path, None);
    let b = Navigation::for_map(&client, path, None);
    assert_eq!(a.grid.points().collect::<Vec<_>>(), b.grid.points().collect::<Vec<_>>());
    assert!(a.grid.transitions().eq(b.grid.transitions()));
    client.add_player(client.bases[0][0]);
    client.frozen = true;
    let mut bot = Bot::new(Skill::normal(), 42);
    let grid = Navigation::for_map(&client, path, None);
    bot.think(&client, Some(&grid), 0, &Frame::default(), RoundState { phase: crate::net::round::Phase::Hold, number: 1, ..RoundState::default() });
    let nav = bot.navigation.as_ref().unwrap();
    assert!(grid.grid.attached(&client.world, client.players[0].player.pos, &client.standing_rooms(client.players[0].player.pos, None).0));
    assert!(nav.visited.iter().all(Option::is_none));
    assert!(nav.goal.is_none());
}

#[test]
fn respawning_on_an_uncovered_floor_extends_the_grid_once() {
    let mut map = crate::demo::map();
    let mut object = map.geometries.objects[0].clone();
    object.id = 2;
    object.object_id = 2;
    object.name = "2".to_string();
    object.object_name = "2".to_string();
    for v in &mut object.vertices { v.x += 3000.0; }
    map.geometries.objects.push(object);
    let mut room = map.rooms.rooms[0].clone();
    room.section_id = 2;
    room.section_name = "2".to_string();
    room.sherman_levels[0].tm_with_aabb[0].tm.position.x += 3000.0;
    map.rooms.rooms.push(room);
    let mut sim = build_sim(&demo_setup()).unwrap();
    sim.world = crate::collision::CollisionWorld::from_map(&map, crate::collision::Params::default());
    sim.visibility = crate::visibility::Visibility::new(&map);
    sim.level = std::rc::Rc::new(map);
    sim.add_player([0.0; 3]);
    let mut grid = Navigation::for_map(&sim, "demo", None);
    let mut bot = Bot::new(Skill::normal(), 42);
    let mut neighbor = Navigator::new(&grid);
    neighbor.visited.fill(Some(3.0));
    let old_count = grid.patches.len();
    let old_keys: Vec<_> = grid.patches.iter().map(|p| key(p.room, p.at)).collect();
    let mut checked = None;
    assert!(!grid.prepare_start(&sim, 0, RoundState::default(), &mut checked));
    bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
    assert!(!grid.grid.attached(&sim.world, [3000.0, 0.0, 0.0], &[2]));
    sim.players[0].hp = 0.0;
    grid.prepare_start(&sim, 0, RoundState::default(), &mut checked);
    sim.players[0].hp = 100.0;
    sim.players[0].player.pos = [3000.0, 0.0, 0.0];
    assert!(grid.prepare_start(&sim, 0, RoundState::default(), &mut checked));
    assert!(grid.grid.attached(&sim.world, sim.players[0].player.pos, &[2]));
    assert_eq!(grid.extensions, 1);
    for _ in 0..3 { assert!(!grid.prepare_start(&sim, 0, RoundState::default(), &mut checked)); }
    assert_eq!(grid.extensions, 1);
    neighbor.sync(&grid);
    assert!(neighbor.visited[..old_count].iter().all(|v| *v == Some(3.0)));
    assert!(neighbor.visited[old_count..].iter().all(Option::is_none));
    assert_eq!(old_keys, grid.patches[..old_count].iter().map(|p| key(p.room, p.at)).collect::<Vec<_>>());
    assert!(neighbor.path.is_empty());

}

#[test]
fn a_failed_leaf_is_avoided_by_every_destination_after_replanning() {
    let map = "data/map/rm16/rm16.map";
    if !std::path::Path::new(map).exists() { return; }
    let mut sim = build_sim(&crate::net::proto::Setup { map: map.to_string(), ..demo_setup() }).unwrap();
    let from = [30098.0, sim.world.ground_at(30098.0, 34276.0, 32979.0).unwrap(), 34276.0];
    let to = [30828.8, 33167.0, 35006.4];
    sim.add_player(from);
    let map = Navigation::for_map(&sim, map, None);
    let mut nav = Navigator::new(&map);
    assert!(nav.route(&map, &sim, from, to, 0.0));
    let k = sim.doors.iter().enumerate().find(|(k, _)| {
        let mut previous = from;
        nav.path.iter().any(|&(p, _)| {
            let blocked = !sim.world.door_path_clear(previous, p, *k);
            previous = p;
            blocked
        })
    }).unwrap().0;
    nav.wait_for_door(k, from, 0.0);
    nav.goal = Some(0);
    assert!(nav.route(&map, &sim, from, to, 1.0));
    assert!(nav.door.is_none());
    nav.abandon(&sim, 1.0);
    assert_eq!(nav.failed_door[0].unwrap().0, k);
    assert_eq!(nav.avoided_doors[0].door, k);
    assert!(nav.leaves(&sim, from, 2.0).contains(&k));
    for search in [false, true] {
        if search { nav.search(&map, &sim, from, 3.0, 42); }
        else { nav.route(&map, &sim, from, to, 3.0); }
        let mut previous = from;
        for &(p, _) in &nav.path {
            assert!(sim.world.door_path_clear(previous, p, k));
            previous = p;
        }
    }
    assert!(!nav.leaves(&sim, from, 16.1).contains(&k));
}

#[test]
fn only_an_observed_change_to_the_failed_door_ends_its_cooldown() {
    let map = "data/map/rm16/rm16.map";
    if !std::path::Path::new(map).exists() { return; }
    let mut sim = build_sim(&crate::net::proto::Setup { map: map.to_string(), ..demo_setup() }).unwrap();
    let at = [30098.0, sim.world.ground_at(30098.0, 34276.0, 32979.0).unwrap(), 34276.0];
    sim.add_player(at);
    let grid = Navigation::for_map(&sim, map, Some(at));
    let mut nav = Navigator::new(&grid);
    let k = sim.doors.iter().enumerate().min_by(|a, b| flat(at, a.1.center(0.0)).total_cmp(&flat(at, b.1.center(0.0)))).unwrap().0;
    let target = sim.doors[k].center(0.0);
    let d = [target[0] - at[0], target[1] - at[1], target[2] - at[2]];
    sim.players[0].look.0 = crate::sim::quantize((-d[0]).atan2(d[2]).to_degrees());
    nav.failed_door[0] = Some((k, 0.0));
    nav.cooldown[0] = 15.0;
    nav.avoided_doors.push(FailedDoor { door: k, open: 0.0, until: 15.0 });
    nav.door_attempts.push(DoorAttempts { door: k, count: 2, until: 15.0 });
    let unrelated = (k + 1) % sim.doors.len();
    sim.door_states[unrelated].open = 1.0;
    nav.observe_failed_doors(&sim, 0, 1.0);
    assert_eq!(nav.cooldown[0], 15.0);
    assert_eq!(nav.avoided_doors.len(), 1);
    assert_eq!(nav.door_attempts[0].count, 2);
    sim.door_states[k].open = 0.3;
    nav.observe_failed_doors(&sim, 0, 2.0);
    assert_eq!(nav.cooldown[0], 2.0);
    assert!(nav.avoided_doors.is_empty());
    assert!(nav.door_attempts.is_empty());
    nav.cooldown[0] = 15.0;
    nav.failed_door[0] = Some((k, 0.0));
    nav.avoided_doors.push(FailedDoor { door: k, open: 0.0, until: 15.0 });
    sim.players[0].look.0 = sim.players[0].look.0.wrapping_add(32768);
    nav.observe_failed_doors(&sim, 0, 3.0);
    assert_eq!(nav.cooldown[0], 15.0);
    assert_eq!(nav.avoided_doors.len(), 1);
    nav.observe_failed_doors(&sim, 0, 15.1);
    assert!(nav.avoided_doors.is_empty());
}

fn walk_direct(sim: &mut Sim, from: V3, to: V3) -> Option<V3> {
    if let Some(p) = sim.players.get_mut(0) { p.active = false; }
    sim.activate_slot(0, from);
    let rooms = sim.world.floor_rooms(from);
    sim.players[0].collision_rooms = Some(rooms.into_iter().map(crate::collision::SolidRoom::whole).collect());
    sim.tick(&[Input::default()], TICK);
    let initial = sim.players[0].player.pos;
    if flat(initial, from) > 1.0 || (initial[1] - from[1]).abs() > 1.0 { return None; }
    for _ in 0..120 {
        let at = sim.players[0].player.pos;
        if flat(at, to) < 8.0 && (at[1] - to[1]).abs() < 8.0 { break; }
        let d = [to[0] - at[0], 0.0, to[2] - at[2]];
        let mut input = Input { forward: true, run: false, ..Input::default() };
        input.set_look((-d[0]).atan2(d[2]).to_degrees(), 0.0);
        sim.tick(&[input], TICK);
    }
    Some(sim.players[0].player.pos)
}

#[test]
fn doorway_rises_are_bounded() {
    for map in ["data/map/m00/citystreet_large.map", "data/map/rm11/rm11.map", "data/map/rm02/rm02.map"] {
        if !std::path::Path::new(map).exists() { return; }
        let sim = build_sim(&crate::net::proto::Setup { map: map.to_string(), ..demo_setup() }).unwrap();
        let nav = Navigation::for_map(&sim, map, None);
        assert!(nav.grid.len() > 1000);
        for (a, b, portal) in nav.grid.transitions().filter(|t| t.2.is_some()) {
            assert!(b[1] - a[1] <= sim.world.params().step_up + 0.1, "{map}: {a:?} -> {b:?} at {portal:?}");
        }
    }
}

#[test]
fn a_real_drop_is_walked_down_and_cannot_be_climbed_directly_back() {
    let map = "data/map/rm04/rm04.map";
    if !std::path::Path::new(map).exists() { return; }
    let mut sim = build_sim(&crate::net::proto::Setup { map: map.to_string(), ..demo_setup() }).unwrap();
    sim.set_doors_open(1.0);
    let nav = Navigation::for_map(&sim, map, None);
    let (a, b) = ([35980.0, 33527.0, 28980.0], [35900.0, 33467.0, 29060.0]);
    assert!(nav.grid.transitions().any(|(from, to, _)| from == a && to == b));
    assert!(!nav.grid.transitions().any(|(from, to, _)| from == b && to == a));
    let down = walk_direct(&mut sim, a, b).expect("a valid start above the drop");
    assert!(flat(down, b) < 12.0 && (down[1] - b[1]).abs() < 8.0, "{down:?}");
    let back = walk_direct(&mut sim, b, a).expect("a valid start below the drop");
    assert!(back[1] < a[1] - 45.0, "climbed back to {back:?}");
}

#[test]
fn a_player_walks_on_each_of_two_stacked_floors() {
    let map = "data/map/rm11/rm11.map";
    if !std::path::Path::new(map).exists() { return; }
    let mut sim = build_sim(&crate::net::proto::Setup { map: map.to_string(), ..demo_setup() }).unwrap();
    sim.set_doors_open(1.0);
    let nav = Navigation::for_map(&sim, map, None);
    let floors = [[28700.0, 32767.0, 40580.0], [28700.0, 33167.0, 40580.0]];
    for from in floors {
        let mut walked = false;
        for (_, to, portal) in nav.grid.transitions().filter(|t| t.0 == from) {
            if portal.is_some() || (to[1] - from[1]).abs() > 1.0 || flat(from, to) < 39.0 { continue; }
            let Some(at) = walk_direct(&mut sim, from, to) else { continue };
            if flat(at, to) < 12.0 && (at[1] - from[1]).abs() < 1.0 {
                assert!(nav.grid.attached(&sim.world, at, &sim.standing_rooms(at, None).0));
                walked = true;
                break;
            }
        }
        assert!(walked, "no executable walk at {from:?}");
    }
    assert_ne!(nav.patch_at(&sim, floors[0]), nav.patch_at(&sim, floors[1]));
}

fn citystreet() -> Option<Sim> {
    let path = std::path::Path::new("data/map/m00/citystreet_large.map");
    if !path.exists() { return None; }
    let mut sim = build_sim(&crate::net::proto::Setup { map: path.to_string_lossy().into_owned(), ..demo_setup() }).unwrap();
    let pair = crate::levels::find_bases(path).remove(0);
    sim.bases = [pair.blue, pair.gold].map(|b| b.starts().into_iter().map(|p|
        [p[0], sim.world.ground_at(p[0], p[2], p[1] + 45.0).unwrap(), p[2]]).collect());
    Some(sim)
}

#[test]
fn a_bot_walks_between_citystreet_bases_through_stairs() {
    for reverse in [false, true] {
        let Some(mut sim) = citystreet() else { return };
        let (from, to) = (sim.bases[reverse as usize][0], sim.bases[!reverse as usize][0]);
        sim.add_player(from);
        sim.tick(&[Input::default()], TICK);
        let grid = Navigation::for_map(&sim, "data/map/m00/citystreet_large.map", None);
    let mut bot = Bot::new(Skill::normal(), 42);
        bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
        assert!(bot.navigation.as_mut().unwrap().route(&grid, &sim, from, to, 0.0));
        let mut reached = false;
        for _ in 0..(90.0 / TICK) as usize {
            let input = bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
            sim.tick(&[input], TICK);
            let at = sim.players[0].player.pos;
            if flat(at, to) < 100.0 && (at[1] - to[1]).abs() < 45.0 { reached = true; break; }
        }
        assert!(reached, "reverse={reverse}, at={:?}, goal={to:?}, {}", sim.players[0].player.pos, bot.trace());
    }
}

#[test]
fn a_teammate_in_sight_causes_only_a_brief_yield() {
    let mut sim = build_sim(&demo_setup()).unwrap();
    sim.add_player([-20.0, 0.0, 0.0]);
    sim.add_player([-20.0, 0.0, -60.0]);
    for p in &mut sim.players { p.side = Some(crate::sim::Side::Blue); }
    sim.tick(&[Input::default(); 2], TICK);
    let feet = sim.players[1].player.pos;
    let grid = Navigation::for_map(&sim, "demo", Some(feet));
    let mut nav = Navigator::new(&grid);
    assert!(nav.route(&grid, &sim, feet, [-20.0, 0.0, 300.0], 0.0));
    let view = Frame { players: vec![crate::net::client::Seen { slot: 0, feet: sim.players[0].player.pos,
        look: (0, 0), dead: None, crouched: false, peek: None, joined: sim.players[0].joined }], ..Frame::default() };
    let first = nav.steer(&grid, &sim, 1, &view, 0.0, 0, None).unwrap();
    assert_eq!(nav.reason, Reason::Yielding);
    assert!(first.toward.is_none());
    assert!(nav.steer(&grid, &sim, 1, &view, 0.8, 0, None).unwrap().toward.is_some());
    let mut hidden = Navigator::new(&grid);
    assert!(hidden.route(&grid, &sim, feet, [-20.0, 0.0, 300.0], 0.0));
    assert!(hidden.steer(&grid, &sim, 1, &Frame::default(), 0.0, 0, None).unwrap().toward.is_some());
}

#[test]
fn a_teammate_behind_intact_glass_does_not_cause_a_yield() {
    if !std::path::Path::new("data/map/rm06/rm06.map").exists() { return; }
    let path = "data/map/rm06/rm06.map";
    let mut sim = build_sim(&crate::net::proto::Setup { map: path.to_string(), ..demo_setup() }).unwrap();
    sim.add_player([33227.0, 32767.0, 31577.0]);
    sim.add_player([33227.0, 32767.0, 31507.0]);
    for player in &mut sim.players { player.side = Some(crate::sim::Side::Blue); }
    sim.tick(&[Input::default(); 2], TICK);
    let feet = sim.players[1].player.pos;
    let map = Navigation::for_map(&sim, path, None);
    let view = Frame { players: vec![crate::net::client::Seen { slot: 0, feet: sim.players[0].player.pos,
        look: (0, 0), dead: None, crouched: false, peek: None, joined: sim.players[0].joined }], ..Frame::default() };
    let pane = sim.panes.iter().position(|p| p.name == "15_glass02").unwrap();
    for broken in [false, true] {
        if broken {
            sim.broken_at[pane] = Some(sim.time);
            sim.world.switch_off("15_glass02");
        }
        let mut nav = Navigator::new(&map);
        // Keep a waypoint beyond the ally so this test isolates the yield check.
        nav.path.push_back(([33227.0, 32767.0, 31667.0], None));
        nav.progress = (0.0, f32::INFINITY);
        let steering = nav.steer(&map, &sim, 1, &view, 0.0, 0, None).unwrap();
        assert_eq!(nav.reason == Reason::Yielding, broken, "broken={broken}, feet={feet:?}");
        assert_eq!(steering.toward.is_none(), broken);
    }
}

#[test]
fn a_bot_opens_a_house_door_with_player_inputs() {
    let map = "data/map/rm16/rm16.map";
    if !std::path::Path::new(map).exists() { return; }
    let mut sim = build_sim(&crate::net::proto::Setup { map: map.to_string(), ..demo_setup() }).unwrap();
    let from = [30098.0, sim.world.ground_at(30098.0, 34276.0, 32979.0).unwrap(), 34276.0];
    let to = [30828.8, 33167.0, 35006.4];
    sim.add_player(from);
    sim.tick(&[Input::default()], TICK);
    let grid = Navigation::for_map(&sim, map, None);
    let mut bot = Bot::new(Skill::normal(), 42);
    bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
    assert!(bot.navigation.as_mut().unwrap().route(&grid, &sim, from, to, 0.0));
    let mut presses = 0;
    let mut reached = false;
    let mut checked_wait = false;
    for _ in 0..3600 {
        let input = bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
        presses += usize::from(input.use_key);
        if input.use_key && !checked_wait {
            let mut waiting = Navigator::new(&grid);
            assert!(waiting.route(&grid, &sim, sim.players[0].player.pos, to, 0.0));
            waiting.steer(&grid, &sim, 0, &Frame::default(), 0.0, 42, None);
            assert_eq!(waiting.reason, Reason::OpeningDoor);
            assert!(waiting.steer(&grid, &sim, 0, &Frame::default(), 6.1, 42, None).is_none());
            assert_eq!(waiting.reason, Reason::DoorWaitExpired);
            checked_wait = true;
        }
        sim.tick(&[input], TICK);
        let at = sim.players[0].player.pos;
        if flat(at, to) < 60.0 && (at[1] - to[1]).abs() < 45.0 { reached = true; break; }
    }
    assert!(reached && presses > 0, "at={:?}, presses={presses}, {}", sim.players[0].player.pos, bot.trace());
}

#[test]
fn clients_share_static_navigation_but_keep_separate_visits() {
    let mut game = crate::net::lab::Game::new(crate::net::link::Conditions::lan());
    let a = game.join_bot(Skill::normal(), 42);
    let b = game.join_bot(Skill::normal(), 43);
    let build = game.navigation.as_ref().unwrap().id;
    game.run(0.2);
    assert_eq!(game.navigation.as_ref().unwrap().id, build);
    assert_eq!(game.navigation.as_ref().unwrap().extensions, 0);
    let before = game.peers[b].bot.as_ref().unwrap().navigation.as_ref().unwrap().visited.clone();
    game.peers[a].bot.as_mut().unwrap().navigation.as_mut().unwrap().visited.fill(Some(3.0));
    assert_eq!(game.peers[b].bot.as_ref().unwrap().navigation.as_ref().unwrap().visited, before);

}

#[test]
fn lab_bots_searching_together_grow_several_counted_slots() {
    if !std::path::Path::new("data/map/rm06/rm06.map").exists() { return; }
    let map = "data/map/rm06/rm06.map";
    let setup = crate::net::proto::Setup { map: map.to_string(), grenades: 0, friendly_fire: false, ..demo_setup() };
    let sim = build_sim(&setup).unwrap();
    let spawns: Vec<_> = crate::levels::find_mission_spawns(std::path::Path::new(map)).into_iter()
        .filter_map(|p| sim.world.ground_at(p[0], p[2], p[1] + 45.0).map(|y| [p[0], y, p[2]])).take(4).collect();
    assert!(spawns.len() >= 2);
    let mut game = crate::net::lab::Game::with_spawns(crate::net::link::Conditions::lan(), setup, spawns);
    for seed in 0..4 { game.join_bot(Skill::normal(), seed); }
    game.run(3.0);
    let nav = game.navigation.as_ref().unwrap();
    let slots = nav.searches.slot_bytes();
    assert_eq!(slots.len(), 4);
    assert!(slots.iter().filter(|&&b| b > 0).count() >= 2, "{slots:?}");
    assert_eq!(nav.search_bytes(), slots.iter().sum::<usize>());
    assert!(nav.estimated_bytes() >= nav.grid.estimated_bytes() + nav.search_bytes());
}

#[test]
fn a_client_opens_doors_and_passes_their_open_leaves() {
    let map = "data/map/rm16/rm16.map";
    if !std::path::Path::new(map).exists() { return; }
    let setup = crate::net::proto::Setup { map: map.to_string(), ..demo_setup() };
    let sim = build_sim(&setup).unwrap();
    let from = [30098.0, sim.world.ground_at(30098.0, 34276.0, 32979.0).unwrap(), 34276.0];
    let to = [30828.8, 33167.0, 35006.4];
    let mut game = crate::net::lab::Game::with_spawns(crate::net::link::Conditions::lan(), setup, vec![from]);
    let peer = game.join_bot(Skill::normal(), 42);
    game.run(0.5);
    let own = game.me(peer);
    let grid = game.navigation.as_ref().unwrap();
    let state = &mut game.peers[peer];
    assert!(state.bot.as_mut().unwrap().navigation.as_mut().unwrap().route(grid, state.sim.as_ref().unwrap(), own, to, 1.0));
    let mut reached = false;
    for _ in 0..3000 {
        game.step();
        let at = game.me(peer);
        if flat(at, to) < 60.0 && (at[1] - to[1]).abs() < 45.0 { reached = true; break; }
    }
    assert!(reached, "at={:?}, {}", game.me(peer), game.peers[peer].bot.as_ref().unwrap().trace());
    assert!(game.server.sim.door_states.iter().any(|d| d.open >= 0.95));
}

#[test]
fn room_and_floor_patches_do_not_merge_overlapping_stories() {
    let a = [100.0, 0.0, 100.0];
    assert_ne!(key(1, a), key(2, a));
    assert_ne!(key(1, a), key(1, [a[0], a[1] + 300.0, a[2]]));
}

#[test]
fn an_unattached_ship_corner_uses_local_recovery() {
    let map = "data/map/rm02/rm02.map";
    if !std::path::Path::new(map).exists() { return; }
    let mut sim = build_sim(&crate::net::proto::Setup { map: map.to_string(), ..demo_setup() }).unwrap();
    let from = [29220.0, 32767.0, 33027.0];
    sim.add_player(from);
    sim.tick(&[Input::default()], TICK);
    let grid = Navigation::for_map(&sim, map, None);
    let mut bot = Bot::new(Skill::normal(), 42);
    for _ in 0..600 {
        let input = bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
        sim.tick(&[input], TICK);
    }
    assert!(flat(sim.players[0].player.pos, from) > 100.0, "{}", bot.trace());
}

#[test]
fn a_ship_corner_without_an_attachment_uses_committed_recovery() {
    let map = "data/map/rm02/rm02.map";
    if !std::path::Path::new(map).exists() { return; }
    let mut sim = build_sim(&crate::net::proto::Setup { map: map.to_string(), ..demo_setup() }).unwrap();
    sim.set_doors_open(1.0);
    let from = [28989.096, 32767.0, 32630.781];
    let nav = Navigation::for_map(&sim, map, None);
    assert!(!nav.grid.attached(&sim.world, from, &sim.standing_rooms(from, None).0));
    sim.add_player(from);
    let probe = [from[0], from[1] + 10.0, from[2]];
    let (rooms, hint) = sim.standing_rooms(from, None);
    let around = sim.visibility.player_rooms(&sim.level, probe, hint);
    sim.players[0].collision_rooms = Some(sim.visibility.solid_rooms(&sim.level, &rooms, &around, &[probe, sim.eye(0)]));
    sim.tick(&[Input::default()], TICK);
    assert!(flat(sim.players[0].player.pos, from) < 1.0, "initial feet moved to {:?}", sim.players[0].player.pos);
    let mut bot = Bot::new(Skill::normal(), 123);
    let grid = nav;
    let mut trace = String::new();
    let mut presses = 0;
    for tick in 0..600 {
        let input = bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
        presses += usize::from(input.use_key);
        sim.tick(&[input], TICK);
        if tick % 60 == 0 { trace.push_str(&format!("\n{tick}: at={:?} forward={} look={:?} {}", sim.players[0].player.pos, input.forward, input.look(), bot.trace())); }
    }
    assert!(flat(sim.players[0].player.pos, from) > 100.0, "{trace}");
    assert!(presses > 0, "the touching leaf was not used");
}

#[test]
fn a_blocked_destination_is_replanned_then_cooled_down() {
    let mut sim = build_sim(&demo_setup()).unwrap();
    sim.add_player([-350.0, 0.0, -350.0]);
    sim.tick(&[Input::default()], TICK);
    let map = Navigation::for_map(&sim, "demo", Some(sim.players[0].player.pos));
    let mut nav = Navigator::new(&map);
    nav.steer(&map, &sim, 0, &Frame::default(), 0.0, 8, None);
    let goal = nav.goal.unwrap();
    for tick in 1..=390 {
        nav.steer(&map, &sim, 0, &Frame::default(), tick as f32 * TICK, 8, None);
    }
    assert!(nav.replans >= 3);
    assert!(nav.cooldown[goal] > 6.5);
    assert_ne!(nav.goal, Some(goal));
    assert_eq!(nav.visited.iter().filter(|v| v.is_some()).count(), 1);
}

#[test]
fn a_client_walks_the_repaired_citystreet_route() {
    let Some(sim) = citystreet() else { return };
    let from = sim.bases[0][0];
    let to = sim.bases[1][0];
    let setup = crate::net::proto::Setup { map: "data/map/m00/citystreet_large.map".to_string(),
        bases: sim.bases.clone(), ..demo_setup() };
    let mut game = crate::net::lab::Game::with_spawns(crate::net::link::Conditions::lan(), setup, vec![from]);
    let peer = game.join_bot(Skill::normal(), 42);
    game.run(0.5);
    let own = game.me(peer);
    let grid = game.navigation.as_ref().unwrap();
    let state = &mut game.peers[peer];
    assert!(state.bot.as_mut().unwrap().navigation.as_mut().unwrap().route(grid, state.sim.as_ref().unwrap(), own, to, 1.0));
    let mut reached = false;
    for _ in 0..2400 {
        game.step();
        let at = game.me(peer);
        if flat(at, to) < 100.0 && (at[1] - to[1]).abs() < 45.0 { reached = true; break; }
    }
    assert!(reached, "at={:?}, {}", game.me(peer), game.peers[peer].bot.as_ref().unwrap().trace());
}

#[test]
fn a_bot_explores_the_demo_without_revisiting_only_one_corner() {
    let mut sim = build_sim(&demo_setup()).unwrap();
    sim.add_player([-350.0, 0.0, -350.0]);
    sim.tick(&[Input::default()], TICK);
    let grid = Navigation::for_map(&sim, "demo", None);
    let mut bot = Bot::new(Skill::normal(), 8);
    for _ in 0..1200 {
        let input = bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
        sim.tick(&[input], TICK);
    }
    let nav = bot.navigation.as_ref().unwrap();
    assert!(nav.visited.iter().filter(|v| v.is_some()).count() >= 4, "{}", bot.trace());
    assert!(nav.blocked_seconds < 4.0, "{}", bot.trace());
}

#[test]
fn reaching_a_patch_boundary_does_not_repeat_the_finished_path() {
    let Some(mut sim) = citystreet() else { return };
    sim.add_player(sim.bases[0][0]);
    sim.tick(&[Input::default()], TICK);
    let grid = Navigation::for_map(&sim, "data/map/m00/citystreet_large.map", None);
    let mut bot = Bot::new(Skill::normal(), 42);
    for _ in 0..1800 {
        let input = bot.think(&sim, Some(&grid), 0, &Frame::default(), RoundState::default());
        sim.tick(&[input], TICK);
    }
    assert!(bot.navigation.as_ref().unwrap().visited.iter().filter(|v| v.is_some()).count() > 10, "{}", bot.trace());
}
