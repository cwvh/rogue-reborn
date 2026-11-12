//! Persistent cursors for bot queries. Static geometry is borrowed anew on each slice;
//! dynamic blockers are captured once and never read from the live world during a query.
use super::*;

#[derive(Clone, Debug)]
pub struct Blockers {
    panes: Vec<bool>,
    leaves: Vec<FrozenLeaf>,
}

/// A door leaf as a search saw it when it started.
#[derive(Clone, Copy, Debug)]
pub struct LeafState {
    pub door: usize,
    pub open: f32,
    pub opening: bool,
}

/// A leaf's bounds over its remaining motion, and its triangles if it was still.
#[derive(Clone, Debug)]
struct FrozenLeaf {
    door: usize,
    opening: bool,
    leaf: crate::collision::LeafSnapshot,
}

/// The node whose edges a flood is relaxing, and the next edge to try.
#[derive(Clone, Copy)]
struct Expanding {
    node: u32,
    cost: f32,
    edge: u32,
    end: u32,
}

impl Blockers {
    pub fn capture(
        world: &CollisionWorld,
        pane_count: usize,
        leaves: &[LeafState],
    ) -> Self {
        Self {
            panes: (0..pane_count as u32)
                .map(|p| world.pane_broken(p))
                .collect(),
            leaves: leaves
                .iter()
                .filter_map(|s| {
                    world
                        .freeze_leaf(s.door, s.open, if s.opening { 1.0 } else { 0.0 })
                        .map(|leaf| FrozenLeaf { door: s.door, opening: s.opening, leaf })
                })
                .collect(),
        }
    }

    pub fn changed(&self, other: &Self) -> bool {
        self.panes != other.panes
            || self.leaves.len() != other.leaves.len()
            || self
                .leaves
                .iter()
                .any(|a| !other.leaves.iter().any(|b| b.door == a.door && b.opening == a.opening))
    }

    fn doors_clear(&self, world: &CollisionWorld, a: V3, b: V3) -> bool {
        self.leaves
            .iter()
            .all(|f| f.leaf.clear(*world.params(), a, b))
    }

    pub fn clear(&self, world: &CollisionWorld, a: V3, b: V3) -> bool {
        self.doors_clear(world, a, b)
            && world
                .panes_on_leg(a, b)
                .iter()
                .all(|&p| self.panes[p as usize])
    }

    pub fn attach(
        &self,
        grid: &Grid,
        world: &CollisionWorld,
        feet: V3,
        rooms: &[u32],
    ) -> Option<u32> {
        grid.nearest_where(feet, rooms, |i| self.connects(grid, world, feet, i, rooms))
    }

    pub fn connects(
        &self,
        grid: &Grid,
        world: &CollisionWorld,
        feet: V3,
        node: u32,
        rooms: &[u32],
    ) -> bool {
        self.clear(world, feet, grid.places[node as usize].at)
            && grid.attachment(world, feet, node, rooms)
    }
}

impl Grid {
    pub fn at(&self, node: u32) -> V3 {
        self.places[node as usize].at
    }

    pub fn joined(&self, a: u32, b: u32) -> bool {
        self.group[a as usize] == self.group[b as usize]
    }

    /// Work units for one complete flood: resets, heap pops and edge relaxations.
    pub fn flood_units(&self) -> usize {
        self.len() / RESET_PER_UNIT + self.len() + self.edges.len()
    }
}

/// One reusable workspace. Initialization, heap pops and individual edge relaxations
/// all consume work. A yield in a node preserves the exact neighbor order.
pub struct Flood {
    pub paths: Paths,
    open: BinaryHeap<(Reverse<Key>, Key, u32)>,
    active: Option<Expanding>,
    start: u32,
    /// A goal turns the flood into A*, which stops once the goal leaves the heap.
    goal: Option<(u32, V3)>,
    initialized: usize,
}

/// Places reset per unit of work. Resetting is far cheaper than relaxing an edge.
const RESET_PER_UNIT: usize = 64;

