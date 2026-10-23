use anyhow::Result;
use rogue_reborn::map;
use std::path::Path;

/// List the triangles with a vertex inside a box around a point, with the object (room) and
/// material each belongs to, so overlapping or coplanar surfaces show up:
/// `face-probe MAP x y z radius`
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let level = map::read(Path::new(&a[1]))?;
    let num = |i: usize| a[i].parse::<f32>().unwrap();
    let (c, r) = ([num(2), num(3), num(4)], num(5));
    for object in &level.geometries.objects {
        for od in &object.object_datas {
            let material = level.materials.materials.get(od.material as usize).map_or("?", |m| m.filename.as_str());
            for (i, &(i0, i1, i2)) in od.faces.face_indices.iter().enumerate() {
                let v = [i0, i1, i2].map(|k| &object.vertices[k as usize]);
                let near = v.iter().any(|p| (p.x - c[0]).abs() < r && (p.y - c[1]).abs() < r && (p.z - c[2]).abs() < r);
                if !near {
                    continue;
                }
                let n = od.faces.normals.get(i).map(|n| format!("n ({:.2},{:.2},{:.2}) d {:.1}", n.x, n.y, n.z, n.distance_origin_to_face)).unwrap_or_default();
                println!("{:8} {:24} {}  {}", object.name, material, n,
                    v.iter().map(|p| format!("({:.1},{:.1},{:.1})", p.x, p.y, p.z)).collect::<Vec<_>>().join(" "));
            }
        }
    }
    Ok(())
}
