use anyhow::Result;
use rogue_reborn::map;
use std::path::Path;

/// Every drawn triangle a ray passes through, nearest first, with its object (room), material
/// and which way it faces the ray: `pick MAP x y z dx dy dz` (map space, as an F2 dump gives)
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let level = map::read(Path::new(&a[1]))?;
    let num = |i: usize| a[i].parse::<f32>().unwrap();
    let (o, d) = ([num(2), num(3), num(4)], [num(5), num(6), num(7)]);
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let mut hits = Vec::new();
    for object in &level.geometries.objects {
        for (k, od) in object.object_datas.iter().enumerate() {
            let material = level.materials.materials.get(od.material as usize).map_or("?", |m| m.filename.as_str());
            for (f, &(i0, i1, i2)) in od.faces.face_indices.iter().enumerate() {
                let p = [i0, i1, i2].map(|k| object.vertices.get(k as usize).map(|v| [v.x, v.y, v.z]));
                let [Some(p0), Some(p1), Some(p2)] = p else { continue };
                let (e1, e2) = (sub(p1, p0), sub(p2, p0));
                let h = cross(d, e2);
                let det = dot(e1, h);
                if det.abs() < 1e-9 {
                    continue;
                }
                let s = sub(o, p0);
                let u = dot(s, h) / det;
                let q = cross(s, e1);
                let v = dot(d, q) / det;
                let t = dot(e2, q) / det;
                if u < 0.0 || v < 0.0 || u + v > 1.0 || t <= 0.0 {
                    continue;
                }
                // Counter-clockwise as seen from the eye is the front, as the game draws them
                let facing = if dot(cross(e1, e2), d) < 0.0 { "front" } else { "back" };
                hits.push((t, format!("{} [{k}.{f} of {}/{}]", object.name, od.faces.face_indices.len(), od.faces.texture_indices.len()), material.to_string(), facing));
            }
        }
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (t, name, material, facing) in hits.iter().take(12) {
        println!("{t:8.1}  {name:32} {material:32} {facing}");
    }
    Ok(())
}
