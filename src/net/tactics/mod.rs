//! Paths and exploration for multiplayer bots. Decisions use public map geometry and the
//! bot's own visits. Movement still goes through the client's ordinary player inputs.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::net::client::Frame;
use crate::sim::nav::Grid;
#[cfg(test)]
use crate::sim::nav::PathError;
use crate::sim::Sim;

mod build;
mod search;
pub use build::Builder;

type V3 = [f32; 3];
const PATCH: f32 = 800.0;
const FLOOR: f32 = 180.0;
// A door's open leaf can occupy a grid point. Pass within the body's radius instead of
// requiring the body's center to touch that point.
const ARRIVE: f32 = 24.0;
/// Height tolerance for waypoint arrival, including the ordinary stair step.
const ARRIVE_Y: f32 = 48.0;
/// Radius within which an observed open leaf affects a route.
const LEAF_SIGHT: f32 = 300.0;
/// How open a leaf must be before the bot walks past it.
const DOOR_OPEN: f32 = 0.95;
/// How close the bot gets before using a door.
const DOOR_REACH: f32 = 200.0;
/// Door visibility allows for hitting the near face before the leaf's center.
const DOOR_FACE: f32 = 60.0;
/// Door centers must lie within this vertical band above the feet.
const DOOR_Y: [f32; 2] = [20.0, 220.0];
/// Door selection extends beyond the waypoint and across the travel line by this much.
const DOOR_WIDTH: f32 = 80.0;
const DOOR_BEHIND: f32 = 20.0;
/// Maximum door wait and time between use presses.
const DOOR_WAIT: f32 = 6.0;
const DOOR_PRESS: f32 = 0.6;
const DOOR_ATTEMPTS: u8 = 2;
const DOOR_RETREAT: f32 = 80.0;
const DOOR_RETREAT_MAX: f32 = 240.0;
const DOOR_RETREAT_STEP: f32 = 10.0;
const DOOR_CHANGED: f32 = 0.1;
/// Destination retry delays after failure, completion and an empty search.
const FAILED_AGAIN: f32 = 15.0;
const REACHED_AGAIN: f32 = 2.0;
const SEARCH_AGAIN: f32 = 1.0;
const ABANDON_AGAIN: f32 = crate::net::bot::RECOVER_FOR;
/// An unvisited patch starts this much older than a visited patch.
const UNVISITED: f32 = 40.0;
const REVERSE_COST: f32 = 20.0;
const VARIETY: f32 = 0.025;
const CANDIDATES: usize = 8;
/// Replan after this interval without getting closer by PROGRESS units.
const STALLED: f32 = 2.0;
const PROGRESS: f32 = 8.0;
/// Actual displacement needed to clear the retry count after a replan.
const RECOVERED: f32 = 40.0;
const REPLANS: u8 = 2;
/// Yield briefly to a visible teammate ahead, then allow another attempt later.
const TEAMMATE_NEAR: f32 = 85.0;
const YIELD_FOR: f32 = 0.75;
const YIELD_AGAIN: f32 = 2.0;

#[derive(Clone, Copy, Debug)]
pub struct Patch {
    pub at: V3,
    pub room: u32,
    node: u32,
}

static NEXT_GRID: AtomicU64 = AtomicU64::new(1);

pub struct Navigation {
    searches: std::sync::Arc<search::Pool>,
    pub id: u64,
    map_id: u64,
    pub extensions: u32,
    pub grid: Grid,
    pub patches: Vec<Patch>,
    patch_ids: BTreeMap<(u32, i32, i32, i32), usize>,
    pub built_ms: f64,
    seeds: Vec<V3>,
}

fn key(room: u32, at: V3) -> (u32, i32, i32, i32) {
    (room, (at[1] / FLOOR).floor() as i32, (at[0] / PATCH).floor() as i32, (at[2] / PATCH).floor() as i32)
}

