//! Where a body can walk, for the characters to find their way: a grid of places across each
//! room's floors where a body fits among its walls, joined to the places beside them a step up
//! or down away, and from room to room through the doorways between. The rooms overlap (the
//! outside of a plane sits in its cabin), so each room's places are found among its own walls.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::collision::{CollisionWorld, Player};
use crate::visibility::Visibility;

type V3 = [f32; 3];

/// How far apart the places are, across the floor
const CELL: f32 = 40.0;
/// The most a step goes up or down from one place to the next
const STEP: f32 = 45.0;
/// The most places a level has, in case one spreads out of hand
const MAX_PLACES: usize = 600_000;

struct Place {
    at: V3,
    room: u32,
}

pub struct Grid {
    places: Vec<Place>,
    /// From each place: the next place, how far, and the doorway between if they are in
    /// different rooms (a portal, shut when its door is)
    links: Vec<Vec<(u32, f32, Option<u32>)>>,
    /// The places at each spot across the floor
    columns: HashMap<(i32, i32), Vec<u32>>,
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
    pub fn new(world: &CollisionWorld, visibility: &Visibility, seeds: &[(V3, Vec<u32>)]) -> Grid {
        let mut grid = Grid { places: Vec::new(), links: Vec::new(), columns: HashMap::new() };
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
            let i = grid.places.len() as u32;
            index.insert(key, i);
            grid.places.push(Place { at: [x, y, z], room });
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
                    next.push((j, None));
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
        grid
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
    /// for in turn, as few as a straight walk between them allows, `to` last. None when the
    /// grid has nowhere near either, or no way between that is not `blocked` by a shut door.
    pub fn path(&self, from: V3, start: &[u32], to: V3, end: &[u32], blocked: &[bool]) -> Option<Vec<V3>> {
        let (a, b) = (self.nearest(from, start)?, self.nearest(to, end)?);
        let goal = self.places[b as usize].at;
        // A* across the places
        let mut came: HashMap<u32, u32> = HashMap::new();
        let mut cost: HashMap<u32, f32> = HashMap::from([(a, 0.0)]);
        let mut open = BinaryHeap::from([(Reverse(Key(dist(self.places[a as usize].at, goal))), a)]);
        while let Some((_, i)) = open.pop() {
            if i == b {
                break;
            }
            let here = cost[&i];
            for &(j, d, door) in &self.links[i as usize] {
                if door.is_some_and(|p| blocked.get(p as usize).copied().unwrap_or(false)) {
                    continue;
                }
                let c = here + d;
                if cost.get(&j).is_none_or(|&old| c < old) {
                    cost.insert(j, c);
                    came.insert(j, i);
                    open.push((Reverse(Key(c + dist(self.places[j as usize].at, goal))), j));
                }
            }
        }
        if a != b && !came.contains_key(&b) {
            return None;
        }
        let mut way = vec![b];
        while let Some(&before) = came.get(way.last()?) {
            way.push(before);
        }
        way.reverse();
        // Straight on as far as there is floor all the way, in the rooms the way goes through
        let mut points = Vec::new();
        let mut i = 0;
        while i + 1 < way.len() {
            let mut j = i + 1;
            while j + 1 < way.len() && self.walkable(&way[i..=j + 1]) {
                j += 1;
            }
            points.push(self.places[way[j] as usize].at);
            i = j;
        }
        points.push(to);
        Some(points)
    }

    /// Whether the straight line from the first of `way` to its last has a place of the rooms
    /// along the way under it all along, at about the height between
    fn walkable(&self, way: &[u32]) -> bool {
        let (p, q) = (self.places[way[0] as usize].at, self.places[*way.last().unwrap() as usize].at);
        let rooms: Vec<u32> = way.iter().map(|&i| self.places[i as usize].room).collect();
        let n = (dist(p, q) / (CELL * 0.5)).ceil() as usize;
        (1..n).all(|k| {
            let t = k as f32 / n as f32;
            let at = [0, 1, 2].map(|c| p[c] + (q[c] - p[c]) * t);
            let (cx, cz) = column(at[0], at[2]);
            self.columns.get(&(cx, cz)).is_some_and(|there| there.iter().any(|&i| {
                let s = &self.places[i as usize];
                rooms.contains(&s.room) && (s.at[1] - at[1]).abs() <= STEP
            }))
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
