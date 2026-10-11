use anyhow::Result;
use rogue_reborn::map::{self, DynamicObjectKind};
use std::path::Path;

/// List a MAP's doors (touchplates) with how they move: `door-info MAP`
fn main() -> Result<()> {
    let path = std::env::args().nth(1).unwrap_or("data/map/rm11/rm11.map".into());
    let level = map::read(Path::new(&path))?;
    for (i, d) in level.dynamic_objects.dynamic_objects.iter().enumerate() {
        let p = &d.tm.position;
        match &d.kind {
            DynamicObjectKind::RepeatableTouchplate { common, unknown1, attachments, unknown2, names, name2, name3, animation_type, direction, distance, velocity } => {
                println!("#{i} RepeatableTouchplate {:?} section {:?} at ({:.0},{:.0},{:.0})", d.name, d.section_name, p.x, p.y, p.z);
                println!("    axes x({:.2},{:.2},{:.2}) y({:.2},{:.2},{:.2}) z({:.2},{:.2},{:.2})",
                    d.tm.x_axis.x, d.tm.x_axis.y, d.tm.x_axis.z, d.tm.y_axis.x, d.tm.y_axis.y, d.tm.y_axis.z, d.tm.z_axis.x, d.tm.z_axis.y, d.tm.z_axis.z);
                println!("    common {:?} sounds {:?} 2d {:?} 3d {:?} destruction {:?}/{:?} penetration {:?} name2 {:?}",
                    common.name, common.sounds, common.collision_type_2d, common.collision_type_3d, common.destruction_action, common.destruction_category, common.penetration_type, common.name2);
                println!("    unknown1 {unknown1} unknown2 {unknown2:?} attachments {attachments:?} names {names:?} {name2:?} {name3:?}");
                println!("    animation {animation_type:?} direction ({:.2},{:.2},{:.2}) distance {distance} velocity {velocity}", direction.x, direction.y, direction.z);
            }
            DynamicObjectKind::OneTimeTouchplate { collision_type_2d, collision_type_3d, coordinates, attachments } => {
                println!("#{i} OneTimeTouchplate {:?} section {:?} at ({:.0},{:.0},{:.0}) 2d {collision_type_2d:?} 3d {collision_type_3d:?} attachments {attachments:?} coords {coordinates:?}", d.name, d.section_name, p.x, p.y, p.z);
            }
            _ => {}
        }
    }
    Ok(())
}
