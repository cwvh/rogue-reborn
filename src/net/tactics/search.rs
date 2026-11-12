use super::*;
use crate::sim::nav::{Blockers, Flood, LeafState, Route};
use std::sync::{Arc, Mutex, Weak};

/// Work units per bot tick.
const BUDGET: usize = 65536;
/// How far from a found path's first node the bot may be to take it.
const SPLICE: f32 = 80.0;
/// Path left when the next destination search starts. About 1.4 s at a run.
const AHEAD: f32 = 1000.0;

// Workspaces for the searches of the bots sharing one map. Queued requests own only small
// decision snapshots.
pub struct Pool(Mutex<PoolData>);
struct PoolData {
    /// Idle workspaces, each with its slot number.
    free: Vec<(usize, Flood)>,
    /// The capacity each slot's workspace has reached. It never shrinks.
    bytes: Vec<usize>,
    queue: VecDeque<Weak<()>>,
}
impl Pool {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(PoolData {
            free: vec![(0, Flood::new())],
            bytes: vec![0],
            queue: VecDeque::new(),
        })))
    }
    /// Searches that can run at once. Each slot holds its own flood arrays.
    pub fn grow(&self, slots: usize) {
        let mut pool = self.0.lock().unwrap();
        while pool.bytes.len() < slots {
            let slot = pool.bytes.len();
            pool.free.push((slot, Flood::new()));
            pool.bytes.push(0);
        }
    }
    /// Flood arrays and heaps retained across every slot, busy or idle.
    pub fn retained_bytes(&self) -> usize {
        self.0.lock().unwrap().bytes.iter().sum()
    }
    /// The capacity each slot's workspace has reached.
    #[cfg(test)]
    pub fn slot_bytes(&self) -> Vec<usize> {
        self.0.lock().unwrap().bytes.clone()
    }
    fn record(&self, slot: usize, flood: &Flood) {
        let mut pool = self.0.lock().unwrap();
        pool.bytes[slot] = pool.bytes[slot].max(flood.bytes());
    }
    fn ticket(self: &Arc<Self>) -> Ticket {
        let id = Arc::new(());
        self.0.lock().unwrap().queue.push_back(Arc::downgrade(&id));
        Ticket {
            pool: self.clone(),
            id,
            workspace: None,
        }
    }
}
struct Ticket {
    pool: Arc<Pool>,
    id: Arc<()>,
    workspace: Option<(usize, Flood)>,
}
impl Ticket {
    fn admit(&mut self) -> bool {
        if self.workspace.is_some() {
            return true;
        }
        let mut pool = self.pool.0.lock().unwrap();
        while pool.queue.front().is_some_and(|id| id.strong_count() == 0) {
            pool.queue.pop_front();
        }
        if !pool.free.is_empty()
            && pool
                .queue
                .front()
                .is_some_and(|id| id.ptr_eq(&Arc::downgrade(&self.id)))
        {
            pool.queue.pop_front();
            self.workspace = pool.free.pop();
            return true;
        }
        false
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        if let Some((slot, flood)) = self.workspace.take() {
            self.pool.record(slot, &flood);
            self.pool.0.lock().unwrap().free.push((slot, flood));
        }
    }
}

pub struct Job {
    ticket: Ticket,
    start: V3,
    requested: Option<V3>,
    /// Searching from the end of the current path while the bot still walks it.
    ahead: bool,
    captured: bool,
    blockers: Option<Blockers>,
    validation: Option<Blockers>,
    relaxed: bool,
    retry: bool,
    active_ticks: usize,
    phase: Phase,
    scores: Vec<(usize, f32)>,
    candidate: usize,
    route: Option<Route>,
    goal_node: Option<u32>,
    failures: Vec<usize>,
    now: f32,
    preference: usize,
    cooldown: Vec<f32>,
    visited: Vec<Option<f32>>,
    completed: Vec<Option<f32>>,
    here: Option<usize>,
    previous: Option<usize>,
}
enum Phase {
    Start,
    Flood,
    Candidates(usize),
    Route,
    Publish(Vec<(V3, Option<u32>)>, usize),
    Splice(Vec<(V3, Option<u32>)>, usize),
    Failed,
}
enum Progress {
    Pending,
    Complete(Found),
    Failed(Reason),
}

