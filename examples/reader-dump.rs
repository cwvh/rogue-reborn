//! reader-dump MAP...: every field the Rust reader gets from each map, in the text that
//! `experimental/map/mapdump --full` prints from the C one, to compare the two

use std::fmt::Write;
use std::path::Path;

use rogue_reborn::map::*;

fn f(o: &mut String, x: f32) {
    write!(o, " {:08x}", x.to_bits()).unwrap();
}

fn fs(o: &mut String, xs: &[f32]) {
    xs.iter().for_each(|&x| f(o, x));
}

// The file's bytes: a Latin-1 string's chars are its bytes, a UTF-8 one's bytes its bytes
fn bytes(o: &mut String, b: impl IntoIterator<Item = u8>) {
    o.push_str(" \"");
    for c in b {
        match c {
            b'"' | b'\\' => { o.push('\\'); o.push(c as char) }
            0x20..0x7f => o.push(c as char),
            _ => write!(o, "\\x{c:02x}").unwrap(),
        }
    }
    o.push('"');
}

fn s(o: &mut String, x: &str) {
    bytes(o, x.chars().map(|c| c as u32 as u8));
}

fn utf8(o: &mut String, x: &str) {
    bytes(o, x.bytes());
}

fn strs(o: &mut String, xs: &[String]) {
    write!(o, " {}", xs.len()).unwrap();
    xs.iter().for_each(|x| s(o, x));
}

fn v3(o: &mut String, v: &Vec3f) {
    fs(o, &[v.x, v.y, v.z]);
}

fn tm(o: &mut String, t: &TransformationMatrix) {
    for v in [&t.x_axis, &t.y_axis, &t.z_axis, &t.position] {
        v3(o, v);
    }
}

fn color(o: &mut String, c: &Color4f) {
    fs(o, &[c.r, c.g, c.b, c.a]);
}

fn plane(o: &mut String, p: &FaceNormal) {
    fs(o, &[p.x, p.y, p.z, p.distance_origin_to_face]);
}

fn section(o: &mut String, what: &str, id: u32, name: &str, count: usize) {
    write!(o, "{what} {id}").unwrap();
    s(o, name);
    writeln!(o, " {count}").unwrap();
}

fn common(o: &mut String, c: &DynamicObjectKindCommon) {
    o.push_str("common");
    tm(o, &c.tm);
    s(o, &c.name);
    write!(o, " {}", c.unknown1).unwrap();
    strs(o, &c.sounds);
    for x in [&c.collision_type_2d, &c.collision_type_3d, &c.destruction_action, &c.destruction_category,
              &c.penetration_type, &c.name2, &c.destruction_category2] {
        s(o, x);
    }
    o.push('\n');
}

fn dynamic_object(o: &mut String, d: &DynamicObject) {
    write!(o, "dynamic {}", d.section_id).unwrap();
    s(o, &d.section_name);
    s(o, &d.name);
    tm(o, &d.tm);
    o.push('\n');
    match &d.kind {
        DynamicObjectKind::Dynamic { common: c, params } => {
            common(o, c);
            match params {
                KindDynamicParams::Struct(ps) => for p in ps {
                    o.push_str("param");
                    s(o, &p.name);
                    fs(o, &p.unknown1);
                    writeln!(o, " {} {}", p.unknown2[0], p.unknown2[1]).unwrap();
                },
                KindDynamicParams::Flat { names, unknown } => {
                    o.push_str("flat");
                    strs(o, names);
                    fs(o, unknown);
                    o.push('\n');
                }
            }
        }
        DynamicObjectKind::Animation { common: c, unknown2, names, unknown3, unknown4, name3, name4, animation_type, direction, distance, velocity } => {
            common(o, c);
            write!(o, "animation {unknown2}").unwrap();
            strs(o, names);
            fs(o, unknown3);
            write!(o, " {unknown4}").unwrap();
            s(o, name3);
            s(o, name4);
            s(o, animation_type);
            v3(o, direction);
            fs(o, &[*distance, *velocity]);
            o.push('\n');
        }
        DynamicObjectKind::RepeatableTouchplate { common: c, unknown1, attachments, unknown2, names, name2, name3, animation_type, direction, distance, velocity } => {
            common(o, c);
            write!(o, "touchplate {unknown1}").unwrap();
            strs(o, attachments);
            fs(o, unknown2);
            strs(o, names);
            s(o, name2);
            s(o, name3);
            s(o, animation_type);
            v3(o, direction);
            fs(o, &[*distance, *velocity]);
            o.push('\n');
        }
        DynamicObjectKind::Glass { name } => {
            o.push_str("glass");
            utf8(o, name);
            o.push('\n');
        }
        DynamicObjectKind::OneTimeTouchplate { collision_type_2d, collision_type_3d, coordinates: c, attachments } => {
            o.push_str("once");
            s(o, collision_type_2d);
            s(o, collision_type_3d);
            fs(o, &[c.x1, c.y1, c.z1, c.x2, c.y2, c.z2]);
            strs(o, attachments);
            o.push('\n');
        }
        DynamicObjectKind::Halo { halos } => for (name, v) in halos {
            o.push_str("halo");
            s(o, name);
            fs(o, &[v.x1, v.y1, v.z1, v.x2, v.y2, v.z2, v.x3, v.y3]);
            o.push('\n');
        },
        DynamicObjectKind::StaticEffect => {}
    }
}