impl Navigation {
    pub fn for_map(sim: &Sim, map: &str, start: Option<V3>) -> Self {
        let began = Instant::now();
        let starts = if map == "demo" { crate::demo::player_spawns(crate::sim::MAX_PLAYERS) }
            else { crate::levels::find_mission_spawns(std::path::Path::new(map)) };
        let seeds = starts.into_iter().chain(sim.bases.iter().flatten().copied())
            .chain(sim.base_pairs.iter().flatten().flatten().copied());
        let mut seeds: Vec<_> = seeds.filter_map(|p| sim.world.ground_at(p[0], p[2], p[1] + sim.world.params().step_up).map(|y| [p[0], y, p[2]])).collect();
        seeds.extend(start);
        seeds.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])).then(a[2].total_cmp(&b[2])));
        seeds.dedup();
        let mut map = Self::from_seeds(sim, seeds, began);
        map.map_id = map.id;
        map
    }

    fn extend(&mut self, sim: &Sim, start: V3) {
        let began = Instant::now();
        let mut seeds = self.seeds.clone();
        seeds.push(start);
        let next = Self::from_seeds(sim, seeds, began);
        self.adopt(next);
        self.built_ms = began.elapsed().as_secs_f64() * 1000.0;
    }

    fn adopt(&mut self, mut next: Self) {
        let mut patches: Vec<_> = self.patches.iter().map(|p| {
            let i = next.patch_ids[&key(p.room, p.at)];
            next.patches[i]
        }).collect();
        patches.extend(next.patches.iter().filter(|p| !self.patch_ids.contains_key(&key(p.room, p.at))).copied());
        next.patch_ids = patches.iter().enumerate().map(|(i, p)| (key(p.room, p.at), i)).collect();
        next.patches = patches;
        next.map_id = self.map_id;
        next.extensions = self.extensions + 1;
        next.searches = self.searches.clone();
        *self = next;
    }

    /// The caller checks its own observed start once per life, after receiving a snapshot.
    pub fn prepare_start(&mut self, sim: &Sim, me: usize, round: crate::net::round::RoundState, checked: &mut Option<Life>) -> bool {
        let Some(start) = self.missing_start(sim, me, round, checked) else { return false };
        self.extend(sim, start);
        true
    }

    fn missing_start(&self, sim: &Sim, me: usize, round: crate::net::round::RoundState, checked: &mut Option<Life>) -> Option<V3> {
        let p = sim.players.get(me)?;
        if p.respawn_at.is_some() || p.hp <= 0.0 { *checked = None; return None; }
        let life = Life { joined: p.joined, round: round.number };
        if !p.active || *checked == Some(life) || sim.world.floor_rooms(p.player.pos).is_empty() { return None; }
        *checked = Some(life);
        (!self.grid.attached(&sim.world, p.player.pos, &sim.standing_rooms(p.player.pos, None).0)).then_some(p.player.pos)
    }

    fn from_seeds(sim: &Sim, seeds: Vec<V3>, began: Instant) -> Self {
        let starts: Vec<_> = seeds.iter().copied().map(|p| {
                let rooms = sim.standing_rooms(p, None).0;
                (p, rooms)
            }).collect();
        let grid = Grid::new(&sim.world, &sim.level, &sim.visibility, &starts);
        let mut by_patch = BTreeMap::new();
        for (node, at, room) in grid.points() {
            let k = key(room, at);
            let dx = at[0] - (k.2 as f32 + 0.5) * PATCH;
            let dz = at[2] - (k.3 as f32 + 0.5) * PATCH;
            let score = dx * dx + dz * dz;
            let entry = by_patch.entry(k).or_insert((Patch { at, room, node }, score));
            if score < entry.1 { *entry = (Patch { at, room, node }, score); }
        }
        let patch_ids = by_patch.keys().copied().enumerate().map(|(i, k)| (k, i)).collect();
        let patches = by_patch.into_values().map(|p| p.0).collect();
        Self { searches: search::Pool::new(), id: NEXT_GRID.fetch_add(1, Ordering::Relaxed), map_id: 0, extensions: 0, grid, patches, patch_ids, seeds, built_ms: began.elapsed().as_secs_f64() * 1000.0 }
    }

    pub fn patch_at(&self, sim: &Sim, feet: V3) -> Option<usize> {
        self.patches_at(sim, feet).next()
    }

    pub fn patches_at(&self, sim: &Sim, feet: V3) -> impl Iterator<Item = usize> + '_ {
        sim.world.floor_rooms(feet).into_iter().filter_map(move |r| self.patch_ids.get(&key(r, feet)).copied())
    }

    /// Searches that can run at once among the bots sharing this map.
    pub fn search_slots(&self, slots: usize) {
        self.searches.grow(slots);
    }

    /// Flood arrays and heaps the search slots retain. `estimated_bytes` includes them.
    pub fn search_bytes(&self) -> usize {
        self.searches.retained_bytes()
    }

    pub fn estimated_bytes(&self) -> usize {
        self.grid.estimated_bytes() + self.searches.retained_bytes() + self.patches.capacity() * std::mem::size_of::<Patch>()
            + self.patch_ids.len() * 64 + self.seeds.capacity() * std::mem::size_of::<V3>()
    }
}

