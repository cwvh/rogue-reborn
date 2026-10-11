use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use glfw::{Action, Context, Key, WindowEvent, CursorMode};
use rogue_reborn::{map, rsb};
use rogue_reborn::collision::{CollisionWorld, Params, Player};
use rogue_reborn::{actor, combat, crp, demo, grenade, motion, skeleton, sound, tga, weapon};
use rogue_reborn::ambience;
use rogue_reborn::glass;
use rogue_reborn::impact::{self, Surface};
use rogue_reborn::visibility;
use rogue_reborn::door;

#[cfg_attr(windows, link(name = "opengl32"))]
#[cfg_attr(target_os = "macos", link(name = "OpenGL", kind = "framework"))]
#[cfg_attr(not(any(windows, target_os = "macos")), link(name = "GL"))]
unsafe extern "C" {
    fn glViewport(x: i32, y: i32, width: i32, height: i32);
    fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    fn glEnable(cap: u32);
    fn glClipPlane(plane: u32, equation: *const f64);
    fn glStencilFunc(func: u32, reference: i32, mask: u32);
    fn glStencilOp(fail: u32, zfail: u32, zpass: u32);
    fn glClearStencil(s: i32);
    fn glColorMask(r: u8, g: u8, b: u8, a: u8);
    fn glDepthFunc(func: u32);
    fn glDepthRange(near: f64, far: f64);
    fn glGetFloatv(pname: u32, params: *mut f32);
    fn glGetIntegerv(pname: u32, params: *mut i32);
    fn glPolygonMode(face: u32, mode: u32);
    fn glClear(mask: u32);
    fn glMatrixMode(mode: u32);
    fn glLoadIdentity();
    fn glMultMatrixf(m: *const f32);
    fn glBegin(mode: u32);
    fn glEnd();
    fn glPointSize(size: f32);
    fn glVertex3f(x: f32, y: f32, z: f32);
    fn glRotatef(angle: f32, x: f32, y: f32, z: f32);
    fn glTranslatef(x: f32, y: f32, z: f32);
    fn glScalef(x: f32, y: f32, z: f32);
    fn glPushMatrix();
    fn glLineWidth(width: f32);
    fn glDepthMask(flag: u8);
    fn glPolygonOffset(factor: f32, units: f32);
    fn glColor4f(r: f32, g: f32, b: f32, a: f32);
    fn glBlendFunc(sfactor: u32, dfactor: u32);
    fn glPopMatrix();
    fn glColor3f(r: f32, g: f32, b: f32);
    fn glTexCoord2f(s: f32, t: f32);
    fn glTexParameteri(target: u32, pname: u32, param: i32);
    fn glGenTextures(n: i32, textures: *mut u32);
    fn glBindTexture(target: u32, texture: u32);
    fn glTexImage2D(
        target: u32,
        level: i32,
        internalformat: i32,
        width: i32,
        height: i32,
        border: i32,
        format: u32,
        type_: u32,
        pixels: *const std::ffi::c_void
    );
    fn glDisable(cap: u32);
    fn glReadPixels(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: u32,
        type_: u32,
        pixels: *mut std::ffi::c_void
    );
    fn glPixelStorei(pname: u32, param: i32);
}

// Command-line options; mainly for repeatable headless screenshots
#[derive(Default)]
struct Args {
    /// A level by name, such as rm11 or citystreet_large, found under data/map
    level: Option<String>,
    list_levels: bool,
    /// The built-in box room instead of a level from the game
    demo: bool,
    /// Characters standing in the demo room (0 to 8, default 8)
    npcs: Option<usize>,
    map: Option<PathBuf>,
    screenshot: Option<PathBuf>,
    pos: Option<[f32; 3]>,
    yaw: Option<f32>,
    pitch: Option<f32>,
    textured: bool,
    portals: bool,
    /// Color each room's geometry and report the rooms the player is in
    rooms: bool,
    /// Write the F2 debug dump on the first frame and exit
    dump: bool,
    /// Draw every room instead of only the ones visible through portals
    no_cull: bool,
    /// Start with every door this far open (0 shut, 1 open)
    doors_open: Option<f32>,
    /// Rooms to leave out of the drawing, to find which one a stray surface belongs to
    hide_rooms: Vec<u32>,
    collisions: bool,
    first_person: bool,
    info: bool,
    spawn_zone: Option<String>,
    seed: Option<u64>,
    model: Option<PathBuf>,
    no_models: bool,
    no_sound: bool,
    /// Master volume, 0.0..=1.0
    volume: Option<f32>,
    actor: Option<String>,
    model_yaw: Option<f32>,
    no_blink: bool,
    eyes_closed: bool,
    anim: Option<String>,
    anim_fps: Option<f32>,
    /// Frame rate cap (also turns vsync off)
    max_fps: Option<f32>,
    anim_frame: Option<f32>,
    // Mission spawn point to use, counting from 1
    spawn: Option<usize>,
    speed_scale: Option<f32>,
    // Multiplies the playback rate of the movement animations (default 1.5)
    anim_scale: Option<f32>,
    // Overrides for the player's movement set: slot=motion (walk, run, back, left, right, idle, fire, reload)
    player_anims: Vec<(String, String)>,
    third_person: bool,
    weapon_offset: Option<[f32; 3]>,
    /// Draw the weapon in front of the camera in first person (the original game had no such view)
    viewmodel: bool,
    /// Bullet holes drawn for the surface hit, with splinters, sparks and dust, instead of the game sprites
    realistic_decals: bool,
    /// Treat every wall as this surface (wood, metal, concrete, earth, soft), to preview bullet holes
    decal_surface: Option<Surface>,
    /// Where the held weapon sits in the view, in gun lengths: right, up, forward
    viewmodel_offset: Option<[f32; 3]>,
    view_orbit: f32,
    view_dist: Option<f32>,
    hold: Option<String>,
    // Hold W for this many frames and print the position trace
    walk: Option<u32>,
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--map" => args.map = it.next().map(PathBuf::from),
            "--level" => args.level = it.next(),
            "--list-levels" => args.list_levels = true,
            "--demo" => {
                args.demo = true;
                // The player is always the first of the demo room's spawn points
                args.spawn = Some(1);
            }
            "--npcs" => args.npcs = it.next().and_then(|v| v.parse().ok()),
            "--screenshot" => args.screenshot = it.next().map(PathBuf::from),
            "--pos" => {
                let v: Vec<f32> = it.next().unwrap_or_default()
                    .split(',').filter_map(|n| n.parse().ok()).collect();
                if v.len() == 3 {
                    args.pos = Some([v[0], v[1], v[2]]);
                } else {
                    eprintln!("--pos expects x,y,z");
                }
            }
            "--yaw" => args.yaw = it.next().and_then(|v| v.parse().ok()),
            "--pitch" => args.pitch = it.next().and_then(|v| v.parse().ok()),
            "--textured" => args.textured = true,
            "--portals" => args.portals = true,
            "--collisions" => args.collisions = true,
            "--rooms" => args.rooms = true,
            "--dump" => args.dump = true,
            "--no-cull" => args.no_cull = true,
            "--doors-open" => args.doors_open = it.next().and_then(|v| v.parse().ok()),
            "--hide-rooms" => args.hide_rooms = it.next().map(|v| v.split(',').filter_map(|n| n.parse().ok()).collect()).unwrap_or_default(),
            "--info" => args.info = true,
            "--spawn-zone" => args.spawn_zone = it.next(),
            "--model" => args.model = it.next().map(PathBuf::from),
            "--no-models" => args.no_models = true,
            "--no-sound" => args.no_sound = true,
            "--volume" => args.volume = it.next().and_then(|v| v.parse().ok()),
            "--actor" => args.actor = it.next(),
            "--model-yaw" => args.model_yaw = it.next().and_then(|v| v.parse().ok()),
            "--no-blink" => args.no_blink = true,
            "--eyes-closed" => args.eyes_closed = true,
            "--anim" => args.anim = it.next(),
            "--max-fps" => args.max_fps = it.next().and_then(|v| v.parse().ok()).filter(|f: &f32| *f > 0.0),
            "--anim-fps" => args.anim_fps = it.next().and_then(|v| v.parse().ok()),
            "--third-person" => args.third_person = true,
            "--view-orbit" => args.view_orbit = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0),
            "--view-dist" => args.view_dist = it.next().and_then(|v| v.parse().ok()),
            "--viewmodel" => args.viewmodel = true,
            "--realistic-decals" => args.realistic_decals = true,
            "--decal-surface" => {
                args.decal_surface = match it.next().unwrap_or_default().to_ascii_lowercase().as_str() {
                    "wood" => Some(Surface::Wood),
                    "metal" => Some(Surface::Metal),
                    "concrete" => Some(Surface::Concrete),
                    "earth" => Some(Surface::Earth),
                    "soft" => Some(Surface::Soft),
                    other => {
                        eprintln!("--decal-surface expects wood, metal, concrete, earth or soft, not {other:?}");
                        None
                    }
                };
            }
            "--viewmodel-offset" => {
                let v: Vec<f32> = it.next().unwrap_or_default().split(',').filter_map(|n| n.parse().ok()).collect();
                if v.len() == 3 { args.viewmodel_offset = Some([v[0], v[1], v[2]]); } else { eprintln!("--viewmodel-offset expects right,up,forward"); }
            }
            "--weapon-offset" => {
                let v: Vec<f32> = it.next().unwrap_or_default().split(',').filter_map(|n| n.parse().ok()).collect();
                if v.len() == 3 { args.weapon_offset = Some([v[0], v[1], v[2]]); } else { eprintln!("--weapon-offset expects x,y,z"); }
            }
            "--hold" => args.hold = it.next(),
            "--player-anim" => {
                let v = it.next().unwrap_or_default();
                match v.split_once('=') {
                    Some((k, m)) => args.player_anims.push((k.to_string(), m.to_string())),
                    None => eprintln!("--player-anim expects slot=motion"),
                }
            }
            "--anim-scale" => args.anim_scale = it.next().and_then(|v| v.parse().ok()),
            "--speed-scale" => args.speed_scale = it.next().and_then(|v| v.parse().ok()),
            "--spawn" => args.spawn = it.next().and_then(|v| v.parse().ok()),
            "--anim-frame" => args.anim_frame = it.next().and_then(|v| v.parse().ok()),
            "--seed" => args.seed = it.next().and_then(|v| v.parse().ok()),
            "--first-person" => args.first_person = true,
            "--walk" => {
                args.walk = it.next().and_then(|v| v.parse().ok());
                args.first_person = true;
            }
            other => eprintln!("ignoring unknown argument: {other}"),
        }
    }
    args
}

