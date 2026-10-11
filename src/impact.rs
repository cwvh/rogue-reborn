//! What a bullet leaves behind on a wall: the hole, and the debris thrown out.
//!
//! The original game stuck a small sprite on the surface. `key_alpha` makes
//! those sprites usable (they are stored on a flat background colour). The
//! "realistic" option instead draws a hole for the surface that was hit, with
//! splinters from wood, sparks off metal and dust and chips from plaster.

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn mul(a: V3, s: f32) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 1e-9 { mul(a, 1.0 / l) } else { [0.0, 1.0, 0.0] }
}

/// What a surface is made of, which decides the hole and the debris
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    /// Plaster, stone, brick, tile: anything hard and brittle
    Concrete,
    Wood,
    Metal,
    /// Dirt, grass, sand, snow
    Earth,
    Glass,
    /// Fabric and the like: marks but no debris
    Soft,
    /// The soot a blast leaves. Never what a material classifies as.
    Scorch,
}

impl Surface {
    /// Guess the surface from a MAP material's file name, e.g. "wood_edge_horiz.bmp"
    pub fn classify(material: &str) -> Surface {
        let m = material.to_ascii_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| m.contains(w));
        if has(&["glass"]) {
            Surface::Glass
        } else if has(&["wood", "plywood", "plnk", "plank", "crate", "wod", "beam", "panel_ornate", "wood_"]) {
            Surface::Wood
        } else if has(&["metal", "steal", "steel", "rust", "chain", "ibeam", "brass", "gold", "stainless", "hvac", "turbine", "cylander", "alum", "fuselage"]) {
            Surface::Metal
        } else if has(&["grass", "dirt", "mud", "sand", "snow_ground", "gravel", "clay", "ground"]) {
            Surface::Earth
        } else if has(&["carpet", "rug", "pillow", "sack", "burlap", "bed_", "seat", "curtain", "mummy", "cushion", "fabric"]) {
            Surface::Soft
        } else {
            Surface::Concrete
        }
    }

    /// How big the hole's picture is, in map units (about a centimetre each)
    pub fn decal_size(self, realistic: bool) -> f32 {
        match (self, realistic) {
            (Surface::Wood, true) => 20.0,
            (Surface::Wood, false) => 12.0,
            (Surface::Metal, true) => 11.0,
            (_, true) => 16.0,
            (_, false) => 10.0,
        }
    }
}

/// Make a sprite's flat background transparent. The sprite is `w` by `h` RGBA
/// pixels; the background colour is taken from its corners, and a pixel becomes
/// more opaque the further its colour is from that.
pub fn key_alpha(rgba: &mut [u8], w: usize, h: usize) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return;
    }
    let at = |x: usize, y: usize| [rgba[(y * w + x) * 4], rgba[(y * w + x) * 4 + 1], rgba[(y * w + x) * 4 + 2]];
    let corners = [at(0, 0), at(w - 1, 0), at(0, h - 1), at(w - 1, h - 1)];
    let key: Vec<f32> = (0..3).map(|c| corners.iter().map(|p| p[c] as f32).sum::<f32>() / 4.0).collect();
    for px in rgba.chunks_exact_mut(4).take(w * h) {
        let d = (0..3).map(|c| (px[c] as f32 - key[c]).abs()).fold(0.0, f32::max);
        px[3] = (((d - 10.0) / 45.0).clamp(0.0, 1.0) * 255.0) as u8;
    }
}

// ---- A hole on a wall ----------------------------------------------------------

/// A hole stuck to a surface
#[derive(Clone, Copy, Debug)]
pub struct Decal {
    pub pos: V3,
    pub normal: V3,
    /// A direction along the surface that the picture's "up" lines up with
    pub up: V3,
    pub size: f32,
    pub surface: Surface,
    /// Which of the surface's pictures it uses
    pub variant: usize,
}

impl Decal {
    /// Place a decal where a bullet struck: `normal` faces out of the surface and
    /// `roll` (0..1) turns the picture about it. Wood grain is kept running
    /// up the wall, whatever the roll.
    pub fn new(pos: V3, normal: V3, surface: Surface, realistic: bool, variant: usize, roll: f32) -> Decal {
        let n = norm(normal);
        let world_up = if n[1].abs() > 0.95 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        let mut up = norm(sub(world_up, mul(n, dot(world_up, n))));
        let angle = if surface == Surface::Wood { (roll - 0.5) * 0.3 } else { roll * std::f32::consts::TAU };
        let side = cross(n, up);
        up = add(mul(up, angle.cos()), mul(side, angle.sin()));
        // Sit just off the surface so it is not lost in it
        Decal { pos: add(pos, mul(n, 0.5)), normal: n, up, size: surface.decal_size(realistic), surface, variant }
    }

