//! The characters standing in the level: where they are, how they are posed for hit
//! detection, and the model they share.

use std::collections::HashMap;

use crate::combat::{self, Health, NpcState, Placement};
use crate::crp::{Crp, Skin};
use crate::motion::Motion;
use crate::skeleton::Skeleton;

type V3 = [f32; 3];

/// A character in the level
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

impl Characters {
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
