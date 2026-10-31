//! maptool --full MAP...   every field of each map, as `mapdump --full` prints it
//! maptool --bench MAP...  times reading each map, as `mapbench` does, printing lines
//!   rsarena STEP MAP BYTES RUNS MIN_NS MEDIAN_NS
//! where STEP is "read" (map::read from the path), "parse" (map::parse of the file's bytes
//! already in memory) or "file" (only reading the file).
//!
//! Built on its own with rustc, using the reader in map.rs, or with `--cfg map2` the one in
//! map2.rs, whose lines start "rsarena2".

#[allow(dead_code)]
#[cfg_attr(map2, path = "map2.rs")]
mod map;

const LABEL: &str = if cfg!(map2) { "rsarena2" } else { "rsarena" };

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

use map::*;

const ARENA_SIZE: usize = 32 << 20;

// --- Every field ---------------------------------------------------------------------------

fn f(o: &mut String, x: f32) {
    write!(o, " {:08x}", x.to_bits()).unwrap();
}

fn fs(o: &mut String, xs: &[f32]) {
    xs.iter().for_each(|&x| f(o, x));
}

fn s(o: &mut String, b: &[u8]) {
    o.push_str(" \"");
    for &c in b {
        match c {
            b'"' | b'\\' => {
                o.push('\\');
                o.push(c as char);
            }
            0x20..0x7f => o.push(c as char),
            _ => write!(o, "\\x{c:02x}").unwrap(),
        }
    }
    o.push('"');
}

fn strs(o: &mut String, xs: &[&[u8]]) {
    write!(o, " {}", xs.len()).unwrap();
    xs.iter().for_each(|x| s(o, x));
}

fn v3(o: &mut String, v: Vec3) {
    fs(o, &[v.x, v.y, v.z]);
}

fn tm(o: &mut String, t: Transform) {
    [t.x_axis, t.y_axis, t.z_axis, t.position].into_iter().for_each(|v| v3(o, v));
}

fn color(o: &mut String, c: Color) {
    fs(o, &[c.r, c.g, c.b, c.a]);
}

fn plane(o: &mut String, p: Plane) {
    fs(o, &[p.x, p.y, p.z, p.distance]);
}

fn section(o: &mut String, what: &str, sec: Section, count: usize) {
    write!(o, "{what} {}", sec.id).unwrap();
    s(o, sec.name);
    writeln!(o, " {count}").unwrap();
}

fn common(o: &mut String, c: &DynamicCommon) {
    o.push_str("common");
    tm(o, c.transform);
    s(o, c.name);
    write!(o, " {}", c.unknown1).unwrap();
    strs(o, &c.sounds);
    for x in [c.collision_type_2d, c.collision_type_3d, c.destruction_action, c.destruction_category,
              c.penetration_type, c.name2, c.destruction_category2] {
        s(o, x);
    }
    o.push('\n');
}

fn dynamic_object(o: &mut String, d: &DynamicObject) {
    write!(o, "dynamic {}", d.section.id).unwrap();
    s(o, d.section.name);
    s(o, d.name);
    tm(o, d.transform);
    o.push('\n');
    match &d.kind {
        DynamicKind::Dynamic { common: c, params } => {
            common(o, c);
            match params {
                DynamicParams::Struct(ps) => for p in *ps {
                    o.push_str("param");
                    s(o, p.name);
                    fs(o, &p.unknown1);
                    writeln!(o, " {} {}", p.unknown2[0], p.unknown2[1]).unwrap();
                },
                DynamicParams::Flat { names, unknown } => {
                    o.push_str("flat");
                    strs(o, names);
                    fs(o, unknown);
                    o.push('\n');
                }
            }
        }
        DynamicKind::Animation { common: c, unknown2, names, unknown3, unknown4, name3, name4, animation_type, direction, distance, velocity } => {
            common(o, c);
            write!(o, "animation {unknown2}").unwrap();
            strs(o, names);
            fs(o, unknown3);
            write!(o, " {unknown4}").unwrap();
            s(o, name3);
            s(o, name4);
            s(o, animation_type);
            v3(o, *direction);
            fs(o, &[*distance, *velocity]);
            o.push('\n');
        }
        DynamicKind::RepeatableTouchplate { common: c, unknown1, attachments, unknown2, names, name2, name3, animation_type, direction, distance, velocity } => {
            common(o, c);
            write!(o, "touchplate {unknown1}").unwrap();
            strs(o, attachments);
            fs(o, unknown2);
            strs(o, names);
            s(o, name2);
            s(o, name3);
            s(o, animation_type);
            v3(o, *direction);
            fs(o, &[*distance, *velocity]);
            o.push('\n');
        }
        DynamicKind::Glass { name } => {
            o.push_str("glass");
            s(o, name);
            o.push('\n');
        }
        DynamicKind::OneTimeTouchplate { collision_type_2d, collision_type_3d, coordinates, attachments } => {
            o.push_str("once");
            s(o, collision_type_2d);
            s(o, collision_type_3d);
            fs(o, coordinates);
            strs(o, attachments);
            o.push('\n');
        }
        DynamicKind::Halo(halos) => for h in *halos {
            o.push_str("halo");
            s(o, h.name);
            fs(o, &h.values);
            o.push('\n');
        },
        DynamicKind::StaticEffect => {}
    }
}