/// Why a navigator is doing what it does, for traces and tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Starting,
    Combat,
    Exploring,
    LastSighting,
    Replanning,
    DoorOpened,
    OpeningDoor,
    SteppingClear,
    SwingBlocked,
    DoorWaitExpired,
    DoorRecoveryEnded,
    DestinationReached,
    Unreachable,
    NoDestination,
    StartDetached,
    GoalDetached,
    NoRoute,
    RouteChanged,
    StartMoved,
    StartNoLongerAttaches,
    TimedOut,
    SearchPending,
    Yielding,
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Reason::Starting => "starting",
            Reason::Combat => "combat",
            Reason::Exploring => "exploring",
            Reason::LastSighting => "last sighting",
            Reason::Replanning => "replanning",
            Reason::DoorOpened => "door opened: replanning",
            Reason::OpeningDoor => "opening door",
            Reason::SteppingClear => "stepping clear of door swing",
            Reason::SwingBlocked => "door swing blocked: recovering",
            Reason::DoorWaitExpired => "door wait expired: recovering",
            Reason::DoorRecoveryEnded => "door recovery ended: searching",
            Reason::DestinationReached => "destination reached",
            Reason::Unreachable => "unreachable destination",
            Reason::NoDestination => "no reachable destination",
            Reason::StartDetached => "start does not attach",
            Reason::GoalDetached => "goal does not attach",
            Reason::NoRoute => "no route to goal",
            Reason::RouteChanged => "route changed during search",
            Reason::StartMoved => "search start moved",
            Reason::StartNoLongerAttaches => "search start no longer attaches",
            Reason::TimedOut => "search timed out",
            Reason::SearchPending => "search pending",
            Reason::Yielding => "yielding",
        })
    }
}

/// One life of a slot's occupant: its join number and the round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Life {
    pub joined: u32,
    pub round: u16,
}

#[derive(Clone, Copy)]
pub struct Steering {
    pub toward: Option<V3>,
    pub look: V3,
    pub use_door: Option<usize>,
}

struct DoorWait {
    door: usize,
    from: V3,
    since: f32,
}

struct DoorAttempts {
    door: usize,
    count: u8,
    until: f32,
}

struct FailedDoor {
    door: usize,
    open: f32,
    until: f32,
}

pub struct Navigator {
    pending: Option<search::Job>,
    ahead: Option<search::Found>,
    map_id: u64,
    build_id: u64,
    pub patch_count: usize,
    pub has_grid: bool,
    pub visited: Vec<Option<f32>>,
    cooldown: Vec<f32>,
    completed: Vec<Option<f32>>,
    pub goal: Option<usize>,
    previous: Option<usize>,
    here: Option<usize>,
    path: VecDeque<(V3, Option<u32>)>,
    progress: (f32, f32),
    retries: u8,
    replan_at: Option<V3>,
    next_search: f32,
    door: Option<(usize, f32)>,
    door_wait: Option<DoorWait>,
    door_attempts: Vec<DoorAttempts>,
    avoided_doors: Vec<FailedDoor>,
    relaxed_doors: bool,
    recover_until: Option<f32>,
    pressed: f32,
    pursuit: Option<V3>,
    yielding: Option<f32>,
    yielded_until: f32,
    pub reason: Reason,
    pub fallback: bool,
    pub sight: f32,
    pub field: f32,
    pub replans: u32,
    pub blocked_seconds: f32,
    pub max_search_ms: f64,
    /// Time spent standing still for a search to finish.
    pub waiting_seconds: f32,
    failed_door: Vec<Option<(usize, f32)>>,
}

impl Navigator {
    pub fn next_point(&self) -> Option<V3> { self.path.front().map(|p| p.0) }