// Read the current back buffer as top-down RGB8
fn read_framebuffer(width: i32, height: i32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut bottom_up = vec![0u8; w * h * 3];
    unsafe {
        glPixelStorei(0x0D05, 1); // GL_PACK_ALIGNMENT
        glReadPixels(0, 0, width, height, 0x1907, 0x1401, // GL_RGB, GL_UNSIGNED_BYTE
            bottom_up.as_mut_ptr() as *mut std::ffi::c_void);
    }
    let mut out = Vec::with_capacity(bottom_up.len());
    for row in bottom_up.chunks_exact(w * 3).rev() {
        out.extend_from_slice(row);
    }
    out
}

struct Camera {
    position: [f32; 3],
    yaw: f32,
    pitch: f32,
}

impl Camera {
    fn new() -> Self {
        Self {
            position: [0.0, 0.0, 10.0],
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    fn get_forward(&self) -> [f32; 3] {
        let (yaw_rad, pitch_rad) = (self.yaw.to_radians(), self.pitch.to_radians());
        [
            -yaw_rad.sin() * pitch_rad.cos(),
            pitch_rad.sin(),
            -yaw_rad.cos() * pitch_rad.cos(),
        ]
    }

    fn get_right(&self) -> [f32; 3] {
        let yaw_rad = self.yaw.to_radians();
        [
            yaw_rad.cos(),
            0.0,
            -yaw_rad.sin(),
        ]
    }
}

struct Mesh {
    vertices: Vec<[f32; 3]>,
    indices: Vec<[u32; 3]>,
    center: [f32; 3],
    radius: f32,
}

// What the player's body is doing, which picks the animation
#[derive(Clone, Copy, PartialEq, Debug)]
enum Movement {
    Idle,
    Walk,
    Run,
    Back,
    StrafeLeft,
    StrafeRight,
    /// Standing and shooting, or reloading, with the weapon
    Fire,
    Reload,
    /// Throwing a grenade
    Throw,
}

struct PlayerMotions {
    idle: Option<motion::Motion>,
    walk: Option<motion::Motion>,
    run: Option<motion::Motion>,
    back: Option<motion::Motion>,
    left: Option<motion::Motion>,
    right: Option<motion::Motion>,
    fire: Option<motion::Motion>,
    reload: Option<motion::Motion>,
    throw: Option<motion::Motion>,
}

impl PlayerMotions {
    fn get(&self, m: Movement) -> Option<&motion::Motion> {
        match m {
            Movement::Idle => &self.idle,
            Movement::Walk => &self.walk,
            Movement::Run => &self.run,
            Movement::Back => &self.back,
            Movement::StrafeLeft => &self.left,
            Movement::StrafeRight => &self.right,
            Movement::Fire => &self.fire,
            Movement::Reload => &self.reload,
            Movement::Throw => &self.throw,
        }.as_ref()
    }
}

const RELOAD_SECONDS: f32 = 2.2;

/// A grenade throw: from the start of the swing to the end, and how far into it the grenade
/// leaves the hand (0..1)
const THROW_SECONDS: f32 = 0.85;
const THROW_RELEASE: f32 = 0.35;
/// Soot marks a blast can leave
const SCORCH_VARIANTS: usize = 4;

/// A throw under way
#[derive(Clone, Copy)]
struct Throw {
    start: f32,
    /// How long the throw was held for before letting go (0..1)
    charge: f32,
    /// Whether the grenade has left the hand
    thrown: bool,
}

fn next_rng(r: &mut u64) -> u64 {
    *r ^= *r << 13;
    *r ^= *r >> 7;
    *r ^= *r << 17;
    *r
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn mul3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn unit3(a: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-6);
    [a[0] / l, a[1] / l, a[2] / l]
}

// Bones driven by the weapon hold rather than the locomotion cycle
fn is_upper_body(name: &str) -> bool {
    matches!(name, "Chest" | "LeftCollar" | "RightCollar" | "LeftUpArm" | "LeftLowArm" | "LeftHand"
        | "RightUpArm" | "RightLowArm" | "RightHand" | "Neck" | "Head")
}

// Ground speed (units per second) at which each animation plays at its own
// pace without the feet sliding. These are the speeds with the least slip of
// a planted foot, found with `SPEED=100 cargo run --release --example
// motion-info <motion>`. A planted foot is never perfectly still, so some
// slip is left.
fn nominal_speed(m: Movement) -> f32 {
    match m {
        Movement::Idle | Movement::Fire | Movement::Reload | Movement::Throw => 1.0,
        Movement::Walk => 257.0,
        Movement::Run => 440.0,
        Movement::Back => 126.0,
        Movement::StrafeLeft => 139.0,
        Movement::StrafeRight => 139.0,
    }
}

// Ground speed the player actually moves at. Matches the game where known
// (tuned by the user with --speed-scale); otherwise the animation's own pace.
fn ground_speed(m: Movement) -> f32 {
    match m {
        Movement::Run => 735.0,
        Movement::Walk => 514.0,
        Movement::Back => 378.0,
        Movement::StrafeLeft | Movement::StrafeRight => 348.0,
        other => nominal_speed(other),
    }
}

// The listener for positional sounds, in map space (camera space has Z flipped)
fn listener_of(camera: &Camera) -> sound::Listener {
    let r = camera.get_right();
    sound::Listener {
        pos: [camera.position[0], camera.position[1], -camera.position[2]],
        right: [r[0], r[1], -r[2]],
    }
}

// Pick the movement from the held keys
fn movement_from_keys(forward: bool, back: bool, left: bool, right: bool, run: bool) -> Movement {
    if forward {
        if run { Movement::Run } else { Movement::Walk }
    } else if back {
        Movement::Back
    } else if left && !right {
        Movement::StrafeLeft
    } else if right && !left {
        Movement::StrafeRight
    } else {
        Movement::Idle
    }
}

// The weapon model as flat triangle corner arrays, ready to draw
struct WeaponDraw {
    verts: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    texture: Option<u32>,
    muzzle: [f32; 3],
    /// Middle of the model's bounding box, and its longest side
    center: [f32; 3],
    length: f32,
}

// Draw the weapon model in the current matrix, with a muzzle flash if asked
fn draw_weapon(wd: &WeaponDraw, textured: bool, flash: bool) {
    unsafe {
        if let (true, Some(tex)) = (textured, wd.texture) {
            glEnable(0x0DE1);
            glBindTexture(0x0DE1, tex);
            glColor4f(1.0, 1.0, 1.0, 1.0);
        } else {
            glDisable(0x0DE1);
            glColor4f(0.7, 0.7, 0.75, 1.0);
        }
        glBegin(0x0004); // GL_TRIANGLES
        for (v, uv) in wd.verts.iter().zip(&wd.uvs) {
            glTexCoord2f(uv[0], uv[1]);
            glVertex3f(v[0], v[1], v[2]);
        }
        glEnd();
        // Muzzle flash: a short burst of lines out of the barrel
        if flash {
            glDisable(0x0DE1);
            glColor4f(1.0, 0.85, 0.3, 1.0);
            let m = wd.muzzle;
            glBegin(0x0001); // GL_LINES
            for (dx, dy, dz) in [(0.0, -22.0, 0.0), (7.0, -6.0, 0.0), (-7.0, -6.0, 0.0), (0.0, -6.0, 7.0), (0.0, -6.0, -7.0)] {
                glVertex3f(m[0], m[1], m[2]);
                glVertex3f(m[0] + dx, m[1] + dy, m[2] + dz);
            }
            glEnd();
        }
    }
}

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
                    if let Some((last_x, last_y)) = last_cursor_pos {
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

// Spawn at the center of a named zone from the multiplayer mission file
// (data/mission/<MAP>_MUL.mis). Zones appear there as two MAP-space points
// followed by the quoted zone name, e.g. "ZONE_RM11_COURTYARD". Returns the
// feet position in MAP space, standing on the floor under the zone's center.
fn find_zone_spawn(map_path: &Path, zone: &str, world: &CollisionWorld) -> Option<[f32; 3]> {
    let stem = map_path.file_stem()?.to_string_lossy().to_uppercase();
    let mission = Path::new("data/mission").join(format!("{stem}_MUL.mis"));
    let text = std::fs::read_to_string(mission).ok()?;
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let name = format!("\"ZONE_{stem}_{zone}\"");
    let at = lines.iter().position(|l| l.starts_with(&name))?;
    if at < 2 {
        return None;
    }
    let point = |l: &str| -> Option<[f32; 3]> {
        let v: Vec<f32> = l.split_whitespace().map_while(|n| n.parse().ok()).collect();
        (v.len() >= 3).then(|| [v[0], v[1], v[2]])
    };
    let (a, b) = (point(lines[at - 2])?, point(lines[at - 1])?);
    let (x, y, z) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0);
    let floor = world.ground_at(x, z, y + 100.0).unwrap_or(y);
    Some([x, floor, z])
}

// Collect the adversarial spawn points (MAP space) from every mission file in
// data/mission that names this map. The "Number of spawn points" line is
// followed by that many "x y z" lines. Variants of a mission (_MUL, _MULT)
// repeat the same points, so duplicates are dropped.
fn find_mission_spawns(map_path: &Path) -> Vec<[f32; 3]> {
    let mut points: Vec<[f32; 3]> = Vec::new();
    let Some(map_name) = map_path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return points;
    };
    let Ok(dir) = std::fs::read_dir("data/mission") else { return points };
    let mut files: Vec<PathBuf> = dir.flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mis")))
        .collect();
    files.sort();

    for file in files {
        let Ok(bytes) = std::fs::read(&file) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().map(str::trim).collect();

        // The first quoted string in the file is the map filename
        let named = lines.iter().find_map(|l| {
            let rest = l.strip_prefix('"')?;
            Some(rest[..rest.find('"')?].to_string())
        });
        if !named.is_some_and(|n| n.eq_ignore_ascii_case(&map_name)) {
            continue;
        }

        let Some(at) = lines.iter().position(|l| l.contains("Number of spawn points")) else { continue };
        let count: usize = lines[at].split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
        for l in lines.iter().skip(at + 1).take(count) {
            let v: Vec<f32> = l.split_whitespace().map_while(|n| n.parse().ok()).collect();
            if v.len() >= 3 {
                let p = [v[0], v[1], v[2]];
                if !points.contains(&p) {
                    points.push(p);
                }
            }
        }
    }
    points
}

// Pick an index in 0..n; random unless a seed is given (splitmix64)
fn pick_index(n: usize, seed: Option<u64>) -> usize {
    let seed = seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64)
    });
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z % n as u64) as usize
}

