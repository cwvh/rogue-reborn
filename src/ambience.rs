//! Ambient sound for a level, from the `.ass` file next to the MAP.
//!
//! ```text
//! AMBIENT_LOOPS
//!   l_iair.wav          a looping sound...
//!     2 55              ...at 55% volume while the player is in room "2"
//! AMBIENT_SPOT_SOUNDS
//!   1 5 15              in room "1", every 5 to 15 seconds play one of:
//!     a_crikt.wav 10 20   a sound at a volume of 10 to 20%
//! end
//! ```
//!
//! Rooms are named in the MAP (`Room::section_name`). Loops fade in and out as
//! the player moves between rooms.

use crate::sound::{Listener, Sound};
use rodio::Player;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, PartialEq)]
pub struct Loop {
    pub file: String,
    /// (room name, volume 0..=1)
    pub rooms: Vec<(String, f32)>,
}

#[derive(Debug, PartialEq)]
pub struct Spot {
    pub room: String,
    /// Seconds between sounds
    pub interval: (f32, f32),
    /// (file, minimum volume, maximum volume), volumes 0..=1
    pub sounds: Vec<(String, f32, f32)>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Definition {
    pub loops: Vec<Loop>,
    pub spots: Vec<Spot>,
}

/// Read a level's `.ass` file; `None` if it does not exist
pub fn read(path: &Path) -> Option<Definition> {
    Some(parse(&std::fs::read_to_string(path).ok()?))
}

pub fn parse(text: &str) -> Definition {
    #[derive(PartialEq)]
    enum Section {
        None,
        Loops,
        Spots,
    }
    let mut section = Section::None;
    let mut def = Definition::default();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(&first) = tokens.first() else { continue };
        match first.to_ascii_uppercase().as_str() {
            "AMBIENT_LOOPS" => { section = Section::Loops; continue; }
            "AMBIENT_SPOT_SOUNDS" => { section = Section::Spots; continue; }
            "END" => break,
            _ => {}
        }
        let is_sound = first.to_ascii_lowercase().ends_with(".wav");
        let num = |i: usize| tokens.get(i).and_then(|t| t.parse::<f32>().ok());
        match section {
            Section::Loops if is_sound => def.loops.push(Loop { file: first.to_string(), rooms: Vec::new() }),
            Section::Loops => {
                if let (Some(l), Some(v)) = (def.loops.last_mut(), num(1)) {
                    l.rooms.push((first.to_string(), v / 100.0));
                }
            }
            Section::Spots if is_sound => {
                if let (Some(s), Some(lo), Some(hi)) = (def.spots.last_mut(), num(1), num(2)) {
                    s.sounds.push((first.to_string(), lo / 100.0, hi / 100.0));
                }
            }
            Section::Spots => {
                if let (Some(lo), Some(hi)) = (num(1), num(2)) {
                    def.spots.push(Spot { room: first.to_string(), interval: (lo, hi), sounds: Vec::new() });
                }
            }
            Section::None => {}
        }
    }
    def
}

/// A level's ambience playing: tracks fading loops and spot sound timers
pub struct Ambience {
    def: Definition,
    /// Loop index -> (player, current gain)
    playing: HashMap<usize, (Player, f32)>,
    /// Spot index -> time of its next sound
    next_spot: HashMap<usize, f32>,
    rng: u64,
}

/// Seconds for a loop to fade from silent to full
const FADE_SECONDS: f32 = 1.5;
/// How far from the listener spot sounds are placed
const SPOT_DISTANCE: f32 = 700.0;

impl Ambience {
    pub fn new(def: Definition) -> Self {
        Self { def, playing: HashMap::new(), next_spot: HashMap::new(), rng: 0x9e3779b97f4a7c15 }
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Advance by `dt` seconds with the player in the rooms named `rooms`
    pub fn update(&mut self, snd: &mut Sound, rooms: &[&str], listener: Listener, now: f32, dt: f32) {
        for i in 0..self.def.loops.len() {
            let target = self.def.loops[i].rooms.iter()
                .filter(|(r, _)| rooms.contains(&r.as_str()))
                .map(|&(_, v)| v)
                .fold(0.0f32, f32::max);
            let current = self.playing.get(&i).map_or(0.0, |p| p.1);
            let step = dt / FADE_SECONDS;
            let gain = if target > current { (current + step).min(target) } else { (current - step).max(target) };
            if gain <= 0.0 {
                self.playing.remove(&i);
                continue;
            }
            let file = &self.def.loops[i].file;
            if !self.playing.contains_key(&i) {
                if let Some(player) = snd.play_loop(file, gain) {
                    self.playing.insert(i, (player, gain));
                }
            }
            if let Some((player, g)) = self.playing.get_mut(&i) {
                *g = gain;
                player.set_volume(snd.output_volume(file, gain));
            }
        }

        for i in 0..self.def.spots.len() {
            if !rooms.contains(&self.def.spots[i].room.as_str()) || self.def.spots[i].sounds.is_empty() {
                // Wait the full interval again after coming back into the room
                self.next_spot.remove(&i);
                continue;
            }
            let (lo, hi) = self.def.spots[i].interval;
            let due = match self.next_spot.get(&i) {
                Some(&t) => t,
                None => {
                    let t = now + lo + self.random() * (hi - lo);
                    self.next_spot.insert(i, t);
                    t
                }
            };
            if now < due {
                continue;
            }
            let (pick, vol_roll, angle, again) = (self.random(), self.random(), self.random(), self.random());
            let sounds = &self.def.spots[i].sounds;
            let (file, vmin, vmax) = &sounds[(pick * sounds.len() as f32) as usize % sounds.len()];
            let gain = vmin + vol_roll * (vmax - vmin);
            let a = angle * std::f32::consts::TAU;
            let at = [
                listener.pos[0] + a.cos() * SPOT_DISTANCE,
                listener.pos[1],
                listener.pos[2] + a.sin() * SPOT_DISTANCE,
            ];
            snd.play_at(file, at, listener, gain);
            self.next_spot.insert(i, now + lo + again * (hi - lo));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_loops_and_spots() {
        let def = parse("AMBIENT_LOOPS\r\n  l_iair.wav\r\n    2 55\r\n    400 30\r\n  a_traf1.wav\r\n    200 90\r\nAMBIENT_SPOT_SOUNDS\r\n  1 5 15\r\n    a_crikt.wav 10 20\r\nend\r\n");
        assert_eq!(def.loops.len(), 2);
        assert_eq!(def.loops[0].file, "l_iair.wav");
        assert_eq!(def.loops[0].rooms, vec![("2".to_string(), 0.55), ("400".to_string(), 0.3)]);
        assert_eq!(def.spots, vec![Spot {
            room: "1".into(),
            interval: (5.0, 15.0),
            sounds: vec![("a_crikt.wav".into(), 0.1, 0.2)],
        }]);
    }
}
