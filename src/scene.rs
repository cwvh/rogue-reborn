use std::cell::RefCell;
use std::collections::HashMap;
use rogue_reborn::{blastlight, casings, glass, impact};
use rogue_reborn::net::animation::Animators;
use rogue_reborn::sim::characters::Npc;
use crate::player::BodyState;
use crate::render::draw::{FlashSpot, Flashes};

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
    pub flash_spots: RefCell<Vec<FlashSpot>>,
    pub casings: casings::Casings,
    pub casing_pending: RefCell<HashMap<usize, u32>>,
    pub casing_spots: RefCell<Vec<(usize, casings::Ejector, [f32; 16])>>,
    pub remote_anim: Animators,
    // What each body's drawing remembers, the player's own and the other players'
    pub bodies: HashMap<usize, BodyState>,
    // The bodies of players who died, by slot: when the fall began, and the body falling where
    // they died. It stays after they come back somewhere else.
    pub corpses: HashMap<usize, (f32, Npc)>,
}
