use std::collections::HashMap;
use rogue_reborn::{blastlight, casings, glass, impact};
use rogue_reborn::net::animation::Animators;
use rogue_reborn::sim::characters::Npc;
use crate::player::BodyState;
use crate::render::draw::Flashes;

/// Added to a terrorist's number so its casings are not taken for a player's
pub const NPC_OWNERS: usize = 1000;

// What the world holds that the frames draw and keep: marks, effects, shards, shells, bodies
#[derive(Default)]
pub struct Scene {
    pub decals: Vec<impact::Decal>,
    pub effects: impact::Effects,
    pub blast_lights: Vec<blastlight::BlastLight>,
    // The shards of broken glass fall for a moment afterwards
    pub fractures: Vec<glass::Fracture>,
    pub player_flashes: Flashes,
    pub npc_flashes: Flashes,
    pub casings: casings::Casings,
    // The rounds fired since the guns were last drawn, by owner: the casings they owe
    pub casing_pending: HashMap<usize, u32>,
    pub remote_anim: Animators,
    // What each body's drawing remembers, the player's own and the other players'
    pub bodies: HashMap<usize, BodyState>,
    // The bodies of players who died, by slot: when the fall began, and the body falling where
    // they died. It stays after they come back somewhere else.
    pub corpses: HashMap<usize, (f32, Npc)>,
}
