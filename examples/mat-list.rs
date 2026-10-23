use anyhow::Result;
use rogue_reborn::map;
use std::collections::BTreeMap;
use std::path::Path;

/// List the material file names used by one or more MAPs, with how many faces use each and
/// whether the material is two-sided
fn main() -> Result<()> {
    let mut counts: BTreeMap<String, (usize, bool)> = BTreeMap::new();
    for path in std::env::args().skip(1) {
        let level = map::read(Path::new(&path))?;
        for o in &level.geometries.objects {
            for od in &o.object_datas {
                if let Some(m) = level.materials.materials.get(od.material as usize) {
                    let e = counts.entry(m.filename.to_lowercase()).or_default();
                    e.0 += od.faces.face_indices.len();
                    e.1 |= m.two_sided;
                }
            }
        }
    }
    for (name, (n, two_sided)) in &counts {
        println!("{n:6} {name}{}", if *two_sided { "  two-sided" } else { "" });
    }
    Ok(())
}
