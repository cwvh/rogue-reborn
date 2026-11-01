//! Setting up the game a `Setup` describes. The server and every client build their copy of
//! the game this way, so that the copies start the same.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::collision::{CollisionWorld, Params};
use crate::combat::Health;
use crate::map;
use crate::net::proto::Setup;
use crate::sim::characters::{Characters, Npc, place_terrorists};
use crate::sim::{Config, Sim};

/// A game with the characters in place and nobody playing yet
pub fn build_sim(setup: &Setup) -> Result<Sim> {
    let level = Arc::new(if setup.map == "demo" {
        crate::demo::map()
    } else {
        map::read(Path::new(&setup.map)).with_context(|| format!("could not load the level {}", setup.map))?
    });
    let world = CollisionWorld::from_map(&level, Params::default());
    let mut characters = Characters::load(Path::new(&setup.model), &setup.idle);
    let npcs = match (&setup.mission, &mut characters) {
        (Some(mission), Some(c)) => {
            let teams = crate::mission::read(Path::new(mission))?;
            place_terrorists(&teams, setup.seed, &crate::mission::setting(Path::new(mission)), &world, c)
        }
        _ => setup.npcs.iter().map(|n| Npc {
            health: Health::new(),
            pos: n.pos,
            floor: n.floor,
            yaw: n.yaw,
            scale: n.scale,
            stagger: n.stagger,
            ..Npc::default()
        }).collect(),
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::link::MAX_PACKET;
    use crate::net::proto::ToClient;

    #[test]
    fn everyone_places_the_missions_terrorists_alike_from_a_setup_that_fits_a_datagram() {
        let map = "data/map/rm19/rm19.map";
        let Some(mission) = crate::mission::find(Path::new(map), crate::mission::Difficulty::Medium) else { return };
        let setup = Setup {
            map: map.to_string(), model: "data/character/team_hrt/blue_M_a.CRP".to_string(), idle: "up_none_cyc".to_string(),
            magazine: 30, ammo: 30, reserve: 900, grenades: 3, speed_scale: 1.0, seed: 99, culling: true, pvp: true,
            doors_open: None, mission: Some(mission.display().to_string()), npcs: Vec::new(),
        };
        let (server, client) = (build_sim(&setup).unwrap(), build_sim(&setup).unwrap());
        // The most terrorists on any level at medium
        assert_eq!(server.npcs.len(), 29);
        assert!(server.npcs.iter().all(|n| n.body.is_some() && n.stance.is_some()));
        let at = |s: &Sim| s.npcs.iter().map(|n| (n.pos, n.yaw, n.body, n.stance)).collect::<Vec<_>>();
        assert_eq!(at(&server), at(&client));
        let (bytes, _) = ToClient::Accept { token: 1, slot: 0, tick: 0, setup }.encode();
        assert!(bytes.len() <= MAX_PACKET, "{} bytes", bytes.len());
    }
}
