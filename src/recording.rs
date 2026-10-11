//! Recorded games (demos).
//!
//! The sim is deterministic, so a game is fully described by how it began and the `Input` of
//! every tick. A recording is that, plus a checksum of the sim every second, so that playing
//! it back can tell the moment it stops matching what was recorded (a changed rule, a changed
//! data file, a different build).
//!
//! The file is binary and little-endian: a header, then the inputs, then the checksums. An input
//! is stored as what changed since the one before: a byte says which of the buttons and the two
//! look angles changed, and then come the new buttons and the angle changes, which are small
//! (the mouse moves a little each tick), so a tick with the mouse still costs one byte.

use std::path::Path;

use anyhow::{Context, Result, ensure};

use crate::bytes::{Before, Reader, Writer};
use crate::sim::{Input, Sim, TICK};

/// Every this many ticks, the sim's checksum is recorded
pub const CHECK_EVERY: u32 = 60;

const MAGIC: &[u8; 8] = b"RRDEMO\0\x02";

/// How a game began: enough to set up the same sim again
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    /// The build that recorded it, which may not play it back the same if it differs
    pub build: String,
    /// The level's MAP file, or "demo" for the built-in box room
    pub map: String,
    /// How many characters stand in the box room
    pub npcs: u32,
    /// Which spawn point is the player's, counting from 0, so that the others hold characters
    pub own_spawn: Option<u32>,
    /// Where the view starts (camera space) and which way it looks
    pub camera: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub seed: u64,
    pub magazine: u32,
    pub ammo: u32,
    pub reserve: u32,
    pub grenades: u32,
    pub speed_scale: f32,
    pub culling: bool,
    pub doors_open: Option<f32>,
    /// The character model, or none with `no_models`
    pub model: String,
    pub no_models: bool,
    pub model_yaw: Option<f32>,
    pub actor: Option<String>,
}

/// A game: how it began, what the player did each tick, and checksums to check a replay by
#[derive(Clone, Debug, PartialEq)]
pub struct Recording {
    pub header: Header,
    pub inputs: Vec<Input>,
    /// (ticks run, checksum of the sim after them)
    pub checks: Vec<(u32, u64)>,
}

/// Where a replay stopped matching the recording
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desync {
    pub tick: u32,
    pub recorded: u64,
    pub found: u64,
}

impl std::fmt::Display for Desync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the game stopped matching the recording at tick {} (checksum {:016x}, recorded {:016x})", self.tick, self.found, self.recorded)
    }
}

impl Recording {
    /// Play every input into a sim that was set up from the header, checking the checksums.
    /// Returns where it first stopped matching, if it did.
    pub fn replay(&self, sim: &mut Sim) -> Result<(), Desync> {
        let mut playback = Playback::new(self.clone());
        while let Some(input) = playback.next_input() {
            sim.tick(&[input], TICK);
            if let Some(d) = playback.after_tick(sim) {
                return Err(d);
            }
        }
        match playback.desync() {
            Some(d) => Err(d),
            None => Ok(()),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        w.0.extend_from_slice(MAGIC);
        let h = &self.header;
        w.string(&h.build);
        w.string(&h.map);
        w.u32(h.npcs);
        w.opt_u32(h.own_spawn);
        h.camera.iter().for_each(|&c| w.f32(c));
        w.f32(h.yaw);
        w.f32(h.pitch);
        w.u64(h.seed);
        w.u32(h.magazine);
        w.u32(h.ammo);
        w.u32(h.reserve);
        w.u32(h.grenades);
        w.f32(h.speed_scale);
        w.u8(h.culling as u8);
        w.opt_f32(h.doors_open);
        w.string(&h.model);
        w.u8(h.no_models as u8);
        w.opt_f32(h.model_yaw);
        w.string(h.actor.as_deref().unwrap_or(""));
        w.u8(h.actor.is_some() as u8);

        w.u32(self.inputs.len() as u32);
        let mut before = Before::default();
        for i in &self.inputs {
            w.input(i, &mut before);
        }
        w.u32(self.checks.len() as u32);
        for &(tick, sum) in &self.checks {
            w.u32(tick);
            w.u64(sum);
        }
        w.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Recording> {
        let mut r = Reader { bytes, at: 0 };
        ensure!(r.take(MAGIC.len())? == MAGIC, "not a recording (or one from another version)");
        let build = r.string()?;
        let map = r.string()?;
        let npcs = r.u32()?;
        let own_spawn = r.opt_u32()?;
        let camera = [r.f32()?, r.f32()?, r.f32()?];
        let (yaw, pitch) = (r.f32()?, r.f32()?);
        let seed = r.u64()?;
        let (magazine, ammo, reserve, grenades) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
        let speed_scale = r.f32()?;
        let culling = r.u8()? != 0;
        let doors_open = r.opt_f32()?;
        let model = r.string()?;
        let no_models = r.u8()? != 0;
        let model_yaw = r.opt_f32()?;
        let actor_name = r.string()?;
        let actor = (r.u8()? != 0).then_some(actor_name);
        let header = Header {
            build, map, npcs, own_spawn, camera, yaw, pitch, seed, magazine, ammo, reserve, grenades,
            speed_scale, culling, doors_open, model, no_models, model_yaw, actor,
        };

        let n = r.u32()? as usize;
        let mut inputs = Vec::with_capacity(n.min(1 << 20));
        let mut before = Before::default();
        for _ in 0..n {
            inputs.push(r.input(&mut before)?);
        }
        let n = r.u32()? as usize;
        let mut checks = Vec::with_capacity(n.min(1 << 20));
        for _ in 0..n {
            checks.push((r.u32()?, r.u64()?));
        }
        ensure!(r.at == bytes.len(), "{} bytes left over after the recording", bytes.len() - r.at);
        Ok(Recording { header, inputs, checks })
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_bytes()).with_context(|| format!("could not write {}", path.display()))
    }

    pub fn read(path: &Path) -> Result<Recording> {
        let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
        Recording::from_bytes(&bytes).with_context(|| format!("{}", path.display()))
    }

    /// How long the game ran, in seconds
    pub fn seconds(&self) -> f32 {
        self.inputs.len() as f32 * TICK
    }
}

/// Makes a recording as a game is played
pub struct Recorder {
    rec: Recording,
}

impl Recorder {
    pub fn new(header: Header) -> Recorder {
        Recorder { rec: Recording { header, inputs: Vec::new(), checks: Vec::new() } }
    }