// Load the texture for a material name such as "art_h_torso.bmp" from a
// character directory: the .rsb of the same name, or for .tga materials the
// .tga itself. File names differ in case from the references.
fn load_character_texture(dir: &Path, material: &str) -> Option<u32> {
    let want = Path::new(material);
    let stem = want.file_stem()?.to_string_lossy().to_lowercase();
    let is_tga = want.extension().is_some_and(|e| e.eq_ignore_ascii_case("tga"));
    let path = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        let ext = if is_tga { "tga" } else { "rsb" };
        p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
            && p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == stem)
    })?;
    if is_tga {
        let t = tga::read(&path).ok()?;
        upload_rgba_to_gl(t.width, t.height, &t.rgba)
    } else {
        upload_rsb_to_gl(&rsb::read(&path).ok()?)
    }
}

// Draw one posed character: `verts` are its (possibly animated) vertex
// positions and `face` replaces the HRT placeholder face texture.
fn draw_character(c: &crp::Crp, verts: &[[f32; 3]], part_textures: &[Option<u32>], face: Option<u32>, textured: bool) {
    unsafe {
    // Opaque parts first, then glass (.tga materials) blended over them
    for glass_pass in [false, true] {
    for (pi, part) in c.parts.iter().enumerate() {
        let is_glass = part.material.to_ascii_lowercase().ends_with(".tga");
        if is_glass != glass_pass { continue; }
        // The HRT placeholder face is replaced by the team member's own
        let is_face = part.material.eq_ignore_ascii_case("hrt_face.bmp");
        let tex = if is_face { face } else { None }
            .or(part_textures.get(pi).copied().flatten());
        let uvs = part.corner_uvs.as_ref();
        if is_glass && textured { glEnable(0x0BE2); glBlendFunc(0x0302, 0x0303); } // GL_BLEND, SRC_ALPHA, ONE_MINUS_SRC_ALPHA
        // Solid mode shows the texture; wireframe shows flat color lines
        if let (true, Some(id), Some(_)) = (textured, tex, uvs) {
            glEnable(0x0DE1); // GL_TEXTURE_2D
            glBindTexture(0x0DE1, id);
            glColor4f(1.0, 1.0, 1.0, 1.0);
        } else {
            glDisable(0x0DE1);
            if textured && is_glass {
                // Glass with no texture file: faint tinted pane
                glColor4f(0.45, 0.55, 0.65, 0.2);
            } else if textured {
                glColor4f(0.3, 0.35, 0.4, 1.0);
            } else {
                let (r, g, b) = material_color(&part.material);
                glColor4f(r, g, b, 1.0);
            }
        }
        glBegin(0x0004); // GL_TRIANGLES
        for (k, t) in part.triangles.iter().enumerate() {
            for (j, &i) in t.iter().enumerate() {
                if let Some(uvs) = uvs {
                    glTexCoord2f(uvs[k][j][0], uvs[k][j][1]);
                }
                let v = &verts[i as usize];
                glVertex3f(v[0], v[1], v[2]);
            }
        }
        glEnd();
        if is_glass { glDisable(0x0BE2); }
    }
    }
    }
}

