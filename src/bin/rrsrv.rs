//! The dedicated server: it owns the game and plays it for everyone who connects.
//!
//!   rrsrv --demo                          the built-in box room, with eight characters in it
//!   rrsrv --level rm11 --max-players 4    a level from the game
//!
//! Clients join with `rr --connect HOST:PORT`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rogue_reborn::collision::{CollisionWorld, Params};
use rogue_reborn::levels::{find_level, find_mission_spawns};
use rogue_reborn::net::link::{Conditions, Impaired, UdpLink};
use rogue_reborn::net::proto::{NpcSetup, Setup};
use rogue_reborn::net::server::Server;
use rogue_reborn::net::setup::build_sim;
use rogue_reborn::sim::characters::{Characters, place_npcs};
use rogue_reborn::sim::{Event, MAX_PLAYERS};
use rogue_reborn::{actor, demo, map, weapon};

struct Args {
    port: u16,
    demo: bool,
    map: Option<PathBuf>,
    level: Option<String>,
    npcs: usize,
    max_players: usize,
    seed: u64,
    model: PathBuf,
    no_models: bool,
    /// Grenades each player starts with
    grenades: u32,
    /// Stop after this many seconds and say how things stand
    duration: Option<f64>,
    quiet: bool,
    /// Players cannot hurt each other
    no_pvp: bool,
    /// Judge shots against where the others are now, not where the shooter saw them
    no_lag_compensation: bool,
    /// Snapshots sent to each client a second
    snapshot_rate: f32,
    sim_latency: f64,
    sim_jitter: f64,
    sim_loss: f32,
}

fn parse_args() -> Args {
    let mut a = Args {
        port: 27960, demo: false, map: None, level: None, npcs: demo::MAX_NPCS, max_players: 4, seed: 0x5eed,
        model: PathBuf::from("data/character/team_hrt/blue_M_a.CRP"), no_models: false, grenades: 3, duration: None, quiet: false, no_pvp: false, no_lag_compensation: false,
        snapshot_rate: 30.0, sim_latency: 0.0, sim_jitter: 0.0, sim_loss: 0.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().unwrap_or_else(|| usage(&format!("{name} needs a value")));
        match arg.as_str() {
            "--port" => a.port = value("--port").parse().unwrap_or_else(|_| usage("--port expects a number")),
            "--demo" => a.demo = true,
            "--map" => a.map = Some(PathBuf::from(value("--map"))),
            "--level" => a.level = Some(value("--level")),
            "--npcs" => a.npcs = value("--npcs").parse().unwrap_or_else(|_| usage("--npcs expects a number")),
            "--max-players" => a.max_players = value("--max-players").parse::<usize>().unwrap_or_else(|_| usage("--max-players expects a number")).clamp(1, MAX_PLAYERS),
            "--seed" => a.seed = value("--seed").parse().unwrap_or_else(|_| usage("--seed expects a number")),
            "--model" => a.model = PathBuf::from(value("--model")),
            "--no-models" => a.no_models = true,
            "--grenades" => a.grenades = value("--grenades").parse().unwrap_or_else(|_| usage("--grenades expects a number")),
            "--duration" => a.duration = Some(value("--duration").parse().unwrap_or_else(|_| usage("--duration expects seconds"))),
            "--quiet" => a.quiet = true,
            "--no-pvp" => a.no_pvp = true,
            "--no-lag-compensation" => a.no_lag_compensation = true,
            "--snapshot-rate" => a.snapshot_rate = value("--snapshot-rate").parse().unwrap_or_else(|_| usage("--snapshot-rate expects snapshots a second")),
            "--sim-latency" => a.sim_latency = value("--sim-latency").parse::<f64>().unwrap_or_else(|_| usage("--sim-latency expects milliseconds")) / 1000.0,
            "--sim-jitter" => a.sim_jitter = value("--sim-jitter").parse::<f64>().unwrap_or_else(|_| usage("--sim-jitter expects milliseconds")) / 1000.0,
            "--sim-loss" => a.sim_loss = value("--sim-loss").parse::<f32>().unwrap_or_else(|_| usage("--sim-loss expects a percentage")) / 100.0,
            "--help" | "-h" => usage(""),
            other => usage(&format!("unknown option {other}")),
        }
    }
    a
}

fn usage(problem: &str) -> ! {
    if !problem.is_empty() {
        eprintln!("rrsrv: {problem}");
    }
    eprintln!("usage: rrsrv [--demo | --level NAME | --map PATH] [--port N] [--max-players N] [--npcs N] [--seed N]");
    eprintln!("             [--model FILE] [--no-models] [--grenades N] [--duration SECONDS] [--quiet] [--snapshot-rate HZ] [--no-pvp] [--no-lag-compensation]");
    eprintln!("             [--sim-latency MS] [--sim-jitter MS] [--sim-loss PERCENT]   (to make the connection worse)");
    std::process::exit(if problem.is_empty() { 0 } else { 2 });
}

