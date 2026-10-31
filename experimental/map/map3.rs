//! Reader for Rogue Spear .MAP files, in the style of map.c: everything a map holds is in
//! one arena, strings are the file's bytes (ISO-8859-1), and an error's message is only built
//! when reading fails. It reads the same bytes in the same order as src/map.rs, with the same
//! checks and the same messages.
//!
//! map.rs with the arrays copied at once where they are all there, as map2.c does. Where
//! one is cut short it is read an item at a time as map.rs does, which fails with its message.
//!
//! map2.rs made smaller, with only core and the C library's file functions: an error is built
//! in a fixed buffer, as map.c's is, and kept in the arena; the messages are built out of line;
//! nothing panics; and the lists are read through one body for each type of item.

#![no_std]

use core::ffi::{CStr, c_char, c_int, c_long, c_void};
use core::fmt::{self, Write as _};
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr;

// --- Arena ---------------------------------------------------------------------------------

/// Memory handed out from `beg` up, never freed one piece at a time
pub struct Arena<'m> {
    beg: *mut u8,
    end: *mut u8,
    _mem: PhantomData<&'m mut [u8]>,
}

impl<'m> Arena<'m> {
    pub fn new(mem: &'m mut [u8]) -> Self {
        let range = mem.as_mut_ptr_range();
        Self { beg: range.start, end: range.end, _mem: PhantomData }
    }

    fn left(&self) -> usize {
        self.end as usize - self.beg as usize
    }

    /// Room for `n` items, or None where they do not fit
    fn alloc<T: Copy>(&mut self, n: usize) -> Option<&'m mut [MaybeUninit<T>]> {
        // Bytes, which strings are, need no padding: their room is found here
        let p = if size_of::<T>() == 1 && align_of::<T>() == 1 {
            if n > self.left() {
                return None;
            }
            let p = self.beg;
            // SAFETY: the n bytes are inside the arena's memory
            self.beg = unsafe { p.add(n) };
            p
        } else {
            self.alloc_bytes(n, size_of::<T>(), align_of::<T>())?
        };
        // SAFETY: the room is the arena's, for n items of T
        Some(unsafe { core::slice::from_raw_parts_mut(p.cast(), n) })
    }

    /// The body of `alloc`, of which there is one, not one for each type of item
    #[inline(never)]
    fn alloc_bytes(&mut self, n: usize, size: usize, align: usize) -> Option<*mut u8> {
        let pad = (self.beg as usize).wrapping_neg() & (align - 1);
        let left = self.left();
        let bytes = n.checked_mul(size)?;
        if pad > left || bytes > left - pad {
            return None;
        }
        // SAFETY: the room is inside the arena's memory, after anything handed out before
        unsafe {
            let p = self.beg.add(pad);
            self.beg = p.add(bytes);
            Some(p)
        }
    }
}

/// Items every one of which has been written
unsafe fn assume_init<'m, T>(items: &'m mut [MaybeUninit<T>]) -> &'m [T] {
    unsafe { &*(items as *mut [MaybeUninit<T>] as *const [T]) }
}

// --- What a map holds ----------------------------------------------------------------------

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Vec3 { pub x: f32, pub y: f32, pub z: f32 }
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Color { pub r: f32, pub g: f32, pub b: f32, pub a: f32 }
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Uv { pub u: f32, pub v: f32 }
/// A face's plane: its normal, and the distance from the origin to the face along it
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Plane { pub x: f32, pub y: f32, pub z: f32, pub distance: f32 }
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Tri { pub a: u16, pub b: u16, pub c: u16 }
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Transform { pub x_axis: Vec3, pub y_axis: Vec3, pub z_axis: Vec3, pub position: Vec3 }

/// Every list in the file starts with one of these. Rooms leave out the size.
#[derive(Clone, Copy)]
pub struct Section<'m> {
    pub size: u32,
    pub id: u32,
    /// 0 unless the name is preceded by "Version" and a number
    pub version: u32,
    pub name: &'m [u8],
}

#[derive(Clone, Copy)]
pub struct List<'m, T> {
    pub section: Section<'m>,
    pub items: &'m [T],
}

#[derive(Clone, Copy)]
pub enum AddressMode { Opaque = 0, Wrap = 1, Clamp = 3 }

#[derive(Clone, Copy)]
pub struct Material<'m> {
    pub section: Section<'m>,
    pub filename: &'m [u8],
    pub opacity: f32,
    pub emissive_strength: u32,
    pub address_mode: AddressMode,
    pub ambient: Color,
    pub diffuse: Color,
    pub specular: Color,
    pub specular_level: f32,
    pub two_sided: bool,
}

/// The triangles of an object drawn with one material
#[derive(Clone, Copy)]
pub struct ObjectData<'m> {
    pub material: u32,
    pub face_planes: &'m [Plane],
    pub faces: &'m [Tri],
    pub face_texture_verts: &'m [Tri],
    pub normals: &'m [Vec3],
    pub uvs: &'m [Uv],
    pub colors: &'m [Color],
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Tag {
    pub coord1: [u16; 3],
    pub face_index_1: u16,
    pub coord2: [u16; 3],
    pub face_index_2: u16,
}

#[derive(Clone, Copy)]
pub struct CollisionGroup<'m> {
    pub name: &'m [u8],
    pub flags: u32,
    pub indices: &'m [u16],
}

