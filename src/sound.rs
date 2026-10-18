//! Sound playback for the game's WAV files in `data/sound`.
//!
//! Volumes and audible distances come from `soundvol.txt`, whose lines are
//! `NAME.WAV volume(0-100) min_distance max_distance` in map units: a sound is
//! at full volume inside the minimum distance and fades to silence at the maximum.

use rodio::buffer::SamplesBuffer;
use rodio::mixer::Mixer;
use rodio::source::Zero;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player, Source, SpatialPlayer};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

// A blast that reaches the player is felt before it is heard: a low thump in the chest, and
// then the world goes dull, as it does with ringing ears.

/// The highest sound that gets through when ears are fully stunned, and the highest when they
/// are fine, in hertz
const STUNNED_CUTOFF: f32 = 250.0;
const CLEAR_CUTOFF: f32 = 16000.0;

/// Seconds the ears stay as stunned as the blast made them, and the seconds after that over
/// which they come back
const STUN_HOLD: f32 = 4.0;
const STUN_ROLL_OFF: f32 = 4.0;

/// How stunned the ears are (0..1) `seconds` after a blast that stunned them `strength`: just
/// as much for a good while, then easing back to nothing
pub fn stunned_after(strength: f32, seconds: f32) -> f32 {
    let t = ((seconds - STUN_HOLD) / STUN_ROLL_OFF).clamp(0.0, 1.0);
    strength.clamp(0.0, 1.0) * (1.0 - t * t * (3.0 - 2.0 * t))
}

/// Where to cut off the high end when ears are `stunned` (0..1) of the way to deaf: all of it
/// at 0, falling steadily (by octaves, as the ear hears) to a dull rumble at 1
pub fn muffled_to(stunned: f32) -> f32 {
    CLEAR_CUTOFF * (STUNNED_CUTOFF / CLEAR_CUTOFF).powf(stunned.clamp(0.0, 1.0))
}

/// Everything that is heard, as the ears hear it: a source passed through a steep low-pass
/// (two one-pole filters in a row, 12 dB an octave) whose cutoff can be changed while it
/// plays, through `cutoff` (hertz, as `f32` bits). At or above `CLEAR_CUTOFF` it does nothing.
/// It is on the whole mix, so what was already playing is dulled as well as what comes next.
pub struct DullEars<S: Source> {
    inner: S,
    cutoff: Arc<AtomicU32>,
    rate: f32,
    channels: usize,
    channel: usize,
    state: Vec<[f32; 2]>,
    /// How far each filter moves toward its input each sample: 1 is not filtering at all
    reach: f32,
    until_check: u32,
}

impl<S: Source> DullEars<S> {
    pub fn new(inner: S, cutoff: Arc<AtomicU32>) -> DullEars<S> {
        let channels = inner.channels().get() as usize;
        DullEars { rate: inner.sample_rate().get() as f32, inner, cutoff, channels, channel: 0, state: vec![[0.0; 2]; channels], reach: 1.0, until_check: 0 }
    }
}

impl<S: Source> Iterator for DullEars<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let x = self.inner.next()?;
        let c = self.channel;
        self.channel = (c + 1) % self.channels;
        // The cutoff is read every so often, not every sample
        if c == 0 {
            if self.until_check == 0 {
                self.until_check = 64;
                let hz = f32::from_bits(self.cutoff.load(Ordering::Relaxed));
                self.reach = if hz >= CLEAR_CUTOFF { 1.0 } else { 1.0 - (-std::f32::consts::TAU * hz / self.rate).exp() };
            }
            self.until_check -= 1;
        }
        let s = &mut self.state[c];
        // Not filtering: keep the filters where the sound is, so that turning one on is no click
        if self.reach >= 1.0 {
            *s = [x, x];
            return Some(x);
        }
        s[0] += self.reach * (x - s[0]);
        s[1] += self.reach * (s[0] - s[1]);
        Some(s[1])
    }
}

impl<S: Source> Source for DullEars<S> {
    fn current_span_len(&self) -> Option<usize> { self.inner.current_span_len() }
    fn channels(&self) -> rodio::ChannelCount { self.inner.channels() }
    fn sample_rate(&self) -> rodio::SampleRate { self.inner.sample_rate() }
    fn total_duration(&self) -> Option<std::time::Duration> { self.inner.total_duration() }
}