    /// The soot a blast leaves on a surface, `size` across
    pub fn scorch(pos: V3, normal: V3, size: f32, variant: usize, roll: f32) -> Decal {
        Decal { size, ..Decal::new(pos, normal, Surface::Scorch, true, variant, roll) }
    }

    /// The four corners, counter-clockwise seen from the front, starting bottom-left,
    /// with texture coordinates (0,0), (1,0), (1,1), (0,1)
    pub fn corners(&self) -> [V3; 4] {
        let side = norm(cross(self.up, self.normal));
        let (r, u) = (mul(side, self.size / 2.0), mul(self.up, self.size / 2.0));
        [sub(sub(self.pos, r), u), sub(add(self.pos, r), u), add(add(self.pos, r), u), add(sub(self.pos, r), u)]
    }
}

// ---- Drawing a hole --------------------------------------------------------------

/// Side of the generated pictures, in pixels
pub const TEXTURE_SIZE: usize = 64;

fn hash(seed: u64, x: i32, y: i32) -> f32 {
    let mut h = seed ^ ((x as u64).wrapping_mul(0x9E3779B97F4A7C15)) ^ ((y as u64).wrapping_mul(0xC2B2AE3D27D4EB4F));
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51AFD7ED558CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CEB9FE1A85EC53);
    h ^= h >> 33;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Smooth value noise, 0..1
fn noise(seed: u64, x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (smooth(x - ix as f32), smooth(y - iy as f32));
    let a = hash(seed, ix, iy) * (1.0 - fx) + hash(seed, ix + 1, iy) * fx;
    let b = hash(seed, ix, iy + 1) * (1.0 - fx) + hash(seed, ix + 1, iy + 1) * fx;
    a * (1.0 - fy) + b * fy
}

fn fbm(seed: u64, x: f32, y: f32) -> f32 {
    noise(seed, x, y) * 0.55 + noise(seed ^ 0x55, x * 2.1, y * 2.1) * 0.3 + noise(seed ^ 0xAA, x * 4.3, y * 4.3) * 0.15
}

struct Rng(u64);

impl Rng {
    /// Start from any seed, however small: it is scrambled so the first numbers are not
    fn new(seed: u64) -> Rng {
        let mut z = seed.wrapping_add(0x9E3779B97F4A7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        Rng((z ^ (z >> 31)) | 1)
    }

    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.next() * (hi - lo)
    }
}

/// A square picture, built up by laying shapes over each other
struct Canvas {
    px: Vec<[f32; 4]>,
}

impl Canvas {
    fn new() -> Canvas {
        Canvas { px: vec![[0.0; 4]; TEXTURE_SIZE * TEXTURE_SIZE] }
    }

    /// Lay a shape over the picture: `f(x, y)` gives the colour and opacity at each
    /// pixel, with the middle of the picture at (0, 0)
    fn lay(&mut self, f: impl Fn(f32, f32) -> ([f32; 3], f32)) {
        let c = TEXTURE_SIZE as f32 / 2.0;
        for y in 0..TEXTURE_SIZE {
            for x in 0..TEXTURE_SIZE {
                let (col, a) = f(x as f32 + 0.5 - c, y as f32 + 0.5 - c);
                let a = a.clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                let d = &mut self.px[y * TEXTURE_SIZE + x];
                let out = a + d[3] * (1.0 - a);
                for k in 0..3 {
                    d[k] = (col[k] * a + d[k] * d[3] * (1.0 - a)) / out;
                }
                d[3] = out;
            }
        }
    }

    fn into_rgba(self) -> Vec<u8> {
        self.px.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8)).collect()
    }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Distance from a point to the segment `a`-`b`, and how far along it the closest point is (0..1)
fn segment(px: f32, py: f32, a: [f32; 2], b: [f32; 2]) -> (f32, f32) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = (dx * dx + dy * dy).max(1e-6);
    let t = (((px - a[0]) * dx + (py - a[1]) * dy) / l2).clamp(0.0, 1.0);
    let (cx, cy) = (a[0] + dx * t, a[1] + dy * t);
    (((px - cx).powi(2) + (py - cy).powi(2)).sqrt(), t)
}