/// A route a search found.
pub struct Found {
    path: Vec<(V3, Option<u32>)>,
    /// The destination patch, for an exploration search.
    goal: Option<usize>,
    /// Whether failed doors were let back in to find it.
    relaxed: bool,
    /// The grid node the path starts from.
    first: u32,
}

impl Job {
    pub fn new(
        map: &Navigation,
        nav: &Navigator,
        feet: V3,
        now: f32,
        preference: usize,
        requested: Option<V3>,
    ) -> Self {
        Self {
            ticket: map.searches.ticket(),
            start: feet,
            requested,
            ahead: false,
            captured: false,
            blockers: None,
            validation: None,
            relaxed: requested.is_some() && nav.relaxed_doors,
            retry: false,
            active_ticks: 0,
            phase: Phase::Start,
            scores: Vec::new(),
            candidate: 0,
            route: None,
            goal_node: None,
            failures: Vec::new(),
            now,
            preference,
            cooldown: nav.cooldown.clone(),
            visited: nav.visited.clone(),
            completed: nav.completed.clone(),
            here: nav.here,
            previous: nav.previous,
        }
    }

    /// Choose the next destination from the end of the current path. Patches on the way
    /// count as visited, and the current goal is the starting patch.
    pub fn ahead(map: &Navigation, nav: &Navigator, sim: &Sim, end: V3, now: f32, preference: usize) -> Self {
        let mut job = Self::new(map, nav, end, now, preference, None);
        job.ahead = true;
        for &(at, _) in &nav.path {
            for p in map.patches_at(sim, at) {
                job.visited[p] = Some(now);
            }
        }
        job.previous = nav.here;
        job.here = nav.goal;
        job
    }

    fn snapshot(&self, nav: &Navigator, sim: &Sim, feet: V3, now: f32) -> Blockers {
        Self::blockers_now(nav, sim, feet, now, self.relaxed)
    }

    fn blockers_now(nav: &Navigator, sim: &Sim, feet: V3, now: f32, relaxed: bool) -> Blockers {
        let mut leaves = Navigator::observed_leaves(sim, feet);
        if !relaxed {
            for d in &nav.avoided_doors {
                if d.until > now && !leaves.contains(&d.door) {
                    leaves.push(d.door);
                }
            }
        }
        leaves.sort_unstable();
        Blockers::capture(
            &sim.world,
            sim.panes.len(),
            &leaves
                .into_iter()
                .map(|door| LeafState { door, open: sim.door_states[door].open, opening: sim.door_states[door].opening })
                .collect::<Vec<_>>(),
        )
    }

    fn restart(&mut self, feet: V3) -> bool {
        // A search ahead starts at the path's end, not the moving feet. It fails instead.
        if self.retry || self.ahead {
            return false;
        }
        self.retry = true;
        self.start = feet;
        self.captured = false;
        self.phase = Phase::Start;
        self.scores.clear();
        self.candidate = 0;
        self.route = None;
        self.goal_node = None;
        self.failures.clear();
        true
    }

