//! Finding levels and their spawn points in the game data.

use std::path::{Path, PathBuf};
use crate::collision::CollisionWorld;

// Spawn at the center of a named zone from the multiplayer mission file
// (data/mission/<MAP>_MUL.mis). Zones appear there as two MAP-space points
// followed by the quoted zone name, e.g. "ZONE_RM11_COURTYARD". Returns the
// feet position in MAP space, standing on the floor under the zone's center.
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
    for text in missions_of(map_path) {
        let lines: Vec<&str> = text.lines().map(str::trim).collect();
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

// The text of every mission file in data/mission that names this map, in file name order
fn missions_of(map_path: &Path) -> Vec<String> {
    let Some(map_name) = map_path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let Ok(dir) = std::fs::read_dir("data/mission") else { return Vec::new() };
    let mut files: Vec<PathBuf> = dir.flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mis")))
        .collect();
    files.sort();
    files.into_iter().filter_map(|file| {
        let text = String::from_utf8_lossy(&std::fs::read(&file).ok()?).into_owned();
        // The first quoted string in the file is the map filename
        let named = text.lines().map(str::trim).find_map(|l| {
            let rest = l.strip_prefix('"')?;
            Some(rest[..rest.find('"')?].to_string())
        });
        named.is_some_and(|n| n.eq_ignore_ascii_case(&map_name)).then_some(text)
    }).collect()
}

/// A side's base in a multiplayer mission: a box between two corners (map space), and its name
/// in the game's text (`ZONE_KDBL_TOP` is "Top of Building")
#[derive(Clone, Debug, PartialEq)]
pub struct Base {
    pub name: String,
    pub corners: [[f32; 3]; 2],
}

impl Base {
    /// Where a side's players start, in turn (0x56c7a0): the two corners given, which are
    /// opposite, then the other two, then the middles of the four edges. All but the first two
    /// are at the height of the higher corner. Feet go on the floor below.
    pub fn starts(&self) -> [[f32; 3]; 8] {
        let [a, b] = self.corners;
        let y = a[1].max(b[1]);
        let (mx, mz) = ((a[0] + b[0]) / 2.0, (a[2] + b[2]) / 2.0);
        [a, b, [a[0], y, b[2]], [b[0], y, a[2]], [mx, y, a[2]], [mx, y, b[2]], [a[0], y, mz], [b[0], y, mz]]
    }
}

/// The two bases a match on this level can start from, Blue's and Gold's, and the pair's name
#[derive(Clone, Debug, PartialEq)]
pub struct BasePair {
    pub name: String,
    pub blue: Base,
    pub gold: Base,
}

// The base pairs of the level's multiplayer mission (`_MUL.mis`), as the original reads them
// (0x569310): after "Number of base pairs", each pair is its quoted name, then for each side two
// corners and the base's quoted name. The game picks one at random, or the host names one.
pub fn find_bases(map_path: &Path) -> Vec<BasePair> {
    for text in missions_of(map_path) {
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let Some(at) = lines.iter().position(|l| l.contains("Number of base pairs")) else { continue };
        let count: usize = lines[at].split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
        let quoted = |l: &str| -> Option<String> {
            let rest = l.strip_prefix('"')?;
            Some(rest[..rest.find('"')?].to_string())
        };
        let point = |l: &str| -> Option<[f32; 3]> {
            let v: Vec<f32> = l.split_whitespace().map_while(|n| n.parse().ok()).collect();
            (v.len() >= 3).then(|| [v[0], v[1], v[2]])
        };
        let base = |l: &[&str]| -> Option<Base> { Some(Base { corners: [point(l[0])?, point(l[1])?], name: quoted(l[2])? }) };
        let pairs: Option<Vec<BasePair>> = (0..count).map(|i| {
            let l = lines.get(at + 1 + i * 7..at + 8 + i * 7)?;
            Some(BasePair { name: quoted(l[0])?, blue: base(&l[1..4])?, gold: base(&l[4..7])? })
        }).collect();
        if let Some(pairs) = pairs.filter(|p| !p.is_empty()) {
            return pairs;
        }
    }
    Vec::new()
}

/// The game's text for one of its string IDs (`ZONE_CSL_GROUP_1_DESC` is "Upper Floors"), from
/// its English string table
pub fn game_text(id: &str) -> Option<String> {
    let bytes = std::fs::read("data/text/interface/english/r6_string_table.txt").ok()?;
    // Latin-1, as the game's fonts are
    let text: String = bytes.iter().map(|&b| b as char).collect();
    text.lines().find_map(|line| {
        let (key, rest) = line.trim().split_once(char::is_whitespace)?;
        let rest = rest.trim().strip_prefix('"')?;
        (key == id).then(|| rest[..rest.find('"').unwrap_or(rest.len())].to_string())
    })
}

/// A level's mission name as the original shows it ("City Street Large"), from the multiplayer
/// mission file (`data/mission/*_MUL.mis`) whose map is `map`, the level's file
pub fn mission_name(map: &str) -> Option<String> {
    let file = std::path::Path::new(map).file_name()?.to_string_lossy().to_lowercase();
    std::fs::read_dir("data/mission").ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        let stem = path.file_name()?.to_string_lossy().to_lowercase();
        if !stem.ends_with("_mul.mis") {
            return None;
        }
        // Latin-1, as the string table is. The first quoted values are the map, its folder and
        // the mission's name in the string table.
        let text: String = std::fs::read(&path).ok()?.iter().map(|&b| b as char).collect();
        let quoted: Vec<&str> = text.split('"').skip(1).step_by(2).take(3).collect();
        let [mis_map, _, name] = quoted.as_slice() else { return None };
        (mis_map.to_lowercase() == file).then(|| game_text(name)).flatten()
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_double_killhouse_has_four_pairs_of_bases() {
        let pairs = find_bases(Path::new("data/map/m00/killhouse_2story_dbl.map"));
        assert_eq!(pairs.len(), 4);
        let p = &pairs[0];
        assert_eq!(p.name, "ZONE_KDBL_GROUP_4_DESC");
        assert_eq!(p.blue, Base { name: "ZONE_KDBL_TOP".to_string(), corners: [[32613.9, 33447.0, 30400.8], [32950.6, 33457.0, 30088.4]] });
        assert_eq!(p.gold.name, "ZONE_KDBL_BACK");
        assert_eq!(pairs[3].gold.corners[1], [33109.8, 33457.0, 32342.5]);
    }

    #[test]
    fn a_pair_of_bases_is_named_from_the_games_text() {
        if !Path::new("data/text").exists() {
            return;
        }
        assert_eq!(game_text("ZONE_CSL_GROUP_1_DESC").as_deref(), Some("Upper Floors"));
        assert_eq!(game_text("ZONE_NO_SUCH_THING"), None);
    }

    #[test]
    fn a_level_is_named_as_its_multiplayer_mission_names_it() {
        if !Path::new("data/mission").exists() {
            return;
        }
        assert_eq!(mission_name("data/map\\m00\\citystreet_large.map").as_deref(), Some("City Street Large"));
        assert_eq!(mission_name("no_such_level.map"), None);
    }

    #[test]
    fn a_sides_first_two_start_at_opposite_corners_and_the_rest_around_the_edges() {
        let base = Base { name: String::new(), corners: [[0.0, 10.0, 0.0], [100.0, 20.0, 40.0]] };
        let s = base.starts();
        assert_eq!(&s[..2], &base.corners);
        assert_eq!(&s[2..], &[[0.0, 20.0, 40.0], [100.0, 20.0, 0.0], [50.0, 20.0, 0.0], [50.0, 20.0, 40.0], [0.0, 20.0, 20.0], [100.0, 20.0, 20.0]]);
    }

    #[test]
    fn every_multiplayer_level_has_bases() {
        let mut levels = 0;
        for (_, _, path) in list_levels() {
            if find_mission_spawns(&path).is_empty() {
                continue;
            }
            let pairs = find_bases(&path);
            assert!((1..=20).contains(&pairs.len()), "{} has {} pairs", path.display(), pairs.len());
            levels += 1;
        }
        assert!(levels >= 20, "only {levels} levels with spawn points");
    }
}
