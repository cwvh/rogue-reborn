//! maptool3 --full MAP...   every field of each map, as `mapdump --full` prints it
//! maptool3 --bench MAP...  times reading each map, as `mapbench` does, printing lines
//!   rsarena3 STEP MAP BYTES RUNS MIN_NS MEDIAN_NS
//! where STEP is "read" (map::read from the path), "parse" (map::parse of the file's bytes
//! already in memory) or "file" (only reading the file).
//!
//! maptool.rs with the reader in map3.rs, and like it without std: only core and the C
//! library. Built on its own with
//!   rustc --edition 2024 -C opt-level=2 -C panic=abort -C lto=fat -C codegen-units=1 maptool3.rs
//! where LTO drops what core holds that nothing calls. On x86_64-pc-windows-gnu with a GCC that
//! has no libgcc_eh, add -L to a directory with an empty libgcc_eh.a, as build.rs makes for cargo.

#![no_std]
#![no_main]

// map3.rs's #![no_std] is for when it is a crate of its own
#[allow(dead_code, unused_attributes)]
#[path = "map3.rs"]
mod map;

use core::ffi::{CStr, c_char, c_int, c_long, c_void};
use core::hint::black_box;
use core::ptr;

use map::*;

const ARENA_SIZE: usize = 32 << 20;
const MAX_RUNS: usize = 100_000;

unsafe extern "C" {
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn abort() -> !;
    fn fopen(path: *const c_char, mode: *const c_char) -> *mut c_void;
    fn fseek(file: *mut c_void, offset: c_long, whence: c_int) -> c_int;
    fn ftell(file: *mut c_void) -> c_long;
    fn fread(buf: *mut c_void, size: usize, count: usize, file: *mut c_void) -> usize;
    fn fclose(file: *mut c_void) -> c_int;
}

#[cfg(windows)]
unsafe extern "C" {
    fn _write(fd: c_int, buf: *const c_void, count: u32) -> c_int;
    fn _setmode(fd: c_int, mode: c_int) -> c_int;
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(freq: *mut i64) -> i32;
}

#[cfg(unix)]
#[link(name = "c")]
unsafe extern "C" {
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    fn clock_gettime(clock: c_int, t: *mut [i64; 2]) -> c_int;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { abort() }
}

// The prebuilt core's unwind tables name this, though with panic=abort nothing unwinds
#[unsafe(no_mangle)]
extern "C" fn rust_eh_personality() {}

// --- Output --------------------------------------------------------------------------------

fn write_all(fd: c_int, mut b: &[u8]) {
    while !b.is_empty() {
        #[cfg(windows)]
        let n = unsafe { _write(fd, b.as_ptr().cast(), b.len().min(1 << 30) as u32) } as isize;
        #[cfg(unix)]
        let n = unsafe { write(fd, b.as_ptr().cast(), b.len()) };
        if n <= 0 {
            return;
        }
        b = &b[n as usize..];
    }
}

/// Text written to a file descriptor through a buffer
struct Out {
    fd: c_int,
    buf: [u8; 1 << 16],
    len: usize,
}

impl Out {
    fn new(fd: c_int) -> Self {
        Self { fd, buf: [0; 1 << 16], len: 0 }
    }

    fn flush(&mut self) {
        write_all(self.fd, &self.buf[..self.len]);
        self.len = 0;
    }

    // Called from all over the dumper, which is not timed: out of line, it is written once
    #[inline(never)]
    fn push(&mut self, b: &[u8]) {
        if b.len() > self.buf.len() - self.len {
            self.flush();
            if b.len() > self.buf.len() {
                return write_all(self.fd, b);
            }
        }
        // SAFETY: the bytes fit after the len written
        unsafe { ptr::copy_nonoverlapping(b.as_ptr(), self.buf.as_mut_ptr().add(self.len), b.len()) };
        self.len += b.len();
    }

    fn t(&mut self, s: &str) {
        self.push(s.as_bytes());
    }

