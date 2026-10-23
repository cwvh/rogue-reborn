use anyhow::Result;
use rogue_reborn::collision::{CollisionWorld, Params};
use rogue_reborn::map;
use std::path::Path;

/// Cast a ray against every room and report each surface it passes through, nearest first:
/// `ray-probe MAP x y z dx dy dz max`
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let level = map::read(Path::new(&a[1]))?;
    let num = |i: usize| a[i].parse::<f32>().unwrap();
    let (o, d, max) = ([num(2), num(3), num(4)], [num(5), num(6), num(7)], num(8));
    let world = CollisionWorld::from_map(&level, Params::default());
    let mut from = o;
    let mut gone = 0.0;
    while gone < max {
        let Some(h) = world.raycast_hit(from, d, max - gone) else { break };
        gone += h.dist;
        let p = [o[0] + d[0] * gone, o[1] + d[1] * gone, o[2] + d[2] * gone];
        let material = h.material.and_then(|m| level.materials.materials.get(m)).map_or("-", |m| m.filename.as_str());
        println!("{gone:7.1}  at ({:.1}, {:.1}, {:.1})  room {:?}  {material}", p[0], p[1], p[2], h.room);
        gone += 0.05;
        from = [o[0] + d[0] * gone, o[1] + d[1] * gone, o[2] + d[2] * gone];
    }
    Ok(())
}
