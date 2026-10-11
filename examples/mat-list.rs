use anyhow::Result;
use rogue_reborn::map;
use std::collections::BTreeMap;
use std::path::Path;

/// List the material file names used by one or more MAPs, with how many faces use each
fn main() -> Result<()> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for path in std::env::args().skip(1) {
        let level = map::read(Path::new(&path))?;
        for o in &level.geometries.objects {
            for od in &o.object_datas {
                if let Some(m) = level.materials.materials.get(od.mn as usize) {
                    *counts.entry(m.filename.to_lowercase()).or_default() += od.faces.face_indices.len();
                }
            }
        }
    }
    for (name, n) in &counts {
        println!("{n:6} {name}");
    }
    Ok(())
}
