//! Peeking round a corner, as the original does it. A peek plays one of the soldier's peek
//! motions up to the frame it holds at for as long as the key is down (RogueSpear.exe
//! 0x5cb6a0), and the eye goes where the head goes, to the side and up or down
//! (0x5a9ba0, 0x5d67c0). Letting go plays the rest of the motion, which leans back; let go
//! before the hold and it leans back from as far as it got. Peeking stands the player still.
//! The lean stops short where a wall is in the way: the original moves the body's cylinder to
//! the side as far as the head goes, and pauses the motion where that meets a wall (0x5d6670).

/// The motions play at this many frames a second
const FPS: f32 = 30.0;

/// One of the peek motions: how far the head has moved right and up at each of its stored
/// keys, which come every `KEY_FRAMES` frames (from examples/peek-head.rs --keys)
struct Motion {
    frames: f32,
    hold: f32,
    keys: &'static [[f32; 2]],
}

const KEY_FRAMES: f32 = 4.0;

/// tau_plf, held at frame 21
const STANDING_LEFT: Motion = Motion {
    frames: 41.0,
    hold: 21.0,
    keys: &[[0.0, 0.0], [-1.4, 0.7], [-13.9, 2.2], [-35.9, -4.4], [-51.2, -18.2], [-57.0, -27.9], [-53.5, -22.5], [-39.1, -7.7], [-14.8, 0.8], [0.4, -0.4], [3.4, -1.0]],
};
/// tau_prt, held at frame 25
const STANDING_RIGHT: Motion = Motion {
    frames: 47.0,
    hold: 25.0,
    keys: &[[0.0, 0.0], [1.7, -0.2], [12.6, -0.8], [33.8, -9.0], [44.8, -17.3], [48.8, -21.1], [50.0, -22.2], [45.8, -18.2], [30.7, -6.1], [12.3, -0.1], [4.8, 0.1], [3.0, -0.1]],
};
/// tmac_plf, held at frame 27. The original holds it there or, for some characters (its
/// method at +0x50, unread), plays tac_plf, the same motion, and holds it at 19.
const CROUCHED_LEFT: Motion = Motion {
    frames: 52.0,
    hold: 27.0,
    keys: &[[0.0, 0.0], [-2.6, 0.8], [-12.1, 4.9], [-26.7, 8.0], [-38.3, 7.1], [-45.4, 4.5], [-48.9, 2.5], [-48.4, 3.3], [-44.4, 8.2], [-29.8, 16.3], [-10.5, 13.5], [1.2, 5.4], [4.5, 2.1]],
};
/// tmac_prt, held at frame 27 (or tac_prt at 17)
const CROUCHED_RIGHT: Motion = Motion {
    frames: 49.0,
    hold: 27.0,
    keys: &[[0.0, 0.0], [6.3, 5.2], [24.9, 13.3], [43.9, 12.3], [55.5, 8.2], [61.8, 4.7], [64.3, 3.8], [62.3, 5.4], [50.0, 11.5], [26.5, 12.0], [8.3, 3.5], [-0.0, -0.8], [-2.1, 0.4]],
};

impl Motion {
    fn of(right: bool, crouched: bool) -> &'static Motion {
        match (right, crouched) {
            (false, false) => &STANDING_LEFT,
            (true, false) => &STANDING_RIGHT,
            (false, true) => &CROUCHED_LEFT,
            (true, true) => &CROUCHED_RIGHT,
        }
    }

    /// The frame of the last key: let go on the way out, a peek leans back from this far before
    /// it, which brings the head back the way it went
    fn last_key(&self) -> f32 {
        (self.keys.len() - 1) as f32 * KEY_FRAMES
    }

    /// How far the head has moved right and up at `frame`, between the keys either side
    fn moved(&self, frame: f32) -> [f32; 2] {
        let at = (frame / KEY_FRAMES).max(0.0);
        let k = (at as usize).min(self.keys.len() - 1);
        let (a, b) = (self.keys[k], self.keys[(k + 1).min(self.keys.len() - 1)]);
        let t = (at - k as f32).min(1.0);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }
}

