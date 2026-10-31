use anyhow::Result;
use rogue_reborn::collision::{CollisionWorld, Params};
use rogue_reborn::{levels, map};
use std::path::Path;

/// Every floor the player can stand on, on a grid over the level, and the floor under each
/// mission spawn point, to compare two builds' floors: `floor-levels MAP [step]`. Prints
///   spawn X Y Z GROUND
///   at X Z H...      (the heights of the floors over each other there, highest first)
fn main() -> Result<()> {
    let path = std::env::args().nth(1).unwrap_or("data/map/rm11/rm11.map".into());
    let step: f32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(100.0);
    let level = map::read(Path::new(&path))?;
    let world = CollisionWorld::from_map(&level, Params::default());
    for p in levels::find_mission_spawns(Path::new(&path)) {
        let g = world.ground_at(p[0], p[2], p[1] + 100.0);
        println!("spawn {:.1} {:.1} {:.1} {}", p[0], p[1], p[2], g.map_or("none".into(), |g| format!("{g:.1}")));
    }
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for o in level.geometries.objects.iter().filter(|o| map::object_room(&o.name).is_some()) {
        for v in &o.vertices {
            for (a, c) in [v.x, v.y, v.z].into_iter().enumerate() {
                lo[a] = lo[a].min(c);
                hi[a] = hi[a].max(c);
            }
        }
    }
    let mut z = (lo[2] / step).ceil() * step;
    while z <= hi[2] {
        let mut x = (lo[0] / step).ceil() * step;
        while x <= hi[0] {
            let mut line = String::new();
            let mut top = hi[1] + 1.0;
            while let Some(g) = world.ground_at(x, z, top) {
                line += &format!(" {g:.1}");
                top = g - 1.0;
            }
            if !line.is_empty() {
                println!("at {x:.0} {z:.0}{line}");
            }
            x += step;
        }
        z += step;
    }
    Ok(())
}
