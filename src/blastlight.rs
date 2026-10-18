//! The light of a blast: for a moment everything near it is lit, as if by a camera flash.
//!
//! The renderer has no lighting, so this lights surfaces the old way. Each surface that
//! faces the blast, with nothing in between, gets a second, additive pass with a round
//! patch of light laid over its plane, centered under the blast and as wide as the light
//! reaches there.

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> Option<V3> {
    let l = dot(a, a).sqrt();
    (l > 1e-6).then(|| [a[0] / l, a[1] / l, a[2] / l])
}

/// How far the light reaches, in map units
pub const RADIUS: f32 = 1000.0;
/// Seconds until it is out: about as long as the fragments are in flight
pub const DURATION: f32 = 0.08;
/// How many lights are kept at once
pub const MOST: usize = 4;
/// How much of the light a character takes, next to a wall: a body is lit less than plaster
/// because it is added over a texture that is already bright
const BODY_EXPOSURE: f32 = 0.8;
/// Side of the picture of a patch of light, in pixels
pub const TEXTURE_SIZE: usize = 128;

/// One surface the light reaches
#[derive(Clone, Debug)]
pub struct Lit {
    pub corners: [V3; 3],
    /// Where each corner falls on the patch of light (0..1 across it, the middle at 0.5)
    pub uv: [[f32; 2]; 3],
    /// How much of the light each corner takes: more when near and square on. Per corner, so
    /// it blends across the surface and the triangles of a wall show no seams.
    pub strength: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct BlastLight {
    pub pos: V3,
    pub age: f32,
    pub lit: Vec<Lit>,
}

impl BlastLight {
    /// A light at `pos` over the triangles `near` it. `blocked(from, to)` says whether something
    /// solid stands between two points.
    pub fn new(pos: V3, near: &[[V3; 3]], blocked: &dyn Fn(V3, V3) -> bool) -> BlastLight {
        let mut lit = Vec::new();
        for t in near {
            let Some(n) = norm(cross(sub(t[1], t[0]), sub(t[2], t[0]))) else { continue };
            // Which side faces the light is the side the light is on
            let mut n = n;
            let mut d = dot(n, sub(pos, t[0]));
            if d < 0.0 {
                n = [-n[0], -n[1], -n[2]];
                d = -d;
            }
            if d >= RADIUS {
                continue;
            }
            // The patch is a circle on the plane around the point under the light, as wide
            // as the light's sphere is where it cuts the plane
            let under = [pos[0] - n[0] * d, pos[1] - n[1] * d, pos[2] - n[2] * d];
            let r = (RADIUS * RADIUS - d * d).sqrt();
            let across = norm(cross(n, if n[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] })).unwrap_or([1.0, 0.0, 0.0]);
            let up = cross(n, across);
            let mut uv = [[0.0; 2]; 3];
            for (k, c) in t.iter().enumerate() {
                let from = sub(*c, under);
                uv[k] = [0.5 + dot(from, across) / (2.0 * r), 0.5 + dot(from, up) / (2.0 * r)];
            }
            // Nothing of the triangle is inside the circle
            let centroid = [(t[0][0] + t[1][0] + t[2][0]) / 3.0, (t[0][1] + t[1][1] + t[2][1]) / 3.0, (t[0][2] + t[1][2] + t[2][2]) / 3.0];
            if uv.iter().all(|p| p[0] < 0.0) || uv.iter().all(|p| p[0] > 1.0) || uv.iter().all(|p| p[1] < 0.0) || uv.iter().all(|p| p[1] > 1.0) {
                continue;
            }
            if blocked(pos, centroid) {
                continue;
            }
            // Surfaces edge-on to the light still catch some of it
            let near_ness = 1.0 - (d / RADIUS) * (d / RADIUS);
            let strength = t.map(|c| (0.4 + 0.6 * dot(n, norm(sub(pos, c)).unwrap_or(n)).max(0.0)) * near_ness);
            lit.push(Lit { corners: *t, uv, strength });
        }
        BlastLight { pos, age: 0.0, lit }
    }

    pub fn update(&mut self, dt: f32) {
        self.age += dt;
    }

    /// The light this puts on something at `point` (a character's chest, say): its color, as
    /// bright as it is there, or none if it is out of reach or `blocked(from, to)` says
    /// something solid is between. Checked last, as it is the costly part.
    pub fn glow_at(&self, point: V3, blocked: &dyn Fn(V3, V3) -> bool) -> V3 {
        let d = sub(point, self.pos);
        let near = 1.0 - dot(d, d) / (RADIUS * RADIUS);
        let k = self.intensity() * near * BODY_EXPOSURE;
        if near <= 0.0 || k < 0.003 || blocked(self.pos, point) {
            return [0.0; 3];
        }
        self.color().map(|c| c * k)
    }

    pub fn done(&self) -> bool {
        self.age >= DURATION
    }