/// A hole that is bright where it is eaten out of plaster, stone or dirt: a
/// black hole in a crater of pale dust, with cracks running away from it and
/// chips scattered about. `tint` is the colour of the dust.
fn crater(seed: u64, tint: [f32; 3]) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    let mut cv = Canvas::new();
    let irregular = move |s: u64, x: f32, y: f32, amount: f32| {
        // How far out this direction reaches, 1 +/- amount
        let a = y.atan2(x);
        1.0 + (noise(s, 4.0 + a.cos() * 2.2, 4.0 + a.sin() * 2.2) - 0.5) * 2.0 * amount
    };
    let (r_hole, r_crater, r_soot) = (rng.range(3.0, 3.8), rng.range(9.0, 12.0), rng.range(24.0, 29.0));

    // Soot and scuffing round about
    cv.lay(|x, y| {
        let r = (x * x + y * y).sqrt() / irregular(seed ^ 1, x, y, 0.35);
        ([0.1, 0.1, 0.1], 0.3 * (1.0 - smooth(r / r_soot)) * (0.5 + fbm(seed ^ 2, x * 0.2, y * 0.2)))
    });
    // The crater, pale where the surface has flaked off
    cv.lay(|x, y| {
        let r = (x * x + y * y).sqrt() / irregular(seed ^ 3, x, y, 0.4);
        let inside = 1.0 - smooth((r - r_crater * 0.8) / (r_crater * 0.25));
        let shade = fbm(seed ^ 4, x * 0.35, y * 0.35);
        // Darker at the edge, where the lip casts a shadow
        let lip = smooth((r - r_crater * 0.55) / (r_crater * 0.45));
        (mix(mix(tint, mul(tint, 0.6), shade), mul(tint, 0.4), lip * 0.6), inside * 0.95)
    });
    // Cracks
    for _ in 0..(5 + (rng.next() * 4.0) as usize) {
        let a = rng.range(0.0, std::f32::consts::TAU);
        let (from, len) = (r_crater * 0.6, rng.range(r_crater * 1.1, r_crater * 2.4));
        let start = [a.cos() * from, a.sin() * from];
        let bend = rng.range(-0.25, 0.25);
        let end = [(a + bend).cos() * (from + len), (a + bend).sin() * (from + len)];
        cv.lay(move |x, y| {
            let (d, t) = segment(x, y, start, end);
            ([0.15, 0.14, 0.13], 0.7 * (1.0 - smooth(d / (1.1 - 0.6 * t))) * (1.0 - t * 0.6))
        });
    }
    // Chips lying round the hole
    for _ in 0..14 {
        let (a, r) = (rng.range(0.0, std::f32::consts::TAU), rng.range(r_crater * 0.9, r_soot * 0.9));
        let (cx, cy, cr) = (a.cos() * r, a.sin() * r, rng.range(0.7, 1.7));
        cv.lay(move |x, y| (mul(tint, 1.05), 0.8 * (1.0 - smooth(((x - cx).powi(2) + (y - cy).powi(2)).sqrt() / cr))));
    }
    // The hole itself, black in the middle with a ragged edge
    cv.lay(|x, y| {
        let r = (x * x + y * y).sqrt() / irregular(seed ^ 5, x, y, 0.3);
        let d = 1.0 - smooth((r - r_hole * 0.7) / (r_hole * 0.5));
        let depth = 1.0 - smooth(r / r_hole);
        ([0.02 + 0.08 * (1.0 - depth), 0.02 + 0.08 * (1.0 - depth), 0.02 + 0.07 * (1.0 - depth)], d)
    });
    cv.into_rgba()
}

