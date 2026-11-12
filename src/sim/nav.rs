//! Where a body can walk, for the characters to find their way: a grid of places across each
//! room's floors where a body fits among its walls, joined to the places beside them a step up
//! or down away, and from room to room through the doorways between. The rooms overlap (the
//! outside of a plane sits in its cabin), so each room's places are found among its own walls.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::collision::{CollisionWorld, Player};
use crate::map::Map;
use crate::visibility::Visibility;

mod continuation;
pub use continuation::{Blockers, Flood, LeafState, Route};

type V3 = [f32; 3];

/// How far apart the places are, across the floor
const CELL: f32 = 40.0;
/// The most a step goes up or down from one place to the next
const STEP: f32 = 45.0;
/// The most places a level has, in case one spreads out of hand
const MAX_PLACES: usize = 600_000;
/// How often along a straight leg a body is made sure to fit
const FIT_EVERY: f32 = 10.0;
const ATTACH_CANDIDATES: usize = 8;
/// How much further a way may go to keep from opening a door (a guess: about as far as a
/// terrorist walks while a door opens)
const SHUT_DOOR: f32 = 400.0;

struct Place {
    at: V3,
    room: u32,
    /// Every room that holds it, whose walls a body passing it must keep clear of
    walls: u32,
}

struct Edge {
    node: u32,
    cost: f32,
    /// Doorway portal, or u32::MAX for an edge within one room.
    door: u32,
}

pub struct Grid {
    places: Vec<Place>,
    /// Outgoing edge range for each place.
    links: Vec<(u32, u32)>,
    edges: Vec<Edge>,
    room_sets: Vec<Vec<u32>>,
    /// The places at each spot across the floor
    columns: HashMap<(i32, i32), Vec<u32>>,
    /// The places joined to each place share its group
    group: Vec<u32>,
    /// Only legs that touch panes need entries. Most edges have no glass requirement.
    pane_edges: HashMap<(u32, u32), Vec<u32>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError {
    Start,
    Goal,
    Disconnected,
}

pub struct Paths {
    pub cost: Vec<f32>,
    came: Vec<(u32, Option<u32>)>,
    leaves: Vec<usize>,
}

fn column(x: f32, z: f32) -> (i32, i32) {
    ((x / CELL).floor() as i32, (z / CELL).floor() as i32)
}

fn dist(a: V3, b: V3) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

impl Grid {
    /// The places a body can walk to from `seeds` (where characters stand, and their rooms),
    /// spreading out from place to place: so the sea round a ship, and floors no one can reach,
    /// have none
    pub fn new(world: &CollisionWorld, map: &Map, visibility: &Visibility, seeds: &[(V3, Vec<u32>)]) -> Grid {
        let mut grid = Grid { places: Vec::new(), links: Vec::new(), edges: Vec::new(), room_sets: Vec::new(), columns: HashMap::new(), group: Vec::new(), pane_edges: HashMap::new() };
        let mut index: HashMap<(u32, i32, i32, i32), u32> = HashMap::new();
        let mut room_sets = HashMap::new();
        // A place in `room` over a column, a floor near `near` in height, if a body fits there
        let mut place = |grid: &mut Grid, room: u32, (cx, cz): (i32, i32), near: f32, reach: f32| -> Option<u32> {
            let (x, z) = ((cx as f32 + 0.5) * CELL, (cz as f32 + 0.5) * CELL);
            // A tread can lie just above the floor below it. The body steps onto the highest
            // reachable surface, rather than walking under that tread on the lower floor.
            let y = world.floors_in(room, x, z).into_iter().filter(|y| *y <= near + STEP && *y >= near - reach)
                .max_by(f32::total_cmp)?;
            let key = (room, cx, cz, (y / 10.0).round() as i32);
            if let Some(&i) = index.get(&key) {
                return Some(i);
            }
            if grid.places.len() >= MAX_PLACES || world.structural_clearance_in(&Player::new([x, y, z]), room) < 0.0 {
                return None;
            }
            // Clear of the walls of the other rooms that hold it too, as a room inside another
            let mut walls: Vec<u32> = map.rooms.containing([x, y + 50.0, z]).into_iter()
                .filter_map(|r| map.rooms.rooms[r].section_name.parse().ok()).collect();
            if !walls.contains(&room) {
                walls.push(room);
            }
            if walls.iter().any(|&r| world.structural_clearance_in(&Player::new([x, y, z]), r) < 0.0) || !head_room(world, &walls, [x, y, z]) {
                return None;
            }
            let i = grid.places.len() as u32;
            index.insert(key, i);
            let walls = *room_sets.entry(walls.clone()).or_insert_with(|| {
                let id = grid.room_sets.len() as u32;
                grid.room_sets.push(walls);
                id
            });
            grid.places.push(Place { at: [x, y, z], room, walls });
            grid.links.push((0, 0));
            grid.columns.entry((cx, cz)).or_default().push(i);
            Some(i)
        };
        let mut todo: Vec<u32> = Vec::new();
        for (s, rooms) in seeds {
            let (cx, cz) = column(s[0], s[2]);
            for &room in rooms {
                // The column it stands in, or one beside it when that is too near a wall
                let found = (-1..=1).flat_map(|dx| (-1..=1).map(move |dz| (cx + dx, cz + dz)))
                    .find_map(|c| place(&mut grid, room, c, s[1], STEP));
                todo.extend(found);
            }
        }
        let mut seen = vec![false; 0];
        while let Some(i) = todo.pop() {
            if seen.len() <= i as usize {
                seen.resize(i as usize + 1, false);
            }
            if std::mem::replace(&mut seen[i as usize], true) {
                continue;
            }
            let (at, room) = (grid.places[i as usize].at, grid.places[i as usize].room);
            let (cx, cz) = column(at[0], at[2]);
            let mut next = Vec::new();
            // Beside it in its room
            for (dx, dz) in [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)] {
                if let Some(j) = place(&mut grid, room, (cx + dx, cz + dz), at[1], STEP) {
                    // A step across a corner only where a body fits halfway
                    let to = grid.places[j as usize].at;
                    let half = Player::new([0, 1, 2].map(|k| (at[k] + to[k]) / 2.0));
                    if dx == 0 || dz == 0 || grid.room_sets[grid.places[i as usize].walls as usize].iter().all(|&r| world.structural_clearance_in(&half, r) >= 0.0) {
                        next.push((j, None, Vec::new()));
                    }
                }
            }
            // Through a doorway near it, into the room beyond
            for (door, other, middle) in visibility.doorways_of(room) {
                if ((middle[0] - at[0]).powi(2) + (middle[2] - at[2]).powi(2)).sqrt() > 3.0 * CELL {
                    continue;
                }
                for dx in -2..=2 {
                    for dz in -2..=2 {
                        let Some(j) = place(&mut grid, other, (cx + dx, cz + dz), at[1], 2.0 * STEP) else { continue };
                        if visibility.doorway(room, other, at, grid.places[j as usize].at) == Some(door) {
                            // The portal test can accept a sill that is too high to step over. Check
                            // that a body can walk the full leg before keeping a glass crossing.
                            let panes = world.panes_on_leg(at, grid.places[j as usize].at);
                            let fitted = if panes.is_empty() { Some(panes) } else { grid.portal_fit(world, map, i, j) };
                            if let Some(panes) = fitted {
                                next.push((j, Some(door as u32), panes));
                            }
                        }
                    }
                }
            }
            let begin = grid.edges.len() as u32;
            for (j, door, extra) in next {
                let to = grid.places[j as usize].at;
                let mut panes = world.panes_on_leg(at, to);
                panes.extend(extra);
                panes.sort_unstable();
                panes.dedup();
                if !panes.is_empty() { grid.pane_edges.insert((i, j), panes); }
                let d = dist(at, to);
                grid.edges.push(Edge { node: j, cost: d, door: door.unwrap_or(u32::MAX) });
                todo.push(j);
            }
            grid.links[i as usize] = (begin, grid.edges.len() as u32);
        }
        grid.join();
        grid.places.shrink_to_fit();
        grid.links.shrink_to_fit();
        grid.edges.shrink_to_fit();
        grid.pane_edges.shrink_to_fit();
        grid.room_sets.shrink_to_fit();
        for column in grid.columns.values_mut() { column.shrink_to_fit(); }
        grid
    }

