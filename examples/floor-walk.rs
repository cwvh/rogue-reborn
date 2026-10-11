use anyhow::Result;
use rogue_reborn::collision::{CollisionWorld, Params};
use rogue_reborn::{map, visibility::Visibility};
use std::collections::BTreeMap;
use std::path::Path;

/// Find places where walking from one room's floor onto the next would drop the player:
/// from each grid point the room whose floor it is is the start, as the viewer does, and the
/// floor at each neighboring point (when it is about as high) must still be solid.
///
///   floor-walk MAP x y z radius step
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let level = map::read(Path::new(&a[1]))?;
    let num = |i: usize| a[i].parse::<f32>().unwrap();
    let (cx, cy, cz, radius, step) = (num(2), num(3), num(4), num(5), num(6));

    let mut world = CollisionWorld::from_map(&level, Params::default());
    let vis = Visibility::new(&level);
    let n = (radius / step) as i32;
    let size = (2 * n + 1) as usize;

    // The floor at every grid point with every room solid: its height and room
    world.set_solid_rooms(None);
    let mut floors: Vec<Option<(f32, Option<u32>)>> = vec![None; size * size];
    for ix in -n..=n {
        for iz in -n..=n {
            let (x, z) = (cx + ix as f32 * step, cz + iz as f32 * step);
            // A floor is what the physics stands the player on, and the room it belongs to is
            // that of the first surface below when that is the same one (not a steep ramp or
            // a riser on top of it)
            let Some(ground) = world.ground_at(x, z, cy + 100.0) else { continue };
            let hit = world.raycast_hit([x, cy + 100.0, z], [0.0, -1.0, 0.0], 400.0);
            let room = hit.filter(|h| (cy + 100.0 - h.dist - ground).abs() < 10.0).and_then(|h| h.room);
            floors[(ix + n) as usize * size + (iz + n) as usize] = Some((ground, room));
        }
    }

    // Whether something solid (a wall, a closed door) stands between a point and each
    // neighbor, with every room solid: no one walks through it, so there is no step to take
    let dirs = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let mut walled = vec![[false; 4]; size * size];
    for ix in -n..=n {
        for iz in -n..=n {
            let Some((y, _)) = floors[(ix + n) as usize * size + (iz + n) as usize] else { continue };
            let (x, z) = (cx + ix as f32 * step, cz + iz as f32 * step);
            for (k, (dx, dz)) in dirs.iter().enumerate() {
                let dir = [*dx as f32, 0.0, *dz as f32];
                // At knee and waist height, so a low step does not count as a wall
                let blocked = [y + 60.0, y + 120.0].iter().any(|&h| world.raycast_hit([x, h, z], dir, step).is_some());
                walled[(ix + n) as usize * size + (iz + n) as usize][k] = blocked;
            }
        }
    }

    // (from room, onto room) -> places where the floor drops out
    let mut holes: BTreeMap<(Option<u32>, Option<u32>), Vec<(f32, f32, f32)>> = BTreeMap::new();
    let mut checked = 0;
    let mut applied: Option<Vec<rogue_reborn::collision::SolidRoom>> = None;
    for ix in -n..=n {
        for iz in -n..=n {
            let Some((y, from)) = floors[(ix + n) as usize * size + (iz + n) as usize] else { continue };
            let Some(start) = from.filter(|&r| vis.is_room(r)) else { continue };
            let (x, z) = (cx + ix as f32 * step, cz + iz as f32 * step);
            let around = vis.player_rooms(&level, [x, y + 10.0, z], None);
            let body = [[x, y + 10.0, z], [x, y + 165.0, z]]; // where the viewer says the feet and eyes are
            // The same rooms as for the point before need no rebuilding
            let want = vis.solid_rooms(&level, &[start], &around, &body);
            if applied.as_ref() != Some(&want) {
                world.set_solid_rooms(Some(&want));
                applied = Some(want);
            }
            for (k, (dx, dz)) in dirs.iter().enumerate() {
                if walled[(ix + n) as usize * size + (iz + n) as usize][k] { continue }
                let (jx, jz) = (ix + dx, iz + dz);
                if jx < -n || jx > n || jz < -n || jz > n { continue }
                let Some((y2, onto)) = floors[(jx + n) as usize * size + (jz + n) as usize] else { continue };
                // Only a continuation of the floor: about as high, within a step
                if (y2 - y).abs() > 45.0 || onto == from { continue }
                checked += 1;
                let (x2, z2) = (cx + jx as f32 * step, cz + jz as f32 * step);
                // What the player stands on there: the floor query the physics uses
                let kept = world.ground_at(x2, z2, y2 + 45.0);
                if kept.is_none_or(|g| (g - y2).abs() > 30.0) {
                    holes.entry((from, onto)).or_default().push((x2, y2, z2));
                }
            }
        }
    }
    println!("{checked} room-to-room steps checked; floor drops out at {} of them", holes.values().map(Vec::len).sum::<usize>());
    for ((from, onto), at) in &holes {
        let name = |r: &Option<u32>| r.map_or("none".to_string(), |r| r.to_string());
        println!("  from room {} onto room {}: {} steps, e.g. {:?}", name(from), name(onto), at.len(), &at[..at.len().min(3)]);
    }
    Ok(())
}
