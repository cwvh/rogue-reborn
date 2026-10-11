//! Finding levels and their spawn points in the game data.

use std::path::{Path, PathBuf};
use rogue_reborn::collision::CollisionWorld;

// Spawn at the centre of a named zone from the multiplayer mission file
// (data/mission/<MAP>_MUL.mis). Zones appear there as two MAP-space points
// followed by the quoted zone name, e.g. "ZONE_RM11_COURTYARD". Returns the
// feet position in MAP space, standing on the floor under the zone's centre.
pub fn find_zone_spawn(map_path: &Path, zone: &str, world: &CollisionWorld) -> Option<[f32; 3]> {
    let stem = map_path.file_stem()?.to_string_lossy().to_uppercase();
    let mission = Path::new("data/mission").join(format!("{stem}_MUL.mis"));
    let text = std::fs::read_to_string(mission).ok()?;
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let name = format!("\"ZONE_{stem}_{zone}\"");
    let at = lines.iter().position(|l| l.starts_with(&name))?;
    if at < 2 {
        return None;
    }
    let point = |l: &str| -> Option<[f32; 3]> {
        let v: Vec<f32> = l.split_whitespace().map_while(|n| n.parse().ok()).collect();
        (v.len() >= 3).then(|| [v[0], v[1], v[2]])
    };
    let (a, b) = (point(lines[at - 2])?, point(lines[at - 1])?);
    let (x, y, z) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0);
    let floor = world.ground_at(x, z, y + 100.0).unwrap_or(y);
    Some([x, floor, z])
}

// Collect the adversarial spawn points (MAP space) from every mission file in
// data/mission that names this map. The "Number of spawn points" line is
// followed by that many "x y z" lines. Variants of a mission (_MUL, _MULT)
// repeat the same points, so duplicates are dropped.
pub fn find_mission_spawns(map_path: &Path) -> Vec<[f32; 3]> {
    let mut points: Vec<[f32; 3]> = Vec::new();
    let Some(map_name) = map_path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return points;
    };
    let Ok(dir) = std::fs::read_dir("data/mission") else { return points };
    let mut files: Vec<PathBuf> = dir.flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mis")))
        .collect();
    files.sort();

    for file in files {
        let Ok(bytes) = std::fs::read(&file) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().map(str::trim).collect();

        // The first quoted string in the file is the map filename
        let named = lines.iter().find_map(|l| {
            let rest = l.strip_prefix('"')?;
            Some(rest[..rest.find('"')?].to_string())
        });
        if !named.is_some_and(|n| n.eq_ignore_ascii_case(&map_name)) {
            continue;
        }

        let Some(at) = lines.iter().position(|l| l.contains("Number of spawn points")) else { continue };
        let count: usize = lines[at].split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
        for l in lines.iter().skip(at + 1).take(count) {
            let v: Vec<f32> = l.split_whitespace().map_while(|n| n.parse().ok()).collect();
            if v.len() >= 3 {
                let p = [v[0], v[1], v[2]];
                if !points.contains(&p) {
                    points.push(p);
                }
            }
        }
    }
    points
}

// Pick an index in 0..n; random unless a seed is given (splitmix64)
pub fn pick_index(n: usize, seed: Option<u64>) -> usize {
    let seed = seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64)
    });
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z % n as u64) as usize
}

// Extract the map number (e.g., "m00") from a map path like data/map/m00/citystreet_large.map
// Every level under data/map as (directory, name, path), sorted by directory then name.
// The name is the file name without .map, whatever its case.
pub fn list_levels() -> Vec<(String, String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir("data/map") else { return out };
    for dir in dirs.flatten().filter(|d| d.path().is_dir()) {
        let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
        for f in files.flatten() {
            let path = f.path();
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("map")) {
                let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                out.push((dir.file_name().to_string_lossy().into_owned(), name, path));
            }
        }
    }
    out.sort_by(|a, b| (a.0.to_lowercase(), a.1.to_lowercase()).cmp(&(b.0.to_lowercase(), b.1.to_lowercase())));
    out
}

// Find a level by name, ignoring case; "rm11" and "rm11/rm11" both work. The error
// says what was available.
pub fn find_level(name: &str) -> Result<PathBuf, String> {
    let levels = list_levels();
    let want = name.trim().trim_end_matches(".map").to_lowercase().replace('\\', "/");
    let found: Vec<&(String, String, PathBuf)> = levels.iter()
        .filter(|(dir, n, _)| want == n.to_lowercase() || want == format!("{}/{}", dir.to_lowercase(), n.to_lowercase()))
        .collect();
    match found.as_slice() {
        [one] => Ok(one.2.clone()),
        [] => {
            let names: Vec<&str> = levels.iter().map(|l| l.1.as_str()).collect();
            Err(format!("no level named {name:?}; available: {}", names.join(", ")))
        }
        many => {
            let paths: Vec<String> = many.iter().map(|l| l.2.display().to_string()).collect();
            Err(format!("{name:?} matches several levels, use --map: {}", paths.join(", ")))
        }
    }
}

pub fn extract_map_num(path: &Path) -> Option<String> {
    let mut comps = path.components().map(|c| c.as_os_str());
    while let Some(c) = comps.next() {
        if c == "map" {
            if let Some(next) = comps.next() {
                return Some(next.to_string_lossy().into_owned());
            }
        }
    }
    None
}
