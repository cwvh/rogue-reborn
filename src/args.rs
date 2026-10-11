//! Command-line options; mainly for repeatable headless screenshots.

use std::path::PathBuf;
use rogue_reborn::impact::Surface;

// Command-line options; mainly for repeatable headless screenshots
#[derive(Default)]
pub struct Args {
    /// A level by name, such as rm11 or citystreet_large, found under data/map
    pub level: Option<String>,
    pub list_levels: bool,
    /// Record the game to this file, to play back with --play
    pub record: Option<PathBuf>,
    /// Play back a recorded game, which starts as it did when recorded
    pub play: Option<PathBuf>,
    /// The built-in box room instead of a level from the game
    pub demo: bool,
    /// Characters standing in the demo room (0 to 8, default 8)
    pub npcs: Option<usize>,
    pub map: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
    pub pos: Option<[f32; 3]>,
    pub yaw: Option<f32>,
    pub pitch: Option<f32>,
    pub textured: bool,
    pub portals: bool,
    /// Colour each room's geometry and report the rooms the player is in
    pub rooms: bool,
    /// Write the F2 debug dump on the first frame and exit
    pub dump: bool,
    /// Draw every room instead of only the ones visible through portals
    pub no_cull: bool,
    /// Start with every door this far open (0 shut, 1 open)
    pub doors_open: Option<f32>,
    /// Rooms to leave out of the drawing, to find which one a stray surface belongs to
    pub hide_rooms: Vec<u32>,
    pub collisions: bool,
    pub first_person: bool,
    pub info: bool,
    pub spawn_zone: Option<String>,
    pub seed: Option<u64>,
    pub model: Option<PathBuf>,
    pub no_models: bool,
    pub no_sound: bool,
    /// Master volume, 0.0..=1.0
    pub volume: Option<f32>,
    pub actor: Option<String>,
    pub model_yaw: Option<f32>,
    pub no_blink: bool,
    pub eyes_closed: bool,
    pub anim: Option<String>,
    pub anim_fps: Option<f32>,
    /// Frame rate cap (also turns vsync off)
    pub max_fps: Option<f32>,
    pub anim_frame: Option<f32>,
    // Mission spawn point to use, counting from 1
    pub spawn: Option<usize>,
    pub speed_scale: Option<f32>,
    // Multiplies the playback rate of the movement animations (default 1.5)
    pub anim_scale: Option<f32>,
    // Overrides for the player's movement set: slot=motion (walk, run, back, left, right, idle, fire, reload)
    pub player_anims: Vec<(String, String)>,
    pub third_person: bool,
    pub weapon_offset: Option<[f32; 3]>,
    /// Draw the weapon in front of the camera in first person (the original game had no such view)
    pub viewmodel: bool,
    /// Bullet holes drawn for the surface hit, with splinters, sparks and dust, instead of the game sprites
    pub realistic_decals: bool,
    /// Treat every wall as this surface (wood, metal, concrete, earth, soft), to preview bullet holes
    pub decal_surface: Option<Surface>,
    /// Where the held weapon sits in the view, in gun lengths: right, up, forward
    pub viewmodel_offset: Option<[f32; 3]>,
    pub view_orbit: f32,
    pub view_dist: Option<f32>,
    pub hold: Option<String>,
    // Hold W for this many frames and print the position trace
    pub walk: Option<u32>,
}

pub fn parse_args() -> Args {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--map" => args.map = it.next().map(PathBuf::from),
            "--level" => args.level = it.next(),
            "--list-levels" => args.list_levels = true,
            "--record" => args.record = it.next().map(PathBuf::from),
            "--play" => args.play = it.next().map(PathBuf::from),
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