// Draw blended glass triangles (world space corners and UVs) with a material's texture
fn draw_glass(
    materials: &map::Materials,
    material: usize,
    map_num: &str,
    textures: &mut HashMap<String, u32>,
    textured: bool,
    alpha: f32,
    tris: impl Iterator<Item = ([[f32; 3]; 3], [[f32; 2]; 3])>,
) {
    unsafe {
        glEnable(0x0BE2); // GL_BLEND
        glBlendFunc(0x0302, 0x0303); // SRC_ALPHA, ONE_MINUS_SRC_ALPHA
        match (textured, ensure_texture(materials, material, map_num, textures)) {
            (true, Some(id)) => {
                glEnable(0x0DE1);
                glBindTexture(0x0DE1, id);
                glColor4f(1.0, 1.0, 1.0, alpha);
            }
            _ => {
                glDisable(0x0DE1);
                glColor4f(0.6, 0.8, 0.9, alpha.max(0.3));
            }
        }
        glBegin(0x0004); // GL_TRIANGLES
        for (p, uv) in tris {
            for k in 0..3 {
                glTexCoord2f(uv[k][0], uv[k][1]);
                glVertex3f(p[k][0], p[k][1], p[k][2]);
            }
        }
        glEnd();
        glDisable(0x0BE2);
        glColor4f(1.0, 1.0, 1.0, 1.0);
    }
}

// Draw a broken pane: its pieces with a glint along their edges, and the flying flecks
fn draw_fracture(materials: &map::Materials, map_num: &str, textures: &mut HashMap<String, u32>, textured: bool, f: &glass::Fracture) {
    unsafe {
        glEnable(0x0BE2); // GL_BLEND
        glBlendFunc(0x0302, 0x0303); // SRC_ALPHA, ONE_MINUS_SRC_ALPHA
        let tex = if textured { ensure_texture(materials, f.material, map_num, textures) } else { None };
        for s in &f.shards {
            let (corners, alpha) = (s.corners(), s.alpha());
            if let Some(id) = tex {
                glEnable(0x0DE1);
                glBindTexture(0x0DE1, id);
                glColor4f(1.0, 1.0, 1.0, alpha * f.opacity);
            } else {
                glDisable(0x0DE1);
                glColor4f(0.6, 0.8, 0.9, alpha * 0.4);
            }
            glBegin(0x0006); // GL_TRIANGLE_FAN
            for (c, uv) in corners.iter().zip(s.uvs()) {
                glTexCoord2f(uv[0], uv[1]);
                glVertex3f(c[0], c[1], c[2]);
            }
            glEnd();
            // Edges catch the light
            glDisable(0x0DE1);
            glColor4f(0.9, 0.97, 1.0, alpha * 0.75);
            glBegin(0x0002); // GL_LINE_LOOP
            for c in &corners {
                glVertex3f(c[0], c[1], c[2]);
            }
            glEnd();
        }
        glPointSize(2.5);
        glBegin(0x0000); // GL_POINTS
        for fl in &f.flecks {
            glColor4f(0.95, 0.98, 1.0, fl.alpha());
            glVertex3f(fl.pos[0], fl.pos[1], fl.pos[2]);
        }
        glEnd();
        glDisable(0x0BE2);
        glColor4f(1.0, 1.0, 1.0, 1.0);
    }
}

// Stable, distinguishable color for a material name
fn material_color(name: &str) -> (f32, f32, f32) {
    let h = name.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let c = |shift: u32| 0.35 + 0.65 * ((h >> shift) & 0xff) as f32 / 255.0;
    (c(0), c(8), c(16))
}

