//! A client with no window, for trying the server: it joins, plays a scripted game, and says how
//! it went. It runs the same client and copy of the game that `rr --connect` does.
//!
//!   rrbot --connect 127.0.0.1:27960 --keys WF --seconds 10
//!
//! `--keys` says what the player does, with the letters of the viewer's `--hold`: W A S D to move,
//! F to fire, T to take out a grenade and throw it, L to reload, C to crouch.
//!
//!   rrbot --connect 127.0.0.1:27960 --fight --skill normal
//!
//! `--fight` plays instead, as an opponent (see `net::bot`): it explores, and shoots at whoever it
//! sees, until the server goes away (or for `--seconds`). `--skill` is easy, normal or hard, and
//! `--trace` prints where it is and whom it fights once a second.

use std::net::ToSocketAddrs;
use std::time::{Duration, Instant};

use rogue_reborn::net::bot::{Behavior, Bot, Skill, say_ready};
use rogue_reborn::net::client::{Client, Status};
use rogue_reborn::net::link::{Conditions, Impaired, UdpLink};
use rogue_reborn::net::setup::build_sim;
use rogue_reborn::sim::{Event, Input, Side, TICK};

/// Maximum inputs sent in a catch-up burst. Drop older missed ticks after that.
const CATCH_UP: usize = 5;

fn usage(problem: &str) -> ! {
    if !problem.is_empty() {
        eprintln!("rrbot: {problem}");
    }
    eprintln!("usage: rrbot --connect HOST:PORT [--keys WASDFTL] [--seconds N] [--yaw DEGREES] [--turn DEGREES_PER_SECOND]");
    eprintln!("       rrbot --connect HOST:PORT --fight [--skill easy|normal|hard] [--seconds N, 0 for as long as the server runs] [--trace]");
    eprintln!("             [--side blue|gold]   (the side to ask for in a team game)");
    eprintln!("             [--name NAME]        (the name to ask for; BOT, numbered, without one)");
    eprintln!("             [--behavior legacy|navigate] [--bot-seed N]");
    eprintln!("             [--sim-latency MS] [--sim-jitter MS] [--sim-loss PERCENT] [--quiet]");
    std::process::exit(if problem.is_empty() { 0 } else { 2 });
}