    /// Note a tick that has just run with `input`
    pub fn after_tick(&mut self, input: &Input, sim: &Sim) {
        self.rec.inputs.push(*input);
        let ticks = self.rec.inputs.len() as u32;
        if ticks % CHECK_EVERY == 0 {
            self.rec.checks.push((ticks, sim.checksum()));
        }
    }

    pub fn ticks(&self) -> usize {
        self.rec.inputs.len()
    }

    /// The finished recording, with a last checksum where the game ended
    pub fn finish(mut self, sim: &Sim) -> Recording {
        let ticks = self.rec.inputs.len() as u32;
        if ticks % CHECK_EVERY != 0 {
            self.rec.checks.push((ticks, sim.checksum()));
        }
        self.rec
    }
}

/// Feeds a recording's inputs to a sim, one a tick, checking its checksums
pub struct Playback {
    rec: Recording,
    next: usize,
    next_check: usize,
    desync: Option<Desync>,
}

impl Playback {
    pub fn new(rec: Recording) -> Playback {
        Playback { rec, next: 0, next_check: 0, desync: None }
    }

    pub fn header(&self) -> &Header {
        &self.rec.header
    }

    /// The input for the next tick, or none when the recording is over
    pub fn next_input(&mut self) -> Option<Input> {
        let input = self.rec.inputs.get(self.next).copied();
        self.next += input.is_some() as usize;
        input
    }

    /// Ticks run so far, and in all
    pub fn progress(&self) -> (usize, usize) {
        (self.next, self.rec.inputs.len())
    }

    pub fn finished(&self) -> bool {
        self.next >= self.rec.inputs.len()
    }

    /// Where the replay first stopped matching the recording, if it has
    pub fn desync(&self) -> Option<Desync> {
        self.desync
    }