#[derive(Clone, Copy)]
pub struct Object<'m> {
    pub section: Section<'m>,
    pub object_section: Section<'m>,
    pub vertices: &'m [Vec3],
    pub datas: &'m [ObjectData<'m>],
    pub collision_vertices: &'m [Vec3],
    pub collision_planes: &'m [Plane],
    pub tags: &'m [Tag],
    pub groups: &'m [CollisionGroup<'m>],
}

#[derive(Clone, Copy)]
pub struct Portal<'m> {
    pub section: Section<'m>,
    pub vertices: &'m [Vec3],
    pub room: u32,
    pub opposite_room: u32,
}

#[derive(Clone, Copy)]
pub struct DynamicCommon<'m> {
    pub transform: Transform,
    pub name: &'m [u8],
    pub unknown1: u32,
    pub sounds: [&'m [u8]; 4],
    pub collision_type_2d: &'m [u8],
    pub collision_type_3d: &'m [u8],
    pub destruction_action: &'m [u8],
    pub destruction_category: &'m [u8],
    pub penetration_type: &'m [u8],
    pub name2: &'m [u8],
    pub destruction_category2: &'m [u8],
}

#[derive(Clone, Copy)]
pub struct DynamicParam<'m> {
    pub name: &'m [u8],
    pub unknown1: [f32; 9],
    pub unknown2: [u32; 2],
}

#[derive(Clone, Copy)]
pub struct Halo<'m> {
    pub name: &'m [u8],
    pub values: [f32; 8],
}

#[derive(Clone, Copy)]
pub enum DynamicParams<'m> {
    /// Where the file's count is not 0
    Struct(&'m [DynamicParam<'m>]),
    Flat { names: &'m [&'m [u8]], unknown: [f32; 4] },
}

#[derive(Clone, Copy)]
pub enum DynamicKind<'m> {
    Dynamic { common: DynamicCommon<'m>, params: DynamicParams<'m> },
    Animation {
        common: DynamicCommon<'m>,
        unknown2: u32,
        names: &'m [&'m [u8]],
        unknown3: [f32; 3],
        unknown4: u32,
        name3: &'m [u8],
        name4: &'m [u8],
        animation_type: &'m [u8],
        direction: Vec3,
        distance: f32,
        velocity: f32,
    },
    RepeatableTouchplate {
        common: DynamicCommon<'m>,
        unknown1: u32,
        attachments: &'m [&'m [u8]],
        unknown2: [f32; 3],
        names: &'m [&'m [u8]],
        name2: &'m [u8],
        name3: &'m [u8],
        animation_type: &'m [u8],
        direction: Vec3,
        distance: f32,
        velocity: f32,
    },
    Glass { name: &'m [u8] },
    OneTimeTouchplate {
        collision_type_2d: &'m [u8],
        collision_type_3d: &'m [u8],
        coordinates: [f32; 6],
        attachments: &'m [&'m [u8]],
    },
    Halo(&'m [Halo<'m>]),
    StaticEffect,
}

#[derive(Clone, Copy)]
pub struct DynamicObject<'m> {
    pub section: Section<'m>,
    pub name: &'m [u8],
    pub transform: Transform,
    pub kind: DynamicKind<'m>,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct LevelBox {
    pub transform: Transform,
    pub aabb: [f32; 6],
}

#[derive(Clone, Copy)]
pub struct Level<'m> {
    pub name: &'m [u8],
    pub boxes: &'m [LevelBox],
    pub floors: &'m [f32],
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct RoomFloor {
    pub number: f32,
    pub height: f32,
}

#[derive(Clone, Copy)]
pub struct Room<'m> {
    pub section: Section<'m>,
    pub enclosed: u8,
    /// Where not enclosed
    pub small: Option<u8>,
    pub unknown_flag: u8,
    /// Where the version is above 1: whether weather bounds follow, and the bounds
    pub weather_flag: Option<u8>,
    pub weather_bounds: Option<[Vec3; 2]>,
    pub levels: &'m [Level<'m>],
    pub floors: &'m [RoomFloor],
    pub unknown: &'m [u32],
}

#[derive(Clone, Copy)]
pub struct Transition<'m> {
    pub name: &'m [u8],
    pub p1: Vec3,
    pub p2: Vec3,
}

#[derive(Clone, Copy)]
pub struct PlanningLevel<'m> {
    pub level_number: f32,
    pub floor_height: f32,
    pub room_names: &'m [&'m [u8]],
}

#[derive(Clone, Copy)]
pub struct Map<'m> {
    pub timestamp: u32,
    pub materials: List<'m, Material<'m>>,
    pub geometries: List<'m, Object<'m>>,
    pub portals: List<'m, Portal<'m>>,
    pub lights: (Section<'m>, u32),
    pub dynamic_objects: List<'m, DynamicObject<'m>>,
    pub rooms: List<'m, Room<'m>>,
    pub transitions: List<'m, Transition<'m>>,
    pub planning_levels: List<'m, PlanningLevel<'m>>,
}

/// Laid out in the file as in memory: any bytes are a valid value, and there is no padding
unsafe trait Pod: Copy {}
unsafe impl Pod for u16 {}
unsafe impl Pod for u32 {}
unsafe impl Pod for f32 {}
unsafe impl Pod for Vec3 {}
unsafe impl Pod for Color {}
unsafe impl Pod for Uv {}
unsafe impl Pod for Plane {}
unsafe impl Pod for Tri {}
unsafe impl Pod for Tag {}
unsafe impl Pod for LevelBox {}
unsafe impl Pod for RoomFloor {}

