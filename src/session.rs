//! One run of the game: a replay, a hosted or joined server, or a plain solo level.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;
use std::net::ToSocketAddrs;
use rogue_reborn::{map, dmp, combat, demo, recording, sim, sound, mission};
use rogue_reborn::collision::{CollisionWorld, Params};
use rogue_reborn::levels::{find_zone_spawn, find_mission_spawns, pick_index, list_levels, find_level, extract_map_num};
use rogue_reborn::lighting::Lighting;
use rogue_reborn::net::client::{Client, Status};
use rogue_reborn::net::host::{HostedServer, STARTUP, new_key};
use rogue_reborn::net::link::{Conditions, Impaired, UdpLink};
use rogue_reborn::net::proto::Setup;
use rogue_reborn::net::record::ClientRecorder;
use rogue_reborn::recording::{Playback, Recorder, Recording};
use rogue_reborn::sim::characters::{Characters, Npc, place_npcs, place_terrorists};
use crate::args::Args;
use crate::camera::Camera;
use crate::opengl::*;
use crate::render::assets::Assets;

// The parts of a session that last until the game ends
pub struct Session {
    pub net_recorder: Option<ClientRecorder>,
    pub joined: Option<Setup>,
    pub net: Option<(Client, Impaired<UdpLink>)>,
    pub net_clock: Instant,
    pub hosted: Option<HostedServer>,
    pub playback: Option<Playback>,
    pub recorder: Option<Recorder>,
    pub seed: u64,
    pub map_path: PathBuf,
    pub map_num: String,
    pub level: Rc<map::Map>,
    pub lighting: Option<Lighting>,
    pub baked: Option<rogue_reborn::lighting::Baked>,
}

// What the game needs to run, once its level and models are loaded
pub struct Game {
    pub camera: Camera,
    pub assets: Assets,
    pub sim: sim::Sim,
    pub me: usize,
    pub npc_actor: Vec<Option<usize>>,
    pub eye_height: f32,
    pub crouch_eye_height: f32,
}

