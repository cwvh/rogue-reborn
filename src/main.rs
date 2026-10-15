mod args;
mod camera;
mod debug;
mod draw;
mod hud;
mod opengl;
mod player;
mod textures;

use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use glfw::{Action, Context, Key, WindowEvent, CursorMode};
use std::sync::Arc;
use std::net::ToSocketAddrs;
use rogue_reborn::{map, rsb, actor, combat, demo, grenade, recording, skeleton, sim, sound, weapon, ambience, glass};
use rogue_reborn::net::client::{Client, Status};
use rogue_reborn::net::animation::Doing;
use rogue_reborn::net::link::{Conditions, Impaired, UdpLink};
use rogue_reborn::recording::{Playback, Recorder, Recording};
use rogue_reborn::sim::characters::{Characters, Npc, Rig, place_npcs};
use rogue_reborn::sim::movement::{Movement, ground_speed, movement_from_keys};
use rogue_reborn::collision::{CollisionWorld, Params};
use rogue_reborn::impact::{self, Surface};
use crate::opengl::*;
use crate::args::parse_args;
use crate::camera::{Camera, listener_of, apply_camera};
use rogue_reborn::util::unit3;
use crate::player::{PlayerMotions, is_upper_body};
use crate::draw::{WeaponDraw, draw_weapon, draw_character, draw_glass, draw_fracture, build_mesh, Which, draw_plan, wipe_opening, room_color};
use crate::hud::{draw_hud, draw_debug_text};
use crate::debug::{DebugReport, write_debug_dump};
use rogue_reborn::levels::{find_zone_spawn, find_mission_spawns, pick_index, list_levels, find_level, extract_map_num};
use rogue_reborn::util::find_file;
use crate::textures::{load_character_texture, ensure_texture};

/// Soot marks a blast can leave
const SCORCH_VARIANTS: usize = 4;

/// Frames to skip before timing, while textures load
const BENCH_WARMUP: u32 = 30;

// How long F4 records, in seconds
const CLIP_SECONDS: f32 = 2.0;

