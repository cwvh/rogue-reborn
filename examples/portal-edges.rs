use anyhow::Result;
use rogue_reborn::map;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Print which rooms each room opens onto through portals, and whether every room can be reached from room `start`
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().cloned().unwrap_or("data/map/rm05/rm05.map".into());
    let start: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let level = map::read(Path::new(&path))?;
    let mut adj: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for p in &level.portals.portals {
        adj.entry(p.room).or_default().insert(p.opposite_room);
        adj.entry(p.opposite_room).or_default().insert(p.room);
    }
    for (room, to) in &adj {
        println!("{room:4} -> {to:?}");
    }
    let mut seen = BTreeSet::from([start]);
    let mut queue = vec![start];
    while let Some(r) = queue.pop() {
        for &n in adj.get(&r).into_iter().flatten() {
            if seen.insert(n) {
                queue.push(n);
            }
        }
    }
    let objects: BTreeSet<u32> = level.geometries.objects.iter().filter_map(|o| o.name.parse().ok()).collect();
    println!("\nreachable from {start}: {} rooms; rooms (objects) not reached: {:?}", seen.len(), objects.difference(&seen).collect::<Vec<_>>());
    Ok(())
}