/// A hole in wood: the grain runs up the picture, and the wood tears along it,
/// leaving long pale splinters standing out of a dark ragged crater.
fn wood_hole(seed: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    let mut cv = Canvas::new();
    let r_hole = rng.range(2.8, 3.5);
    // The torn crater, longer along the grain
    let (ax, ay) = (rng.range(6.0, 8.0), rng.range(12.0, 17.0));
    cv.lay(|x, y| {
        let wob = 1.0 + (noise(seed ^ 1, x * 0.3 + 9.0, y * 0.3 + 9.0) - 0.5) * 0.7;
        let r = ((x / ax).powi(2) + (y / ay).powi(2)).sqrt() / wob;
        let streak = noise(seed ^ 2, x * 1.4, y * 0.12); // fibres running along the grain
        (mix([0.2, 0.11, 0.05], [0.42, 0.26, 0.12], streak), 0.9 * (1.0 - smooth((r - 0.6) / 0.4)))
    });
    // Splinters, mostly lying along the grain, some torn out to the side
    for _ in 0..(14 + (rng.next() * 8.0) as usize) {
        let along = rng.next() < 0.75;
        let mut a = if along {
            std::f32::consts::FRAC_PI_2 + rng.range(-0.3, 0.3) + if rng.next() < 0.5 { std::f32::consts::PI } else { 0.0 }
        } else {
            rng.range(0.0, std::f32::consts::TAU)
        };
        let len = if along { rng.range(10.0, 29.0) } else { rng.range(5.0, 13.0) };
        let width = rng.range(1.2, 2.8);
        let from = rng.range(1.0, 5.0);
        a += 0.0;
        let start = [a.cos() * from, a.sin() * from];
        let bend = rng.range(-0.18, 0.18);
        let end = [start[0] + (a + bend).cos() * len, start[1] + (a + bend).sin() * len];
        let pale = mix([0.86, 0.68, 0.42], [0.74, 0.54, 0.3], rng.next());
        cv.lay(move |x, y| {
            let (d, t) = segment(x, y, start, end);
            let w = width * (1.0 - t * 0.92);
            if d > w {
                return (pale, 0.0);
            }
            // The edges are darker, the flat of the splinter pale
            let edge = smooth((d / w - 0.55) / 0.45);
            (mix(pale, [0.4, 0.25, 0.12], edge * 0.8), 0.95 * (1.0 - smooth((d - w * 0.85) / (w * 0.15 + 0.01))))
        });
    }
    // The hole, a little longer along the grain
    cv.lay(|x, y| {
        let r = ((x / r_hole).powi(2) + (y / (r_hole * 1.25)).powi(2)).sqrt()
            / (1.0 + (noise(seed ^ 3, x * 0.5 + 3.0, y * 0.5 + 3.0) - 0.5) * 0.5);
        let depth = 1.0 - smooth(r);
        ([0.03 + 0.1 * (1.0 - depth), 0.02 + 0.06 * (1.0 - depth), 0.01 + 0.03 * (1.0 - depth)], 1.0 - smooth((r - 0.8) / 0.3))
    });
    cv.into_rgba()
}

/// A hole punched through sheet metal: a black hole with its edge turned in as
/// petals, a bright burnished lip, and paint scuffed back to bare steel.
fn metal_hole(seed: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    let mut cv = Canvas::new();
    let r0 = rng.range(2.6, 3.3);
    let petals = 6 + (rng.next() * 3.0) as i32;
    let phase = rng.range(0.0, std::f32::consts::TAU);
    let r_scuff = rng.range(15.0, 20.0);
    // Paint scuffed off to bare metal, streaked outward
    cv.lay(|x, y| {
        let a = y.atan2(x);
        let r = (x * x + y * y).sqrt();
        let reach = r_scuff * (0.6 + 0.7 * noise(seed ^ 1, 3.0 + a.cos() * 3.0 + a * 2.0, 3.0 + a.sin() * 3.0));
        let streak = 0.6 + 0.6 * noise(seed ^ 2, a * 9.0, 1.0);
        (mix([0.5, 0.53, 0.58], [0.72, 0.75, 0.8], noise(seed ^ 3, x * 0.4, y * 0.4)), 0.45 * (1.0 - smooth(r / reach)) * streak)
    });
    // The petals of metal pushed in round the hole
    cv.lay(|x, y| {
        let a = y.atan2(x);
        let r = (x * x + y * y).sqrt();
        let reach = r0 * (1.9 + 0.7 * noise(seed ^ 4, a * 3.0 + 5.0, 2.0));
        let lit = 0.5 + 0.5 * (a * petals as f32 + phase).sin();
        (mix([0.4, 0.42, 0.46], [0.8, 0.82, 0.87], lit), 0.9 * (1.0 - smooth((r - reach) / 1.2)))
    });
    // A bright lip
    cv.lay(move |x, y| {
        let r = (x * x + y * y).sqrt();
        let lip = 1.0 - smooth(((r - r0 * 1.15).abs()) / (r0 * 0.28));
        ([0.95, 0.96, 0.98], 0.65 * lip)
    });
    // Scratches where the bullet dragged the paint
    for _ in 0..(3 + (rng.next() * 3.0) as usize) {
        let a = rng.range(0.0, std::f32::consts::TAU);
        let (from, len) = (r0 * 1.6, rng.range(7.0, 16.0));
        let (start, end) = ([a.cos() * from, a.sin() * from], [a.cos() * (from + len), a.sin() * (from + len)]);
        cv.lay(move |x, y| {
            let (d, t) = segment(x, y, start, end);
            ([0.95, 0.96, 0.98], 0.55 * (1.0 - smooth(d / 0.8)) * (1.0 - t))
        });
    }
    // The hole
    cv.lay(move |x, y| {
        let a = y.atan2(x);
        let r = (x * x + y * y).sqrt() / (1.0 + (noise(seed ^ 5, 4.0 + a.cos() * 2.0, 4.0 + a.sin() * 2.0) - 0.5) * 0.35);
        ([0.015, 0.015, 0.02], 1.0 - smooth((r - r0 * 0.8) / (r0 * 0.3)))
    });
    cv.into_rgba()
}