const _: () = assert!(cfg!(target_endian = "little"));
const _: () = assert!(size_of::<Vec3>() == 12 && size_of::<Color>() == 16 && size_of::<Uv>() == 8);
const _: () = assert!(size_of::<Plane>() == 16 && size_of::<Tri>() == 6 && size_of::<Tag>() == 16);
const _: () = assert!(size_of::<LevelBox>() == 72 && size_of::<RoomFloor>() == 8);

// --- Reading -------------------------------------------------------------------------------

/// Reading failed: the reader's `err` says why
struct Fail;

type R<T> = Result<T, Fail>;

/// An error's message, cut off at 1024 bytes as map.c's is
struct Msg {
    buf: [u8; 1024],
    len: usize,
}

impl Msg {
    fn new() -> Self {
        Self { buf: [0; 1024], len: 0 }
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: len is never more than the buffer
        unsafe { self.buf.get_unchecked(..self.len) }
    }

    fn push(&mut self, b: &[u8]) {
        let n = b.len().min(self.buf.len() - self.len);
        // SAFETY: the n bytes fit after the len written
        unsafe { ptr::copy_nonoverlapping(b.as_ptr(), self.buf.as_mut_ptr().add(self.len), n) };
        self.len += n;
    }
}

impl fmt::Write for Msg {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push(s.as_bytes());
        Ok(())
    }
}

struct Reader<'d, 'a, 'm> {
    data: &'d [u8],
    pos: usize,
    arena: &'a mut Arena<'m>,
    err: Msg,
}

/// The value of `$e`, or with a context put in front of the error, as anyhow's .context() does,
/// a return of the failure
macro_rules! ctx {
    ($r:expr, $e:expr, $($fmt:tt)+) => {
        match $e {
            Ok(v) => v,
            Err(Fail) => return Err($r.context(format_args!($($fmt)+))),
        }
    };
}

impl<'d, 'a, 'm> Reader<'d, 'a, 'm> {
    fn fail<T>(&mut self, msg: impl fmt::Display) -> R<T> {
        Err(self.fail_with(format_args!("{msg}")))
    }

    #[cold]
    #[inline(never)]
    fn fail_with(&mut self, msg: fmt::Arguments) -> Fail {
        self.err.len = 0;
        let _ = self.err.write_fmt(msg);
        Fail
    }

    /// Put `head` in front of the error
    #[cold]
    #[inline(never)]
    fn context(&mut self, head: fmt::Arguments) -> Fail {
        let mut m = Msg::new();
        let _ = m.write_fmt(head);
        m.push(b": ");
        m.push(self.err.as_bytes());
        self.err = m;
        Fail
    }

    fn left(&self) -> usize {
        self.data.len() - self.pos
    }

    #[cold]
    #[inline(never)]
    fn eof(&mut self) -> Fail {
        self.pos = self.data.len();
        self.fail_with(format_args!("failed to fill whole buffer"))
    }

    fn take<const N: usize>(&mut self) -> R<[u8; N]> {
        if self.left() < N {
            return Err(self.eof());
        }
        // SAFETY: the N bytes are in the data
        let bytes = unsafe { self.data.as_ptr().add(self.pos).cast::<[u8; N]>().read_unaligned() };
        self.pos += N;
        Ok(bytes)
    }

    fn u8(&mut self) -> R<u8> {
        Ok(self.take::<1>()?[0])
    }

    fn u16(&mut self) -> R<u16> {
        Ok(u16::from_le_bytes(self.take()?))
    }

    fn u32(&mut self) -> R<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn f32(&mut self) -> R<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn bool(&mut self) -> R<bool> {
        Ok(ctx!(self, self.u8(), "could not read bool") == 1)
    }

    fn xyz(&mut self) -> R<Vec3> {
        Ok(Vec3 { x: ctx!(self, self.f32(), "x"), y: ctx!(self, self.f32(), "y"), z: ctx!(self, self.f32(), "z") })
    }

    fn plane(&mut self) -> R<Plane> {
        let v = self.xyz()?;
        Ok(Plane { x: v.x, y: v.y, z: v.z, distance: ctx!(self, self.f32(), "distance origin to face") })
    }

    fn color(&mut self) -> R<Color> {
        Ok(Color {
            r: ctx!(self, self.f32(), "red"),
            g: ctx!(self, self.f32(), "green"),
            b: ctx!(self, self.f32(), "blue"),
            a: ctx!(self, self.f32(), "alpha"),
        })
    }

    fn uv(&mut self) -> R<Uv> {
        Ok(Uv { u: ctx!(self, self.f32(), "u"), v: ctx!(self, self.f32(), "v") })
    }

    fn transform(&mut self) -> R<Transform> {
        Ok(Transform {
            x_axis: ctx!(self, self.xyz(), "transformation matrix x-axis"),
            y_axis: ctx!(self, self.xyz(), "transformation matrix y-axis"),
            z_axis: ctx!(self, self.xyz(), "transformation matrix z-axis"),
            position: ctx!(self, self.xyz(), "transformation matrix position"),
        })
    }

    fn floats<const N: usize>(&mut self, what: &str) -> R<[f32; N]> {
        let mut out = [0.0; N];
        for (i, x) in out.iter_mut().enumerate() {
            *x = ctx!(self, self.f32(), "{what} {i}");
        }
        Ok(out)
    }