fn build_mesh(level: &map::Map) -> Mesh {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for object in &level.geometries.objects {
        let base = vertices.len() as u32;

        // Append object vertices
        for v in &object.vertices {
            let p = [v.x, v.y, v.z];
            for i in 0..3 {
                if p[i] < min[i] {
                    min[i] = p[i];
                }
                if p[i] > max[i] {
                    max[i] = p[i];
                }
            }
            vertices.push(p);
        }

        // Append triangle indices from all object data groups
        for od in &object.object_datas {
            for &(i0, i1, i2) in &od.faces.face_indices {
                indices.push([
                    base + i0 as u32,
                    base + i1 as u32,
                    base + i2 as u32,
                ]);
            }
        }
    }

    // Fallback if empty
    if vertices.is_empty() {
        return Mesh {
            vertices,
            indices,
            center: [0.0, 0.0, 0.0],
            radius: 10.0,
        };
    }

    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let dx = max[0] - center[0];
    let dy = max[1] - center[1];
    let dz = max[2] - center[2];
    let radius = (dx * dx + dy * dy + dz * dz).sqrt().max(1.0);

    Mesh {
        vertices,
        indices,
        center,
        radius,
    }
}

// Camera transformation for OpenGL 1.x
fn apply_camera(camera: &Camera) {
    unsafe {
        // Apply pitch (look up/down), then yaw (look left/right)
        glRotatef(-camera.pitch, 1.0, 0.0, 0.0);
        glRotatef(-camera.yaw, 0.0, 1.0, 0.0);
        glTranslatef(-camera.position[0], -camera.position[1], -camera.position[2]);
    }
}

 // Minimal gluPerspective replacement for OpenGL 1.x
fn glu_perspective(fov_y: f32, aspect: f32, z_near: f32, z_far: f32) {
    let f = 1.0 / (fov_y.to_radians() / 2.0).tan();
    let nf = 1.0 / (z_near - z_far);
    unsafe {
        glLoadIdentity();
        let mut mat = [0.0f32; 16];
        mat[0] = f / aspect;
        mat[5] = f;
        mat[10] = (z_far + z_near) * nf;
        mat[11] = -1.0;
        mat[14] = (2.0 * z_far * z_near) * nf;
        glMultMatrixf(mat.as_ptr());
    }
}


// 2D debug text (stroke lines) overlay

// Heads-up display: crosshair at the center, status lines bottom-left and a
// message under the crosshair. Leaves the 2D projection set; the debug text
// that follows re-enables depth testing.
fn draw_hud(win_w: i32, win_h: i32, status: &[String], message: &str, hit_marker: bool, veil: f32) {
    unsafe {
        glMatrixMode(0x1701); // GL_PROJECTION
        glLoadIdentity();
        ortho_2d(0.0, win_w.max(1) as f32, 0.0, win_h.max(1) as f32);
        glMatrixMode(0x1700); // GL_MODELVIEW
        glLoadIdentity();
        glDisable(0x0B71); // GL_DEPTH_TEST
        glDisable(0x0DE1); // GL_TEXTURE_2D

        // The white-out from a blast close by
        if veil > 0.01 {
            glEnable(0x0BE2); // GL_BLEND
            glBlendFunc(0x0302, 0x0303);
            glColor4f(1.0, 0.97, 0.92, veil.min(1.0) * 0.9);
            glBegin(0x0007); // GL_QUADS
            for (x, y) in [(0.0, 0.0), (win_w as f32, 0.0), (win_w as f32, win_h as f32), (0.0, win_h as f32)] {
                glVertex3f(x, y, 0.0);
            }
            glEnd();
            glDisable(0x0BE2);
        }

        let (cx, cy) = (win_w as f32 / 2.0, win_h as f32 / 2.0);
        if hit_marker { glColor3f(1.0, 0.2, 0.2); } else { glColor3f(1.0, 1.0, 1.0); }
        glBegin(0x0001); // GL_LINES
        for (dx0, dy0, dx1, dy1) in [(-12.0, 0.0, -4.0, 0.0), (4.0, 0.0, 12.0, 0.0), (0.0, -12.0, 0.0, -4.0), (0.0, 4.0, 0.0, 12.0)] {
            glVertex3f(cx + dx0, cy + dy0, 0.0);
            glVertex3f(cx + dx1, cy + dy1, 0.0);
        }
        glEnd();

        glColor3f(1.0, 1.0, 1.0);
        let scale = 14.0f32;
        let adv = scale * 0.8;
        for (i, line) in status.iter().enumerate() {
            let mut x = 12.0;
            for ch in line.chars() {
                draw_glyph(ch, x, 12.0 + i as f32 * scale * 1.5, scale);
                x += adv;
            }
        }
        if !message.is_empty() {
            glColor3f(1.0, 0.85, 0.3);
            let mut x = cx - adv * message.chars().count() as f32 / 2.0;
            for ch in message.chars() {
                draw_glyph(ch, x, cy - 50.0, scale);
                x += adv;
            }
        }
        glColor3f(1.0, 1.0, 1.0);
    }
}

/// A column-major 4x4 matrix times a vector
fn mul_mat4(m: &[f32; 16], v: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|row| (0..4).map(|col| m[col * 4 + row] * v[col]).sum())
}

/// Cut the drawing at these planes (a, b, c, d), keeping a*x + b*y + c*z + d >= 0, and
/// stop cutting at any plane not given
unsafe fn set_clip_planes(planes: &[[f32; 4]]) {
    unsafe {
        for i in 0..6u32 {
            match planes.get(i as usize) {
                Some(p) => {
                    glClipPlane(0x3000 + i, [p[0] as f64, p[1] as f64, p[2] as f64, p[3] as f64].as_ptr());
                    glEnable(0x3000 + i);
                }
                None => glDisable(0x3000 + i),
            }
        }
    }
}

/// Which of the level's objects to draw
#[derive(Clone, Copy)]
enum Which {
    All,
    /// The objects of one room
    Room(u32),
    /// Those that belong to no room
    Unowned,
}

/// How far away the door being opened or closed can be
const DOOR_REACH: f32 = 220.0;

