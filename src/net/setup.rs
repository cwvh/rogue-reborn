//! Setting up the game a `Setup` describes. The server and every client build their copy of
//! the game this way, so that the copies start the same.

use std::path::Path;
use std::rc::Rc;

use anyhow::{Context, Result};

use crate::collision::{CollisionWorld, Params};
use crate::combat::Health;
use crate::map;
use crate::net::proto::Setup;
use crate::sim::characters::{Characters, Npc, place_terrorists};
use crate::sim::{Config, Sim};

/// A game with the characters in place and nobody playing yet
pub fn build_sim(setup: &Setup) -> Result<Sim> {
    let level = Rc::new(if setup.map == "demo" {
        crate::demo::map()
    } else {
        map::read(Path::new(&setup.map)).with_context(|| format!("could not load the level {}", setup.map))?
    });
    let world = CollisionWorld::from_map(&level, Params::default());
    // No model named is none wanted
    let mut characters = (!setup.model.is_empty()).then(|| Characters::load(Path::new(&setup.model), &setup.idle)).flatten();
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
    sim.friendly_fire = setup.friendly_fire;
    sim.respawn = setup.respawn;
    sim.bases = setup.bases.clone();
    sim.base_pairs = setup.base_pairs.iter().map(|p| p.bases.clone()).collect();
    if let Some(open) = setup.doors_open {
        sim.set_doors_open(open);
    }
    sim.mark_start();
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
            friendly_fire: true, respawn: true, bases: [Vec::new(), Vec::new()], uniforms: crate::net::proto::UNIFORMS.map(String::from), base_pairs: Vec::new(), doors_open: None, mission: Some(mission.display().to_string()), npcs: Vec::new(),
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

    #[test]
    fn a_new_round_puts_back_the_glass_the_doors_and_the_characters() {
        let map = "data/map/rm19/rm19.map";
        let Some(mission) = crate::mission::find(Path::new(map), crate::mission::Difficulty::Medium) else { return };
        let setup = Setup {
            map: map.to_string(), model: "data/character/team_hrt/blue_M_a.CRP".to_string(), idle: "up_none_cyc".to_string(),
            magazine: 30, ammo: 30, reserve: 900, grenades: 3, speed_scale: 1.0, seed: 99, culling: true, pvp: true,
            friendly_fire: true, respawn: false, bases: [Vec::new(), Vec::new()], uniforms: crate::net::proto::UNIFORMS.map(String::from), base_pairs: Vec::new(), doors_open: None, mission: Some(mission.display().to_string()), npcs: Vec::new(),
        };
        let mut sim = build_sim(&setup).unwrap();
        assert!(!sim.panes.is_empty() && !sim.doors.is_empty());
        // Straight at the first pane, from a little in front of it
        let t = &sim.panes[0].tris[0].p;
        let (e1, e2) = ([0, 1, 2].map(|k| t[1][k] - t[0][k]), [0, 1, 2].map(|k| t[2][k] - t[0][k]));
        let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let n = n.map(|v| v / len);
        let c = sim.panes[0].center();
        let from = [0, 1, 2].map(|k| c[k] + n[k] * 20.0);
        let to_pane = |sim: &Sim| sim.world.raycast(from, n.map(|v| -v), 25.0);
        assert!(to_pane(&sim).is_some_and(|d| (d - 20.0).abs() < 1.0));
        let (doors, npcs) = (sim.door_states.iter().map(|d| d.open).collect::<Vec<_>>(), sim.npcs.iter().map(|n| n.pos).collect::<Vec<_>>());

        // A round's damage: the pane broken, every door open, a character moved and a grenade out
        sim.broken_at[0] = Some(1.0);
        let name = sim.panes[0].name.clone();
        sim.world.switch_off(&name);
        assert!(to_pane(&sim).is_none_or(|d| d > 21.0));
        sim.set_doors_open(1.0);
        sim.npcs[0].pos[0] += 500.0;
        sim.add_player([0.0; 3]);
        sim.thrown.push(crate::sim::Thrown { grenade: crate::grenade::Grenade::thrown([0.0; 3], [0.0, 1.0, 0.0], 0.5, [0.0; 3]), owner: 0 });

        sim.new_round(5);
        assert!(sim.broken_at.iter().all(Option::is_none));
        assert!(to_pane(&sim).is_some_and(|d| (d - 20.0).abs() < 1.0), "the pane is not back");
        assert_eq!(sim.door_states.iter().map(|d| d.open).collect::<Vec<_>>(), doors);
        assert_eq!(sim.npcs.iter().map(|n| n.pos).collect::<Vec<_>>(), npcs);
        assert!(sim.thrown.is_empty());

        // A client whose copy still has the pane broken puts it back from a snapshot
        let mut client = build_sim(&setup).unwrap();
        client.broken_at[0] = Some(1.0);
        client.world.switch_off(&name);
        crate::net::proto::Snapshot::capture(&sim, 10, 0, 0).apply(&mut client, 0);
        assert!(client.broken_at[0].is_none());
        assert!(to_pane(&client).is_some_and(|d| (d - 20.0).abs() < 1.0), "the client's pane is not back");
    }
}
