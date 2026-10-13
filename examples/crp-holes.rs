use anyhow::Result;
use rogue_reborn::crp;
use std::path::Path;

/// Report a character mesh's parts and its open edges (see `Crp::open_edges`): a missing
/// triangle in a closed surface shows up as a loop of three. Rings at wrists, cuffs and hair
/// are where separate pieces meet, and are expected.
/// `crp-holes FILE.CRP`
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let c = crp::read(Path::new(&a[1]))?;
    println!("{}: {} vertices", c.name, c.vertices.len());
    for p in &c.parts {
        println!("  part {:28} {:4} triangles", p.material, p.triangles.len());
    }
    let open = c.open_edges();
    println!("{} edges used by one triangle", open.len());
    for (p, q) in open {
        println!("  ({:7.2}, {:7.2}, {:7.2}) - ({:7.2}, {:7.2}, {:7.2})", p[0], p[1], p[2], q[0], q[1], q[2]);
    }
    Ok(())
}
