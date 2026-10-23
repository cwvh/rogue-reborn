use anyhow::Result;
use rogue_reborn::map;
use std::path::Path;

/// The faces over a point (x, z), drawn or not, with their height, material and the flags of the
/// collision groups that list them: `floor-flags MAP x z`
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let level = map::read(Path::new(&a[1]))?;
    let (x, z) = (a[2].parse::<f32>()?, a[3].parse::<f32>()?);
    let over = |p: [[f32; 3]; 3]| {
        let s = |a: [f32; 3], b: [f32; 3]| (b[0] - a[0]) * (z - a[2]) - (b[2] - a[2]) * (x - a[0]);
        let (d0, d1, d2) = (s(p[0], p[1]), s(p[1], p[2]), s(p[2], p[0]));
        let (neg, pos) = (d0 < 0.0 || d1 < 0.0 || d2 < 0.0, d0 > 0.0 || d1 > 0.0 || d2 > 0.0);
        if neg && pos { return None; }
        let (u, v) = (sub(p[1], p[0]), sub(p[2], p[0]));
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        (n[1].abs() > 1e-6).then(|| p[0][1] - (n[0] * (x - p[0][0]) + n[2] * (z - p[0][2])) / n[1])
    };
    for o in &level.geometries.objects {
        for (ti, t) in o.tags.iter().enumerate() {
            let flags: Vec<String> = o.ind.iter().filter(|g| g.indices.contains(&(ti as u16))).map(|g| format!("{:#x}", g.mn)).collect();
            let (verts, material) = if t.coord2.2 == u16::MAX {
                (&o.collisions.vertices, "(not drawn)".to_string())
            } else {
                let mn = o.object_datas[t.coord2.2 as usize].mn as usize;
                (&o.vertices, level.materials.materials.get(mn).map_or("?".into(), |m| m.filename.clone()))
            };
            let p = [t.coord1.0, t.coord1.1, t.coord1.2].map(|k| { let v = &verts[k as usize]; [v.x, v.y, v.z] });
            if let Some(y) = over(p) {
                println!("{:8} y {:8.1} {:28} flags {}", o.name, y, material, flags.join(" "));
            }
        }
    }
    Ok(())
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
