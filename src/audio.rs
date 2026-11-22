use std::collections::HashMap;
use std::path::Path;
use rogue_reborn::{ambience, sound};
use crate::args::Args;

// Everything the game plays: the sound device, and what the frames remember to play next. The
// fields drop in this order, which is the order the loose locals dropped in.
pub struct Audio {
    // Full auto: a sustained firing sound that runs while the trigger is held
    pub firing_sound: Option<rodio::Player>,
    // The level's ambient loops and spot sounds, which follow the rooms the player is in
    pub ambience: Option<ambience::Ambience>,
    pub was_reloading: bool,
    pub step_phase: i32,
    pub shell_sound_at: HashMap<usize, f32>,
    pub master_volume: f32,
    // Sound is optional: no output device just means silence
    pub sound: Option<sound::Sound>,
}

impl Audio {
    pub fn new(args: &Args, map_path: &Path) -> Audio {
        let mut sound = if args.no_sound { None } else { sound::Sound::open("data/sound") };
        if let Some(snd) = sound.as_mut() {
            snd.master = args.volume.unwrap_or(1.0).clamp(0.0, 1.0);
        }
        let ambience = ambience::read(&map_path.with_extension("ass"))
            .map(ambience::Ambience::new)
            .filter(|_| sound.is_some());
        Audio {
            firing_sound: None,
            ambience,
            was_reloading: false,
            step_phase: 0,
            shell_sound_at: HashMap::new(),
            master_volume: args.volume.unwrap_or(1.0).clamp(0.0, 1.0),
            sound,
        }
    }
}
