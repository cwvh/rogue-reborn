use anyhow::Result;
use rogue_reborn::{cxp, map};
use std::path::Path;

/// The .bmp materials of each map with no texture properties of their own whose .tga name has
/// some, and whether a plain "<name>.rsb" exists: `tga-properties MAP...`
fn main() -> Result<()> {
    let properties = cxp::TextureProperties::load();
    for arg in std::env::args().skip(1) {
        let level = map::read(Path::new(&arg))?;
        let dir = Path::new(&arg).parent().unwrap().to_path_buf();
        let names: Vec<String> = [dir, Path::new("data/texture").to_path_buf()].iter()
            .filter_map(|d| std::fs::read_dir(d).ok()).flat_map(|e| e.flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect::<Vec<_>>()).collect();
        for m in &level.materials.materials {
            let Some(stem) = m.filename.to_lowercase().strip_suffix(".bmp").map(String::from) else { continue };
            if properties.get(&m.filename).is_some() { continue }
            let Some(p) = properties.get(&format!("{stem}.tga")) else { continue };
            let plain = names.contains(&format!("{stem}.rsb"));
            println!("{arg}: {} plain={plain} gun={} grenade={}", m.filename, p.gun_pass, p.grenade_pass);
        }
    }
    Ok(())
}
