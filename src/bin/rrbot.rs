//! A client with no window, for trying the server: it joins, plays a scripted game, and says how
//! it went. It runs the same client and copy of the game that `rr --connect` does.
//!
//!   rrbot --connect 127.0.0.1:27960 --keys WF --seconds 10
//!
//! `--keys` says what the player does, with the letters of the viewer's `--hold`: W A S D to move,
//! F to fire, T to take out a grenade and throw it, L to reload.
//!
//!   rrbot --connect 127.0.0.1:27960 --fight --skill normal
//!
//! `--fight` plays instead, as an opponent (see `net::bot`): it wanders, and shoots at whoever it
//! sees, until the server goes away (or for `--seconds`). `--skill` is easy, normal or hard, and
//! `--trace` prints where it is and whom it fights once a second.

use std::net::ToSocketAddrs;
use std::time::{Duration, Instant};

use rogue_reborn::net::bot::{Bot, Skill};
use rogue_reborn::net::client::{Client, Status};
use rogue_reborn::net::link::{Conditions, Impaired, UdpLink};
use rogue_reborn::net::setup::build_sim;
use rogue_reborn::sim::{Event, Input, TICK};

fn usage(problem: &str) -> ! {
    if !problem.is_empty() {
        eprintln!("rrbot: {problem}");
    }
    eprintln!("usage: rrbot --connect HOST:PORT [--keys WASDFTL] [--seconds N] [--yaw DEGREES] [--turn DEGREES_PER_SECOND]");
    eprintln!("       rrbot --connect HOST:PORT --fight [--skill easy|normal|hard] [--seconds N, 0 for as long as the server runs] [--trace]");
    eprintln!("             [--sim-latency MS] [--sim-jitter MS] [--sim-loss PERCENT] [--quiet]");
    std::process::exit(if problem.is_empty() { 0 } else { 2 });
}

fn main() {
    let (mut connect, mut keys, mut seconds, mut yaw, mut turn) = (None, String::new(), 10.0f64, 0.0f32, 0.0f32);
    let (mut latency, mut jitter, mut loss, mut quiet) = (0.0f64, 0.0f64, 0.0f32, false);
    let (mut fight, mut skill, mut seconds_given, mut trace) = (false, Skill::normal(), false, false);
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().unwrap_or_else(|| usage(&format!("{name} needs a value")));
        match arg.as_str() {
            "--connect" => connect = Some(value("--connect")),
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
        Conditions { latency, jitter, loss, duplicate: 0.0 },
        std::process::id() as u64,
    );
    let to_server = link.inner_mut().addr_of(server_addr);
    let token = (std::process::id() ^ 0x5bd1e995).wrapping_mul(2654435761);
    let build = option_env!("GIT_HASH").unwrap_or("unknown");
    let mut client = Client::new(to_server, token, build);

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
    let mut bot = fight.then(|| Bot::new(skill, std::process::id() as u64 ^ start.elapsed().as_nanos() as u64));

    // Play
    let held = |c: char| keys.contains(c);
    let (mut ticks, mut events, mut worst, mut corrections) = (0u32, Vec::<Event>::new(), 0.0f32, 0u32);
    let played_from = start.elapsed().as_secs_f64();
    let mut next = played_from;
    while start.elapsed().as_secs_f64() - played_from < seconds && client.status == Status::Connected {
        let now = start.elapsed().as_secs_f64();
        link.set_time(now);
        if now >= next {
            next += TICK as f64;
            let input = match bot.as_mut() {
                Some(bot) => {
                    let view = client.view(now);
                    bot.think(&sim, slot, &view)
                }
                None => {
                    let mut input = Input {
                        forward: held('W'), left: held('A'), back: held('S'), right: held('D'), run: !held('R'),
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
                println!("t={:.0} pos=({:.0}, {:.0}, {:.0}) hp={:.0} target={target:?}", ticks as f32 * TICK, p.player.pos[0], p.player.pos[1], p.player.pos[2], p.hp);
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
