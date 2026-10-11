use anyhow::Result;
use rogue_reborn::collision::{CollisionWorld, Params};
use rogue_reborn::{map, visibility::Visibility};
use std::collections::BTreeMap;
use std::path::Path;

/// Walk a grid around a point and find places where the floor would be missing from
/// collision (the room whose floor it is, its portal neighbors clipped at the portals, and the
/// rooms it is nested in): `floor-scan MAP x y z radius step`
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let level = map::read(Path::new(&a[1]))?;
    let num = |i: usize| a[i].parse::<f32>().unwrap();
    let (cx, cy, cz, radius, step) = (num(2), num(3), num(4), num(5), num(6));

    let mut world = CollisionWorld::from_map(&level, Params::default());
    let vis = Visibility::new(&level);
    let n = (radius / step) as i32;
    let mut checked = 0;
    let mut missing: BTreeMap<(String, String), Vec<(i32, i32)>> = BTreeMap::new();
    for ix in -n..=n {
        for iz in -n..=n {
            let (x, z) = (cx + ix as f32 * step, cz + iz as f32 * step);
            world.set_active_rooms(None);
            let Some(floor) = world.raycast_hit([x, cy + 100.0, z], [0.0, -1.0, 0.0], 400.0) else { continue };
            let ground = cy + 100.0 - floor.dist;
            let probe = [x, ground + 10.0, z];
            let around = vis.player_rooms(&level, probe, None);
            // What the viewer does: start from the room whose floor this is
            let starts: Vec<u32> = floor.room.into_iter().filter(|n| level.rooms.rooms.iter().any(|r| r.section_name == n.to_string())).collect();
            let starts = if starts.is_empty() { around.clone() } else { starts };
            if starts.is_empty() { continue }
            let name = format!("{starts:?} around {around:?}");
            checked += 1;
            world.set_solid_rooms(Some(&vis.solid_rooms(&level, &starts, &around, &[])));
            let kept = world.raycast_hit([x, ground + 100.0, z], [0.0, -1.0, 0.0], 400.0);
            if kept.is_none_or(|h| (ground + 100.0 - h.dist - ground).abs() > 30.0) {
                let floor_room = floor.room.map_or("none".to_string(), |r| r.to_string());
                missing.entry((name, floor_room)).or_default().push((ix, iz));
            }
        }
    }
    println!("{checked} points checked; floor missing at {} of them", missing.values().map(Vec::len).sum::<usize>());
    for ((located, floor_room), at) in &missing {
        println!("  located in room {located}, floor belongs to room {floor_room}: {} points, e.g. offsets {:?}", at.len(), &at[..at.len().min(4)]);
    }
    Ok(())
}
