//! Thrown grenades: how they fly and bounce, and how far a blast reaches.
//!
//! Units are map units, about a centimeter each. The numbers are ours: the original
//! game's explosion classes are not in any data file.

use crate::collision::CollisionWorld;

type V3 = [f32; 3];

fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn mul(a: V3, s: f32) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn len(a: V3) -> f32 { dot(a, a).sqrt() }

pub const GRAVITY: f32 = 980.0;
/// Half the grenade's width: how close it gets to a surface before it bounces
pub const RADIUS: f32 = 5.0;
/// Seconds from the pin to the blast
pub const FUSE: f32 = 3.0;
/// Holding the throw this long makes it as hard as it goes
pub const CHARGE_SECONDS: f32 = 1.2;
const SLOWEST_THROW: f32 = 350.0;
const FASTEST_THROW: f32 = 1700.0;
/// Speed kept along the surface's normal, and across it, at each bounce
const BOUNCINESS: f32 = 0.38;
const GRIP: f32 = 0.72;

/// Everything within this of a blast, in the open, is killed
pub const LETHAL: f32 = 300.0;
/// Beyond this a blast does no harm
pub const REACH: f32 = 650.0;
/// Glass this near a blast breaks
pub const GLASS_REACH: f32 = 800.0;
/// The player feels a blast this far away: shaken and deafened
pub const SHAKE_REACH: f32 = 1500.0;

/// How hard a thrown grenade leaves the hand after holding the throw for `charge` (0..1)
pub fn throw_speed(charge: f32) -> f32 {
    SLOWEST_THROW + (FASTEST_THROW - SLOWEST_THROW) * charge.clamp(0.0, 1.0)
}

/// How much of a blast reaches `distance` away in the open: full inside `LETHAL`,
/// fading to nothing at `REACH`
pub fn blast_strength(distance: f32) -> f32 {
    ((REACH - distance) / (REACH - LETHAL)).clamp(0.0, 1.0)
}

/// How hard the player is shaken `distance` from a blast (0..1), more when nothing is in the way
pub fn shake(distance: f32, in_the_open: bool) -> f32 {
    let near = (1.0 - distance / SHAKE_REACH).clamp(0.0, 1.0);
    near * near * if in_the_open { 1.0 } else { 0.45 }
}

/// Whether nothing between `from` and `to` stops a blast: walls do, but not a chain fence or a
/// door (see `CollisionWorld::blast_hit`)
pub fn clear_between(world: &CollisionWorld, from: V3, to: V3) -> bool {
    let d = sub(to, from);
    let dist = len(d);
    dist < 1.0 || world.blast_hit(from, mul(d, 1.0 / dist), dist).is_none_or(|hit| hit.dist > dist - 6.0)
}

/// A surface the grenade struck
#[derive(Clone, Copy, Debug)]
pub struct Bounce {
    pub point: V3,
    pub normal: V3,
    pub material: Option<usize>,
    /// How fast it was going into the surface
    pub speed: f32,
    /// Its velocity just before it struck
    pub incoming: V3,
}

#[derive(Clone, Debug)]
pub struct Grenade {
    pub pos: V3,
    pub vel: V3,
    /// Seconds until it goes off
    pub fuse: f32,
    /// How far it has turned in the air, in degrees
    pub spin: f32,
    pub at_rest: bool,
}

impl Grenade {
    /// A grenade leaving the hand at `from` along unit `dir`, as hard as `charge` (0..1) says,
    /// carrying the thrower's own `motion`
    pub fn thrown(from: V3, dir: V3, charge: f32, motion: V3) -> Grenade {
        Grenade { pos: from, vel: add(mul(dir, throw_speed(charge)), motion), fuse: FUSE, spin: 0.0, at_rest: false }
    }

    pub fn goes_off(&self) -> bool {
        self.fuse <= 0.0
    }