/// Draw a room of the plan, then the rooms seen through its portals. `level` is the
/// stencil value of the part of the screen this room owns. Each portal's opening is
/// marked in the stencil buffer where it can be seen, the depth there is wiped, and the
/// room beyond is drawn only into the mark: whatever the room in front has past the
/// portal (rooms overlap each other) cannot show in the doorway.
unsafe fn draw_plan(node: &visibility::PlanNode<'_>, level: i32, fill_mode: u32, textured: bool, draw: &mut dyn FnMut(u32, &[[f32; 4]])) {
    const EQUAL: u32 = 0x0202;
    const KEEP: u32 = 0x1E00;
    unsafe {
        glStencilFunc(EQUAL, level, !0);
        glStencilOp(KEEP, KEEP, KEEP);
        // Cut where the portal it is seen through begins, and at the portals the player is
        // standing in (on this room's side). Where the player stands in a doorway both rooms
        // are drawn whole and each is cut only where it overlaps the other: what is on the
        // near side of the plane, or outside the other room's floor plan, stays. That is a
        // union, which clip planes cannot say, so the room is drawn once for each way to stay.
        let outside = |b: [f32; 4]| [[-1.0, 0.0, 0.0, b[0]], [1.0, 0.0, 0.0, -b[2]], [0.0, 0.0, -1.0, b[1]], [0.0, 0.0, 1.0, -b[3]]];
        let mut fixed: Vec<[f32; 4]> = Vec::new();
        let mut ways: Vec<Vec<[f32; 4]>> = Vec::new();
        match (node.plane, node.cut_in_self) {
            // Entered through a doorway the player stands in
            (Some(p), Some(b)) if node.touched => {
                ways.push(vec![p]);
                ways.extend(outside(b).into_iter().map(|o| vec![o]));
            }
            (Some(p), _) => fixed.push(p),
            _ => {}
        }
        for child in node.children.iter().filter(|c| c.touched) {
            let Some(p) = child.plane else { continue };
            let near = [-p[0], -p[1], -p[2], -p[3]];
            match child.cut_in_parent {
                Some(b) if ways.is_empty() => {
                    ways.push(vec![near]);
                    ways.extend(outside(b).into_iter().map(|o| vec![o]));
                }
                _ => fixed.push(near),
            }
        }
        if ways.is_empty() {
            fixed.truncate(6);
            draw(node.room, &fixed);
        }
        for way in &ways {
            let mut planes = fixed.clone();
            planes.extend(way);
            planes.truncate(6);
            draw(node.room, &planes);
        }

        for child in &node.children {
            // Standing in the doorway: both rooms are drawn, each cut at the portal's plane
            if child.touched {
                draw_plan(child, level, fill_mode, textured, draw);
                continue;
            }
            let opening = || {
                glBegin(0x0006); // GL_TRIANGLE_FAN
                for p in child.polygon {
                    glVertex3f(p[0], p[1], p[2]);
                }
                glEnd();
            };
            set_clip_planes(&[]);
            glDisable(0x0DE1); // GL_TEXTURE_2D
            glPolygonMode(0x0408, 0x1B02);
            glColorMask(0, 0, 0, 0);
            glDepthMask(0);

            // Mark the part of the opening that is not behind something nearer. The polygon
            // is nudged deeper, so walls lying in the portal's own plane are not wiped.
            glStencilFunc(EQUAL, level, !0);
            glStencilOp(KEEP, KEEP, 0x1E02); // GL_INCR
            glEnable(0x8037); // GL_POLYGON_OFFSET_FILL
            glPolygonOffset(2.0, 2.0);
            opening();
            glDisable(0x8037);
            // Wipe the depth there, so the room beyond competes only with itself
            glStencilFunc(EQUAL, level + 1, !0);
            glStencilOp(KEEP, KEEP, KEEP);
            glDepthFunc(0x0207); // GL_ALWAYS
            glDepthMask(1);
            glDepthRange(1.0, 1.0);
            opening();
            glDepthRange(0.0, 1.0);
            glDepthFunc(0x0201); // GL_LESS
            glColorMask(1, 1, 1, 1);
            glPolygonMode(0x0408, fill_mode);
            if textured {
                glEnable(0x0DE1);
            }

            draw_plan(child, level + 1, fill_mode, textured, draw);

            // The opening is done: nothing else may draw into it
            set_clip_planes(&[]);
            glColorMask(0, 0, 0, 0);
            glDepthMask(0);
            glDisable(0x0B71); // GL_DEPTH_TEST
            glPolygonMode(0x0408, 0x1B02);
            glStencilFunc(EQUAL, level + 1, !0);
            glStencilOp(KEEP, KEEP, 0x0000); // GL_ZERO
            opening();
            glEnable(0x0B71);
            glDepthMask(1);
            glColorMask(1, 1, 1, 1);
            glPolygonMode(0x0408, fill_mode);
            if textured {
                glEnable(0x0DE1);
            }
            glStencilOp(KEEP, KEEP, KEEP);
        }
    }
}

/// A stable color for a room's name, so the same room looks the same every run.
/// The rooms the player is standing in are drawn white.
fn room_color(name: &str, current: bool) -> [f32; 3] {
    if current {
        return [1.0, 1.0, 1.0];
    }
    let hash = name.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let hue = (hash % 360) as f32 / 60.0;
    let x = 1.0 - (hue % 2.0 - 1.0).abs();
    let (r, g, b) = match hue as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    [r * 0.8 + 0.1, g * 0.8 + 0.1, b * 0.8 + 0.1]
}

// What F2 records. Positions are in MAP space (the camera's Z is negated), except
// the `--pos` in the repro command, which takes the camera's own coordinates.
struct DebugReport<'a> {
    map_path: &'a Path,
    map_num: &'a str,
    level: &'a map::Map,
    eye: [f32; 3],
    feet: [f32; 3],
    yaw: f32,
    pitch: f32,
    dir: [f32; 3],
    hit: Option<(rogue_reborn::collision::RayHit, [f32; 3], &'a str)>,
    first_person: bool,
    third_person: bool,
    textured: bool,
    room_debug: bool,
    show_portals: bool,
    show_collisions: bool,
    fps: f32,
    render_ms: f32,
    culling: bool,
    /// The room the player is in
    room: Option<&'a str>,
    /// The rooms drawn this frame, when culling is on
    drawn: &'a [String],
    /// The rooms that can stop movement and bullets, when culling is on
    collision: &'a Option<Vec<rogue_reborn::collision::SolidRoom>>,
    /// Every room whose volume holds the player (rooms overlap)
    player_rooms: &'a [u32],
    /// The room drawing starts from (the floor's room)
    render_rooms: &'a [u32],
    /// Doors within reach, and how open they are
    doors: &'a [String],
}

