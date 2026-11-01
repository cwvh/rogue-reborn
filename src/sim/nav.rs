//! Where a body can walk, for the characters to find their way: a grid of places across each
//! room's floors where a body fits among its walls, joined to the places beside them a step up
//! or down away, and from room to room through the doorways between. The rooms overlap (the
//! outside of a plane sits in its cabin), so each room's places are found among its own walls.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::collision::{CollisionWorld, Player};
use crate::map::Map;
use crate::visibility::Visibility;

type V3 = [f32; 3];

/// How far apart the places are, across the floor
const CELL: f32 = 40.0;
/// The most a step goes up or down from one place to the next
const STEP: f32 = 45.0;
/// The most places a level has, in case one spreads out of hand
const MAX_PLACES: usize = 600_000;
/// How often along a straight leg a body is made sure to fit
const FIT_EVERY: f32 = 10.0;
/// How much further a way may go to keep from opening a door (a guess: about as far as a
/// terrorist walks while a door opens)
const SHUT_DOOR: f32 = 400.0;

struct Place {
    at: V3,
    room: u32,
    /// Every room that holds it, whose walls a body passing it must keep clear of
    walls: Vec<u32>,
}

pub struct Grid {
    places: Vec<Place>,
    /// From each place: the next place, how far, and the doorway between if they are in
    /// different rooms (a portal, shut when its door is)
    links: Vec<Vec<(u32, f32, Option<u32>)>>,
    /// The places at each spot across the floor
    columns: HashMap<(i32, i32), Vec<u32>>,
    /// The places joined to each place share its group
    group: Vec<u32>,
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
        let mut grid = Grid { places: Vec::new(), links: Vec::new(), columns: HashMap::new(), group: Vec::new() };
        let mut index: HashMap<(u32, i32, i32, i32), u32> = HashMap::new();
        // A place in `room` over a column, a floor near `near` in height, if a body fits there
        let mut place = |grid: &mut Grid, room: u32, (cx, cz): (i32, i32), near: f32, reach: f32| -> Option<u32> {
            let (x, z) = ((cx as f32 + 0.5) * CELL, (cz as f32 + 0.5) * CELL);
            let y = world.floors_in(room, x, z).into_iter().filter(|y| (y - near).abs() <= reach)
                .min_by(|a, b| (a - near).abs().total_cmp(&(b - near).abs()))?;
            let key = (room, cx, cz, (y / 10.0).round() as i32);
            if let Some(&i) = index.get(&key) {
                return Some(i);
            }
            if grid.places.len() >= MAX_PLACES || world.wall_clearance_in(&Player::new([x, y, z]), room) < 0.0 {
                return None;
            }
            // Clear of the walls of the other rooms that hold it too, as a room inside another
            let mut walls: Vec<u32> = map.rooms.containing([x, y + 50.0, z]).into_iter()
                .filter_map(|r| map.rooms.rooms[r].section_name.parse().ok()).collect();
            if !walls.contains(&room) {
                walls.push(room);
            }
            if walls.iter().any(|&r| world.wall_clearance_in(&Player::new([x, y, z]), r) < 0.0) {
                return None;
            }
            let i = grid.places.len() as u32;
            index.insert(key, i);
            grid.places.push(Place { at: [x, y, z], room, walls });
            grid.links.push(Vec::new());
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
                    if dx == 0 || dz == 0 || grid.places[i as usize].walls.iter().all(|&r| world.wall_clearance_in(&half, r) >= 0.0) {
                        next.push((j, None));
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
                            next.push((j, Some(door as u32)));
                        }
                    }
                }
            }
            for (j, door) in next {
                let d = dist(at, grid.places[j as usize].at);
                grid.links[i as usize].push((j, d, door));
                todo.push(j);
            }
        }
        grid.join();
        grid
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
        for (i, links) in self.links.iter().enumerate() {
            for &(j, _, _) in links {
                let (a, b) = (root(&mut parent, i as u32), root(&mut parent, j));
                parent[a as usize] = b;
            }
        }
        self.group = (0..self.places.len() as u32).map(|i| root(&mut parent, i)).collect();
    }

    pub fn len(&self) -> usize {
        self.places.len()
    }

    /// The place nearest `at` (feet, or a point above them), among the rooms `rooms` if any of
    /// them has one near, else in any
    fn nearest(&self, at: V3, rooms: &[u32]) -> Option<u32> {
        let (cx, cz) = column(at[0], at[2]);
        let mut best: Option<(bool, f32, u32)> = None;
        for dx in -3..=3 {
            for dz in -3..=3 {
                for &i in self.columns.get(&(cx + dx, cz + dz)).map(Vec::as_slice).unwrap_or_default() {
                    let p = &self.places[i as usize];
                    // Not far above where it is wanted
                    if p.at[1] > at[1] + STEP {
                        continue;
                    }
                    let (outside, d) = (!rooms.contains(&p.room), dist(p.at, at));
                    if best.is_none_or(|(o, e, _)| (outside, d) < (o, e)) {
                        best = Some((outside, d, i));
                    }
                }
            }
        }
        best.map(|(_, _, i)| i)
    }

    /// A way to walk from `from` (in the rooms `start`) to `to` (in `end`): the places to make
    /// for in turn, as few as a straight walk between them allows, `to` last, each with the
    /// doorway (a portal) its leg goes through, if it does. A way stops before each doorway,
    /// to open its door if it is shut; a door `shut` costs a little more walk than an open
    /// way, which it takes when it is not much longer. None when the grid has nowhere near
    /// either, or nothing joins them.
    pub fn path(&self, world: &CollisionWorld, from: V3, start: &[u32], to: V3, end: &[u32], shut: &[bool]) -> Option<Vec<(V3, Option<u32>)>> {
        let (a, b) = (self.nearest(from, start)?, self.nearest(to, end)?);
        if self.group[a as usize] != self.group[b as usize] {
            return None;
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
            for &(j, d, door) in &self.links[i as usize] {
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
            return None;
        }
        self.way_back(world, &came, b, to)
    }

    /// A way from `from` (in the rooms `start`) to the place farthest from `threat` that is
    /// no more than `reach` of walking away, to run to; none when it is nowhere on the grid
    pub fn away(&self, world: &CollisionWorld, from: V3, start: &[u32], threat: V3, reach: f32, shut: &[bool]) -> Option<Vec<(V3, Option<u32>)>> {
        let a = self.nearest(from, start)?;
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
            for &(j, d, door) in &self.links[i as usize] {
                let opening = door.is_some_and(|p| shut.get(p as usize).copied().unwrap_or(false));
                let c = here + d + if opening { SHUT_DOOR } else { 0.0 };
                if c <= reach && c < cost[j as usize] {
                    cost[j as usize] = c;
                    came[j as usize] = (i, door);
                    open.push((Reverse(Key(c)), j));
                }
            }
        }
        self.way_back(world, &came, best, self.places[best as usize].at)
    }

    /// The way to place `b` that `came` says each place was reached from, ending at `to`
    fn way_back(&self, world: &CollisionWorld, came: &[(u32, Option<u32>)], b: u32, to: V3) -> Option<Vec<(V3, Option<u32>)>> {
        // Back from the end: each place, and the doorway on the way into it
        let mut way = vec![(b, None)];
        loop {
            let (before, door) = came[way.last()?.0 as usize];
            if before == u32::MAX {
                break;
            }
            way.last_mut()?.1 = door;
            way.push((before, None));
        }
        way.reverse();
        let places: Vec<u32> = way.iter().map(|w| w.0).collect();
        // Straight on as far as there is floor all the way, in the rooms the way goes through,
        // and up to each doorway, found by doubling the reach and then halving; then back,
        // until a body fits all along (only the last leg is made sure of, as that takes longer)
        let mut points = Vec::new();
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
                while j > i + 1 && !self.walkable(Some(world), &places[i..=j]) {
                    j -= 1;
                }
            }
            points.push((self.places[way[j].0 as usize].at, way[j].1));
            i = j;
        }
        points.push((to, None));
        Some(points)
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
                s.walls.iter().all(|&r| world.wall_clearance_in(&Player::new([at[0], s.at[1], at[2]]), r) >= 0.0)
            }) && any
        })
    }
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
