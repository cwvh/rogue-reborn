// Print the size and contents of standalone models: model-info a.qob [b.qob ...]

use std::path::Path;

use rogue_reborn::map;

fn main() -> anyhow::Result<()> {
    for path in std::env::args().skip(1) {
        let m = map::read_model(Path::new(&path))?;
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        let mut tris = 0;
        for o in &m.geometries.objects {
            for v in &o.vertices {
                for (i, c) in [v.x, v.y, v.z].into_iter().enumerate() {
                    min[i] = min[i].min(c);
                    max[i] = max[i].max(c);
                }
            }
            tris += o.object_datas.iter().map(|d| d.faces.face_indices.len()).sum::<usize>();
        }
        println!("{path}: {} objects, {tris} triangles", m.geometries.objects.len());
        for o in &m.geometries.objects {
            println!("  object {:?}: {} vertices, {} materials used", o.name, o.vertices.len(), o.object_datas.len());
        }
        for mat in &m.materials.materials {
            println!("  material {:?} ({})", mat.filename, mat.name);
        }
        println!("  bounds x {:.1}..{:.1}  y {:.1}..{:.1}  z {:.1}..{:.1}", min[0], max[0], min[1], max[1], min[2], max[2]);
    }
    Ok(())
}