// Write debug/dump-<time>.png and .txt; returns the path without extension
fn write_debug_dump(r: &DebugReport, rgb: &[u8], width: u32, height: u32) -> anyhow::Result<String> {
    use std::fmt::Write as _;
    std::fs::create_dir_all("debug")?;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let stem = format!("debug/dump-{secs}");
    rogue_reborn::png::write_png(Path::new(&format!("{stem}.png")), width, height, rgb)?;

    let rooms_at = |p: [f32; 3]| -> Vec<&str> {
        r.level.rooms.rooms.iter().filter(|room| room.contains(p, 50.0)).map(|room| room.section_name.as_str()).collect()
    };
    let in_rooms = rooms_at(r.feet);
    let mut t = String::new();
    writeln!(t, "map: {} ({})", r.map_path.display(), r.map_num)?;
    writeln!(t, "args: {}", std::env::args().skip(1).collect::<Vec<_>>().join(" "))?;
    writeln!(t, "mode: {}{}", if r.first_person { "first person" } else { "free fly" }, if r.third_person { ", third person view" } else { "" })?;
    writeln!(t, "toggles: textured={} room_colors={} portals={} collisions={}", r.textured, r.room_debug, r.show_portals, r.show_collisions)?;
    writeln!(t, "frame: {:.1} fps, {:.2} ms render", r.fps, r.render_ms)?;
    writeln!(t)?;
    writeln!(t, "eye (map space): {:.1}, {:.1}, {:.1}", r.eye[0], r.eye[1], r.eye[2])?;
    writeln!(t, "feet (map space): {:.1}, {:.1}, {:.1}", r.feet[0], r.feet[1], r.feet[2])?;
    writeln!(t, "yaw {:.1}, pitch {:.1}; looking along ({:.3}, {:.3}, {:.3})", r.yaw, r.pitch, r.dir[0], r.dir[1], r.dir[2])?;
    writeln!(t, "rooms whose volumes contain the feet: {in_rooms:?}")?;
    writeln!(t, "room the player is in: {}", r.room.unwrap_or("none"))?;
    writeln!(t, "rooms whose volumes hold the player (collide): {:?}", r.player_rooms)?;
    writeln!(t, "room drawing starts from (floor under the feet): {:?}", r.render_rooms)?;
    if r.culling {
        writeln!(t, "rooms drawn (visible through portals): {}", r.drawn.join(" "))?;
        let collide: Vec<String> = r.collision.as_deref().unwrap_or_default().iter()
            .map(|c| if c.keep.is_empty() && c.cut.is_empty() { c.room.to_string() } else { format!("{} (part)", c.room) })
            .collect();
        writeln!(t, "rooms that collide (\"part\" is cut at a portal): {}", collide.join(" "))?;
    } else {
        writeln!(t, "room culling is off: every room is drawn")?;
    }

    // Portals touching those rooms
    let numbers: Vec<u32> = r.room.and_then(|n| n.parse().ok()).into_iter().collect();
    let portals: Vec<String> = r.level.portals.portals.iter()
        .filter(|p| numbers.contains(&p.room) || numbers.contains(&p.opposite_room))
        .map(|p| format!("{} ({} -> {})", p.name, p.room, p.opposite_room))
        .collect();
    writeln!(t, "portals of that room: {}", if portals.is_empty() { "none".to_string() } else { portals.join(", ") })?;

    writeln!(t, "doors within 500 units: {}", if r.doors.is_empty() { "none".to_string() } else { format!("\n  {}", r.doors.join("\n  ")) })?;

    match &r.hit {
        Some((hit, point, material)) => {
            writeln!(t, "\nlooking at: {:.0} units away, point ({:.1}, {:.1}, {:.1}), normal ({:.2}, {:.2}, {:.2})",
                hit.dist, point[0], point[1], point[2], hit.normal[0], hit.normal[1], hit.normal[2])?;
            writeln!(t, "  material: {material}")?;
            writeln!(t, "  room whose geometry this is: {}", hit.room.map_or("none".to_string(), |n| n.to_string()))?;
            writeln!(t, "  rooms at that point: {:?}", rooms_at(*point))?;
        }
        None => writeln!(t, "\nlooking at: nothing within range")?,
    }

    // `--pos` takes the camera's own coordinates, with Z negated
    let mut repro = format!("cargo rr --map {} --pos {:.1},{:.1},{:.1} --yaw {:.1} --pitch {:.1}",
        r.map_path.display(), r.eye[0], r.eye[1], -r.eye[2], r.yaw, r.pitch);
    for (on, flag) in [(!r.culling, "--no-cull"), (r.first_person, "--first-person"), (r.textured, "--textured"), (r.room_debug, "--rooms"), (r.show_portals, "--portals"), (r.show_collisions, "--collisions")] {
        if on {
            repro.push(' ');
            repro.push_str(flag);
        }
    }
    writeln!(t, "\nreproduce: {repro}")?;
    writeln!(t, "git commit: {}", option_env!("GIT_HASH").unwrap_or("unknown"))?;
    std::fs::write(format!("{stem}.txt"), t)?;
    Ok(stem)
}

fn draw_debug_text(win_w: i32, win_h: i32, lines: &[&str]) {
    unsafe {
        // Ortho for pixel coordinates with origin at bottom-left
        glMatrixMode(0x1701); // GL_PROJECTION
        glLoadIdentity();
        ortho_2d(0.0, win_w.max(1) as f32, 0.0, win_h.max(1) as f32);
        glMatrixMode(0x1700); // GL_MODELVIEW
        glLoadIdentity();

        // Render settings for HUD
        glDisable(0x0B71); // GL_DEPTH_TEST
        glDisable(0x0DE1); // GL_TEXTURE_2D
        glColor3f(1.0, 1.0, 1.0);

        let margin = 10.0f32;
        let scale = 14.0f32;
        let line_h = scale * 1.5;
        let adv = scale * 0.8;

        let w = win_w.max(1) as f32;
        let h = win_h.max(1) as f32;

        for (i, text) in lines.iter().enumerate() {
            let y = h - margin - (i as f32) * line_h;
            let len = text.chars().count() as f32;
            let tw = adv * len;
            let mut x = w - margin - tw;
            for ch in text.chars() {
                draw_glyph(ch, x, y, scale);
                x += adv;
            }
        }

        // Leave depth state restored for next frame's 3D
        glEnable(0x0B71); // GL_DEPTH_TEST
    }
}

fn ortho_2d(left: f32, right: f32, bottom: f32, top: f32) {
    // Column-major orthographic matrix
    let rl = right - left;
    let tb = top - bottom;
    let mut m = [0.0f32; 16];
    m[0]  = 2.0 / rl;
    m[5]  = 2.0 / tb;
    m[10] = -1.0;
    m[12] = -(right + left) / rl;
    m[13] = -(top + bottom) / tb;
    m[15] = 1.0;
    unsafe { glMultMatrixf(m.as_ptr()); }
}

