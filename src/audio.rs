use std::collections::HashMap;
use std::path::Path;
use rogue_reborn::{ambience, sound};
use rogue_reborn::{map, sim};
use crate::args::Args;
use crate::camera::listener_of;
use crate::view::PlayerView;

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

    // The sound the frame makes besides the events': reloads, full auto and the ambience
    pub fn play_frame(&mut self, sim: &sim::Sim, me: usize, level: &map::Map, pview: &PlayerView, showing: bool, game_time: f32, step_dt: f32) {
        // Reload sound when a reload starts (by key or running dry)
        if sim.players[me].reload_until.is_some() && !self.was_reloading {
            if let Some(snd) = self.sound.as_mut() {
                snd.play("C_SBMAG1.WAV", 1.0);
            }
        }
        self.was_reloading = sim.players[me].reload_until.is_some();

        // Full auto: the firing sound runs while the trigger is held with rounds to fire,
        // then stops with a closing crack
        let firing = sim.firing(me);
        if let Some(snd) = self.sound.as_mut() {
            if showing {
                self.firing_sound = None;
            } else if firing && self.firing_sound.is_none() {
                self.firing_sound = snd.play_loop("C_SB5KB.WAV", 1.0);
            } else if !firing && self.firing_sound.take().is_some() {
                snd.play("C_SB5KR.WAV", 1.0);
            }
        }

        // Ambient sound for the rooms around the player's feet
        if let (Some(snd), Some(amb)) = (self.sound.as_mut(), self.ambience.as_mut()) {
            let listener = listener_of(&pview.camera);
            let feet = [listener.pos[0], listener.pos[1] - pview.eye_now, listener.pos[2]];
            let rooms: Vec<&str> = level.rooms.rooms.iter()
                .filter(|r| r.contains(feet, 50.0))
                .map(|r| r.section_name.as_str())
                .collect();
            if showing {
                amb.silence();
            } else {
                amb.update(snd, &rooms, listener, game_time, step_dt);
            }
        }
    }
}
