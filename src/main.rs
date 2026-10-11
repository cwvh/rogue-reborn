mod args;
mod camera;
mod debug;
mod draw;
mod hud;
mod levels;
mod opengl;
mod player;
mod textures;
mod util;

use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use glfw::{Action, Context, Key, WindowEvent, CursorMode};
use rogue_reborn::{map, rsb, actor, combat, crp, demo, grenade, motion, skeleton, sound, weapon, ambience, glass, door};
use rogue_reborn::collision::{CollisionWorld, Params, Player};
use rogue_reborn::impact::{self, Surface};
use crate::opengl::*;
use crate::args::parse_args;
use crate::camera::{Camera, listener_of, apply_camera};
use crate::util::{next_rng, dist3, mul3, unit3};
use crate::player::{Movement, PlayerMotions, RELOAD_SECONDS, THROW_SECONDS, THROW_RELEASE, Throw, is_upper_body, ground_speed, movement_from_keys};
use crate::draw::{WeaponDraw, draw_weapon, draw_character, draw_glass, draw_fracture, build_mesh, Which, draw_plan, room_color};
use crate::hud::{draw_hud, draw_debug_text};
use crate::debug::{DebugReport, write_debug_dump};
use crate::levels::{find_zone_spawn, find_mission_spawns, pick_index, list_levels, find_level, extract_map_num};
use crate::textures::{load_character_texture, ensure_texture};

/// Soot marks a blast can leave
const SCORCH_VARIANTS: usize = 4;

/// Frames to skip before timing, while textures load
const BENCH_WARMUP: u32 = 30;

