//! The characters standing in the level: where they are, how they are posed for hit
//! detection, and the model they share.

use std::collections::HashMap;
use std::path::Path;

use crate::actor::Actor;
use crate::collision::CollisionWorld;
use crate::combat::{self, Health, NpcState, Placement};
use crate::crp::{self, Crp, Skin};
use crate::motion::{self, Motion};
use crate::skeleton::{self, Skeleton};
use crate::util::find_file;

type V3 = [f32; 3];

/// A character in the level
#[derive(Clone)]
pub struct Npc {
    pub health: Health,
    /// The model's origin in map space
    pub pos: V3,
    /// Where its feet are
    pub floor: f32,
    /// Degrees about the vertical axis
    pub yaw: f32,
    pub scale: V3,
    /// Staggers each character's idle animation and blinking, so they do not all move as one
    pub stagger: f32,
}

/// A skeleton and the skin that follows it
pub struct Rig {
    pub skeleton: Skeleton,
    pub skin: Skin,
}

/// The model all the characters share, and the motions they play
pub struct Characters {
    pub model: Crp,
    /// Without a skeleton the model stays in its bind pose
    pub rig: Option<Rig>,
    /// By name: the idle motion, injury reactions and deaths
    pub motions: HashMap<String, Motion>,
    /// The name of the idle motion, which also stands in for any that is missing
    pub idle: String,
}

/// The motions characters play besides idling: flinches from wounds and the falls they die with
pub const REACTION_MOTIONS: [&str; 29] = [
    "thwu_bf", "thwu_bl", "thwu_bb", "thwu_br", "thwu_la", "thwu_ra", "thwu_ll", "thwu_rl",
    "thdu_1", "thdu_2", "thdu_3", "thdu_4", "thdu_e1", "thdu_e2",
    "hw_fr_up2_g", "hw_lf_up2_g", "hw_bk_up2_g", "hw_bk_up2", "hw_rt_up2",
    "thiu_cf", "thiu_cl", "thiu_cb", "thiu_ab", "thiu_hf", "thiu_hb", "thiu_la", "thiu_ra", "thiu_ll", "thiu_rl",
];

/// A motion from the game's data, by name
pub fn load_motion(name: &str) -> Option<Motion> {
    motion::read(&find_file(Path::new("data/motion"), name, "dam").or_else(|| { eprintln!("no motion named {name}"); None })?).ok()
}

impl Characters {
    /// Read the model, its skeleton and the motions from the game's data. None when the model
    /// cannot be read. `idle` names the motion characters idle with.
    pub fn load(model_path: &Path, idle: &str) -> Option<Characters> {
        let model = match crp::read(model_path) {
            Ok(c) => {
                println!("loaded {} ({} vertices, {} triangles)", c.name, c.vertices.len(),
                    c.parts.iter().map(|p| p.triangles.len()).sum::<usize>());
                c
            }
            Err(e) => {
                eprintln!("could not load {}: {e:#}", model_path.display());
                return None;
            }
        };
        // Skeleton and skinning for posed models
        let rig = (|| {
            let dir = model_path.parent()?;
            let stem = model_path.file_stem()?.to_string_lossy().to_string();
            let skeleton = skeleton::read(&find_file(dir, &stem, "skl").or_else(|| { eprintln!("no skeleton beside {}", model_path.display()); None })?).ok()?;
            let skin = model.skin(&skeleton).or_else(|| { eprintln!("mesh does not match skeleton"); None })?;
            Some(Rig { skeleton, skin })
        })();
        let mut motions = HashMap::new();
        for name in std::iter::once(idle).chain(REACTION_MOTIONS) {
            if let Some(m) = load_motion(name) {
                motions.insert(name.to_string(), m);
            }
        }
        Some(Characters { model, rig, motions, idle: idle.to_string() })
    }

    /// How many frames a motion lasts, one if it is not loaded
    pub fn frames(&self, motion: &str) -> f32 {
        self.motions.get(motion).map_or(1, |m| m.frames) as f32
    }

    /// Where a character stands, for shots and blasts
    pub fn placement(&self, npc: &Npc) -> Placement {
        Placement {
            pos: [npc.pos[0], if self.rig.is_some() { npc.floor } else { npc.pos[1] }, npc.pos[2]],
            yaw_deg: npc.yaw,
            scale: npc.scale,
        }
    }

    /// The frame an idle character is on at `now`
    pub fn idle_frame(npc: &Npc, now: f32) -> f32 {
        now * 30.0 + npc.stagger * 9.0
    }

    /// The model's vertices posed for what the character is doing: idling at `idle_frame`,
    /// reacting to a hit or dead. Empty when there is no skeleton to pose it with.
    pub fn pose(&self, npc: &Npc, now: f32, idle_frame: f32) -> Vec<V3> {
        let Some(Rig { skeleton, skin }) = &self.rig else { return Vec::new() };
        let (name, frame, looping): (&str, f32, bool) = match npc.health.state {
            NpcState::Idle => (self.idle.as_str(), idle_frame, true),
            NpcState::Reacting { motion, start } => (motion, (now - start) * 30.0, false),
            NpcState::Dead { motion, start } => (motion, (now - start) * 30.0, false),
        };
        let Some(mot) = self.motions.get(name).or_else(|| self.motions.get(self.idle.as_str())) else {
            return self.model.vertices.clone();
        };
        let frames = mot.frames.max(1) as f32;
        let f = if looping { frame % frames } else { frame.min(frames - 1.0) };
        let (root, rots) = mot.sample(f);
        // Hit and death motions carry the body along with them
        let k0 = mot.keys[0].root;
        let (dx, dz) = if looping { (0.0, 0.0) } else { (root[0] - k0[0], root[2] - k0[2]) };
        skin.apply(&skeleton.pose([dx, root[1], dz], &rots))
    }

    /// Where a shot from `origin` along the unit vector `dir` strikes the character, if it does
    pub fn raycast(&self, npc: &Npc, now: f32, origin: V3, dir: V3) -> Option<combat::Hit> {
        let rig = self.rig.as_ref()?;
        let verts = self.pose(npc, now, Self::idle_frame(npc, now));
        combat::raycast_character(&self.model, &rig.skeleton, &rig.skin, &verts, &self.placement(npc), origin, dir)
    }
}

/// Stand a character at each spawn point (a point on the map's floor) but those in `skip`,
/// dealing the team members' faces out in turn. `yaw` says which way the character at a
/// spawn point faces, given its index and position. Also returns which team member each one is.
pub fn place_npcs(
    spawns: &[V3],
    skip: &[usize],
    world: &CollisionWorld,
    model: &Crp,
    actors: &[Actor],
    yaw: &dyn Fn(usize, V3) -> f32,
) -> (Vec<Npc>, Vec<Option<usize>>) {
    let feet_to_origin = -model.bounds().0[1];
    let (mut npcs, mut faces) = (Vec::new(), Vec::new());
    for (n, (i, p)) in spawns.iter().enumerate().filter(|(i, _)| !skip.contains(i)).enumerate() {
        let floor = world.ground_at(p[0], p[2], p[1] + 100.0).unwrap_or(p[1]);
        let actor = (!actors.is_empty()).then(|| n % actors.len());
        let scale_y = actor.map_or(1.0, |a| actors[a].scale[1]);
        npcs.push(Npc {
            health: Health::new(),
            pos: [p[0], floor + feet_to_origin * scale_y, p[2]],
            floor,
            yaw: yaw(i, *p),
            scale: actor.map_or([1.0; 3], |a| actors[a].scale),
            stagger: i as f32 * 0.73,
        });
        faces.push(actor);
    }
    (npcs, faces)
}