    pub fn recovery_finished(&self, now: f32) -> bool {
        self.recover_until.is_some_and(|until| now >= until)
    }

    pub fn new(map: &Navigation) -> Self {
        let n = map.patches.len();
        let skill = crate::net::bot::Skill::normal();
        Self { pending: None, ahead: None, map_id: map.map_id, build_id: map.id, patch_count: n, has_grid: map.grid.len() != 0, visited: vec![None; n], cooldown: vec![0.0; n], completed: vec![None; n], goal: None, previous: None,
            here: None, path: VecDeque::new(), progress: (0.0, f32::INFINITY), retries: 0, replan_at: None,
            next_search: 0.0, door: None, door_wait: None, pressed: -DOOR_PRESS, pursuit: None, yielding: None,
            door_attempts: Vec::new(), avoided_doors: Vec::new(), relaxed_doors: false, recover_until: None,
            yielded_until: 0.0, reason: Reason::Starting, fallback: false, sight: skill.sight, field: skill.field,
            replans: 0, blocked_seconds: 0.0, max_search_ms: 0.0, waiting_seconds: 0.0, failed_door: vec![None; n] }
    }

    pub fn sync(&mut self, map: &Navigation) {
        if self.map_id != map.map_id { *self = Self::new(map); return; }
        if self.build_id != map.id {
            self.pending = None;
            self.ahead = None;
            assert!(map.patches.len() >= self.patch_count);
            let n = map.patches.len();
            self.visited.resize(n, None);
            self.cooldown.resize(n, 0.0);
            self.completed.resize(n, None);
            self.failed_door.resize(n, None);
            self.patch_count = n;
            self.has_grid = map.grid.len() != 0;
            self.build_id = map.id;
            self.path.clear();
            self.door = None;
        }
        assert_eq!(self.patch_count, map.patches.len());
    }

    pub fn interrupt(&mut self) {
        self.pending = None;
        self.ahead = None;
        self.path.clear();
        self.door = None;
        self.door_wait = None;
        self.recover_until = None;
        self.relaxed_doors = false;
        self.pursuit = None;
        self.reason = Reason::Combat;
        self.fallback = false;
        self.retries = 0;
        self.replan_at = None;
    }

    pub fn cancel_search(&mut self) { self.pending = None; }

    pub fn press_door(&mut self, k: usize, now: f32) -> bool {
        if now - self.pressed < DOOR_PRESS { return false; }
        if !self.door_wait.as_ref().is_some_and(|w| w.door == k) { return false; }
        self.door_attempts.retain(|p| p.until > now);
        let p = if let Some(i) = self.door_attempts.iter().position(|p| p.door == k) { &mut self.door_attempts[i] }
            else { self.door_attempts.push(DoorAttempts { door: k, count: 0, until: now + FAILED_AGAIN }); self.door_attempts.last_mut().unwrap() };
        if p.count >= DOOR_ATTEMPTS { return false; }
        p.count += 1;
        p.until = now + FAILED_AGAIN;
        self.pressed = now;
        true
    }

    pub fn route(&mut self, map: &Navigation, sim: &Sim, from: V3, to: V3, now: f32) -> bool {
        self.pending = None;
        self.ahead = None;
        let mut leaves = self.leaves(sim, from, now);
        if let Some((k, _)) = self.door {
            if sim.door_states[k].open >= DOOR_OPEN && !leaves.contains(&k) { leaves.push(k); }
        }
        let result = map.grid.path_past_doors(&sim.world, from, &sim.standing_rooms(from, None).0, to, &sim.standing_rooms(to, None).0, &[], &leaves);
        self.path = result.unwrap_or_default().into();
        self.progress = (now, f32::INFINITY);
        self.door = None;
        self.fallback = self.path.is_empty();
        !self.path.is_empty()
    }

    fn observed_leaves(sim: &Sim, from: V3) -> Vec<usize> {
        let eye = [from[0], from[1] + sim.world.params().eye_height, from[2]];
        sim.doors.iter().enumerate().filter_map(|(k, d)| {
            let at = d.center(sim.door_states[k].open);
            let distance = (0..3).map(|c| (at[c] - eye[c]).powi(2)).sum::<f32>().sqrt();
            (distance < LEAF_SIGHT && sim.door_states[k].open >= DOOR_OPEN
                && sim.bot_sight_reach(eye, at) >= distance - DOOR_FACE).then_some(k)
        }).collect()
    }