/// A low thump that dies away over `seconds`: a tone dropping from about 80 Hz to 35 Hz, with
/// a harder click at the start and a harmonic so small speakers have something to play
pub fn thump_samples(rate: u32, seconds: f32) -> Vec<f32> {
    let mut phase = 0.0f32;
    (0..(seconds * rate as f32) as usize).map(|i| {
        let t = i as f32 / rate as f32;
        let hz = 35.0 + 45.0 * (-t / 0.07).exp();
        phase += std::f32::consts::TAU * hz / rate as f32;
        let attack = 1.0 - (-t / 0.003).exp();
        let body = (-t / (seconds * 0.3)).exp();
        (phase.sin() + 0.45 * (phase * 2.0).sin()) * attack * body * 0.7
    }).collect()
}

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
    // Where the sounds of the game go: it is mixed together and passed through `DullEars` on
    // its way to the device. The ringing and the thump go straight to the device.
    bus: Mixer,
    /// The cutoff of `DullEars`, in hertz as `f32` bits
    cutoff: Arc<AtomicU32>,
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
        // The bus takes the device's own format, so nothing is converted twice
        let (bus, mixed) = rodio::mixer::mixer(device.config().channel_count(), device.config().sample_rate());
        // A mixer with nothing in it ends, which would end the bus: keep a silence playing in it
        bus.add(Zero::new(device.config().channel_count(), device.config().sample_rate()));
        let cutoff = Arc::new(AtomicU32::new(CLEAR_CUTOFF.to_bits()));
        let to_device = Player::connect_new(device.mixer());
        to_device.append(DullEars::new(mixed, cutoff.clone()));
        to_device.detach();
        Some(Sound { device, bus, cutoff, dir, volumes, cache: HashMap::new(), master: 1.0 })
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

    /// How stunned the ears are, 0.0..=1.0: everything heard is duller as it rises
    pub fn set_muffle(&mut self, stunned: f32) {
        let hz = if stunned < 0.01 { CLEAR_CUTOFF } else { muffled_to(stunned) };
        self.cutoff.store(hz.to_bits(), Ordering::Relaxed);
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
        let player = Player::connect_new(&self.bus);
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
            &self.bus,
            [dir[0] * 2.0, dir[1] * 2.0, dir[2] * 2.0],
            [-r[0] * 0.1, -r[1] * 0.1, -r[2] * 0.1],
            [r[0] * 0.1, r[1] * 0.1, r[2] * 0.1],
        );
        player.set_volume(vol.gain * fade * gain * self.master);
        player.append(buf);
        player.detach();
    }

    /// A high whine that fades out over `seconds`: the ringing in the ears after a loud bang. It
    /// ignores `master`, which is turned down to muffle everything else at the same time.
    pub fn ring(&mut self, seconds: f32, gain: f32) {
        const RATE: u32 = 44100;
        let samples: Vec<f32> = (0..(seconds * RATE as f32) as usize).map(|i| {
            let t = i as f32 / RATE as f32;
            let fade = (1.0 - t / seconds).max(0.0).powi(2);
            ((t * 3400.0 * std::f32::consts::TAU).sin() * 0.6 + (t * 5200.0 * std::f32::consts::TAU).sin() * 0.25) * fade
        }).collect();
        let player = Player::connect_new(self.device.mixer());
        player.set_volume(gain);
        player.append(SamplesBuffer::new(std::num::NonZero::new(1).unwrap(), std::num::NonZero::new(RATE).unwrap(), samples));
        player.detach();
    }

    /// The thump of a blast, felt in the chest, `strength` (0..1) hard. Like the ringing, it
    /// ignores `master`, which is turned down for the rest.
    pub fn thump(&mut self, strength: f32) {
        const RATE: u32 = 44100;
        let player = Player::connect_new(self.device.mixer());
        player.set_volume(strength.clamp(0.0, 1.0));
        player.append(SamplesBuffer::new(std::num::NonZero::new(1).unwrap(), std::num::NonZero::new(RATE).unwrap(), thump_samples(RATE, 0.5)));
        player.detach();
    }

    /// Start a looping sound, which stops when the returned player is dropped
    pub fn play_loop(&mut self, name: &str, gain: f32) -> Option<Player> {
        let buf = self.load(name)?;
        let player = Player::connect_new(&self.bus);
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
    fn stunned_ears_hear_less_and_less_of_the_highs() {
        assert_eq!(muffled_to(0.0), CLEAR_CUTOFF);
        assert!((muffled_to(1.0) - STUNNED_CUTOFF).abs() < 0.5);
        assert!(muffled_to(0.5) < muffled_to(0.25) && muffled_to(0.25) < muffled_to(0.0));
    }

    #[test]
    fn stunned_ears_stay_that_way_for_the_hold_then_recover_smoothly() {
        assert_eq!(stunned_after(0.8, 0.0), 0.8);
        assert_eq!(stunned_after(0.8, STUN_HOLD - 0.1), 0.8);
        assert!(stunned_after(0.8, STUN_HOLD + 2.0) < 0.8 && stunned_after(0.8, STUN_HOLD + 2.0) > 0.0);
        assert_eq!(stunned_after(0.8, STUN_HOLD + STUN_ROLL_OFF), 0.0);
        assert_eq!(stunned_after(0.8, 60.0), 0.0);
        // Slowly at first, quickly in the middle, slowly at the end
        let drop = |a: f32, b: f32| stunned_after(1.0, STUN_HOLD + a) - stunned_after(1.0, STUN_HOLD + b);
        assert!(drop(0.0, 1.0) < drop(1.5, 2.5) && drop(3.0, 4.0) < drop(1.5, 2.5));
    }

    fn through_the_ears(samples: Vec<f32>, channels: u16, rate: u32, cutoff: f32) -> Vec<f32> {
        let buf = SamplesBuffer::new(std::num::NonZero::new(channels).unwrap(), std::num::NonZero::new(rate).unwrap(), samples);
        DullEars::new(buf, Arc::new(AtomicU32::new(cutoff.to_bits()))).collect()
    }

    #[test]
    fn dull_ears_keep_the_lows_and_take_the_highs_and_leave_a_clear_sound_alone() {
        let rate = 44100;
        let tone = |hz: f32| -> Vec<f32> { (0..rate).map(|i| (i as f32 / rate as f32 * hz * std::f32::consts::TAU).sin()).collect() };
        let loudest = |v: &[f32]| v[v.len() / 2..].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let low = through_the_ears(tone(100.0), 1, rate, 1000.0);
        let high = through_the_ears(tone(8000.0), 1, rate, 1000.0);
        assert!(loudest(&low) > 0.95, "{}", loudest(&low));
        assert!(loudest(&high) < 0.05, "{}", loudest(&high));
        assert_eq!(through_the_ears(tone(8000.0), 1, rate, CLEAR_CUTOFF), tone(8000.0));
        // Two channels are filtered on their own
        let stereo = through_the_ears(vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0], 2, rate, 1000.0);
        assert!(stereo.iter().skip(1).step_by(2).all(|&v| v == 0.0));
        assert!(stereo.iter().step_by(2).all(|&v| v > 0.0));
    }

    #[test]
    fn the_cutoff_can_change_while_a_sound_plays() {
        let rate = 44100;
        let buf = SamplesBuffer::new(std::num::NonZero::new(1).unwrap(), std::num::NonZero::new(rate).unwrap(),
            (0..rate * 2).map(|i| (i as f32 / rate as f32 * 8000.0 * std::f32::consts::TAU).sin()).collect::<Vec<f32>>());
        let cutoff = Arc::new(AtomicU32::new(CLEAR_CUTOFF.to_bits()));
        let mut ears = DullEars::new(buf, cutoff.clone());
        let first: Vec<f32> = ears.by_ref().take(rate as usize).collect();
        cutoff.store(500.0f32.to_bits(), Ordering::Relaxed);
        let second: Vec<f32> = ears.collect();
        let loudest = |v: &[f32]| v[v.len() / 2..].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(loudest(&first) > 0.9 && loudest(&second) < 0.02, "{} {}", loudest(&first), loudest(&second));
    }

    // The game's sounds are added to a bus that stays alive on a silence, and the whole of it is
    // heard through the ears
    #[test]
    fn what_is_added_to_the_bus_later_is_heard_dulled_live() {
        let (two, rate) = (std::num::NonZero::new(2u16).unwrap(), std::num::NonZero::new(8000u32).unwrap());
        let (bus, mixed) = rodio::mixer::mixer(two, rate);
        bus.add(Zero::new(two, rate));
        let cutoff = Arc::new(AtomicU32::new(CLEAR_CUTOFF.to_bits()));
        let mut ears = DullEars::new(mixed, cutoff.clone());
        // Nothing is playing: silence, and the bus does not end
        assert!(ears.by_ref().take(1000).all(|v| v == 0.0));
        // A sound, a brief one that ends, then silence again
        let tone: Vec<f32> = (0..4000).map(|i| (i as f32 / 8000.0 * 1500.0 * std::f32::consts::TAU).sin()).collect();
        bus.add(SamplesBuffer::new(two, rate, tone.clone()));
        let heard: Vec<f32> = ears.by_ref().take(2000).collect();
        assert!(heard.iter().fold(0.0f32, |m, v| m.max(v.abs())) > 0.9);
        assert!(ears.by_ref().take(5000).count() == 5000, "the bus ended");
        // And the same sound once the ears are stunned
        cutoff.store(300.0f32.to_bits(), Ordering::Relaxed);
        bus.add(SamplesBuffer::new(two, rate, tone));
        let dull: Vec<f32> = ears.take(2000).collect();
        let loudest = dull[500..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(loudest < 0.1, "{loudest}");
    }

    #[test]
    fn the_thump_is_low_hits_at_once_and_dies_away() {
        let rate = 44100;
        let t = thump_samples(rate, 0.5);
        assert_eq!(t.len(), rate as usize / 2);
        let loudest = |a: f32, b: f32| t[(a * rate as f32) as usize..(b * rate as f32) as usize].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(loudest(0.0, 0.05) > 0.4);
        assert!(loudest(0.4, 0.5) < 0.1 * loudest(0.0, 0.05));
        assert!(t.iter().all(|v| v.abs() <= 1.0));
        // Mostly low: the swing from one sample to the next is small
        let busy = t.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(busy < 0.1, "{busy}");
    }

    #[test]
    fn parses_volume_lines() {
        let v = parse_volumes("F_TWINT1.WAV 15 200 500\nbad line\n\nC_SB5K3.WAV 100 1000 10000\n");
        assert_eq!(v.len(), 2);
        let f = v["f_twint1.wav"];
        assert_eq!((f.gain, f.min, f.max), (0.15, 200.0, 500.0));
    }
}