impl Flood {
    pub fn new() -> Self {
        Self {
            paths: Paths {
                cost: Vec::new(),
                came: Vec::new(),
                leaves: Vec::new(),
            },
            open: BinaryHeap::new(),
            active: None,
            start: 0,
            goal: None,
            initialized: 0,
        }
    }

    pub fn reset(&mut self, start: u32, goal: Option<(u32, V3)>) {
        self.start = start;
        self.goal = goal;
        self.initialized = 0;
        self.active = None;
        self.open.clear();
    }

    pub fn bytes(&self) -> usize {
        self.paths.cost.capacity() * std::mem::size_of::<f32>()
            + self.paths.came.capacity() * std::mem::size_of::<(u32, Option<u32>)>()
            + self.open.capacity() * std::mem::size_of::<(Reverse<Key>, Key, u32)>()
    }

    fn heuristic(&self, grid: &Grid, node: u32) -> f32 {
        self.goal.map_or(0.0, |(_, at)| dist(grid.places[node as usize].at, at))
    }

    pub fn step(
        &mut self,
        grid: &Grid,
        world: &CollisionWorld,
        blockers: &Blockers,
        shut: &[bool],
        work: &mut usize,
    ) -> bool {
        while *work > 0 {
            *work -= 1;
            if self.initialized < grid.len() {
                let (from, to) = (self.initialized, (self.initialized + RESET_PER_UNIT).min(grid.len()));
                self.paths.cost.resize(self.paths.cost.len().max(to), f32::INFINITY);
                self.paths.came.resize(self.paths.came.len().max(to), (u32::MAX, None));
                self.paths.cost[from..to].fill(f32::INFINITY);
                self.paths.came[from..to].fill((u32::MAX, None));
                self.initialized = to;
                if self.initialized == grid.len() {
                    self.paths.cost.truncate(grid.len());
                    self.paths.came.truncate(grid.len());
                    self.paths.cost[self.start as usize] = 0.0;
                    let h = self.heuristic(grid, self.start);
                    self.open.push((Reverse(Key(h)), Key(0.0), self.start));
                }
                continue;
            }
            if let Some(Expanding { node: i, cost: here, edge, end }) = self.active {
                if edge == end {
                    self.active = None;
                    continue;
                }
                self.active = Some(Expanding { node: i, cost: here, edge: edge + 1, end });
                let e = &grid.edges[edge as usize];
                // Most maps have few glass legs and most searches no frozen leaves. Skip those
                // checks then: the hash and the extra place read cost more than the edge itself.
                if (!grid.pane_edges.is_empty() && grid
                    .pane_edges
                    .get(&(i, e.node))
                    .is_some_and(|p| p.iter().any(|&p| !blockers.panes[p as usize])))
                    || (!blockers.leaves.is_empty() && !blockers.doors_clear(
                        world,
                        grid.places[i as usize].at,
                        grid.places[e.node as usize].at,
                    ))
                {
                    continue;
                }
                let door = (e.door != u32::MAX).then_some(e.door);
                let c = here
                    + e.cost
                    + if door.is_some_and(|p| shut.get(p as usize).copied().unwrap_or(false)) {
                        SHUT_DOOR
                    } else {
                        0.0
                    };
                if c < self.paths.cost[e.node as usize] {
                    self.paths.cost[e.node as usize] = c;
                    self.paths.came[e.node as usize] = (i, door);
                    let h = self.heuristic(grid, e.node);
                    self.open.push((Reverse(Key(c + h)), Key(c), e.node));
                }
            } else {
                let Some((_, Key(here), i)) = self.open.pop() else {
                    return true;
                };
                if self.goal.is_some_and(|(g, _)| g == i) {
                    self.open.clear();
                    return true;
                }
                if here > self.paths.cost[i as usize] {
                    continue;
                }
                let (begin, end) = grid.links[i as usize];
                self.active = Some(Expanding { node: i, cost: here, edge: begin, end });
            }
        }
        false
    }
}

