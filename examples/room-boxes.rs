use anyhow::Result;
use rogue_reborn::map;
use std::path::Path;

/// Print each room's level boxes (transform position, axes and AABB), and which rooms
/// contain an optional point: `room-boxes MAP [x y z]`.
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().unwrap_or("data/map/rm05/rm05.map".into());
    let level = map::read(Path::new(&path))?;
    for r in &level.rooms.rooms {
        for l in &r.sherman_levels {
            for b in &l.tm_with_aabb {
                let p = &b.tm.position;
                println!("room {:>4} {:28} pos ({:.0},{:.0},{:.0}) x ({:.2},{:.2},{:.2}) y ({:.2},{:.2},{:.2}) z ({:.2},{:.2},{:.2}) aabb {:.0?}",
                    r.section_name, l.name, p.x, p.y, p.z,
                    b.tm.x_axis.x, b.tm.x_axis.y, b.tm.x_axis.z,
                    b.tm.y_axis.x, b.tm.y_axis.y, b.tm.y_axis.z,
                    b.tm.z_axis.x, b.tm.z_axis.y, b.tm.z_axis.z, b.aabb);
            }
        }
    }
    if let [_, _, x, y, z] = &args[..] {
        let pt = [x.parse::<f32>()?, y.parse::<f32>()?, z.parse::<f32>()?];
        let names: Vec<_> = level.rooms.rooms.iter().filter(|r| r.contains(pt, 50.0)).map(|r| r.section_name.as_str()).collect();
        println!("\n{pt:?} is in {names:?}");
    }
    Ok(())
}