    fn portal_fit(&self, world: &CollisionWorld, map: &Map, a: u32, b: u32) -> Option<Vec<u32>> {
        let (a, b) = (&self.places[a as usize], &self.places[b as usize]);
        let mut rooms = self.room_sets[a.walls as usize].clone();
        for &r in &self.room_sets[b.walls as usize] {
            if !rooms.contains(&r) { rooms.push(r); }
        }
        let steps = (dist(a.at, b.at) / FIT_EVERY).ceil().max(1.0) as usize;
        let mut height = a.at[1];
        let mut previous = a.at;
        let mut panes = Vec::new();
        let fits = (0..=steps).all(|i| {
            let mut at = [0, 1, 2].map(|k| a.at[k] + (b.at[k] - a.at[k]) * i as f32 / steps as f32);
            let mut walls = rooms.clone();
            for r in map.rooms.containing([at[0], at[1] + 50.0, at[2]]) {
                if let Ok(r) = map.rooms.rooms[r].section_name.parse() {
                    if !walls.contains(&r) { walls.push(r); }
                }
            }
            let floor = walls.iter().flat_map(|&r| world.floors_in(r, at[0], at[2]))
                .filter(|&y| y <= height + STEP && y >= height - 2.0 * STEP).max_by(f32::total_cmp);
            let Some(y) = floor else { return false };
            height = y;
            // Stay above the lower floor while crossing a drop. Snapping down at the edge puts
            // the body's lower sphere into the face of the ledge it is leaving.
            at[1] = at[1].max(y);
            panes.extend(world.panes_on_leg(previous, at));
            previous = at;
            walls.iter().all(|&r| world.structural_clearance_in(&Player::new(at), r) >= -0.5)
                && head_room(world, &walls, at)
        }) && (height - b.at[1]).abs() <= STEP;
        panes.sort_unstable();
        panes.dedup();
        fits.then_some(panes)
    }

    fn edge_clear(&self, world: &CollisionWorld, i: u32, j: u32, leaves: &[usize]) -> bool {
        self.pane_edges.get(&(i, j)).is_none_or(|panes| panes.iter().all(|&p| world.pane_broken(p)))
            && leaves.iter().all(|&d| world.door_path_clear(self.places[i as usize].at, self.places[j as usize].at, d))
    }