    /// How bright it is now: a hard flash that drops away fast and is out with the fragments
    pub fn intensity(&self) -> f32 {
        if self.done() {
            return 0.0;
        }
        let left = 1.0 - self.age / DURATION;
        1.5 * left * left
    }

    /// The color of the light: white at first, then a hot orange
    pub fn color(&self) -> V3 {
        let t = (self.age / DURATION).clamp(0.0, 1.0);
        [1.0, 0.93 - 0.38 * t, 0.78 - 0.58 * t]
    }
}

/// The picture of a patch of light, RGBA, bright in the middle and gone at the edge
pub fn texture() -> Vec<u8> {
    let n = TEXTURE_SIZE;
    let mut px = vec![255u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = ((x as f32 + 0.5) / n as f32 * 2.0 - 1.0, (y as f32 + 0.5) / n as f32 * 2.0 - 1.0);
            let r2 = (dx * dx + dy * dy).min(1.0);
            // Falls off quickly, and the very edge is zero so a surface can run on past it
            let v = (1.0 - r2) * (1.0 - r2);
            let b = (v * 255.0) as u8;
            let i = (y * n + x) * 4;
            px[i..i + 3].copy_from_slice(&[b, b, b]);
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOOR: [V3; 3] = [[-500.0, 0.0, -500.0], [500.0, 0.0, -500.0], [0.0, 0.0, 500.0]];

    #[test]
    fn a_floor_under_the_light_is_lit_with_the_light_in_the_middle_of_its_patch() {
        let light = BlastLight::new([0.0, 100.0, 0.0], &[FLOOR], &|_, _| false);
        assert_eq!(light.lit.len(), 1);
        let uv = light.lit[0].uv;
        // The point under the light is (0, 0, 0), which lies inside the triangle: its
        // coordinates bracket the middle of the patch
        assert!(uv.iter().any(|p| p[0] < 0.5) && uv.iter().any(|p| p[0] > 0.5));
        assert!(light.lit[0].strength.iter().all(|&s| s > 0.3 && s <= 1.0));
    }

    #[test]
    fn it_lights_the_side_facing_it_whichever_way_the_corners_wind() {
        let flipped = [FLOOR[0], FLOOR[2], FLOOR[1]];
        let above = BlastLight::new([0.0, 100.0, 0.0], &[flipped], &|_, _| false);
        let below = BlastLight::new([0.0, -100.0, 0.0], &[FLOOR], &|_, _| false);
        assert_eq!(above.lit.len(), 1);
        assert_eq!(below.lit.len(), 1);
    }

    #[test]
    fn nothing_out_of_reach_or_in_shadow_is_lit() {
        assert!(BlastLight::new([0.0, RADIUS + 1.0, 0.0], &[FLOOR], &|_, _| false).lit.is_empty());
        assert!(BlastLight::new([5000.0, 10.0, 0.0], &[FLOOR], &|_, _| false).lit.is_empty());
        assert!(BlastLight::new([0.0, 100.0, 0.0], &[FLOOR], &|_, _| true).lit.is_empty());
    }

    #[test]
    fn a_body_in_reach_and_in_sight_is_lit_and_one_behind_a_wall_is_not() {
        let light = BlastLight::new([0.0, 100.0, 0.0], &[], &|_, _| false);
        let near = light.glow_at([200.0, 100.0, 0.0], &|_, _| false);
        let far = light.glow_at([700.0, 100.0, 0.0], &|_, _| false);
        assert!(near[0] > far[0] && far[0] > 0.0, "{near:?} {far:?}");
        assert_eq!(light.glow_at([200.0, 100.0, 0.0], &|_, _| true), [0.0; 3]);
        assert_eq!(light.glow_at([RADIUS + 5.0, 100.0, 0.0], &|_, _| false), [0.0; 3]);
        let mut out = light.clone();
        out.update(DURATION);
        assert_eq!(out.glow_at([200.0, 100.0, 0.0], &|_, _| false), [0.0; 3]);
    }

    #[test]
    fn the_light_is_hardest_at_first_and_goes_out() {
        let mut light = BlastLight::new([0.0, 100.0, 0.0], &[], &|_, _| false);
        let mut last = light.intensity();
        assert!(last > 1.0);
        while !light.done() {
            light.update(1.0 / 60.0);
            assert!(light.intensity() <= last + 1e-6);
            last = light.intensity();
        }
        assert_eq!(light.intensity(), 0.0);
    }

    #[test]
    fn the_patch_is_bright_in_the_middle_and_dark_at_the_edge() {
        let px = texture();
        let at = |x: usize, y: usize| px[(y * TEXTURE_SIZE + x) * 4];
        assert!(at(TEXTURE_SIZE / 2, TEXTURE_SIZE / 2) > 240);
        assert_eq!(at(0, 0), 0);
        assert!(at(0, TEXTURE_SIZE / 2) < 4 && at(TEXTURE_SIZE - 1, TEXTURE_SIZE / 2) < 4);
    }
}
