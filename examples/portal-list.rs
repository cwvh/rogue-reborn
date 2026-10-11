use anyhow::Result;
use rogue_reborn::map;
use std::path::Path;

/// List every portal with its rooms, corner coordinates, centre and size, optionally only
/// those touching one room: `portal-list MAP [ROOM]`.
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().unwrap_or("data/map/rm05/rm05.map".into());
    let only: Option<u32> = args.get(2).and_then(|r| r.parse().ok());
    let level = map::read(Path::new(&path))?;
    for p in &level.portals.portals {
        if only.is_some_and(|r| p.room != r && p.opposite_room != r) {
            continue;
        }
        let n = p.coordinates.len().max(1) as f32;
        let c = [
            p.coordinates.iter().map(|v| v.x).sum::<f32>() / n,
            p.coordinates.iter().map(|v| v.y).sum::<f32>() / n,
            p.coordinates.iter().map(|v| v.z).sum::<f32>() / n,
        ];
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &p.coordinates {
            for (a, x) in [v.x, v.y, v.z].into_iter().enumerate() {
                lo[a] = lo[a].min(x);
                hi[a] = hi[a].max(x);
            }
        }
        println!("{:12} {:>4} -> {:<4} centre ({:.0}, {:.0}, {:.0}) size ({:.0}, {:.0}, {:.0})",
            p.name, p.room, p.opposite_room, c[0], c[1], c[2], hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]);
    }
    Ok(())
}