/// The floor and body-fit loops of one smoothing probe.
struct Fit {
    start: usize,
    end: usize,
    rooms: std::collections::HashSet<u32>,
    gather: usize,
    sample: usize,
    body: bool,
    with_body: bool,
}

impl Fit {
    fn new(start: usize, end: usize, with_body: bool) -> Self {
        Self {
            start,
            end,
            rooms: std::collections::HashSet::new(),
            gather: start,
            sample: 1,
            body: false,
            with_body,
        }
    }

    fn step(
        &mut self,
        grid: &Grid,
        world: &CollisionWorld,
        blockers: &Blockers,
        chain: &[(u32, Option<u32>)],
        work: &mut usize,
    ) -> Option<bool> {
        while *work > 0 {
            *work -= 1;
            if self.gather <= self.end {
                self.rooms
                    .insert(grid.places[chain[self.gather].0 as usize].room);
                self.gather += 1;
                continue;
            }
            let (p, q) = (
                grid.places[chain[self.start].0 as usize].at,
                grid.places[chain[self.end].0 as usize].at,
            );
            let n = (dist(p, q) / if self.body { FIT_EVERY } else { CELL * 0.5 }).ceil() as usize;
            if self.sample >= n {
                if self.body || !self.with_body {
                    return Some(true);
                }
                self.body = true;
                self.sample = 1;
                continue;
            }
            let at = [0, 1, 2].map(|k| p[k] + (q[k] - p[k]) * self.sample as f32 / n as f32);
            self.sample += 1;
            let mut any = false;
            for &i in grid
                .columns
                .get(&column(at[0], at[2]))
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let s = &grid.places[i as usize];
                if !self.rooms.contains(&s.room) || (s.at[1] - at[1]).abs() > STEP {
                    continue;
                }
                any = true;
                if self.body {
                    let feet = [at[0], s.at[1], at[2]];
                    let walls = &grid.room_sets[s.walls as usize];
                    if !walls
                        .iter()
                        .all(|&r| world.structural_clearance_in(&Player::new(feet), r) >= 0.0)
                        || !head_room(world, walls, feet)
                        || !blockers.clear(world, feet, feet)
                    {
                        return Some(false);
                    }
                }
            }
            if !any {
                return Some(false);
            }
        }
        None
    }
}