    fn leaves(&self, sim: &Sim, from: V3, now: f32) -> Vec<usize> {
        let mut leaves = Self::observed_leaves(sim, from);
        for failed in &self.avoided_doors {
            if !self.relaxed_doors && failed.until > now && !leaves.contains(&failed.door) { leaves.push(failed.door); }
        }
        leaves
    }

    fn wait_for_door(&mut self, k: usize, feet: V3, now: f32) {
        if !self.door_wait.as_ref().is_some_and(|w| w.door == k) {
            self.door_wait = Some(DoorWait { door: k, from: feet, since: now });
        }
    }

    fn passed_door(&self, sim: &Sim, feet: V3, next: V3) -> bool {
        let Some(wait) = &self.door_wait else { return false };
        let k = wait.door;
        if sim.door_states[k].open < DOOR_OPEN { return false; }
        let at = sim.doors[k].center(0.0);
        let reach = flat(at, wait.from);
        let past = (feet[0] - at[0]) * (at[0] - wait.from[0]) + (feet[2] - at[2]) * (at[2] - wait.from[2]);
        reach > 1.0 && past >= sim.world.params().radius * reach
            && !sim.world.closed_door_blocks(feet, next, k) && sim.world.door_path_clear(feet, next, k)
    }

    fn observe_failed_doors(&mut self, sim: &Sim, me: usize, now: f32) {
        let eye = sim.eye(me);
        let changed = |k: usize, open: f32| {
            let at = sim.doors[k].center(sim.door_states[k].open);
            let d = [0, 1, 2].map(|c| at[c] - eye[c]);
            let distance = d.iter().map(|v| v * v).sum::<f32>().sqrt();
            let yaw = crate::sim::dequantize(sim.players[me].look.0).to_radians();
            let ahead = -d[0] * yaw.sin() + d[2] * yaw.cos();
            distance <= self.sight && ahead >= flat(eye, at) * self.field.to_radians().cos()
                && sim.bot_sight_reach(eye, at) >= distance - DOOR_FACE && (sim.door_states[k].open - open).abs() > DOOR_CHANGED
        };
        for (goal, failed) in self.failed_door.iter_mut().enumerate() {
            let Some((k, open)) = *failed else { continue };
            if now >= self.cooldown[goal] { *failed = None; continue; }
            if changed(k, open) {
                self.cooldown[goal] = now;
                *failed = None;
            }
        }
        let mut released = Vec::new();
        self.avoided_doors.retain(|failed| {
            let keep = failed.until > now && !changed(failed.door, failed.open);
            if !keep { released.push(failed.door); }
            keep
        });
        self.door_attempts.retain(|p| p.until > now && !released.contains(&p.door));
    }

    fn abandon(&mut self, sim: &Sim, now: f32) {
        let door = self.door.map(|(k, _)| k).or_else(|| self.door_wait.as_ref().map(|w| w.door));
        if let Some(goal) = self.goal.take() {
            self.cooldown[goal] = now + FAILED_AGAIN;
            self.failed_door[goal] = door.map(|k| (k, sim.door_states[k].open));
        }
        if let Some(k) = door {
            let failed = FailedDoor { door: k, open: sim.door_states[k].open, until: now + FAILED_AGAIN };
            if let Some(p) = self.avoided_doors.iter_mut().find(|p| p.door == k) { *p = failed; }
            else { self.avoided_doors.push(failed); }
            self.recover_until = Some(now + ABANDON_AGAIN);
        }
        self.path.clear();
        self.pending = None;
        self.ahead = None;
        self.door = None;
        self.door_wait = None;
        self.relaxed_doors = false;
        self.pursuit = None;
        self.retries = 0;
        self.next_search = now + ABANDON_AGAIN;
        self.reason = Reason::Unreachable;
        self.fallback = true;
    }

    #[cfg(test)]
    fn search(&mut self, map: &Navigation, sim: &Sim, feet: V3, now: f32, preference: usize) {
        let began = Instant::now();
        self.search_inner(map, sim, feet, now, preference);
        self.max_search_ms = self.max_search_ms.max(began.elapsed().as_secs_f64() * 1000.0);
    }

