use anyhow::Result;
use rogue_reborn::map;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// How a MAP divides into rooms: which objects belong to which room (by the number
/// at the start of their name), which rooms the portals join, and which rooms
/// overlap in space without being joined, the signature of "impossible geometry".
fn main() -> Result<()> {
    let path = std::env::args().nth(1).unwrap_or("data/map/rm05/rm05.map".into());
    let level = map::read(Path::new(&path))?;

    let prefix = |name: &str| -> String {
        let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && name[digits.len()..].starts_with('_') { digits } else { "-".into() }
    };

    // Bounding box of each room's objects
    let mut boxes: BTreeMap<String, ([f32; 3], [f32; 3], usize)> = BTreeMap::new();
    for o in &level.geometries.objects {
        let e = boxes.entry(prefix(&o.name)).or_insert(([f32::MAX; 3], [f32::MIN; 3], 0));
        e.2 += 1;
        for v in &o.vertices {
            for (a, c) in [v.x, v.y, v.z].into_iter().enumerate() {
                e.0[a] = e.0[a].min(c);
                e.1[a] = e.1[a].max(c);
            }
        }
    }
    println!("objects={} rooms(sections)={} portals={} transitions={} planning levels={}",
        level.geometries.objects.len(), level.rooms.rooms.len(), level.portals.portals.len(),
        level.transitions.transitions.len(), level.planning_levels.levels.len());
    println!("\nobjects by name prefix (bounding box in map units):");
    for (p, (lo, hi, n)) in &boxes {
        println!("  {p:>4}: {n:4} objects  x {:.0}..{:.0}  y {:.0}..{:.0}  z {:.0}..{:.0}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]);
    }

    println!("\nportals (room -> opposite room):");
    let mut edges: BTreeSet<(u32, u32)> = BTreeSet::new();
    for p in &level.portals.portals {
        edges.insert((p.room.min(p.opposite_room), p.room.max(p.opposite_room)));
    }
    for p in level.portals.portals.iter().take(12) {
        println!("  {:?}: {} -> {}, {} corners", p.name, p.room, p.opposite_room, p.coordinates.len());
    }
    println!("  ... {} distinct room pairs joined", edges.len());

    println!("\nroom sections (name, levels):");
    for (i, r) in level.rooms.rooms.iter().enumerate().take(12) {
        println!("  #{i} {:?} levels={:?}", r.section_name, r.sherman_levels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>());
    }

    println!("\nplanning levels:");
    for l in &level.planning_levels.levels {
        println!("  level {} floor {} rooms {}", l.level_number, l.floor_height, l.room_names.len());
    }

    // Rooms whose boxes overlap a lot but that no portal joins
    println!("\nlarge overlaps between room prefixes (not necessarily impossible):");
    let keys: Vec<&String> = boxes.keys().filter(|k| k.as_str() != "-").collect();
    for (i, a) in keys.iter().enumerate() {
        for b in &keys[i + 1..] {
            let (la, ha, _) = boxes[*a];
            let (lb, hb, _) = boxes[*b];
            let ov: Vec<f32> = (0..3).map(|k| (ha[k].min(hb[k]) - la[k].max(lb[k])).max(0.0)).collect();
            let vol = ov[0] * ov[1] * ov[2];
            let small = |l: [f32; 3], h: [f32; 3]| (h[0] - l[0]) * (h[1] - l[1]) * (h[2] - l[2]);
            let frac = vol / small(la, ha).min(small(lb, hb)).max(1.0);
            if frac > 0.3 {
                let na: u32 = a.parse().unwrap_or(u32::MAX);
                let nb: u32 = b.parse().unwrap_or(u32::MAX);
                let joined = edges.contains(&(na.min(nb), na.max(nb)));
                println!("  {a:>3} x {b:>3}: {:.0}% of the smaller box{}", frac * 100.0, if joined { "  (joined by a portal)" } else { "" });
            }
        }
    }
    Ok(())
}