enum Phase {
    Parents(u32),
    Reverse(usize),
    Begin,
    Double,
    Binary,
    Backoff,
    Emit,
    Validate(usize),
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_paused_flood_keeps_its_door_and_pane_snapshot() {
        if !std::path::Path::new("data/map/rm16/rm16.map").exists() { return; }
        let path = "data/map/rm16/rm16.map";
        let mut sim = crate::net::setup::build_sim(&crate::net::proto::Setup {
            map: path.into(), ..crate::net::lab::demo_setup()
        }).unwrap();
        sim.set_doors_open(1.0);
        let nav = crate::net::tactics::Navigation::for_map(&sim,path,None);
        let leaves: Vec<_> = (0..sim.doors.len()).map(|door| LeafState { door, open: 1.0, opening: true }).collect();
        let blockers = Blockers::capture(&sim.world,sim.panes.len(),&leaves);
        let mut reference = Flood::new(); reference.reset(0, None);
        assert!(reference.step(&nav.grid,&sim.world,&blockers,&[],&mut (usize::MAX/2)));
        let mut sliced = Flood::new(); sliced.reset(0, None);
        assert!(!sliced.step(&nav.grid,&sim.world,&blockers,&[],&mut 1));
        for k in 0..sim.doors.len() { sim.world.set_door_open(k,0.0); }
        for pane in &sim.panes { sim.world.switch_off(&pane.name); }
        while !sliced.step(&nav.grid,&sim.world,&blockers,&[],&mut 97) {}
        assert_eq!(sliced.paths.cost.iter().map(|v|v.to_bits()).collect::<Vec<_>>(),reference.paths.cost.iter().map(|v|v.to_bits()).collect::<Vec<_>>());
        assert_eq!(sliced.paths.came,reference.paths.came);
    }
    #[test]
    fn sliced_flood_preserves_cost_bits_parents_and_portals() {
        for path in ["demo", "data/map/rm06/rm06.map"] {
            if path != "demo" && !std::path::Path::new(path).exists() { continue; }
            let sim = crate::net::setup::build_sim(&crate::net::proto::Setup {
                map: path.into(),
                ..crate::net::lab::demo_setup()
            })
            .unwrap();
            let nav = crate::net::tactics::Navigation::for_map(&sim, path, None);
            let grid = &nav.grid;
            let feet = grid.places[0].at;
            let rooms = sim.standing_rooms(feet, None).0;
            let blockers = Blockers::capture(&sim.world, sim.panes.len(), &[]);
            let start = blockers.attach(grid, &sim.world, feet, &rooms).unwrap();
            let reference = grid.routes(&sim.world, feet, &rooms, &[]).unwrap();
            for budget in [1, 97, 65536] {
                let mut flood = Flood::new();
                flood.reset(start, None);
                while !flood.step(grid, &sim.world, &blockers, &[], &mut { budget }) {}
                assert_eq!(
                    flood
                        .paths
                        .cost
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>(),
                    reference
                        .cost
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>()
                );
                assert_eq!(flood.paths.came, reference.came);
                for goal in (0..grid.len()).step_by((grid.len() / 8).max(1)) {
                    if !reference.cost[goal].is_finite() {
                        continue;
                    }
                    let to = grid.places[goal].at;
                    let mut route = Route::new(goal as u32, to);
                    let result = loop {
                        if let Some(result) =
                            route.step(grid, &sim.world, &blockers, &flood.paths, &mut { budget })
                        {
                            break result;
                        }
                    };
                    let expected = grid
                        .way_back(&sim.world, &reference.came, goal as u32, to, &[])
                        .ok_or(PathError::Disconnected);
                    assert_eq!(result, expected, "{path}, goal={goal}, budget={budget}");
                }
            }
        }
    }
}

/// Reconstruction and smoothing preserve the synchronous doubling, binary search and
/// backoff order. Long parent chains and fit probes can yield independently.
pub struct Route {
    stop: Option<u32>,
    chain: Vec<(u32, Option<u32>)>,
    points: Vec<(V3, Option<u32>)>,
    to: V3,
    phase: Phase,
    i: usize,
    j: usize,
    last: usize,
    stride: usize,
    beyond: usize,
    fit: Option<Fit>,
}

impl Route {
    pub fn new(goal: u32, to: V3) -> Self {
        Self {
            stop: None,
            chain: vec![(goal, None)],
            points: Vec::new(),
            to,
            phase: Phase::Parents(goal),
            i: 0,
            j: 0,
            last: 0,
            stride: 1,
            beyond: 0,
            fit: None,
        }
    }

    pub fn node(&self, index: usize) -> Option<u32> {
        self.chain.get(index).map(|p| p.0)
    }
    pub fn suffix(&self, start: u32) -> Self {
        let mut route = Self::new(self.chain.last().unwrap().0, self.to);
        route.stop = Some(start);
        route
    }