    #[cfg(test)]
    fn search_inner(&mut self, map: &Navigation, sim: &Sim, feet: V3, now: f32, preference: usize) {
        let retry = self.avoided_doors.iter().any(|p| p.until > now);
        let before = retry.then(|| self.cooldown.clone());
        self.path.clear();
        self.relaxed_doors = false;
        self.search_pass(map, sim, feet, now, preference);
        if self.path.is_empty() && let Some(before) = before {
            // Retry the same destinations when avoidance of failed doors blocks every route.
            self.cooldown = before;
            self.relaxed_doors = true;
            self.search_pass(map, sim, feet, now, preference);
            if self.path.is_empty() { self.relaxed_doors = false; }
        }
    }

    #[cfg(test)]
    fn search_pass(&mut self, map: &Navigation, sim: &Sim, feet: V3, now: f32, preference: usize) {
        let rooms = sim.standing_rooms(feet, None).0;
        let leaves = self.leaves(sim, feet, now);
        let Ok(paths) = map.grid.routes_past_doors(&sim.world, feet, &rooms, &[], &leaves) else {
            self.next_search = now + SEARCH_AGAIN;
            self.reason = Reason::StartDetached;
            self.fallback = true;
            return;
        };
        let cost = &paths.cost;
        let mut candidates: Vec<_> = map.patches.iter().enumerate()
            .filter(|(i, p)| Some(*i) != self.here && self.cooldown[*i] <= now && cost[p.node as usize].is_finite())
            .map(|(i, p)| {
                let last = self.visited[i].into_iter().chain(self.completed[i]).max_by(f32::total_cmp);
                let age = last.map_or(now + UNVISITED, |t| now - t);
                let variety = ((i.wrapping_mul(2654435761) ^ preference) % 100) as f32 * VARIETY;
                let score = age - cost[p.node as usize] / crate::sim::movement::ground_speed(crate::sim::movement::Movement::Run) + variety - if Some(i) == self.previous { REVERSE_COST } else { 0.0 };
                (i, score)
            }).collect();
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        for (goal, _) in candidates.into_iter().take(CANDIDATES) {
            let to = map.patches[goal].at;
            let end = sim.standing_rooms(to, None).0;
            let result = map.grid.path_from_tree(&sim.world, &paths, to, &end);
            match result {
                Ok(path) => {
                    self.path = path.into();
                    self.progress = (now, f32::INFINITY);
                    self.door = None;
                    self.fallback = false;
                    self.goal = Some(goal);
                    self.retries = 0;
                    self.replan_at = None;
                    self.reason = Reason::Exploring;
                    return;
                }
                Err(PathError::Start) => {
                    self.next_search = now + SEARCH_AGAIN;
                    self.reason = Reason::StartDetached;
                    self.fallback = true;
                    return;
                }
                Err(_) => self.cooldown[goal] = now + FAILED_AGAIN,
            }
        }
        self.next_search = now + SEARCH_AGAIN;
        self.reason = Reason::NoDestination;
        self.fallback = true;
    }