impl Session {
    // Everything before the window: the replay, the server, the seed and the level. None when
    // --list-levels or --info has said what was asked for.
    pub fn start(args: &mut Args) -> Option<(Session, CollisionWorld)> {
        // Load the map
        // let map_path = Path::new("data/map/m00/demolitions_area.MAP");
        // let map_path = Path::new("data/map/rm11/rm11.map");
        // let map_path = Path::new("data/map/rm02/rm02.map");
        if args.list_levels {
            for (dir, name, _) in list_levels() {
                println!("{name:24} data/map/{dir}");
            }
            return None;
        }
        // Playing a recording back: the game starts as it did when it was recorded
        let recorded = args.play.as_ref().map(|path| {
            Recording::read(path).unwrap_or_else(|e| {
                eprintln!("{e:#}");
                std::process::exit(2);
            })
        });
        let playback = recorded.map(|rec| {
            let h = &rec.header;
            let build = option_env!("GIT_HASH").unwrap_or("unknown");
            if h.build != build {
                eprintln!("warning: recorded by build {}, this is {build}, so it may not play back the same", h.build);
            }
            args.demo = h.map == "demo";
            args.map = (!args.demo).then(|| PathBuf::from(&h.map));
            args.level = None;
            args.npcs = Some(h.npcs as usize);
            args.seed = Some(h.seed);
            args.speed_scale = Some(h.speed_scale);
            args.no_cull = !h.culling;
            args.doors_open = h.doors_open;
            args.god = h.god;
            args.model = Some(PathBuf::from(&h.model));
            args.no_models = h.no_models;
            args.model_yaw = h.model_yaw;
            args.actor = h.actor.clone();
            args.pos = Some(h.camera);
            args.yaw = Some(h.yaw);
            args.pitch = Some(h.pitch);
            // The recording says what the player did, so no script and no real mouse
            args.walk = None;
            Playback::new(rec)
        });
        // Hosting: a server of our own for a Team Survival game, which we join as its host. A key
        // in RR_HOST_KEY makes us the host of a server started with it.
        let mut hosted: Option<HostedServer> = None;
        let mut key = std::env::var("RR_HOST_KEY").ok().and_then(|k| k.trim().parse::<u64>().ok()).unwrap_or(0);
        if args.host {
            key = new_key();
            let port = args.port.unwrap_or(27960);
            let mut server_args: Vec<String> = ["--teams", "--rounds", "--port", &port.to_string()].map(String::from).to_vec();
            match (&args.map, &args.level) {
                _ if args.demo => server_args.push("--demo".to_string()),
                (Some(map), _) => server_args.extend(["--map".to_string(), map.display().to_string()]),
                (None, level) => server_args.extend(["--level".to_string(), level.clone().unwrap_or_else(|| "citystreet_large".to_string())]),
            }
            let max_players = Some(args.max_players.clone().unwrap_or_else(|| "8".to_string()));
            for (flag, value) in [("--seed", args.seed.map(|s| s.to_string())), ("--round-time", args.round_time.clone()), ("--base-pair", args.base_pair.clone()), ("--max-players", max_players)] {
                if let Some(v) = value {
                    server_args.extend([flag.to_string(), v]);
                }
            }
            if args.no_friendly_fire {
                server_args.push("--no-friendly-fire".to_string());
            }
            // rrsrv is beside rr
            let exe = std::env::current_exe().map(|e| e.with_file_name(format!("rrsrv{}", std::env::consts::EXE_SUFFIX)));
            match exe.map_err(anyhow::Error::from).and_then(|exe| HostedServer::start(&exe, &server_args, key, STARTUP)) {
                Ok(server) => hosted = Some(server),
                Err(e) => {
                    eprintln!("could not host: {e:#}");
                    std::process::exit(1);
                }
            }
            args.connect = Some(format!("127.0.0.1:{port}"));
        }
        // Joining a server: the game is set up as it says, so ask it before anything is loaded
        let net_clock = Instant::now();
        let mut net: Option<(Client, Impaired<UdpLink>)> = None;
        let mut joined: Option<rogue_reborn::net::proto::Setup> = None;
        // On a server, --record keeps what the player saw (see net::record)
        let mut net_recorder: Option<ClientRecorder> = None;
        if let Some(address) = args.connect.clone() {
            let server = address.to_socket_addrs().ok().and_then(|mut a| a.next()).unwrap_or_else(|| {
                eprintln!("cannot find the server {address}");
                std::process::exit(2);
            });
            let mut link = Impaired::new(
                UdpLink::bind("0.0.0.0:0").expect("cannot open a socket"),
                Conditions { latency: args.sim_latency, jitter: args.sim_jitter, loss: args.sim_loss, duplicate: 0.0, ..Conditions::NONE },
                std::process::id() as u64,
            );
            let to_server = link.inner_mut().addr_of(server);
            let token = (std::process::id() ^ 0x5bd1e995).wrapping_mul(2654435761);
            let mut client = Client::new(to_server, token, option_env!("GIT_HASH").unwrap_or("unknown"));
            client.ask_side(args.side);
            client.set_key(key);
            client.set_name(args.name.as_deref().unwrap_or(""));
            if args.record.is_some() {
                client.start_tape();
                net_recorder = Some(ClientRecorder::new(option_env!("GIT_HASH").unwrap_or("unknown"), token));
            }
            let setup = loop {
                let now = net_clock.elapsed().as_secs_f64();
                link.set_time(now);
                let setup = client.connect(now, &mut link).cloned();
                if let Some(rec) = net_recorder.as_mut() {
                    rec.connected(now, &mut client);
                }
                if let Some(setup) = setup {
                    break setup;
                }
                match &client.status {
                    Status::Refused(why) => {
                        eprintln!("the server refused us: {why}");
                        drop(hosted.take());
                        std::process::exit(1);
                    }
                    _ if now > 10.0 => {
                        eprintln!("no answer from {address}");
                        drop(hosted.take());
                        std::process::exit(1);
                    }
                    _ => {}
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            println!("joined {address} as player {}: {}, {} characters", client.slot(), setup.map, setup.npcs.len());
            args.demo = setup.map == "demo";
            args.map = (!args.demo).then(|| PathBuf::from(&setup.map));
            args.level = None;
            args.seed = Some(setup.seed);
            args.speed_scale = Some(setup.speed_scale);
            args.no_cull = !setup.culling;
            args.doors_open = setup.doors_open;
            args.model = Some(PathBuf::from(&setup.model));
            args.anim = Some(setup.idle.clone());
            args.first_person = true;
            net = Some((client, link));
            joined = Some(setup);
        }
        // Without --seed each run picks its own, which a recording keeps
        let seed = *args.seed.get_or_insert_with(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64)
        });
        let map_buf = match (&args.map, &args.level) {
            _ if args.demo => PathBuf::from("data/map/m00/demo.map"),
            (Some(path), level) => {
                if level.is_some() {
                    eprintln!("--map and --level both given: using --map");
                }
                path.clone()
            }
            (None, Some(name)) => find_level(name).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(2);
            }),
            (None, None) => PathBuf::from("data/map/m00/citystreet_large.map"),
        };
        let map_path = map_buf.as_path();
        let map_num = extract_map_num(map_path).unwrap_or_else(|| String::from("m00"));
        let level = Rc::new(if args.demo { demo::map() } else { map::read(map_path).expect("Failed to load map") });
        // The level's lights (its .dmp file) and the light baked into every corner of it. The
        // built-in room has none, and --flat-light leaves them out.
        let lighting = (!args.flat_light && !args.demo).then(|| {
            let file = map_path.with_extension("dmp");
            rogue_reborn::lighting::Lighting::new(if file.exists() {
                dmp::read(&file).unwrap_or_else(|e| {
                    eprintln!("{e:#}");
                    dmp::Dmp::none()
                })
            } else {
                dmp::Dmp::none()
            }, args.gamma)
        });
        set_world_gamma(args.gamma);
        let baked = lighting.as_ref().map(|l| {
            let start = Instant::now();
            let baked = l.bake(&level);
            println!("lit the level in {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
            baked
        });

        // Collision world built from the render triangles (MAP space)
        let world = CollisionWorld::from_map(&level, Params::default());
        if args.info {
            println!("rooms={} objects={} portals={} planning_levels={} collision_triangles={}",
                level.rooms.rooms.len(), level.geometries.objects.len(),
                level.portals.portals.len(), level.planning_levels.levels.len(),
                world.triangle_count());
            return None;
        }

        Some((Session {
            net_recorder, joined, net, net_clock, hosted, playback, recorder: None, seed, map_path: map_buf, map_num, level, lighting, baked,
        }, world))
    }

    // The player, the characters and the models, once the window is up
    pub fn start_game(&mut self, args: &Args, world: CollisionWorld) -> Game {
        let map_path = self.map_path.as_path();
        // Camera setup: start at mission-provided location
        let mut camera = Camera::new();

        // negate z_lh because original coordinates were RH for Direct3D
        // TODO: pull a player spawn from the map itself
        camera.position = [33924.6_f32, 33307.0_f32, -30028.3_f32];   // CSL
        // Spawn priority: --pos, --spawn-zone, a random mission spawn point, then
        // the fallback position above
        let mission_spawns = if args.demo { demo::spawns(args.npcs.unwrap_or(demo::MAX_NPCS)) } else { find_mission_spawns(map_path) };
        let mut own_spawn = None;
        let spawn = if args.pos.is_some() {
            None
        } else if let Some(zone) = args.spawn_zone.as_deref() {
            find_zone_spawn(map_path, zone, &world).inspect(|s| println!("spawning in zone {zone} at {s:?}"))
        } else if mission_spawns.is_empty() {
            None
        } else {
            let i = args.spawn.map_or_else(|| pick_index(mission_spawns.len(), args.seed), |n| n.clamp(1, mission_spawns.len()) - 1);
            let p = mission_spawns[i];
            // Mission y is a little above the floor; stand on the floor
            let floor = world.ground_at(p[0], p[2], p[1] + 100.0).unwrap_or(p[1]);
            println!("spawning at mission spawn point {} of {}: {p:?}", i + 1, mission_spawns.len());
            own_spawn = Some(i);
            Some([p[0], floor, p[2]])
        };
        if let Some(spawn) = spawn {
            camera.position = [spawn[0], spawn[1] + world.params().eye_height, -spawn[2]];
        }
        if let Some(p) = &self.playback {
            own_spawn = p.header().own_spawn.map(|i| i as usize);
        }
        // Character models standing at every other mission spawn point
        let model_path = args.model.clone()
            .unwrap_or_else(|| PathBuf::from("data/character/team_hrt/blue_M_a.CRP"));
        // Motions other characters play: idle (or --anim), injury reactions and deaths
        let idle_name = args.anim.clone().unwrap_or_else(|| "up_none_cyc".to_string());
        let mut characters = Characters::load(&model_path, &idle_name);
        let mut assets = Assets::load(&args, &characters, &model_path, &self.joined);
        // The mission whose terrorists stand in the level: the one a recording or the server names,
        // or else the level's own at the difficulty asked for
        let mission_file: Option<PathBuf> = match (&self.playback, &self.joined) {
            (Some(p), _) => p.header().mission.as_ref().map(PathBuf::from),
            (_, Some(s)) => s.mission.as_ref().map(PathBuf::from),
            _ => mission::find(map_path, args.difficulty).filter(|_| !args.demo),
        };
        let (npcs, npc_actor) = match (&self.joined, &mut characters, args.no_models, &mission_file) {
            (_, Some(c), false, Some(path)) => match mission::read(path) {
                Ok(teams) => {
                    let npcs = place_terrorists(&teams, self.seed, &mission::setting(path), &world, c);
                    println!("{} terrorists from {}", npcs.len(), path.display());
                    let n = npcs.len();
                    (npcs, vec![None; n])
                }
                Err(e) => {
                    eprintln!("{e:#}");
                    (Vec::new(), Vec::new())
                }
            },
            (Some(s), _, _, _) => (
                s.npcs.iter().map(|n| Npc {
                    health: combat::Health::new(), pos: n.pos, floor: n.floor, yaw: n.yaw, scale: n.scale, stagger: n.stagger, ..Npc::default()
                }).collect(),
                s.npcs.iter().map(|n| n.actor.map(|a| a as usize)).collect(),
            ),
            // Characters at the spawn points
            // `--npcs` caps them in any level. 0 skips the terrorists and their walking grid.
            (None, Some(c), false, None) => place_npcs(&mission_spawns[..args.npcs.unwrap_or(usize::MAX).min(mission_spawns.len())], own_spawn.as_slice(), &world, &c.model, &assets.actors, &|i, p| {
                args.model_yaw.unwrap_or_else(|| match mission_spawns.first() {
                    // In the demo room everyone faces the player
                    Some(player) if args.demo => (player[0] - p[0]).atan2(player[2] - p[2]).to_degrees(),
                    _ => i as f32 * 137.5,
                })
            }),
            _ => (Vec::new(), Vec::new()),
        };
        assets.load_held(&characters);
        if let Some(p) = args.pos { camera.position = p; }
        if let Some(y) = args.yaw { camera.yaw = y; }
        if let Some(p) = args.pitch { camera.pitch = p.clamp(-89.0, 89.0); }

        // The game: the player stands on the floor below the camera
        let eye_height = world.params().eye_height;
        let crouch_eye_height = world.params().crouch_eye_height;
        let (mag_size, reserve) = match (&self.playback, &self.joined) {
            (Some(p), _) => (p.header().magazine, p.header().reserve),
            (_, Some(s)) => (s.magazine, s.reserve),
            _ => assets.weapon_def.as_ref().map_or((30, 900), |w| (w.magazine, w.reserve)),
        };
        // A scripted run can hold L to start with a short magazine and reload it, and E with it empty
        let scripted = |key| args.walk.is_some() && args.hold.as_deref().is_some_and(|h| h.to_ascii_uppercase().contains(key));
        let ammo = match &self.playback {
            Some(p) => p.header().ammo,
            None if self.joined.is_some() => self.joined.as_ref().map_or(mag_size, |s| s.ammo),
            None if scripted('L') => if scripted('E') { 0 } else { 10 },
            None => mag_size,
        };
        let grenades = self.playback.as_ref().map(|p| p.header().grenades).or(self.joined.as_ref().map(|s| s.grenades)).or(args.grenades).unwrap_or(3);
        self.recorder = args.record.as_ref().filter(|_| self.net.is_none()).map(|_| Recorder::new(recording::Header {
            build: option_env!("GIT_HASH").unwrap_or("unknown").to_string(),
            map: if args.demo { "demo".to_string() } else { map_path.display().to_string() },
            npcs: args.npcs.unwrap_or(demo::MAX_NPCS) as u32,
            own_spawn: own_spawn.map(|i| i as u32),
            camera: camera.position,
            yaw: camera.yaw,
            pitch: camera.pitch,
            seed: args.seed.unwrap_or(0x5eed),
            magazine: mag_size,
            ammo,
            reserve,
            grenades,
            speed_scale: args.speed_scale.unwrap_or(1.0),
            culling: !args.no_cull,
            doors_open: args.doors_open,
            model: model_path.display().to_string(),
            no_models: args.no_models,
            model_yaw: args.model_yaw,
            actor: args.actor.clone(),
            mission: mission_file.as_ref().map(|p| p.display().to_string()),
            god: args.god,
        }));
        let mut sim = sim::Sim::new(
            self.level.clone(),
            world,
            sim::Config {
                magazine: mag_size,
                ammo,
                reserve,
                grenades,
                speed_scale: args.speed_scale.unwrap_or(1.0),
                seed: args.seed.unwrap_or(0x5eed),
                culling: !args.no_cull,
            },
            characters,
            npcs,
        );
        // The player on the floor below the camera: slot 0, who this viewer plays
        let feet = [camera.position[0], camera.position[1] - eye_height, -camera.position[2]];
        // On a server the slot is the server's choice, and the first snapshot says where in it the player stands
        let me = match &self.net {
            Some((client, _)) => {
                sim.activate_slot(client.slot(), feet);
                client.slot()
            }
            None => sim.add_player(feet).expect("a game has room for a player"),
        };
        if let Some(open) = args.doors_open {
            sim.set_doors_open(open);
        }
        sim.god = args.god;
        if let Some(s) = &self.joined {
            // Set up as `build_sim` sets up the server's game, so that a recording plays back
            sim.pvp = s.pvp;
        }
        if let Some(rec) = self.net_recorder.as_mut() {
            rec.began(feet, args.god, &sim);
        }

        Game { camera, assets, sim, me, npc_actor, eye_height, crouch_eye_height }
    }

    // What runs after the loop: leave the server, write the recordings, report a replay, and
    // start the viewer again if F3 asked for it. `release` holds the window and the sound
    // device then, which are let go first so the new viewer has them to itself.
    pub fn finish(&mut self, args: &Args, sim: &sim::Sim, release: Option<(glfw::PWindow, Option<sound::Sound>)>) {
        if let Some((client, link)) = self.net.as_mut() {
            client.leave(link);
        }
        if let (Some(rec), Some(path)) = (&self.net_recorder, &args.record) {
            let rec = rec.recording(sim);
            match rec.write(path) {
                Ok(()) => println!("recorded {:.1} s of the game as you saw it to {}", rec.seconds(), path.display()),
                Err(e) => eprintln!("{e:#}"),
            }
        }
        if let (Some(rec), Some(path)) = (self.recorder.take(), &args.record) {
            let rec = rec.finish(sim);
            match rec.write(path) {
                Ok(()) => println!("recorded {} ticks ({:.1} s) to {}", rec.inputs.len(), rec.seconds(), path.display()),
                Err(e) => eprintln!("{e:#}"),
            }
        }
        if let Some(pb) = &self.playback {
            let (done, all) = pb.progress();
            match pb.desync() {
                None => println!("played {done} of {all} ticks: the game matched the recording"),
                Some(d) => {
                    println!("played {done} of {all} ticks: {d}");
                    std::process::exit(1);
                }
            }
        }

        if release.is_some() {
            drop(release);
            let exe = std::env::current_exe().expect("cannot find the viewer to restart");
            match std::process::Command::new(exe).args(std::env::args_os().skip(1)).spawn() {
                Ok(_) => {}
                Err(e) => eprintln!("could not reload: {e}"),
            }
        }
    }
}