    /// A string: its length, counting a NUL at the end, then its bytes. The last byte is
    /// dropped whatever it is, as in src/map.rs.
    fn string(&mut self) -> R<&'m [u8]> {
        let len = ctx!(self, self.u32(), "could not read length") as usize;
        if len < 1 {
            return self.fail("empty string");
        }
        if len > self.left() {
            let left = self.left();
            return self.fail(format_args!("could not read {len} bytes, only {left} left"));
        }
        let from = self.pos;
        self.pos += len;
        let Some(s) = self.arena.alloc::<u8>(len - 1) else { return self.fail("out of memory") };
        // SAFETY: the len bytes are in the data, and every byte is written
        unsafe {
            ptr::copy_nonoverlapping(self.data.as_ptr().add(from), s.as_mut_ptr().cast(), len - 1);
            Ok(assume_init(s))
        }
    }

    fn utf8(&mut self, s: &[u8]) -> R<()> {
        if core::str::from_utf8(s).is_err() {
            return self.fail("invalid utf-8");
        }
        Ok(())
    }

    /// Room for `n` items in the arena, or the failure to find it
    fn slots<T: Copy>(&mut self, n: u32, what: &str) -> R<&'m mut [MaybeUninit<T>]> {
        match self.arena.alloc::<T>(n as usize) {
            Some(slots) => Ok(slots),
            None => Err(self.out_of_memory(what)),
        }
    }

    #[cold]
    #[inline(never)]
    fn out_of_memory(&mut self, what: &str) -> Fail {
        self.fail_with(format_args!("out of memory for {what}"))
    }

    /// `n` items, each read by `each`, in the arena
    fn list<T: Copy>(&mut self, n: u32, what: &str, mut each: impl FnMut(&mut Self, u32) -> R<T>) -> R<&'m [T]> {
        self.list_of(n, what, &mut each)
    }

    /// The body of `list`, of which there is one for each type of item, not one for each `each`
    fn list_of<T: Copy>(&mut self, n: u32, what: &str, each: &mut dyn FnMut(&mut Self, u32) -> R<T>) -> R<&'m [T]> {
        let slots = self.slots::<T>(n, what)?;
        for (i, slot) in slots.iter_mut().enumerate() {
            slot.write(each(self, i as u32)?);
        }
        // SAFETY: every item is written
        Ok(unsafe { assume_init(slots) })
    }

    /// `n` items copied at once where all of them are there, or else read as `list` does
    fn items<T: Pod>(&mut self, n: u32, what: &str, mut each: impl FnMut(&mut Self, u32) -> R<T>) -> R<&'m [T]> {
        self.items_of(n, what, &mut each)
    }

    fn items_of<T: Pod>(&mut self, n: u32, what: &str, each: &mut dyn FnMut(&mut Self, u32) -> R<T>) -> R<&'m [T]> {
        let bytes = n as u64 * size_of::<T>() as u64;
        if bytes > self.left() as u64 {
            return self.list_of(n, what, each);
        }
        let slots = self.slots::<T>(n, what)?;
        // SAFETY: the bytes are in the data, and as T is Pod they are n valid items
        unsafe {
            let bytes = bytes as usize;
            ptr::copy_nonoverlapping(self.data.as_ptr().add(self.pos), slots.as_mut_ptr().cast::<u8>(), bytes);
            self.pos += bytes;
            Ok(assume_init(slots))
        }
    }

    /// A count, then that many strings. The context for one of them is `item` and its index,
    /// followed where `of` is set by "of" and the count.
    fn strings(&mut self, count_what: &str, item: &str, of: bool) -> R<&'m [&'m [u8]]> {
        let n = ctx!(self, self.u32(), "{count_what}");
        self.list(n, "strings", |r, i| match r.string() {
            Ok(s) => Ok(s),
            Err(Fail) if of => Err(r.context(format_args!("{item} {i} of {n}"))),
            Err(Fail) => Err(r.context(format_args!("{item} {i}"))),
        })
    }

    /// The id and name, with the "Version" and number that may come before the name
    fn short_section(&mut self) -> R<Section<'m>> {
        let id = ctx!(self, self.u32(), "failed to read material id");
        let mut name = ctx!(self, self.string(), "section header name");
        let mut version = 0;
        if name == b"Version" {
            version = ctx!(self, self.u32(), "version number");
            name = ctx!(self, self.string(), "texture short name");
        }
        Ok(Section { size: 0, id, version, name })
    }

    fn section(&mut self) -> R<Section<'m>> {
        let size = ctx!(self, self.u32(), "failed to read section size");
        Ok(Section { size, ..self.short_section()? })
    }

    // --- Materials ---

    fn material(&mut self) -> R<Material<'m>> {
        let section = ctx!(self, self.section(), "material section header");
        let filename = ctx!(self, self.string(), "texture filename");
        let opacity = ctx!(self, self.f32(), "opacity");
        let emissive_strength = ctx!(self, self.u32(), "emissive strength");
        let address_mode = match ctx!(self, self.u32(), "texture address mode") {
            0 => AddressMode::Opaque,
            1 => AddressMode::Wrap,
            3 => AddressMode::Clamp,
            e => return self.fail(format_args!("unknown texture mode address value: {e}")),
        };
        Ok(Material {
            section,
            filename,
            opacity,
            emissive_strength,
            address_mode,
            ambient: ctx!(self, self.color(), "ambient"),
            diffuse: ctx!(self, self.color(), "diffuse"),
            specular: ctx!(self, self.color(), "specular"),
            specular_level: ctx!(self, self.f32(), "specular level"),
            two_sided: ctx!(self, self.bool(), "two sided"),
        })
    }

    fn materials(&mut self) -> R<List<'m, Material<'m>>> {
        let section = ctx!(self, self.section(), "material list section header");
        let n = ctx!(self, self.u32(), "missing number of materials");
        let items = self.list(n, "materials", |r, i| Ok(ctx!(r, r.material(), "material section header {i}")))?;
        Ok(List { section, items })
    }

    // --- Geometry ---

    fn tris(&mut self, n: u32, what: &str) -> R<&'m [Tri]> {
        self.items(n, what, |r, i| {
            Ok(Tri {
                a: ctx!(r, r.u16(), "p1 of face index {i}"),
                b: ctx!(r, r.u16(), "p2 of face index {i}"),
                c: ctx!(r, r.u16(), "p3 of face index {i}"),
            })
        })
    }

    fn object_data(&mut self) -> R<ObjectData<'m>> {
        let material = ctx!(self, self.u32(), "material");

        // Faces: their planes, then their corners, then their texture corners
        let n = ctx!(self, self.u32(), "face count");
        let face_planes = self.items(n, "face planes", |r, i| Ok(ctx!(r, r.plane(), "face normal {i}")))?;
        let faces = self.tris(n, "faces")?;
        let face_texture_verts = self.tris(n, "face texture vertices")?;

        // Texture vertices: normals, then UVs, then colors
        let n = ctx!(self, self.u32(), "vertices count");
        let normals = self.items(n, "normals", |r, i| Ok(ctx!(r, r.xyz(), "normal coordinate {i} of {n}")))?;
        let uvs = self.items(n, "uvs", |r, i| Ok(ctx!(r, r.uv(), "UV texture coordinate {i} of {n}")))?;
        let colors = self.items(n, "colors", |r, i| Ok(ctx!(r, r.color(), "face color {i} of {n}")))?;
        Ok(ObjectData { material, face_planes, faces, face_texture_verts, normals, uvs, colors })
    }

    fn tag(&mut self) -> R<Tag> {
        Ok(Tag {
            coord1: [ctx!(self, self.u16(), "coord1 p1"), ctx!(self, self.u16(), "coord1 p2"), ctx!(self, self.u16(), "coord1 p3")],
            face_index_1: ctx!(self, self.u16(), "face index 1"),
            coord2: [ctx!(self, self.u16(), "coord2 p1"), ctx!(self, self.u16(), "coord2 p2"), ctx!(self, self.u16(), "coord2 p3")],
            face_index_2: ctx!(self, self.u16(), "face index 2"),
        })
    }

    fn collision_group(&mut self) -> R<CollisionGroup<'m>> {
        let name = ctx!(self, self.string(), "collision group name");
        let flags = ctx!(self, self.u32(), "collision group flags");
        let n = ctx!(self, self.u32(), "collision group indices count");
        let indices = self.items(n, "collision group indices", |r, _| r.u16())?;
        self.utf8(name)?;
        Ok(CollisionGroup { name, flags, indices })
    }

    fn object(&mut self) -> R<Object<'m>> {
        // An object has two section headers, for reasons unknown
        let section = ctx!(self, self.section(), "section header");
        let object_section = ctx!(self, self.section(), "object section header");

        let n = ctx!(self, self.u32(), "vertex count");
        let vertices = self.items(n, "vertices", |r, _| r.xyz())?;

        let n = ctx!(self, self.u32(), "objects data count");
        let datas = self.list(n, "object datas", |r, _| r.object_data())?;

        let n = ctx!(self, self.u32(), "collision vertices count");
        let collision_vertices = self.items(n, "collision vertices", |r, i| Ok(ctx!(r, r.xyz(), "collision vertex {i}")))?;
        let n = ctx!(self, self.u32(), "collision faces count");
        let collision_planes = self.items(n, "collision planes", |r, i| Ok(ctx!(r, r.plane(), "collision face normal {i}")))?;

        let n = ctx!(self, self.u32(), "object tag count");
        let tags = self.items(n, "tags", |r, _| r.tag())?;

        let n = ctx!(self, self.u32(), "collision group count");
        let groups = self.list(n, "collision groups", |r, i| Ok(ctx!(r, r.collision_group(), "collision group {i}")))?;
        Ok(Object { section, object_section, vertices, datas, collision_vertices, collision_planes, tags, groups })
    }

    fn geometries(&mut self) -> R<List<'m, Object<'m>>> {
        let section = ctx!(self, self.section(), "geometry list section header");
        let n = ctx!(self, self.u32(), "missing number of objects");
        let items = self.list(n, "objects", |r, _| r.object())?;
        Ok(List { section, items })
    }

    // --- Portals and lights ---

    fn portal(&mut self) -> R<Portal<'m>> {
        let section = ctx!(self, self.section(), "portal");
        let n = ctx!(self, self.u32(), "coordinates count");
        let vertices = self.items(n, "portal vertices", |r, i| Ok(ctx!(r, r.xyz(), "coordinate vertex {i}")))?;
        let room = ctx!(self, self.u32(), "room");
        let opposite_room = ctx!(self, self.u32(), "opposite room");
        Ok(Portal { section, vertices, room, opposite_room })
    }

    fn portals(&mut self) -> R<List<'m, Portal<'m>>> {
        let section = ctx!(self, self.section(), "portals");
        let n = ctx!(self, self.u32(), "portal count");
        let items = self.list(n, "portals", |r, i| Ok(ctx!(r, r.portal(), "portal {i}")))?;
        Ok(List { section, items })
    }

    /// No Rogue Spear map has any lights: only the count is read
    fn lights(&mut self) -> R<(Section<'m>, u32)> {
        let section = ctx!(self, self.section(), "lights");
        Ok((section, ctx!(self, self.u32(), "light count")))
    }

    // --- Dynamic objects ---

    fn dynamic_common(&mut self) -> R<DynamicCommon<'m>> {
        let transform = ctx!(self, self.transform(), "transformation matrix");
        let name = ctx!(self, self.string(), "name");
        let unknown1 = ctx!(self, self.u32(), "unknown1");
        let mut sounds: [&[u8]; 4] = [&[]; 4];
        for (i, sound) in sounds.iter_mut().enumerate() {
            *sound = ctx!(self, self.string(), "sound {i}");
        }
        Ok(DynamicCommon {
            transform,
            name,
            unknown1,
            sounds,
            collision_type_2d: ctx!(self, self.string(), "2D collision type"),
            collision_type_3d: ctx!(self, self.string(), "3D collision type"),
            destruction_action: ctx!(self, self.string(), "destruction action"),
            destruction_category: ctx!(self, self.string(), "destruction category"),
            penetration_type: ctx!(self, self.string(), "penetration type"),
            name2: ctx!(self, self.string(), "name2"),
            destruction_category2: ctx!(self, self.string(), "destruction category 2"),
        })
    }

    fn dynamic_param(&mut self) -> R<DynamicParam<'m>> {
        let name = ctx!(self, self.string(), "dynamic object kind struct name");
        let unknown1 = self.floats("dynamic object kind struct unknown1")?;
        let mut unknown2 = [0; 2];
        for (i, x) in unknown2.iter_mut().enumerate() {
            *x = ctx!(self, self.u32(), "dynamic object kind struct unknown2 {i}");
        }
        Ok(DynamicParam { name, unknown1, unknown2 })
    }

    fn dynamic(&mut self) -> R<DynamicKind<'m>> {
        let common = self.dynamic_common()?;
        let count = ctx!(self, self.u32(), "dynamic object kind count");
        let params = if count > 0 {
            DynamicParams::Struct(self.list(count, "dynamic params", |r, i| {
                Ok(ctx!(r, r.dynamic_param(), "dynamic object kind struct {i} of {count}"))
            })?)
        } else {
            // A second count, for the names
            let names = self.strings("dynamic object flat count", "dynamic object kind flat name", true)?;
            DynamicParams::Flat { names, unknown: self.floats("dynamic object kind flat unknown")? }
        };
        Ok(DynamicKind::Dynamic { common, params })
    }

    fn animation(&mut self) -> R<DynamicKind<'m>> {
        let common = self.dynamic_common()?;
        let unknown2 = ctx!(self, self.u32(), "unknown2");
        let names = self.strings("name count", "animation name", false)?;
        let unknown3 = self.floats("animation unknown3")?;
        Ok(DynamicKind::Animation {
            common,
            unknown2,
            names,
            unknown3,
            unknown4: ctx!(self, self.u32(), "animation unknown4"),
            name3: ctx!(self, self.string(), "animation name3"),
            name4: ctx!(self, self.string(), "animation name4"),
            animation_type: ctx!(self, self.string(), "animation type"),
            direction: ctx!(self, self.xyz(), "animation direction"),
            distance: ctx!(self, self.f32(), "animation distance"),
            velocity: ctx!(self, self.f32(), "animation velocity"),
        })
    }

    /// Often named "ADT", perhaps for "automatic door touchplate"
    fn repeatable_touchplate(&mut self) -> R<DynamicKind<'m>> {
        let common = self.dynamic_common()?;
        let unknown1 = ctx!(self, self.u32(), "ADT unknown1");
        let attachments = self.strings("ADT attachment count", "ADT attachment", false)?;
        let unknown2 = self.floats("ADT unknown2")?;
        let names = self.strings("name count", "ADT name", false)?;
        Ok(DynamicKind::RepeatableTouchplate {
            common,
            unknown1,
            attachments,
            unknown2,
            names,
            name2: ctx!(self, self.string(), "ADT name2"),
            name3: ctx!(self, self.string(), "ADT name3"),
            animation_type: ctx!(self, self.string(), "animation type"),
            direction: ctx!(self, self.xyz(), "animation direction"),
            distance: ctx!(self, self.f32(), "animation distance"),
            velocity: ctx!(self, self.f32(), "animation velocity"),
        })
    }

    fn glass(&mut self) -> R<DynamicKind<'m>> {
        let name = self.string()?;
        self.utf8(name)?;
        Ok(DynamicKind::Glass { name })
    }

    fn six(&mut self) -> R<[f32; 6]> {
        let (a, b) = (self.xyz()?, self.xyz()?);
        Ok([a.x, a.y, a.z, b.x, b.y, b.z])
    }

    fn one_time_touchplate(&mut self) -> R<DynamicKind<'m>> {
        let collision_type_2d = ctx!(self, self.string(), "one-time touchplate 2D collision type");
        let collision_type_3d = ctx!(self, self.string(), "one-time touchplate 3D collision type");
        let coordinates = ctx!(self, self.six(), "one-time touchplate coordinates");
        let attachments = self.strings("one-time touchplate attachment count", "one-time touchplate attachment name", true)?;
        Ok(DynamicKind::OneTimeTouchplate { collision_type_2d, collision_type_3d, coordinates, attachments })
    }

    fn eight(&mut self) -> R<[f32; 8]> {
        let six = self.six()?;
        let (x, y) = (ctx!(self, self.f32(), "x"), ctx!(self, self.f32(), "y"));
        Ok([six[0], six[1], six[2], six[3], six[4], six[5], x, y])
    }

    fn halo(&mut self) -> R<DynamicKind<'m>> {
        let n = ctx!(self, self.u32(), "halo count");
        Ok(DynamicKind::Halo(self.list(n, "halos", |r, i| {
            Ok(Halo {
                name: ctx!(r, r.string(), "halo name {i} of {n}"),
                values: ctx!(r, r.eight(), "halo vec {i} of {n}"),
            })
        })?))
    }

    fn dynamic_object(&mut self) -> R<DynamicObject<'m>> {
        let section = ctx!(self, self.section(), "dynamic object section header");
        let name = ctx!(self, self.string(), "name");
        let transform = ctx!(self, self.transform(), "transformation matrix");
        let kind = match section.id {
            14 => self.dynamic()?,
            15 => self.animation()?,
            16 => self.repeatable_touchplate()?,
            20 => self.glass()?,
            25 => self.one_time_touchplate()?,
            31 => self.halo()?,
            36 => DynamicKind::StaticEffect,
            id => return self.fail(format_args!("unhandled id value: {id}")),
        };
        Ok(DynamicObject { section, name, transform, kind })
    }

    fn dynamic_objects(&mut self) -> R<List<'m, DynamicObject<'m>>> {
        let section = ctx!(self, self.section(), "dynamic objects section header");
        let n = ctx!(self, self.u32(), "dynamic object count");
        let items = self.list(n, "dynamic objects", |r, i| Ok(ctx!(r, r.dynamic_object(), "dynamic object {i} of {n}")))?;
        Ok(List { section, items })
    }

    // --- Rooms ---

    fn level_box(&mut self) -> R<LevelBox> {
        let transform = ctx!(self, self.transform(), "TM + AABB");
        Ok(LevelBox { transform, aabb: self.floats("level TM + AABB side")? })
    }

    fn level(&mut self) -> R<Level<'m>> {
        let name = ctx!(self, self.string(), "level name");
        let n = ctx!(self, self.u32(), "level TM + AABB count");
        let boxes = self.items(n, "level boxes", |r, i| Ok(ctx!(r, r.level_box(), "level TM + AABBB {i}")))?;
        let n = ctx!(self, self.u32(), "level floor count");
        let floors = self.items(n, "level floors", |r, i| Ok(ctx!(r, r.f32(), "level floor {i}")))?;
        if ctx!(self, self.u8(), "level has model") != 0 {
            return self.fail("a level with a model, which is not read");
        }
        Ok(Level { name, boxes, floors })
    }

    fn room_floor(&mut self) -> R<RoomFloor> {
        Ok(RoomFloor { number: ctx!(self, self.f32(), "number"), height: ctx!(self, self.f32(), "height") })
    }

    fn room(&mut self) -> R<Room<'m>> {
        let section = ctx!(self, self.short_section(), "room section header short");
        let enclosed = ctx!(self, self.u8(), "room enclosed");
        let mut small = None;
        if enclosed == 0 {
            let s = ctx!(self, self.u8(), "room small");
            if s > 1 {
                return self.fail(format_args!("unknown room small value: {s}"));
            }
            small = Some(s);
        } else if enclosed > 1 {
            return self.fail(format_args!("unknown room enclosed value: {enclosed}"));
        }
        let unknown_flag = ctx!(self, self.u8(), "room unknown flag");
        let (mut weather_flag, mut weather_bounds) = (None, None);
        if section.version > 1 {
            let flag = ctx!(self, self.u8(), "room has weather bounds");
            weather_flag = Some(flag);
            if flag != 0 {
                let min = ctx!(self, self.xyz(), "room weather bounds min");
                let max = ctx!(self, self.xyz(), "room weather bounds max");
                weather_bounds = Some([min, max]);
            }
        }

        let n = ctx!(self, self.u32(), "room level count");
        let levels = self.list(n, "levels", |r, i| Ok(ctx!(r, r.level(), "room sherman level {i} of {n}")))?;
        let n = ctx!(self, self.u32(), "room floor count");
        let floors = self.items(n, "room floors", |r, i| Ok(ctx!(r, r.room_floor(), "room floor {i} of {n}")))?;
        let n = ctx!(self, self.u32(), "room unknown count");
        let unknown = self.items(n, "room unknowns", |r, i| Ok(ctx!(r, r.u32(), "room unknown {i}")))?;
        Ok(Room { section, enclosed, small, unknown_flag, weather_flag, weather_bounds, levels, floors, unknown })
    }

    fn rooms(&mut self) -> R<List<'m, Room<'m>>> {
        let section = ctx!(self, self.section(), "room list");
        let n = ctx!(self, self.u32(), "room count");
        let items = self.list(n, "rooms", |r, i| Ok(ctx!(r, r.room(), "room {i}")))?;
        Ok(List { section, items })
    }

    // --- Transitions and planning levels ---

    fn transition(&mut self) -> R<Transition<'m>> {
        Ok(Transition {
            name: ctx!(self, self.string(), "transition"),
            p1: ctx!(self, self.xyz(), "transition coords P1"),
            p2: ctx!(self, self.xyz(), "transition coords P2"),
        })
    }

    fn transitions(&mut self) -> R<List<'m, Transition<'m>>> {
        let section = ctx!(self, self.section(), "transitions");
        let n = ctx!(self, self.u32(), "transitions count");
        let items = self.list(n, "transitions", |r, i| Ok(ctx!(r, r.transition(), "transition {i} of {n}")))?;
        Ok(List { section, items })
    }

    fn planning_level(&mut self) -> R<PlanningLevel<'m>> {
        let level_number = ctx!(self, self.f32(), "planning level number");
        let floor_height = ctx!(self, self.f32(), "planning level floor height");
        let room_names = self.strings("planning level room count", "planning level room name", true)?;
        Ok(PlanningLevel { level_number, floor_height, room_names })
    }

    fn planning_levels(&mut self) -> R<List<'m, PlanningLevel<'m>>> {
        let section = ctx!(self, self.section(), "planning levels");
        let n = ctx!(self, self.u32(), "planning levels count");
        let items = self.list(n, "planning levels", |r, i| Ok(ctx!(r, r.planning_level(), "planning level {i} of {n}")))?;
        Ok(List { section, items })
    }

    // --- The file ---

    fn magic(&mut self, what: &str, want: &[u8], bad: &str) -> R<()> {
        if ctx!(self, self.string(), "{what}") != want {
            return self.fail(bad);
        }
        Ok(())
    }

    fn header(&mut self) -> R<u32> {
        self.magic("missing magic", b"BeginMapv2.1", "incorrect magic")?;
        Ok(ctx!(self, self.u32(), "failed to read file creation timestamp"))
    }

    fn map(&mut self) -> R<Map<'m>> {
        let m = Map {
            timestamp: ctx!(self, self.header(), "MAP Header"),
            materials: ctx!(self, self.materials(), "Materials List"),
            geometries: ctx!(self, self.geometries(), "Geometry List"),
            portals: ctx!(self, self.portals(), "Portal List"),
            lights: ctx!(self, self.lights(), "Light List"),
            dynamic_objects: ctx!(self, self.dynamic_objects(), "Dynamic Object List"),
            rooms: ctx!(self, self.rooms(), "Rooms List"),
            transitions: ctx!(self, self.transitions(), "Transition List"),
            planning_levels: ctx!(self, self.planning_levels(), "Planning Levels List"),
        };
        // Anything after the end is left alone
        self.magic("end", b"EndMap", "missing MAP end")?;
        Ok(m)
    }
}

