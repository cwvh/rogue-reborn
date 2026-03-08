use std::path::Path;

use anyhow::Result;
use rogue_reborn::map;

fn main() -> Result<()> {
    let map = map::read(Path::new("data/map/m00/citystreet_large.map"))?;
    println!("{map:#?}");
    Ok(())
}