    fn nl(&mut self) {
        self.push(b"\n");
    }

    /// A space, then `x` in decimal
    #[inline(never)]
    fn u(&mut self, mut x: u64) {
        let mut d = [0u8; 21];
        let mut i = d.len();
        loop {
            i -= 1;
            d[i] = b'0' + (x % 10) as u8;
            x /= 10;
            if x == 0 {
                break;
            }
        }
        i -= 1;
        d[i] = b' ';
        self.push(&d[i..]);
    }

    /// `x` in lowercase hex, `digits` long
    #[inline(never)]
    fn hex(&mut self, x: u32, digits: usize) {
        let mut d = [0u8; 8];
        for (i, c) in d[..digits].iter_mut().enumerate() {
            *c = b"0123456789abcdef"[(x >> (4 * (digits - 1 - i)) & 15) as usize];
        }
        self.push(&d[..digits]);
    }
}

// --- Every field ---------------------------------------------------------------------------

fn f(o: &mut Out, x: f32) {
    o.push(b" ");
    o.hex(x.to_bits(), 8);
}

#[inline(never)]
fn fs(o: &mut Out, xs: &[f32]) {
    xs.iter().for_each(|&x| f(o, x));
}

#[inline(never)]
fn s(o: &mut Out, b: &[u8]) {
    o.push(b" \"");
    for &c in b {
        match c {
            b'"' | b'\\' => o.push(&[b'\\', c]),
            0x20..0x7f => o.push(&[c]),
            _ => {
                o.push(b"\\x");
                o.hex(c as u32, 2);
            }
        }
    }
    o.push(b"\"");
}

fn strs(o: &mut Out, xs: &[&[u8]]) {
    o.u(xs.len() as u64);
    xs.iter().for_each(|x| s(o, x));
}

fn v3(o: &mut Out, v: Vec3) {
    fs(o, &[v.x, v.y, v.z]);
}

fn tm(o: &mut Out, t: Transform) {
    [t.x_axis, t.y_axis, t.z_axis, t.position].into_iter().for_each(|v| v3(o, v));
}

fn color(o: &mut Out, c: Color) {
    fs(o, &[c.r, c.g, c.b, c.a]);
}

fn plane(o: &mut Out, p: Plane) {
    fs(o, &[p.x, p.y, p.z, p.distance]);
}

fn section(o: &mut Out, what: &str, sec: Section, count: usize) {
    o.t(what);
    o.u(sec.id as u64);
    s(o, sec.name);
    o.u(count as u64);
    o.nl();
}

fn common(o: &mut Out, c: &DynamicCommon) {
    o.t("common");
    tm(o, c.transform);
    s(o, c.name);
    o.u(c.unknown1 as u64);
    strs(o, &c.sounds);
    for x in [c.collision_type_2d, c.collision_type_3d, c.destruction_action, c.destruction_category,
              c.penetration_type, c.name2, c.destruction_category2] {
        s(o, x);
    }
    o.nl();
}