/// Parse a .MAP file already in memory into the arena. A map that fails to read leaves at
/// most its message in the arena.
pub fn parse<'m>(data: &[u8], arena: &mut Arena<'m>) -> Result<Map<'m>, &'m str> {
    let saved = arena.beg;
    let mut r = Reader { data, pos: 0, arena, err: Msg::new() };
    match r.map() {
        Ok(m) => Ok(m),
        Err(Fail) => {
            r.arena.beg = saved;
            let msg = r.err.as_bytes();
            let Some(s) = r.arena.alloc::<u8>(msg.len()) else { return Err("out of memory") };
            // SAFETY: every byte is written
            let s = unsafe {
                ptr::copy_nonoverlapping(msg.as_ptr(), s.as_mut_ptr().cast(), msg.len());
                assume_init(s)
            };
            Err(core::str::from_utf8(s).unwrap_or("invalid message"))
        }
    }
}

unsafe extern "C" {
    fn fopen(path: *const c_char, mode: *const c_char) -> *mut c_void;
    fn fseek(file: *mut c_void, offset: c_long, whence: c_int) -> c_int;
    fn ftell(file: *mut c_void) -> c_long;
    fn fread(buf: *mut c_void, size: usize, count: usize, file: *mut c_void) -> usize;
    fn fclose(file: *mut c_void) -> c_int;
}

