use anyhow::Result;
use rogue_reborn::map;
use std::collections::BTreeMap;
use std::path::Path;

/// The collision group flags each material of a room's drawn faces is in, with face counts and
/// the middle of the first face and its normal: `group-of MAP ROOM [MATERIAL]`
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let level = map::read(Path::new(&a[1]))?;
    let o = level.geometries.objects.iter().find(|o| o.name == a[2]).expect("no such room");
    let mut per: BTreeMap<(String, u32), (usize, [f32; 3], [f32; 3])> = BTreeMap::new();
    for g in &o.ind {
        for &i in &g.indices {
            let t = &o.tags[i as usize];
            let (m, verts) = if t.coord2.2 == u16::MAX { ("(not drawn)".to_string(), &o.collisions.vertices) } else {
                (level.materials.materials.get(o.object_datas[t.coord2.2 as usize].mn as usize).map_or("?".into(), |m| m.filename.clone()), &o.vertices)
            };
            if a.get(3).is_none_or(|want| *want == m) {
                let p = [t.coord1.0, t.coord1.1, t.coord1.2].map(|k| { let v = &verts[k as usize]; [v.x, v.y, v.z] });
                let middle = [0, 1, 2].map(|k| (p[0][k] + p[1][k] + p[2][k]) / 3.0);
                let (u, v) = ([0, 1, 2].map(|k| p[1][k] - p[0][k]), [0, 1, 2].map(|k| p[2][k] - p[0][k]));
                let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                per.entry((m, g.mn)).or_insert((0, middle, n.map(|c| c / l))).0 += 1;
            }
        }
    }
    for ((m, f), (n, at, normal)) in per {
        println!("{f:#06x} {n:5} {m:32} at ({:.0}, {:.0}, {:.0}) facing ({:.2}, {:.2}, {:.2})", at[0], at[1], at[2], normal[0], normal[1], normal[2]);
    }
    Ok(())
}