/// Soot thrown out from a blast: dark and thick in the middle, with ragged, streaky edges
fn scorch(seed: u64) -> Vec<u8> {
    let mut cv = Canvas::new();
    let r_max = TEXTURE_SIZE as f32 * 0.47;
    cv.lay(|x, y| {
        let a = y.atan2(x);
        let r = (x * x + y * y).sqrt();
        let reach = r_max * (0.55 + 0.45 * noise(seed ^ 1, 3.0 + a.cos() * 2.5, 3.0 + a.sin() * 2.5));
        let streak = (0.75 + 0.5 * noise(seed ^ 2, a * 7.0, 1.0)).min(1.1);
        let grain = fbm(seed ^ 3, x * 0.22, y * 0.22);
        ([0.05, 0.045, 0.04], 0.85 * (1.0 - smooth(r / (reach * streak))) * (0.55 + 0.6 * grain))
    });
    cv.into_rgba()
}

/// The picture of a hole in `surface`, `TEXTURE_SIZE` pixels square, RGBA. `seed` makes each one different.
pub fn hole_texture(surface: Surface, seed: u64) -> Vec<u8> {
    match surface {
        Surface::Wood => wood_hole(seed),
        Surface::Metal => metal_hole(seed),
        Surface::Earth => crater(seed, [0.42, 0.33, 0.22]),
        Surface::Soft => crater(seed, [0.45, 0.44, 0.42]),
        Surface::Concrete | Surface::Glass => crater(seed, [0.80, 0.78, 0.72]),
        Surface::Scorch => scorch(seed),
    }
}

// ---- Debris ------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A thin sliver of wood
    Splinter,
    /// A hot streak of metal
    Spark,
    /// A small hard fragment
    Chip,
}

#[derive(Clone, Debug)]
pub struct Particle {
    pub kind: Kind,
    pub pos: V3,
    vel: V3,
    pub age: f32,
    pub life: f32,
    pub color: [f32; 3],
    /// How long the piece is, for drawing it as a streak
    pub len: f32,
    gravity: f32,
}

impl Particle {
    /// The tail of the streak: where the particle was a moment ago
    pub fn tail(&self) -> V3 {
        let v = norm(self.vel);
        let l = match self.kind {
            Kind::Spark => (dot(self.vel, self.vel).sqrt() * 0.025).clamp(0.5, self.len),
            _ => self.len,
        };
        sub(self.pos, mul(v, l))
    }