fn full(m: &Map) -> String {
    let mut o = String::new();
    let o = &mut o;
    writeln!(o, "timestamp {}", m.header.timestamp).unwrap();

    writeln!(o, "materials {} {}", m.materials.id, m.materials.materials.len()).unwrap();
    for t in &m.materials.materials {
        write!(o, "material {}", t.id).unwrap();
        s(o, &t.name);
        s(o, &t.filename);
        f(o, t.opacity);
        let mode = match t.address_mode { TextureAddressMode::Opaque => 0, TextureAddressMode::Wrap => 1, TextureAddressMode::Clamp => 3 };
        write!(o, " {} {mode}", t.emissive_strength).unwrap();
        color(o, &t.ambient);
        color(o, &t.diffuse);
        color(o, &t.specular);
        f(o, t.specular_level);
        writeln!(o, " {}", t.two_sided as u8).unwrap();
    }

    writeln!(o, "geometries {} {}", m.geometries.id, m.geometries.objects.len()).unwrap();
    for ob in &m.geometries.objects {
        write!(o, "object {}", ob.id).unwrap();
        s(o, &ob.name);
        write!(o, " {}", ob.object_id).unwrap();
        s(o, &ob.object_name);
        o.push('\n');
        for v in &ob.vertices {
            o.push('v');
            fs(o, &[v.x, v.y, v.z]);
            o.push('\n');
        }
        for d in &ob.object_datas {
            let (fc, tv) = (&d.faces, &d.texture_vertices);
            writeln!(o, "data {} {} {}", d.material, fc.normals.len(), tv.normals.len()).unwrap();
            for k in 0..fc.normals.len() {
                o.push_str("face");
                plane(o, &fc.normals[k]);
                let (a, b) = (fc.face_indices[k], fc.texture_indices[k]);
                writeln!(o, " {} {} {} {} {} {}", a.0, a.1, a.2, b.0, b.1, b.2).unwrap();
            }
            for k in 0..tv.normals.len() {
                let (n, uv) = (&tv.normals[k], &tv.uv_coords[k]);
                o.push_str("tv");
                fs(o, &[n.x, n.y, n.z, uv.u, uv.v]);
                color(o, &tv.face_colors[k]);
                o.push('\n');
            }
        }
        for v in &ob.collisions.vertices {
            o.push_str("cv");
            fs(o, &[v.x, v.y, v.z]);
            o.push('\n');
        }
        for p in &ob.collisions.faces {
            o.push_str("cp");
            plane(o, p);
            o.push('\n');
        }
        for t in &ob.tags {
            writeln!(o, "tag {} {} {} {} {} {} {} {}", t.coord1.0, t.coord1.1, t.coord1.2, t.face_index_1,
                     t.coord2.0, t.coord2.1, t.coord2.2, t.face_index_2).unwrap();
        }
        for g in &ob.collision_groups {
            o.push_str("group");
            utf8(o, &g.name);
            write!(o, " {} {}", g.flags, g.indices.len()).unwrap();
            for i in &g.indices {
                write!(o, " {i}").unwrap();
            }
            o.push('\n');
        }
    }

    section(o, "portals", m.portals.id, &m.portals.name, m.portals.portals.len());
    for p in &m.portals.portals {
        write!(o, "portal {}", p.id).unwrap();
        s(o, &p.name);
        write!(o, " {} {} {}", p.room, p.opposite_room, p.coordinates.len()).unwrap();
        for v in &p.coordinates {
            fs(o, &[v.x, v.y, v.z]);
        }
        o.push('\n');
    }

    section(o, "lights", m.lights.id, &m.lights.name, m.lights.light_count as usize);

    let d = &m.dynamic_objects;
    section(o, "dynamic_objects", d.id, &d.name, d.dynamic_objects.len());
    for x in &d.dynamic_objects {
        dynamic_object(o, x);
    }

    section(o, "rooms", m.rooms.section_id, &m.rooms.section_name, m.rooms.rooms.len());
    for r in &m.rooms.rooms {
        write!(o, "room {}", r.section_id).unwrap();
        s(o, &r.section_name);
        o.push_str(match r.flash_space { FlashSpace::Open => " 0 0", FlashSpace::Small => " 0 1", FlashSpace::Enclosed => " 1 -" });
        write!(o, " {} {}", r.unknown_flag, r.weather_bounds.is_some() as u8).unwrap();
        match &r.weather_bounds { Some([min, max]) => { v3(o, min); v3(o, max) } None => o.push_str(" -") }
        o.push('\n');
        for l in &r.sherman_levels {
            o.push_str("level");
            s(o, &l.name);
            write!(o, " 0 {}", l.floors.len()).unwrap();
            fs(o, &l.floors);
            o.push('\n');
            for b in &l.tm_with_aabb {
                o.push_str("box");
                tm(o, &b.tm);
                fs(o, &b.aabb);
                o.push('\n');
            }
        }
        for &(number, height) in &r.floors {
            o.push_str("floor");
            fs(o, &[number, height]);
            o.push('\n');
        }
        write!(o, "unknown {}", r.unknown_list.len()).unwrap();
        for x in &r.unknown_list {
            write!(o, " {x:08x}").unwrap();
        }
        o.push('\n');
    }

    let t = &m.transitions;
    section(o, "transitions", t.section_id, &t.section_name, t.transitions.len());
    for x in &t.transitions {
        o.push_str("transition");
        s(o, &x.name);
        v3(o, &x.coords.p1);
        v3(o, &x.coords.p2);
        o.push('\n');
    }

    let p = &m.planning_levels;
    section(o, "planning_levels", p.section_id, &p.section_name, p.levels.len());
    for l in &p.levels {
        o.push_str("planning");
        fs(o, &[l.level_number, l.floor_height]);
        strs(o, &l.room_names);
        o.push('\n');
    }
    std::mem::take(o)
}

fn main() {
    let mut status = 0;
    for path in std::env::args().skip(1) {
        match read(Path::new(&path)) {
            Ok(m) => print!("{}", full(&m)),
            Err(e) => {
                eprintln!("{path}: {e:#}");
                status = 1;
            }
        }
    }
    std::process::exit(status);
}
