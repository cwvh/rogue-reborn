use anyhow::Result;
use rogue_reborn::map;
use std::path::Path;

/// List a MAP's geometry objects: name, vertices, faces, size, and what the planning levels call rooms
fn main() -> Result<()> {
    let path = std::env::args().nth(1).unwrap_or("data/map/rm05/rm05.map".into());
    let level = map::read(Path::new(&path))?;
    for (i, o) in level.geometries.objects.iter().enumerate() {
        let faces: usize = o.object_datas.iter().map(|d| d.faces.face_indices.len()).sum();
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &o.vertices {
            for (a, c) in [v.x, v.y, v.z].into_iter().enumerate() {
                lo[a] = lo[a].min(c);
                hi[a] = hi[a].max(c);
            }
        }
        println!("#{i:2} {:28} id={} {:6} verts {:6} faces  y {:.0}..{:.0}  x {:.0}..{:.0} z {:.0}..{:.0}", o.name, o.id, o.vertices.len(), faces, lo[1], hi[1], lo[0], hi[0], lo[2], hi[2]);
    }
    println!();
    for l in &level.planning_levels.levels {
        println!("planning level {} floor {}: {:?}", l.level_number, l.floor_height, l.room_names);
    }
    Ok(())
}
