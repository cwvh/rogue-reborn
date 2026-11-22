use std::time::Instant;
use crate::args::Args;

// What the keys switch on and off, and the dumps and the restart they ask for
pub struct Toggles {
    pub textured: bool,
    pub first_person: bool,
    pub third_person: bool,
    pub planning_level_index: usize,
    pub lag_meter: bool,
    pub show_portals: bool,
    pub show_collisions: bool,
    pub sprite_mode: u8,
    pub room_debug: bool,
    pub last_rooms: Vec<String>,
    pub dump_requested: bool,
    // F4: a dump of every frame for CLIP_SECONDS, in debug/clip-<time>/: when it started, the folder, the frames so far
    pub clip: Option<(std::time::Instant, String, u32)>,
    // F3 closes the window and starts the viewer again with the same options
    pub reload: bool,
    pub blink_enabled: bool,
    pub show_hitboxes: bool,
    // Where the third-person view sits round the player: with --cinematic, Shift and the mouse
    // move it, and it stays put when Shift is let go
    pub orbit: (f32, f32),
}

impl Toggles {
    pub fn new(args: &Args) -> Toggles {
        Toggles {
            textured: args.textured,
            first_person: args.first_person,
            third_person: args.third_person,
            planning_level_index: 0,
            lag_meter: args.lag_meter,
            show_portals: args.portals,
            show_collisions: args.collisions,
            sprite_mode: args.sprite_mode % crate::SPRITE_MODES.len() as u8,
            room_debug: args.rooms,
            last_rooms: Vec::new(),
            dump_requested: args.dump,
            clip: None,
            reload: false,
            blink_enabled: !args.no_blink,
            show_hitboxes: false,
            orbit: (args.view_orbit, 0.0),
        }
    }
}

// The frame clock and the count of frames
pub struct Timing {
    pub start_time: Instant,
    pub last_step: Instant,
    // A scripted run, or a replay, takes one tick a frame instead of keeping time
    pub fixed_step: bool,
    // Time the sim has yet to run
    pub tick_budget: f32,
    pub frames_drawn: u32,
    // Frames timed, summed render ms and summed whole-frame ms
    pub bench: (u32, f64, f64),
}

impl Timing {
    // The frame clock starts before the level's tables and the sound load, and the game clock after
    pub fn new(fixed_step: bool, last_step: Instant) -> Timing {
        Timing { start_time: Instant::now(), last_step, fixed_step, tick_budget: 0.0, frames_drawn: 0, bench: (0, 0.0, 0.0) }
    }
}

// Keys pressed since the sim last ticked, which counts once
#[derive(Default)]
pub struct Pressed {
    pub use_key: bool,
    pub reload: bool,
    pub gun: bool,
    pub grenade: bool,
    pub culling: bool,
}