fn full(m: &Map) -> String {
    let mut o = String::new();
    let o = &mut o;
    writeln!(o, "timestamp {}", m.timestamp).unwrap();

    writeln!(o, "materials {} {}", m.materials.section.id, m.materials.items.len()).unwrap();
    for t in m.materials.items {
        write!(o, "material {}", t.section.id).unwrap();
        s(o, t.section.name);
        s(o, t.filename);
        f(o, t.opacity);
        write!(o, " {} {}", t.emissive_strength, t.address_mode as u32).unwrap();
        color(o, t.ambient);
        color(o, t.diffuse);
        color(o, t.specular);
        f(o, t.specular_level);
        writeln!(o, " {}", t.two_sided as u8).unwrap();
    }

    writeln!(o, "geometries {} {}", m.geometries.section.id, m.geometries.items.len()).unwrap();
    for ob in m.geometries.items {
        write!(o, "object {}", ob.section.id).unwrap();
        s(o, ob.section.name);
        write!(o, " {}", ob.object_section.id).unwrap();
        s(o, ob.object_section.name);
        o.push('\n');
        for &v in ob.vertices {
            o.push('v');
            v3(o, v);
            o.push('\n');
        }
        for d in ob.datas {
            writeln!(o, "data {} {} {}", d.material, d.face_planes.len(), d.normals.len()).unwrap();
            for k in 0..d.face_planes.len() {
                o.push_str("face");
                plane(o, d.face_planes[k]);
                let (a, b) = (d.faces[k], d.face_texture_verts[k]);
                writeln!(o, " {} {} {} {} {} {}", a.a, a.b, a.c, b.a, b.b, b.c).unwrap();
            }
            for k in 0..d.normals.len() {
                o.push_str("tv");
                v3(o, d.normals[k]);
                fs(o, &[d.uvs[k].u, d.uvs[k].v]);
                color(o, d.colors[k]);
                o.push('\n');
            }
        }
        for &v in ob.collision_vertices {
            o.push_str("cv");
            v3(o, v);
            o.push('\n');
        }
        for &p in ob.collision_planes {
            o.push_str("cp");
            plane(o, p);
            o.push('\n');
        }
        for t in ob.tags {
            writeln!(o, "tag {} {} {} {} {} {} {} {}", t.coord1[0], t.coord1[1], t.coord1[2], t.face_index_1,
                     t.coord2[0], t.coord2[1], t.coord2[2], t.face_index_2).unwrap();
        }
        for g in ob.groups {
            o.push_str("group");
            s(o, g.name);
            write!(o, " {} {}", g.flags, g.indices.len()).unwrap();
            for i in g.indices {
                write!(o, " {i}").unwrap();
            }
            o.push('\n');
        }
    }

    section(o, "portals", m.portals.section, m.portals.items.len());
    for p in m.portals.items {
        write!(o, "portal {}", p.section.id).unwrap();
        s(o, p.section.name);
        write!(o, " {} {} {}", p.room, p.opposite_room, p.vertices.len()).unwrap();
        for &v in p.vertices {
            v3(o, v);
        }
        o.push('\n');
    }

    section(o, "lights", m.lights.0, m.lights.1 as usize);

    section(o, "dynamic_objects", m.dynamic_objects.section, m.dynamic_objects.items.len());
    for d in m.dynamic_objects.items {
        dynamic_object(o, d);
    }

    section(o, "rooms", m.rooms.section, m.rooms.items.len());
    for r in m.rooms.items {
        write!(o, "room {}", r.section.id).unwrap();
        s(o, r.section.name);
        write!(o, " {}", r.enclosed).unwrap();
        match r.small { Some(x) => write!(o, " {x}").unwrap(), None => o.push_str(" -") }
        write!(o, " {}", r.unknown_flag).unwrap();
        match r.weather_flag { Some(x) => write!(o, " {x}").unwrap(), None => o.push_str(" -") }
        match r.weather_bounds {
            Some([min, max]) => {
                v3(o, min);
                v3(o, max);
            }
            None => o.push_str(" -"),
        }
        o.push('\n');
        for l in r.levels {
            o.push_str("level");
            s(o, l.name);
            write!(o, " 0 {}", l.floors.len()).unwrap();
            fs(o, l.floors);
            o.push('\n');
            for b in l.boxes {
                o.push_str("box");
                tm(o, b.transform);
                fs(o, &b.aabb);
                o.push('\n');
            }
        }
        for fl in r.floors {
            o.push_str("floor");
            fs(o, &[fl.number, fl.height]);
            o.push('\n');
        }
        write!(o, "unknown {}", r.unknown.len()).unwrap();
        for x in r.unknown {
            write!(o, " {x:08x}").unwrap();
        }
        o.push('\n');
    }

    section(o, "transitions", m.transitions.section, m.transitions.items.len());
    for t in m.transitions.items {
        o.push_str("transition");
        s(o, t.name);
        v3(o, t.p1);
        v3(o, t.p2);
        o.push('\n');
    }

    section(o, "planning_levels", m.planning_levels.section, m.planning_levels.items.len());
    for l in m.planning_levels.items {
        o.push_str("planning");
        fs(o, &[l.level_number, l.floor_height]);
        strs(o, l.room_names);
        o.push('\n');
    }
    std::mem::take(o)
}