fn dynamic_object(o: &mut Out, d: &DynamicObject) {
    o.t("dynamic");
    o.u(d.section.id as u64);
    s(o, d.section.name);
    s(o, d.name);
    tm(o, d.transform);
    o.nl();
    match &d.kind {
        DynamicKind::Dynamic { common: c, params } => {
            common(o, c);
            match params {
                DynamicParams::Struct(ps) => for p in *ps {
                    o.t("param");
                    s(o, p.name);
                    fs(o, &p.unknown1);
                    o.u(p.unknown2[0] as u64);
                    o.u(p.unknown2[1] as u64);
                    o.nl();
                },
                DynamicParams::Flat { names, unknown } => {
                    o.t("flat");
                    strs(o, names);
                    fs(o, unknown);
                    o.nl();
                }
            }
        }
        DynamicKind::Animation { common: c, unknown2, names, unknown3, unknown4, name3, name4, animation_type, direction, distance, velocity } => {
            common(o, c);
            o.t("animation");
            o.u(*unknown2 as u64);
            strs(o, names);
            fs(o, unknown3);
            o.u(*unknown4 as u64);
            s(o, name3);
            s(o, name4);
            s(o, animation_type);
            v3(o, *direction);
            fs(o, &[*distance, *velocity]);
            o.nl();
        }
        DynamicKind::RepeatableTouchplate { common: c, unknown1, attachments, unknown2, names, name2, name3, animation_type, direction, distance, velocity } => {
            common(o, c);
            o.t("touchplate");
            o.u(*unknown1 as u64);
            strs(o, attachments);
            fs(o, unknown2);
            strs(o, names);
            s(o, name2);
            s(o, name3);
            s(o, animation_type);
            v3(o, *direction);
            fs(o, &[*distance, *velocity]);
            o.nl();
        }
        DynamicKind::Glass { name } => {
            o.t("glass");
            s(o, name);
            o.nl();
        }
        DynamicKind::OneTimeTouchplate { collision_type_2d, collision_type_3d, coordinates, attachments } => {
            o.t("once");
            s(o, collision_type_2d);
            s(o, collision_type_3d);
            fs(o, coordinates);
            strs(o, attachments);
            o.nl();
        }
        DynamicKind::Halo(halos) => for h in *halos {
            o.t("halo");
            s(o, h.name);
            fs(o, &h.values);
            o.nl();
        },
        DynamicKind::StaticEffect => {}
    }
}

