use anyhow::Result;
use rogue_reborn::map;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Survey the `ind` groups' names and flags over maps: per flag value, faces and top materials
fn main() -> Result<()> {
    let mut per: BTreeMap<u32, (usize, usize, BTreeMap<String, usize>)> = BTreeMap::new();
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    let mut tags_vs_faces = (0, 0, 0);
    for path in std::env::args().skip(1) {
        let level = map::read(Path::new(&path))?;
        for o in &level.geometries.objects {
            let key = |a: u16, b: u16, c: u16| { let mut k = [a, b, c]; k.sort(); k };
            let mut mat: HashMap<[u16; 3], String> = HashMap::new();
            for d in &o.object_datas {
                let name = level.materials.materials.get(d.mn as usize).map(|m| m.filename.clone()).unwrap_or("?".into());
                for &(a, b, c) in &d.faces.face_indices { mat.insert(key(a, b, c), name.clone()); }
            }
            tags_vs_faces.0 += o.tags.len();
            tags_vs_faces.1 += mat.len();
            tags_vs_faces.2 += o.ind.iter().map(|e| e.indices.len()).sum::<usize>();
            for e in &o.ind {
                *names.entry(e.text.clone()).or_default() += 1;
                let s = per.entry(e.mn).or_default();
                s.0 += 1;
                for &i in &e.indices {
                    s.1 += 1;
                    let t = &o.tags[i as usize];
                    let m = mat.get(&key(t.coord1.0, t.coord1.1, t.coord1.2)).cloned().unwrap_or("(none)".into());
                    *s.2.entry(m).or_default() += 1;
                }
            }
        }
    }
    println!("tags {} render faces {} grouped {}", tags_vs_faces.0, tags_vs_faces.1, tags_vs_faces.2);
    println!("names: {names:?}");
    for (f, (g, n, m)) in per {
        let mut v: Vec<_> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        let top: Vec<_> = v.iter().take(8).map(|(k, n)| format!("{k}:{n}")).collect();
        println!("flags {f:#06x}: {g} groups, {n} faces; {}", top.join(" "));
    }
    Ok(())
}
