use anyhow::Result;
use rogue_reborn::map;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// The walls only the drawn level has: steep faces in collision groups flagged no walls (2),
/// leaving out foliage (.tga), with no wall of the game's within 40 units of their middle. By
/// room and material, with the area and a point on each: `lost-walls MAP...`
fn main() -> Result<()> {
    for path in std::env::args().skip(1) {
        let level = map::read(Path::new(&path))?;
        let mut faces: Vec<([[f32; 3]; 3], bool, String, String)> = Vec::new();
        for o in &level.geometries.objects {
            let mut wall: HashMap<usize, bool> = HashMap::new();
            for g in &o.collision_groups {
                for &i in &g.indices {
                    *wall.entry(i as usize).or_insert(false) |= g.flags & 2 == 0;
                }
            }
            for (ti, t) in o.tags.iter().enumerate() {
                let (verts, material) = if t.coord2.2 == u16::MAX {
                    (&o.collisions.vertices, "(not drawn)".to_string())
                } else {
                    let material = o.object_datas[t.coord2.2 as usize].material as usize;
                    (&o.vertices, level.materials.materials.get(material).map_or("?".into(), |m| m.filename.clone()))
                };
                let p = [t.coord1.0, t.coord1.1, t.coord1.2].map(|k| { let v = &verts[k as usize]; [v.x, v.y, v.z] });
                faces.push((p, wall.get(&ti).copied().unwrap_or(true), o.name.clone(), material));
            }
        }
        let steep = |p: &[[f32; 3]; 3]| {
            let (u, v) = (sub(p[1], p[0]), sub(p[2], p[0]));
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            (l > 1e-6 && (n[1] / l).abs() < 0.643).then_some(l * 0.5)
        };
        let cell = 200.0;
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, f) in faces.iter().enumerate().filter(|(_, f)| f.1 && steep(&f.0).is_some()) {
            let m = middle(&f.0);
            grid.entry(((m[0] / cell).floor() as i32, (m[2] / cell).floor() as i32)).or_default().push(i);
        }
        let mut lost: BTreeMap<(String, String), (f32, [f32; 3])> = BTreeMap::new();
        for f in faces.iter().filter(|f| !f.1 && !f.3.to_lowercase().ends_with(".tga")) {
            let Some(area) = steep(&f.0) else { continue };
            let m = middle(&f.0);
            let (cx, cz) = ((m[0] / cell).floor() as i32, (m[2] / cell).floor() as i32);
            let held = (-1..=1).any(|dx| (-1..=1).any(|dz| grid.get(&(cx + dx, cz + dz)).is_some_and(|c| c.iter().any(|&i| {
                closest(m, &faces[i].0) <= 40.0
            }))));
            if !held {
                let e = lost.entry((f.2.clone(), f.3.clone())).or_insert((0.0, m));
                e.0 += area;
            }
        }
        println!("{path}");
        let mut v: Vec<_> = lost.into_iter().collect();
        v.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
        for ((room, material), (area, at)) in v.iter().take(8) {
            println!("  room {room:6} {material:30} {:9.0} sq ({:.0}, {:.0}, {:.0})", area, at[0], at[1], at[2]);
        }
    }
    Ok(())
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn middle(p: &[[f32; 3]; 3]) -> [f32; 3] { [0, 1, 2].map(|k| (p[0][k] + p[1][k] + p[2][k]) / 3.0) }

/// Distance from a point to the nearest of a triangle's corners and middle (rough)
fn closest(q: [f32; 3], p: &[[f32; 3]; 3]) -> f32 {
    let d = |a: [f32; 3]| { let s = sub(a, q); (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt() };
    // The plane distance where the point is over the face, else the nearest corner
    let (u, v) = (sub(p[1], p[0]), sub(p[2], p[0]));
    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
    let w = sub(q, p[0]);
    let plane = ((w[0] * n[0] + w[1] * n[1] + w[2] * n[2]) / l).abs();
    let corners = d(p[0]).min(d(p[1])).min(d(p[2])).min(d(middle(p)));
    let span = d(p[0]).max(d(p[1])).max(d(p[2]));
    if corners <= span { plane.max(0.0).min(corners) } else { corners }
}
