use anyhow::Result;
use rogue_reborn::collision::{CollisionWorld, Params, Player};
use rogue_reborn::map;
use std::path::Path;

/// Walk from a point in a direction and report where the player ends up:
/// `walk-try MAP x y z dx dz frames`
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let num = |i: usize| a[i].parse::<f32>().unwrap();
    let w = CollisionWorld::from_map(&map::read(Path::new(&a[1]))?, Params::default());
    let mut p = Player::new([num(2), num(3), num(4)]);
    for _ in 0..240 { w.step(&mut p, [0.0, 0.0], 1.0 / 60.0); }
    let s = p.pos;
    for _ in 0..num(7) as u32 { w.step(&mut p, [num(5) * 7.5, num(6) * 7.5], 1.0 / 60.0); }
    let d = ((p.pos[0] - s[0]).powi(2) + (p.pos[2] - s[2]).powi(2)).sqrt();
    println!("from {s:?}: moved {d:.0}, on ground {}, at {:?}", p.on_ground, p.pos);
    Ok(())
}