fn main() {
    // Load the map
    // let map_path = Path::new("data/map/m00/demolitions_area.MAP");
    // let map_path = Path::new("data/map/rm11/rm11.map");
    // let map_path = Path::new("data/map/rm02/rm02.map");
    let args = parse_args();
    if args.list_levels {
        for (dir, name, _) in list_levels() {
            println!("{name:24} data/map/{dir}");
        }
        return;
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
    let level = if args.demo { demo::map() } else { map::read(map_path).expect("Failed to load map") };

    // Build a flattened mesh from the MAP geometry
    let mesh = build_mesh(&level);
    // Collision world built from the render triangles (MAP space)
    let mut world = CollisionWorld::from_map(&level, Params::default());
    // Glass panes, which break when shot; their shards fall for a moment afterwards
    let panes = glass::panes(&level);
    let mut broken_at: Vec<Option<f32>> = vec![None; panes.len()];
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
    // Character models standing at every other mission spawn point
    let model_path = args.model.clone()
        .unwrap_or_else(|| PathBuf::from("data/character/team_hrt/blue_M_a.CRP"));
    let character = {
        match crp::read(&model_path) {
            Ok(c) => {
                println!("loaded {} ({} vertices, {} triangles)", c.name, c.vertices.len(),
                    c.parts.iter().map(|p| p.triangles.len()).sum::<usize>());
                Some(c)
            }
            Err(e) => { eprintln!("could not load {}: {e:#}", model_path.display()); None }
        }
    };
    // Skeleton and skinning for posed models
    let find_file = |dir: &Path, stem: &str, ext: &str| -> Option<PathBuf> {
        std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
            p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
                && p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == stem.to_lowercase())
        })
    };
    let rig = (|| {
        let c = character.as_ref()?;
        let dir = model_path.parent()?;
        let stem = model_path.file_stem()?.to_string_lossy().to_string();
        let skel = skeleton::read(&find_file(dir, &stem, "skl").or_else(|| { eprintln!("no skeleton beside {}", model_path.display()); None })?).ok()?;
        let skin = c.skin(&skel).or_else(|| { eprintln!("mesh does not match skeleton"); None })?;
        Some((skel, skin))
    })();
    let load_motion = |name: &str| -> Option<motion::Motion> {
        let m = motion::read(&find_file(Path::new("data/motion"), name, "dam").or_else(|| { eprintln!("no motion named {name}"); None })?).ok();
        m
    };
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

    // Motions other characters play: idle (or --anim), injury reactions and deaths
    let idle_name = args.anim.clone().unwrap_or_else(|| "up_none_cyc".to_string());
    let mut npc_motions: HashMap<String, motion::Motion> = HashMap::new();
    for name in [idle_name.as_str(), "thiu_ab", "thiu_cb", "thiu_cf", "thiu_cl", "thiu_cr", "thiu_hb", "thiu_hf",
        "thiu_la", "thiu_ll", "thiu_ra", "thiu_rl", "thdu_1", "thdu_2", "thdu_3", "thdu_4", "thdh_1", "thdh_2"] {
        if let Some(m) = load_motion(name) {
            npc_motions.insert(name.to_string(), m);
        }
    }

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

    // Where the weapon sits relative to the right hand. Calibrated so that in the
    // weapon-ready idle pose the gun points the way the body faces (the model's
    // forward is -Y and up is +Z, so it turns -90 degrees about X to face +Z with
    // +Y up); from then on it just follows the hand. `--weapon-offset` shifts the
    // grip, in body space at that pose.
    let weapon_attach: Option<(usize, skeleton::Transform)> = (|| {
        let (skel, _) = rig.as_ref()?;
        let hand = skel.bones.iter().position(|b| b.name == "RightHand")?;
        let (root, rots) = player_motions.get(Movement::Idle)?.sample(0.0);
        let q = skel.pose([0.0, root[1], 0.0], &rots)[hand].rot;
        let conj = [-q[0], -q[1], -q[2], q[3]];
        let h = std::f32::consts::FRAC_1_SQRT_2;
        Some((hand, skeleton::Transform {
            rot: motion::quat_mul(conj, [-h, 0.0, 0.0, h]),
            pos: motion::quat_rotate(conj, args.weapon_offset.unwrap_or([0.0, 0.0, 0.0])),
        }))
    })();

    // One GL texture per part, from the character's own directory
    let part_textures: Vec<Option<u32>> = character.as_ref().map_or_else(Vec::new, |c| {
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

    struct ModelSpot {
        health: combat::Health,
        pos: [f32; 3],
        floor: f32,
        yaw: f32,
        actor: Option<usize>,
        blink_offset: f32,
    }
    let mut model_spots: Vec<ModelSpot> = Vec::new();
    if let (false, Some(c)) = (args.no_models, &character) {
        let feet_to_origin = -c.bounds().0[1];
        for (n, (i, p)) in mission_spawns.iter().enumerate().filter(|(i, _)| own_spawn != Some(*i)).enumerate() {
            let floor = world.ground_at(p[0], p[2], p[1] + 100.0).unwrap_or(p[1]);
            let actor = (!actors.is_empty()).then(|| n % actors.len());
            let scale_y = actor.map_or(1.0, |a| actors[a].scale[1]);
            model_spots.push(ModelSpot {
                health: combat::Health::new(),
                pos: [p[0], floor + feet_to_origin * scale_y, p[2]],
                floor,
                // Spread facing directions deterministically by spawn index
                yaw: args.model_yaw.unwrap_or_else(|| match mission_spawns.first() {
                    // In the demo room everyone faces the player
                    Some(player) if args.demo => (player[0] - p[0]).atan2(player[2] - p[2]).to_degrees(),
                    _ => i as f32 * 137.5,
                }),
                actor,
                blink_offset: i as f32 * 0.73,
            });
        }
    }

    // Pose a character for its state: idling, reacting to a hit or dead
    let npc_verts = |npc: &ModelSpot, now: f32| -> Vec<[f32; 3]> {
        let (Some(c), Some((skel, skin))) = (&character, &rig) else { return Vec::new() };
        let (name, frame, looping): (&str, f32, bool) = match npc.health.state {
            combat::NpcState::Idle => (
                idle_name.as_str(),
                args.anim_frame.unwrap_or_else(|| now * args.anim_fps.unwrap_or(30.0) + npc.blink_offset * 9.0),
                true,
            ),
            combat::NpcState::Reacting { motion, start } => (motion, (now - start) * 30.0, false),
            combat::NpcState::Dead { motion, start } => (motion, (now - start) * 30.0, false),
        };
        let Some(mot) = npc_motions.get(name).or_else(|| npc_motions.get(idle_name.as_str())) else {
            return c.vertices.clone();
        };
        let frames = mot.frames.max(1) as f32;
        let f = if looping { frame % frames } else { frame.min(frames - 1.0) };
        let (root, rots) = mot.sample(f);
        // Hit and death motions carry the body along with them
        let k0 = mot.keys[0].root;
        let (dx, dz) = if looping { (0.0, 0.0) } else { (root[0] - k0[0], root[2] - k0[2]) };
        skin.apply(&skel.pose([dx, root[1], dz], &rots))
    };
    let placement = |npc: &ModelSpot| combat::Placement {
        pos: [npc.pos[0], if rig.is_some() { npc.floor } else { npc.pos[1] }, npc.pos[2]],
        yaw_deg: npc.yaw,
        scale: npc.actor.map_or([1.0; 3], |a| actors[a].scale),
    };

    if let Some(p) = args.pos { camera.position = p; }
    if let Some(y) = args.yaw { camera.yaw = y; }
    if let Some(p) = args.pitch { camera.pitch = p.clamp(-89.0, 89.0); }

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
    // F3 closes the window and starts the viewer again with the same options
    let mut reload = false;
    // Draw only the rooms the portals show (V toggles); drawn_rooms is last frame's set
    let mut culling = !args.no_cull;
    let visibility = rogue_reborn::visibility::Visibility::new(&level);
    // Which objects are glass panes (drawn in place), and every object's index
    let pane_objects: Vec<bool> = level.geometries.objects.iter().map(|o| panes.iter().any(|g| g.name == o.name)).collect();
    let all_objects: Vec<usize> = (0..level.geometries.objects.len()).collect();
    // Doors: each leaf is drawn and collided with where the door is. Space or Backspace
    // opens and closes the one being looked at.
    let doors: Vec<door::Door> = world.doors().cloned().collect();
    let mut door_states = vec![door::DoorState::default(); doors.len()];
    let door_leaf: Vec<bool> = (0..level.geometries.objects.len()).map(|i| doors.iter().any(|d| d.model_index == i)).collect();
    let mut doors_by_room: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, d) in doors.iter().enumerate() {
        for &r in &d.rooms {
            doors_by_room.entry(r).or_default().push(i);
        }
    }
    // Whether each portal is shut by a closed door
    let mut blocked_portals = vec![false; level.portals.portals.len()];
    let all_doors: Vec<usize> = (0..doors.len()).collect();
    let mut use_pressed = false;
    if let Some(open) = args.doors_open {
        for (i, s) in door_states.iter_mut().enumerate() {
            s.open = open.clamp(0.0, 1.0);
            s.opening = open > 0.5;
            world.set_door_open(i, s.open);
        }
    }
    let mut current_room: Option<usize> = None;
    let mut collision_rooms: Option<Vec<rogue_reborn::collision::SolidRoom>> = None;
    let mut last_render_rooms: Vec<u32> = Vec::new();
    let mut drawn_rooms: Vec<String> = Vec::new();
    let mut blink_enabled = !args.no_blink;
    // Combat state; times are in game seconds
    let mut game_time = 0.0f32;
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
    let mut trigger = false;
    // Right mouse button held: walk instead of the default run
    let mut walk_held = false;
    let (mag_size, mut reserve) = weapon_def.as_ref().map_or((30, 900), |w| (w.magazine, w.reserve));
    let mut ammo = mag_size;
    let mut reload_until: Option<f32> = None;
    let mut next_shot = 0.0f32;
    let mut kills = 0u32;
    let mut message = (String::new(), 0.0f32);
    let mut target_hp: Option<f32> = None;
    let mut flash_until = 0.0f32;
    // Grenades: key 3 takes one out, hold the trigger to wind up and let go to throw
    let mut equipped_grenade = false;
    let mut grenades_left = 3u32;
    let mut charge_from: Option<f32> = None;
    let mut throw: Option<Throw> = None;
    let mut thrown: Vec<grenade::Grenade> = Vec::new();
    // A blast leaves the player shaken (the view), dazzled (a white veil) and deafened (muffled sound)
    let (mut shake, mut veil, mut deaf) = (0.0f32, 0.0f32, 0.0f32);
    let master_volume = args.volume.unwrap_or(1.0).clamp(0.0, 1.0);
    // Last shot: start, end, whether it struck a character, and when it fades
    let mut tracer: Option<([f32; 3], [f32; 3], bool, f32)> = None;
    let mut rng = args.seed.unwrap_or(0x5eed) | 1;
    let mut third_person = args.third_person;
    let mut movement = Movement::Idle;
    let mut player_frame = 0.0f32;
    let start_time = Instant::now();
    let eye_height = world.params().eye_height;
    // Player body in MAP space; the camera is at the eyes with Z negated
    let mut player = Player::new([
        camera.position[0],
        camera.position[1] - eye_height,
        -camera.position[2],
    ]);
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
                        (Key::F3, Action::Press) => {
                            reload = true;
                            window.set_should_close(true);
                        }
                        (Key::Space, Action::Press) | (Key::Backspace, Action::Press) => {
                            use_pressed = true;
                        }
                        (Key::V, Action::Press) => {
                            culling = !culling;
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
                            // Back to the gun
                            if throw.is_none() {
                                equipped_grenade = false;
                                charge_from = None;
                            }
                        }
                        (Key::Num3, Action::Press) => {
                            if first_person && throw.is_none() && reload_until.is_none() {
                                equipped_grenade = true;
                                if grenades_left == 0 {
                                    message = ("NO GRENADES".to_string(), game_time + 1.0);
                                }
                            }
                        }
                        (Key::R, Action::Press) => {
                            // Reload: takes time, then fills the magazine from the reserve
                            if first_person && !equipped_grenade && reload_until.is_none() && ammo < mag_size && reserve > 0 {
                                reload_until = Some(game_time + RELOAD_SECONDS);
                            }
                        }
                        (Key::F1, Action::Press) => {
                            first_person = !first_person;
                            if first_person {
                                // Drop in from wherever the camera is
                                player = Player::new([
                                    camera.position[0],
                                    camera.position[1] - eye_height,
                                    -camera.position[2],
                                ]);
                            }
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
                    // A scripted run (--walk) is not steered by a real mouse
                    if let (Some((last_x, last_y)), None) = (last_cursor_pos, args.walk) {
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
                equipped_grenade = true;
                trigger = frames_drawn < 30;
            }
            // U presses the use key (Space) on frame 5, to open or close the door in front
            if held.contains('U') && frames_drawn == 5 {
                use_pressed = true;
            }
            // L starts a reload on the first frame (for testing the animation)
            if held.contains('L') && frames_drawn == 0 {
                ammo = 10;
                reload_until = Some(game_time + RELOAD_SECONDS);
            }
            walk_held = frames_drawn < n && held.contains('R');
            for (ch, key) in [('W', Key::W), ('A', Key::A), ('S', Key::S), ('D', Key::D)] {
                if frames_drawn < n && held.contains(ch) { keys.insert(key); } else { keys.remove(&key); }
            }
            if frames_drawn % 10 == 0 {
                println!("frame {frames_drawn:4} feet=({:.1}, {:.1}, {:.1}) on_ground={} movement={movement:?} anim_frame={player_frame:.1}",
                    player.pos[0], player.pos[1], player.pos[2], player.on_ground);
            }
        }

        // The trigger fires the gun, unless a grenade is out
        let gun_trigger = trigger && !equipped_grenade;

        // WASD movement (free-fly: hold LeftShift to move faster)
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

        // Time since the previous step, measured here so it covers render and vsync
        let raw_dt = last_step.elapsed().as_secs_f32();
        let real_dt = raw_dt.min(0.05);
        last_step = Instant::now();
        let step_dt = if args.walk.is_some() { 1.0 / 60.0 } else { real_dt };
        game_time += step_dt;

        // Space or Backspace opens or closes the door the player is looking at
        if std::mem::take(&mut use_pressed) && first_person {
            let eye = [camera.position[0], camera.position[1], -camera.position[2]];
            let f = camera.get_forward();
            let look = [f[0], f[1], -f[2]];
            let mut best: Option<(usize, f32)> = None;
            for (i, d) in doors.iter().enumerate() {
                // Aim at the doorway, or at the leaf wherever it has gone (a door swung open
                // to the side is still the one in front of you)
                for c in [d.center(0.0), d.center(door_states[i].open)] {
                    let to = [c[0] - eye[0], c[1] - eye[1], c[2] - eye[2]];
                    let dist = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
                    if dist > DOOR_REACH || dist < 1.0 {
                        continue;
                    }
                    // Within 60 degrees of where they are looking
                    let facing = (to[0] * look[0] + to[1] * look[1] + to[2] * look[2]) / dist;
                    if facing < 0.5 {
                        continue;
                    }
                    let score = dist * (2.0 - facing);
                    if best.is_none_or(|(_, s)| score < s) {
                        best = Some((i, score));
                    }
                }
            }
            match best {
                Some((i, _)) => {
                    door_states[i].opening = !door_states[i].opening;
                    let sound_name = if door_states[i].opening { &doors[i].open_sound } else { &doors[i].close_sound };
                    if let Some(snd) = sound.as_mut() {
                        if !sound_name.eq_ignore_ascii_case("none") {
                            snd.play_at(sound_name, doors[i].center(door_states[i].open), listener_of(&camera), 1.0);
                        }
                    }
                    message = (format!("DOOR {}", if door_states[i].opening { "OPENING" } else { "CLOSING" }), game_time + 1.2);
                    println!("t={game_time:.2} door {} {}", doors[i].name, if door_states[i].opening { "opening" } else { "closing" });
                }
                None => message = ("NO DOOR IN REACH".to_string(), game_time + 1.0),
            }
        }
        // Doors move, and the collision and the portals follow
        for (i, (d, s)) in doors.iter().zip(door_states.iter_mut()).enumerate() {
            if s.advance(d, step_dt) {
                world.set_door_open(i, s.open);
            }
            if let Some(p) = d.portal {
                blocked_portals[p] = s.closed();
            }
        }
        // Finish a reload
        if reload_until.is_some_and(|t| game_time >= t) {
            let take = (mag_size - ammo).min(reserve);
            ammo += take;
            reserve -= take;
            reload_until = None;
        }
        // Injury reactions end and the character goes back to idling
        for npc in model_spots.iter_mut() {
            if let combat::NpcState::Reacting { motion, start } = npc.health.state {
                let frames = npc_motions.get(motion).map_or(1, |m| m.frames) as f32;
                if (game_time - start) * 30.0 >= frames {
                    npc.health.state = combat::NpcState::Idle;
                }
            }
        }

        if first_person {
            // Speeds are map units per second, scaled by dt so the walk speed
            // does not depend on the frame rate
            let dt = step_dt;
            // Walking speeds come from the animations so the feet do not slide;
            // free-fly keeps its fast debug speeds
            let (w, a, s_, d, run) = (
                keys.contains(&Key::W), keys.contains(&Key::A), keys.contains(&Key::S),
                keys.contains(&Key::D), !walk_held,
            );
            let scale = args.speed_scale.unwrap_or(1.0);
            let fwd_speed = scale * if w == s_ { 0.0 }
                else if w { ground_speed(if run { Movement::Run } else { Movement::Walk }) }
                else { -ground_speed(Movement::Back) };
            let side_speed = scale * if a == d { 0.0 }
                else if d { ground_speed(Movement::StrafeRight) } else { -ground_speed(Movement::StrafeLeft) };
            let mut vel = [
                fwd_flat[0] * fwd_speed + right[0] * side_speed,
                fwd_flat[2] * fwd_speed + right[2] * side_speed,
            ];
            // Moving diagonally is no faster than the faster of the two
            let (len, cap) = ((vel[0] * vel[0] + vel[1] * vel[1]).sqrt(), fwd_speed.abs().max(side_speed.abs()));
            if len > cap && len > 0.0 {
                vel = [vel[0] / len * cap, vel[1] / len * cap];
            }

            let before = player.pos;
            world.step(&mut player, [vel[0] * dt, -vel[1] * dt], dt);

            // Choose the animation from what the player is trying to do, and
            // play it at the pace the player actually moved (a wall stops it)
            let wanted = movement_from_keys(
                keys.contains(&Key::W), keys.contains(&Key::S),
                keys.contains(&Key::A), keys.contains(&Key::D),
                !walk_held,
            );
            let moved = ((player.pos[0] - before[0]).powi(2) + (player.pos[2] - before[2]).powi(2)).sqrt() / dt.max(1e-4);
            // A wall stops the walking animation; standing still can fire or reload
            let walking = wanted != Movement::Idle && moved >= 5.0;
            let now = if walking {
                wanted
            } else if throw.is_some() {
                Movement::Throw
            } else if reload_until.is_some() {
                Movement::Reload
            } else if gun_trigger && ammo > 0 {
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
            player_frame = if let (Movement::Reload, Some(t)) = (movement, reload_until) {
                // Spread the reload animation across the whole reload time
                (1.0 - (t - game_time) / RELOAD_SECONDS).clamp(0.0, 1.0) * (frames - 1.0)
            } else if let (Movement::Throw, Some(th)) = (movement, throw) {
                ((game_time - th.start) / THROW_SECONDS).clamp(0.0, 1.0) * (frames - 1.0)
            } else {
                (player_frame + rate * 30.0 * dt) % frames
            };
            // A footstep at the start of each half of a walk or run cycle
            let phase = (player_frame / (frames / 2.0).max(1.0)) as i32;
            if phase != step_phase {
                step_phase = phase;
                if walking && player.on_ground {
                    if let Some(snd) = sound.as_mut() {
                        snd.play(if movement == Movement::Run { "F_HRCON1.WAV" } else { "F_HWCON1.WAV" }, 1.0);
                    }
                }
            }
            camera.position = [
                player.pos[0],
                player.pos[1] + eye_height,
                -player.pos[2],
            ];

            // A grenade: hold the trigger to wind up, let go to throw. The swing plays for a
            // moment (in third person you see it), and the grenade leaves the hand part of the way in.
            if equipped_grenade {
                let mut finished = false;
                match throw.as_mut() {
                    None => {
                        if trigger && grenades_left > 0 {
                            charge_from.get_or_insert(game_time);
                        } else if let Some(from) = charge_from.take() {
                            let charge = ((game_time - from) / grenade::CHARGE_SECONDS).clamp(0.0, 1.0);
                            throw = Some(Throw { start: game_time, charge, thrown: false });
                        } else if trigger && grenades_left == 0 && game_time >= message.1 {
                            message = ("NO GRENADES".to_string(), game_time + 0.8);
                        }
                    }
                    Some(th) => {
                        let progress = (game_time - th.start) / THROW_SECONDS;
                        if !th.thrown && progress >= THROW_RELEASE {
                            th.thrown = true;
                            grenades_left -= 1;
                            let eye = [player.pos[0], player.pos[1] + eye_height, player.pos[2]];
                            let f = camera.get_forward();
                            // Thrown a little upward, as a lob
                            let aim = unit3([f[0], f[1] + 0.14, -f[2]]);
                            let side = listener_of(&camera).right;
                            let want = [eye[0] + aim[0] * 28.0 + side[0] * 10.0, eye[1] + aim[1] * 28.0 - 12.0, eye[2] + aim[2] * 28.0 + side[2] * 10.0];
                            // Not through a wall that is right in front of the player
                            let to = unit3([want[0] - eye[0], want[1] - eye[1], want[2] - eye[2]]);
                            let reach = dist3(eye, want);
                            let from = match world.raycast(eye, to, reach + grenade::RADIUS) {
                                Some(d) => { let d = (d - grenade::RADIUS - 1.0).max(0.0); [eye[0] + to[0] * d, eye[1] + to[1] * d, eye[2] + to[2] * d] }
                                None => want,
                            };
                            thrown.push(grenade::Grenade::thrown(from, aim, th.charge, [0.0; 3]));
                            println!("t={game_time:.2} threw a grenade at {:.0}% power", th.charge * 100.0);
                        }
                        finished = progress >= 1.0;
                    }
                }
                if finished {
                    throw = None;
                    if grenades_left == 0 {
                        equipped_grenade = false;
                        message = ("OUT OF GRENADES".to_string(), game_time + 1.2);
                    }
                }
            }

            // Shooting: a ray from the eye along the view, stopped by walls
            if gun_trigger && reload_until.is_none() && ammo == 0 && reserve > 0 {
                reload_until = Some(game_time + RELOAD_SECONDS);
            }
            if gun_trigger && reload_until.is_none() && ammo > 0 && game_time >= next_shot {
                ammo -= 1;
                next_shot = game_time + 60.0 / 800.0; // the MP5K fires about 800 rounds a minute
                flash_until = game_time + 0.05;

                let eye = [player.pos[0], player.pos[1] + eye_height, player.pos[2]];
                let f = camera.get_forward();
                let dir = [f[0], f[1], -f[2]]; // camera space to MAP space
                let mut wall = world.raycast(eye, dir, 20000.0);

                let mut best: Option<(usize, combat::Hit)> = None;
                if let (Some(c), Some((skel, skin))) = (&character, &rig) {
                    for (i, npc) in model_spots.iter().enumerate() {
                        if npc.health.is_dead() {
                            continue;
                        }
                        let verts = npc_verts(npc, game_time);
                        if let Some(h) = combat::raycast_character(c, skel, skin, &verts, &placement(npc), eye, dir) {
                            if best.map_or(true, |(_, b)| h.distance < b.distance) {
                                best = Some((i, h));
                            }
                        }
                    }
                }

                // Glass in the way breaks and lets the bullet through
                loop {
                    let glass_hit = panes.iter().enumerate()
                        .filter(|(i, _)| broken_at[*i].is_none())
                        .filter_map(|(i, p)| p.raycast(eye, dir).map(|d| (i, d)))
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    let Some((gi, gd)) = glass_hit else { break };
                    // Something solid, or a character, is in front of the glass
                    if wall.is_some_and(|w| w < gd - 0.5) || best.as_ref().is_some_and(|(_, h)| h.distance < gd) {
                        break;
                    }
                    broken_at[gi] = Some(game_time);
                    world.switch_off(&panes[gi].name);
                    rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
                    let at = [eye[0] + dir[0] * gd, eye[1] + dir[1] * gd, eye[2] + dir[2] * gd];
                    fractures.push(glass::shatter(&panes[gi], at, dir, rng));
                    println!("t={game_time:.2} shot broke glass {}", panes[gi].name);
                    if let Some(snd) = sound.as_mut() {
                        snd.play_at("C_GBRKMD.WAV", panes[gi].center(), listener_of(&camera), 1.0);
                    }
                    wall = world.raycast(eye, dir, 20000.0);
                }

                let start = [eye[0] + dir[0] * 30.0, eye[1] + dir[1] * 30.0 - 8.0, eye[2] + dir[2] * 30.0];
                match best {
                    Some((i, hit)) if wall.map_or(true, |w| hit.distance <= w) => {
                        rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
                        let result = model_spots[i].health.apply_hit(&hit, game_time, (rng >> 33) as u32);
                        target_hp = Some(model_spots[i].health.hp);
                        println!("t={game_time:.2} shot hit character {i}: {:?} from the {:?}, {:?}, hp now {}", hit.part, hit.facing, result, model_spots[i].health.hp);
                        message = (match result {
                            combat::HitResult::Killed => {
                                kills += 1;
                                if hit.part == combat::BodyPart::Head { "HEAD SHOT KILL".to_string() } else { "KILLED".to_string() }
                            }
                            _ => format!("HIT {:?}", hit.part).to_uppercase(),
                        }, game_time + 1.2);
                        if let Some(snd) = sound.as_mut() {
                            let listener = listener_of(&camera);
                            snd.play_at("C_BHTHT.WAV", hit.point, listener, 1.0);
                            // Voices: a grunt when hurt, a cry when killed
                            let (name, n) = if matches!(result, combat::HitResult::Killed) { ("re_ded", 2) } else { ("re_wnd", 3) };
                            let pick = 1 + (rng >> 40) as u32 % n;
                            snd.play_at(&format!("English/{name}{pick}.wav"), model_spots[i].pos, listener, 1.0);
                        }
                        tracer = Some((start, hit.point, true, game_time + 0.1));
                    }
                    _ => {
                        // The bullet struck the level: leave a hole and throw some debris
                        if let Some(hit) = wall.and_then(|_| world.raycast_hit(eye, dir, 20000.0)) {
                            let point = [eye[0] + dir[0] * hit.dist, eye[1] + dir[1] * hit.dist, eye[2] + dir[2] * hit.dist];
                            let surface = hit.material
                                .and_then(|m| level.materials.materials.get(m))
                                .map_or(Surface::Concrete, |m| Surface::classify(&m.filename));
                            let surface = args.decal_surface.unwrap_or(surface);
                            let material = hit.material.and_then(|m| level.materials.materials.get(m)).map_or("?", |m| m.filename.as_str());
                            rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
                            let mut sparked = false;
                            if args.realistic_decals {
                                sparked = effects.impact(surface, point, hit.normal, dir, rng);
                            }
                            println!("t={game_time:.2} shot hit {surface:?} ({material}) at {:.0} units{}", hit.dist, if sparked { ", sparks" } else { "" });
                            if surface != Surface::Glass {
                                let (art, variant) = if args.realistic_decals {
                                    (surface, (rng >> 33) as usize % HOLE_VARIANTS)
                                } else {
                                    (if surface == Surface::Wood { Surface::Wood } else { Surface::Concrete }, 0)
                                };
                                if hole_tex.contains_key(&(art, variant)) {
                                    let roll = ((rng >> 20) & 0xFFFF) as f32 / 65536.0;
                                    decals.push(impact::Decal::new(point, hit.normal, art, args.realistic_decals, variant, roll));
                                    if decals.len() > MAX_DECALS {
                                        decals.remove(0);
                                    }
                                }
                            }
                            if let Some(snd) = sound.as_mut() {
                                let name = match surface {
                                    _ if sparked => if rng & 1 == 0 { "C_BLRIC1.WAV" } else { "C_BLRIC2.WAV" },
                                    Surface::Wood => "C_BLWOD.WAV",
                                    Surface::Metal => if rng & 1 == 0 { "C_BLMTTK.WAV" } else { "C_BLMTTN.WAV" },
                                    Surface::Earth => "C_BLEAR.WAV",
                                    _ => "C_BLCON.WAV",
                                };
                                snd.play_at(name, point, listener_of(&camera), 1.0);
                            }
                        }
                        let d = wall.unwrap_or(5000.0);
                        tracer = Some((start, [eye[0] + dir[0] * d, eye[1] + dir[1] * d, eye[2] + dir[2] * d], false, game_time + 0.1));
                    }
                }
            }
        } else {
            // mv is per-frame at 60 FPS; scale so free-fly speed does not depend on frame rate
            camera.position[0] += mv[0] * step_dt * 60.0;
            camera.position[2] += mv[1] * step_dt * 60.0;
        }

        // Grenades in the air: they bounce off the level and go off when the fuse runs out
        shake = (shake - 1.4 * step_dt).max(0.0);
        veil = (veil - 1.6 * step_dt).max(0.0);
        deaf = (deaf - 0.2 * step_dt).max(0.0);
        if let Some(snd) = sound.as_mut() {
            snd.master = master_volume * (1.0 - 0.85 * deaf.min(1.0));
        }
        let mut blasts: Vec<[f32; 3]> = Vec::new();
        for g in thrown.iter_mut() {
            for b in g.update(step_dt, &world) {
                // Glass: it goes through, shattering the pane, slowed and knocked off line
                let heading = unit3(b.incoming);
                let behind = [b.point[0] - heading[0] * 20.0, b.point[1] - heading[1] * 20.0, b.point[2] - heading[2] * 20.0];
                let pane = panes.iter().enumerate()
                    .filter(|(i, _)| broken_at[*i].is_none())
                    .filter_map(|(i, p)| p.raycast(behind, heading).filter(|d| *d < 45.0).map(|d| (i, d)))
                    .min_by(|a, c| a.1.total_cmp(&c.1));
                if let Some((gi, _)) = pane {
                    broken_at[gi] = Some(game_time);
                    world.switch_off(&panes[gi].name);
                    next_rng(&mut rng);
                    fractures.push(glass::shatter(&panes[gi], b.point, heading, rng));
                    let wobble = |bits: u64| ((bits & 0xFFFF) as f32 / 32768.0 - 1.0) * 0.18;
                    let (sx, sy, sz) = (wobble(rng >> 8), wobble(rng >> 24), wobble(rng >> 40));
                    let speed = (b.incoming[0].powi(2) + b.incoming[1].powi(2) + b.incoming[2].powi(2)).sqrt() * 0.6;
                    g.vel = mul3(unit3([heading[0] + sx, heading[1] + sy, heading[2] + sz]), speed);
                    g.pos = [b.point[0] + heading[0] * 12.0, b.point[1] + heading[1] * 12.0, b.point[2] + heading[2] * 12.0];
                    g.at_rest = false;
                    if let Some(snd) = sound.as_mut() {
                        snd.play_at("C_GBRKMD.WAV", b.point, listener_of(&camera), 1.0);
                    }
                    continue;
                }
                if let Some(snd) = sound.as_mut() {
                    let surface = b.material.and_then(|m| level.materials.materials.get(m)).map_or(Surface::Concrete, |m| Surface::classify(&m.filename));
                    let name = match surface {
                        Surface::Metal => "C_GRMET.WAV",
                        Surface::Wood => "C_GRWOD.WAV",
                        Surface::Earth | Surface::Soft => "C_GREAR.WAV",
                        _ => "C_GRCON.WAV",
                    };
                    snd.play_at(name, b.point, listener_of(&camera), (b.speed / 500.0).clamp(0.25, 1.0));
                }
            }
            if g.goes_off() {
                blasts.push(g.pos);
            }
        }
        thrown.retain(|g| !g.goes_off());
        for c in blasts {
            // The surface it went off against, the nearest of the six directions
            let mut near: Option<(f32, [f32; 3], [f32; 3], Option<usize>)> = None;
            for d in [[0.0, -1.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]] {
                if let Some(h) = world.raycast_hit(c, d, 80.0) {
                    if near.is_none_or(|n| h.dist < n.0) {
                        near = Some((h.dist, [c[0] + d[0] * h.dist, c[1] + d[1] * h.dist, c[2] + d[2] * h.dist], h.normal, h.material));
                    }
                }
            }
            let (point, normal, material) = near.map_or((c, [0.0, 1.0, 0.0], None), |n| (n.1, n.2, n.3));
            let surface = material.and_then(|m| level.materials.materials.get(m)).map_or(Surface::Concrete, |m| Surface::classify(&m.filename));
            next_rng(&mut rng);
            // In mid-air (nothing within a meter) there is no ground to scorch or throw dust along
            let on_surface = near.is_some_and(|n| n.0 < 100.0);
            effects.explosion(point, normal, rng, on_surface);
            if on_surface && surface != Surface::Glass {
                let roll = ((rng >> 20) & 0xFFFF) as f32 / 65536.0;
                decals.push(impact::Decal::scorch(point, normal, 100.0 + (rng >> 50) as f32 % 40.0, (rng >> 33) as usize % SCORCH_VARIANTS, roll));
                if decals.len() > MAX_DECALS {
                    decals.remove(0);
                }
            }
            println!("t={game_time:.2} grenade went off at ({:.0}, {:.0}, {:.0}) on {surface:?}", c[0], c[1], c[2]);
            if let Some(snd) = sound.as_mut() {
                snd.play_at("C_GRFRAG.WAV", c, listener_of(&camera), 1.0);
            }

            // Characters: killed close in, hurt further out, and shielded by walls
            for i in 0..model_spots.len() {
                if model_spots[i].health.is_dead() {
                    continue;
                }
                let place = placement(&model_spots[i]);
                let chest = [place.pos[0], place.pos[1] + 100.0, place.pos[2]];
                let head = [place.pos[0], place.pos[1] + 160.0, place.pos[2]];
                let strength = grenade::blast_strength(dist3(c, chest));
                if strength < 0.02 || !(grenade::clear_between(&world, c, chest) || grenade::clear_between(&world, c, head)) {
                    continue;
                }
                let from = place.facing_of_shot(unit3([chest[0] - c[0], chest[1] - c[1], chest[2] - c[2]]));
                next_rng(&mut rng);
                let result = model_spots[i].health.apply_blast(100.0 * strength, from, game_time, (rng >> 33) as u32);
                target_hp = Some(model_spots[i].health.hp);
                println!("t={game_time:.2} grenade hit character {i} at {:.0} units: {result:?}, hp now {}", dist3(c, chest), model_spots[i].health.hp);
                if matches!(result, combat::HitResult::Killed) {
                    kills += 1;
                    message = ("GRENADE KILL".to_string(), game_time + 1.2);
                }
                if let Some(snd) = sound.as_mut() {
                    let (name, n) = if matches!(result, combat::HitResult::Killed) { ("re_ded", 2) } else { ("re_wnd", 3) };
                    snd.play_at(&format!("English/{name}{}.wav", 1 + (rng >> 40) as u32 % n), model_spots[i].pos, listener_of(&camera), 1.0);
                }
            }

            // Glass in reach breaks, the nearest panes first
            let mut panes_near: Vec<(usize, f32)> = panes.iter().enumerate()
                .filter(|(i, _)| broken_at[*i].is_none())
                .map(|(i, p)| (i, p.distance_to(c)))
                .filter(|(_, d)| *d <= grenade::GLASS_REACH)
                .collect();
            panes_near.sort_by(|a, b| a.1.total_cmp(&b.1));
            let mut heard = 0;
            for (gi, _) in panes_near {
                let at = panes[gi].center();
                if !grenade::clear_between(&world, c, at) {
                    continue;
                }
                broken_at[gi] = Some(game_time);
                world.switch_off(&panes[gi].name);
                next_rng(&mut rng);
                fractures.push(glass::shatter(&panes[gi], at, unit3([at[0] - c[0], at[1] - c[1], at[2] - c[2]]), rng));
                if heard < 3 {
                    heard += 1;
                    if let Some(snd) = sound.as_mut() {
                        snd.play_at("C_GBRKMD.WAV", at, listener_of(&camera), 1.0);
                    }
                }
            }

            // The player: shaken, dazzled and deafened more the nearer, and less behind a wall
            if first_person {
                let eye = [player.pos[0], player.pos[1] + eye_height, player.pos[2]];
                let d = dist3(c, eye);
                let open = grenade::clear_between(&world, c, eye);
                let s = grenade::shake(d, open);
                shake = shake.max(s);
                if open {
                    veil = veil.max(((900.0 - d) / 900.0).clamp(0.0, 1.0));
                }
                if s > 0.1 {
                    deaf = deaf.max(s.min(1.0));
                    if let Some(snd) = sound.as_mut() {
                        snd.ring(3.5 * s.min(1.0) + 1.0, 0.2 * s.min(1.0));
                    }
                }
            }
        }

        // Reload sound when a reload starts (by key or running dry)
        if reload_until.is_some() && !was_reloading {
            if let Some(snd) = sound.as_mut() {
                snd.play("C_SBMAG1.WAV", 1.0);
            }
        }
        was_reloading = reload_until.is_some();

        // Full auto: the firing sound runs while the trigger is held with rounds to fire,
        // then stops with a closing crack
        let firing = first_person && gun_trigger && reload_until.is_none() && ammo > 0;
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

        // The room the player is in: the point just above the feet (the camera when
        // flying), low enough to land inside the thin slabs that stairs and ramps use.
        // Where no room claims it, the last room stays.
        // The camera's Z is negated relative to the map
        let probe = [camera.position[0], camera.position[1] - if first_person { eye_height - 10.0 } else { 0.0 }, -camera.position[2]];
        if let Some(i) = level.rooms.locate(probe, current_room) {
            current_room = Some(i);
        }
        // Where the player's feet and eyes are, for portals they are standing in
        let body = [probe, [camera.position[0], camera.position[1], -camera.position[2]]];
        // Only the player's room and the rooms its portals lead to can stop movement or
        // bullets: the level overlaps itself, and the exterior of a plane must not block
        // anyone inside it
        let player_rooms = visibility.player_rooms(&level, probe, current_room);
        // The room drawing starts from: the one whose floor the player stands on, as
        // overlapping rooms (a stairwell beside a hall, an upstairs over a downstairs)
        // all claim the same spot. Airborne, the last one stays; flying, the smallest
        // room around the camera.
        let floor_room = if first_person {
            world.floor_room([probe[0], probe[1] - 10.0, probe[2]])
                .filter(|&n| visibility.is_room(n))
        } else {
            None
        };
        let render_rooms: Vec<u32> = match floor_room {
            Some(n) => vec![n],
            None if first_person && !last_render_rooms.is_empty() => last_render_rooms.clone(),
            None => current_room.and_then(|i| visibility.number_of(i)).map(|n| vec![n]).unwrap_or_else(|| player_rooms.clone()),
        };
        last_render_rooms = render_rooms.clone();
        // What stops the player: their room, the rooms through its portals but only
        // beyond them, and the other rooms they are well inside (they can hold the floor)
        let wanted: Option<Vec<rogue_reborn::collision::SolidRoom>> = if culling && !render_rooms.is_empty() {
            Some(visibility.solid_rooms(&level, &render_rooms, &player_rooms, &body))
        } else {
            None
        };
        if wanted != collision_rooms {
            world.set_solid_rooms(wanted.as_deref());
            collision_rooms = wanted;
        }
        let current_rooms: Vec<&str> = current_room.map(|i| level.rooms.rooms[i].section_name.as_str()).into_iter().collect();
        if room_debug && current_rooms != last_rooms {
            println!("in rooms: {current_rooms:?}");
            last_rooms = current_rooms.iter().map(|s| s.to_string()).collect();
        }

        // Compute dynamic aspect and clip planes
        let (w, h) = window.get_size();
        let aspect = (w as f32 / h.max(1) as f32).max(0.1);
        let z_near = (mesh.radius / 1000.0).max(0.05);
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
            if culling && !render_rooms.is_empty() {
                let (mut modelview, mut projection) = ([0f32; 16], [0f32; 16]);
                glGetFloatv(0x0BA6, modelview.as_mut_ptr()); // GL_MODELVIEW_MATRIX
                glGetFloatv(0x0BA7, projection.as_mut_ptr()); // GL_PROJECTION_MATRIX
                let clip = |v: [f32; 3]| mul_mat4(&projection, mul_mat4(&modelview, [v[0], v[1], v[2], 1.0]));
                plan = render_rooms.iter().map(|&r| visibility.plan(&level, r, &body, cam_eye, &blocked_portals, &clip)).collect();
            }
            drawn_rooms = {
                let mut v: Vec<u32> = Vec::new();
                for node in &plan {
                    node.rooms(&mut v);
                }
                v.sort();
                v.dedup();
                v.iter().map(|n| n.to_string()).collect()
            };

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
                        Which::Room(n) => visibility.objects_of(n),
                        Which::Unowned => visibility.unowned_objects(),
                    };
                    // The doors of these rooms are drawn along with them, each leaf where
                    // its door is
                    let room_doors: &[usize] = match which {
                        Which::All => &all_doors,
                        Which::Room(n) => doors_by_room.get(&n).map(Vec::as_slice).unwrap_or_default(),
                        Which::Unowned => &[],
                    };
                    let items = indices.iter().map(|&i| (i, None)).chain(room_doors.iter().map(|&d| (doors[d].model_index, Some(d))));
                    for (object_index, door) in items {
                        let object = &level.geometries.objects[object_index];
                        // Panes are drawn in place below, and door leaves with their doors
                        if pane_objects[object_index] || (door_leaf[object_index] && door.is_none()) {
                            continue;
                        }
                        let room = visibility.room_of_object(object_index);
                        if room.is_some_and(|n| args.hide_rooms.contains(&n)) {
                            continue;
                        }
                        if let Some(d) = door {
                            glPushMatrix();
                            glMultMatrixf(doors[d].matrix(door_states[d].open).as_ptr());
                        }
                        if room_debug {
                            let [r, g, b] = room_color(&object.name, current_rooms.iter().any(|n| *n == object.name));
                            glColor3f(r, g, b);
                        }
                    for od in &object.object_datas {
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

                            glBegin(0x0004); // GL_TRIANGLES
                            for i in 0..face_count {
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
                    }
                    if door.is_some() {
                        glPopMatrix();
                    }
                    }
                };

                if plan.is_empty() {
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

            // Characters at the spawn points, posed for what they are doing
            if let Some(c) = &character {
                glDisable(0x0DE1); // GL_TEXTURE_2D
                let clock = start_time.elapsed().as_secs_f32();
                for spot in &model_spots {
                    let posed = npc_verts(spot, game_time);
                    let verts: &[[f32; 3]] = if posed.is_empty() { &c.vertices } else { &posed };
                    let y = if posed.is_empty() { spot.pos[1] } else { spot.floor };
                    glPushMatrix();
                    glTranslatef(spot.pos[0], y, spot.pos[2]);
                    glRotatef(spot.yaw, 0.0, 1.0, 0.0);
                    let scale = spot.actor.map_or([1.0; 3], |a| actors[a].scale);
                    glScalef(scale[0], scale[1], scale[2]);
                    // Eyes shut for 0.15 s every 4 s, staggered per model; the dead keep them shut
                    let blinking = args.eyes_closed || spot.health.is_dead()
                        || (blink_enabled && (clock + spot.blink_offset) % 4.0 < 0.15);
                    let face = spot.actor.map(|a| face_textures[a]).and_then(|(main, blink)| {
                        if blinking { blink.or(main) } else { main }
                    });
                    draw_character(c, verts, &part_textures, face, textured);
                    glPopMatrix();
                }
                glColor4f(1.0, 1.0, 1.0, 1.0);
            }

            // The player's own body, shown in third person
            if let (true, true, Some(c), Some((skel, skin))) = (first_person, third_person, &character, &rig) {
                if let Some(mot) = player_motions.get(movement) {
                    let (root, mut rots) = mot.sample(player_frame.min(mot.frames.saturating_sub(1) as f32));
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
                    let clock = start_time.elapsed().as_secs_f32();
                    let blinking = args.eyes_closed || (blink_enabled && clock % 4.0 < 0.15);
                    let face = (!actors.is_empty()).then(|| face_textures[0]).and_then(|(main, blink)| {
                        if blinking { blink.or(main) } else { main }
                    });
                    glPushMatrix();
                    glTranslatef(player.pos[0], player.pos[1], player.pos[2]);
                    // The body faces where the player looks (model forward is +Z)
                    glRotatef(-camera.yaw, 0.0, 1.0, 0.0);
                    let scale = actors.first().map_or([1.0; 3], |a| a.scale);
                    glScalef(scale[0], scale[1], scale[2]);
                    draw_character(c, &verts, &part_textures, face, textured);

                    // The MP5K in the right hand, or a grenade until it is thrown
                    let in_hand = if equipped_grenade { throw.is_none_or(|t| !t.thrown).then_some(&grenade_draw) } else { Some(&weapon_draw) };
                    if let (Some(Some(wd)), Some((hand, off))) = (in_hand, &weapon_attach) {
                        glPushMatrix();
                        glMultMatrixf(pose[*hand].matrix().as_ptr());
                        glMultMatrixf(off.matrix().as_ptr());
                        if equipped_grenade {
                            glTranslatef(-wd.center[0], -wd.center[1], -wd.center[2]);
                        }
                        draw_weapon(wd, textured, !equipped_grenade && game_time < flash_until);
                        glPopMatrix();
                    }
                    glPopMatrix();
                    glColor4f(1.0, 1.0, 1.0, 1.0);
                }
            }

            // Grenades in the air
            if let Some(gd) = &grenade_draw {
                for g in &thrown {
                    glPushMatrix();
                    glTranslatef(g.pos[0], g.pos[1], g.pos[2]);
                    glRotatef(g.spin, 1.0, 0.3, 0.0);
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
                f.update(step_dt, &|x, z, y| world.ground_at(x, z, y + 30.0));
            }
            fractures.retain(|f| !f.done());
            for (pane, _) in panes.iter().zip(&broken_at).filter(|(_, b)| b.is_none()) {
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
        let held = if equipped_grenade { grenade_draw.as_ref().filter(|_| throw.is_none_or(|t| !t.thrown)) } else { weapon_draw.as_ref() };
        if let (true, true, false, Some(wd)) = (args.viewmodel, first_person, third_person, held) {
            let l = wd.length;
            let [ox, oy, oz] = if equipped_grenade {
                let held_for = charge_from.map_or(0.0, |from| ((game_time - from) / grenade::CHARGE_SECONDS).clamp(0.0, 1.0));
                let swing = throw.map_or(0.0, |t| t.charge.max(((game_time - t.start) / THROW_SECONDS / THROW_RELEASE).clamp(0.0, 1.0)));
                let wind = held_for.max(swing);
                [1.7 + 0.9 * wind, -1.2 - 0.6 * wind, 8.0 - 1.8 * wind]
            } else {
                args.viewmodel_offset.unwrap_or([0.45, -0.45, 1.5])
            };
            // Recoil pushes the gun back at the player; a reload dips it out of view
            let kick = ((flash_until - game_time) / 0.05).clamp(0.0, 1.0) * 0.04 * l;
            let dip = reload_until.map_or(0.0, |t| {
                let p = (1.0 - (t - game_time) / RELOAD_SECONDS).clamp(0.0, 1.0);
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
                draw_weapon(wd, textured, game_time < flash_until);
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
                format!("KILLS: {kills}"),
                if equipped_grenade { format!("GRENADES: {grenades_left}") }
                    else if reload_until.is_some() { "RELOADING".to_string() } else { format!("AMMO: {ammo}/{reserve}") },
            ];
            if let Some(from) = charge_from {
                status.insert(0, format!("POWER: {:.0}", ((game_time - from) / grenade::CHARGE_SECONDS).clamp(0.0, 1.0) * 100.0));
            }
            if let Some(hp) = target_hp {
                status.insert(0, format!("TARGET: {hp:.0}"));
            }
            let msg = if game_time < message.1 { message.0.as_str() } else { "" };
            let marker = tracer.is_some_and(|t| t.2 && game_time < t.3);
            draw_hud(w, h, &status, msg, marker, veil);
        }
        let rooms_text = if culling {
            format!("ROOM: {} DRAWN: {}", current_rooms.join(" "), drawn_rooms.join(" "))
        } else {
            format!("ROOM: {} (CULLING OFF)", current_rooms.join(" "))
        };
        let mut debug_lines: Vec<&str> = vec![&level_text, &fps_text, &render_text];
        if room_debug {
            debug_lines.push(&rooms_text);
        }
        draw_debug_text(w, h, &debug_lines);

        // F2: a screenshot and a text log of where the player is and what they see
        if dump_requested && (!args.dump || frames_drawn >= 2) {
            dump_requested = false;
            let (fw, fh) = window.get_framebuffer_size();
            let rgb = read_framebuffer(fw, fh);
            let eye = [camera.position[0], camera.position[1], -camera.position[2]];
            let f = camera.get_forward();
            let dir = [f[0], f[1], -f[2]];
            let hit = world.raycast_hit(eye, dir, 20000.0).map(|hit| {
                let point = [eye[0] + dir[0] * hit.dist, eye[1] + dir[1] * hit.dist, eye[2] + dir[2] * hit.dist];
                let material = hit.material.and_then(|m| level.materials.materials.get(m)).map_or("?", |m| m.filename.as_str());
                (hit, point, material)
            });
            let feet = [eye[0], eye[1] - if first_person { eye_height } else { 0.0 }, eye[2]];
            let nearby_doors: Vec<String> = doors.iter().zip(&door_states)
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
                fps, render_ms, culling, room: current_rooms.first().copied(), drawn: &drawn_rooms, collision: &collision_rooms, player_rooms: &player_rooms, render_rooms: &render_rooms, doors: &nearby_doors,
            };
            match write_debug_dump(&report, &rgb, fw as u32, fh as u32) {
                Ok(stem) => println!("wrote {stem}.png and {stem}.txt"),
                Err(e) => eprintln!("debug dump failed: {e:#}"),
            }
            if args.dump {
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
        if let Some(path) = &args.screenshot {
            if frames_drawn >= args.walk.map_or(3, |n| n.max(3)) {
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

/// How far away the door being opened or closed can be
const DOOR_REACH: f32 = 220.0;