    pub fn step(
        &mut self,
        grid: &Grid,
        world: &CollisionWorld,
        blockers: &Blockers,
        paths: &Paths,
        work: &mut usize,
    ) -> Option<Result<Vec<(V3, Option<u32>)>, PathError>> {
        while *work > 0 {
            *work -= 1;
            match self.phase {
                Phase::Parents(node) => {
                    let (before, door) = paths.came[node as usize];
                    if before == u32::MAX || self.stop == Some(node) {
                        self.phase = Phase::Reverse(0);
                        continue;
                    }
                    if !blockers.clear(
                        world,
                        grid.places[before as usize].at,
                        grid.places[node as usize].at,
                    ) {
                        return Some(Err(PathError::Disconnected));
                    }
                    self.chain.last_mut().unwrap().1 = door;
                    self.chain.push((before, None));
                    self.phase = Phase::Parents(before);
                }
                Phase::Reverse(k) => {
                    if k < self.chain.len() / 2 {
                        let end = self.chain.len() - 1 - k;
                        self.chain.swap(k, end);
                        self.phase = Phase::Reverse(k + 1);
                    } else {
                        self.points
                            .push((grid.places[self.chain[0].0 as usize].at, None));
                        self.phase = Phase::Begin;
                    }
                }
                Phase::Begin => {
                    if self.i + 1 >= self.chain.len() {
                        self.points.push((self.to, None));
                        self.phase = Phase::Validate(0);
                        continue;
                    }
                    self.j = self.i + 1;
                    if self.chain[self.j].1.is_some() {
                        self.phase = Phase::Emit;
                        continue;
                    }
                    self.last = self.j;
                    self.phase = Phase::Double;
                    self.stride = 1;
                    // Find the next portal incrementally, using beyond as a cursor.
                    self.beyond = self.i + 2;
                }
                Phase::Double => {
                    if self.beyond < self.chain.len() {
                        if self.chain[self.beyond].1.is_some() {
                            self.last = self.beyond - 1;
                            self.beyond = self.chain.len();
                        } else {
                            self.last = self.beyond;
                            self.beyond += 1;
                        }
                        continue;
                    }
                    if self.j + self.stride > self.last {
                        self.beyond = (self.j + self.stride).min(self.last + 1);
                        self.phase = Phase::Binary;
                        continue;
                    }
                    let fit = self
                        .fit
                        .get_or_insert_with(|| Fit::new(self.i, self.j + self.stride, false));
                    *work += 1; // The nested cursor charges this unit itself.
                    match fit.step(grid, world, blockers, &self.chain, work) {
                        None => return None,
                        Some(true) => {
                            self.fit = None;
                            self.j += self.stride;
                            self.stride *= 2;
                        }
                        Some(false) => {
                            self.fit = None;
                            self.beyond = (self.j + self.stride).min(self.last + 1);
                            self.phase = Phase::Binary;
                        }
                    }
                }
                Phase::Binary => {
                    if self.beyond - self.j <= 1 {
                        self.phase = Phase::Backoff;
                        continue;
                    }
                    let mid = (self.j + self.beyond) / 2;
                    let fit = self.fit.get_or_insert_with(|| Fit::new(self.i, mid, false));
                    *work += 1;
                    match fit.step(grid, world, blockers, &self.chain, work) {
                        None => return None,
                        Some(ok) => {
                            self.fit = None;
                            if ok {
                                self.j = mid;
                            } else {
                                self.beyond = mid;
                            }
                        }
                    }
                }
                Phase::Backoff => {
                    if self.j <= self.i + 1 {
                        self.phase = Phase::Emit;
                        continue;
                    }
                    let fit = self
                        .fit
                        .get_or_insert_with(|| Fit::new(self.i, self.j, true));
                    *work += 1;
                    match fit.step(grid, world, blockers, &self.chain, work) {
                        None => return None,
                        Some(ok) => {
                            self.fit = None;
                            if ok
                                && blockers.clear(
                                    world,
                                    grid.places[self.chain[self.i].0 as usize].at,
                                    grid.places[self.chain[self.j].0 as usize].at,
                                )
                            {
                                self.phase = Phase::Emit;
                            } else {
                                self.j -= 1;
                            }
                        }
                    }
                }
                Phase::Emit => {
                    self.points.push((
                        grid.places[self.chain[self.j].0 as usize].at,
                        self.chain[self.j].1,
                    ));
                    self.i = self.j;
                    self.phase = Phase::Begin;
                }
                Phase::Validate(k) => {
                    if k + 1 >= self.points.len() {
                        self.phase = Phase::Done;
                        return Some(Ok(std::mem::take(&mut self.points)));
                    }
                    if !blockers.clear(world, self.points[k].0, self.points[k + 1].0) {
                        return Some(Err(PathError::Disconnected));
                    }
                    self.phase = Phase::Validate(k + 1);
                }
                Phase::Done => panic!("completed route polled again"),
            }
        }
        None
    }
}
