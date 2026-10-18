//! What a close blast does to the player beyond the shake and the white-out: the head is
//! knocked, so the aim jumps a little and drops, and the eyes squint shut against the flash
//! to a dark slit, then open again. The kick is real (it moves where the player points);
//! the eyes are only drawn.

/// Seconds the eyes stay shut tight before they begin to open
const HOLD: f32 = 0.05;
/// Seconds from the hit until the eyes are open again
const RECOVER: f32 = 0.1;
/// Smallest and largest turn of the kick at full strength, in degrees (the side is chosen)
const TURN: (f32, f32) = (15.0, 30.0); // (3.0, 8.0);
/// Smallest and largest drop of the aim, in degrees
const DROP: (f32, f32) = (10.0, 30.0); // (2.0, 6.0);

/// How much taller than wide the view is squeezed to make the shape of an eye
const EYE_SQUASH: f32 = 1.7;
/// How pointed the ends of the eye are: 2 is an oval, 1 a diamond
const EYE_POINT: f32 = 1.7;

#[derive(Clone, Copy, Debug, Default)]
pub struct Concussion {
    /// Seconds since the last hit
    age: f32,
    /// How hard it was (0..1)
    peak: f32,
}

/// A number in 0..1 from any seed, scrambled so nearby seeds do not look alike
fn unit(seed: u64, salt: u64) -> f32 {
    let mut z = seed.wrapping_add(salt.wrapping_mul(0x9E3779B97F4A7C15)).wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

impl Concussion {
    /// A blast shook the player by `shake` (0..1). A weak one does nothing. `seed` picks which
    /// way the head is knocked. Returns the kick to give the aim, in degrees: how far to turn
    /// (positive is left) and how far to drop it.
    pub fn hit(&mut self, shake: f32, seed: u64) -> Option<(f32, f32)> {
        if shake < 0.05 {
            return None;
        }
        // Anything this side of the room shuts the eyes all the way, and far off ones still count
        let peak = (shake * 2.5).min(1.0).sqrt();
        let side = if unit(seed, 1) < 0.5 { -1.0 } else { 1.0 };
        let turn = side * (TURN.0 + (TURN.1 - TURN.0) * unit(seed, 2)) * peak;
        let drop = (DROP.0 + (DROP.1 - DROP.0) * unit(seed, 3)) * peak;
        // A new hit over an old one never makes the sight clearer than what is left of the old
        if peak >= self.daze() {
            *self = Concussion { age: 0.0, peak };
        }
        Some((turn, drop))
    }

    pub fn update(&mut self, dt: f32) {
        self.age += dt;
    }

    /// How shut the eyes are, 0..1: fully at once, held a moment, then opening the way a
    /// person's do: slowly to begin with, quickly once they are going, and easing at the end
    pub fn daze(&self) -> f32 {
        if self.peak <= 0.0 || self.age >= RECOVER {
            return 0.0;
        }
        let t = ((self.age - HOLD).max(0.0) / (RECOVER - HOLD)).clamp(0.0, 1.0);
        self.peak * (1.0 - t * t * (3.0 - 2.0 * t))
    }
}

/// How far from the middle of the view the edge of the eye is, at `angle` around it, for a
/// shape that reaches 1 straight out to the sides. The ends are pointed and it is shallower
/// than it is wide.
pub fn eye(angle: f32) -> f32 {
    let (c, s) = (angle.cos().abs(), angle.sin().abs() * EYE_SQUASH);
    (c.powf(EYE_POINT) + s.powf(EYE_POINT)).powf(-1.0 / EYE_POINT)
}

/// How wide the clear middle of the view is at `daze` (0..1), as a share of the whole
/// shape: all of it when not dazed, a thin slit at the worst
pub fn clear_radius(daze: f32) -> f32 {
    1.0 - 0.88 * daze.clamp(0.0, 1.0)
}

/// How dark the edge of the view is at `daze`. Nearly black, not quite.
pub fn edge_darkness(daze: f32) -> f32 {
    0.98 * (daze * 1.3).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hard_hit_closes_the_view_at_once_and_it_opens_again() {
        let mut c = Concussion::default();
        assert_eq!(c.daze(), 0.0);
        c.hit(1.0, 7);
        assert_eq!(c.daze(), 1.0);
        let mut last = 1.0;
        let mut t = 0.0;
        while t < RECOVER {
            c.update(1.0 / 60.0);
            t += 1.0 / 60.0;
            assert!(c.daze() <= last + 1e-6);
            last = c.daze();
        }
        assert_eq!(c.daze(), 0.0);
    }

    #[test]
    fn the_view_stays_dark_for_a_moment_before_it_opens() {
        let mut c = Concussion::default();
        c.hit(1.0, 7);
        c.update(HOLD * 0.9);
        assert_eq!(c.daze(), 1.0);
    }

    #[test]
    fn the_eyes_open_slowly_then_fast_then_slowly() {
        let mut c = Concussion::default();
        c.hit(1.0, 7);
        c.update(HOLD);
        let mut at = Vec::new();
        for _ in 0..9 {
            c.update((RECOVER - HOLD) / 9.0);
            at.push(c.daze());
        }
        let steps: Vec<f32> = std::iter::once(1.0).chain(at.iter().copied()).collect::<Vec<_>>().windows(2).map(|w| w[0] - w[1]).collect();
        // Each step opens them further than the last until the middle, then less
        assert!(steps[0] < steps[3] && steps[3] > steps[8], "{steps:?}");
    }

    #[test]
    fn a_weak_blast_does_nothing_and_a_far_one_a_little() {
        let mut c = Concussion::default();
        assert_eq!(c.hit(0.02, 1), None);
        assert_eq!(c.daze(), 0.0);
        let kick = c.hit(0.1, 1).unwrap();
        assert!(c.daze() > 0.3 && c.daze() < 0.8, "{}", c.daze());
        assert!(kick.0.abs() <= TURN.1 * 0.8 && kick.1 <= DROP.1 * 0.8, "{kick:?}");
        assert_eq!(Concussion::default().hit(0.4, 1).is_some() as u8, 1);
        let mut near = Concussion::default();
        near.hit(0.4, 1);
        assert_eq!(near.daze(), 1.0);
    }

    #[test]
    fn the_kick_is_a_turn_either_way_and_a_drop() {
        let (mut left, mut right) = (false, false);
        for seed in 0..40 {
            let (turn, drop) = Concussion::default().hit(1.0, seed).unwrap();
            assert!(turn.abs() >= TURN.0 - 0.01 && turn.abs() <= TURN.1 + 0.01, "{turn}");
            assert!(drop >= DROP.0 - 0.01 && drop <= DROP.1 + 0.01, "{drop}");
            left |= turn > 0.0;
            right |= turn < 0.0;
        }
        assert!(left && right);
    }

    #[test]
    fn a_gentle_hit_does_not_cut_short_a_hard_one() {
        let mut c = Concussion::default();
        c.hit(1.0, 3);
        c.update(HOLD + (RECOVER - HOLD) * 0.2);
        let before = c.daze();
        assert!(before > 0.5);
        c.hit(0.1, 4);
        assert_eq!(c.daze(), before);
    }

    #[test]
    fn the_view_is_never_fully_black_and_the_eye_is_wider_than_it_is_tall() {
        assert!(edge_darkness(1.0) < 1.0);
        assert!(clear_radius(1.0) > 0.05);
        assert_eq!(clear_radius(0.0), 1.0);
        assert!((eye(0.0) - 1.0).abs() < 1e-5);
        assert!(eye(std::f32::consts::FRAC_PI_2) < eye(0.0) / 1.5);
        // The same in every quarter
        assert!((eye(0.7) - eye(std::f32::consts::PI - 0.7)).abs() < 1e-5);
    }
}
