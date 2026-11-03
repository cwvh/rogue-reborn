//! A dead player watching a living one in a round (see docs/multiplayer.md, "Watching"): their
//! own side's, through their eyes or from behind, and no free camera, so the dead cannot scout.
//! Only what the client already has is used: its copy of the game and the view it draws.

use crate::net::client::{Frame, Seen};
use crate::net::round::{Phase, RoundState};
use crate::sim::{Input, Sim};

type V3 = [f32; 3];

/// How long (seconds) after dying before watching starts: the fall and the turn to whoever did it
pub const DEATH_VIEW: f32 = 2.0;

/// How long (seconds) a watched player is still watched once seen dead, so their death is seen
pub const TARGET_GONE: f32 = 1.5;

/// How far back and how far up the camera behind a player goes
pub const BEHIND: f32 = 220.0;
pub const RISE: f32 = 40.0;

/// How far short of what it would hit the camera behind stops: well beyond the near plane, which
/// is at most 4 units
const CLEARANCE: f32 = 12.0;

/// How far beside the line from the eyes the camera's other four lines run, for a little volume
const SPREAD: f32 = 8.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WatchMode {
    #[default]
    Eyes,
    Behind,
}

/// What the choice of whom to watch depends on, each frame
pub struct WatchContext<'a> {
    /// The client's copy of the game: sides, joins, who is alive as of the newest snapshot
    pub sim: &'a Sim,
    /// What the screen shows: where the others are, how they stand, who is shown dead
    pub frame: &'a Frame,
    pub me: usize,
    pub round: RoundState,
    /// The setup has bases for both sides
    pub team_game: bool,
    pub connected: bool,
}

impl WatchContext<'_> {
    fn seen(&self, slot: usize) -> Option<&Seen> {
        self.frame.players.iter().find(|s| s.slot == slot)
    }

    /// Whether `slot` may be watched now: someone else, alive on screen and in the newest
    /// snapshot, and on the watcher's side in a team game (with no sides, anyone)
    fn eligible(&self, slot: usize) -> bool {
        let (Some(p), Some(s)) = (self.sim.players.get(slot), self.seen(slot)) else { return false };
        slot != self.me && p.active && p.respawn_at.is_none() && s.dead.is_none() && s.joined == p.joined && self.on_side(slot)
    }

    /// On the watcher's side in a team game; with no sides, anyone
    fn on_side(&self, slot: usize) -> bool {
        if !self.team_game {
            return true;
        }
        let mine = self.sim.players.get(self.me).and_then(|m| m.side);
        mine.is_some() && self.sim.players.get(slot).and_then(|p| p.side) == mine
    }

    fn candidates(&self) -> Vec<usize> {
        let mut slots: Vec<usize> = self.frame.players.iter().map(|s| s.slot).filter(|&s| self.eligible(s)).collect();
        slots.sort();
        slots
    }
}

/// Where a camera is and which way it looks: degrees, as an `Input`'s look
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub eye: V3,
    pub yaw: f32,
    pub pitch: f32,
}

#[derive(Clone, Copy, Debug)]
struct Target {
    slot: usize,
    joined: u32,
    /// When (view time) they were first shown dead
    gone_at: Option<f32>,
}

/// Whom a dead player watches, and how
#[derive(Default)]
pub struct Watch {
    /// The round's number and the watcher's slot it began in
    key: Option<(u16, usize)>,
    /// When (view time) watching began, once the death view was over
    started: Option<f32>,
    target: Option<Target>,
    pub mode: WatchMode,
    /// The latest view through a living target's eyes, held once they are gone, with whose it
    /// was and where they stood
    last: Option<(Pose, usize, V3)>,
}

impl Watch {
    fn reset(&mut self) {
        *self = Watch { mode: self.mode, ..Watch::default() };
    }

