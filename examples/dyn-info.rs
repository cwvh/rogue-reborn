use anyhow::Result;
use rogue_reborn::map::{self, DynamicObjectKind};
use std::path::Path;

/// List a MAP's dynamic objects (doors, glass, effects...) and where they are
fn main() -> Result<()> {
    let path = std::env::args().nth(1).unwrap_or("data/map/m00/citystreet_large.map".into());
    let level = map::read(Path::new(&path))?;
    for (i, d) in level.dynamic_objects.dynamic_objects.iter().enumerate() {
        let kind = match &d.kind {
            DynamicObjectKind::Glass { name } => format!("Glass {name:?}"),
            DynamicObjectKind::Dynamic { .. } => "Dynamic".into(),
            DynamicObjectKind::Animation { .. } => "Animation".into(),
            DynamicObjectKind::RepeatableTouchplate { .. } => "RepeatableTouchplate".into(),
            DynamicObjectKind::OneTimeTouchplate { .. } => "OneTimeTouchplate".into(),
            DynamicObjectKind::Halo { .. } => "Halo".into(),
            DynamicObjectKind::StaticEffect => "StaticEffect".into(),
        };
        let p = &d.tm.position;
        println!("#{i} {:?} sect={:?} at ({:.0},{:.0},{:.0}) {kind}", d.name, d.section_name, p.x, p.y, p.z);
    }
    Ok(())
}
