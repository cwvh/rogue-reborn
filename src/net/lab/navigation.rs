//! Movement comparisons through the same client, prediction and server used by rrbot.

use std::collections::{BTreeSet, VecDeque};
use std::path::Path;
use std::time::Instant;

use super::{Game, demo_setup};
use crate::net::bot::{Behavior, Skill};
use crate::net::link::Conditions;
use crate::net::proto::Setup;
use crate::net::setup::build_sim;
use crate::net::tactics::Navigation;
use crate::sim::TICK;

pub struct MovementReport {
    pub behavior: Behavior,
    pub patches: f32,
    pub rooms: f32,
    pub first_contact: Option<f32>,
    pub contacts: usize,
    pub blocked: f32,
    pub build_ms: f64,
    /// The bots' shared navigation, search workspaces included. Zero for the old wanderer.
    pub bytes: usize,
    /// The part of `bytes` held by search workspaces.
    pub search_bytes: usize,
    pub elapsed_ms: f64,
    pub circling: f32,
    pub moving_windows: f32,
    pub search_ms: f64,
    pub waiting: f32,
}

const LOOP_WINDOW: f32 = 10.0;
const LOOP_WALKED: f32 = 1000.0;
const LOOP_RATIO: f32 = 0.25;

#[derive(Default)]
struct MovingWindow {
    samples: VecDeque<(f32, [f32; 3], f32, u32)>,
    walked: f32,
    contacts: u32,
    circling: f32,
    eligible: f32,
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt()
}

impl MovingWindow {
    fn sample(&mut self, now: f32, at: [f32; 3], contact: bool) {
        if let Some(last) = self.samples.back() { self.walked += distance(last.1, at); }
        self.contacts += u32::from(contact);
        self.samples.push_back((now, at, self.walked, self.contacts));
        while self.samples.get(1).is_some_and(|p| p.0 <= now - LOOP_WINDOW) { self.samples.pop_front(); }
        let &(from, old, walked, contacts) = self.samples.front().unwrap();
        let length = self.walked - walked;
        if now - from >= LOOP_WINDOW && contacts == self.contacts && length >= LOOP_WALKED {
            self.eligible += TICK;
            if distance(old, at) / length < LOOP_RATIO { self.circling += TICK; }
        }
    }
}

