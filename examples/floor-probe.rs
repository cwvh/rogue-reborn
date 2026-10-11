use anyhow::Result;
use rogue_reborn::collision::{CollisionWorld, Params};
use rogue_reborn::map;
use std::path::Path;

/// Shoot rays straight down along a line and report which room owns the floor under each
/// point, with every room solid and with only a chosen set solid:
/// `floor-probe MAP x y z dx dz steps step_len room,room,...`
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let level = map::read(Path::new(&a[1]))?;
    let num = |i: usize| a[i].parse::<f32>().unwrap();
    let (x, y, z, dx, dz, steps, step) = (num(2), num(3), num(4), num(5), num(6), num(7) as usize, num(8));
    let active: Vec<u32> = a[9].split(',').filter_map(|s| s.parse().ok()).collect();

    let mut world = CollisionWorld::from_map(&level, Params::default());
    for i in 0..=steps {
        let (px, pz) = (x + dx * step * i as f32, z + dz * step * i as f32);
        world.set_active_rooms(None);
        let all = world.raycast_hit([px, y + 100.0, pz], [0.0, -1.0, 0.0], 3000.0);
        world.set_active_rooms(Some(&active));
        let some = world.raycast_hit([px, y + 100.0, pz], [0.0, -1.0, 0.0], 3000.0);
        let show = |h: Option<rogue_reborn::collision::RayHit>| h.map_or("nothing".to_string(),
            |h| format!("y {:.0} room {:?}", y + 100.0 - h.dist, h.room));
        println!("({px:.0}, {pz:.0}): all rooms -> {}   |   active rooms -> {}", show(all), show(some));
    }
    Ok(())
}
