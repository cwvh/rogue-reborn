use anyhow::Result;
use rogue_reborn::map;
use std::collections::BTreeMap;
use std::path::Path;

/// The collision group flags each material of a room's drawn faces is in, with face counts:
/// `group-of MAP ROOM [MATERIAL]`
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let level = map::read(Path::new(&a[1]))?;
    let o = level.geometries.objects.iter().find(|o| o.name == a[2]).expect("no such room");
    let mut per: BTreeMap<(String, u32), usize> = BTreeMap::new();
    for g in &o.ind {
        for &i in &g.indices {
            let t = &o.tags[i as usize];
            let m = if t.coord2.2 == u16::MAX { "(not drawn)".to_string() } else {
                level.materials.materials.get(o.object_datas[t.coord2.2 as usize].mn as usize).map_or("?".into(), |m| m.filename.clone())
            };
            if a.get(3).is_none_or(|want| *want == m) {
                *per.entry((m, g.mn)).or_default() += 1;
            }
        }
    }
    for ((m, f), n) in per { println!("{f:#06x} {n:5} {m}"); }
    Ok(())
}