    /// Once a frame
    pub fn update(&mut self, cx: &WatchContext) {
        let dead = cx.sim.players.get(cx.me).filter(|p| p.active && p.respawn_at.is_some());
        let Some(me) = dead.filter(|_| cx.connected && cx.round.phase != Phase::Open) else {
            self.reset();
            return;
        };
        let key = (cx.round.number, cx.me);
        if self.key != Some(key) {
            self.reset();
            self.key = Some(key);
        }
        let now = cx.frame.time;
        if self.started.is_none() {
            // A late joiner sitting out has no fall to watch first
            if me.hit.is_some_and(|h| now < h.start + DEATH_VIEW) {
                return;
            }
            self.started = Some(now);
        }
        if let Some(t) = self.target.as_mut() {
            // Still the same person, on the same side: a slot can change hands between two
            // snapshots
            let same = cx.seen(t.slot).is_some_and(|s| s.joined == t.joined)
                && cx.sim.players.get(t.slot).is_some_and(|p| p.active && p.joined == t.joined)
                && cx.on_side(t.slot);
            if !same {
                self.target = None;
            } else if cx.seen(t.slot).is_some_and(|s| s.dead.is_some()) {
                // Shown dead: followed a moment, then let go. The newest snapshot can know
                // sooner, but until the screen shows it they are followed as they are seen
                let gone = *t.gone_at.get_or_insert(now);
                if now - gone >= TARGET_GONE {
                    self.target = None;
                }
            } else {
                t.gone_at = None;
            }
        }
        if self.target.is_none() {
            self.target = cx.candidates().first().map(|&slot| self.aim_at(cx, slot));
        }
    }

    fn aim_at(&self, cx: &WatchContext, slot: usize) -> Target {
        Target { slot, joined: cx.sim.players[slot].joined, gone_at: None }
    }

    /// The next one to watch, in slot order. Only once watching has started.
    pub fn next(&mut self, cx: &WatchContext) {
        self.step(cx, true);
    }

    pub fn previous(&mut self, cx: &WatchContext) {
        self.step(cx, false);
    }

    fn step(&mut self, cx: &WatchContext, forward: bool) {
        if self.started.is_none() {
            return;
        }
        let slots = cx.candidates();
        let here = self.target.map(|t| t.slot);
        let pick = match (forward, here) {
            (true, Some(h)) => slots.iter().find(|&&s| s > h).or(slots.first()),
            (false, Some(h)) => slots.iter().rev().find(|&&s| s < h).or(slots.last()),
            (_, None) => slots.first(),
        };
        if let Some(&slot) = pick {
            self.target = Some(self.aim_at(cx, slot));
        }
    }

    pub fn toggle_mode(&mut self) {
        self.mode = if self.mode == WatchMode::Eyes { WatchMode::Behind } else { WatchMode::Eyes };
    }

    /// Whom the camera is on and where they stand, as of its latest view, held with it: the
    /// view is drawn from the rooms around them
    pub fn standing(&self) -> Option<(usize, V3)> {
        self.last.map(|(_, slot, feet)| (slot, feet))
    }

    /// Whether the player is dead in a round, so that F is the watch's even before it starts
    pub fn in_round(&self) -> bool {
        self.key.is_some()
    }

    /// Whether watching has started: until then the dead player's own death view shows
    pub fn watching(&self) -> bool {
        self.started.is_some()
    }

    /// Whom it watches, while they are alive
    pub fn target(&self) -> Option<usize> {
        self.target.filter(|t| t.gone_at.is_none()).map(|t| t.slot)
    }