    /// Check the sim after a tick against the recording. Returns the first mismatch, once.
    pub fn after_tick(&mut self, sim: &Sim) -> Option<Desync> {
        let ticks = self.next as u32;
        let mut found = None;
        while let Some(&(tick, sum)) = self.rec.checks.get(self.next_check) {
            if tick > ticks {
                break;
            }
            self.next_check += 1;
            if tick == ticks && sum != sim.checksum() && self.desync.is_none() {
                let d = Desync { tick, recorded: sum, found: sim.checksum() };
                self.desync = Some(d);
                found = Some(d);
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{CollisionWorld, Params};
    use crate::sim::Config;
    use std::sync::Arc;

    fn header() -> Header {
        Header {
            build: "abc1234".to_string(), map: "demo".to_string(), npcs: 8, own_spawn: Some(0),
            camera: [0.0, 165.0, 350.0], yaw: 12.5, pitch: -3.0, seed: 7, magazine: 30, ammo: 30,
            reserve: 90, grenades: 3, speed_scale: 1.0, culling: true, doors_open: Some(0.5),
            model: "data/character/team_hrt/blue_M_a.CRP".to_string(), no_models: false, model_yaw: None,
            actor: Some("ding".to_string()),
        }
    }

    fn demo_sim() -> Sim {
        let level = Arc::new(crate::demo::map());
        let world = CollisionWorld::from_map(&level, Params::default());
        let config = Config { magazine: 30, ammo: 30, reserve: 90, grenades: 3, speed_scale: 1.0, seed: 7, culling: true };
        let mut sim = Sim::new(level, world, config, None, Vec::new());
        sim.add_player([0.0, 0.0, -350.0]);
        sim
    }

    /// Play a made-up game, recording it
    fn play() -> (Recording, u64) {
        let mut sim = demo_sim();
        let mut recorder = Recorder::new(header());
        for tick in 0..400u32 {
            let mut input = Input {
                forward: tick % 120 < 70,
                right: tick % 50 < 10,
                run: true,
                fire: tick % 100 < 40,
                grenade: tick == 150,
                use_key: tick == 10,
                toggle_culling: tick == 200,
                observer: (tick >= 300 && tick < 330).then_some([1.0, 150.0, -100.0 - tick as f32]),
                ..Input::default()
            };
            input.set_look((tick as f32 * 0.05).sin() * 60.0, 5.0);
            sim.tick(&[input], TICK);
            recorder.after_tick(&input, &sim);
        }
        let sum = sim.checksum();
        (recorder.finish(&sim), sum)
    }

    #[test]
    fn a_recording_survives_being_written_and_read() {
        let (rec, _) = play();
        let back = Recording::from_bytes(&rec.to_bytes()).unwrap();
        assert_eq!(back, rec);
        // Seven checksums a second apart, and the last at the end
        assert_eq!(rec.checks.len(), 7);
        assert_eq!(rec.checks.last().unwrap().0, 400);
    }

    #[test]
    fn a_recording_is_small() {
        let (rec, _) = play();
        // The mouse sweeps all the time here, and still it is a few bytes a tick
        let size = rec.to_bytes().len();
        assert!(size < 400 * 4 + 300, "{size} bytes");
    }

    #[test]
    fn a_still_mouse_and_steady_buttons_cost_a_byte_a_tick() {
        let mut rec = play().0;
        rec.inputs = vec![Input { forward: true, run: true, ..Input::default() }; 1000];
        rec.checks.clear();
        let with = rec.to_bytes().len();
        rec.inputs.clear();
        assert_eq!(with - rec.to_bytes().len(), 1000 + 2);
    }

    #[test]
    fn angles_survive_any_mouse_path_including_going_round() {
        let mut rec = play().0;
        // Spins, sudden flicks of half a turn, and rests, in both directions
        let path = [0.0, 0.01, 359.99, 0.0, 180.0, 359.0, 1.0, 270.0, 90.0, 90.0, 0.0, 359.9945, 180.0];
        rec.inputs = path.iter().flat_map(|&a| [a, -a]).map(|a| {
            let mut i = Input::default();
            i.set_look(a, (a / 4.0 - 40.0).clamp(-89.0, 89.0));
            i
        }).collect();
        rec.checks.clear();
        let back = Recording::from_bytes(&rec.to_bytes()).unwrap();
        assert_eq!(back.inputs, rec.inputs);
    }

    #[test]
    fn angles_are_kept_to_a_65536th_of_a_turn() {
        use crate::sim::{dequantize, quantize};
        for degrees in [0.0, 0.003, 12.5, 90.0, 179.99, 359.99, -1.0, -90.0, 725.0] {
            let back = dequantize(quantize(degrees));
            let off = (back - degrees.rem_euclid(360.0)).abs();
            assert!(off.min(360.0 - off) <= 0.0028, "{degrees} came back as {back}");
        }
        // Right angles are exact, and a full turn is no turn
        assert_eq!(dequantize(quantize(90.0)), 90.0);
        assert_eq!(quantize(360.0), 0);
        assert_eq!(quantize(-90.0), 49152);
        let mut i = Input::default();
        i.set_look(45.0, -45.0);
        assert_eq!(i.look(), (45.0, -45.0));
    }

    #[test]
    fn playing_it_back_gives_the_same_game_all_the_way() {
        let (rec, sum) = play();
        let mut sim = demo_sim();
        assert_eq!(rec.replay(&mut sim), Ok(()));
        assert_eq!(sim.checksum(), sum);
    }

    #[test]
    fn a_changed_input_is_caught_at_the_next_checkpoint() {
        let (mut rec, _) = play();
        // Stop firing at tick 105: the game goes its own way from there
        rec.inputs[105].fire = false;
        rec.inputs[106].fire = false;
        rec.inputs[107].fire = false;
        let mut sim = demo_sim();
        let d = rec.replay(&mut sim).unwrap_err();
        assert_eq!(d.tick, 120);
    }

    #[test]
    fn damaged_files_are_refused() {
        let (rec, _) = play();
        let bytes = rec.to_bytes();
        assert!(Recording::from_bytes(&bytes[..bytes.len() - 3]).is_err());
        assert!(Recording::from_bytes(b"not a demo at all").is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(Recording::from_bytes(&longer).is_err());
    }
}