    /// 1 when new, fading to 0
    pub fn alpha(&self) -> f32 {
        (1.0 - self.age / self.life).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PuffKind {
    Dust,
    /// A bright flash, drawn with a spark sprite
    Flash,
    /// The white-hot instant of a blast
    Glare,
    /// Dark smoke and dust thrown up by a blast
    Smoke,
    /// The shockwave: a ring spreading over the surface (see `Puff::axis`)
    Ring,
}

/// A soft cloud that swells and fades
#[derive(Clone, Debug)]
pub struct Puff {
    pub kind: PuffKind,
    pub pos: V3,
    vel: V3,
    pub age: f32,
    pub life: f32,
    pub size: (f32, f32),
    pub color: [f32; 3],
    /// For a ring: the direction it faces (the surface's normal). Other puffs face the camera.
    pub axis: Option<V3>,
}

impl Puff {
    pub fn alpha(&self) -> f32 {
        (1.0 - self.age / self.life).clamp(0.0, 1.0)
    }

    /// Current size, growing from the first to the second
    pub fn current_size(&self) -> f32 {
        self.size.0 + (self.size.1 - self.size.0) * (self.age / self.life).clamp(0.0, 1.0)
    }
}

/// Everything in the air from bullet impacts
#[derive(Clone, Debug, Default)]
pub struct Effects {
    pub particles: Vec<Particle>,
    pub puffs: Vec<Puff>,
}

/// Chance a bullet striking metal throws sparks
pub const SPARK_CHANCE: f32 = 0.4;

impl Effects {
    /// Throw debris from a bullet hitting `surface` at `point`. `normal` faces out
    /// of the surface and `dir` is the bullet's direction. Returns whether metal
    /// sparked, so the caller can play a ricochet.
    pub fn impact(&mut self, surface: Surface, point: V3, normal: V3, dir: V3, seed: u64) -> bool {
        let mut rng = Rng::new(seed);
        let n = norm(normal);
        let p = add(point, mul(n, 0.6));
        // A random direction along the surface
        let along = |rng: &mut Rng| {
            let r = norm([rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)]);
            norm(sub(r, mul(n, dot(r, n))))
        };
        // The bullet's direction bounced off the surface
        let bounce = norm(sub(dir, mul(n, 2.0 * dot(dir, n))));
        let mut sparked = false;
        match surface {
            Surface::Wood => {
                for _ in 0..(10 + (rng.next() * 9.0) as usize) {
                    let speed = rng.range(120.0, 420.0);
                    let v = add(add(mul(n, speed), mul(along(&mut rng), rng.range(0.0, 160.0))), mul(bounce, rng.range(0.0, 60.0)));
                    self.particles.push(Particle {
                        kind: Kind::Splinter,
                        pos: p,
                        vel: v,
                        age: 0.0,
                        life: rng.range(0.5, 1.1),
                        color: mix([0.88, 0.7, 0.45], [0.6, 0.42, 0.24], rng.next()),
                        len: rng.range(1.5, 5.5),
                        gravity: 980.0,
                    });
                }
                self.dust(&mut rng, p, n, [0.6, 0.48, 0.32], 2, 0.2);
            }
            Surface::Metal => {
                if rng.next() < SPARK_CHANCE {
                    sparked = true;
                    // Sparks keep most of the bullet's speed and carry on much the way it was
                    // going, skimming off the surface: a few fast streaks in a narrow fan, not a plume
                    let along_surface = norm(sub(dir, mul(n, dot(dir, n))));
                    let heading = norm(add(mul(bounce, 0.55), mul(along_surface, 0.45)));
                    for _ in 0..(2 + (rng.next() * 4.0) as usize) {
                        let speed = rng.range(900.0, 2600.0);
                        let fan = add(mul(along(&mut rng), rng.range(0.0, 0.16)), mul(n, rng.range(0.0, 0.12)));
                        let v = mul(norm(add(heading, fan)), speed);
                        self.particles.push(Particle {
                            kind: Kind::Spark,
                            pos: p,
                            vel: v,
                            age: 0.0,
                            life: rng.range(0.08, 0.28),
                            color: mix([1.0, 0.96, 0.8], [1.0, 0.6, 0.2], rng.next()),
                            len: rng.range(6.0, 16.0),
                            gravity: 500.0,
                        });
                    }
                    self.puffs.push(Puff { kind: PuffKind::Flash, pos: p, vel: [0.0; 3], age: 0.0, life: 0.04, size: (4.0, 7.0), color: [1.0, 0.9, 0.6], axis: None });
                }
            }
            Surface::Concrete | Surface::Earth => {
                let dust = if surface == Surface::Earth { [0.45, 0.36, 0.25] } else { [0.72, 0.7, 0.66] };
                for _ in 0..(5 + (rng.next() * 5.0) as usize) {
                    let v = add(mul(n, rng.range(90.0, 320.0)), mul(along(&mut rng), rng.range(0.0, 200.0)));
                    self.particles.push(Particle {
                        kind: Kind::Chip,
                        pos: p,
                        vel: v,
                        age: 0.0,
                        life: rng.range(0.4, 0.9),
                        color: mix(dust, mul(dust, 0.6), rng.next()),
                        len: rng.range(0.6, 1.4),
                        gravity: 980.0,
                    });
                }
                self.dust(&mut rng, p, n, dust, 3, 0.35);
            }
            Surface::Glass | Surface::Soft | Surface::Scorch => {}
        }
        sparked
    }

    /// A grenade going off at `point` on a surface facing `normal`. Not flames: a hard white
    /// flash, a shockwave spreading over the surface, a wave of dust and a dark cloud lifting
    /// off it, and debris flung out.
    /// `on_surface` is false for a blast in mid-air, which has no ground to throw dust along.
    pub fn explosion(&mut self, point: V3, normal: V3, seed: u64, on_surface: bool) {
        let mut rng = Rng::new(seed);
        let n = norm(normal);
        let p = add(point, mul(n, 6.0));
        let along = |rng: &mut Rng| {
            let r = norm([rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)]);
            r
        };
        let puff = |kind, pos, vel, life, size, color, axis| Puff { kind, pos, vel, age: 0.0, life, size, color, axis };
        // The flash, and the ring racing out
        self.puffs.push(puff(PuffKind::Glare, add(p, mul(n, 25.0)), [0.0; 3], 0.22, (260.0, 1000.0), [1.0, 0.96, 0.88], None));
        self.puffs.push(puff(PuffKind::Flash, add(p, mul(n, 25.0)), [0.0; 3], 0.10, (200.0, 500.0), [1.0, 0.9, 0.7], None));
        if on_surface {
            self.puffs.push(puff(PuffKind::Ring, add(point, mul(n, 3.0)), [0.0; 3], 0.6, (100.0, 1500.0), [0.95, 0.93, 0.88], Some(n)));
        }
        // A wave of dust along the surface
        for _ in 0..if on_surface { 30 } else { 0 } {
            let d = along(&mut rng);
            let flat = norm(sub(d, mul(n, dot(d, n))));
            let vel = add(mul(flat, rng.range(500.0, 1700.0)), mul(n, rng.range(20.0, 260.0)));
            self.puffs.push(puff(PuffKind::Dust, p, vel, rng.range(1.6, 3.2), (rng.range(80.0, 140.0), rng.range(340.0, 600.0)), mix([0.78, 0.75, 0.7], [0.55, 0.52, 0.48], rng.next()), None));
        }
        // Dark smoke and dust billowing up from the middle
        for _ in 0..10 {
            let d = along(&mut rng);
            let flat = norm(sub(d, mul(n, dot(d, n))));
            let vel = if on_surface {
                add(mul(flat, rng.range(60.0, 320.0)), mul(n, rng.range(200.0, 650.0)))
            } else {
                // Spreading every way from the burst
                mul(d, rng.range(80.0, 380.0))
            };
            self.puffs.push(puff(PuffKind::Smoke, add(p, mul(n, 10.0)), vel, rng.range(2.5, 4.5), (rng.range(120.0, 180.0), rng.range(380.0, 650.0)), mix([0.34, 0.32, 0.3], [0.7, 0.66, 0.6], rng.next()), None));
        }
        // Debris flung out, mostly low and fast
        for _ in 0..70 {
            let d = along(&mut rng);
            let v = add(mul(d, rng.range(400.0, 2200.0)), if on_surface { mul(n, rng.range(150.0, 900.0)) } else { [0.0; 3] });
            self.particles.push(Particle {
                kind: Kind::Chip,
                pos: p,
                vel: v,
                age: 0.0,
                life: rng.range(0.6, 1.5),
                color: mix([0.7, 0.67, 0.62], [0.32, 0.3, 0.28], rng.next()),
                len: rng.range(0.8, 2.2),
                gravity: 980.0,
            });
        }
        // A few fast glints of casing
        for _ in 0..10 {
            let d = along(&mut rng);
            self.particles.push(Particle {
                kind: Kind::Spark,
                pos: p,
                vel: mul(d, rng.range(1400.0, 3200.0)),
                age: 0.0,
                life: rng.range(0.08, 0.22),
                color: mix([1.0, 0.95, 0.8], [1.0, 0.7, 0.3], rng.next()),
                len: rng.range(8.0, 22.0),
                gravity: 400.0,
            });
        }
    }