    /// The camera: at the watched player's eyes, or behind them, as the view shows them. Once
    /// they are gone, the last view of them, held. None before watching starts, or with no one
    /// ever to watch, which leaves the dead player's own death view.
    pub fn camera(&mut self, cx: &WatchContext) -> Option<Pose> {
        self.started?;
        if let Some(s) = self.target().and_then(|slot| cx.seen(slot)) {
            let eye = cx.sim.eye_at(s.feet, s.crouched, s.peek, s.look.0);
            let (yaw, pitch) = Input { yaw: s.look.0, pitch: s.look.1, ..Input::default() }.look();
            self.last = Some((Pose { eye, yaw, pitch }, s.slot, s.feet));
        }
        let (eyes, ..) = self.last?;
        Some(match self.mode {
            WatchMode::Eyes => eyes,
            WatchMode::Behind => behind(cx.sim, eyes, BEHIND, RISE, 0.0),
        })
    }
}

/// A camera `distance` back along `from`'s look, turned `orbit` degrees round the eye, and `rise`
/// up, stopped short of whatever is in the way. Five lines are traced, the middle one and four
/// `SPREAD` beside it, and the camera stops `CLEARANCE` short of the nearest hit along them: an
/// approximation of a camera with some size, so the near plane does not reach a wall.
pub fn behind(sim: &Sim, from: Pose, distance: f32, rise: f32, orbit: f32) -> Pose {
    let yaw = from.yaw + orbit;
    let (y, p) = (yaw.to_radians(), from.pitch.to_radians());
    // Along the look, in map space
    let ahead = [-y.sin() * p.cos(), p.sin(), y.cos() * p.cos()];
    let e = from.eye;
    let want = [e[0] - ahead[0] * distance, e[1] - ahead[1] * distance + rise, e[2] - ahead[2] * distance];
    let line = [want[0] - e[0], want[1] - e[1], want[2] - e[2]];
    let length = (line[0] * line[0] + line[1] * line[1] + line[2] * line[2]).sqrt();
    if length < 1e-3 {
        return Pose { yaw, ..from };
    }
    let right = [y.cos(), 0.0, y.sin()];
    let offsets = [[0.0; 3], right, right.map(|v| -v), [0.0, 1.0, 0.0], [0.0, -1.0, 0.0]];
    // Traced past where it is wanted by the clearance, so a wall just beyond is kept off too
    let beyond = (length + CLEARANCE) / length;
    let reach = offsets.iter().map(|o| {
        let a = [0, 1, 2].map(|k| e[k] + o[k] * SPREAD);
        let b = [0, 1, 2].map(|k| a[k] + line[k] * beyond);
        sim.line_reach(a, b)
    }).fold(f32::INFINITY, f32::min);
    let go = (reach - CLEARANCE).clamp(0.0, length) / length;
    Pose { eye: [0, 1, 2].map(|k| e[k] + line[k] * go), yaw, pitch: from.pitch }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::lab::{demo_setup, team_setup};
    use crate::net::proto::Setup;
    use crate::net::setup::build_sim;
    use crate::sim::{Side, Struck};

    /// Me (slot 0) dead on Blue, having fallen at time 0; Blue in slots 1 and 3, Gold in 2. With
    /// no sides, the same without them.
    fn game(team: bool) -> Sim {
        let setup = if team { team_setup() } else { Setup { pvp: true, ..demo_setup() } };
        let mut sim = build_sim(&setup).unwrap();
        for (i, side) in [Side::Blue, Side::Blue, Side::Gold, Side::Blue].into_iter().enumerate() {
            sim.add_player([-300.0 + i as f32 * 150.0, 0.0, 0.0]);
            sim.players[i].side = team.then_some(side);
        }
        sim.players[0].respawn_at = Some(10.0);
        sim.players[0].hit = Some(Struck { motion: "thdu_1", start: 0.0 });
        sim
    }

    /// The screen at `time`, showing everyone as they are
    fn frame(sim: &Sim, time: f32) -> Frame {
        let players = sim.players.iter().enumerate().filter(|(_, p)| p.active).map(|(slot, p)| Seen {
            slot, feet: p.player.pos, look: p.look, dead: p.respawn_at, crouched: p.player.crouched, peek: p.peek, joined: p.joined,
        }).collect();
        Frame { time, players, ..Frame::default() }
    }

    fn live() -> RoundState {
        RoundState { phase: Phase::Live, number: 1, ..RoundState::default() }
    }

    /// Update the watch with what the screen shows, and give back what it was given
    fn at<'a>(watch: &mut Watch, sim: &'a Sim, view: &'a Frame, round: RoundState) -> WatchContext<'a> {
        let cx = WatchContext { sim, frame: view, me: 0, round, team_game: sim.bases.iter().all(|b| !b.is_empty()), connected: true };
        watch.update(&cx);
        cx
    }

    fn kill(sim: &mut Sim, slot: usize) {
        sim.players[slot].respawn_at = Some(99.0);
    }

    #[test]
    fn after_the_fall_it_watches_the_first_living_teammate_and_never_the_other_side() {
        let sim = game(true);
        let mut w = Watch::default();
        let view = frame(&sim, 1.0);
        let cx = at(&mut w, &sim, &view, live());
        // The fall first: no watching, and the keys do nothing yet (nor F to the player's own)
        w.next(&cx);
        assert_eq!((w.in_round(), w.watching(), w.target(), w.camera(&cx)), (true, false, None, None));
        let view = frame(&sim, DEATH_VIEW + 0.1);
        let cx = at(&mut w, &sim, &view, live());
        assert_eq!(w.target(), Some(1));
        // Round the living teammates, in slot order, never Gold's
        w.next(&cx);
        assert_eq!(w.target(), Some(3));
        w.next(&cx);
        assert_eq!(w.target(), Some(1));
        w.previous(&cx);
        assert_eq!(w.target(), Some(3));
    }

    #[test]
    fn a_teammate_who_dies_is_watched_a_moment_then_the_next() {
        let mut sim = game(true);
        let mut w = Watch::default();
        let view = frame(&sim, 3.0);
        let cx = at(&mut w, &sim, &view, live());
        let last = w.camera(&cx).unwrap();
        kill(&mut sim, 1);
        // Shown dead: the view holds where they were
        for t in [3.1, 3.1 + TARGET_GONE - 0.05] {
            let view = frame(&sim, t);
            let cx = at(&mut w, &sim, &view, live());
            assert_eq!((w.target(), w.camera(&cx)), (None, Some(last)));
        }
        let view = frame(&sim, 3.1 + TARGET_GONE);
        at(&mut w, &sim, &view, live());
        assert_eq!(w.target(), Some(3));
    }

    #[test]
    fn a_death_not_yet_on_screen_is_not_held_until_it_is() {
        let mut sim = game(true);
        let mut w = Watch::default();
        at(&mut w, &sim, &frame(&sim, 3.0), live());
        assert_eq!(w.target(), Some(1));
        // The newest snapshot has them dead, but the screen, behind it, still shows them moving
        kill(&mut sim, 1);
        let mut view = frame(&sim, 3.1);
        view.players[1].dead = None;
        view.players[1].feet[0] += 30.0;
        let cx = at(&mut w, &sim, &view, live());
        let moving = w.camera(&cx).unwrap();
        assert_eq!((w.target(), moving.eye[0]), (Some(1), view.players[1].feet[0]));
        // Shown dead at 4: the view holds from there for the whole while, then moves on
        for t in [4.0, 4.0 + TARGET_GONE - 0.05] {
            let view = frame(&sim, t);
            let cx = at(&mut w, &sim, &view, live());
            assert_eq!((w.target(), w.camera(&cx)), (None, Some(moving)), "at {t}");
        }
        at(&mut w, &sim, &frame(&sim, 4.0 + TARGET_GONE), live());
        assert_eq!(w.target(), Some(3));
    }

    #[test]
    fn with_no_one_left_the_last_view_is_held() {
        let mut sim = game(true);
        let mut w = Watch::default();
        let view = frame(&sim, 3.0);
        let cx = at(&mut w, &sim, &view, live());
        w.next(&cx);
        assert_eq!(w.target(), Some(3));
        let (held, feet) = (w.camera(&cx).unwrap(), sim.players[3].player.pos);
        kill(&mut sim, 1);
        kill(&mut sim, 3);
        // However the slot changes after, the view stays, and is drawn from where they stood
        sim.players[3].player.pos = [400.0, 0.0, 400.0];
        let view = frame(&sim, 9.0);
        let cx = at(&mut w, &sim, &view, live());
        assert_eq!((w.target(), w.camera(&cx), w.standing()), (None, Some(held), Some((3, feet))));
    }

    #[test]
    fn one_who_leaves_or_is_replaced_is_let_go_at_once() {
        let mut sim = game(true);
        let mut w = Watch::default();
        at(&mut w, &sim, &frame(&sim, 3.0), live());
        assert_eq!(w.target(), Some(1));
        // One who changes side is let go at once too, not held as a death is
        sim.players[1].side = Some(Side::Gold);
        at(&mut w, &sim, &frame(&sim, 3.05), live());
        assert_eq!(w.target(), Some(3));
        sim.players[1].side = Some(Side::Blue);
        let view = frame(&sim, 3.07);
        let cx = at(&mut w, &sim, &view, live());
        w.previous(&cx);
        assert_eq!(w.target(), Some(1));
        sim.remove_player(1);
        at(&mut w, &sim, &frame(&sim, 3.1), live());
        assert_eq!(w.target(), Some(3));
        // Between two snapshots slot 3 changes hands: someone new, on Gold
        sim.players[3].joined = 77;
        sim.players[3].side = Some(Side::Gold);
        at(&mut w, &sim, &frame(&sim, 3.2), live());
        assert_eq!(w.target(), None, "the new player in slot 3 is not followed");
    }

    #[test]
    fn without_a_side_in_a_team_game_there_is_no_one_to_watch() {
        let mut sim = game(true);
        sim.players[0].side = None;
        let mut w = Watch::default();
        let view = frame(&sim, 3.0);
        let cx = at(&mut w, &sim, &view, live());
        assert!(w.watching());
        assert_eq!((w.target(), w.camera(&cx)), (None, None));
    }

    #[test]
    fn without_sides_anyone_may_be_watched() {
        let sim = game(false);
        let mut w = Watch::default();
        let view = frame(&sim, 3.0);
        let cx = at(&mut w, &sim, &view, live());
        assert_eq!(w.target(), Some(1));
        w.next(&cx);
        assert_eq!(w.target(), Some(2));
    }

    #[test]
    fn only_the_dead_watch_and_only_in_rounds() {
        let mut sim = game(true);
        let mut w = Watch::default();
        at(&mut w, &sim, &frame(&sim, 3.0), RoundState::default());
        assert!(!w.watching() && !w.in_round(), "open play comes back instead");
        at(&mut w, &sim, &frame(&sim, 3.0), live());
        assert!(w.watching());
        sim.players[0].respawn_at = None;
        at(&mut w, &sim, &frame(&sim, 3.1), live());
        assert!(!w.watching() && !w.in_round());
    }

    #[test]
    fn a_new_round_starts_over_and_a_late_joiner_watches_at_once() {
        let mut sim = game(true);
        let mut w = Watch::default();
        let view = frame(&sim, 3.0);
        let cx = at(&mut w, &sim, &view, live());
        w.next(&cx);
        assert_eq!(w.target(), Some(3));
        // Sitting out the next round: no fall, so no wait, and the first teammate again
        sim.players[0].hit = None;
        let round = RoundState { number: 2, ..live() };
        at(&mut w, &sim, &frame(&sim, 3.1), round);
        assert_eq!(w.target(), Some(1));
    }

    #[test]
    fn the_view_through_the_eyes_is_theirs_crouched_and_leaning() {
        let mut sim = game(true);
        sim.players[1].player.crouched = true;
        sim.players[1].peek = Some(crate::sim::peek::Peek { right: true, crouched: true, frame: 6.0, returning: false });
        sim.players[1].look = (crate::sim::quantize(30.0), crate::sim::quantize(-10.0));
        let mut w = Watch::default();
        let view = frame(&sim, 3.0);
        let cx = at(&mut w, &sim, &view, live());
        let pose = w.camera(&cx).unwrap();
        assert_eq!(pose.eye, sim.eye(1));
        assert!((pose.yaw - 30.0).abs() < 0.01 && (pose.pitch + 10.0).abs() < 0.01);
    }

    /// How far a point is inside the box room's walls, floor and ceiling
    fn inside(p: V3) -> f32 {
        let (hx, hz) = (crate::demo::WIDTH / 2.0, crate::demo::DEPTH / 2.0);
        [hx - p[0].abs(), hz - p[2].abs(), p[1], crate::demo::HEIGHT - p[1]].into_iter().fold(f32::INFINITY, f32::min)
    }

    #[test]
    fn the_camera_behind_stops_short_of_walls_corners_and_the_ceiling() {
        let sim = build_sim(&demo_setup()).unwrap();
        let pose = |eye: V3, yaw: f32, pitch: f32| Pose { eye, yaw, pitch };
        // In the open, the whole way back
        let cam = behind(&sim, pose([0.0, 165.0, 0.0], 0.0, 0.0), BEHIND, RISE, 0.0);
        assert!((cam.eye[2] + BEHIND).abs() < 0.01 && (cam.eye[1] - 205.0).abs() < 0.01, "{:?}", cam.eye);
        for (name, from, distance, orbit) in [
            ("a wall right behind", pose([0.0, 165.0, -450.0], 0.0, 0.0), BEHIND, 0.0),
            ("a corner", pose([450.0, 165.0, -450.0], 45.0, 0.0), BEHIND, 0.0),
            ("the ceiling, looking down", pose([0.0, 165.0, 0.0], 0.0, -70.0), BEHIND, 0.0),
            ("turned round and further back", pose([0.0, 165.0, -400.0], 0.0, 0.0), 400.0, 180.0),
            ("the far corner, turned", pose([400.0, 165.0, 400.0], 0.0, 0.0), 300.0, 45.0),
        ] {
            let cam = behind(&sim, from, distance, RISE, orbit);
            assert!(inside(cam.eye) >= 4.0, "{name}: the camera at {:?} is {} inside", cam.eye, inside(cam.eye));
        }
    }

    #[test]
    fn the_camera_behind_keeps_out_of_the_walls_where_rooms_overlap() {
        // citystreet_large's killhouse, where rooms 1 and 3 overlap (see the bullet test in sim)
        let setup = Setup { map: "data/map/m00/citystreet_large.map".to_string(), ..demo_setup() };
        let Ok(sim) = build_sim(&setup) else { return };
        let feet = [33533.0, 32887.0, 34907.4];
        for yaw in [0.0f32, 90.0, 133.3, 180.0, 270.0, 313.3] {
            let from = Pose { eye: [feet[0], feet[1] + 165.0, feet[2]], yaw, pitch: 0.0 };
            let cam = behind(&sim, from, BEHIND, RISE, 0.0);
            let d = ((cam.eye[0] - from.eye[0]).powi(2) + (cam.eye[1] - from.eye[1]).powi(2) + (cam.eye[2] - from.eye[2]).powi(2)).sqrt();
            // Nothing between the eyes and the camera, nor just past it
            let past = [0, 1, 2].map(|k| from.eye[k] + (cam.eye[k] - from.eye[k]) * (d + 6.0) / d.max(1e-3));
            assert!(sim.line_reach(from.eye, past) >= d + 5.9, "at {yaw} degrees, the camera {d} back is at a wall");
        }
    }
}