fn main() {
    let (mut connect, mut keys, mut seconds, mut yaw, mut turn) = (None, String::new(), 10.0f64, 0.0f32, 0.0f32);
    let (mut latency, mut jitter, mut loss, mut quiet) = (0.0f64, 0.0f64, 0.0f32, false);
    let (mut fight, mut skill, mut seconds_given, mut trace) = (false, Skill::normal(), false, false);
    let mut side = None;
    let mut behavior = Behavior::Navigate;
    let mut bot_seed = None;
    // Bots are "BOT", and the server numbers them: "BOT 2", "BOT 3"
    let mut name = "BOT".to_string();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().unwrap_or_else(|| usage(&format!("{name} needs a value")));
        match arg.as_str() {
            "--connect" => connect = Some(value("--connect")),
            "--side" => side = Some(Side::named(&value("--side")).unwrap_or_else(|| usage("--side expects blue or gold"))),
            "--name" => name = value("--name"),
            "--behavior" => behavior = Behavior::named(&value("--behavior")).unwrap_or_else(|| usage("--behavior expects legacy or navigate")),
            "--bot-seed" => bot_seed = Some(value("--bot-seed").parse::<u64>().unwrap_or_else(|_| usage("--bot-seed expects an integer"))),
            "--keys" => keys = value("--keys").to_ascii_uppercase(),
            "--seconds" => {
                seconds = value("--seconds").parse().unwrap_or_else(|_| usage("--seconds expects a number"));
                seconds_given = true;
            }
            "--fight" => fight = true,
            "--trace" => trace = true,
            "--skill" => skill = Skill::named(&value("--skill")).unwrap_or_else(|| usage("--skill expects easy, normal or hard")),
            "--yaw" => yaw = value("--yaw").parse().unwrap_or_else(|_| usage("--yaw expects degrees")),
            "--turn" => turn = value("--turn").parse().unwrap_or_else(|_| usage("--turn expects degrees a second")),
            "--sim-latency" => latency = value("--sim-latency").parse::<f64>().unwrap_or_else(|_| usage("--sim-latency expects milliseconds")) / 1000.0,
            "--sim-jitter" => jitter = value("--sim-jitter").parse::<f64>().unwrap_or_else(|_| usage("--sim-jitter expects milliseconds")) / 1000.0,
            "--sim-loss" => loss = value("--sim-loss").parse::<f32>().unwrap_or_else(|_| usage("--sim-loss expects a percentage")) / 100.0,
            "--quiet" => quiet = true,
            "--help" | "-h" => usage(""),
            other => usage(&format!("unknown option {other}")),
        }
    }
    let Some(connect) = connect else { usage("--connect is needed") };
    // A fighter plays for as long as the server runs, unless told otherwise
    if fight && !seconds_given || seconds <= 0.0 {
        seconds = f64::INFINITY;
    }
    let server_addr = connect.to_socket_addrs().ok().and_then(|mut a| a.next()).unwrap_or_else(|| usage(&format!("cannot find {connect}")));

    let mut link = Impaired::new(
        UdpLink::bind("0.0.0.0:0").expect("cannot open a socket"),
        Conditions { latency, jitter, loss, duplicate: 0.0, ..Conditions::NONE },
        std::process::id() as u64,
    );
    let to_server = link.inner_mut().addr_of(server_addr);
    let token = (std::process::id() ^ 0x5bd1e995).wrapping_mul(2654435761);
    let build = option_env!("GIT_HASH").unwrap_or("unknown");
    let mut client = Client::new(to_server, token, build);
    client.ask_side(side);
    client.set_name(&name);

    // Join: ask until the server says yes or no
    let start = Instant::now();
    let setup = loop {
        let now = start.elapsed().as_secs_f64();
        link.set_time(now);
        if let Some(setup) = client.connect(now, &mut link).cloned() {
            break setup;
        }
        match &client.status {
            Status::Refused(why) => {
                eprintln!("rrbot: the server refused us: {why}");
                std::process::exit(1);
            }
            _ if now > 10.0 => {
                eprintln!("rrbot: no answer from {server_addr}");
                std::process::exit(1);
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let mut sim = build_sim(&setup).unwrap_or_else(|e| {
        eprintln!("rrbot: {e:#}");
        std::process::exit(1);
    });
    let slot = client.slot();
    sim.activate_slot(slot, [0.0, 0.0, 0.0]);
    println!("joined as player {slot}: {}, {} characters", setup.map, setup.npcs.len());
    let seed = bot_seed.unwrap_or_else(|| std::process::id() as u64 ^ start.elapsed().as_nanos() as u64);
    let mut bot = fight.then(|| {
        let mut b = Bot::new(skill, seed);
        b.behavior = behavior;
        b
    });
    let mut navigation = None;
    let mut builder = if fight && behavior != Behavior::Legacy {
        match rogue_reborn::net::tactics::Builder::new(setup.clone()) {
            Ok(builder) => Some(builder),
            Err(e) => { eprintln!("rrbot: navigation: {e}"); None }
        }
    } else { None };
    let mut navigation_life = None;
    if fight { println!("bot seed={seed} behavior={behavior:?}"); }

    // Play
    let held = |c: char| keys.contains(c);
    let (mut ticks, mut events, mut worst, mut corrections) = (0u32, Vec::<Event>::new(), 0.0f32, 0u32);
    let played_from = start.elapsed().as_secs_f64();
    let mut next = played_from;
    let mut catch_up = 0;
    while start.elapsed().as_secs_f64() - played_from < seconds && client.status == Status::Connected {
        let now = start.elapsed().as_secs_f64();
        link.set_time(now);
        if now >= next {
            if catch_up == CATCH_UP {
                next = now + TICK as f64;
                catch_up = 0;
                std::thread::sleep(Duration::from_micros(500));
                continue;
            }
            catch_up += 1;
            next += TICK as f64;
            let input = match bot.as_mut() {
                Some(bot) => {
                    let view = client.view(now);
                    if let Some(builder) = &mut builder {
                        match builder.poll(&mut navigation) {
                            Ok(true) if !quiet || trace => {
                                let n = navigation.as_ref().unwrap();
                                println!("navigation: {} points, {} patches, {:.1} ms, about {:.2} MiB, extension {}",
                                    n.grid.len(), n.patches.len(), n.built_ms,
                                    n.estimated_bytes() as f64 / 1048576.0, n.extensions);
                            }
                            Err(e) => eprintln!("rrbot: navigation: {e}"),
                            _ => {}
                        }
                        if client.snapshots != 0 && let Err(e) = builder.prepare_start(&navigation, &sim, slot, client.round, &mut navigation_life) {
                            eprintln!("rrbot: navigation: {e}");
                        }
                    }
                    let input = if client.snapshots == 0 { Input::default() }
                        else { bot.think(&sim, navigation.as_ref(), slot, &view, client.round) };
                    input
                }
                None => {
                    let mut input = Input {
                        forward: held('W'), left: held('A'), back: held('S'), right: held('D'), run: !held('R'), crouch: held('C'),
                        fire: held('F') || (held('T') && ticks < 30), ..Input::default()
                    };
                    // T takes out a grenade first and throws it after holding for half a second
                    input.grenade = held('T') && ticks == 0;
                    input.reload = held('L') && ticks == 5;
                    input.set_look(yaw + turn * ticks as f32 * TICK, 0.0);
                    input
                }
            };
            let before = client.snapshots;
            let news = client.tick(now, &mut link, &mut sim, input);
            say_ready(&mut client);
            // A fighter says at once who killed whom, as it may never get to the end
            if fight && !quiet {
                for e in &news {
                    if let Event::Kill { killer, victim, .. } = e {
                        let who = |s: usize| if s == slot { "this bot".to_string() } else { format!("player {s}") };
                        match killer {
                            Some(k) => println!("{} killed {}", who(*k), who(*victim)),
                            None => println!("{} died", who(*victim)),
                        }
                    }
                }
            }
            events.extend(news);
            // Where it is and whom it fights, once a second
            if trace && ticks % 60 == 0 {
                let p = &sim.players[slot];
                let target = bot.as_ref().and_then(|b| b.target());
                println!("t={:.0} round={} phase={:?} pos=({:.0}, {:.0}, {:.0}) hp={:.0} target={target:?}", ticks as f32 * TICK, client.round.number, client.round.phase, p.player.pos[0], p.player.pos[1], p.player.pos[2], p.hp);
                if let Some(bot) = &bot { println!("  {}", bot.trace()); }
            }
            if client.snapshots != before {
                corrections += 1;
                // The first few snapshots put the player where the server has them, which is
                // not a misprediction
                if corrections > 20 {
                    worst = worst.max(client.correction);
                }
            }
            ticks += 1;
        } else {
            catch_up = 0;
            std::thread::sleep(Duration::from_micros(500));
        }
    }
    let me = &sim.players[slot];
    let count = |f: &dyn Fn(&Event) -> bool| events.iter().filter(|e| f(e)).count();
    if !quiet {
        for e in &events {
            if let Event::Log(line) = e {
                println!("{line}");
            }
        }
    }
    println!(
        "final slot={slot} status={:?} pos=({:.1}, {:.1}, {:.1}) ammo={} grenades={} kills={} deaths={} hp={:.0} snapshots={} worst_correction={worst:.2} blasts={} bullet_hits={}",
        client.status, me.player.pos[0], me.player.pos[1], me.player.pos[2], me.ammo, me.grenades_left, me.kills, me.deaths, me.hp,
        corrections, count(&|e| matches!(e, Event::Blast { .. })), count(&|e| matches!(e, Event::BulletHit { .. })),
    );
    client.leave(&mut link);
    if client.status == Status::Lost && ticks == 0 {
        std::process::exit(1);
    }
}