    fn step(
        &mut self,
        map: &Navigation,
        nav: &Navigator,
        sim: &Sim,
        feet: V3,
        now: f32,
        work: &mut usize,
    ) -> Progress {
        if !self.ticket.admit() {
            return Progress::Pending;
        }
        self.active_ticks += 1;
        // At most three floods: the first, a restart and the relaxed pass. Allow four.
        let floods = 4 * (map.grid.flood_units() / BUDGET + 1);
        if self.active_ticks > floods + (3.0 / crate::sim::TICK) as usize {
            return Progress::Failed(Reason::TimedOut);
        }
        let current = self.snapshot(nav, sim, feet, now);
        if self.blockers.as_ref().is_some_and(|b| b.changed(&current)) && !self.retry && !self.ahead {
            self.restart(feet);
        }
        if !self.captured {
            self.blockers = Some(current.clone());
            self.captured = true;
        }
        while *work > 0 {
            *work -= 1;
            let blockers = self.blockers.as_ref().unwrap();
            let flood = &mut self.ticket.workspace.as_mut().unwrap().1;
            match &mut self.phase {
                Phase::Start => {
                    let Some(start) = blockers.attach(
                        &map.grid,
                        &sim.world,
                        self.start,
                        &sim.standing_rooms(self.start, None).0,
                    ) else {
                        return Progress::Failed(Reason::StartDetached);
                    };
                    // A requested point needs only the way there. A* stops at its node.
                    let goal = match self.requested {
                        Some(to) => {
                            let rooms = sim.standing_rooms(to, None).0;
                            let Some(goal) = blockers.attach(&map.grid, &sim.world, to, &rooms) else {
                                return Progress::Failed(Reason::GoalDetached);
                            };
                            if !map.grid.joined(start, goal) {
                                return Progress::Failed(Reason::NoRoute);
                            }
                            self.goal_node = Some(goal);
                            Some((goal, map.grid.at(goal)))
                        }
                        None => None,
                    };
                    flood.reset(start, goal);
                    self.phase = Phase::Flood;
                }
                Phase::Flood => {
                    *work += 1;
                    if !flood.step(&map.grid, &sim.world, blockers, &[], work) {
                        return Progress::Pending;
                    }
                    self.phase = Phase::Candidates(0);
                }
                Phase::Candidates(i) => {
                    if let Some(to) = self.requested {
                        let goal = self.goal_node.unwrap();
                        if !flood.paths.cost[goal as usize].is_finite() {
                            return Progress::Failed(Reason::NoRoute);
                        }
                        self.route = Some(Route::new(goal, to));
                        self.phase = Phase::Route;
                        continue;
                    }
                    if *i == map.patches.len() {
                        self.phase = Phase::Route;
                        continue;
                    }
                    let k = *i;
                    *i += 1;
                    let p = &map.patches[k];
                    let cost = flood.paths.cost[p.node as usize];
                    if Some(k) == self.here || self.cooldown[k] > self.now || !cost.is_finite() {
                        continue;
                    }
                    let last = self.visited[k]
                        .into_iter()
                        .chain(self.completed[k])
                        .max_by(f32::total_cmp);
                    let age = last.map_or(self.now + UNVISITED, |t| self.now - t);
                    let variety =
                        ((k.wrapping_mul(2654435761) ^ self.preference) % 100) as f32 * VARIETY;
                    let score = age
                        - cost
                            / crate::sim::movement::ground_speed(
                                crate::sim::movement::Movement::Run,
                            )
                        + variety
                        - if Some(k) == self.previous {
                            REVERSE_COST
                        } else {
                            0.0
                        };
                    self.scores.push((k, score));
                    self.scores
                        .sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                    self.scores.truncate(CANDIDATES);
                }
                Phase::Route => {
                    if self.route.is_none() {
                        let Some(&(goal, _)) = self.scores.get(self.candidate) else {
                            if !self.relaxed && nav.avoided_doors.iter().any(|d| d.until > now) {
                                self.relaxed = true;
                                self.captured = false;
                                self.blockers = None;
                                self.scores.clear();
                                self.candidate = 0;
                                self.failures.clear();
                                self.phase = Phase::Start;
                                // Capture the relaxed pass now, without spending another tick's timeout.
                                self.blockers = Some(self.snapshot(nav, sim, feet, now));
                                self.captured = true;
                                continue;
                            }
                            self.phase = Phase::Failed;
                            continue;
                        };
                        let to = map.patches[goal].at;
                        let Some(node) = blockers.attach(
                            &map.grid,
                            &sim.world,
                            to,
                            &sim.standing_rooms(to, None).0,
                        ) else {
                            self.failures.push(goal);
                            self.candidate += 1;
                            continue;
                        };
                        if !flood.paths.cost[node as usize].is_finite() {
                            self.failures.push(goal);
                            self.candidate += 1;
                            continue;
                        }
                        self.route = Some(Route::new(node, to));
                    }
                    *work += 1;
                    match self.route.as_mut().unwrap().step(
                        &map.grid,
                        &sim.world,
                        blockers,
                        &flood.paths,
                        work,
                    ) {
                        None => return Progress::Pending,
                        Some(Ok(points)) => {
                            self.validation = None;
                            self.phase = Phase::Publish(points, 0);
                        }
                        Some(Err(_)) => {
                            self.route = None;
                            if self.requested.is_some() {
                                return Progress::Failed(Reason::NoRoute);
                            }
                            self.failures.push(self.scores[self.candidate].0);
                            self.candidate += 1;
                        }
                    }
                }
                Phase::Publish(points, k) => {
                    if self.validation.as_ref().is_none_or(|b| b.changed(&current)) {
                        *k = 0;
                        self.validation = Some(current.clone());
                    }
                    // Validate one leg per unit against current observed state before publication.
                    if *k + 1 < points.len() {
                        if !current.clear(&sim.world, points[*k].0, points[*k + 1].0) {
                            if self.restart(feet) {
                                return Progress::Pending;
                            }
                            return Progress::Failed(Reason::RouteChanged);
                        }
                        *k += 1;
                        continue;
                    }
                    self.phase = Phase::Splice(std::mem::take(points), 0);
                }
                Phase::Splice(points, k) => {
                    if self.ahead {
                        let first = self.route.as_ref().unwrap().node(0).unwrap();
                        let goal = self.scores.get(self.candidate).map(|c| c.0);
                        return Progress::Complete(Found { path: std::mem::take(points), goal, relaxed: self.relaxed, first });
                    }
                    if self.validation.as_ref().is_some_and(|b| b.changed(&current)) {
                        self.phase = Phase::Publish(std::mem::take(points),0);
                        continue;
                    }
                    // Rejoin the raw node chain, so sparse smoothed endpoints cannot prevent
                    // publication while the body is standing on a long smoothed leg.
                    if flat(feet, self.start) > 1.0 || (feet[1] - self.start[1]).abs() > 1.0 {
                        let route = self.route.as_ref().unwrap();
                        if let Some(node) = route.node(*k) {
                            *k += 1;
                            let at = map.grid.at(node);
                            if flat(feet, at) <= SPLICE
                                && current.connects(
                                    &map.grid,
                                    &sim.world,
                                    feet,
                                    node,
                                    &sim.standing_rooms(feet, None).0,
                                )
                            {
                                self.route = Some(route.suffix(node));
                                self.start = feet;
                                self.phase = Phase::Route;
                            }
                            continue;
                        }
                        if self.restart(feet) {
                            return Progress::Pending;
                        }
                        return Progress::Failed(Reason::StartMoved);
                    }
                    let first = self.route.as_ref().unwrap().node(0).unwrap();
                    if !current.connects(&map.grid,&sim.world,feet,first,&sim.standing_rooms(feet,None).0) {
                        if self.restart(feet) { return Progress::Pending; }
                        return Progress::Failed(Reason::StartNoLongerAttaches);
                    }
                    let goal = self
                        .requested
                        .is_none()
                        .then(|| self.scores[self.candidate].0);
                    return Progress::Complete(Found { path: std::mem::take(points), goal, relaxed: self.relaxed, first });
                }
                Phase::Failed => return Progress::Failed(Reason::NoDestination),
            }
        }
        Progress::Pending
    }