fn main() {
    // Load the map
    // let map_path = Path::new("data/map/m00/demolitions_area.MAP");
    // let map_path = Path::new("data/map/rm11/rm11.map");
    // let map_path = Path::new("data/map/rm02/rm02.map");
    let mut args = parse_args();
    if args.list_levels {
        for (dir, name, _) in list_levels() {
            println!("{name:24} data/map/{dir}");
        }
        return;
    }
    // Playing a recording back: the game starts as it did when it was recorded
    let recorded = args.play.as_ref().map(|path| {
        Recording::read(path).unwrap_or_else(|e| {
            eprintln!("{e:#}");
            std::process::exit(2);
        })
    });
    let mut playback = recorded.map(|rec| {
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
    // Joining a server: the game is set up as it says, so ask it before anything is loaded
    let net_clock = Instant::now();
    let mut net: Option<(Client, Impaired<UdpLink>)> = None;
    let mut joined: Option<rogue_reborn::net::proto::Setup> = None;
    if let Some(address) = args.connect.clone() {
        let server = address.to_socket_addrs().ok().and_then(|mut a| a.next()).unwrap_or_else(|| {
            eprintln!("cannot find the server {address}");
            std::process::exit(2);
        });
        let mut link = Impaired::new(
            UdpLink::bind("0.0.0.0:0").expect("cannot open a socket"),
            Conditions { latency: args.sim_latency, jitter: args.sim_jitter, loss: args.sim_loss, duplicate: 0.0 },
            std::process::id() as u64,
        );
        let to_server = link.inner_mut().addr_of(server);
        let token = (std::process::id() ^ 0x5bd1e995).wrapping_mul(2654435761);
        let mut client = Client::new(to_server, token, option_env!("GIT_HASH").unwrap_or("unknown"));
        let setup = loop {
            let now = net_clock.elapsed().as_secs_f64();
            link.set_time(now);
            if let Some(setup) = client.connect(now, &mut link).cloned() {
                break setup;
            }
            match &client.status {
                Status::Refused(why) => {
                    eprintln!("the server refused us: {why}");
                    std::process::exit(1);
                }
                _ if now > 10.0 => {
                    eprintln!("no answer from {address}");
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
    let level = Arc::new(if args.demo { demo::map() } else { map::read(map_path).expect("Failed to load map") });

    // Build a flattened mesh from the MAP geometry
    let mesh = build_mesh(&level);
    // Collision world built from the render triangles (MAP space)
    let world = CollisionWorld::from_map(&level, Params::default());
    // The shards of broken glass fall for a moment afterwards
    let mut fractures: Vec<glass::Fracture> = Vec::new();

    if args.info {
        println!("rooms={} objects={} portals={} planning_levels={} collision_triangles={}",
            level.rooms.rooms.len(), level.geometries.objects.len(),
            level.portals.portals.len(), level.planning_levels.levels.len(),
            world.triangle_count());
        return;
    }

    const WIDTH: i32 = 1024;
    const HEIGHT: i32 = 768;

    // Initialize GLFW. The portal drawing masks the screen with the stencil buffer, so ask
    // for one rather than hope the default has it.
    let mut glfw = glfw::init_no_callbacks().unwrap();
    glfw.window_hint(glfw::WindowHint::StencilBits(Some(8)));
    glfw.window_hint(glfw::WindowHint::DepthBits(Some(24)));
    let (mut window, events) = glfw
        .create_window(WIDTH as _, HEIGHT as _, "Wireframe Viewer", glfw::WindowMode::Windowed)
        .expect("Failed to create GLFW window");
    window.make_current();
    {
        let mut stencil_bits = 0;
        unsafe { glGetIntegerv(0x0D57, &mut stencil_bits) }; // GL_STENCIL_BITS
        if stencil_bits == 0 {
            eprintln!("warning: no stencil buffer, so rooms seen through doorways will not be masked (use --no-cull)");
        }
    }
    // vsync gives steady pacing; --max-fps replaces it with an explicit cap
    glfw.set_swap_interval(if args.max_fps.is_some() { glfw::SwapInterval::None } else { glfw::SwapInterval::Sync(1) });
    window.set_key_polling(true);
    window.set_cursor_pos_polling(true);
    window.set_framebuffer_size_polling(true);
    window.set_scroll_polling(true);
    window.set_mouse_button_polling(true);
    gl::load_with(|s| window.get_proc_address(s) as *const _);

    // OpenGL setup
    unsafe {
        glViewport(0, 0, WIDTH, HEIGHT);
        glClearColor(0.1, 0.1, 0.1, 1.0);
        glEnable(0x0B71); // GL_DEPTH_TEST
        glPolygonMode(0x0408, 0x1B01); // GL_FRONT_AND_BACK, GL_LINE
    }

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
    if let Some(p) = &playback {
        own_spawn = p.header().own_spawn.map(|i| i as usize);
    }
    // Character models standing at every other mission spawn point
    let model_path = args.model.clone()
        .unwrap_or_else(|| PathBuf::from("data/character/team_hrt/blue_M_a.CRP"));
    // Motions other characters play: idle (or --anim), injury reactions and deaths
    let idle_name = args.anim.clone().unwrap_or_else(|| "up_none_cyc".to_string());
    let characters = Characters::load(&model_path, &idle_name);
    let load_motion = rogue_reborn::sim::characters::load_motion;
    // The player carries the MP5K, so uses the SMG-holding movement set
    let slot_motion = |slot: &str, default: &str| {
        let name = args.player_anims.iter().rev().find(|(k, _)| k == slot).map_or(default, |(_, m)| m.as_str());
        load_motion(name)
    };
    let player_motions = PlayerMotions {
        idle: slot_motion("idle", "up_cyc_mp5k"),
        walk: slot_motion("walk", "tmu_wk1"),
        run: slot_motion("run", "tmu_rn"),
        back: slot_motion("back", "tmu_bk1"),
        left: slot_motion("left", "tms_l2"),
        right: slot_motion("right", "tms_r2"),
        fire: slot_motion("fire", "fire_up_mp5k"),
        reload: slot_motion("reload", "twru_s9"),
        throw: slot_motion("throw", "gr_ov_up"),
    };

    // The MP5K: definition, model and texture
    let weapon_def = weapon::read(Path::new("data/kit/pw_mp5k.wpn")).ok();
    let load_model_draw = |file: &str, muzzle: [f32; 3]| -> Option<WeaponDraw> {
        let model = map::read_model(&Path::new("data/model").join(file)).ok()?;
        let mut verts = Vec::new();
        let mut uvs = Vec::new();
        for object in &model.geometries.objects {
            for od in &object.object_datas {
                let n = od.faces.face_indices.len().min(od.faces.texture_indices.len());
                for k in 0..n {
                    let (i0, i1, i2) = od.faces.face_indices[k];
                    let (t0, t1, t2) = od.faces.texture_indices[k];
                    for (i, t) in [(i0, t0), (i1, t1), (i2, t2)] {
                        let v = &object.vertices[i as usize];
                        verts.push([v.x, v.y, v.z]);
                        let uv = &od.texture_vertices.uv_coords[t as usize];
                        uvs.push([uv.u, uv.v]);
                    }
                }
            }
        }
        let material = model.materials.materials.first()?.filename.clone();
        let stem = Path::new(&material).file_stem()?.to_string_lossy().to_string();
        let texture = find_file(Path::new("data/texture"), &stem, "rsb")
            .and_then(|p| rsb::read(&p).ok())
            .and_then(|t| upload_rsb_to_gl(&t));
        println!("loaded {} ({} triangles, texture {})", file, verts.len() / 3, if texture.is_some() { "ok" } else { "missing" });
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &verts {
            for a in 0..3 {
                lo[a] = lo[a].min(v[a]);
                hi[a] = hi[a].max(v[a]);
            }
        }
        let center = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
        let length = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f32::max).max(1.0);
        Some(WeaponDraw { verts, uvs, texture, muzzle, center, length })
    };
    let weapon_draw = weapon_def.as_ref().and_then(|w| load_model_draw(&w.model, w.muzzle));
    // The grenade in the hand (the original's grenade.qob is a flat inventory picture)
    let grenade_draw = load_model_draw("rsw_smokegrenade.qob", [0.0; 3]);

    // Bullet holes: the game own sprites, or with --realistic-decals pictures drawn for each surface
    const HOLE_VARIANTS: usize = 6;
    let mut hole_tex: HashMap<(Surface, usize), u32> = HashMap::new();
    if args.realistic_decals {
        for surface in [Surface::Concrete, Surface::Wood, Surface::Metal, Surface::Earth, Surface::Soft] {
            for v in 0..HOLE_VARIANTS {
                let seed = 0x1000 + (v as u64) * 7919 + surface as u64 * 104729;
                let px = impact::hole_texture(surface, seed);
                if let Some(id) = upload_rgba_to_gl(impact::TEXTURE_SIZE as u32, impact::TEXTURE_SIZE as u32, &px) {
                    hole_tex.insert((surface, v), id);
                }
            }
        }
    } else {
        for (surface, stem) in [(Surface::Wood, "SP_wood_hit_1"), (Surface::Concrete, "Bullet_hole_simple_1")] {
            let sprite = find_file(Path::new("data/texture"), stem, "rsb").and_then(|p| rsb::read(&p).ok());
            if let Some(t) = sprite {
                let mut px = t.to_rgba8();
                impact::key_alpha(&mut px, t.width as usize, t.height as usize);
                if let Some(id) = upload_rgba_to_gl(t.width as u32, t.height as u32, &px) {
                    hole_tex.insert((surface, 0), id);
                }
            }
        }
    }
    // Soft clouds for dust and a bright sprite for the flash of a spark
    let cloud_tex = find_file(Path::new("data/texture"), "TGAdirt_cloud", "rsb").and_then(|p| rsb::read(&p).ok()).and_then(|t| upload_rsb_to_gl(&t));
    // The flash sprite sits on a flat color, which would show as a square, so key it out
    let flash_tex = find_file(Path::new("data/texture"), "Sparks1", "rsb").and_then(|p| rsb::read(&p).ok()).and_then(|t| {
        let mut px = t.to_rgba8();
        impact::key_alpha(&mut px, t.width as usize, t.height as usize);
        upload_rgba_to_gl(t.width as u32, t.height as u32, &px)
    });
    // A blast: its flash, its smoke, the ring of its shockwave, and the soot it leaves
    let sprite = |stem: &str| find_file(Path::new("data/texture"), stem, "rsb").and_then(|p| rsb::read(&p).ok()).and_then(|t| upload_rsb_to_gl(&t));
    let glare_tex = sprite("TGAgrenade_explosion_1");
    let smoke_tex = sprite("TGAgrenade_blast_03");
    let ring_tex = sprite("TGAgrenade_smoke_ring_1");
    for v in 0..SCORCH_VARIANTS {
        let px = impact::hole_texture(Surface::Scorch, 0x5C0 + v as u64 * 3571);
        if let Some(id) = upload_rgba_to_gl(impact::TEXTURE_SIZE as u32, impact::TEXTURE_SIZE as u32, &px) {
            hole_tex.insert((Surface::Scorch, v), id);
        }
    }
    let mut decals: Vec<impact::Decal> = Vec::new();
    let mut effects = impact::Effects::default();
    const MAX_DECALS: usize = 300;

    // Where the weapon sits relative to the right hand. The weapon models are made in the
    // hand bone's own space: the origin is the wrist, forward (-Y) runs along the hand as
    // it hangs in the bind pose, and up (+Z) is the way the body faces. So the gun goes on
    // the hand as it is, and the motions aim it: low and ahead at the ready, level when
    // firing. `--weapon-offset` shifts the gun, in the model's own axes.
    let weapon_attach: Option<(usize, skeleton::Transform)> = (|| {
        let skel = &characters.as_ref()?.rig.as_ref()?.skeleton;
        let hand = skel.bones.iter().position(|b| b.name == "RightHand")?;
        Some((hand, skeleton::Transform {
            rot: [0.0, 0.0, 0.0, 1.0],
            pos: args.weapon_offset.unwrap_or([0.0, 0.0, 0.0]),
        }))
    })();

    // One GL texture per part, from the character's own directory
    let part_textures: Vec<Option<u32>> = characters.as_ref().map(|c| &c.model).map_or_else(Vec::new, |c| {
        let dir = model_path.parent().unwrap_or(Path::new("."));
        let mut cache: HashMap<String, Option<u32>> = HashMap::new();
        c.parts.iter().map(|part| {
            *cache.entry(part.material.clone()).or_insert_with(|| {
                let id = load_character_texture(dir, &part.material);
                if id.is_none() {
                    println!("no texture for {}", part.material);
                }
                id
            })
        }).collect()
    });
    // Team members whose faces the models wear: --actor picks one, otherwise
    // they are dealt out in turn
    let mut actors = actor::read_all(Path::new("data/actor"));
    if let Some(name) = &args.actor {
        actors.retain(|a| a.id.eq_ignore_ascii_case(name));
        if actors.is_empty() {
            eprintln!("no actor named {name}");
        }
    }
    // GL textures for each actor's HRT face: (main, blink)
    let faces_dir = Path::new("data/texture/faces");
    let face_textures: Vec<(Option<u32>, Option<u32>)> = actors.iter().map(|a| {
        let load = |p: &Path| rsb::read(p).ok().and_then(|t| upload_rsb_to_gl(&t));
        let found = a.hrt_face.as_deref().and_then(|f| actor::find_face_textures(faces_dir, f));
        match found {
            Some((main, blink)) => (load(&main), blink.and_then(|b| load(&b))),
            None => (None, None),
        }
    }).collect();

    let (npcs, npc_actor) = match (&joined, &characters, args.no_models) {
        (Some(s), _, _) => (
            s.npcs.iter().map(|n| Npc { health: combat::Health::new(), pos: n.pos, floor: n.floor, yaw: n.yaw, scale: n.scale, stagger: n.stagger }).collect(),
            s.npcs.iter().map(|n| n.actor.map(|a| a as usize)).collect(),
        ),
        (None, Some(c), false) => place_npcs(&mission_spawns, own_spawn.as_slice(), &world, &c.model, &actors, &|i, p| {
            args.model_yaw.unwrap_or_else(|| match mission_spawns.first() {
                // In the demo room everyone faces the player
                Some(player) if args.demo => (player[0] - p[0]).atan2(player[2] - p[2]).to_degrees(),
                _ => i as f32 * 137.5,
            })
        }),
        _ => (Vec::new(), Vec::new()),
    };

    if let Some(p) = args.pos { camera.position = p; }
    if let Some(y) = args.yaw { camera.yaw = y; }
    if let Some(p) = args.pitch { camera.pitch = p.clamp(-89.0, 89.0); }

    // The game: the player stands on the floor below the camera
    let eye_height = world.params().eye_height;
    let (mag_size, reserve) = match (&playback, &joined) {
        (Some(p), _) => (p.header().magazine, p.header().reserve),
        (_, Some(s)) => (s.magazine, s.reserve),
        _ => weapon_def.as_ref().map_or((30, 900), |w| (w.magazine, w.reserve)),
    };
    // A scripted run can hold L to start with a short magazine and reload it
    let ammo = match &playback {
        Some(p) => p.header().ammo,
        None if joined.is_some() => joined.as_ref().map_or(mag_size, |s| s.ammo),
        None if args.walk.is_some() && args.hold.as_deref().is_some_and(|h| h.to_ascii_uppercase().contains('L')) => 10,
        None => mag_size,
    };
    let grenades = playback.as_ref().map(|p| p.header().grenades).or(joined.as_ref().map(|s| s.grenades)).unwrap_or(3);
    let mut recorder = args.record.as_ref().map(|_| Recorder::new(recording::Header {
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
    }));
    let mut sim = sim::Sim::new(
        level.clone(),
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
    let me = match &net {
        Some((client, _)) => {
            sim.activate_slot(client.slot(), feet);
            client.slot()
        }
        None => sim.add_player(feet).expect("a game has room for a player"),
    };
    if let Some(open) = args.doors_open {
        sim.set_doors_open(open);
    }

    let mut last_cursor_pos = None;
    window.set_cursor_mode(CursorMode::Disabled);

    let mut render_time = Instant::now();
    let mut last_step = Instant::now();

    let mut keys = HashSet::new();
    let mut textures = HashMap::new();
    let mut textured = args.textured;
    let mut first_person = args.first_person;
    let mut planning_level_index = 0;
    let mut show_portals = args.portals;
    let mut show_collisions = args.collisions;
    let mut room_debug = args.rooms;
    let mut last_rooms: Vec<String> = Vec::new();
    let mut dump_requested = args.dump;
    // F4: a dump of every frame for CLIP_SECONDS, in debug/clip-<time>/: when it started, the folder, the frames so far
    let mut clip: Option<(std::time::Instant, String, u32)> = None;
    // F3 closes the window and starts the viewer again with the same options
    let mut reload = false;
    // Which objects are glass panes (drawn in place), and every object's index
    // Faces lying over others in the same plane (a door frame on its wall) get drawn on top
    let coplanar_layers: Vec<rogue_reborn::coplanar::Layers> = level.geometries.objects.iter().map(rogue_reborn::coplanar::layers).collect();
    let pane_objects: Vec<bool> = level.geometries.objects.iter().map(|o| sim.panes.iter().any(|g| g.name == o.name)).collect();
    let all_objects: Vec<usize> = (0..level.geometries.objects.len()).collect();
    // Doors: each leaf is drawn where its door is. Space or Backspace opens and closes the
    // one being looked at.
    let door_leaf: Vec<bool> = (0..level.geometries.objects.len()).map(|i| sim.doors.iter().any(|d| d.model_index == i)).collect();
    let mut doors_by_room: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, d) in sim.doors.iter().enumerate() {
        for &r in &d.rooms {
            doors_by_room.entry(r).or_default().push(i);
        }
    }
    let all_doors: Vec<usize> = (0..sim.doors.len()).collect();
    // Keys pressed since the sim last ticked, which counts once
    let (mut use_pressed, mut reload_pressed, mut gun_pressed, mut grenade_pressed, mut culling_pressed) = (false, false, false, false, false);
    // A scripted run, or a replay, takes one tick a frame instead of keeping time
    let fixed_step = args.walk.is_some() || (playback.is_some() && args.screenshot.is_some());
    // What the player did in the last tick, which the animation follows
    let mut shown;
    let mut was_free = false;
    let mut drawn_rooms: Vec<String> = Vec::new();
    let mut blink_enabled = !args.no_blink;
    // Sound is optional: no output device just means silence
    let mut sound = if args.no_sound { None } else { sound::Sound::open("data/sound") };
    if let Some(snd) = sound.as_mut() {
        snd.master = args.volume.unwrap_or(1.0).clamp(0.0, 1.0);
    }
    // The level's ambient loops and spot sounds, which follow the rooms the player is in
    let mut ambience = ambience::read(&map_path.with_extension("ass"))
        .map(ambience::Ambience::new)
        .filter(|_| sound.is_some());
    // Full auto: a sustained firing sound that runs while the trigger is held
    let mut firing_sound: Option<rodio::Player> = None;
    let mut was_reloading = false;
    let mut step_phase = 0i32;
    let mut remote_anim = rogue_reborn::net::animation::Animators::default();
    // The latest kills, with when they leave the screen and their color, and how this player died:
    // by whom, and where they were looking before
    let mut feed: Vec<(f32, String, [f32; 3])> = Vec::new();
    let mut killed_by: Option<Option<usize>> = None;
    let mut alive_look: Option<(f32, f32)> = None;
    let mut trigger = false;
    // Right mouse button held: walk instead of the default run
    let mut walk_held = false;
    // A blast leaves the player shaken (the view), dazzled (a white veil) and deafened (muffled sound)
    let (mut shake, mut veil, mut deaf) = (0.0f32, 0.0f32, 0.0f32);
    let master_volume = args.volume.unwrap_or(1.0).clamp(0.0, 1.0);
    // Last shot: start, end, whether it struck a character, and when it fades
    let mut tracer: Option<([f32; 3], [f32; 3], bool, f32)> = None;
    let mut third_person = args.third_person;
    let mut movement = Movement::Idle;
    let mut player_frame = 0.0f32;
    let start_time = Instant::now();
    // Time the sim has yet to run, and where the player stood a tick ago, to draw between ticks
    let mut tick_budget = 0.0f32;
    let mut prev_feet = sim.players[me].player.pos;
    let base_speed = 7.5;
    let mut frames_drawn = 0u32;
    // Frames timed, summed render ms and summed whole-frame ms
    let mut bench = (0u32, 0f64, 0f64);

    while !window.should_close() {
        glfw.poll_events();

        for (_, event) in glfw::flush_messages(&events) {
            match event {
                WindowEvent::Key(key, _, action, _) => {
                    match (key, action) {
                        (Key::Escape, Action::Press) => {
                            window.set_should_close(true)
                        }
                        (Key::T, Action::Press) => {
                            textured = !textured;
                        }
                        (Key::L, Action::Press) => {
                            if !level.planning_levels.levels.is_empty() {
                                planning_level_index = (planning_level_index + 1) % level.planning_levels.levels.len();
                            }
                        }
                        (Key::P, Action::Press) => {
                            show_portals = !show_portals;
                        }
                        (Key::F2, Action::Press) => {
                            dump_requested = true;
                        }
                        (Key::F4, Action::Press) if clip.is_none() => {
                            let dir = format!("debug/clip-{}", crate::debug::stamp());
                            println!("recording {CLIP_SECONDS} seconds to {dir}");
                            clip = Some((std::time::Instant::now(), dir, 0));
                        }
                        (Key::F3, Action::Press) => {
                            reload = true;
                            window.set_should_close(true);
                        }
                        (Key::Space, Action::Press) | (Key::Backspace, Action::Press) => {
                            use_pressed = true;
                        }
                        (Key::V, Action::Press) => {
                            culling_pressed = true;
                        }
                        (Key::G, Action::Press) => {
                            room_debug = !room_debug;
                            last_rooms.clear();
                        }
                        (Key::C, Action::Press) => {
                            show_collisions = !show_collisions;
                        }
                        (Key::B, Action::Press) => {
                            blink_enabled = !blink_enabled;
                        }
                        (Key::F, Action::Press) => {
                            third_person = !third_person;
                        }
                        (Key::Num1, Action::Press) => {
                            gun_pressed = true;
                        }
                        (Key::Num3, Action::Press) => {
                            grenade_pressed = true;
                        }
                        (Key::R, Action::Press) => {
                            reload_pressed = true;
                        }
                        (Key::F1, Action::Press) if playback.is_none() && net.is_none() => {
                            // Into the player's body, who drops in from wherever the view is, and out again
                            first_person = !first_person;
                        }
                        (k, Action::Press) => {
                            keys.insert(k);
                        }
                        (k, Action::Release) => {
                            keys.remove(&k);
                        }
                        _ => {}
                    }
                }
                WindowEvent::CursorPos(xpos, ypos) => {
                    // A scripted run (--walk) or a replay is not steered by a real mouse
                    if let (Some((last_x, last_y)), None, false) = (last_cursor_pos, args.walk, playback.is_some()) {
                        let dx = xpos - last_x;
                        let dy = ypos - last_y;
                        camera.yaw -= dx as f32 * 0.1;
                        camera.pitch -= dy as f32 * 0.1;
                        camera.pitch = camera.pitch.clamp(-89.0, 89.0);
                    }
                    last_cursor_pos = Some((xpos, ypos));
                }
                WindowEvent::FramebufferSize(width, height) => {
                    unsafe { glViewport(0, 0, width, height); }
                }
                WindowEvent::MouseButton(glfw::MouseButton::Button1, action, _) => {
                    trigger = action != Action::Release;
                }
                WindowEvent::MouseButton(glfw::MouseButton::Button2, action, _) => {
                    walk_held = action != Action::Release;
                }
                WindowEvent::Scroll(_x, y) => {
                    if !first_person {
                        let scroll = y as f32;
                        if scroll != 0.0 {
                            let dir = camera.get_forward();
                            let zoom_speed = base_speed * 10.0;
                            camera.position[0] += dir[0] * zoom_speed * scroll;
                            camera.position[1] += dir[1] * zoom_speed * scroll;
                            camera.position[2] += dir[2] * zoom_speed * scroll;
                        }
                    }
                }
                _ => {}
            }
        }

        if let Some(n) = args.walk {
            // --hold picks the keys to press: W A S D, and R for walk (right mouse button)
            let held = args.hold.as_deref().unwrap_or("W").to_ascii_uppercase();
            trigger = frames_drawn < n && held.contains('F');
            // T takes out a grenade and throws it after holding for half a second
            if held.contains('T') {
                grenade_pressed = true;
                trigger = frames_drawn < 30;
            }
            // U presses the use key (Space) on frame 5, to open or close the door in front
            if held.contains('U') && frames_drawn == 5 {
                use_pressed = true;
            }
            // L starts a reload on the first frame (for testing the animation)
            if held.contains('L') && frames_drawn == 0 {
                reload_pressed = true;
            }
            walk_held = frames_drawn < n && held.contains('R');
            for (ch, key) in [('W', Key::W), ('A', Key::A), ('S', Key::S), ('D', Key::D)] {
                if frames_drawn < n && held.contains(ch) { keys.insert(key); } else { keys.remove(&key); }
            }
            if frames_drawn % 10 == 0 {
                println!("frame {frames_drawn:4} feet=({:.1}, {:.1}, {:.1}) on_ground={} movement={movement:?} anim_frame={player_frame:.1}",
                    sim.players[me].player.pos[0], sim.players[me].player.pos[1], sim.players[me].player.pos[2], sim.players[me].player.on_ground);
            }
        }

        // Time since the previous step, measured here so it covers render and vsync
        let raw_dt = last_step.elapsed().as_secs_f32();
        let real_dt = raw_dt.min(0.05);
        last_step = Instant::now();
        let step_dt = if fixed_step { sim::TICK } else { real_dt };

        // Free-fly: WASD moves the view (hold LeftShift to move faster)
        if !first_person && playback.is_none() {
            let mut speed = base_speed;
            if keys.contains(&Key::LeftShift) {
                speed *= 5.0;
            }
            let right = camera.get_right();
            let yaw_rad = camera.yaw.to_radians();
            let fwd_flat = [-yaw_rad.sin(), 0.0, -yaw_rad.cos()];
            // Desired horizontal displacement for this frame, in camera space
            let mut mv = [0.0f32; 2];
            if keys.contains(&Key::W) {
                mv[0] += fwd_flat[0] * speed;
                mv[1] += fwd_flat[2] * speed;
            }
            if keys.contains(&Key::S) {
                mv[0] -= fwd_flat[0] * speed;
                mv[1] -= fwd_flat[2] * speed;
            }
            if keys.contains(&Key::A) {
                mv[0] -= (right[0] * speed) / 2.0;
                mv[1] -= (right[2] * speed) / 2.0;
            }
            if keys.contains(&Key::D) {
                mv[0] += (right[0] * speed) / 2.0;
                mv[1] += (right[2] * speed) / 2.0;
            }
            // mv is per-frame at 60 FPS; scale so free-fly speed does not depend on frame rate
            camera.position[0] += mv[0] * step_dt * 60.0;
            camera.position[2] += mv[1] * step_dt * 60.0;
        }

        // The game runs in fixed ticks, however fast the frames come
        let mut input = sim::Input {
            forward: keys.contains(&Key::W),
            back: keys.contains(&Key::S),
            left: keys.contains(&Key::A),
            right: keys.contains(&Key::D),
            run: !walk_held,
            fire: trigger,
            use_key: use_pressed,
            reload: reload_pressed,
            gun: gun_pressed,
            grenade: grenade_pressed,
            toggle_culling: culling_pressed,
            observer: (!first_person).then(|| [camera.position[0], camera.position[1], -camera.position[2]]),
            ..sim::Input::default()
        };
        input.set_look(camera.yaw, camera.pitch);
        shown = input;
        tick_budget += step_dt;
        let mut events: Vec<sim::Event> = Vec::new();
        let mut ticks = 0;
        while tick_budget >= sim::TICK && ticks < 5 {
            // What the player does comes from the keys, or from the recording
            let tick_input = match playback.as_mut() {
                Some(pb) => match pb.next_input() {
                    Some(i) => i,
                    None => break,
                },
                None => input,
            };
            prev_feet = sim.players[me].player.pos;
            events.extend(match net.as_mut() {
                // On a server the game is its to run: send it what the player did, and put
                // what it says into this copy
                Some((client, link)) => {
                    let now = net_clock.elapsed().as_secs_f64();
                    link.set_time(now);
                    client.tick(now, link, &mut sim, tick_input)
                }
                None => sim.tick(&[tick_input], sim::TICK),
            });
            if let Some(rec) = recorder.as_mut() {
                rec.after_tick(&tick_input, &sim);
            }
            if let Some(pb) = playback.as_mut() {
                if let Some(d) = pb.after_tick(&sim) {
                    eprintln!("DESYNC: {d}");
                }
                // The view follows the recorded player
                (camera.yaw, camera.pitch) = tick_input.look();
                first_person = tick_input.observer.is_none();
                if let Some(o) = tick_input.observer {
                    camera.position = [o[0], o[1], -o[2]];
                }
            }
            // Dropping in from the free view is a jump, not something to glide through
            if was_free && tick_input.observer.is_none() {
                prev_feet = sim.players[me].player.pos;
            }
            was_free = tick_input.observer.is_some();
            shown = tick_input;
            // A key press counts once
            (input.use_key, input.reload, input.gun, input.grenade, input.toggle_culling) = (false, false, false, false, false);
            tick_budget -= sim::TICK;
            ticks += 1;
        }
        if ticks > 0 {
            (use_pressed, reload_pressed, gun_pressed, grenade_pressed, culling_pressed) = (false, false, false, false, false);
        }
        // Past the limit the rest is dropped rather than caught up
        if tick_budget >= sim::TICK {
            tick_budget = 0.0;
        }
        let game_time = sim.time;
        if let Some((client, _)) = &net {
            if client.status != Status::Connected {
                eprintln!("the connection to the server was lost: {:?}", client.status);
                window.set_should_close(true);
            }
        }

        // The view is at the player's eyes, between where they were a tick ago and now
        let draw_feet = if first_person {
            let alpha = if fixed_step { 1.0 } else { (tick_budget / sim::TICK).clamp(0.0, 1.0) };
            let (a, b) = (prev_feet, sim.players[me].player.pos);
            let feet = [a[0] * (1.0 - alpha) + b[0] * alpha, a[1] * (1.0 - alpha) + b[1] * alpha, a[2] * (1.0 - alpha) + b[2] * alpha];
            camera.position = [feet[0], feet[1] + eye_height, -feet[2]];
            feet
        } else {
            sim.players[me].player.pos
        };

        feed.retain(|(until, ..)| *until > game_time);

        // Dead on a server: the view sinks to the floor and turns to whoever did it, and is given
        // back the way it was when the player comes back
        match net.as_ref().and_then(|_| sim.players[me].respawn_at) {
            Some(until) if first_person => {
                let (yaw0, pitch0) = *alive_look.get_or_insert((camera.yaw, camera.pitch));
                let fall = ((game_time - (until - sim::RESPAWN_SECONDS)) / 0.6).clamp(0.0, 1.0);
                camera.position[1] -= (eye_height - 25.0) * fall * fall * (3.0 - 2.0 * fall);
                let (want_yaw, want_pitch) = match killed_by.flatten().filter(|&k| k != me && sim.players[k].active) {
                    Some(k) => {
                        let t = sim.players[k].player.pos;
                        let d = [t[0] - camera.position[0], t[1] + 100.0 - camera.position[1], -t[2] - camera.position[2]];
                        let flat = d[0].hypot(d[2]);
                        ((-d[0]).atan2(-d[2]).to_degrees(), d[1].atan2(flat).to_degrees())
                    }
                    None => (yaw0, pitch0),
                };
                let k = 1.0 - (-5.0 * step_dt).exp();
                camera.yaw += ((want_yaw - camera.yaw + 540.0).rem_euclid(360.0) - 180.0) * k;
                camera.pitch += (want_pitch - camera.pitch) * k;
            }
            Some(_) => {}
            None => {
                if let Some((yaw, pitch)) = alive_look.take() {
                    (camera.yaw, camera.pitch) = (yaw, pitch);
                }
                killed_by = None;
            }
        }

        // A blast leaves the view shaken and whited out, and the sound muffled, for a while
        shake = (shake - 1.4 * step_dt).max(0.0);
        veil = (veil - 1.6 * step_dt).max(0.0);
        deaf = (deaf - 0.2 * step_dt).max(0.0);
        if let Some(snd) = sound.as_mut() {
            snd.master = master_volume * (1.0 - 0.85 * deaf.min(1.0));
        }

        // What happened in the ticks: sounds to play, effects to draw, lines for the log
        for event in events {
            match event {
                sim::Event::Sound { name, at, gain } => {
                    if let Some(snd) = sound.as_mut() {
                        match at {
                            Some(p) => snd.play_at(&name, p, listener_of(&camera), gain),
                            None => snd.play(&name, gain),
                        }
                    }
                }
                sim::Event::Log(line) => println!("{line}"),
                sim::Event::Shot { player, start, end, hit_character } => {
                    if player == me {
                        tracer = Some((start, end, hit_character, game_time + 0.1));
                    }
                }
                sim::Event::BulletHit { point, normal, dir, surface, material, dist, seed } => {
                    // The bullet struck the level: leave a hole and throw some debris
                    let surface = args.decal_surface.unwrap_or(surface);
                    let mut sparked = false;
                    if args.realistic_decals {
                        sparked = effects.impact(surface, point, normal, dir, seed);
                    }
                    println!("t={game_time:.2} shot hit {surface:?} ({material}) at {dist:.0} units{}", if sparked { ", sparks" } else { "" });
                    if surface != Surface::Glass {
                        let (art, variant) = if args.realistic_decals {
                            (surface, (seed >> 33) as usize % HOLE_VARIANTS)
                        } else {
                            (if surface == Surface::Wood { Surface::Wood } else { Surface::Concrete }, 0)
                        };
                        if hole_tex.contains_key(&(art, variant)) {
                            let roll = ((seed >> 20) & 0xFFFF) as f32 / 65536.0;
                            decals.push(impact::Decal::new(point, normal, art, args.realistic_decals, variant, roll));
                            if decals.len() > MAX_DECALS {
                                decals.remove(0);
                            }
                        }
                    }
                    if let Some(snd) = sound.as_mut() {
                        let name = match surface {
                            _ if sparked => if seed & 1 == 0 { "C_BLRIC1.WAV" } else { "C_BLRIC2.WAV" },
                            Surface::Wood => "C_BLWOD.WAV",
                            Surface::Metal => if seed & 1 == 0 { "C_BLMTTK.WAV" } else { "C_BLMTTN.WAV" },
                            Surface::Earth => "C_BLEAR.WAV",
                            _ => "C_BLCON.WAV",
                        };
                        snd.play_at(name, point, listener_of(&camera), 1.0);
                    }
                }
                sim::Event::Glass { pane, at, dir, seed } => {
                    fractures.push(glass::shatter(&sim.panes[pane], at, dir, seed));
                }
                sim::Event::Blast { point, normal, surface, on_surface, seed } => {
                    effects.explosion(point, normal, seed, on_surface);
                    if on_surface && surface != Surface::Glass {
                        let roll = ((seed >> 20) & 0xFFFF) as f32 / 65536.0;
                        decals.push(impact::Decal::scorch(point, normal, 100.0 + (seed >> 50) as f32 % 40.0, (seed >> 33) as usize % SCORCH_VARIANTS, roll));
                        if decals.len() > MAX_DECALS {
                            decals.remove(0);
                        }
                    }
                }
                sim::Event::PlayerBlast { player, shake: s, veil: v } if player == me => {
                    shake = shake.max(s);
                    veil = veil.max(v);
                    if s > 0.1 {
                        deaf = deaf.max(s.min(1.0));
                        if let Some(snd) = sound.as_mut() {
                            snd.ring(3.5 * s.min(1.0) + 1.0, 0.2 * s.min(1.0));
                        }
                    }
                }
                // What happened to other players is not felt here
                sim::Event::PlayerBlast { .. } => {}
                sim::Event::Kill { killer, victim, cause } => {
                    let who = |s: usize| format!("PLAYER {}", s + 1);
                    let text = match (killer, cause) {
                        (Some(k), sim::Cause::HeadShot) => format!("{} HEAD SHOT {}", who(k), who(victim)),
                        (Some(k), sim::Cause::Shot) => format!("{} SHOT {}", who(k), who(victim)),
                        (Some(k), sim::Cause::Grenade) => format!("{} BLEW UP {}", who(k), who(victim)),
                        (None, _) => format!("{} KILLED THEMSELVES", who(victim)),
                    };
                    let color = if victim == me { [1.0, 0.35, 0.3] } else if killer == Some(me) { [1.0, 0.85, 0.3] } else { [1.0, 1.0, 1.0] };
                    feed.push((game_time + 6.0, text, color));
                    if feed.len() > 5 {
                        feed.remove(0);
                    }
                    if victim == me {
                        killed_by = Some(killer);
                    }
                }
            }
        }

        if first_person {
            // The animation follows what the player is trying to do, and plays at the pace
            // they actually move (a wall stops it)
            let dt = step_dt;
            let wanted = movement_from_keys(shown.forward, shown.back, shown.left, shown.right, shown.run);
            let moved = sim.players[me].moved_speed;
            // A wall stops the walking animation; standing still can fire or reload
            let walking = wanted != Movement::Idle && moved >= 5.0;
            let now = if walking {
                wanted
            } else if sim.players[me].throw.is_some() {
                Movement::Throw
            } else if sim.players[me].reload_until.is_some() {
                Movement::Reload
            } else if sim.players[me].gun_trigger && sim.players[me].ammo > 0 {
                Movement::Fire
            } else {
                Movement::Idle
            };
            if now != movement {
                movement = now;
                player_frame = 0.0;
            }
            let rate = if matches!(movement, Movement::Idle | Movement::Fire | Movement::Reload | Movement::Throw) { 1.0 } else { (moved / (ground_speed(movement) * args.speed_scale.unwrap_or(1.0))).clamp(0.5, 3.0) * args.anim_scale.unwrap_or(1.5) };
            let frames = player_motions.get(movement).map_or(1, |m| m.frames.max(1)) as f32;
            player_frame = if let (Movement::Reload, Some(t)) = (movement, sim.players[me].reload_until) {
                // Spread the reload animation across the whole reload time
                (1.0 - (t - game_time) / sim::RELOAD_SECONDS).clamp(0.0, 1.0) * (frames - 1.0)
            } else if let (Movement::Throw, Some(th)) = (movement, sim.players[me].throw) {
                ((game_time - th.start) / sim::THROW_SECONDS).clamp(0.0, 1.0) * (frames - 1.0)
            } else {
                (player_frame + rate * 30.0 * dt) % frames
            };
            // A footstep at the start of each half of a walk or run cycle
            let phase = (player_frame / (frames / 2.0).max(1.0)) as i32;
            if phase != step_phase {
                step_phase = phase;
                if walking && sim.players[me].player.on_ground {
                    if let Some(snd) = sound.as_mut() {
                        snd.play(if movement == Movement::Run { "F_HRCON1.WAV" } else { "F_HWCON1.WAV" }, 1.0);
                    }
                }
            }
        }

        // Reload sound when a reload starts (by key or running dry)
        if sim.players[me].reload_until.is_some() && !was_reloading {
            if let Some(snd) = sound.as_mut() {
                snd.play("C_SBMAG1.WAV", 1.0);
            }
        }
        was_reloading = sim.players[me].reload_until.is_some();

        // Full auto: the firing sound runs while the trigger is held with rounds to fire,
        // then stops with a closing crack
        let firing = sim.firing(me);
        if let Some(snd) = sound.as_mut() {
            if firing && firing_sound.is_none() {
                firing_sound = snd.play_loop("C_SB5KB.WAV", 1.0);
            } else if !firing && firing_sound.take().is_some() {
                snd.play("C_SB5KR.WAV", 1.0);
            }
        }

        // Ambient sound for the rooms around the player's feet
        if let (Some(snd), Some(amb)) = (sound.as_mut(), ambience.as_mut()) {
            let listener = listener_of(&camera);
            let feet = [listener.pos[0], listener.pos[1] - eye_height, listener.pos[2]];
            let rooms: Vec<&str> = level.rooms.rooms.iter()
                .filter(|r| r.contains(feet, 50.0))
                .map(|r| r.section_name.as_str())
                .collect();
            amb.update(snd, &rooms, listener, game_time, step_dt);
        }

        let current_rooms: Vec<&str> = sim.players[me].current_room.map(|i| level.rooms.rooms[i].section_name.as_str()).into_iter().collect();
        if room_debug && current_rooms != last_rooms {
            println!("in rooms: {current_rooms:?}");
            last_rooms = current_rooms.iter().map(|s| s.to_string()).collect();
        }

        // Compute dynamic aspect and clip planes
        let (w, h) = window.get_size();
        let aspect = (w as f32 / h.max(1) as f32).max(0.1);
        // The near plane stays well inside the player's 30 units from a wall, and inside the
        // distance at which a doorway counts as one the player stands in: past it, the wall
        // is cut open, or the doorway's opening is cut and the room beyond goes missing
        const MAX_Z_NEAR: f32 = 4.0;
        let z_near = (mesh.radius / 1000.0).clamp(0.05, MAX_Z_NEAR);
        let z_far = (mesh.radius * 10.0 + 100.0).max(z_near + 1.0);

        // Frame timing for FPS
        let fps = if raw_dt > 0.0 { 1.0 / raw_dt } else { 0.0 };

        render_time = Instant::now();
        unsafe {
            glClearStencil(1);
            glClear(0x00004000 | 0x00000100 | 0x00000400); // GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT
            glMatrixMode(0x1701); // GL_PROJECTION
            glLoadIdentity();
            glu_perspective(60.0, aspect, z_near, z_far);

            glMatrixMode(0x1700); // GL_MODELVIEW
            glLoadIdentity();
            // A blast shakes the view: a quick tremble that dies away
            if shake > 0.002 {
                let t = game_time;
                let s = shake.min(1.0);
                glRotatef(5.0 * s * (t * 47.0).sin(), 0.0, 0.0, 1.0);
                glTranslatef(9.0 * s * (t * 61.0 + 1.0).sin(), 7.0 * s * (t * 53.0 + 2.0).sin(), 0.0);
                glRotatef(2.5 * s * (t * 39.0 + 3.0).sin(), 1.0, 0.0, 0.0);
            }
            // Third person: pull the view back behind and above the player's eyes
            let view = if first_person && third_person {
                // Orbit the view around the player to see them from the side
                let f = Camera { position: [0.0; 3], yaw: camera.yaw + args.view_orbit, pitch: camera.pitch }.get_forward();
                let dist = args.view_dist.unwrap_or(220.0);
                Camera {
                    position: [camera.position[0] - f[0] * dist, camera.position[1] - f[1] * dist + 40.0, camera.position[2] - f[2] * dist],
                    yaw: camera.yaw + args.view_orbit,
                    pitch: camera.pitch,
                }
            } else {
                Camera { position: camera.position, yaw: camera.yaw, pitch: camera.pitch }
            };
            apply_camera(&view);
            glScalef(1.0, 1.0, -1.0);

            // What the portals show from here: the rooms the player is in, and each room
            // seen through the previous one's portals (see visibility::plan)
            let cam_eye = [view.position[0], view.position[1], -view.position[2]];
            let mut plan: Vec<rogue_reborn::visibility::PlanNode> = Vec::new();
            // Each visible room once, with its screen rectangle (unless --portal-paths)
            let mut rect_plan: Vec<rogue_reborn::visibility::RectRoom> = Vec::new();
            if sim.culling && !sim.players[me].render_rooms.is_empty() {
                let (mut modelview, mut projection) = ([0f32; 16], [0f32; 16]);
                glGetFloatv(0x0BA6, modelview.as_mut_ptr()); // GL_MODELVIEW_MATRIX
                glGetFloatv(0x0BA7, projection.as_mut_ptr()); // GL_PROJECTION_MATRIX
                let clip = |v: [f32; 3]| mul_mat4(&projection, mul_mat4(&modelview, [v[0], v[1], v[2], 1.0]));
                if args.portal_paths {
                    plan = sim.players[me].render_rooms.iter().map(|&r| sim.visibility.plan(&level, r, &sim.players[me].body, cam_eye, &sim.blocked_portals, &clip)).collect();
                } else {
                    rect_plan = sim.visibility.rect_plan(&level, &sim.players[me].render_rooms, &sim.players[me].body, cam_eye, &sim.blocked_portals, &clip);
                    // The player's rooms and the doorways they stand in are still cut where
                    // they overlap
                    plan = sim.players[me].render_rooms.iter().map(|&r| sim.visibility.touched_plan(&level, r, &sim.players[me].body, cam_eye, &sim.blocked_portals)).collect();
                }
            }
            let mut visible_rooms: Vec<u32> = rect_plan.iter().map(|rr| rr.room).collect();
            for node in &plan {
                node.rooms(&mut visible_rooms);
            }
            visible_rooms.sort();
            visible_rooms.dedup();
            drawn_rooms = visible_rooms.iter().map(|n| n.to_string()).collect();

            // Combined
            {
                // Textured mode
                if textured {
                    glEnable(0x0DE1); // GL_TEXTURE_2D
                    glPolygonMode(0x0408, 0x1B02); // GL_FRONT_AND_BACK, GL_FILL
                } else {
                    glDisable(0x0DE1); // GL_TEXTURE_2D
                    glPolygonMode(0x0408, 0x1B01); // GL_FRONT_AND_BACK, GL_LINE
                }
                let level_mode = if textured { 0x1B02 } else { 0x1B01 };

                // Determine which room names are active for this planning level (if any)
                let use_filter = false; // !level.planning_levels.levels.is_empty();
                // Iterate map geometry by material groups
                if room_debug {
                    glDisable(0x0DE1); // GL_TEXTURE_2D
                }

                // Draws some of the level's objects, and only the part beyond `plane`.
                // An object belongs to the room its name starts with ("204", "20_doormetal").
                let mut draw_objects = |which: Which, planes: &[[f32; 4]]| {
                    set_clip_planes(planes);
                    let indices: &[usize] = match which {
                        Which::All => &all_objects,
                        Which::Room(n) => sim.visibility.objects_of(n),
                        Which::Unowned => sim.visibility.unowned_objects(),
                    };
                    // The doors of these rooms are drawn along with them, each leaf where
                    // its door is
                    let room_doors: &[usize] = match which {
                        Which::All => &all_doors,
                        Which::Room(n) => doors_by_room.get(&n).map(Vec::as_slice).unwrap_or_default(),
                        Which::Unowned => &[],
                    };
                    let items = indices.iter().map(|&i| (i, None)).chain(room_doors.iter().map(|&d| (sim.doors[d].model_index, Some(d))));
                    for (object_index, door) in items {
                        let object = &level.geometries.objects[object_index];
                        // Panes are drawn in place below, and door leaves with their doors
                        if pane_objects[object_index] || (door_leaf[object_index] && door.is_none()) {
                            continue;
                        }
                        let room = sim.visibility.room_of_object(object_index);
                        if room.is_some_and(|n| args.hide_rooms.contains(&n)) {
                            continue;
                        }
                        if let Some(d) = door {
                            glPushMatrix();
                            glMultMatrixf(sim.doors[d].matrix(sim.door_states[d].open).as_ptr());
                        }
                        if room_debug {
                            let [r, g, b] = room_color(&object.name, current_rooms.iter().any(|n| *n == object.name));
                            glColor3f(r, g, b);
                        }
                    for (od_index, od) in object.object_datas.iter().enumerate() {
                        if use_filter {
                            let rooms = &level.planning_levels.levels[planning_level_index].room_names;
                            if !rooms.iter().any(|r| r == &object.name) {
                                continue;
                            }
                        }
                        let mat_index = od.mn as usize;
                        // Room colors replace the textures, so they show even where textures are missing
                        let tex_id = if room_debug { Some(0) } else { ensure_texture(&level.materials, mat_index, &map_num, &mut textures) };
                        if let Some(tex_id) = tex_id {
                            // TODO put all faces with same tex-id together so I can bind once
                            if !room_debug {
                                glBindTexture(0x0DE1, tex_id); // GL_TEXTURE_2D
                            }

                            // Faces and UVs are parallel arrays
                            let face_count = od.faces.face_indices.len().min(od.faces.texture_indices.len());
                            if face_count == 0 {
                                continue;
                            }

                            // Faces over others in their plane come after, each layer nearer
                            let layers = &coplanar_layers[object_index];
                            for layer in 0..=layers.max {
                            // Most groups have faces in only one of their object's layers
                            if !layers.per_face[od_index][..face_count].contains(&layer) {
                                continue;
                            }
                            if layer > 0 {
                                glEnable(0x8037); // GL_POLYGON_OFFSET_FILL
                                glPolygonOffset(-f32::from(layer), -f32::from(layer));
                            }
                            glBegin(0x0004); // GL_TRIANGLES
                            for i in 0..face_count {
                                if layers.per_face[od_index][i] != layer {
                                    continue;
                                }
                                let (i0, i1, i2) = od.faces.face_indices[i];
                                let (t0, t1, t2) = od.faces.texture_indices[i];

                                let p0 = &object.vertices[i0 as usize];
                                let p1 = &object.vertices[i1 as usize];
                                let p2 = &object.vertices[i2 as usize];

                                let uv0 = &od.texture_vertices.uv_coords[t0 as usize];
                                let uv1 = &od.texture_vertices.uv_coords[t1 as usize];
                                let uv2 = &od.texture_vertices.uv_coords[t2 as usize];

                                if textured {
                                    glTexCoord2f(uv0.u, uv0.v); glVertex3f(p0.x, p0.y, p0.z);
                                    glTexCoord2f(uv1.u, uv1.v); glVertex3f(p1.x, p1.y, p1.z);
                                    glTexCoord2f(uv2.u, uv2.v); glVertex3f(p2.x, p2.y, p2.z);
                                } else {
                                    glVertex3f(p0.x, p0.y, p0.z);
                                    glVertex3f(p1.x, p1.y, p1.z);
                                    glVertex3f(p2.x, p2.y, p2.z);
                                }
                            }
                            glEnd();
                            }
                            if layers.max > 0 {
                                glDisable(0x8037);
                            }
                        }
                    }
                    if door.is_some() {
                        glPopMatrix();
                    }
                    }
                };

                if !rect_plan.is_empty() {
                    // The player's rooms and the doorways they stand in, cut where they
                    // overlap, then every other room whole, within its rectangle, nearest
                    // first, sorted out by depth as the original engine does. Inside the
                    // openings of the player's rooms' own doorways the room beyond shows in
                    // front of them (rm11's hall's stairs inside 119); deeper, overlaps show,
                    // as in the original (the hall's wall in 120's doorway, see BUGS.txt)
                    draw_objects(Which::Unowned, &[]);
                    let mut cut_rooms: Vec<u32> = Vec::new();
                    glEnable(0x0B90); // GL_STENCIL_TEST
                    for node in &plan {
                        node.rooms(&mut cut_rooms);
                        draw_plan(node, 1, level_mode, textured && !room_debug, &mut |room, planes| draw_objects(Which::Room(room), planes));
                    }
                    glDisable(0x0B90);
                    set_clip_planes(&[]);
                    let mut viewport = [0i32; 4];
                    glGetIntegerv(0x0BA2, viewport.as_mut_ptr()); // GL_VIEWPORT
                    glEnable(0x0C11); // GL_SCISSOR_TEST
                    for rr in rect_plan.iter().filter(|rr| !cut_rooms.contains(&rr.room)) {
                        let r = rr.rect;
                        let x0 = viewport[0] + ((r[0].max(-1.0) + 1.0) / 2.0 * viewport[2] as f32).floor() as i32;
                        let y0 = viewport[1] + ((r[1].max(-1.0) + 1.0) / 2.0 * viewport[3] as f32).floor() as i32;
                        let x1 = viewport[0] + ((r[2].min(1.0) + 1.0) / 2.0 * viewport[2] as f32).ceil() as i32;
                        let y1 = viewport[1] + ((r[3].min(1.0) + 1.0) / 2.0 * viewport[3] as f32).ceil() as i32;
                        glScissor(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0));
                        // Through the player's room's own doorways, as the original engine does
                        for &(p, plan) in &rr.doorways {
                            wipe_opening(sim.visibility.polygon(p), plan, cam_eye, level_mode, textured && !room_debug);
                        }
                        draw_objects(Which::Room(rr.room), &[]);
                    }
                    glDisable(0x0C11);
                } else if plan.is_empty() {
                    draw_objects(Which::All, &[]);
                } else {
                    // Things that belong to no room first, then each room through its portals
                    draw_objects(Which::Unowned, &[]);
                    glEnable(0x0B90); // GL_STENCIL_TEST
                    for node in &plan {
                        draw_plan(node, 1, level_mode, textured && !room_debug, &mut |room, planes| draw_objects(Which::Room(room), planes));
                    }
                    glDisable(0x0B90);
                }
                set_clip_planes(&[]);
                glColor3f(1.0, 1.0, 1.0);
            }

            // On a server, what moves is shown a little in the past, between its snapshots, so that
            // it moves smoothly
            let view = net.as_mut().map(|(client, _)| client.view(net_clock.elapsed().as_secs_f64()));

            // A character shows only when a room it is in is drawn: the level overlaps itself,
            // and on rm05 a character on the plane's upper deck pokes out of the top of the hull.
            // `feet` in map space; with no rooms drawn (culling off) everyone shows.
            let shown = |feet: [f32; 3]| visible_rooms.is_empty()
                || sim.visibility.player_rooms(&level, [feet[0], feet[1] + 10.0, feet[2]], None).iter().any(|r| visible_rooms.contains(r));

            // Characters at the spawn points, posed for what they are doing
            if let Some(chars) = &sim.characters {
                let c = &chars.model;
                glDisable(0x0DE1); // GL_TEXTURE_2D
                let clock = start_time.elapsed().as_secs_f32();
                for (i, spot) in sim.npcs.iter().enumerate() {
                    let mut moved;
                    let spot = match view.as_ref().and_then(|v| v.npcs.get(i)) {
                        Some(&(pos, yaw)) => {
                            moved = spot.clone();
                            (moved.pos[0], moved.pos[2], moved.yaw) = (pos[0], pos[2], yaw);
                            &moved
                        }
                        None => spot,
                    };
                    if !shown([spot.pos[0], spot.floor, spot.pos[2]]) {
                        continue;
                    }
                    let idle = args.anim_frame.unwrap_or_else(|| game_time * args.anim_fps.unwrap_or(30.0) + spot.stagger * 9.0);
                    let posed = chars.pose(spot, game_time, idle);
                    let verts: &[[f32; 3]] = if posed.is_empty() { &c.vertices } else { &posed };
                    let y = if posed.is_empty() { spot.pos[1] } else { spot.floor };
                    glPushMatrix();
                    glTranslatef(spot.pos[0], y, spot.pos[2]);
                    glRotatef(spot.yaw, 0.0, 1.0, 0.0);
                    glScalef(spot.scale[0], spot.scale[1], spot.scale[2]);
                    // Eyes shut for 0.15 s every 4 s, staggered per model; the dead keep them shut
                    let blinking = args.eyes_closed || spot.health.is_dead()
                        || (blink_enabled && (clock + spot.stagger) % 4.0 < 0.15);
                    let face = npc_actor[i].map(|a| face_textures[a]).and_then(|(main, blink)| {
                        if blinking { blink.or(main) } else { main }
                    });
                    draw_character(c, verts, &part_textures, face, textured);
                    glPopMatrix();
                }
                glColor4f(1.0, 1.0, 1.0, 1.0);
            }

            // How a posed body is drawn, for the other players and for the player's own
            let draw_body = |chars: &Characters, movement: Movement, frame: f32, feet: [f32; 3], yaw_deg: f32, scale: [f32; 3],
                             face: Option<u32>, hand: Option<&Option<WeaponDraw>>, grenade: bool, flash: bool| -> bool {
                let Some(Rig { skeleton: skel, skin }) = &chars.rig else { return false };
                let Some(mot) = player_motions.get(movement) else { return false };
                let (root, mut rots) = mot.sample(frame.min(mot.frames.saturating_sub(1) as f32));
                // Moving: legs from the locomotion cycle, upper body keeps the gun hold
                if matches!(movement, Movement::Walk | Movement::Run | Movement::Back | Movement::StrafeLeft | Movement::StrafeRight) {
                    if let Some(hold) = player_motions.get(Movement::Idle) {
                        let (_, hold_rots) = hold.sample(0.0);
                        for b in &skel.bones {
                            if is_upper_body(&b.name) {
                                if let (Some(r), Some(h)) = (rots.get_mut(b.anim_index), hold_rots.get(b.anim_index)) { *r = *h; }
                            }
                        }
                    }
                }
                let pose = skel.pose([0.0, root[1], 0.0], &rots);
                let verts = skin.apply(&pose);
                glPushMatrix();
                glTranslatef(feet[0], feet[1], feet[2]);
                // The body faces where the player looks (model forward is +Z)
                glRotatef(yaw_deg, 0.0, 1.0, 0.0);
                glScalef(scale[0], scale[1], scale[2]);
                draw_character(&chars.model, &verts, &part_textures, face, textured);

                // The MP5K in the right hand, or a grenade until it is thrown
                if let (Some(Some(wd)), Some((hand, off))) = (hand, &weapon_attach) {
                    glPushMatrix();
                    glMultMatrixf(pose[*hand].matrix().as_ptr());
                    glMultMatrixf(off.matrix().as_ptr());
                    if grenade {
                        glTranslatef(-wd.center[0], -wd.center[1], -wd.center[2]);
                    }
                    draw_weapon(wd, textured, flash);
                    glPopMatrix();
                }
                glPopMatrix();
                glColor4f(1.0, 1.0, 1.0, 1.0);
                true
            };

            // The other players on the server, moving and handling their weapons as they are seen to
            if let Some(chars) = &sim.characters {
                glDisable(0x0DE1); // GL_TEXTURE_2D
                let others = view.as_ref().map_or(&[][..], |v| &v.players[..]);
                for &(slot, pos, look) in others.iter().filter(|(i, ..)| *i != me) {
                    let visible = shown(pos);
                    let p = &sim.players[slot];
                    let yaw = sim::dequantize(look.0);
                    let face = (!actors.is_empty()).then(|| face_textures[slot % actors.len()].0).flatten();
                    if p.respawn_at.is_none() {
                        let doing = Doing {
                            throwing: p.throw.map(|t| (game_time - t.start) / sim::THROW_SECONDS),
                            reloading: p.reload_until.map(|t| 1.0 - (t - game_time) / sim::RELOAD_SECONDS),
                            firing: p.gun_trigger && p.ammo > 0,
                        };
                        let frames = |m: Movement| player_motions.get(m).map_or(1, |m| m.frames.max(1)) as f32;
                        let (movement, frame) = remote_anim.update(slot, pos, yaw, step_dt, doing, frames, args.speed_scale.unwrap_or(1.0), args.anim_scale.unwrap_or(1.5));
                        let scale = actors.get(slot % actors.len().max(1)).map_or([1.0; 3], |a| a.scale);
                        let hand = if p.equipped_grenade { p.throw.is_none_or(|t| !t.thrown).then_some(&grenade_draw) } else { Some(&weapon_draw) };
                        let flash = !p.equipped_grenade && game_time < p.flash_until;
                        // Hidden, the animation still follows them
                        if !visible || draw_body(chars, movement, frame, pos, -yaw, scale, face, hand, p.equipped_grenade, flash) {
                            continue;
                        }
                    } else {
                        remote_anim.forget(slot);
                    }
                    if !visible {
                        continue;
                    }
                    // Without the player animations, or when dead: a character, lying where they
                    // fell until they come back
                    let mut health = combat::Health::new();
                    if let Some(until) = p.respawn_at {
                        health.state = combat::NpcState::Dead { motion: "thdu_1", start: until - sim::RESPAWN_SECONDS };
                    }
                    let ghost = Npc {
                        health,
                        pos,
                        floor: pos[1],
                        yaw: -yaw,
                        scale: [1.0; 3],
                        stagger: slot as f32 * 0.73,
                    };
                    let idle = game_time * 30.0 + ghost.stagger * 9.0;
                    let posed = chars.pose(&ghost, game_time, idle);
                    let verts: &[[f32; 3]] = if posed.is_empty() { &chars.model.vertices } else { &posed };
                    glPushMatrix();
                    glTranslatef(ghost.pos[0], ghost.pos[1], ghost.pos[2]);
                    glRotatef(ghost.yaw, 0.0, 1.0, 0.0);
                    draw_character(&chars.model, verts, &part_textures, face, textured);
                    glPopMatrix();
                }
                glColor4f(1.0, 1.0, 1.0, 1.0);
            }

            // The player's own body, shown in third person
            if let (true, true, Some(chars)) = (first_person, third_person, &sim.characters) {
                let clock = start_time.elapsed().as_secs_f32();
                let blinking = args.eyes_closed || (blink_enabled && clock % 4.0 < 0.15);
                let face = (!actors.is_empty()).then(|| face_textures[0]).and_then(|(main, blink)| {
                    if blinking { blink.or(main) } else { main }
                });
                let scale = actors.first().map_or([1.0; 3], |a| a.scale);
                let mine = &sim.players[me];
                let hand = if mine.equipped_grenade { mine.throw.is_none_or(|t| !t.thrown).then_some(&grenade_draw) } else { Some(&weapon_draw) };
                let flash = !mine.equipped_grenade && game_time < mine.flash_until;
                draw_body(chars, movement, player_frame, draw_feet, -camera.yaw, scale, face, hand, mine.equipped_grenade, flash);
            }

            // Grenades in the air
            if let Some(gd) = &grenade_draw {
                let flying: Vec<([f32; 3], f32)> = match &view {
                    Some(v) => v.grenades.iter().map(|&(_, pos, spin)| (pos, spin)).collect(),
                    None => sim.thrown.iter().map(|t| (t.grenade.pos, t.grenade.spin)).collect(),
                };
                for (pos, spin) in flying {
                    glPushMatrix();
                    glTranslatef(pos[0], pos[1], pos[2]);
                    glRotatef(spin, 1.0, 0.3, 0.0);
                    // Model space (forward -Y, up +Z) turned so its length runs the way it spins
                    glTranslatef(-gd.center[0], -gd.center[1], -gd.center[2]);
                    draw_weapon(gd, textured, false);
                    glPopMatrix();
                }
                glColor4f(1.0, 1.0, 1.0, 1.0);
            }

            // Bullet holes on the walls, then the debris thrown from them
            effects.update(step_dt);
            if textured && !decals.is_empty() {
                glEnable(0x0DE1); // GL_TEXTURE_2D
                glEnable(0x0BE2); // GL_BLEND
                glBlendFunc(0x0302, 0x0303); // SRC_ALPHA, ONE_MINUS_SRC_ALPHA
                glEnable(0x8037); // GL_POLYGON_OFFSET_FILL: keeps the hole from flickering against the wall
                glPolygonOffset(-2.0, -2.0);
                glDepthMask(0);
                glColor4f(1.0, 1.0, 1.0, 1.0);
                for d in &decals {
                    let Some(&id) = hole_tex.get(&(d.surface, d.variant)) else { continue };
                    glBindTexture(0x0DE1, id);
                    glBegin(0x0007); // GL_QUADS
                    // The top row of the picture is its top edge
                    for (c, uv) in d.corners().iter().zip([[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]) {
                        glTexCoord2f(uv[0], uv[1]);
                        glVertex3f(c[0], c[1], c[2]);
                    }
                    glEnd();
                }
                glDepthMask(1);
                glDisable(0x8037);
                glDisable(0x0BE2);
            }
            if !effects.particles.is_empty() || !effects.puffs.is_empty() {
                let right = listener_of(&camera).right;
                glEnable(0x0BE2); // GL_BLEND
                glDepthMask(0);
                // Soft clouds and the flash of a spark, as sprites facing the camera
                for puff in &effects.puffs {
                    let (tex, additive, strength) = match puff.kind {
                        impact::PuffKind::Dust => (cloud_tex, false, 0.55),
                        impact::PuffKind::Flash => (flash_tex, true, 1.0),
                        impact::PuffKind::Glare => (glare_tex, true, 1.0),
                        impact::PuffKind::Smoke => (smoke_tex, false, 0.6),
                        impact::PuffKind::Ring => (ring_tex, false, 0.9),
                    };
                    let Some(id) = tex else { continue };
                    glEnable(0x0DE1);
                    glBindTexture(0x0DE1, id);
                    glBlendFunc(0x0302, if additive { 1 } else { 0x0303 }); // SRC_ALPHA, ONE or ONE_MINUS_SRC_ALPHA
                    // A blast fades quickly at first, then lingers; the rest fade steadily
                    let fade = if puff.kind == impact::PuffKind::Smoke { puff.alpha().sqrt() * puff.alpha() } else { puff.alpha() };
                    glColor4f(puff.color[0], puff.color[1], puff.color[2], fade * strength);
                    let h = puff.current_size() / 2.0;
                    // Facing the camera, or lying in the surface for a shockwave ring
                    let (r, u) = match puff.axis {
                        Some(n) => {
                            let across = if n[1].abs() > 0.95 { [1.0, 0.0, 0.0] } else { unit3([n[2], 0.0, -n[0]]) };
                            (across, [n[1] * across[2] - n[2] * across[1], n[2] * across[0] - n[0] * across[2], n[0] * across[1] - n[1] * across[0]])
                        }
                        None => (right, [0.0, 1.0, 0.0]),
                    };
                    // Dust hugs the ground as a low bank and smoke sits on it as a ball, so
                    // neither is cut off by the floor where the flat sprite meets it
                    let (squash, lift) = match (puff.kind, puff.axis) {
                        (_, Some(_)) => (1.0, 0.0),
                        (impact::PuffKind::Dust, _) => (0.55, 0.55),
                        (impact::PuffKind::Smoke, _) => (1.0, 0.85),
                        _ => (1.0, 0.0),
                    };
                    let p = [puff.pos[0], puff.pos[1] + lift * h, puff.pos[2]];
                    glBegin(0x0007); // GL_QUADS
                    for (dx, dy, tu, tv) in [(-1.0, -1.0, 0.0, 1.0), (1.0, -1.0, 1.0, 1.0), (1.0, 1.0, 1.0, 0.0), (-1.0, 1.0, 0.0, 0.0)] {
                        glTexCoord2f(tu, tv);
                        let dy = dy * squash;
                        glVertex3f(p[0] + r[0] * dx * h + u[0] * dy * h, p[1] + r[1] * dx * h + u[1] * dy * h, p[2] + r[2] * dx * h + u[2] * dy * h);
                    }
                    glEnd();
                }
                // Splinters, then sparks that glow over everything, then chips
                glDisable(0x0DE1);
                for sparks in [false, true] {
                    glBlendFunc(0x0302, if sparks { 1 } else { 0x0303 });
                    glLineWidth(if sparks { 1.5 } else { 2.0 });
                    glBegin(0x0001); // GL_LINES
                    for pt in effects.particles.iter().filter(|p| (p.kind == impact::Kind::Spark) == sparks && p.kind != impact::Kind::Chip) {
                        let (a, b) = (pt.tail(), pt.pos);
                        glColor4f(pt.color[0], pt.color[1], pt.color[2], pt.alpha());
                        glVertex3f(a[0], a[1], a[2]);
                        glVertex3f(b[0], b[1], b[2]);
                    }
                    glEnd();
                }
                glBlendFunc(0x0302, 0x0303);
                glPointSize(2.5);
                glBegin(0x0000); // GL_POINTS
                for pt in effects.particles.iter().filter(|p| p.kind == impact::Kind::Chip) {
                    glColor4f(pt.color[0], pt.color[1], pt.color[2], pt.alpha());
                    glVertex3f(pt.pos[0], pt.pos[1], pt.pos[2]);
                }
                glEnd();
                glLineWidth(1.0);
                glDepthMask(1);
                glDisable(0x0BE2);
                glColor4f(1.0, 1.0, 1.0, 1.0);
            }

            // Glass: intact panes, then the pieces of broken ones falling away
            for f in fractures.iter_mut() {
                f.update(step_dt, &|x, z, y| sim.world.ground_at(x, z, y + 30.0));
            }
            fractures.retain(|f| !f.done());
            // A pane shows only with its room. Some maps glaze a window on the inside alone and
            // give the outside a solid one: from outside, the room and its glass are not drawn.
            let pane_shows = |pane: &glass::Pane| plan.is_empty()
                || map::object_room(&pane.name).is_none_or(|n| visible_rooms.contains(&n));
            for (pane, _) in sim.panes.iter().zip(&sim.broken_at).filter(|(p, b)| b.is_none() && pane_shows(p)) {
                draw_glass(&level.materials, pane.material, &map_num, &mut textures, textured, pane.opacity, pane.tris.iter().map(|t| (t.p, t.uv)));
            }
            for f in &fractures {
                draw_fracture(&level.materials, &map_num, &mut textures, textured, f);
            }

            // Bullet trail and impact
            if let Some((a, b, hit, until)) = tracer {
                if game_time < until {
                    glDisable(0x0DE1);
                    glColor4f(1.0, 0.9, 0.4, 1.0);
                    glBegin(0x0001); // GL_LINES
                    glVertex3f(a[0], a[1], a[2]);
                    glVertex3f(b[0], b[1], b[2]);
                    glEnd();
                    if hit { glColor4f(1.0, 0.1, 0.1, 1.0); } else { glColor4f(0.9, 0.9, 0.9, 1.0); }
                    let r = 6.0;
                    glBegin(0x0001);
                    for (dx, dy, dz) in [(r, 0.0, 0.0), (0.0, r, 0.0), (0.0, 0.0, r)] {
                        glVertex3f(b[0] - dx, b[1] - dy, b[2] - dz);
                        glVertex3f(b[0] + dx, b[1] + dy, b[2] + dz);
                    }
                    glEnd();
                    glColor4f(1.0, 1.0, 1.0, 1.0);
                }
            }

            // Render portals overlay when toggled
            if show_portals {
                glDisable(0x0DE1); // GL_TEXTURE_2D
                glColor3f(0.0, 1.0, 0.0);
                for portal in &level.portals.portals {
                    if portal.coordinates.len() >= 2 {
                        glBegin(0x0002); // GL_LINE_LOOP
                        for v in &portal.coordinates {
                            glVertex3f(v.x, v.y, v.z);
                        }
                        glEnd();
                    }
                }
                glColor3f(1.0, 1.0, 1.0); // restore default for next pass
            }

            // Brushes and some collision hints that augment level geometry
            if show_collisions {
                glDisable(0x0DE1); // GL_TEXTURE_2D
                glColor3f(1.0, 0.0, 0.0);
                for object in &level.geometries.objects {
                    glPointSize(7.5);
                    glBegin(0x0000); // GL_POINTS
                    for v in &object.collisions.vertices {
                        glVertex3f(v.x, v.y, v.z)
                    }
                    glEnd();
                }
                glColor3f(1.0, 1.0, 1.0); // restore default for next pass
            }
        }

        // The held weapon in front of the camera, over the scene. A grenade is small, so it is held
        // further out in proportion, drawn back by the wind-up, and gone once it has been thrown.
        let held = if sim.players[me].equipped_grenade { grenade_draw.as_ref().filter(|_| sim.players[me].throw.is_none_or(|t| !t.thrown)) } else { weapon_draw.as_ref() };
        if let (true, true, false, Some(wd)) = (args.viewmodel, first_person, third_person, held) {
            let l = wd.length;
            let [ox, oy, oz] = if sim.players[me].equipped_grenade {
                let held_for = sim.players[me].charge_from.map_or(0.0, |from| ((game_time - from) / grenade::CHARGE_SECONDS).clamp(0.0, 1.0));
                let swing = sim.players[me].throw.map_or(0.0, |t| t.charge.max(((game_time - t.start) / sim::THROW_SECONDS / sim::THROW_RELEASE).clamp(0.0, 1.0)));
                let wind = held_for.max(swing);
                [1.7 + 0.9 * wind, -1.2 - 0.6 * wind, 8.0 - 1.8 * wind]
            } else {
                args.viewmodel_offset.unwrap_or([0.45, -0.45, 1.5])
            };
            // Recoil pushes the gun back at the player; a reload dips it out of view
            let kick = ((sim.players[me].flash_until - game_time) / 0.05).clamp(0.0, 1.0) * 0.04 * l;
            let dip = sim.players[me].reload_until.map_or(0.0, |t| {
                let p = (1.0 - (t - game_time) / sim::RELOAD_SECONDS).clamp(0.0, 1.0);
                (p * std::f32::consts::PI).sin() * 0.8 * l
            });
            unsafe {
                // Its own depth range, so the gun never sinks into a wall
                glClear(0x00000100); // GL_DEPTH_BUFFER_BIT
                glMatrixMode(0x1701); // GL_PROJECTION
                glLoadIdentity();
                glu_perspective(45.0, aspect, l * 0.05, l * 20.0); // narrower than the world, as in most shooters
                glMatrixMode(0x1700); // GL_MODELVIEW
                glLoadIdentity();
                glTranslatef(ox * l, oy * l - dip, -oz * l + kick);
                // Angle the barrel slightly in toward the middle of the view
                glRotatef(20.0, 0.0, 1.0, 0.0);
                // Model space (forward -Y, up +Z) to view space (forward -Z, up +Y)
                glRotatef(180.0, 0.0, 1.0, 0.0);
                glRotatef(-90.0, 1.0, 0.0, 0.0);
                glTranslatef(-wd.center[0], -wd.center[1], -wd.center[2]);
                draw_weapon(wd, textured, game_time < sim.players[me].flash_until);
                glColor4f(1.0, 1.0, 1.0, 1.0);
            }
        }
        // println!("render_time:{:?}", render_time.elapsed());

        // Draw simple debug text overlay (upper-right)
        let render_ms = (render_time.elapsed().as_secs_f64() * 1000.0) as f32;
        let level_text = if !level.planning_levels.levels.is_empty() {
            format!("LEVEL: {}", planning_level_index)
        } else {
            String::from("LEVEL: -")
        };
        let fps_text = format!("FPS: {:.1}", fps);
        let render_text = format!("RENDER: {:.2} MS", render_ms);
        if first_person {
            let mut status = vec![
                format!("KILLS: {}", sim.players[me].kills),
                if sim.players[me].equipped_grenade { format!("GRENADES: {}", sim.players[me].grenades_left) }
                    else if sim.players[me].reload_until.is_some() { "RELOADING".to_string() } else { format!("AMMO: {}/{}", sim.players[me].ammo, sim.players[me].reserve) },
            ];
            if let Some(from) = sim.players[me].charge_from {
                status.insert(0, format!("POWER: {:.0}", ((game_time - from) / grenade::CHARGE_SECONDS).clamp(0.0, 1.0) * 100.0));
            }
            if let Some(hp) = sim.players[me].target_hp {
                status.insert(0, format!("TARGET: {hp:.0}"));
            }
            if net.is_some() {
                let p = &sim.players[me];
                status.insert(0, if p.respawn_at.is_some() { "DEAD".to_string() } else { format!("HEALTH: {:.0}", p.hp.ceil()) });
                status.insert(1, format!("DEATHS: {}", p.deaths));
            }
            let msg = if game_time < sim.players[me].message.1 { sim.players[me].message.0.as_str() } else { "" };
            let marker = tracer.is_some_and(|t| t.2 && game_time < t.3);
            let lines: Vec<(String, [f32; 3])> = feed.iter().map(|(_, text, color)| (text.clone(), *color)).collect();
            let dead = net.is_some() && sim.players[me].respawn_at.is_some();
            draw_hud(w, h, &status, msg, marker, veil, &lines, dead);
        }
        let rooms_text = if sim.culling {
            format!("ROOM: {} DRAWN: {}", current_rooms.join(" "), drawn_rooms.join(" "))
        } else {
            format!("ROOM: {} (CULLING OFF)", current_rooms.join(" "))
        };
        let demo_text = match (&recorder, &playback) {
            (Some(r), _) => format!("REC {:.0} S", r.ticks() as f32 * sim::TICK),
            (_, Some(p)) => {
                let (done, all) = p.progress();
                format!("DEMO {:.0}/{:.0} S", done as f32 * sim::TICK, all as f32 * sim::TICK)
            }
            _ => String::new(),
        };
        let net_text = net.as_ref().map_or(String::new(), |(c, _)| format!("NET: {:?} {} PLAYERS", c.status, sim.players.iter().filter(|p| p.active).count()).to_uppercase());
        let mut debug_lines: Vec<&str> = vec![&level_text, &fps_text, &render_text];
        if !net_text.is_empty() {
            debug_lines.push(&net_text);
        }
        if room_debug {
            debug_lines.push(&rooms_text);
        }
        if !demo_text.is_empty() {
            debug_lines.push(&demo_text);
        }
        draw_debug_text(w, h, &debug_lines);

        // F2: a screenshot and a text log of where the player is and what they see
        if (dump_requested && (!args.dump || frames_drawn >= 2)) || clip.is_some() {
            let one_dump = std::mem::take(&mut dump_requested);
            let stem = clip.as_mut().map(|(_, dir, n)| {
                *n += 1;
                format!("{dir}/{:04}", *n)
            });
            let (fw, fh) = window.get_framebuffer_size();
            let rgb = read_framebuffer(fw, fh);
            let eye = [camera.position[0], camera.position[1], -camera.position[2]];
            let f = camera.get_forward();
            let dir = [f[0], f[1], -f[2]];
            let hit = sim.world.raycast_hit(eye, dir, 20000.0).map(|hit| {
                let point = [eye[0] + dir[0] * hit.dist, eye[1] + dir[1] * hit.dist, eye[2] + dir[2] * hit.dist];
                let material = hit.material.and_then(|m| level.materials.materials.get(m)).map_or("?", |m| m.filename.as_str());
                (hit, point, material)
            });
            let feet = [eye[0], eye[1] - if first_person { eye_height } else { 0.0 }, eye[2]];
            let nearby_doors: Vec<String> = sim.doors.iter().zip(&sim.door_states)
                .filter_map(|(d, s)| {
                    let c = d.center(s.open);
                    let dist = ((c[0] - eye[0]).powi(2) + (c[1] - eye[1]).powi(2) + (c[2] - eye[2]).powi(2)).sqrt();
                    (dist <= 500.0).then(|| format!("{} {:.0} units away, {:.0}% open{}", d.name, dist, s.open * 100.0,
                        d.portal.map_or(String::new(), |p| format!(", portal {}", level.portals.portals[p].name))))
                })
                .collect();
            let report = DebugReport {
                map_path, map_num: &map_num, level: &level, eye, feet, yaw: camera.yaw, pitch: camera.pitch, dir,
                hit, first_person, third_person, textured, room_debug, show_portals, show_collisions,
                fps, render_ms, culling: sim.culling, portal_paths: args.portal_paths, room: current_rooms.first().copied(), drawn: &drawn_rooms, collision: &sim.players[me].collision_rooms, player_rooms: &sim.players[me].player_rooms, render_rooms: &sim.players[me].render_rooms, doors: &nearby_doors,
            };
            let clipping = stem.is_some();
            if let Err(e) = std::fs::create_dir_all(stem.as_deref().and_then(|s| Path::new(s).parent()).unwrap_or(Path::new("debug"))) {
                eprintln!("debug dump failed: {e:#}");
            }
            match write_debug_dump(&report, &rgb, fw as u32, fh as u32, stem) {
                Ok(stem) if !clipping => println!("wrote {stem}.png and {stem}.txt"),
                Ok(_) => {}
                Err(e) => eprintln!("debug dump failed: {e:#}"),
            }
            if clip.as_ref().is_some_and(|(start, ..)| start.elapsed().as_secs_f32() >= CLIP_SECONDS) {
                if let Some((_, dir, n)) = clip.take() {
                    println!("recorded {n} frames in {dir}");
                }
            }
            if args.dump && one_dump {
                break;
            }
        }

        // Screenshot mode: capture after a couple of frames so textures are
        // uploaded and the context has settled, then exit.
        frames_drawn += 1;
        // Timing for benchmarks (--walk N --hold X --max-fps 10000), after a warm-up
        if frames_drawn > BENCH_WARMUP {
            bench = (bench.0 + 1, bench.1 + render_ms as f64, bench.2 + raw_dt as f64 * 1000.0);
        }
        // A replay ends when the recording does
        let replay_over = playback.as_ref().is_some_and(|p| p.finished());
        if replay_over && args.screenshot.is_none() {
            window.set_should_close(true);
        }
        if let Some(path) = &args.screenshot {
            let done = if playback.is_some() { replay_over } else { frames_drawn >= args.walk.map_or(3, |n| n.max(3)) };
            if done {
                if bench.0 > 0 {
                    println!("timing over {} frames: render {:.3} ms, whole frame {:.3} ms", bench.0, bench.1 / bench.0 as f64, bench.2 / bench.0 as f64);
                }
                let (fw, fh) = window.get_framebuffer_size();
                let rgb = read_framebuffer(fw, fh);
                match rogue_reborn::png::write_png(path, fw as u32, fh as u32, &rgb) {
                    Ok(()) => println!("wrote {} ({fw}x{fh})", path.display()),
                    Err(e) => eprintln!("failed to write screenshot: {e}"),
                }
                break;
            }
        }

        window.swap_buffers();
        if let Some(max) = args.max_fps {
            // Sleep most of the remaining frame time, then spin for accuracy
            let target = std::time::Duration::from_secs_f32(1.0 / max);
            loop {
                let left = target.saturating_sub(last_step.elapsed());
                if left.is_zero() {
                    break;
                }
                if left > std::time::Duration::from_millis(2) {
                    std::thread::sleep(left - std::time::Duration::from_millis(1));
                } else {
                    std::hint::spin_loop();
                }
            }
        }
    }

    if let Some((client, link)) = net.as_mut() {
        client.leave(link);
    }
    if let (Some(rec), Some(path)) = (recorder, &args.record) {
        let rec = rec.finish(&sim);
        match rec.write(path) {
            Ok(()) => println!("recorded {} ticks ({:.1} s) to {}", rec.inputs.len(), rec.seconds(), path.display()),
            Err(e) => eprintln!("{e:#}"),
        }
    }
    if let Some(pb) = &playback {
        let (done, all) = pb.progress();
        match pb.desync() {
            None => println!("played {done} of {all} ticks: the game matched the recording"),
            Some(d) => {
                println!("played {done} of {all} ticks: {d}");
                std::process::exit(1);
            }
        }
    }

    if reload {
        // Let go of the window and the sound device first, so the new one has them to itself
        drop(window);
        drop(sound);
        let exe = std::env::current_exe().expect("cannot find the viewer to restart");
        match std::process::Command::new(exe).args(std::env::args_os().skip(1)).spawn() {
            Ok(_) => {}
            Err(e) => eprintln!("could not reload: {e}"),
        }
    }
}

