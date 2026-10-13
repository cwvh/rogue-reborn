//! The F2 debug dump (and the F4 clip, which is one every frame): a screenshot and a text report of where you are.

use std::path::Path;
use rogue_reborn::map;

// What F2 records. Positions are in MAP space (the camera's Z is negated), except
// the `--pos` in the repro command, which takes the camera's own coordinates.
pub struct DebugReport<'a> {
    pub map_path: &'a Path,
    pub map_num: &'a str,
    pub level: &'a map::Map,
    pub eye: [f32; 3],
    pub feet: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub dir: [f32; 3],
    pub hit: Option<(rogue_reborn::collision::RayHit, [f32; 3], &'a str)>,
    pub first_person: bool,
    pub third_person: bool,
    pub textured: bool,
    pub room_debug: bool,
    pub show_portals: bool,
    pub show_collisions: bool,
    pub fps: f32,
    pub render_ms: f32,
    pub culling: bool,
    /// The room the player is in
    pub room: Option<&'a str>,
    /// The rooms drawn this frame, when culling is on
    pub drawn: &'a [String],
    /// The rooms that can stop movement and bullets, when culling is on
    pub collision: &'a Option<Vec<rogue_reborn::collision::SolidRoom>>,
    /// Every room whose volume holds the player (rooms overlap)
    pub player_rooms: &'a [u32],
    /// The room drawing starts from (the floor's room)
    pub render_rooms: &'a [u32],
    /// Doors within reach, and how open they are
    pub doors: &'a [String],
}

// The seconds since the epoch, to name dumps and clips by
pub fn stamp() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// Write `<stem>.png` and `.txt` (by default debug/dump-<time>); returns the path without extension
pub fn write_debug_dump(r: &DebugReport, rgb: &[u8], width: u32, height: u32, stem: Option<String>) -> anyhow::Result<String> {
    use std::fmt::Write as _;
    std::fs::create_dir_all("debug")?;
    let stem = stem.unwrap_or_else(|| format!("debug/dump-{}", stamp()));
    rogue_reborn::png::write_png(Path::new(&format!("{stem}.png")), width, height, rgb)?;

    let rooms_at = |p: [f32; 3]| -> Vec<&str> {
        r.level.rooms.rooms.iter().filter(|room| room.contains(p, 50.0)).map(|room| room.section_name.as_str()).collect()
    };
    let in_rooms = rooms_at(r.feet);
    let mut t = String::new();
    writeln!(t, "map: {} ({})", r.map_path.display(), r.map_num)?;
    writeln!(t, "args: {}", std::env::args().skip(1).collect::<Vec<_>>().join(" "))?;
    writeln!(t, "mode: {}{}", if r.first_person { "first person" } else { "free fly" }, if r.third_person { ", third person view" } else { "" })?;
    writeln!(t, "toggles: textured={} room_colors={} portals={} collisions={}", r.textured, r.room_debug, r.show_portals, r.show_collisions)?;
    writeln!(t, "frame: {:.1} fps, {:.2} ms render", r.fps, r.render_ms)?;
    writeln!(t)?;
    writeln!(t, "eye (map space): {:.1}, {:.1}, {:.1}", r.eye[0], r.eye[1], r.eye[2])?;
    writeln!(t, "feet (map space): {:.1}, {:.1}, {:.1}", r.feet[0], r.feet[1], r.feet[2])?;
    writeln!(t, "yaw {:.1}, pitch {:.1}; looking along ({:.3}, {:.3}, {:.3})", r.yaw, r.pitch, r.dir[0], r.dir[1], r.dir[2])?;
    writeln!(t, "rooms whose volumes contain the feet: {in_rooms:?}")?;
    writeln!(t, "room the player is in: {}", r.room.unwrap_or("none"))?;
    writeln!(t, "rooms whose volumes hold the player (collide): {:?}", r.player_rooms)?;
    writeln!(t, "room drawing starts from (floor under the feet): {:?}", r.render_rooms)?;
    if r.culling {
        writeln!(t, "rooms drawn (visible through portals): {}", r.drawn.join(" "))?;
        let collide: Vec<String> = r.collision.as_deref().unwrap_or_default().iter()
            .map(|c| {
                let part = if c.keep.is_empty() && c.cut.is_empty() { "" } else { " (part)" };
                let holes = if c.holes.is_empty() { "" } else { " (holed)" };
                format!("{}{part}{holes}", c.room)
            })
            .collect();
        writeln!(t, "rooms that collide (\"part\" is cut at a portal, \"holed\" where the player's room reaches through it): {}", collide.join(" "))?;
    } else {
        writeln!(t, "room culling is off: every room is drawn")?;
    }

    // Portals touching those rooms
    let numbers: Vec<u32> = r.room.and_then(|n| n.parse().ok()).into_iter().collect();
    let portals: Vec<String> = r.level.portals.portals.iter()
        .filter(|p| numbers.contains(&p.room) || numbers.contains(&p.opposite_room))
        .map(|p| format!("{} ({} -> {})", p.name, p.room, p.opposite_room))
        .collect();
    writeln!(t, "portals of that room: {}", if portals.is_empty() { "none".to_string() } else { portals.join(", ") })?;

    writeln!(t, "doors within 500 units: {}", if r.doors.is_empty() { "none".to_string() } else { format!("\n  {}", r.doors.join("\n  ")) })?;

    match &r.hit {
        Some((hit, point, material)) => {
            writeln!(t, "\nlooking at: {:.0} units away, point ({:.1}, {:.1}, {:.1}), normal ({:.2}, {:.2}, {:.2})",
                hit.dist, point[0], point[1], point[2], hit.normal[0], hit.normal[1], hit.normal[2])?;
            writeln!(t, "  material: {material}")?;
            writeln!(t, "  room whose geometry this is: {}", hit.room.map_or("none".to_string(), |n| n.to_string()))?;
            writeln!(t, "  rooms at that point: {:?}", rooms_at(*point))?;
        }
        None => writeln!(t, "\nlooking at: nothing within range")?,
    }

    // `--pos` takes the camera's own coordinates, with Z negated
    let mut repro = format!("cargo rr --map {} --pos {:.1},{:.1},{:.1} --yaw {:.1} --pitch {:.1}",
        r.map_path.display(), r.eye[0], r.eye[1], -r.eye[2], r.yaw, r.pitch);
    for (on, flag) in [(!r.culling, "--no-cull"), (r.first_person, "--first-person"), (r.textured, "--textured"), (r.room_debug, "--rooms"), (r.show_portals, "--portals"), (r.show_collisions, "--collisions")] {
        if on {
            repro.push(' ');
            repro.push_str(flag);
        }
    }
    writeln!(t, "\nreproduce: {repro}")?;
    writeln!(t, "git commit: {}", option_env!("GIT_HASH").unwrap_or("unknown"))?;
    std::fs::write(format!("{stem}.txt"), t)?;
    Ok(stem)
}