fn full(o: &mut Out, m: &Map) {
    o.t("timestamp");
    o.u(m.timestamp as u64);
    o.nl();

    o.t("materials");
    o.u(m.materials.section.id as u64);
    o.u(m.materials.items.len() as u64);
    o.nl();
    for t in m.materials.items {
        o.t("material");
        o.u(t.section.id as u64);
        s(o, t.section.name);
        s(o, t.filename);
        f(o, t.opacity);
        o.u(t.emissive_strength as u64);
        o.u(t.address_mode as u64);
        color(o, t.ambient);
        color(o, t.diffuse);
        color(o, t.specular);
        f(o, t.specular_level);
        o.u(t.two_sided as u64);
        o.nl();
    }

    o.t("geometries");
    o.u(m.geometries.section.id as u64);
    o.u(m.geometries.items.len() as u64);
    o.nl();
    for ob in m.geometries.items {
        o.t("object");
        o.u(ob.section.id as u64);
        s(o, ob.section.name);
        o.u(ob.object_section.id as u64);
        s(o, ob.object_section.name);
        o.nl();
        for &v in ob.vertices {
            o.t("v");
            v3(o, v);
            o.nl();
        }
        for d in ob.datas {
            o.t("data");
            o.u(d.material as u64);
            o.u(d.face_planes.len() as u64);
            o.u(d.normals.len() as u64);
            o.nl();
            for k in 0..d.face_planes.len() {
                o.t("face");
                plane(o, d.face_planes[k]);
                let (a, b) = (d.faces[k], d.face_texture_verts[k]);
                for x in [a.a, a.b, a.c, b.a, b.b, b.c] {
                    o.u(x as u64);
                }
                o.nl();
            }
            for k in 0..d.normals.len() {
                o.t("tv");
                v3(o, d.normals[k]);
                fs(o, &[d.uvs[k].u, d.uvs[k].v]);
                color(o, d.colors[k]);
                o.nl();
            }
        }
        for &v in ob.collision_vertices {
            o.t("cv");
            v3(o, v);
            o.nl();
        }
        for &p in ob.collision_planes {
            o.t("cp");
            plane(o, p);
            o.nl();
        }
        for t in ob.tags {
            o.t("tag");
            for x in [t.coord1[0], t.coord1[1], t.coord1[2], t.face_index_1, t.coord2[0], t.coord2[1], t.coord2[2], t.face_index_2] {
                o.u(x as u64);
            }
            o.nl();
        }
        for g in ob.groups {
            o.t("group");
            s(o, g.name);
            o.u(g.flags as u64);
            o.u(g.indices.len() as u64);
            for &i in g.indices {
                o.u(i as u64);
            }
            o.nl();
        }
    }

    section(o, "portals", m.portals.section, m.portals.items.len());
    for p in m.portals.items {
        o.t("portal");
        o.u(p.section.id as u64);
        s(o, p.section.name);
        o.u(p.room as u64);
        o.u(p.opposite_room as u64);
        o.u(p.vertices.len() as u64);
        for &v in p.vertices {
            v3(o, v);
        }
        o.nl();
    }

    section(o, "lights", m.lights.0, m.lights.1 as usize);

    section(o, "dynamic_objects", m.dynamic_objects.section, m.dynamic_objects.items.len());
    for d in m.dynamic_objects.items {
        dynamic_object(o, d);
    }

    section(o, "rooms", m.rooms.section, m.rooms.items.len());
    for r in m.rooms.items {
        o.t("room");
        o.u(r.section.id as u64);
        s(o, r.section.name);
        o.u(r.enclosed as u64);
        match r.small { Some(x) => o.u(x as u64), None => o.t(" -") }
        o.u(r.unknown_flag as u64);
        match r.weather_flag { Some(x) => o.u(x as u64), None => o.t(" -") }
        match r.weather_bounds {
            Some([min, max]) => {
                v3(o, min);
                v3(o, max);
            }
            None => o.t(" -"),
        }
        o.nl();
        for l in r.levels {
            o.t("level");
            s(o, l.name);
            o.t(" 0");
            o.u(l.floors.len() as u64);
            fs(o, l.floors);
            o.nl();
            for b in l.boxes {
                o.t("box");
                tm(o, b.transform);
                fs(o, &b.aabb);
                o.nl();
            }
        }
        for fl in r.floors {
            o.t("floor");
            fs(o, &[fl.number, fl.height]);
            o.nl();
        }
        o.t("unknown");
        o.u(r.unknown.len() as u64);
        for &x in r.unknown {
            o.push(b" ");
            o.hex(x, 8);
        }
        o.nl();
    }

    section(o, "transitions", m.transitions.section, m.transitions.items.len());
    for t in m.transitions.items {
        o.t("transition");
        s(o, t.name);
        v3(o, t.p1);
        v3(o, t.p2);
        o.nl();
    }

    section(o, "planning_levels", m.planning_levels.section, m.planning_levels.items.len());
    for l in m.planning_levels.items {
        o.t("planning");
        fs(o, &[l.level_number, l.floor_height]);
        strs(o, l.room_names);
        o.nl();
    }
}

// --- Timing --------------------------------------------------------------------------------

#[cfg(windows)]
fn now_ns() -> u64 {
    let (mut c, mut f) = (0i64, 0i64);
    unsafe {
        QueryPerformanceCounter(&mut c);
        QueryPerformanceFrequency(&mut f);
    }
    let (c, f) = (c as u64, f as u64);
    c / f * 1_000_000_000 + c % f * 1_000_000_000 / f
}

#[cfg(unix)]
fn now_ns() -> u64 {
    let mut t = [0i64; 2];
    unsafe { clock_gettime(1, &mut t) }; // CLOCK_MONOTONIC
    t[0] as u64 * 1_000_000_000 + t[1] as u64
}

/// Runs `once` twice to warm up, then until it has taken 300 ms and run 20 times
fn bench(o: &mut Out, times: &mut [u64], name: &str, path: &[u8], bytes: usize, once: &mut dyn FnMut() -> u64) {
    once();
    once();
    let (mut runs, mut total) = (0, 0);
    while (total < 300_000_000 || runs < 20) && runs < times.len() {
        let t = once();
        total += t;
        times[runs] = t;
        runs += 1;
    }
    // An insertion sort, which is small and, at a few thousand runs, quick enough
    let times = &mut times[..runs];
    for i in 1..runs {
        let mut j = i;
        while j > 0 && times[j - 1] > times[j] {
            times.swap(j - 1, j);
            j -= 1;
        }
    }
    o.t("rsarena3 ");
    o.t(name);
    o.push(b" ");
    o.push(path);
    o.u(bytes as u64);
    o.u(runs as u64);
    o.u(times[0]);
    o.u(times[runs / 2]);
    o.nl();
    o.flush();
}

