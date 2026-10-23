use anyhow::Result;
use rogue_reborn::map;
use std::path::Path;

/// Print each room's name, flags and level volumes
fn main() -> Result<()> {
    let path = std::env::args().nth(1).unwrap_or("data/map/m00/citystreet_large.map".into());
    let level = map::read(Path::new(&path))?;
    for (i, r) in level.rooms.rooms.iter().enumerate() {
        println!("#{i} id={} name={:?} {:?} unknown_flag={} weather_bounds={:?} floors={:?} levels={}",
            r.section_id, r.section_name, r.flash_space, r.unknown_flag, r.weather_bounds, r.floors, r.sherman_levels.len());
        for l in &r.sherman_levels {
            println!("    level {:?} boxes={}", l.name, l.tm_with_aabb.len());
            for b in &l.tm_with_aabb {
                let (x, y, z) = (&b.tm.x_axis, &b.tm.y_axis, &b.tm.z_axis);
                if (x.x, y.y, z.z) != (1.0, 1.0, 1.0) || x.y != 0.0 || x.z != 0.0 || y.x != 0.0 || y.z != 0.0 || z.x != 0.0 || z.y != 0.0 {
                    println!("      ROTATED {x:?} {y:?} {z:?}");
                }
                println!("      pos=({:.0},{:.0},{:.0}) aabb={:?}", b.tm.position.x, b.tm.position.y, b.tm.position.z, b.aabb.map(|v| v.round()));
            }
        }
    }
    Ok(())
}
