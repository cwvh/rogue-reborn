//! Sound playback for the game's WAV files in `data/sound`.
//!
//! Volumes and audible distances come from `soundvol.txt`, whose lines are
//! `NAME.WAV volume(0-100) min_distance max_distance` in map units: a sound is
//! at full volume inside the minimum distance and fades to silence at the maximum.

use rodio::buffer::SamplesBuffer;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player, SpatialPlayer};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
struct Volume {
    gain: f32,
    min: f32,
    max: f32,
}

/// Where the listener is, in map space
#[derive(Clone, Copy)]
pub struct Listener {
    pub pos: [f32; 3],
    /// Unit vector pointing to the listener's right
    pub right: [f32; 3],
}

pub struct Sound {
    // Keeps the output device open
    device: MixerDeviceSink,
    dir: PathBuf,
    volumes: HashMap<String, Volume>,
    cache: HashMap<String, Option<SamplesBuffer>>,
    /// Scales everything, 0.0..=1.0
    pub master: f32,
}

impl Sound {
    /// Open the default output device; `None` (with a message) when there is none
    pub fn open(dir: impl Into<PathBuf>) -> Option<Sound> {
        let dir = dir.into();
        let device = match DeviceSinkBuilder::open_default_sink() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("sound disabled: {e}");
                return None;
            }
        };
        let volumes = std::fs::read_to_string(dir.join("soundvol.txt")).map(|t| parse_volumes(&t)).unwrap_or_default();
        Some(Sound { device, dir, volumes, cache: HashMap::new(), master: 1.0 })
    }

    fn load(&mut self, name: &str) -> Option<SamplesBuffer> {
        let key = name.to_ascii_lowercase();
        if !self.cache.contains_key(&key) {
            let buf = load_wav(&self.dir.join(name));
            if buf.is_none() {
                eprintln!("sound {name}: could not load");
            }
            self.cache.insert(key.clone(), buf);
        }
        self.cache[&key].clone()
    }

    fn volume(&self, name: &str) -> Volume {
        self.volumes.get(&name.to_ascii_lowercase()).copied().unwrap_or(Volume { gain: 1.0, min: 300.0, max: 2000.0 })
    }

    /// The volume to give a player for `name` at `gain`: soundvol.txt and master applied
    pub fn output_volume(&self, name: &str, gain: f32) -> f32 {
        self.volume(name).gain * gain * self.master
    }

    /// Play a sound with no position (weapon, interface), `gain` scaling soundvol.txt
    pub fn play(&mut self, name: &str, gain: f32) {
        let Some(buf) = self.load(name) else { return };
        let player = Player::connect_new(self.device.mixer());
        player.set_volume(self.volume(name).gain * gain * self.master);
        player.append(buf);
        player.detach();
    }

    /// Play a sound at `at` (map space), panned and faded by distance from the listener
    pub fn play_at(&mut self, name: &str, at: [f32; 3], listener: Listener, gain: f32) {
        let vol = self.volume(name);
        let d = distance(at, listener.pos);
        if d >= vol.max {
            return;
        }
        let fade = if d <= vol.min { 1.0 } else { 1.0 - (d - vol.min) / (vol.max - vol.min) };
        let Some(buf) = self.load(name) else { return };

        // The mixer attenuates by distance itself, so keep the emitter at a fixed small
        // distance in the right direction and apply our own fade to the volume
        let dir = if d > 1e-3 { [(at[0] - listener.pos[0]) / d, (at[1] - listener.pos[1]) / d, (at[2] - listener.pos[2]) / d] } else { [0.0, 0.0, 1.0] };
        let r = listener.right;
        let player = SpatialPlayer::connect_new(
            self.device.mixer(),
            [dir[0] * 2.0, dir[1] * 2.0, dir[2] * 2.0],
            [-r[0] * 0.1, -r[1] * 0.1, -r[2] * 0.1],
            [r[0] * 0.1, r[1] * 0.1, r[2] * 0.1],
        );
        player.set_volume(vol.gain * fade * gain * self.master);
        player.append(buf);
        player.detach();
    }

    /// Start a looping sound, which stops when the returned player is dropped
    pub fn play_loop(&mut self, name: &str, gain: f32) -> Option<Player> {
        use rodio::Source;
        let buf = self.load(name)?;
        let player = Player::connect_new(self.device.mixer());
        player.set_volume(self.output_volume(name, gain));
        player.append(buf.repeat_infinite());
        Some(player)
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn parse_volumes(text: &str) -> HashMap<String, Volume> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(name), Some(v), Some(min), Some(max)) = (it.next(), it.next(), it.next(), it.next()) else { continue };
        let (Ok(v), Ok(min), Ok(max)) = (v.parse::<f32>(), min.parse::<f32>(), max.parse::<f32>()) else { continue };
        out.insert(name.to_ascii_lowercase(), Volume { gain: v / 100.0, min, max: max.max(min + 1.0) });
    }
    out
}

fn load_wav(path: &Path) -> Option<SamplesBuffer> {
    use rodio::Source;
    let decoder = rodio::Decoder::try_from(File::open(path).ok()?).ok()?;
    let (channels, rate) = (decoder.channels(), decoder.sample_rate());
    let samples: Vec<_> = decoder.collect();
    Some(SamplesBuffer::new(channels, rate, samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_volume_lines() {
        let v = parse_volumes("F_TWINT1.WAV 15 200 500\nbad line\n\nC_SB5K3.WAV 100 1000 10000\n");
        assert_eq!(v.len(), 2);
        let f = v["f_twint1.wav"];
        assert_eq!((f.gain, f.min, f.max), (0.15, 200.0, 500.0));
    }
}