/// The file's bytes at the top of `mem`, as map::read reads them, or None where it fails
fn load(path: &CStr, mem: &mut [u8]) -> Option<usize> {
    unsafe {
        let f = fopen(path.as_ptr(), c"rb".as_ptr());
        if f.is_null() {
            return None;
        }
        let size = if fseek(f, 0, 2) != 0 { -1 } else { ftell(f) };
        let ok = size >= 0 && size as usize <= mem.len() && fseek(f, 0, 0) == 0
            && fread(mem.as_mut_ptr().add(mem.len() - size as usize).cast(), 1, size as usize, f) == size as usize;
        fclose(f);
        ok.then_some(size as usize)
    }
}

fn alloc(size: usize) -> &'static mut [u8] {
    let p = unsafe { calloc(size, 1) }.cast::<u8>();
    if p.is_null() {
        unsafe { abort() }
    }
    unsafe { core::slice::from_raw_parts_mut(p, size) }
}

#[unsafe(no_mangle)]
pub extern "C" fn main(argc: c_int, argv: *const *const c_char) -> c_int {
    // Lines end in LF, not CRLF, as std writes them
    #[cfg(windows)]
    unsafe {
        _setmode(1, 0x8000);
        _setmode(2, 0x8000);
    }
    let args = unsafe { core::slice::from_raw_parts(argv, argc as usize) };
    let mut args = args.iter().skip(1).map(|&a| unsafe { CStr::from_ptr(a) }).peekable();
    let mode = match args.peek() {
        Some(a) if a.to_bytes().starts_with(b"--") => args.next().unwrap().to_bytes(),
        _ => b"",
    };
    let mem = alloc(ARENA_SIZE);
    let times = unsafe { core::slice::from_raw_parts_mut(alloc(MAX_RUNS * 8).as_mut_ptr().cast::<u64>(), MAX_RUNS) };
    let mut o = Out::new(1);
    let mut status = 0;
    for path in args {
        let name = path.to_bytes();
        match mode {
            b"--full" => {
                let mut arena = Arena::new(mem);
                match map::read(path, &mut arena) {
                    Ok(m) => full(&mut o, &m),
                    Err(e) => {
                        o.flush();
                        let mut err = Out::new(2);
                        err.push(name);
                        err.t(": ");
                        err.t(e);
                        err.nl();
                        err.flush();
                        status = 1;
                    }
                }
            }
            b"--bench" => {
                let data = alloc(ARENA_SIZE);
                let Some(size) = load(path, data) else { unsafe { abort() } };
                let data = &data[ARENA_SIZE - size..];
                bench(&mut o, times, "read", name, size, &mut || {
                    let mut arena = Arena::new(mem);
                    let t0 = now_ns();
                    if black_box(map::read(path, &mut arena)).is_err() {
                        unsafe { abort() }
                    }
                    now_ns() - t0
                });
                bench(&mut o, times, "parse", name, size, &mut || {
                    let mut arena = Arena::new(mem);
                    let t0 = now_ns();
                    if black_box(map::parse(data, &mut arena)).is_err() {
                        unsafe { abort() }
                    }
                    now_ns() - t0
                });
                // The file's reading as map::read does it
                bench(&mut o, times, "file", name, size, &mut || {
                    let t0 = now_ns();
                    if load(path, mem).is_none() {
                        unsafe { abort() }
                    }
                    now_ns() - t0
                });
            }
            _ => {
                let mut err = Out::new(2);
                err.t("usage: maptool3 --full|--bench MAP...\n");
                err.flush();
                return 2;
            }
        }
    }
    o.flush();
    status
}