    fn neighbors(&self, i: usize) -> impl Iterator<Item = (u32, f32, Option<u32>)> + '_ {
        let (begin, end) = self.links[i];
        self.edges[begin as usize..end as usize].iter().map(|e| (e.node, e.cost, (e.door != u32::MAX).then_some(e.door)))
    }

    /// Which places are joined to which, through doorways shut or open, so a way to somewhere
    /// nothing joins is found to be none at once
    fn join(&mut self) {
        let mut parent: Vec<u32> = (0..self.places.len() as u32).collect();
        fn root(parent: &mut [u32], mut i: u32) -> u32 {
            while parent[i as usize] != i {
                parent[i as usize] = parent[parent[i as usize] as usize];
                i = parent[i as usize];
            }
            i
        }
        for i in 0..self.links.len() {
            for (j, _, _) in self.neighbors(i) {
                let (a, b) = (root(&mut parent, i as u32), root(&mut parent, j));
                parent[a as usize] = b;
            }
        }
        self.group = (0..self.places.len() as u32).map(|i| root(&mut parent, i)).collect();
    }

    pub fn len(&self) -> usize {
        self.places.len()
    }

    pub fn points(&self) -> impl Iterator<Item = (u32, V3, u32)> + '_ {
        self.places.iter().enumerate().map(|(i, p)| (i as u32, p.at, p.room))
    }

    pub fn transitions(&self) -> impl Iterator<Item = (V3, V3, Option<u32>)> + '_ {
        (0..self.links.len()).flat_map(move |i| self.neighbors(i).map(move |(j, _, p)| (self.places[i].at, self.places[j as usize].at, p)))
    }

    pub fn locate(&self, feet: V3, rooms: &[u32]) -> Option<u32> {
        self.nearest(feet, rooms)
    }

    /// Convert an above-floor sound or plan point to a walking destination in its rooms.
    pub fn floor(world: &CollisionWorld, at: V3, rooms: &[u32]) -> Option<V3> {
        rooms.iter().copied().chain(world.floor_rooms(at)).chain(world.floor_rooms([at[0], at[1] - STEP, at[2]])).flat_map(|r| world.floors_in(r, at[0], at[2]))
            .filter(|&y| y <= at[1] + STEP).max_by(f32::total_cmp)
            .map(|y| [at[0], y, at[2]])
    }

    /// Directed walking distances from a start, with an extra cost for shut doors.
    pub fn distances(&self, world: &CollisionWorld, feet: V3, rooms: &[u32], shut: &[bool]) -> Vec<f32> {
        self.routes(world, feet, rooms, shut).map(|p| p.cost).unwrap_or_else(|_| vec![f32::INFINITY; self.len()])
    }

    pub fn routes(&self, world: &CollisionWorld, feet: V3, rooms: &[u32], shut: &[bool]) -> Result<Paths, PathError> {
        self.routes_past_doors(world, feet, rooms, shut, &[])
    }

    pub fn routes_past_doors(&self, world: &CollisionWorld, feet: V3, rooms: &[u32], shut: &[bool], leaves: &[usize]) -> Result<Paths, PathError> {
        let a = self.attach(world, feet, rooms, leaves).ok_or(PathError::Start)?;
        let mut paths = self.flood_where(a, shut, |i, j| self.edge_clear(world, i, j, leaves));
        paths.leaves = leaves.to_vec();
        Ok(paths)
    }

    fn flood_where(&self, a: u32, shut: &[bool], clear: impl Fn(u32, u32) -> bool) -> Paths {
        let mut cost = vec![f32::INFINITY; self.len()];
        let mut came = vec![(u32::MAX, None); self.len()];
        let mut open = BinaryHeap::from([(Reverse(Key(0.0)), a)]);
        cost[a as usize] = 0.0;
        while let Some((Reverse(Key(here)), i)) = open.pop() {
            if here > cost[i as usize] { continue; }
            for (j, d, door) in self.neighbors(i as usize) {
                if !clear(i, j) { continue; }
                let c = here + d + if door.is_some_and(|p| shut.get(p as usize).copied().unwrap_or(false)) { SHUT_DOOR } else { 0.0 };
                if c < cost[j as usize] {
                    cost[j as usize] = c;
                    came[j as usize] = (i, door);
                    open.push((Reverse(Key(c)), j));
                }
            }
        }
        Paths { cost, came, leaves: Vec::new() }
    }

    pub fn path_from_tree(&self, world: &CollisionWorld, paths: &Paths, to: V3, rooms: &[u32]) -> Result<Vec<(V3, Option<u32>)>, PathError> {
        let b = self.attach(world, to, rooms, &paths.leaves).ok_or(PathError::Goal)?;
        if !paths.cost[b as usize].is_finite() { return Err(PathError::Disconnected); }
        self.way_back(world, &paths.came, b, to, &paths.leaves).ok_or(PathError::Disconnected)
    }

    pub fn attached(&self, world: &CollisionWorld, feet: V3, rooms: &[u32]) -> bool {
        self.attach(world, feet, rooms, &[]).is_some()
    }

    fn attach(&self, world: &CollisionWorld, feet: V3, rooms: &[u32], leaves: &[usize]) -> Option<u32> {
        self.nearest_where(feet, rooms, |i| clear_leg(world, feet, self.places[i as usize].at, leaves) && self.attachment(world, feet, i, rooms))
    }

    /// An investigation may end beside a wall sound, on a nearby valid point of the same floor.
    pub fn investigate(&self, world: &CollisionWorld, from: V3, start: &[u32], sound: V3, rooms: &[u32], shut: &[bool]) -> Option<Vec<(V3, Option<u32>)>> {
        let floor = Self::floor(world, sound, rooms)?;
        let there = world.floor_rooms(floor);
        if let Some(path) = self.path(world, from, start, floor, &there, shut) { return Some(path); }
        let node = self.nearest(floor, &there)?;
        let goal = &self.places[node as usize];
        if !there.contains(&goal.room) { return None; }
        self.path(world, from, start, goal.at, &there, shut)
    }

    pub fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.places.capacity() * std::mem::size_of::<Place>()
            + self.room_sets.capacity() * std::mem::size_of::<Vec<u32>>()
            + self.room_sets.iter().map(|r| r.capacity() * 4).sum::<usize>()
            + self.links.capacity() * std::mem::size_of::<(u32, u32)>()
            + self.edges.capacity() * std::mem::size_of::<Edge>()
            + self.columns.capacity() * (std::mem::size_of::<((i32, i32), Vec<u32>)>() + 1)
            + self.columns.values().map(|c| c.capacity() * 4).sum::<usize>() + self.group.capacity() * 4
            + self.pane_edges.capacity() * (std::mem::size_of::<((u32, u32), Vec<u32>)>() + 1)
            + self.pane_edges.values().map(|p| p.capacity() * 4).sum::<usize>()
    }

    #[cfg(test)]
    pub fn profile(&self) {
        let mut rooms = std::collections::BTreeMap::new();
        let mut heights = std::collections::BTreeMap::new();
        let mut components = std::collections::BTreeMap::new();
        for (i, p) in self.places.iter().enumerate() {
            *rooms.entry(p.room).or_insert(0usize) += 1;
            *heights.entry((p.at[1] / 500.0).floor() as i32 * 500).or_insert(0usize) += 1;
            *components.entry(self.group[i]).or_insert(0usize) += 1;
        }
        println!("rooms={rooms:?}\nheights={heights:?}\ncomponents={components:?}");
        println!("places={} links={} columns={} room_sets={} bytes={}", self.places.len(), self.edges.len(), self.columns.len(), self.room_sets.len(), self.estimated_bytes());
        println!("place allocation={} room allocation={} link allocation={}", self.places.capacity() * std::mem::size_of::<Place>(), self.room_sets.iter().map(|r| r.capacity() * 4).sum::<usize>(), self.links.capacity() * 8 + self.edges.capacity() * std::mem::size_of::<Edge>());
    }

    /// The place nearest the feet at `at`, among the rooms `rooms` if any of
    /// them has one near, else in any
    fn nearest(&self, at: V3, rooms: &[u32]) -> Option<u32> {
        self.nearest_where(at, rooms, |_| true)
    }

    fn nearest_where(&self, at: V3, rooms: &[u32], accept: impl Fn(u32) -> bool) -> Option<u32> {
        let (cx, cz) = column(at[0], at[2]);
        let mut candidates = Vec::new();
        for dx in -3..=3 {
            for dz in -3..=3 {
                for &i in self.columns.get(&(cx + dx, cz + dz)).map(Vec::as_slice).unwrap_or_default() {
                    let p = &self.places[i as usize];
                    if (p.at[1] - at[1]).abs() > STEP {
                        continue;
                    }
                    let (outside, d) = (!rooms.contains(&p.room), dist(p.at, at));
                    candidates.push((outside, d, i));
                }
            }
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        candidates.into_iter().take(ATTACH_CANDIDATES).map(|p| p.2).find(|&i| accept(i))
    }

    /// A way to walk from `from` (in the rooms `start`) to `to` (in `end`): the places to make
    /// for in turn, as few as a straight walk between them allows, `to` last, each with the
    /// doorway (a portal) its leg goes through, if it does. A way stops before each doorway,
    /// to open its door if it is shut; a door `shut` costs a little more walk than an open
    /// way, which it takes when it is not much longer. None when the grid has nowhere near
    /// either, or nothing joins them.
    pub fn path(&self, world: &CollisionWorld, from: V3, start: &[u32], to: V3, end: &[u32], shut: &[bool]) -> Option<Vec<(V3, Option<u32>)>> {
        self.path_past_doors(world, from, start, to, end, shut, &[]).ok()
    }

    /// Route around observed open leaves. Other doors remain optimistic opening opportunities.
    pub fn path_past_doors(&self, world: &CollisionWorld, from: V3, start: &[u32], to: V3, end: &[u32], shut: &[bool], leaves: &[usize]) -> Result<Vec<(V3, Option<u32>)>, PathError> {
        let a = self.attach(world, from, start, leaves).ok_or(PathError::Start)?;
        let b = self.attach(world, to, end, leaves).ok_or(PathError::Goal)?;
        if self.group[a as usize] != self.group[b as usize] {
            return Err(PathError::Disconnected);
        }
        let goal = self.places[b as usize].at;
        // A* across the places
        let mut came = vec![(u32::MAX, None); self.places.len()];
        let mut cost = vec![f32::INFINITY; self.places.len()];
        cost[a as usize] = 0.0;
        let mut open = BinaryHeap::from([(Reverse(Key(dist(self.places[a as usize].at, goal))), Key(0.0), a)]);
        while let Some((_, Key(here), i)) = open.pop() {
            if i == b {
                break;
            }
            // Already reached a shorter way
            if here > cost[i as usize] {
                continue;
            }
            for (j, d, door) in self.neighbors(i as usize) {
                if !self.edge_clear(world, i, j, leaves) { continue; }
                let opening = door.is_some_and(|p| shut.get(p as usize).copied().unwrap_or(false));
                let c = here + d + if opening { SHUT_DOOR } else { 0.0 };
                if c < cost[j as usize] {
                    cost[j as usize] = c;
                    came[j as usize] = (i, door);
                    open.push((Reverse(Key(c + dist(self.places[j as usize].at, goal))), Key(c), j));
                }
            }
        }
        if a != b && came[b as usize].0 == u32::MAX {
            return Err(PathError::Disconnected);
        }
        self.way_back(world, &came, b, to, leaves).ok_or(PathError::Disconnected)
    }

    /// The short connection from an exact floor point to its grid point must fit too.
    fn attachment(&self, world: &CollisionWorld, at: V3, node: u32, rooms: &[u32]) -> bool {
        let place = &self.places[node as usize];
        let steps = (dist(at, place.at) / FIT_EVERY).ceil().max(1.0) as usize;
        let mut floors = self.room_sets[place.walls as usize].clone();
        for &r in rooms {
            if !floors.contains(&r) { floors.push(r); }
        }
        (0..=steps).all(|i| {
            let p = [0, 1, 2].map(|k| at[k] + (place.at[k] - at[k]) * i as f32 / steps as f32);
            floors.iter().any(|&room| world.floors_in(room, p[0], p[2]).iter().any(|y| (y - p[1]).abs() <= STEP))
                && floors.iter().all(|&r| world.wall_clearance_in(&Player::new(p), r) >= -0.5)
                && head_room(world, &floors, p)
        })
    }

    /// A way from `from` (in the rooms `start`) to the place farthest from `threat` that is
    /// no more than `reach` of walking away, to run to; none when it is nowhere on the grid
    pub fn away(&self, world: &CollisionWorld, from: V3, start: &[u32], threat: V3, reach: f32, shut: &[bool]) -> Option<Vec<(V3, Option<u32>)>> {
        let a = self.attach(world, from, start, &[])?;
        let far = |i: u32| {
            let p = self.places[i as usize].at;
            (p[0] - threat[0]).powi(2) + (p[2] - threat[2]).powi(2)
        };
        let mut came = vec![(u32::MAX, None); self.places.len()];
        let mut cost = vec![f32::INFINITY; self.places.len()];
        cost[a as usize] = 0.0;
        let mut best = a;
        let mut open = BinaryHeap::from([(Reverse(Key(0.0)), a)]);
        while let Some((Reverse(Key(here)), i)) = open.pop() {
            if here > cost[i as usize] {
                continue;
            }
            if far(i) > far(best) {
                best = i;
            }
            for (j, d, door) in self.neighbors(i as usize) {
                if !self.edge_clear(world, i, j, &[]) { continue; }
                let opening = door.is_some_and(|p| shut.get(p as usize).copied().unwrap_or(false));
                let c = here + d + if opening { SHUT_DOOR } else { 0.0 };
                if c <= reach && c < cost[j as usize] {
                    cost[j as usize] = c;
                    came[j as usize] = (i, door);
                    open.push((Reverse(Key(c)), j));
                }
            }
        }
        self.way_back(world, &came, best, self.places[best as usize].at, &[])
    }

    /// The way to place `b` that `came` says each place was reached from, ending at `to`
    fn way_back(&self, world: &CollisionWorld, came: &[(u32, Option<u32>)], b: u32, to: V3, leaves: &[usize]) -> Option<Vec<(V3, Option<u32>)>> {
        // Back from the end: each place, and the doorway on the way into it
        let mut way = vec![(b, None)];
        loop {
            let (before, door) = came[way.last()?.0 as usize];
            if before == u32::MAX {
                break;
            }
            if !self.edge_clear(world, before, way.last()?.0, leaves) { return None; }
            way.last_mut()?.1 = door;
            way.push((before, None));
        }
        way.reverse();
        let places: Vec<u32> = way.iter().map(|w| w.0).collect();
        // Straight on as far as there is floor all the way, in the rooms the way goes through,
        // and up to each doorway, found by doubling the reach and then halving; then back,
        // until a body fits all along (only the last leg is made sure of, as that takes longer)
        let mut points = vec![(self.places[way[0].0 as usize].at, None)];
        let mut i = 0;
        while i + 1 < way.len() {
            let mut j = i + 1;
            if way[j].1.is_none() {
                let last = (i + 2..way.len()).find(|&k| way[k].1.is_some()).map_or(way.len() - 1, |k| k - 1);
                let floored = |k: usize| self.walkable(None, &places[i..=k]);
                let mut step = 1;
                while j + step <= last && floored(j + step) {
                    j += step;
                    step *= 2;
                }
                let mut beyond = (j + step).min(last + 1);
                while beyond - j > 1 {
                    let mid = (j + beyond) / 2;
                    if floored(mid) {
                        j = mid;
                    } else {
                        beyond = mid;
                    }
                }
                while j > i + 1 && (!self.walkable(Some(world), &places[i..=j])
                    || !clear_leg(world, self.places[way[i].0 as usize].at, self.places[way[j].0 as usize].at, leaves)) {
                    j -= 1;
                }
            }
            points.push((self.places[way[j].0 as usize].at, way[j].1));
            i = j;
        }
        points.push((to, None));
        points.windows(2).all(|p| clear_leg(world, p[0].0, p[1].0, leaves)).then_some(points)
    }

    /// Whether the straight line from the first of `way` to its last has a place of the rooms
    /// along the way under it all along, at about the height between; and with the `world`, a
    /// body fits all along it: places are only where a body fits at the middle of each column,
    /// and a line between them can cut the corner of a wall
    fn walkable(&self, world: Option<&CollisionWorld>, way: &[u32]) -> bool {
        let (p, q) = (self.places[way[0] as usize].at, self.places[*way.last().unwrap() as usize].at);
        let rooms: Vec<u32> = way.iter().map(|&i| self.places[i as usize].room).collect();
        let rooms = &rooms;
        // Those under the line at `t` of the way along it
        let under = |t: f32| {
            let at = [0, 1, 2].map(|c| p[c] + (q[c] - p[c]) * t);
            let there = self.columns.get(&column(at[0], at[2])).map(Vec::as_slice).unwrap_or_default();
            (at, there.iter().map(|&i| &self.places[i as usize]).filter(move |s| rooms.contains(&s.room) && (s.at[1] - at[1]).abs() <= STEP))
        };
        let n = (dist(p, q) / (CELL * 0.5)).ceil() as usize;
        if !(1..n).all(|k| under(k as f32 / n as f32).1.next().is_some()) {
            return false;
        }
        let Some(world) = world else { return true };
        let n = (dist(p, q) / FIT_EVERY).ceil() as usize;
        (1..n).all(|k| {
            let (at, places) = under(k as f32 / n as f32);
            let mut any = false;
            places.into_iter().all(|s| {
                any = true;
                let feet = [at[0], s.at[1], at[2]];
                let walls = &self.room_sets[s.walls as usize];
                walls.iter().all(|&r| world.wall_clearance_in(&Player::new(feet), r) >= 0.0) && head_room(world, walls, feet)
            }) && any
        })
    }
}