/// A peek under way
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Peek {
    pub right: bool,
    /// Which motion it plays, chosen as it starts
    pub crouched: bool,
    /// How far through the motion it is
    pub frame: f32,
    /// The key was let go, and it is leaning back
    pub returning: bool,
}

impl Peek {
    /// The peek one tick on with the peek keys held so: one starts when only one of them is
    /// down, keeps its side until it has played out, and ends when it has. `clearance` says how
    /// far the body would be from the walls moved this far to the right (negative inside one).
    pub fn step(peek: Option<Peek>, left: bool, right: bool, crouched: bool, dt: f32, clearance: impl Fn(f32) -> f32) -> Option<Peek> {
        let Some(mut p) = peek else {
            return (left != right).then_some(Peek { right, crouched, frame: 0.0, returning: false });
        };
        let m = Motion::of(p.right, p.crouched);
        let held = if p.right { right } else { left };
        if held {
            // Out to the hold and still there, unless a wall is in the way; pressed again while
            // leaning back, it stops where it is
            if !p.returning && p.frame < m.hold {
                let next = (p.frame + FPS * dt).min(m.hold);
                let (now, then) = (clearance(m.moved(p.frame)[0]), clearance(m.moved(next)[0]));
                if then >= 0.0 || then >= now {
                    p.frame = next;
                }
            }
            p.returning = false;
            return Some(p);
        }
        if !p.returning {
            p.returning = true;
            // Let go on the way out, it leans back from the same place on the way back
            if p.frame < m.hold {
                p.frame = m.last_key() - p.frame;
            }
        }
        p.frame += FPS * dt;
        (p.frame < m.frames - 1.0).then_some(p)
    }

    /// How far the eye is moved, to the right and up
    pub fn eye(&self) -> [f32; 2] {
        Motion::of(self.right, self.crouched).moved(self.frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn hold(mut p: Option<Peek>, left: bool, right: bool, seconds: f32) -> Option<Peek> {
        for _ in 0..(seconds / DT).round() as usize {
            p = Peek::step(p, left, right, false, DT, |_| f32::INFINITY);
        }
        p
    }

    #[test]
    fn a_peek_leans_out_holds_and_leans_back_when_let_go() {
        let out = hold(None, true, false, 2.0).expect("peeking");
        assert_eq!(out.frame, STANDING_LEFT.hold);
        let [side, up] = out.eye();
        assert!(side < -50.0 && up < -20.0, "{side} {up}");
        // Back in the rest of the motion's 19 frames
        let back = hold(Some(out), false, false, 18.0 / FPS).expect("still leaning back");
        assert!(back.eye()[0].abs() < 20.0, "{:?}", back.eye());
        assert_eq!(hold(Some(back), false, false, 2.0 / FPS), None);
    }

    #[test]
    fn let_go_on_the_way_out_a_peek_leans_back_from_where_it_got() {
        let half = hold(None, false, true, 0.4).expect("peeking");
        let back = Peek::step(Some(half), false, false, false, 0.0, |_| f32::INFINITY).expect("leaning back");
        assert!(back.returning);
        assert!((back.eye()[0] - half.eye()[0]).abs() < 4.0, "{:?} from {:?}", back.eye(), half.eye());
    }

    #[test]
    fn a_wall_stops_the_lean_short_and_one_behind_lets_it_go() {
        // A wall 10 units to the left of the body
        let wall = |side: f32| 10.0 + side;
        let mut p = None;
        for _ in 0..120 {
            p = Peek::step(p, true, false, false, DT, wall);
        }
        let side = p.expect("peeking").eye()[0];
        assert!((-11.0..=-9.0).contains(&side), "{side}");
        // Already against it, leaning away is no trouble
        let mut p = None;
        for _ in 0..120 {
            p = Peek::step(p, false, true, false, DT, |side: f32| -5.0 + side);
        }
        assert_eq!(p.expect("peeking").frame, STANDING_RIGHT.hold);
    }

    #[test]
    fn a_peek_keeps_its_side_until_it_has_played_out() {
        let left = hold(None, true, false, 0.2).expect("peeking");
        let switched = hold(Some(left), false, true, 0.1).expect("leaning back");
        assert!(!switched.right && switched.returning);
        assert_eq!(hold(None, true, true, 0.5), None, "both keys is neither");
    }
}