    pub fn steer(&mut self, map: &Navigation, sim: &Sim, me: usize, view: &Frame, now: f32, preference: usize, pursuit: Option<V3>) -> Option<Steering> {
        self.sync(map);
        let feet = sim.players[me].player.pos;
        self.observe_failed_doors(sim, me, now);
        let next = self.path.iter().find(|(p, _)| flat(feet, *p) >= ARRIVE || (feet[1] - p[1]).abs() >= ARRIVE_Y).map_or(feet, |p| p.0);
        if self.passed_door(sim, feet, next) {
            let k = self.door_wait.take().unwrap().door;
            self.door_attempts.retain(|p| p.door != k);
        }
        if let Some(until) = self.recover_until {
            if now < until { self.fallback = true; return None; }
            self.recover_until = None;
            self.reason = Reason::DoorRecoveryEnded;
        }
        if self.door_wait.as_ref().is_some_and(|w| now - w.since > DOOR_WAIT) {
            self.abandon(sim, now);
            self.reason = Reason::DoorWaitExpired;
            return None;
        }
        if self.replan_at.is_some_and(|p| (0..3).map(|k| (feet[k] - p[k]).powi(2)).sum::<f32>() >= RECOVERED * RECOVERED) {
            self.retries = 0;
            self.replan_at = None;
        }
        if let Some(here) = map.patch_at(sim, feet).filter(|_| sim.players[me].player.on_ground) {
            if self.here != Some(here) { self.previous = self.here; self.here = Some(here); }
            for p in map.patches_at(sim, feet) { self.visited[p] = Some(now); }
        }
        if pursuit != self.pursuit {
            self.pending = None;
            self.ahead = None;
            self.pursuit = pursuit;
            self.path.clear();
            if let Some(to) = pursuit {
                self.request(map, feet, now, preference, Some(to));
                self.reason = Reason::LastSighting;
            }
        }
        if self.pending.is_none() && self.path.is_empty() && self.ahead.is_some() {
            self.adopt_ahead(map, sim, feet, now);
        }
        if self.pending.is_none() && self.path.is_empty() && now >= self.next_search {
            if let Some(goal) = self.goal {
                if self.here != Some(goal) && self.cooldown[goal] <= now {
                    self.request(map, feet, now, preference, Some(map.patches[goal].at));
                } else { self.goal = None; }
            }
            if self.pending.is_none() && self.path.is_empty() { self.request(map, feet, now, preference, None); }
        }
        if self.advance_search(map, sim, feet, now) {
            self.waiting_seconds += crate::sim::TICK;
            let yaw = crate::sim::dequantize(sim.players[me].look.0).to_radians();
            return Some(Steering { toward: None, look: [feet[0]-100.0*yaw.sin(),feet[1]+sim.world.params().eye_height,feet[2]+100.0*yaw.cos()], use_door: None });
        }
        if self.path.is_empty() { return None; }
        if self.door.is_some_and(|(k, _)| sim.door_states[k].open >= DOOR_OPEN) {
            let to = self.path.back()?.0;
            self.request(map, feet, now, preference, Some(to));
            self.reason = Reason::DoorOpened;
            return None;
        }
        while self.path.front().is_some_and(|&(p, door)| door.is_none() && flat(feet, p) < ARRIVE && (feet[1] - p[1]).abs() < ARRIVE_Y) {
            self.path.pop_front();
            self.progress = (now, f32::INFINITY);
        }
        if self.path.is_empty() {
            self.relaxed_doors = false;
            if self.pursuit.is_none() {
                if let Some(goal) = self.goal.take() {
                    self.cooldown[goal] = now + REACHED_AGAIN;
                    self.completed[goal] = Some(now);
                }
            }
            self.reason = Reason::DestinationReached;
            return None;
        }
        self.look_ahead(map, sim, feet, now, preference);
        let &(next, portal) = self.path.front()?;
        let distance = flat(feet, next);
        if distance + PROGRESS < self.progress.1 { self.progress = (now, distance); }
        // A grid point can lie inside a door leaf before the path crosses its portal.
        // Check nearby leaves too, including doors within a single room.
        let door = self.door.map(|(k, _)| k).filter(|&k| sim.door_states[k].open < DOOR_OPEN).or_else(|| sim.doors.iter().enumerate().filter_map(|(k, door)| {
            let at = door.center(0.0);
            let near = flat(feet, at);
            if near >= DOOR_REACH || at[1] < feet[1] + DOOR_Y[0] || at[1] > feet[1] + DOOR_Y[1]
                || sim.door_states[k].open >= DOOR_OPEN || (!self.relaxed_doors && self.avoided_doors.iter().any(|p| p.door == k && p.until > now)) { return None; }
            let d = [next[0] - feet[0], next[2] - feet[2]];
            let along = ((at[0] - feet[0]) * d[0] + (at[2] - feet[2]) * d[1]) / distance.max(1.0);
            let across = ((at[0] - feet[0]) * d[1] - (at[2] - feet[2]) * d[0]).abs() / distance.max(1.0);
            if along < -DOOR_BEHIND || along > distance + DOOR_WIDTH || across > DOOR_WIDTH
                || !sim.world.closed_door_blocks(feet, next, k) { return None; }
            let eye = sim.eye(me);
            let sight = (0..3).map(|c| (at[c] - eye[c]).powi(2)).sum::<f32>().sqrt();
            (sim.bot_sight_reach(eye, at) >= sight - DOOR_FACE).then_some((k, near))
        }).min_by(|a, b| a.1.total_cmp(&b.1)).map(|p| p.0));
        if let Some(k) = door {
            let at = sim.doors[k].center(0.0);
            if flat(feet, at) < DOOR_REACH {
                self.door = Some((k, now));
                self.wait_for_door(k, feet, now);
                if !sim.world.door_sweep_clear(&sim.players[me].player, k) {
                    let away = (feet[0] - at[0]).atan2(feet[2] - at[2]);
                    for distance in [DOOR_RETREAT, DOOR_RETREAT * 2.0, DOOR_RETREAT_MAX] {
                        for angle in [0.0f32, 45.0, -45.0, 90.0, -90.0, 135.0, -135.0, 180.0] {
                            let yaw = away + angle.to_radians();
                            let mut body = sim.players[me].player;
                            let mut safe = true;
                            let first = DOOR_RETREAT.max(distance - DOOR_RETREAT + DOOR_RETREAT_STEP);
                            for step_index in 1..=(distance / DOOR_RETREAT_STEP) as usize {
                                let step = DOOR_RETREAT_STEP;
                                sim.world.step(&mut body, [yaw.sin() * step, yaw.cos() * step], crate::sim::TICK);
                                safe &= body.on_ground && (body.pos[1] - feet[1]).abs() <= sim.world.params().step_up;
                                if !safe { break; }
                                if step_index as f32 * step >= first && flat(body.pos, feet) > sim.world.params().radius
                                    && flat(body.pos, at) < DOOR_REACH && sim.world.door_sweep_clear(&body, k) {
                                    self.reason = Reason::SteppingClear;
                                    return Some(Steering { toward: Some(body.pos), look: body.pos, use_door: None });
                                }
                            }
                        }
                    }
                    self.abandon(sim, now);
                    self.reason = Reason::SwingBlocked;
                    return None;
                }
                self.reason = Reason::OpeningDoor;
                self.progress = (now, f32::INFINITY);
                return Some(Steering { toward: None, look: at, use_door: (!sim.door_states[k].opening).then_some(k) });
            }
        } else if portal.is_some() && distance < ARRIVE && (feet[1] - next[1]).abs() < ARRIVE_Y {
            self.path.pop_front();
            self.door = None;
            return Some(Steering { toward: None, look: next, use_door: None });
        }
        if now - self.progress.0 >= STALLED {
            self.blocked_seconds += now - self.progress.0;
            self.replans += 1;
            self.retries += 1;
            if self.retries > REPLANS { self.abandon(sim, now); return None; }
            self.replan_at = Some(feet);
            let end = self.path.back()?.0;
            self.request(map, feet, now, preference, Some(end));
            self.reason = Reason::Replanning;
            return Some(Steering { toward: None, look: next, use_door: None });
        }
        // Yield only to a teammate actually in the bot's view, never a hidden snapshot body.
        let side = sim.players[me].side;
        let yaw = crate::sim::dequantize(sim.players[me].look.0).to_radians();
        let teammate = side.is_some() && view.players.iter().any(|p| p.slot < me && p.dead.is_none()
            && sim.players.get(p.slot).is_some_and(|s| s.active && s.joined == p.joined && s.side == side)
            && flat(feet, p.feet) < TEAMMATE_NEAR && flat(p.feet, next) < distance
            && -(p.feet[0] - feet[0]) * yaw.sin() + (p.feet[2] - feet[2]) * yaw.cos() > flat(feet, p.feet) * self.field.to_radians().cos()
            && {
                let eye = sim.eye(me);
                let chest = [p.feet[0], p.feet[1] + crate::net::bot::CHEST, p.feet[2]];
                let distance = (0..3).map(|k| (chest[k] - eye[k]).powi(2)).sum::<f32>().sqrt();
                distance <= self.sight && sim.line_reach(eye, chest) >= distance - 1.0
            });
        if teammate && now >= self.yielded_until {
            let since = *self.yielding.get_or_insert(now);
            if now - since < YIELD_FOR {
                self.reason = Reason::Yielding;
                return Some(Steering { toward: None, look: next, use_door: None });
            }
            self.yielded_until = now + YIELD_AGAIN;
        }
        self.yielding = None;
        Some(Steering { toward: Some(next), look: next, use_door: None })
    }
}

pub fn flat(a: V3, b: V3) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests;
