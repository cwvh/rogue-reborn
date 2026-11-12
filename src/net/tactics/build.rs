use std::thread::{self, JoinHandle};
use std::time::Instant;

use super::Navigation;
use crate::net::proto::Setup;
use crate::net::round::RoundState;
use crate::net::setup::build_sim;
use crate::sim::Sim;

pub struct Builder {
    setup: Setup,
    pending: Option<Pending>,
}

struct Pending {
    family: Option<u64>,
    job: JoinHandle<Result<Navigation, String>>,
}

impl Builder {
    pub fn new(mut setup: Setup) -> Result<Self, String> {
        // Navigation needs the map and public bases. Loading character models would duplicate
        // work and could build the unrelated NPC grid in this temporary simulation.
        setup.model.clear();
        setup.mission = None;
        setup.npcs.clear();
        let mut builder = Self { setup, pending: None };
        builder.start(None, None)?;
        Ok(builder)
    }

    fn start(&mut self, family: Option<u64>, seeds: Option<Vec<[f32; 3]>>) -> Result<(), String> {
        let setup = self.setup.clone();
        let job = thread::Builder::new().name("navigation".to_string()).spawn(move || {
            // Sim owns an Rc<Map>. Construct and drop that context entirely on this thread.
            let sim = build_sim(&setup).map_err(|e| format!("{e:#}"))?;
            Ok(match seeds {
                Some(seeds) => Navigation::from_seeds(&sim, seeds, Instant::now()),
                None => Navigation::for_map(&sim, &setup.map, None),
            })
        }).map_err(|e| e.to_string())?;
        self.pending = Some(Pending { family, job });
        Ok(())
    }

    pub fn poll(&mut self, navigation: &mut Option<Navigation>) -> Result<bool, String> {
        if !self.pending.as_ref().is_some_and(|p| p.job.is_finished()) { return Ok(false); }
        let Pending { family, job } = self.pending.take().unwrap();
        let next = job.join().map_err(|_| "navigation worker panicked".to_string())??;
        match (family, navigation.as_mut()) {
            (None, None) => *navigation = Some(next),
            (Some(id), Some(current)) if current.map_id == id => current.adopt(next),
            _ => return Err("discarded navigation for a replaced map".to_string()),
        }
        Ok(true)
    }

    pub fn prepare_start(&mut self, navigation: &Option<Navigation>, sim: &Sim, me: usize, round: RoundState, checked: &mut Option<super::Life>) -> Result<(), String> {
        if self.pending.is_some() {
            if sim.players.get(me).is_some_and(|p| p.hp <= 0.0 || p.respawn_at.is_some()) { *checked = None; }
            return Ok(());
        }
        let Some(nav) = navigation else { return Ok(()) };
        let Some(start) = nav.missing_start(sim, me, round, checked) else { return Ok(()) };
        let mut seeds = nav.seeds.clone();
        seeds.push(start);
        self.start(Some(nav.map_id), Some(seeds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::lab::demo_setup;
    use crate::net::tactics::Navigator;
    use std::time::Duration;

    fn finish(builder: &mut Builder, nav: &mut Option<Navigation>) -> Result<bool, String> {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            match builder.poll(nav) {
                Ok(false) if Instant::now() < until => thread::sleep(Duration::from_millis(1)),
                result => return result,
            }
        }
    }

    #[test]
    fn a_worker_extension_keeps_the_old_grid_and_visit_order() {
        let setup = demo_setup();
        let mut sim = build_sim(&setup).unwrap();
        sim.add_player([0.0; 3]);
        let grid = Navigation::for_map(&sim, "demo", None);
        let id = grid.id;
        let keys: Vec<_> = grid.patches.iter().map(|p| super::super::key(p.room, p.at)).collect();
        let mut personal = Navigator::new(&grid);
        personal.visited.fill(Some(3.0));
        let mut seeds = grid.seeds.clone();
        seeds.push([0.0; 3]);
        let mut builder = Builder { setup, pending: None };
        builder.start(Some(grid.map_id), Some(seeds)).unwrap();
        let mut nav = Some(grid);
        assert_eq!(nav.as_ref().unwrap().id, id);
        assert!(nav.as_ref().unwrap().grid.attached(&sim.world, [0.0; 3], &[1]));
        assert!(finish(&mut builder, &mut nav).unwrap());
        let nav = nav.unwrap();
        assert_ne!(nav.id, id);
        assert_eq!(nav.extensions, 1);
        assert_eq!(keys, nav.patches.iter().map(|p| super::super::key(p.room, p.at)).collect::<Vec<_>>());
        personal.sync(&nav);
        assert!(personal.visited.iter().all(|v| *v == Some(3.0)));
        let mut checked = None;
        let nav = Some(nav);
        for _ in 0..3 { builder.prepare_start(&nav, &sim, 0, RoundState::default(), &mut checked).unwrap(); }
        assert!(builder.pending.is_none());
        assert_eq!(checked, Some(super::super::Life { joined: sim.players[0].joined, round: 0 }));
    }

    #[test]
    fn a_missing_start_requests_one_worker_build_per_life() {
        let setup = demo_setup();
        let mut sim = build_sim(&setup).unwrap();
        sim.add_player([0.0; 3]);
        let mut grid = Navigation::from_seeds(&sim, Vec::new(), Instant::now());
        grid.map_id = grid.id;
        let id = grid.id;
        let mut nav = Some(grid);
        let mut builder = Builder { setup, pending: None };
        let mut checked = None;
        builder.prepare_start(&nav, &sim, 0, RoundState::default(), &mut checked).unwrap();
        let job = builder.pending.as_ref().unwrap().job.thread().id();
        for _ in 0..3 {
            builder.prepare_start(&nav, &sim, 0, RoundState::default(), &mut checked).unwrap();
            assert_eq!(builder.pending.as_ref().unwrap().job.thread().id(), job);
            assert_eq!(nav.as_ref().unwrap().id, id);
        }
        sim.players[0].hp = 0.0;
        builder.prepare_start(&nav, &sim, 0, RoundState::default(), &mut checked).unwrap();
        assert_eq!(checked, None);
        sim.players[0].hp = 100.0;
        assert!(finish(&mut builder, &mut nav).unwrap());
        assert!(nav.as_ref().unwrap().grid.attached(&sim.world, [0.0; 3], &[1]));
        assert_eq!(nav.as_ref().unwrap().extensions, 1);
        builder.prepare_start(&nav, &sim, 0, RoundState::default(), &mut checked).unwrap();
        assert!(builder.pending.is_none());
    }

    #[test]
    fn a_worker_reports_load_failure_and_discards_a_replaced_map() {
        let mut bad = demo_setup();
        bad.map = "no-such-navigation-map.map".to_string();
        let mut builder = Builder::new(bad).unwrap();
        assert!(finish(&mut builder, &mut None).unwrap_err().contains("could not load"));
        let setup = demo_setup();
        let sim = build_sim(&setup).unwrap();
        let mut builder = Builder::new(setup).unwrap();
        let other = Navigation::for_map(&sim, "demo", None);
        let id = other.id;
        let mut nav = Some(other);
        assert!(finish(&mut builder, &mut nav).unwrap_err().contains("replaced map"));
        assert_eq!(nav.unwrap().id, id);
    }
}