/// Two opponents start at the first public base pair. Without bases, use mission starts.
pub fn compare(path: &Path, seconds: f32, seed: u64, trace: bool) -> Result<Vec<MovementReport>, String> {
    let mut setup = Setup { map: path.to_string_lossy().into_owned(), grenades: 0,
        friendly_fire: false, ..demo_setup() };
    let sim = build_sim(&setup).map_err(|e| e.to_string())?;
    let floor = |p: [f32; 3]| sim.world.ground_at(p[0], p[2], p[1] + 45.0)
        .map(|y| [p[0], y, p[2]]);
    let mut spawns = Vec::new();
    if let Some(pair) = crate::levels::find_bases(path).first() {
        setup.bases = [pair.blue.starts(), pair.gold.starts()].map(|s| s.into_iter().filter_map(floor).collect());
        spawns.extend(setup.bases.iter().filter_map(|b| b.first().copied()));
    } else {
        spawns.extend(crate::levels::find_mission_spawns(path).into_iter().filter_map(floor));
    }
    if spawns.len() < 2 { return Err("the map needs at least two public starts".to_string()); }
    // The observer uses public geometry to measure both policies in the same patches.
    let observer_sim = build_sim(&setup).map_err(|e| e.to_string())?;
    let map = Navigation::for_map(&observer_sim, &setup.map, None);
    let mut reports = Vec::new();
    for behavior in [Behavior::Legacy, Behavior::Navigate] {
        let began = Instant::now();
        let mut game = Game::with_spawns(Conditions::lan(), setup.clone(), spawns.clone());
        for i in 0..2 { game.join_bot_with(Skill::normal(), seed.wrapping_add(i), behavior); }
        let mut patches = [BTreeSet::new(), BTreeSet::new()];
        let mut rooms = [BTreeSet::new(), BTreeSet::new()];
        let mut previous = [([0.0; 3], 0.0f32); 2];
        let mut engaged = [false; 2];
        let mut contacts = 0;
        let mut first_contact = None;
        let mut blocked = 0.0;
        let mut movement = [MovingWindow::default(), MovingWindow::default()];
        let mut search_ms = 0.0f64;
        let mut waiting = [0.0f32; 2];
        for tick in 0..(seconds / TICK).round() as usize {
            game.step();
            let now = tick as f32 * TICK;
            for (i, peer) in game.peers.iter().enumerate() {
                let (Some(sim), Some(bot)) = (&peer.sim, &peer.bot) else { continue };
                if peer.client.snapshots == 0 { continue; }
                let me = &sim.players[peer.client.slot()];
                let at = me.player.pos;
                let contact = bot.target().is_some();
                if let Some(n) = &bot.navigation { search_ms = search_ms.max(n.max_search_ms); waiting[i] = waiting[i].max(n.waiting_seconds); }
                if trace && tick % 300 == 0 {
                    eprintln!("{behavior:?} t={now:.0} peer={i} at={at:?} hp={:.0} {}", me.hp, bot.trace());
                    if let Some(next) = bot.navigation.as_ref().and_then(|n| n.next_point()) {
                        let door = sim.doors.iter().enumerate().filter(|(_, d)| crate::net::tactics::flat(at, d.center(0.0)) < 200.0)
                            .map(|(k, d)| (k, d.center(0.0), sim.door_states[k].open)).collect::<Vec<_>>();
                        eprintln!("  next clearance={} rooms={:?} nearby doors={door:?}", sim.world.wall_clearance(&crate::collision::Player::new(next)), sim.standing_rooms(at, None).0);
                    }
                }
                if contact && !engaged[i] { contacts += 1; first_contact.get_or_insert(now); }
                engaged[i] = contact;
                if me.hp <= 0.0 || me.respawn_at.is_some() {
                    previous[i] = (at, now);
                    movement[i].samples.clear();
                    continue;
                }
                movement[i].sample(now, at, contact);
                if me.player.on_ground {
                    for p in map.patches_at(sim, at) {
                        patches[i].insert(p);
                        rooms[i].insert(map.patches[p].room);
                    }
                }
                let moved = crate::net::tactics::flat(at, previous[i].0);
                if moved > 30.0 { previous[i] = (at, now); }
                // Count time after two seconds without 30 units of movement, outside combat.
                if !contact && now - previous[i].1 > 2.0 { blocked += TICK; }
            }
        }
        reports.push(MovementReport { behavior,
            patches: patches.iter().map(|p| p.len()).sum::<usize>() as f32 / 2.0,
            rooms: rooms.iter().map(|r| r.len()).sum::<usize>() as f32 / 2.0,
            first_contact, contacts, blocked: blocked / 2.0,
            // Report the grid the bots used, not the observer's, which never searches.
            build_ms: game.navigation.as_ref().map_or(0.0, |n| n.built_ms),
            bytes: game.navigation.as_ref().map_or(0, |n| n.estimated_bytes()),
            search_bytes: game.navigation.as_ref().map_or(0, |n| n.search_bytes()),
            elapsed_ms: began.elapsed().as_secs_f64() * 1000.0,
            circling: movement.iter().map(|w| w.circling).sum::<f32>() / 2.0,
            moving_windows: movement.iter().map(|w| w.eligible).sum::<f32>() / 2.0, search_ms,
            waiting: waiting.iter().sum::<f32>() / 2.0 });
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_has_low_displacement_but_a_straight_walk_and_holding_do_not() {
        let (mut laps, mut straight, mut holding, mut combat) = (MovingWindow::default(), MovingWindow::default(), MovingWindow::default(), MovingWindow::default());
        for tick in 0..1800 {
            let t = tick as f32 * TICK;
            let angle = t * std::f32::consts::TAU / LOOP_WINDOW;
            let at = [300.0 * angle.cos(), 0.0, 300.0 * angle.sin()];
            laps.sample(t, at, false);
            straight.sample(t, [t * 200.0, 0.0, 0.0], false);
            holding.sample(t, [0.0; 3], false);
            combat.sample(t, at, true);
        }
        assert!(laps.circling > 15.0);
        assert_eq!(straight.circling, 0.0);
        assert_eq!(holding.eligible, 0.0);
        assert_eq!(combat.eligible, 0.0);
    }
}