    fn dust(&mut self, rng: &mut Rng, p: V3, n: V3, color: [f32; 3], count: usize, strength: f32) {
        for _ in 0..count {
            let drift = [rng.range(-12.0, 12.0), rng.range(0.0, 22.0), rng.range(-12.0, 12.0)];
            self.puffs.push(Puff {
                kind: PuffKind::Dust,
                pos: p,
                vel: add(mul(n, rng.range(15.0, 55.0)), drift),
                age: 0.0,
                life: rng.range(0.5, 1.0),
                size: (rng.range(2.0, 4.0), rng.range(9.0, 17.0)),
                color: mul(color, 1.0 + strength * 0.0),
                axis: None,
            });
        }
    }

    pub fn update(&mut self, dt: f32) {
        for p in &mut self.particles {
            p.age += dt;
            p.vel[1] -= p.gravity * dt;
            p.pos = add(p.pos, mul(p.vel, dt));
        }
        self.particles.retain(|p| p.age < p.life);
        for f in &mut self.puffs {
            f.age += dt;
            f.pos = add(f.pos, mul(f.vel, dt));
            f.vel = mul(f.vel, (1.0 - 2.5 * dt).max(0.0));
        }
        self.puffs.retain(|f| f.age < f.life);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surfaces_come_from_material_names() {
        for (name, want) in [
            ("wood_edge_horiz.bmp", Surface::Wood),
            ("Plywood_Tan.bmp", Surface::Wood),
            ("rusty_steal.bmp", Surface::Metal),
            ("metaloldgray.bmp", Surface::Metal),
            ("grass_clump.tga", Surface::Earth),
            ("wallmountglass01.bmp", Surface::Glass),
            ("carpet_blue.bmp", Surface::Soft),
            ("concrete.bmp", Surface::Concrete),
            ("cinderblock1.bmp", Surface::Concrete),
        ] {
            assert_eq!(Surface::classify(name), want, "{name}");
        }
    }

    #[test]
    fn a_sprites_flat_background_becomes_transparent() {
        let mut px = vec![150u8; 8 * 8 * 4];
        let mid = (4 * 8 + 4) * 4;
        px[mid..mid + 3].copy_from_slice(&[10, 10, 10]);
        key_alpha(&mut px, 8, 8);
        assert_eq!(px[3], 0);
        assert_eq!(px[mid + 3], 255);
    }

    #[test]
    fn decals_lie_flat_on_the_wall() {
        // A wall facing +Z
        let d = Decal::new([100.0, 50.0, 20.0], [0.0, 0.0, 1.0], Surface::Concrete, false, 0, 0.3);
        let c = d.corners();
        for p in c {
            assert!((p[2] - 20.5).abs() < 1e-3, "off the plane: {p:?}");
        }
        let side = |a: V3, b: V3| dot(sub(a, b), sub(a, b)).sqrt();
        assert!((side(c[0], c[1]) - d.size).abs() < 1e-3 && (side(c[1], c[2]) - d.size).abs() < 1e-3);
        // Wood keeps its grain running up the wall
        let w = Decal::new([0.0; 3], [0.0, 0.0, 1.0], Surface::Wood, true, 0, 0.9);
        assert!(w.up[1] > 0.95, "{:?}", w.up);
    }

    #[test]
    fn every_hole_has_a_dark_middle_and_clear_corners() {
        for surface in [Surface::Concrete, Surface::Wood, Surface::Metal, Surface::Earth] {
            for seed in 1..6u64 {
                let px = hole_texture(surface, seed);
                assert_eq!(px.len(), TEXTURE_SIZE * TEXTURE_SIZE * 4);
                let at = |x: usize, y: usize| &px[(y * TEXTURE_SIZE + x) * 4..(y * TEXTURE_SIZE + x) * 4 + 4];
                let mid = at(TEXTURE_SIZE / 2, TEXTURE_SIZE / 2);
                assert!(mid[3] > 240 && mid[0] < 40 && mid[1] < 40, "{surface:?} {seed}: middle is {mid:?}");
                assert_eq!(at(0, 0)[3], 0, "{surface:?} {seed}: corner is not clear");
                assert_eq!(at(TEXTURE_SIZE - 1, TEXTURE_SIZE - 1)[3], 0);
                // A real mark, not just a speck
                let covered = px.chunks_exact(4).filter(|p| p[3] > 128).count();
                assert!(covered > 60, "{surface:?} {seed}: only {covered} pixels");
            }
        }
    }

    #[test]
    fn different_seeds_give_different_holes() {
        assert_ne!(hole_texture(Surface::Wood, 1), hole_texture(Surface::Wood, 2));
    }

    #[test]
    fn wood_splinters_metal_sometimes_sparks_and_plaster_gives_dust() {
        let (n, dir) = ([0.0, 0.0, 1.0], [0.0, 0.0, -1.0]);
        let mut wood = Effects::default();
        assert!(!wood.impact(Surface::Wood, [0.0; 3], n, dir, 5));
        assert!(wood.particles.iter().filter(|p| p.kind == Kind::Splinter).count() >= 10);

        let mut sparks = 0;
        for seed in 0..400 {
            let mut e = Effects::default();
            if e.impact(Surface::Metal, [0.0; 3], n, dir, seed * 7919 + 1) {
                sparks += 1;
                assert!(e.particles.iter().all(|p| p.kind == Kind::Spark) && e.particles.len() >= 2 && e.particles.len() <= 5);
                assert!(e.puffs.iter().any(|p| p.kind == PuffKind::Flash));
            } else {
                assert!(e.particles.is_empty());
            }
        }
        assert!((100..220).contains(&sparks), "{sparks} of 400 sparked");

        let mut plaster = Effects::default();
        plaster.impact(Surface::Concrete, [0.0; 3], n, dir, 9);
        assert!(plaster.particles.iter().all(|p| p.kind == Kind::Chip));
        assert!(plaster.puffs.iter().any(|p| p.kind == PuffKind::Dust));
        let mut none = Effects::default();
        none.impact(Surface::Glass, [0.0; 3], n, dir, 9);
        assert!(none.particles.is_empty() && none.puffs.is_empty());
    }

    #[test]
    fn debris_flies_out_of_the_wall_falls_and_dies() {
        let mut e = Effects::default();
        e.impact(Surface::Wood, [0.0, 100.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0], 3);
        // Thrown out of the wall, not into it
        assert!(e.particles.iter().all(|p| p.vel[2] > 0.0));
        for _ in 0..30 {
            e.update(1.0 / 60.0);
        }
        assert!(e.particles.iter().all(|p| p.pos[2] > 0.0 && p.pos[1] < 100.0));
        for _ in 0..120 {
            e.update(1.0 / 60.0);
        }
        assert!(e.particles.is_empty() && e.puffs.is_empty());
    }
}