    pub fn finish_cooldowns(&self, nav: &mut Navigator, now: f32) {
        if !self.retry {
            for &k in &self.failures {
                nav.cooldown[k] = now + FAILED_AGAIN;
            }
        }
    }
}

impl Navigator {
    pub fn request(
        &mut self,
        map: &Navigation,
        feet: V3,
        now: f32,
        preference: usize,
        to: Option<V3>,
    ) {
        self.pending = Some(Job::new(map, self, feet, now, preference, to));
        self.path.clear();
        self.door = None;
    }

    /// Start choosing the next destination while the current path's last stretch is walked.
    pub fn look_ahead(&mut self, map: &Navigation, sim: &Sim, feet: V3, now: f32, preference: usize) {
        if self.pending.is_some() || self.ahead.is_some() || self.pursuit.is_some() || self.goal.is_none() {
            return;
        }
        let Some(&(end, _)) = self.path.back() else { return };
        let mut left = 0.0;
        let mut from = feet;
        for &(at, _) in &self.path {
            left += flat(from, at);
            from = at;
        }
        if left <= AHEAD {
            self.pending = Some(Job::ahead(map, self, sim, end, now, preference));
        }
    }

    /// Take the path found ahead, if the bot stands at its start and it is still clear.
    pub fn adopt_ahead(&mut self, map: &Navigation, sim: &Sim, feet: V3, now: f32) -> bool {
        let clock = Instant::now();
        let adopted = self.adopt_ahead_inner(map, sim, feet, now);
        self.max_search_ms = self.max_search_ms.max(clock.elapsed().as_secs_f64() * 1000.0);
        adopted
    }