fn draw_glyph(ch: char, x: f32, y: f32, scale: f32) {
    // Return a 5x7 bitmap for supported glyphs (top row first, bit 4 = leftmost)
    fn glyph_bitmap(c: char) -> Option<[u8; 7]> {
        match c {
            // Digits 0-9
            '0' => Some([0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E]),
            '1' => Some([0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E]),
            '2' => Some([0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F]),
            '3' => Some([0x0E, 0x11, 0x01, 0x06, 0x01, 0x11, 0x0E]),
            '4' => Some([0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02]),
            '5' => Some([0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E]),
            '6' => Some([0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E]),
            '7' => Some([0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08]),
            '8' => Some([0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E]),
            '9' => Some([0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C]),
            // Letters we commonly render
            'L' => Some([0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F]),
            'E' => Some([0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F]),
            'V' => Some([0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04]),
            'F' => Some([0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10]),
            'P' => Some([0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10]),
            'S' => Some([0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E]),
            'R' => Some([0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11]),
            'N' => Some([0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11]),
            'D' => Some([0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E]),
            'M' => Some([0x11, 0x1B, 0x15, 0x11, 0x11, 0x11, 0x11]),
            'A' => Some([0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]),
            'B' => Some([0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E]),
            'C' => Some([0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E]),
            'G' => Some([0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F]),
            'H' => Some([0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]),
            'I' => Some([0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E]),
            'J' => Some([0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C]),
            'K' => Some([0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11]),
            'O' => Some([0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E]),
            'Q' => Some([0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D]),
            'T' => Some([0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
            'U' => Some([0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E]),
            'W' => Some([0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11]),
            'X' => Some([0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11]),
            'Y' => Some([0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04]),
            'Z' => Some([0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F]),
            '/' => Some([0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10]),
            '-' => Some([0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00]),
            ':' => Some([0x00, 0x04, 0x00, 0x00, 0x04, 0x00, 0x00]),
            '.' => Some([0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00]),
            ' ' => Some([0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
            _ => None,
        }
    }

    let cu = ch.to_ascii_uppercase();
    if let Some(rows) = glyph_bitmap(cu) {
        let px = scale / 7.0; // pixel size based on 7 rows
        let py = px;
        unsafe {
            glBegin(0x0007); // GL_QUADS
            for (row_idx, row_bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if ((row_bits >> (4 - col)) & 1) != 0 {
                        let vx = x + (col as f32) * px;
                        let vy = y + ((6 - row_idx) as f32) * py;
                        glVertex3f(vx, vy, 0.0);
                        glVertex3f(vx + px, vy, 0.0);
                        glVertex3f(vx + px, vy + py, 0.0);
                        glVertex3f(vx, vy + py, 0.0);
                    }
                }
            }
            glEnd();
        }
    } else {
        // Fallback: outline box to visualize unsupported chars
        unsafe {
            glBegin(0x0001); // GL_LINES
            glVertex3f(x, y, 0.0);                 glVertex3f(x + scale, y, 0.0);
            glVertex3f(x + scale, y, 0.0);         glVertex3f(x + scale, y + scale, 0.0);
            glVertex3f(x + scale, y + scale, 0.0); glVertex3f(x, y + scale, 0.0);
            glVertex3f(x, y + scale, 0.0);         glVertex3f(x, y, 0.0);
            glEnd();
        }
    }
}

// Texture utilities

fn ensure_texture(
    materials: &map::Materials,
    mat_index: usize,
    map_num: &str,
    cache: &mut HashMap<String, u32>,
) -> Option<u32> {
    if mat_index >= materials.materials.len() {
        return None;
    }
    let filename = materials.materials[mat_index].filename.clone();
    if let Some(id) = cache.get(&filename) {
        return Some(*id);
    }

    let resolved = resolve_texture_path(map_num, &filename)?;
    let rsb_tex = match rsb::read(&resolved) {
        Ok(t) => t,
        Err(_) => return None,
    };

    let tex_id = upload_rsb_to_gl(&rsb_tex)?;
    cache.insert(filename, tex_id);
    Some(tex_id)
}

// Find an on-disk RSB path by searching map-local and shared texture dirs,
// converting MAP's .bmp references to .RSB/.rsb.
fn resolve_texture_path(map_num: &str, material_filename: &str) -> Option<PathBuf> {
    // Preserve any subdirectories from the material filename (e.g., "faces/foo.bmp")
    let rel = Path::new(material_filename);

    // Texture name without extension (MAP refers to .bmp)
    let stem = rel.file_stem()?.to_string_lossy().into_owned();


    // File names differ in case from the MAP's (the data comes from a
    // case-insensitive filesystem), and .tga materials are stored as
    // "TGA<name>.rsb"
    let mut wanted = vec![format!("{stem}.rsb").to_lowercase()];
    if rel.extension().is_some_and(|e| e.eq_ignore_ascii_case("tga")) {
        wanted.push(format!("tga{stem}.rsb").to_lowercase());
    }
    let parent = rel.parent().unwrap_or(Path::new(""));

    for dir in [Path::new("data").join("map").join(map_num).join(parent), Path::new("data").join("texture").join(parent)] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let names: Vec<(String, PathBuf)> = entries.flatten()
            .map(|e| (e.file_name().to_string_lossy().to_lowercase(), e.path())).collect();
        for want in &wanted {
            if let Some((_, path)) = names.iter().find(|(n, _)| n == want) {
                return Some(path.clone());
            }
        }
    }

    None
}

// Extract the map number (e.g., "m00") from a map path like data/map/m00/citystreet_large.map
// Every level under data/map as (directory, name, path), sorted by directory then name.
// The name is the file name without .map, whatever its case.
fn list_levels() -> Vec<(String, String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir("data/map") else { return out };
    for dir in dirs.flatten().filter(|d| d.path().is_dir()) {
        let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
        for f in files.flatten() {
            let path = f.path();
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("map")) {
                let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                out.push((dir.file_name().to_string_lossy().into_owned(), name, path));
            }
        }
    }
    out.sort_by(|a, b| (a.0.to_lowercase(), a.1.to_lowercase()).cmp(&(b.0.to_lowercase(), b.1.to_lowercase())));
    out
}

// Find a level by name, ignoring case; "rm11" and "rm11/rm11" both work. The error
// says what was available.
fn find_level(name: &str) -> Result<PathBuf, String> {
    let levels = list_levels();
    let want = name.trim().trim_end_matches(".map").to_lowercase().replace('\\', "/");
    let found: Vec<&(String, String, PathBuf)> = levels.iter()
        .filter(|(dir, n, _)| want == n.to_lowercase() || want == format!("{}/{}", dir.to_lowercase(), n.to_lowercase()))
        .collect();
    match found.as_slice() {
        [one] => Ok(one.2.clone()),
        [] => {
            let names: Vec<&str> = levels.iter().map(|l| l.1.as_str()).collect();
            Err(format!("no level named {name:?}; available: {}", names.join(", ")))
        }
        many => {
            let paths: Vec<String> = many.iter().map(|l| l.2.display().to_string()).collect();
            Err(format!("{name:?} matches several levels, use --map: {}", paths.join(", ")))
        }
    }
}

fn extract_map_num(path: &Path) -> Option<String> {
    let mut comps = path.components().map(|c| c.as_os_str());
    while let Some(c) = comps.next() {
        if c == "map" {
            if let Some(next) = comps.next() {
                return Some(next.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn upload_rsb_to_gl(tex: &rsb::Rsb) -> Option<u32> {
    upload_rgba_to_gl(tex.width, tex.height, &tex.to_rgba8())
}

fn upload_rgba_to_gl(width: u32, height: u32, rgba: &[u8]) -> Option<u32> {
    if rgba.len() != (width * height * 4) as usize || rgba.is_empty() {
        return None;
    }

    let mut id = 0;
    unsafe {
        glGenTextures(1, &mut id as *mut u32);
        glBindTexture(0x0DE1, id); // GL_TEXTURE_2D

        // Simple filters; can be refined per material address mode
        glTexParameteri(0x0DE1, 0x2801, 0x2601); // GL_TEXTURE_MIN_FILTER, GL_LINEAR
        glTexParameteri(0x0DE1, 0x2800, 0x2601); // GL_TEXTURE_MAG_FILTER, GL_LINEAR

        glTexImage2D(
            0x0DE1,              // GL_TEXTURE_2D
            0,                   // level
            0x1908,              // GL_RGBA
            width as i32,
            height as i32,
            0,                   // border
            0x1908,              // GL_RGBA
            0x1401,              // GL_UNSIGNED_BYTE
            rgba.as_ptr() as *const std::ffi::c_void
        );
    }
    Some(id)
}
