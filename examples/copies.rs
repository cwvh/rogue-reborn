use anyhow::Result;
use rogue_reborn::map;
use std::collections::HashMap;
use std::path::Path;

/// How many of one room's faces of a material have a copy (the same corners, to a unit) in
/// another room, and the copies' group flags: `copies MAP ROOM MATERIAL`
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let level = map::read(Path::new(&a[1]))?;
    let key = |p: [[f32; 3]; 3]| { let mut k = p.map(|v| v.map(|c| c.round() as i32)); k.sort(); k };
    let mut faces: HashMap<[[i32; 3]; 3], Vec<(String, u32)>> = HashMap::new();
    for o in &level.geometries.objects {
        for g in &o.ind {
            for &i in &g.indices {
                let t = &o.tags[i as usize];
                if t.coord2.2 == u16::MAX { continue; }
                let d = &o.object_datas[t.coord2.2 as usize];
                if level.materials.materials.get(d.mn as usize).is_none_or(|m| m.filename != a[3]) { continue; }
                let p = [t.coord1.0, t.coord1.1, t.coord1.2].map(|k| { let v = &o.vertices[k as usize]; [v.x, v.y, v.z] });
                faces.entry(key(p)).or_default().push((o.name.clone(), g.mn));
            }
        }
    }
    let mut tally: HashMap<Vec<(String, u32)>, usize> = HashMap::new();
    for (_, mut v) in faces.into_iter().filter(|(_, v)| v.iter().any(|(r, _)| *r == a[2])) {
        v.sort();
        *tally.entry(v).or_default() += 1;
    }
    for (rooms, n) in tally { println!("{n:4} faces in {rooms:?}"); }
    Ok(())
}