fn clear_leg(world: &CollisionWorld, from: V3, to: V3, leaves: &[usize]) -> bool {
    world.pane_path_clear(from, to) && leaves.iter().all(|&d| world.door_path_clear(from, to, d))
}

/// Whether nothing that faces up, as a table's top, is over a body standing at `feet` in any of
/// `rooms`: such a face counts as floor, so no wall keeps a body out from under it
fn head_room(world: &CollisionWorld, rooms: &[u32], feet: V3) -> bool {
    let (r, top) = (world.params().radius, feet[1] + world.params().height);
    [(0.0, 0.0), (r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r)].iter().all(|&(dx, dz)| {
        rooms.iter().all(|&room| world.floors_in(room, feet[0] + dx, feet[2] + dz).iter().all(|&f| f <= feet[1] + STEP || f >= top))
    })
}

/// A cost that orders as a float does, for the heap
#[derive(Clone, Copy, PartialEq)]
struct Key(f32);

impl Eq for Key {}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::Params;

    #[test]
    fn glass_routes_follow_the_clients_pane_state() {
        if !std::path::Path::new("data/map/rm06/rm06.map").exists() { return; }
        let path = "data/map/rm06/rm06.map";
        let setup = crate::net::proto::Setup { map: path.to_string(), ..crate::net::lab::demo_setup() };
        let mut server = crate::net::setup::build_sim(&setup).unwrap();
        let mut client = crate::net::setup::build_sim(&setup).unwrap();
        let nav = crate::net::tactics::Navigation::for_map(&client, path, None);
        let grid = &nav.grid;
        let pane = client.panes.iter().position(|p| p.name == "15_glass02").unwrap();
        let tagged: Vec<_> = grid.pane_edges.iter().filter(|(_, panes)| panes.contains(&(pane as u32))).collect();
        assert!(!tagged.is_empty(), "the low window must remain in the structural graph");
        assert!(tagged.iter().any(|((i, j), _)| grid.neighbors(*i as usize).any(|(k, _, p)| k == *j && p == Some(69))));
        for &(&(i, j), _) in &tagged { assert!(!grid.edge_clear(&client.world, i, j, &[])); }
        let (from, to) = ([33220.0, 32767.0, 31460.0], [33220.0, 32767.0, 31620.0]);
        let (start, end) = (client.standing_rooms(from, None).0, client.standing_rooms(to, None).0);
        let (eye, aim) = ([from[0], from[1] + 150.0, from[2]], [to[0], to[1] + 120.0, to[2]]);
        assert!(!client.line_clear(eye, &start, aim, &end), "terrorists retain their glass sight rule");
        let touching = [33220.0, 32767.0, 31525.0];
        let touching_rooms = client.standing_rooms(touching, None).0;
        assert!(!grid.attached(&client.world, touching, &touching_rooms));
        let length = |way: &[(V3, Option<u32>)]| way.windows(2).map(|w| dist(w[0].0, w[1].0)).sum::<f32>();
        let closed = grid.path(&client.world, from, &start, to, &end, &[]).unwrap();
        assert!(closed.windows(2).all(|w| client.world.pane_path_clear(w[0].0, w[1].0)));
        let sound = [to[0], to[1] + 50.0, to[2]];
        let investigate = grid.investigate(&client.world, from, &start, sound, &end, &[]).unwrap();
        assert!(investigate.windows(2).all(|w| client.world.pane_path_clear(w[0].0, w[1].0)));
        let threat = [33220.0, 32767.0, 31000.0];
        let away = grid.away(&client.world, from, &start, threat, 250.0, &[]).unwrap();
        assert!(away.windows(2).all(|w| client.world.pane_path_clear(w[0].0, w[1].0)));
        server.broken_at[pane] = Some(1.0);
        server.world.switch_off("15_glass02");
        // A distant break does not change a client until its snapshot arrives.
        for &(&(i, j), _) in &tagged { assert!(!grid.edge_clear(&client.world, i, j, &[])); }
        crate::net::proto::Snapshot::capture(&server, 60, 0, 0).apply(&mut client, 0);
        for &(&(i, j), _) in &tagged { assert!(grid.edge_clear(&client.world, i, j, &[])); }
        assert!(client.line_clear(eye, &start, aim, &end));
        assert!(grid.attached(&client.world, touching, &touching_rooms));
        let open = grid.path(&client.world, from, &start, to, &end, &[]).unwrap();
        assert!(length(&open) + 100.0 < length(&closed), "closed={closed:?}, open={open:?}");
        assert!(open.windows(2).any(|w| client.world.panes_on_leg(w[0].0, w[1].0).contains(&(pane as u32))));
        let tree = grid.routes(&client.world, from, &start, &[]).unwrap();
        assert!(grid.path_from_tree(&client.world, &tree, to, &end).is_ok());
        let investigate = grid.investigate(&client.world, from, &start, sound, &end, &[]).unwrap();
        assert!(investigate.windows(2).any(|w| client.world.panes_on_leg(w[0].0, w[1].0).contains(&(pane as u32))));
        let away = grid.away(&client.world, from, &start, threat, 250.0, &[]).unwrap();
        assert!(away.windows(2).any(|w| client.world.panes_on_leg(w[0].0, w[1].0).contains(&(pane as u32))), "{away:?}");
        server.new_round(7);
        crate::net::proto::Snapshot::capture(&server, 120, 0, 0).apply(&mut client, 0);
        for &(&(i, j), _) in &tagged { assert!(!grid.edge_clear(&client.world, i, j, &[])); }
        // A tree made before the reset cannot publish its old path through the pane.
        assert!(grid.path_from_tree(&client.world, &tree, to, &end).is_err());
        let reset = grid.path(&client.world, from, &start, to, &end, &[]).unwrap();
        assert!((length(&reset) - length(&closed)).abs() < 1.0);
        println!("glass detour={:.1}, broken={:.1}, tagged={}", length(&closed), length(&open), tagged.len());
    }

    #[test]
    fn breaking_glass_does_not_change_the_structural_graph_or_open_a_high_sill() {
        let path = "data/map/m00/citystreet_large.map";
        let setup = crate::net::proto::Setup { map: path.to_string(), ..crate::net::lab::demo_setup() };
        let mut sim = crate::net::setup::build_sim(&setup).unwrap();
        let pane = sim.panes.iter().position(|p| p.name == "02_glass").unwrap() as u32;
        let intact = crate::net::tactics::Navigation::for_map(&sim, path, None);
        assert!(!intact.grid.pane_edges.values().any(|p| p.contains(&pane)), "this window's sill is 60 units high");
        for (i, pane) in sim.panes.iter().enumerate() {
            sim.broken_at[i] = Some(1.0);
            sim.world.switch_off(&pane.name);
        }
        let broken = crate::net::tactics::Navigation::for_map(&sim, path, None);
        assert!(intact.grid.points().eq(broken.grid.points()));
        assert!(intact.grid.transitions().eq(broken.grid.transitions()));
        assert_eq!(intact.grid.pane_edges, broken.grid.pane_edges);
    }

    #[test]
    #[ignore = "manual survey of every installed level"]
    fn survey_glass_navigation_on_all_levels() {
        let levels = crate::levels::list_levels();
        assert!(!levels.is_empty());
        for (_, name, path) in levels {
            let path = path.to_string_lossy().into_owned();
            let setup = crate::net::proto::Setup { map: path.clone(), ..crate::net::lab::demo_setup() };
            let mut sim = match crate::net::setup::build_sim(&setup) {
                Ok(sim) => sim,
                Err(error) => { println!("{name}: UNREADABLE {error:#}"); continue; }
            };
            let mut names = std::collections::BTreeSet::new();
            for pane in &sim.panes { assert!(names.insert(&pane.name), "{name}: duplicate pane name {}", pane.name); }
            let nav = crate::net::tactics::Navigation::for_map(&sim, &path, None);
            let grid = &nav.grid;
            assert!(sim.panes.is_empty() || grid.len() > 0, "{name}: panes need a seeded graph for this survey");
            let mut panes = std::collections::BTreeSet::new();
            let mut portals = std::collections::BTreeSet::new();
            let mut legs = 0;
            for i in 0..grid.len() {
                for (j, _, portal) in grid.neighbors(i) {
                    let required = sim.world.panes_on_leg(grid.places[i].at, grid.places[j as usize].at);
                    assert_eq!(sim.world.pane_path_clear(grid.places[i].at, grid.places[j as usize].at), required.is_empty());
                    let stored = grid.pane_edges.get(&(i as u32, j)).map(Vec::as_slice).unwrap_or_default();
                    assert!(required.iter().all(|p| stored.contains(p)), "{name} edge {i}->{j}");
                    assert!(portal.is_some() || stored == required.as_slice());
                    assert_eq!(grid.edge_clear(&sim.world, i as u32, j, &[]), stored.is_empty());
                    if !stored.is_empty() {
                        legs += 1;
                        panes.extend(stored.iter().map(|&p| sim.panes[p as usize].name.clone()));
                        portals.insert(portal);
                    }
                }
            }
            let sample: Vec<_> = grid.points().step_by((grid.len() / 16).max(1)).take(16).map(|(_, at, _)| at).collect();
            for (&from, &to) in sample.iter().zip(sample.iter().rev()) {
                assert_eq!(sim.world.pane_path_clear(from, to), sim.world.panes_on_leg(from, to).is_empty());
            }
            for pane in &sim.panes { sim.world.switch_off(&pane.name); }
            for &(i, j) in grid.pane_edges.keys() {
                assert!(grid.edge_clear(&sim.world, i, j, &[]));
                assert!(sim.world.pane_path_clear(grid.places[i as usize].at, grid.places[j as usize].at));
            }
            println!("{name}: panes={} nodes={} edges={} bytes={} affected={legs} portals={portals:?} names={panes:?}", sim.panes.len(), grid.len(), grid.edges.len(), nav.estimated_bytes());
        }
    }

    #[test]
    fn endpoints_must_be_on_the_right_floor_and_clear_of_walls() {
        let map = crate::demo::map();
        let world = CollisionWorld::from_map(&map, Params::default());
        let vis = Visibility::new(&map);
        let from = [0.0; 3];
        let rooms = world.floor_rooms(from);
        let grid = Grid::new(&world, &map, &vis, &[(from, rooms.clone())]);
        assert!(grid.path(&world, from, &rooms, [200.0, 0.0, 200.0], &rooms, &[]).is_some());
        for to in [[200.0, 300.0, 200.0], [499.0, 0.0, 200.0], [700.0, 0.0, 200.0]] {
            assert!(grid.path(&world, from, &rooms, to, &rooms, &[]).is_none(), "accepted {to:?}");
            assert!(grid.path(&world, to, &rooms, from, &rooms, &[]).is_none(), "accepted start {to:?}");
        }
    }

    #[test]
    fn a_directed_link_does_not_grant_a_path_back() {
        let world = CollisionWorld::from_map(&crate::demo::map(), Params::default());
        let room = world.floor_rooms([20.0, 0.0, 20.0])[0];
        let mut grid = Grid { places: vec![
            Place { at: [20.0, 0.0, 20.0], room, walls: 0 },
            Place { at: [60.0, 0.0, 20.0], room, walls: 0 }],
            links: vec![(0, 1), (1, 1)], edges: vec![Edge { node: 1, cost: 40.0, door: u32::MAX }], room_sets: vec![vec![room]],
            columns: HashMap::from([((0, 0), vec![0]), ((1, 0), vec![1])]), group: vec![], pane_edges: HashMap::new() };
        grid.join();
        let (a, b) = (grid.places[0].at, grid.places[1].at);
        assert!(grid.path(&world, a, &[room], b, &[room], &[]).is_some());
        assert!(grid.path(&world, b, &[room], a, &[room], &[]).is_none());
        assert!(!grid.distances(&world, b, &[room], &[])[0].is_finite());
    }

    #[test]
    fn citystreet_bases_are_connected_in_both_directions() {
        let path = std::path::Path::new("data/map/m00/citystreet_large.map");
        let Ok(level) = crate::map::read(path) else { return };
        let world = CollisionWorld::from_map(&level, Params::default());
        let vis = Visibility::new(&level);
        let bases = crate::levels::find_bases(path);
        let pair = &bases[0];
        let feet = |at: V3| [at[0], world.ground_at(at[0], at[2], at[1] + STEP).unwrap(), at[2]];
        let (blue, gold) = (feet(pair.blue.starts()[0]), feet(pair.gold.starts()[0]));
        let rooms = |p: V3| vis.rooms_holding(&level, [p[0], p[1] + 50.0, p[2]], None);
        for starts in [vec![blue], vec![gold], vec![blue, gold]] {
            let seeds: Vec<_> = starts.iter().map(|&p| (p, rooms(p))).collect();
            let grid = Grid::new(&world, &level, &vis, &seeds);
            for (from, to) in [(blue, gold), (gold, blue)] {
                let way = grid.path(&world, from, &rooms(from), to, &rooms(to), &[]);
                assert!(way.is_some(), "no path from {from:?} to {to:?}, seeded from {starts:?}");
                let way = way.unwrap();
                let last = way.iter().rev().nth(1).map_or(from, |p| p.0);
                assert!((last[1] - to[1]).abs() <= STEP, "wrong-floor attachment: {last:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn no_way_goes_under_a_table() {
        // rm02's room 100 has a block about 280 units square whose top (45 to 61 over the
        // floor) counts as floor, so no wall kept a body from walking through it
        let path = std::path::Path::new("data/map/rm02/rm02.map");
        let Ok(level) = crate::map::read(path) else { return };
        let world = CollisionWorld::from_map(&level, Params::default());
        let vis = Visibility::new(&level);
        let rooms = |p: V3| vis.rooms_holding(&level, [p[0], p[1] + 50.0, p[2]], None);
        let (from, to) = ([29300.0, 32767.0, 32900.0], [29960.0, 32767.0, 32720.0]);
        let grid = Grid::new(&world, &level, &vis, &[(from, rooms(from))]);
        let height = world.params().height;
        let under = |at: V3, f: f32| f > at[1] + STEP && f < at[1] + height;
        for p in &grid.places {
            for &r in &grid.room_sets[p.walls as usize] {
                let over = world.floors_in(r, p.at[0], p.at[2]).into_iter().find(|&f| under(p.at, f));
                assert!(over.is_none(), "a place at {:?} under a top of room {r} at {over:?}", p.at);
            }
        }
        // From beside the block to past it: a straight line would cross it
        let way = grid.path(&world, from, &rooms(from), to, &rooms(to), &[]).expect("a way across");
        let mut prev = from;
        for &(p, _) in &way {
            let n = (dist(prev, p) / 5.0).ceil() as usize;
            for k in 1..n {
                let t = k as f32 / n as f32;
                let at = [0, 1, 2].map(|c| prev[c] + (p[c] - prev[c]) * t);
                let Some(y) = world.ground_at(at[0], at[2], at[1] + STEP) else { continue };
                for r in rooms([at[0], y, at[2]]) {
                    let over = world.floors_in(r, at[0], at[2]).into_iter().find(|&f| under([at[0], y, at[2]], f));
                    assert!(over.is_none(), "{prev:?} -> {p:?} under a top of room {r} at {over:?}, at {at:?}");
                }
            }
            prev = p;
        }
    }
}