// --- Timing --------------------------------------------------------------------------------

/// Runs `once` twice to warm up, then until it has taken 300 ms and run 20 times
fn bench(name: &str, path: &str, bytes: usize, mut once: impl FnMut() -> Duration) {
    once();
    once();
    let mut times = Vec::new();
    let mut total = Duration::ZERO;
    while total < Duration::from_millis(300) || times.len() < 20 {
        let t = once();
        total += t;
        times.push(t);
    }
    times.sort();
    println!("{LABEL} {name} {path} {bytes} {} {} {}", times.len(), times[0].as_nanos(), times[times.len() / 2].as_nanos());
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mode = if args.first().is_some_and(|a| a.starts_with("--")) { args.remove(0) } else { String::new() };
    let mut mem = vec![0u8; ARENA_SIZE];
    let mut status = 0;
    for path in &args {
        match mode.as_str() {
            "--full" => {
                let mut arena = Arena::new(&mut mem);
                match map::read(path, &mut arena) {
                    Ok(m) => std::io::stdout().write_all(full(&m).as_bytes()).unwrap(),
                    Err(e) => {
                        eprintln!("{path}: {e}");
                        status = 1;
                    }
                }
            }
            "--bench" => {
                let data = std::fs::read(path).expect("the map");
                bench("read", path, data.len(), || {
                    let mut arena = Arena::new(&mut mem);
                    let t0 = Instant::now();
                    std::hint::black_box(map::read(path, &mut arena).expect("the map reads"));
                    t0.elapsed()
                });
                bench("parse", path, data.len(), || {
                    let mut arena = Arena::new(&mut mem);
                    let t0 = Instant::now();
                    std::hint::black_box(map::parse(&data, &mut arena).expect("the map reads"));
                    t0.elapsed()
                });
                // The file's reading as map::read does it
                bench("file", path, data.len(), || {
                    let t0 = Instant::now();
                    let mut file = std::fs::File::open(path).expect("the map");
                    let size = file.metadata().expect("its size").len() as usize;
                    file.read_exact(&mut mem[ARENA_SIZE - size..]).expect("the map");
                    t0.elapsed()
                });
            }
            _ => {
                eprintln!("usage: maptool --full|--bench MAP...");
                std::process::exit(2);
            }
        }
    }
    std::process::exit(status);
}
