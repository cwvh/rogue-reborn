//! Small helpers: a random number generator, vector arithmetic, and finding files in the game data.

use std::path::{Path, PathBuf};

/// The file in `dir` with this name and extension, whatever case they are in. The game's
/// data comes from a case-insensitive filesystem, so the names do not match in case.
pub fn find_file(dir: &Path, stem: &str, ext: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
            && p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == stem.to_lowercase())
    })
}

pub fn next_rng(r: &mut u64) -> u64 {
    *r ^= *r << 13;
    *r ^= *r >> 7;
    *r ^= *r << 17;
    *r
}

pub fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

pub fn mul3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn unit3(a: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-6);
    [a[0] / l, a[1] / l, a[2] / l]
}