fn main() {
    let args = parse_args();
    let build = option_env!("GIT_HASH").unwrap_or("unknown");

    // The level, and where everyone starts in it
    let (map_name, level_path) = if args.demo || (args.map.is_none() && args.level.is_none()) {
        ("demo".to_string(), None)
    } else {
        let path = match (&args.map, &args.level) {
            (Some(p), _) => p.clone(),
            (None, Some(name)) => find_level(name).unwrap_or_else(|e| {
                eprintln!("rrsrv: {e}");
                std::process::exit(2);
            }),
            (None, None) => unreachable!(),
        };
        (path.display().to_string(), Some(path))
    };
    let level = match &level_path {
        None => demo::map(),
        Some(p) => map::read(p).unwrap_or_else(|e| {
            eprintln!("rrsrv: could not load {}: {e:#}", p.display());
            std::process::exit(2);
        }),
    };
    let world = CollisionWorld::from_map(&level, Params::default());
    let (player_spawns, npc_spawns): (Vec<[f32; 3]>, Vec<[f32; 3]>) = match &level_path {
        None => (demo::player_spawns(args.max_players), demo::spawns(args.npcs).into_iter().skip(1).collect()),
        Some(p) => {
            let all = find_mission_spawns(p);
            let (mine, rest) = all.split_at(args.max_players.min(all.len()));
            let feet = |q: &[f32; 3]| [q[0], world.ground_at(q[0], q[2], q[1] + 100.0).unwrap_or(q[1]), q[2]];
            (mine.iter().map(feet).collect(), rest.to_vec())
        }
    };
    if player_spawns.is_empty() {
        eprintln!("rrsrv: the level has nowhere for players to start");
        std::process::exit(2);
    }

    // The characters standing about
    let idle = "up_none_cyc";
    let weapon_def = weapon::read(Path::new("data/kit/pw_mp5k.wpn")).ok();
    let characters = if args.no_models { None } else { Characters::load(&args.model, idle) };
    let actors = actor::read_all(Path::new("data/actor"));
    let (npcs, faces) = match &characters {
        Some(c) => place_npcs(&npc_spawns, &[], &world, &c.model, &actors, &|_, p| {
            // In the box room they face the players; elsewhere they are dealt a facing
            let first = player_spawns[0];
            if level_path.is_none() { (first[0] - p[0]).atan2(first[2] - p[2]).to_degrees() } else { (p[0] * 7.0 + p[2] * 3.0) % 360.0 }
        }),
        None => (Vec::new(), Vec::new()),
    };
    let (magazine, reserve) = weapon_def.map_or((30, 900), |w| (w.magazine, w.reserve));
    let setup = Setup {
        map: map_name.clone(),
        model: args.model.display().to_string(),
        idle: idle.to_string(),
        magazine,
        ammo: magazine,
        reserve,
        grenades: args.grenades,
        speed_scale: 1.0,
        seed: args.seed,
        culling: true,
        pvp: !args.no_pvp,
        doors_open: None,
        npcs: npcs.iter().zip(&faces).map(|(n, f)| NpcSetup {
            pos: n.pos, floor: n.floor, yaw: n.yaw, scale: n.scale, stagger: n.stagger, actor: f.map(|a| a as u16),
        }).collect(),
    };
    // The server's own copy of the game is built the way every client's is
    drop((characters, world, level));
    let sim = build_sim(&setup).unwrap_or_else(|e| {
        eprintln!("rrsrv: {e:#}");
        std::process::exit(2);
    });
    let n_npcs = setup.npcs.len();
    let mut server = Server::new(sim, setup, build, player_spawns);
    server.set_snapshot_rate(args.snapshot_rate);
    server.set_lag_compensation(!args.no_lag_compensation);

    let mut link = Impaired::new(
        UdpLink::bind(&format!("0.0.0.0:{}", args.port)).unwrap_or_else(|e| {
            eprintln!("rrsrv: cannot listen on port {}: {e}", args.port);
            std::process::exit(1);
        }),
        Conditions { latency: args.sim_latency, jitter: args.sim_jitter, loss: args.sim_loss, duplicate: 0.0 },
        args.seed,
    );
    println!("rrsrv build {build} listening on port {} for up to {} players: {map_name}, {n_npcs} characters", args.port, args.max_players);
    if args.sim_latency > 0.0 || args.sim_loss > 0.0 {
        println!("pretending to be a bad connection: {:.0} ms each way, {:.0} ms jitter, {:.0}% loss", args.sim_latency * 1000.0, args.sim_jitter * 1000.0, args.sim_loss * 100.0);
    }

    let start = Instant::now();
    let mut players = 0;
    loop {
        let now = start.elapsed().as_secs_f64();
        link.set_time(now);
        for event in server.update(now, &mut link) {
            if let (Event::Log(line), false) = (&event, args.quiet) {
                println!("{line}");
            }
        }
        if server.players() != players {
            players = server.players();
            println!("t={now:.2} {players} player{} connected", if players == 1 { "" } else { "s" });
        }
        if args.duration.is_some_and(|d| now >= d) {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    // How things stand, for whoever wants to check
    println!("server stopped at tick {}, checksum {:016x}", server.tick, server.sim.checksum());
    if server.snapshots_sent > 0 {
        println!("sent {} snapshots, {} bytes each on average", server.snapshots_sent, server.snapshot_bytes / server.snapshots_sent);
    }
    for (i, p) in server.sim.players.iter().enumerate().filter(|(_, p)| p.active) {
        println!("player {i} pos=({:.1}, {:.1}, {:.1}) ammo={} grenades={} kills={} deaths={} hp={:.0}", p.player.pos[0], p.player.pos[1], p.player.pos[2], p.ammo, p.grenades_left, p.kills, p.deaths, p.hp);
    }
}
