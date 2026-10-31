// Which way the missions' headings point: mission-headings
//
// A team starts at a place and a heading, 0 to 255. Guards mostly face into the room, not the
// wall, so for each way the heading could turn into a yaw (which way 0 points, and which way it
// turns), this casts a ray ahead of every terrorist in every mission and says how far it got.

use std::path::Path;

use rogue_reborn::collision::{CollisionWorld, Params};
use rogue_reborn::{map, mission};

fn main() -> anyhow::Result<()> {
    let mut far: Vec<Vec<f32>> = vec![Vec::new(); 8];
    for level in 1..=19 {
        let map_path = format!("data/map/rm{level:02}/rm{level:02}.map");
        let Some(mdf) = mission::find(Path::new(&map_path), mission::Difficulty::Medium) else { continue };
        let level_map = map::read(Path::new(&map_path))?;
        let world = CollisionWorld::from_map(&level_map, Params::default());
        for team in mission::read(&mdf)?.iter().filter(|t| t.terrorists) {
            for &(p, heading) in &team.starts {
                let floor = world.ground_at(p[0], p[2], p[1] + 100.0).unwrap_or(p[1]);
                let eye = [p[0], floor + 120.0, p[2]];
                for (k, list) in far.iter_mut().enumerate() {
                    let (sign, offset) = (if k < 4 { 1.0 } else { -1.0 }, (k % 4) as f32 * 90.0);
                    let yaw = (sign * heading * 360.0 / 256.0 + offset).to_radians();
                    let dir = [yaw.sin(), 0.0, yaw.cos()];
                    list.push(world.raycast(eye, dir, 3000.0).unwrap_or(3000.0));
                }
            }
        }
    }
    for (k, list) in far.iter_mut().enumerate() {
        list.sort_by(f32::total_cmp);
        let (sign, offset) = (if k < 4 { "+" } else { "-" }, (k % 4) * 90);
        let close = list.iter().filter(|&&d| d < 100.0).count();
        println!("yaw = {sign}heading + {offset:3}: median {:6.0} ahead, {:3} of {} facing a wall within 100", list[list.len() / 2], close, list.len());
    }
    Ok(())
}