    /// Fly for `dt` seconds, bouncing off whatever it meets. Returns each bounce.
    pub fn update(&mut self, dt: f32, world: &CollisionWorld) -> Vec<Bounce> {
        self.fuse -= dt;
        let mut bounces = Vec::new();
        if self.at_rest {
            return bounces;
        }
        self.spin += len(self.vel) * dt * 2.0;
        let mut left = dt;
        while left > 0.0 {
            // Small steps, so a fast throw cannot pass through a thin wall
            let h = left.min(1.0 / 120.0);
            left -= h;
            self.vel[1] -= GRAVITY * h;
            let step = mul(self.vel, h);
            let travel = len(step);
            if travel < 1e-4 {
                continue;
            }
            let dir = mul(step, 1.0 / travel);
            // Not stopped by the faces grenades pass, such as a railing
            let Some(hit) = world.grenade_hit(self.pos, dir, travel + RADIUS) else {
                self.pos = add(self.pos, step);
                continue;
            };
            self.pos = add(self.pos, mul(dir, (hit.dist - RADIUS).clamp(0.0, travel)));
            let incoming = self.vel;
            let into = -dot(self.vel, hit.normal);
            if into > 0.0 {
                let along = add(self.vel, mul(hit.normal, into));
                self.vel = add(mul(along, GRIP), mul(hit.normal, into * BOUNCINESS));
                if into > 60.0 {
                    bounces.push(Bounce { point: add(self.pos, mul(dir, RADIUS)), normal: hit.normal, material: hit.material, speed: into, incoming });
                }
            }
            // Lying on something level: it stays
            if hit.normal[1] > 0.7 && len(self.vel) < 45.0 {
                self.vel = [0.0; 3];
                self.at_rest = true;
                break;
            }
        }
        bounces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_harder_throw_goes_faster() {
        assert!(throw_speed(1.0) > throw_speed(0.5) && throw_speed(0.5) > throw_speed(0.0));
        assert_eq!(throw_speed(5.0), throw_speed(1.0));
    }

    #[test]
    fn a_blast_is_full_close_in_and_gone_at_its_reach() {
        assert_eq!(blast_strength(0.0), 1.0);
        assert_eq!(blast_strength(LETHAL), 1.0);
        let mid = blast_strength((LETHAL + REACH) / 2.0);
        assert!(mid > 0.4 && mid < 0.6, "{mid}");
        assert_eq!(blast_strength(REACH), 0.0);
        assert_eq!(blast_strength(REACH * 3.0), 0.0);
    }

    #[test]
    fn a_wall_softens_the_shake() {
        assert!(shake(200.0, true) > shake(200.0, false));
        assert!(shake(200.0, true) > shake(900.0, true));
        assert_eq!(shake(SHAKE_REACH, true), 0.0);
    }

    #[test]
    fn the_fuse_burns_down_even_at_rest() {
        let mut g = Grenade::thrown([0.0; 3], [0.0, 0.0, 1.0], 0.5, [0.0; 3]);
        g.at_rest = true;
        let world = CollisionWorld::flat_floor(-100000.0, 1000.0);
        g.update(FUSE - 0.1, &world);
        assert!(!g.goes_off());
        g.update(0.2, &world);
        assert!(g.goes_off());
    }

    // Dropped onto a floor it bounces a little, then lies still and stays above the floor
    #[test]
    fn a_dropped_grenade_comes_to_rest_on_the_floor() {
        let world = CollisionWorld::flat_floor(0.0, 1000.0);
        let mut g = Grenade::thrown([0.0, 120.0, 0.0], [0.0, 0.0, 0.0], 0.0, [0.0; 3]);
        g.vel = [0.0; 3];
        let mut bounces = 0;
        for _ in 0..600 {
            bounces += g.update(1.0 / 60.0, &world).len();
        }
        assert!(g.at_rest, "{:?}", g.pos);
        assert!((g.pos[1] - RADIUS).abs() < 1.5, "{:?}", g.pos);
        assert!(bounces >= 1 && bounces < 6, "{bounces}");
    }

    #[test]
    fn a_thrown_grenade_falls() {
        let mut g = Grenade::thrown([0.0, 100.0, 0.0], [0.0, 0.0, 1.0], 0.5, [0.0; 3]);
        let world = CollisionWorld::flat_floor(-100000.0, 1000.0);
        g.update(0.5, &world);
        assert!(g.pos[2] > 100.0 && g.pos[1] < 100.0, "{:?}", g.pos);
    }
}