    // The check runs within one tick, so the blockers cannot change partway. A smoothed path has
    // few legs: at most 5 on rm08 and City Street Large, checked in under 2 microseconds.
    fn adopt_ahead_inner(&mut self, map: &Navigation, sim: &Sim, feet: V3, now: f32) -> bool {
        let Some(ahead) = self.ahead.take() else { return false };
        if ahead.goal.is_some_and(|g| self.cooldown[g] > now) {
            return false;
        }
        // The same reach a moving start may splice to.
        let first = map.grid.at(ahead.first);
        if flat(feet, first) > SPLICE || (feet[1] - first[1]).abs() > ARRIVE_Y {
            return false;
        }
        let current = Job::blockers_now(self, sim, feet, now, ahead.relaxed);
        let rooms = sim.standing_rooms(feet, None).0;
        if !current.connects(&map.grid, &sim.world, feet, ahead.first, &rooms)
            || !ahead.path.windows(2).all(|w| current.clear(&sim.world, w[0].0, w[1].0))
        {
            return false;
        }
        self.path = ahead.path.into();
        self.goal = ahead.goal;
        self.relaxed_doors = ahead.relaxed;
        self.progress = (now, f32::INFINITY);
        self.fallback = false;
        self.retries = 0;
        self.replan_at = None;
        self.reason = Reason::Exploring;
        true
    }