/// Read a .MAP file into the arena, which also holds the file's bytes while it is read
pub fn read<'m>(path: &CStr, arena: &mut Arena<'m>) -> Result<Map<'m>, &'m str> {
    const SEEK_SET: c_int = 0;
    const SEEK_END: c_int = 2;
    // SAFETY: the file is open until it is closed, and the top `size` bytes are the arena's,
    // which nothing else is handed while the file is read and parsed
    unsafe {
        let f = fopen(path.as_ptr(), c"rb".as_ptr());
        if f.is_null() {
            return Err("could not open MAP file");
        }
        let size = if fseek(f, 0, SEEK_END) != 0 { -1 } else { ftell(f) };
        if size < 0 || fseek(f, 0, SEEK_SET) != 0 {
            fclose(f);
            return Err("failed to read MAP file");
        }
        let size = size as usize;
        if size > arena.left() {
            fclose(f);
            return Err("MAP file is larger than the memory left");
        }

        // The file comes off the top, so it is not kept when the map is
        let top = arena.end.sub(size);
        let got = fread(top.cast(), 1, size, f);
        fclose(f);
        if got != size {
            return Err("failed to read MAP file");
        }
        let data = core::slice::from_raw_parts(top, size);
        let mut scratch = Arena { beg: arena.beg, end: top, _mem: PhantomData };
        let result = parse(data, &mut scratch);
        arena.beg = scratch.beg;
        result
    }
}
