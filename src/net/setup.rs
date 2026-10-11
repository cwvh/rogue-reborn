//! Setting up the game a `Setup` describes. The server and every client build their copy of
//! the game this way, so that the copies start the same.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::collision::{CollisionWorld, Params};
use crate::combat::Health;
use crate::map;
use crate::net::proto::Setup;
use crate::sim::characters::{Characters, Npc};
use crate::sim::{Config, Sim};

/// A game with the characters in place and nobody playing yet
pub fn build_sim(setup: &Setup) -> Result<Sim> {
    let level = Arc::new(if setup.map == "demo" {
        crate::demo::map()
    } else {
        map::read(Path::new(&setup.map)).with_context(|| format!("could not load the level {}", setup.map))?
    });
    let world = CollisionWorld::from_map(&level, Params::default());
    let characters = Characters::load(Path::new(&setup.model), &setup.idle);
    let npcs = setup.npcs.iter().map(|n| Npc {
        health: Health::new(),
        pos: n.pos,
        floor: n.floor,
        yaw: n.yaw,
        scale: n.scale,
        stagger: n.stagger,
    }).collect();
    let config = Config {
        magazine: setup.magazine,
        ammo: setup.ammo,
        reserve: setup.reserve,
        grenades: setup.grenades,
        speed_scale: setup.speed_scale,
        seed: setup.seed,
        culling: setup.culling,
    };
    let mut sim = Sim::new(level, world, config, characters, npcs);
    sim.pvp = setup.pvp;
    if let Some(open) = setup.doors_open {
        sim.set_doors_open(open);
    }
    Ok(sim)
}