    pub fn advance_search(
        &mut self,
        map: &Navigation,
        sim: &Sim,
        feet: V3,
        now: f32,
    ) -> bool {
        let Some(mut job) = self.pending.take() else {
            return false;
        };
        let clock = Instant::now();
        let result = job.step(map, self, sim, feet, now, &mut { BUDGET });
        if let Some((slot, flood)) = &job.ticket.workspace {
            job.ticket.pool.record(*slot, flood);
        }
        self.max_search_ms = self
            .max_search_ms
            .max(clock.elapsed().as_secs_f64() * 1000.0);
        if job.ahead {
            // The bot keeps walking. It waits only if it arrives before the search ends.
            match result {
                Progress::Pending => {
                    self.pending = Some(job);
                    if self.path.is_empty() {
                        self.reason = Reason::SearchPending;
                        return true;
                    }
                }
                Progress::Complete(found) => {
                    job.finish_cooldowns(self, now);
                    self.ahead = Some(found);
                }
                Progress::Failed(_) => {}
            }
            return false;
        }
        match result {
            Progress::Pending => {
                self.pending = Some(job);
                self.fallback = false;
                self.reason = Reason::SearchPending;
                true
            }
            Progress::Complete(Found { path, goal, relaxed, .. }) => {
                job.finish_cooldowns(self, now);
                self.path = path.into();
                self.goal = goal.or(self.goal);
                self.relaxed_doors = relaxed;
                self.progress = (now, f32::INFINITY);
                self.fallback = false;
                self.reason = Reason::Exploring;
                if job.requested.is_none() {
                    self.retries = 0;
                    self.replan_at = None;
                }
                false
            }
            Progress::Failed(reason) => {
                if reason == Reason::NoDestination {
                    job.finish_cooldowns(self, now);
                }
                self.next_search = now + SEARCH_AGAIN;
                self.reason = reason;
                self.fallback = true;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{lab::demo_setup, setup::build_sim};

    #[test]
    fn sliced_selection_matches_the_synchronous_reference() {
        let sim = build_sim(&demo_setup()).unwrap();
        let map = Navigation::for_map(&sim, "demo", None);
        let feet = [0.0; 3];
        let mut reference = Navigator::new(&map);
        reference.search(&map, &sim, feet, 1.0, 42);
        let nav = Navigator::new(&map);
        for budget in [1024, 65536, usize::MAX / 2] {
            let mut job = Job::new(&map, &nav, feet, 1.0, 42, None);
            loop {
                match job.step(&map, &nav, &sim, feet, 1.0, &mut { budget }) {
                    Progress::Pending => {}
                    Progress::Complete(Found { path, goal, .. }) => {
                        assert_eq!(goal, reference.goal);
                        assert_eq!(VecDeque::from(path), reference.path);
                        break;
                    }
                    Progress::Failed(reason) => panic!("{reason}"),
                }
            }
        }
    }

    #[test]
    fn a_requested_route_matches_the_synchronous_a_star() {
        for path in ["demo", "data/map/rm06/rm06.map"] {
            if path != "demo" && !std::path::Path::new(path).exists() { continue; }
            let sim = build_sim(&crate::net::proto::Setup { map: path.into(), ..demo_setup() }).unwrap();
            let map = Navigation::for_map(&sim, path, None);
            let nav = Navigator::new(&map);
            let points: Vec<_> = map.grid.points().map(|p| p.1).collect();
            let from = points[0];
            let start = sim.standing_rooms(from, None).0;
            let mut compared = 0;
            for &to in points.iter().step_by((points.len() / 12).max(1)) {
                let end = sim.standing_rooms(to, None).0;
                let expected = map.grid.path_past_doors(&sim.world, from, &start, to, &end, &[], &[]);
                let mut job = Job::new(&map, &nav, from, 1.0, 42, Some(to));
                let result = loop {
                    match job.step(&map, &nav, &sim, from, 1.0, &mut 97) {
                        Progress::Pending => job.active_ticks = 0,
                        Progress::Complete(found) => break Ok(found.path),
                        Progress::Failed(reason) => break Err(reason),
                    }
                };
                match (expected, result) {
                    (Ok(a), Ok(b)) => { assert_eq!(a, b, "{path} to {to:?}"); compared += 1; }
                    (Err(_), Err(_)) => {}
                    (a, b) => panic!("{path} to {to:?}: synchronous {a:?}, sliced {b:?}"),
                }
            }
            assert!(compared >= 6, "{path}: {compared}");
        }
    }

    #[test]
    fn a_requested_route_stops_before_flooding_the_whole_map() {
        if !std::path::Path::new("data/map/rm06/rm06.map").exists() { return; }
        let path = "data/map/rm06/rm06.map";
        let sim = build_sim(&crate::net::proto::Setup { map: path.into(), ..demo_setup() }).unwrap();
        let map = Navigation::for_map(&sim, path, None);
        let nav = Navigator::new(&map);
        let from = map.grid.points().next().unwrap().1;
        let near = map.grid.points().map(|p| p.1).filter(|&p| flat(p, from) > 200.0 && (p[1] - from[1]).abs() < 1.0)
            .min_by(|a, b| flat(*a, from).total_cmp(&flat(*b, from))).unwrap();
        let mut job = Job::new(&map, &nav, from, 1.0, 42, Some(near));
        let mut ticks = 0;
        while matches!(job.step(&map, &nav, &sim, from, 1.0, &mut { BUDGET }), Progress::Pending) { ticks += 1; }
        assert!(ticks * BUDGET < map.grid.flood_units() / 4, "{ticks} ticks");
    }

    #[test]
    fn the_next_destination_is_found_while_walking_and_taken_on_arrival() {
        let sim = build_sim(&demo_setup()).unwrap();
        let map = Navigation::for_map(&sim, "demo", None);
        let mut nav = Navigator::new(&map);
        let feet = [0.0; 3];
        nav.request(&map, feet, 1.0, 42, None);
        while nav.advance_search(&map, &sim, feet, 1.0) {}
        let first = nav.goal.unwrap();
        let end = nav.path.back().unwrap().0;
        // Stand on the last leg, then start the search from the path's end.
        let near = nav.path.iter().rev().nth(1).map_or(feet, |p| p.0);
        while nav.path.len() > 1 { nav.path.pop_front(); }
        nav.look_ahead(&map, &sim, near, 2.0, 42);
        assert!(nav.pending.as_ref().is_some_and(|j| j.ahead));
        let mut ticks = 0;
        while nav.pending.is_some() {
            assert!(!nav.advance_search(&map, &sim, near, 2.0), "a bot with a path keeps walking");
            ticks += 1;
            assert!(ticks < 1000);
        }
        assert!(nav.ahead.is_some());
        nav.path.clear();
        nav.goal = None;
        nav.cooldown[first] = 4.0;
        assert!(nav.adopt_ahead(&map, &sim, end, 3.0));
        assert!(!nav.path.is_empty());
        assert_ne!(nav.goal, Some(first));
    }

    #[test]
    fn a_path_found_ahead_is_dropped_away_from_its_start() {
        let sim = build_sim(&demo_setup()).unwrap();
        let map = Navigation::for_map(&sim, "demo", None);
        let mut nav = Navigator::new(&map);
        nav.request(&map, [0.0; 3], 1.0, 42, None);
        while nav.advance_search(&map, &sim, [0.0; 3], 1.0) {}
        let end = nav.path.back().unwrap().0;
        nav.pending = Some(Job::ahead(&map, &nav, &sim, end, 2.0, 42));
        while nav.pending.is_some() { nav.advance_search(&map, &sim, [0.0; 3], 2.0); }
        assert!(nav.ahead.is_some());
        nav.path.clear();
        // Far from the end and from its first node: the bot searches afresh instead.
        let away = map.grid.points().map(|p| p.1).max_by(|a, b| flat(*a, end).total_cmp(&flat(*b, end))).unwrap();
        assert!(!nav.adopt_ahead(&map, &sim, away, 3.0));
        assert!(nav.ahead.is_none());
    }

    #[test]
    fn extra_slots_admit_searches_together() {
        let sim = build_sim(&demo_setup()).unwrap();
        let map = Navigation::for_map(&sim, "demo", None);
        map.search_slots(2);
        let (mut a, mut b, mut c) = (map.searches.ticket(), map.searches.ticket(), map.searches.ticket());
        assert!(a.admit());
        assert!(b.admit());
        assert!(!c.admit());
        drop(a);
        assert!(c.admit());
    }

    #[test]
    fn cancelled_requests_release_the_single_workspace_in_fifo_order() {
        let sim = build_sim(&demo_setup()).unwrap();
        let map = Navigation::for_map(&sim, "demo", None);
        let nav = Navigator::new(&map);
        let mut jobs: Vec<_> = (0..16)
            .map(|_| Job::new(&map, &nav, [0.0; 3], 1.0, 42, None))
            .collect();
        assert!(matches!(
            jobs[0].step(&map, &nav, &sim, [0.0; 3], 1.0, &mut 1),
            Progress::Pending
        ));
        for job in &mut jobs[1..] {
            assert!(!job.ticket.admit());
        }
        jobs.remove(0);
        assert!(!jobs[1].ticket.admit());
        assert!(jobs[0].ticket.admit());
        jobs.remove(0);
        assert!(jobs[0].ticket.admit());
        drop(jobs);
        let mut ticket = map.searches.ticket();
        assert!(ticket.admit());
    }

    #[test]
    fn incomplete_queries_do_not_cool_down_destinations_and_combat_cancels() {
        let sim = build_sim(&demo_setup()).unwrap();
        let map = Navigation::for_map(&sim, "demo", None);
        let mut nav = Navigator::new(&map);
        nav.request(&map, [0.0; 3], 1.0, 42, None);
        let mut job = nav.pending.take().unwrap();
        assert!(matches!(
            job.step(&map, &nav, &sim, [0.0; 3], 1.0, &mut 1),
            Progress::Pending
        ));
        assert!(nav.cooldown.iter().all(|&t| t == 0.0));
        nav.pending = Some(job);
        nav.interrupt();
        assert!(nav.pending.is_none());
        let mut ticket = map.searches.ticket();
        assert!(ticket.admit());
    }

    #[test]
    fn publication_splices_from_current_feet_on_the_raw_chain() {
        let sim = build_sim(&demo_setup()).unwrap();
        let map = Navigation::for_map(&sim, "demo", None);
        let nav = Navigator::new(&map);
        let mut job = Job::new(&map, &nav, [0.0; 3], 1.0, 42, Some([300.0, 0.0, 300.0]));
        for _ in 0..100000 {
            // Exercise cursor boundaries within one simulated tick, separately from timeout.
            job.active_ticks = 0;
            let result = job.step(&map, &nav, &sim, [0.0; 3], 1.0, &mut 1);
            assert!(matches!(result, Progress::Pending));
            if matches!(job.phase, Phase::Publish(..)) {
                break;
            }
        }
        assert!(matches!(job.phase, Phase::Publish(..)));
        let feet = [60.0, 0.0, 60.0];
        loop {
            match job.step(&map, &nav, &sim, feet, 1.0, &mut 1024) {
                Progress::Pending => {}
                Progress::Complete(Found { path, .. }) => {
                    assert!(flat(feet, path[0].0) <= 80.0);
                    assert!(!job.retry);
                    break;
                }
                Progress::Failed(reason) => panic!("{reason}"),
            }
        }
    }

    #[test]
    fn queue_time_is_excluded_and_displacement_has_only_one_restart() {
        let sim = build_sim(&demo_setup()).unwrap();
        let map = Navigation::for_map(&sim, "demo", None);
        let nav = Navigator::new(&map);
        let mut first = map.searches.ticket();
        assert!(first.admit());
        let mut job = Job::new(&map, &nav, [0.0; 3], 1.0, 42, None);
        for _ in 0..1000 {
            assert!(matches!(
                job.step(&map, &nav, &sim, [0.0; 3], 1.0, &mut 1),
                Progress::Pending
            ));
        }
        assert_eq!(job.active_ticks, 0);
        drop(first);
        assert!(matches!(
            job.step(&map, &nav, &sim, [0.0; 3], 1.0, &mut 1),
            Progress::Pending
        ));
        assert_eq!(job.active_ticks, 1);
        assert!(job.restart([100.0, 0.0, 100.0]));
        assert!(!job.restart([200.0, 0.0, 200.0]));
    }

    #[test]
    fn retained_memory_counts_every_slot_and_survives_their_return() {
        if !std::path::Path::new("data/map/rm06/rm06.map").exists() { return; }
        let path = "data/map/rm06/rm06.map";
        let sim = build_sim(&crate::net::proto::Setup { map: path.into(), ..demo_setup() }).unwrap();
        let map = Navigation::for_map(&sim, path, None);
        map.search_slots(3);
        let feet = map.grid.points().next().unwrap().1;
        let mut one = 0;
        for cycle in 0..2 {
            let mut navigators: Vec<_> = (0..3).map(|_| Navigator::new(&map)).collect();
            for (i, nav) in navigators.iter_mut().enumerate() {
                nav.request(&map, feet, 1.0, i, None);
                nav.advance_search(&map, &sim, feet, 1.0);
            }
            // Every slot is busy and has grown its arrays.
            assert_eq!(map.searches.0.lock().unwrap().free.len(), 0);
            if cycle == 0 {
                one = navigators[0].pending.as_ref().unwrap().ticket.workspace.as_ref().unwrap().1.bytes();
                assert!(one > 0);
            }
            // Cancel one and finish the others. Returned workspaces stay counted.
            navigators[0].interrupt();
            for nav in &mut navigators[1..] {
                while nav.pending.is_some() { nav.advance_search(&map, &sim, feet, 1.0); }
            }
            assert_eq!(map.searches.0.lock().unwrap().free.len(), 3);
            assert!(map.searches.retained_bytes() >= 3 * one, "{} < 3 * {one}", map.searches.retained_bytes());
        }
        assert!(map.estimated_bytes() >= map.grid.estimated_bytes() + 3 * one);
    }

    #[test]
    #[ignore = "manual sixteen-bot continuation timing and workspace survey"]
    fn profile_continuation_pool() {
        for path in [
            "data/map/rm08/rm08.map",
            "data/map/m00/citystreet_large.map",
        ] {
            let sim = build_sim(&crate::net::proto::Setup {
                map: path.into(),
                ..demo_setup()
            })
            .unwrap();
            let feet = Navigation::for_map(&sim, path, None).grid.points().next().unwrap().1;
            for slots in [1, 4] {
                let map = Navigation::for_map(&sim, path, None);
                map.search_slots(slots);
                let mut timings = Vec::new();
                let mut tick_max = 0.0f64;
                for cycle in 0..3 {
                    let mut navigators: Vec<_> = (0..16).map(|_| Navigator::new(&map)).collect();
                    for (i, nav) in navigators.iter_mut().enumerate() {
                        nav.request(&map, feet, 1.0, i, None);
                    }
                    let mut ticks = 0;
                    while navigators.iter().any(|n| n.pending.is_some()) {
                        ticks += 1;
                        assert!(ticks < 10000);
                        // All the bots' search work in one tick of a shared lab game.
                        let tick = Instant::now();
                        for nav in &mut navigators {
                            if nav.pending.is_none() {
                                continue;
                            }
                            let clock = Instant::now();
                            nav.advance_search(&map, &sim, feet, 1.0);
                            timings.push(clock.elapsed().as_secs_f64() * 1000.0);
                        }
                        tick_max = tick_max.max(tick.elapsed().as_secs_f64() * 1000.0);
                    }
                    println!(
                        "{path} slots={slots} cycle={cycle} queue_ticks={ticks} retained_scratch_bytes={} completed={} failures={:?}",
                        map.searches.retained_bytes(),
                        navigators.iter().filter(|n| !n.path.is_empty()).count(),
                        navigators
                            .iter()
                            .filter(|n| n.path.is_empty())
                            .map(|n| n.reason)
                            .collect::<Vec<_>>()
                    );
                }
                timings.sort_by(f64::total_cmp);
                println!(
                    "{path} slots={slots} slices={} median_ms={:.3} p95_ms={:.3} max_ms={:.3} tick_max_ms={tick_max:.3}",
                    timings.len(),
                    timings[timings.len() / 2],
                    timings[timings.len() * 95 / 100],
                    timings.last().unwrap()
                );
            }
        }
    }
}
