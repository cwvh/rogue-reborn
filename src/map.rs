use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use byteorder::{LE, ReadBytesExt};

const MAGIC: &[u8] = b"BeginMapv2.1";
const END: &[u8] = b"EndMap";

/// RSE "MAP" format reader that is known to work with Rogue Spear, Urban Ops
/// and Covert Ops maps. Each section of the MAP format is represented with its
/// own structure that knows how to parse itself and its children completely.
///
/// The `Map` type does not represent the on-disk format exactly. Where the
/// on-disk format has list lengths, the in-memory `Map` type uses `Vec<T>` and
/// omits the explicit length.
#[derive(Clone, Debug)]
pub struct Map {
    pub header: MapHeader,
    pub materials: Materials,
    pub geometries: Geometries,
    pub portals: Portals,
    pub lights: Lights,
    pub dynamic_objects: DynamicObjects,
    pub rooms: Rooms,
    pub transitions: Transitions,
    pub planning_levels: PlanningLevels,
}

impl Map {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let header = MapHeader::read(buf).context("MAP Header")?;
        let materials = Materials::read(buf).context("Materials List")?;
        let geometries = Geometries::read(buf).context("Geometry List")?;
        let portals = Portals::read(buf).context("Portal List")?;
        let lights = Lights::read(buf).context("Light List")?;
        let dynamic_objects = DynamicObjects::read(buf)
            .context("Dynamic Object List")?;
        let rooms = Rooms::read(buf).context("Rooms List")?;
        let transitions = Transitions::read(buf).context("Transition List")?;
        let planning_levels = PlanningLevels::read(buf)
            .context("Planning Levels List")?;

        ensure!(buf.read_cstring().context("end")? == END, "missing MAP end");

        Ok(Self {
            header,
            materials,
            geometries,
            portals,
            lights,
            dynamic_objects,
            rooms,
            transitions,
            planning_levels,
        })
    }
}

// TODO replace or remove
/// Utility function for driving the IO. Should be replaced or moved.
pub fn read(filename: &Path) -> anyhow::Result<Map> {
    let file = File::open(filename).context("could not open MAP file")?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).context("failed to read MAP file")?;
    let mut buf = Cursor::new(buf);
    Map::read(&mut buf)
}

#[derive(Clone, Debug)]
pub struct MapHeader {
    /// Unix timestamp of when the MAP file was created
    pub timestamp: u32,
}

impl MapHeader {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let magic = buf.read_cstring().context("missing magic")?;
        if magic != MAGIC {
            anyhow::bail!("incorrect magic: '{magic:?}'");
        }

        let timestamp = buf.read_u32::<LE>()
            .context("failed to read file creation timestamp")?;

        Ok(Self { timestamp })
    }
}

/// List of all `Material`s for the level
#[derive(Clone, Debug)]
pub struct Materials {
    pub id: u32,
    pub materials: Vec<Material>,
}

impl Materials {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (id, _material_list) = section_header(buf)
            .context("material list section header")?;

        let n = buf.read_u32::<LE>().context("missing number of materials")?;
        let mut materials = Vec::with_capacity(n as usize);
        for i in 0..n {
            materials.push(Material::read(buf)
                .with_context(|| format!("material section header {i}"))?);
        }

        Ok(Self {
            id,
            materials,
        })

    }
}

/// Texture material reference and rendering parameters
#[derive(Clone, Debug)]
pub struct Material {
    pub id: u32,
    pub filename: String,
    pub name: String,
    pub opacity: f32,
    pub emissive_strength: u32,
    pub address_mode: TextureAddressMode,
    pub ambient: Color4f,
    pub diffuse: Color4f,
    pub specular: Color4f,
    pub specular_level: f32,
    pub two_sided: bool,
}

impl Material {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (id, name) = section_header(buf)
            .context("material section header")?;

        let filename = buf.read_cstring().context("texture filename")?;

        let opacity = buf.read_f32::<LE>().context("opacity")?;
        let emissive_strength = buf.read_u32::<LE>()
            .context("emissive strength")?;

        let address_mode = TextureAddressMode::read(buf)?;

        let ambient = Color4f::read(buf).context("ambient")?;
        let diffuse = Color4f::read(buf).context("diffuse")?;
        let specular = Color4f::read(buf).context("specular")?;
        let specular_level = buf.read_f32::<LE>().context("specular level")?;
        let two_sided = buf.read_bool().context("two sided")?;

        Ok(Self {
            id,
            filename: latin1_to_utf8(&filename),
            name,
            opacity,
            emissive_strength,
            address_mode,
            ambient,
            diffuse,
            specular,
            specular_level,
            two_sided,
        })
    }
}

#[derive(Clone, Debug)]
pub enum TextureAddressMode {
    Opaque,
    Wrap,
    Clamp,
}

impl TextureAddressMode {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let address_mode = buf.read_u32::<LE>()
            .context("texture address mode")?;
        Ok(match address_mode {
            0 => Self::Opaque,
            1 => Self::Wrap,
            3 => Self::Clamp,
            e => bail!("unknown texture mode address value: {e}"),
        })
    }
}


#[derive(Clone, Debug)]
pub struct Color4f {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color4f {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let r = buf.read_f32::<LE>().context("red")?;
        let g = buf.read_f32::<LE>().context("green")?;
        let b = buf.read_f32::<LE>().context("blue")?;
        let a = buf.read_f32::<LE>().context("alpha")?;
        Ok(Self { r, g, b, a })
    }
}

#[derive(Clone, Debug)]
pub struct Geometries {
    pub id: u32,
    pub objects: Vec<Object>,
}

impl Geometries {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (id, _material_list) = section_header(buf)
            .context("geometry list section header")?;

        let n = buf.read_u32::<LE>().context("missing number of objects")?;
        let mut objects = Vec::with_capacity(n as usize);
        for _ in 0..n {
            objects.push(Object::read(buf)?);
        }

        Ok(Self {
            id,
            objects,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Object {
    pub id: u32,
    pub name: String,
    pub object_id: u32,
    pub object_name: String,
    pub vertices: Vec<Vertex>,
    // TODO: bad name
    pub object_datas: Vec<ObjectData>,
    pub collisions: Collisions,
    pub tags: Vec<Tag>,
    pub ind: Vec<EIndices>,
}

impl Object {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (id, name) = section_header(buf)
            .context("section header")?;

        // Not sure why there are two section headers for Objects
        let (object_id, object_name) = section_header(buf)
            .context("object section header")?;

        let n = buf.read_u32::<LE>().context("vertex count")?;
        let mut vertices = Vec::with_capacity(n as usize);
        for _ in 0..n {
            vertices.push(Vertex::read(buf)?);
        }

        let n = buf.read_u32::<LE>().context("objects data count")?;
        let mut object_datas = Vec::with_capacity(n as usize);
        for _ in 0..n {
            object_datas.push(ObjectData::read(buf)?);
        }

        let collisions = Collisions::read(buf)?;

        let n = buf.read_u32::<LE>().context("object tag count")?;
        let mut tags = Vec::with_capacity(n as usize);
        for _ in 0..n {
            tags.push(Tag::read(buf)?);
        }

        // TODO: these field names are wonky
        let ff_count = buf.read_u32::<LE>().context("FF count")?;
        let mut ind = Vec::with_capacity(ff_count as usize);
        for i in 0..ff_count {
            let index = EIndices::read(buf)
                .with_context(|| format!("EIndices {i}"))?;
            ind.push(index);
        }

        Ok(Self {
            id,
            name,
            object_id,
            object_name,
            vertices,
            object_datas,
            collisions,
            tags,
            ind,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Vertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vertex {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (x, y, z) = buf.read_f32_xyz()?;
        Ok(Self { x, y, z })
    }
}

#[derive(Clone, Debug)]
pub struct ObjectData {
    // TODO: what is this?
    pub mn: u32,
    pub faces: Faces,
    pub texture_vertices: TextureVertices,
}

impl ObjectData {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let mn = buf.read_u32::<LE>().context("MN")?;
        let faces = Faces::read(buf)?;
        let texture_vertices = TextureVertices::read(buf)?;
        Ok(Self {
            mn,
            faces,
            texture_vertices,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Faces {
    pub normals: Vec<FaceNormal>,
    pub face_indices: Vec<(u16, u16, u16)>,
    pub texture_indices: Vec<(u16, u16, u16)>,
}

impl Faces {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let n = buf.read_u32::<LE>().context("face count")? as usize;

        let mut normals = Vec::with_capacity(n);
        for i in 0..n {
            let normal = FaceNormal::read(buf)
                .with_context(|| format!("face normal {i}"))?;
            normals.push(normal);
        }

        let mut face_indices = Vec::with_capacity(n);
        for i in 0..n {
            let p1 = buf.read_u16::<LE>()
                .with_context(|| format!("p1 of face index {i}"))?;
            let p2 = buf.read_u16::<LE>()
                .with_context(|| format!("p2 of face index {i}"))?;
            let p3 = buf.read_u16::<LE>()
                .with_context(|| format!("p3 of face index {i}"))?;
            face_indices.push((p1, p2, p3));
        }

        let mut texture_indices = Vec::with_capacity(n);
        for i in 0..n {
            let p1 = buf.read_u16::<LE>()
                .with_context(|| format!("p1 of face index {i}"))?;
            let p2 = buf.read_u16::<LE>()
                .with_context(|| format!("p2 of face index {i}"))?;
            let p3 = buf.read_u16::<LE>()
                .with_context(|| format!("p3 of face index {i}"))?;
            texture_indices.push((p1, p2, p3))
        }

        Ok(Self {
            normals,
            face_indices,
            texture_indices,
        })
    }
}

#[derive(Clone, Debug)]
pub struct FaceNormal {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Distance from the origin to the face where sign is normal direction
    pub distance_origin_to_face: f32,
}

impl FaceNormal {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (x, y, z) = buf.read_f32_xyz()?;
        let dist = buf.read_f32::<LE>().context("distance origin to face")?;
        Ok(Self { x, y, z, distance_origin_to_face: dist })
    }
}


#[derive(Clone, Debug)]
pub struct TextureVertices {
    pub normals: Vec<NormalCoord>,
    pub uv_coords: Vec<UvCoord>,
    pub face_colors: Vec<Color4f>,
}

impl TextureVertices {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let n = buf.read_u32::<LE>().context("vertices count")? as usize;

        let mut normals = Vec::with_capacity(n);
        for i in 0..n {
            normals.push(NormalCoord::read(buf)
                .with_context(|| format!("normal coordinate {i} of {n}"))?);
        }

        let mut uv_coords = Vec::with_capacity(n);
        for i in 0..n {
            uv_coords.push(UvCoord::read(buf)
                .with_context(|| format!("UV texture coordinate {i} of {n}"))?);
        }

        let mut face_colors = Vec::with_capacity(n);
        for i in 0..n {
            face_colors.push(Color4f::read(buf)
                .with_context(|| format!("face color {i} of {n}"))?);
        }

        Ok(Self {
            normals,
            uv_coords,
            face_colors,
        })
    }
}

#[derive(Clone, Debug)]
pub struct NormalCoord {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl NormalCoord {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (x, y, z) = buf.read_f32_xyz()?;
        Ok(Self { x, y, z })
    }
}

#[derive(Clone, Debug)]
pub struct UvCoord {
    pub u: f32,
    pub v: f32,
}

impl UvCoord {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let u = buf.read_f32::<LE>().context("u")?;
        let v = buf.read_f32::<LE>().context("v")?;
        Ok(Self { u, v })
    }
}

#[derive(Clone, Debug)]
pub struct Collisions {
    pub vertices: Vec<Vertex>,
    pub faces: Vec<FaceNormal>,
}

impl Collisions {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let n = buf.read_u32::<LE>().context("collision vertices count")?;
        let mut vertices = Vec::with_capacity(n as usize);
        for i in 0..n {
            vertices.push(Vertex::read(buf)
                .with_context(|| format!("collision vertex {i}"))?);
        }

        let n = buf.read_u32::<LE>().context("collision faces count")?;
        let mut faces = Vec::with_capacity(n as usize);
        for i in 0..n {
            faces.push(FaceNormal::read(buf)
                .with_context(|| format!("collision face normal {i}"))?);
        }

        Ok(Self { vertices, faces })
    }
}

#[derive(Clone, Debug)]
pub struct Tag {
    pub coord1: (u16, u16, u16),
    pub face_index_1: u16,
    pub coord2: (u16, u16, u16),
    pub face_index_2: u16,
}

impl Tag {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let p11 = buf.read_u16::<LE>().context("coord1 p1")?;
        let p21 = buf.read_u16::<LE>().context("coord1 p2")?;
        let p31 = buf.read_u16::<LE>().context("coord1 p3")?;
        let face_index_1 = buf.read_u16::<LE>().context("face index 1")?;

        let p12 = buf.read_u16::<LE>().context("coord2 p1")?;
        let p22 = buf.read_u16::<LE>().context("coord2 p2")?;
        let p32 = buf.read_u16::<LE>().context("coord2 p3")?;
        let face_index_2 = buf.read_u16::<LE>().context("face index 2")?;

        Ok(Self {
            coord1: (p11, p21, p31),
            face_index_1,
            coord2: (p12, p22, p32),
            face_index_2,
        })
    }
}

#[derive(Clone, Debug)]
pub struct EIndices {
    // TODO: what is this?
    pub text: String,
    pub mn: u32,
    pub indices: Vec<u16>,
}

impl EIndices {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let text = buf.read_cstring().context("EIndex text")?;
        let mn = buf.read_u32::<LE>().context("EIndex MN")?;
        let n = buf.read_u32::<LE>().context("EIndex indices count")?;
        let mut indices = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let p1 = buf.read_u16::<LE>()?;
            indices.push(p1);
        }
        Ok(Self {
            text: String::from_utf8(text)?,
            mn,
            indices,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Portals {
    pub id: u32,
    pub name: String,
    pub portals: Vec<Portal>,
}

impl Portals {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (id, name) = section_header(buf).context("portals")?;
        let n = buf.read_u32::<LE>().context("portal count")?;
        let mut portals = Vec::with_capacity(n as usize);
        for i in 0..n {
            portals.push(Portal::read(buf)
                .with_context(|| format!("portal {i}"))?);
        }
        Ok(Self {
            id,
            name,
            portals,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Portal {
    pub id: u32,
    pub name: String,
    pub coordinates: Vec<Vertex>,
    pub room: u32,
    pub opposite_room: u32,
}

impl Portal {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (id, name) = section_header(buf).context("portal")?;
        let n = buf.read_u32::<LE>().context("coordinates count")?;
        let mut coordinates = Vec::with_capacity(n as usize);
        for i in 0..n {
            coordinates.push(Vertex::read(buf)
                .with_context(|| format!("coordinate vertex {i}"))?);
        }
        let room = buf.read_u32::<LE>().context("room")?;
        let opposite_room = buf.read_u32::<LE>().context("opposite room")?;
        Ok(Self {
            id,
            name,
            coordinates,
            room,
            opposite_room,
        })
    }
}

// TODO: light count is zero for every RS map I tested. I think lights are
// in the DMP files for RS.
#[derive(Clone, Debug)]
pub struct Lights {
    pub id: u32,
    pub name: String,
    // TODO: 100+ maps across all version=1 games had no light lists. Delete?
    pub light_count: u32,
}

impl Lights {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (id, name) = section_header(buf).context("lights")?;
        let n = buf.read_u32::<LE>().context("light count")?;
        Ok(Self { id, name, light_count: n })
    }
}

#[derive(Clone, Debug)]
pub struct DynamicObjects {
    pub id: u32,
    pub name: String,
    pub dynamic_objects: Vec<DynamicObject>,
}

impl DynamicObjects {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (id, name) = section_header(buf)
            .context("dynamic objects section header")?;
        let n = buf.read_u32::<LE>().context("dynamic object count")?;
        let mut dynamic_objects = Vec::with_capacity(n as usize);
        for i in 0..n {
            dynamic_objects.push(DynamicObject::read(buf)
                .with_context(|| format!("dynamic object {i} of {n}"))?);
        }
        Ok(Self { id, name, dynamic_objects })
    }
}

#[derive(Clone, Debug)]
pub struct DynamicObject {
    // TODO: if keeping the section header, then make a `SectionHeader` type
    // that holds the version and any additional names.
    pub section_id: u32,
    pub section_name: String,

    // TODO: All strings should be latin1 encoded
    pub name: String,
    pub tm: TransformationMatrix,

    /// The kind of dynamic object this is: television; billboard; smoke, etc.,
    /// with object parameters like sounds, collisions, etc.
    pub kind: DynamicObjectKind,
}

/// Mappings of section header ID to the object type. This list is
/// non-exhaustive at the moment and only used in `DynamicObject`.
#[derive(Clone, Copy, Debug)]
pub enum Id {
    /// An object with dynamic properties, such as televisions
    Dynamic = 14,

    /// An object with an attached animation
    Animation = 15,

    /// A door or automatic door that the player can interact with more than
    /// once.
    RepeatableTouchplate = 16,

    /// Breakable glass
    Glass = 20,

    /// A one-time interaction, such as some doors that open once
    OneTimeTouchplate = 25,

    /// Halo
    Halo = 31,

    /// Static world effects like manhole steam and smoke stacks
    StaticEffect = 36,
}

impl std::convert::TryFrom<u32> for Id {
    type Error = anyhow::Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(match value {
            x if x == Self::Dynamic as u32 => Self::Dynamic,
            x if x == Self::Animation as u32 => Self::Animation,
            x if x == Self::RepeatableTouchplate as u32 => {
                Self::RepeatableTouchplate
            },
            x if x == Self::Glass as u32 => Self::Glass,
            x if x == Self::OneTimeTouchplate as u32 => Self::OneTimeTouchplate,
            x if x == Self::Halo as u32 => Self::Halo,
            x if x == Self::StaticEffect as u32 => Self::StaticEffect,
            e => bail!("unhandled id value: {e}"),
        })
    }
}

impl DynamicObject {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (section_id, section_name) = section_header(buf)
            .context("dynamic object section header")?;
        let name = buf.read_cstring().context("name")?;
        let tm = TransformationMatrix::read(buf)
            .context("transformation matrix")?;

        let kind_id = section_id.try_into()?;
        let kind = DynamicObjectKind::read(kind_id, buf)?;

        Ok(Self {
            section_id,
            section_name,
            name: latin1_to_utf8(&name),
            tm,
            kind,
        })
    }
}

#[derive(Clone, Debug)]
pub struct TransformationMatrix {
    pub x_axis: Vec3f,
    pub y_axis: Vec3f,
    pub z_axis: Vec3f,
    pub position: Vec3f,
}

impl TransformationMatrix {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let x_axis = Vec3f::read(buf).context("transformation matrix x-axis")?;
        let y_axis = Vec3f::read(buf).context("transformation matrix y-axis")?;
        let z_axis = Vec3f::read(buf).context("transformation matrix z-axis")?;
        let position = Vec3f::read(buf)
            .context("transformation matrix position")?;
        Ok(Self { x_axis, y_axis, z_axis, position })
    }
}

#[derive(Clone, Debug)]
pub struct Vec3f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3f {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (x, y, z) = buf.read_f32_xyz()?;
        Ok(Self { x, y, z })
    }
}

#[derive(Clone, Debug)]
pub struct Vec6f {
    pub x1: f32,
    pub y1: f32,
    pub z1: f32,
    pub x2: f32,
    pub y2: f32,
    pub z2: f32,
}

impl Vec6f {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (x1, y1, z1) = buf.read_f32_xyz()?;
        let (x2, y2, z2) = buf.read_f32_xyz()?;
        Ok(Self { x1, y1, z1, x2, y2, z2 })
    }
}

// TODO: I don't think we really have an 8-D vector in the game. Rename this
// once I figure out what a 6-D vector with an additional X+Y is.
#[derive(Clone, Debug)]
pub struct Vec8f {
    pub x1: f32,
    pub y1: f32,
    pub z1: f32,
    pub x2: f32,
    pub y2: f32,
    pub z2: f32,
    pub x3: f32,
    pub y3: f32,
}

impl Vec8f {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (x1, y1, z1) = buf.read_f32_xyz()?;
        let (x2, y2, z2) = buf.read_f32_xyz()?;
        let (x3, y3) = buf.read_f32_xy()?;
        Ok(Self { x1, y1, z1, x2, y2, z2, x3, y3 })
    }
}


/// When the dynamic object section header is value 14
#[derive(Clone, Debug)]
pub enum KindDynamicParams {
    // TODO: bad names; do better

    /// Count field is greater than zero
    Struct(Vec<KindDynamicParamStruct>),

    /// Count field is zero
    Flat {
        names: Vec<String>,
        // TODO: something about duration or color?
        unknown: [f32; 4],
    },
}

impl KindDynamicParams {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let count = buf.read_u32::<LE>().context("dynamic object kind count")?;
        Ok(if count > 0 {
            let mut structs = Vec::with_capacity(count as usize);
            for i in 0..count {
                structs.push(KindDynamicParamStruct::read(buf).with_context(|| {
                    format!("dynamic object kind struct {i} of {count}")
                })?);
            }
            Self::Struct(structs)
        } else {
            // Yes, another count that shadows the previous one. MAP quirk.
            let count = buf.read_u32::<LE>()
                .context("dynamic object flat count")?;
            let mut names = Vec::with_capacity(count as usize);
            for i in 0..count {
                let cstring = buf.read_cstring().with_context(|| {
                    format!("dynamic object kind flat name {i} of {count}")
                })?;
                let latin1 = latin1_to_utf8(&cstring);
                names.push(latin1);
            }

            let mut unknowns = vec![0f32; 4];
            for (i, unknown) in unknowns.iter_mut().enumerate() {
                *unknown = buf.read_f32::<LE>().with_context(|| {
                    format!("dynamic object kind flat unknown {i}")
                })?;
            }
            let unknown = unknowns.try_into()
                .expect("dynamic object kind flat unknown is 4 elements");

            Self::Flat { names, unknown }
        })
    }
}

#[derive(Clone, Debug)]
pub struct KindDynamicParamStruct {
    pub name: String,
    // TODO: translation matrix?
    pub unknown1: [f32; 9],
    pub unknown2: [u32; 2],
}

impl KindDynamicParamStruct {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let name = latin1_to_utf8(&buf.read_cstring()
            .context("dynamic object kind struct name")?);

        let mut unknown1 = vec![0f32; 9];
        for (i, x) in unknown1.iter_mut().enumerate() {
            *x = buf.read_f32::<LE>().with_context(|| {
                format!("dynamic object kind struct unknown1 {i}")
            })?;
        }
        let unknown1 = unknown1.try_into()
            .expect("dynamic object kind struct unknown1 is 9 elements");

        let mut unknown2 = vec![0u32; 2];
        for (i, x) in unknown2.iter_mut().enumerate() {
            *x = buf.read_u32::<LE>().with_context(|| {
                format!("dynamic object kind struct unknown2 {i}")
            })?;
        }
        let unknown2 = unknown2.try_into()
            .expect("dynamic object kind struct unknown2 is 2 elements");

        Ok(Self { name, unknown1, unknown2 })
    }
}

#[derive(Clone, Debug)]
pub struct DynamicObjectKindCommon {
    pub tm: TransformationMatrix,
    pub name: String,
    // TODO: Always zero? zero is documented
    pub unknown1: u32,
    pub sounds: [String; 4],
    // TODO: convert these strings to enums once all variants are known
    pub collision_type_2d: String,
    pub collision_type_3d: String,
    pub destruction_action: String,
    pub destruction_category: String,
    pub penetration_type: String,
    pub name2: String,
    pub destruction_category2: String,
}

impl DynamicObjectKindCommon {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let tm = TransformationMatrix::read(buf)
            .context("transformation matrix")?;
        let name = buf.read_cstring().context("name")?;
        let unknown1 = buf.read_u32::<LE>().context("unknown1")?;

        // TODO: clean up this array processing
        let mut sounds = vec![String::new(); 4];
        for (i, sound) in sounds.iter_mut().enumerate() {
            let latin1 = buf.read_cstring()
                .with_context(|| format!("sound {i}"))?;
            *sound = latin1_to_utf8(&latin1);
        }
        let sounds = sounds.try_into().expect("sounds array is 4 elements");

        let collision_type_2d = buf.read_cstring()
            .context("2D collision type")?;
        let collision_type_3d = buf.read_cstring()
            .context("3D collision type")?;
        let destruction_action = buf.read_cstring()
            .context("destruction action")?;
        let destruction_category = buf.read_cstring()
            .context("destruction category")?;
        let penetration_type = buf.read_cstring().context("penetration type")?;
        let name2 = buf.read_cstring().context("name2")?;
        let destruction_category2 = buf.read_cstring()
            .context("destruction category 2")?;

        Ok(Self {
            tm,
            name: latin1_to_utf8(&name),
            unknown1,
            sounds,
            collision_type_2d: latin1_to_utf8(&collision_type_2d),
            collision_type_3d: latin1_to_utf8(&collision_type_3d),
            destruction_action: latin1_to_utf8(&destruction_action),
            destruction_category: latin1_to_utf8(&destruction_category),
            penetration_type: latin1_to_utf8(&penetration_type),
            name2: latin1_to_utf8(&name2),
            destruction_category2: latin1_to_utf8(&destruction_category2),
        })
    }
}

#[derive(Clone, Debug)]
pub enum DynamicObjectKind {
    /// An object with dynamic properties like a television
    // Dynamic = 14,
    Dynamic {
        // unknown1 is documented as always 0
        common: DynamicObjectKindCommon,
        params: KindDynamicParams,
    },

    /// An object with an attached animation
    // Animation = 15,
    Animation {
        // unknown1 is documented as always 0 or 2
        common: DynamicObjectKindCommon,

        unknown2: u32,
        names: Vec<String>,
        unknown3: [f32; 3],
        unknown4: u32,
        name3: String,
        name4: String,

        // TODO: convert to enum once all variants are known
        animation_type: String,
        direction: Vec3f,
        distance: f32,
        velocity: f32,
    },

    /// A door or automatic door that the player can interact with more than
    /// once.
    // RepeatableTouchplate = 16,
    RepeatableTouchplate {
        // unknown1 is documented as always 0 or 2
        common: DynamicObjectKindCommon,

        unknown1: u32,
        attachments: Vec<String>,
        unknown2: [f32; 3],

        names: Vec<String>,

        // More sounds
        name2: String,
        name3: String,

        // TODO: convert to enum once all variants are known
        animation_type: String,
        direction: Vec3f,
        distance: f32,
        velocity: f32,
    },

    /// Breakable glass
    // Glass = 20,
    Glass {
        name: String,
    },

    /// A one-time interaction, such as some doors that open once
    // OneTimeTouchplate = 25,
    OneTimeTouchplate {
        collision_type_2d: String,
        collision_type_3d: String,

        coordinates: Vec6f,
        attachments: Vec<String>,
    },

    /// Halo
    // Halo = 31,
    Halo {
        /// A list of (name, coords) of halos
        halos: Vec<(String, Vec8f)>,
    },

    /// Static world effects like manhole steam and smoke stacks
    // StaticEffect = 36,
    StaticEffect,
}

impl DynamicObjectKind {
    fn read(id: Id, buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let reader = match id {
            Id::Dynamic => Self::dynamic,
            Id::Animation => Self::animation,
            Id::RepeatableTouchplate => Self::repeatable_touchplate,
            Id::Glass => Self::glass,
            Id::OneTimeTouchplate => Self::one_time_touchplate,
            Id::Halo => Self::halo,
            Id::StaticEffect => Self::static_effect,
        };
        Ok(reader(buf)?)
    }

    /// An object with dynamic properties like a television
    fn dynamic(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let common = DynamicObjectKindCommon::read(buf)?;
        let params = KindDynamicParams::read(buf)?;
        Ok(Self::Dynamic {
            common,
            params,
        })
    }

    /// An object with an attached animation
    fn animation(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let common = DynamicObjectKindCommon::read(buf)?;

        let unknown2 = buf.read_u32::<LE>().context("unknown2")?;

        let n = buf.read_u32::<LE>().context("name count")?;
        let mut names = Vec::with_capacity(n as usize);
        for i in 0..n {
            let name = buf.read_cstring()
                .with_context(|| format!("animation name {i}"))?;
            names.push(latin1_to_utf8(&name));
        }

        // TODO: clean up this array processing
        let mut unknown3 = vec![0f32; 3];
        for (i, unknown) in unknown3.iter_mut().enumerate() {
            *unknown = buf.read_f32::<LE>()
                .with_context(|| format!("animation unknown3 {i}"))?;
        }
        let unknown3 = unknown3.try_into()
            .expect("animation unknown3 is 3 elements");

        let unknown4 = buf.read_u32::<LE>().context("animation unknown4")?;
        let name3 = buf.read_cstring().context("animation name3")?;
        let name4 = buf.read_cstring().context("animation name4")?;

        // // TODO: convert to enum once all variants are known
        let animation_type = buf.read_cstring().context("animation type")?;
        let direction = Vec3f::read(buf).context("animation direction")?;
        let distance = buf.read_f32::<LE>().context("animation distance")?;
        let velocity = buf.read_f32::<LE>().context("animation velocity")?;

        Ok(Self::Animation {
            common,
            unknown2,
            names,
            unknown3,
            unknown4,
            name3: latin1_to_utf8(&name3),
            name4: latin1_to_utf8(&name4),
            animation_type: latin1_to_utf8(&animation_type),
            direction,
            distance,
            velocity,
        })
    }

    /// A door or automatic door that the player can interact with more than
    /// once. These often have the name "ADT" in MAPs. I think that stands for
    /// "Automatic Door Touchplate".
    fn repeatable_touchplate(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let common = DynamicObjectKindCommon::read(buf)?;
        let unknown1 = buf.read_u32::<LE>().context("ADT unknown1")?;
        let n = buf.read_u32::<LE>().context("ADT attachment count")?;
        let mut attachments = Vec::with_capacity(n as usize);
        for i in 0..n {
            let attachment = buf.read_cstring()
                .with_context(|| format!("ADT attachment {i}"))?;
            attachments.push(latin1_to_utf8(&attachment));
        }

        // TODO: clean up this array processing
        let mut unknown2 = vec![0f32; 3];
        for (i, unknown) in unknown2.iter_mut().enumerate() {
            *unknown = buf.read_f32::<LE>()
                .with_context(|| format!("ADT unknown2 {i}"))?;
        }
        let unknown2 = unknown2.try_into()
            .expect("ADT unknown2 is 3 elements");

        let n = buf.read_u32::<LE>().context("name count")?;
        let mut names = Vec::with_capacity(n as usize);
        for i in 0..n {
            let name = buf.read_cstring()
                .with_context(|| format!("ADT name {i}"))?;
            names.push(latin1_to_utf8(&name));
        }

        let name2 = latin1_to_utf8(&buf.read_cstring().context("ADT name2")?);
        let name3 = latin1_to_utf8(&buf.read_cstring().context("ADT name3")?);

        // TODO: convert to enum once all variants are known
        let animation_type = latin1_to_utf8(&buf.read_cstring()
            .context("animation type")?);
        let direction = Vec3f::read(buf).context("animation direction")?;
        let distance = buf.read_f32::<LE>().context("animation distance")?;
        let velocity = buf.read_f32::<LE>().context("animation velocity")?;

        Ok(Self::RepeatableTouchplate {
            common,
            unknown1,
            attachments,
            unknown2,
            names,
            name2,
            name3,
            animation_type,
            direction,
            distance,
            velocity,
        })
    }

    /// Breakable glass
    fn glass(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let name = buf.read_cstring()?;
        Ok(Self::Glass { name: String::from_utf8(name)? })
    }

    /// A one-time interaction, such as some doors that open once
    fn one_time_touchplate(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let collision_type_2d = latin1_to_utf8(&buf.read_cstring()
            .context("one-time touchplate 2D collision type")?);
        let collision_type_3d = latin1_to_utf8(&buf.read_cstring()
            .context("one-time touchplate 3D collision type")?);

        let coordinates = Vec6f::read(buf)
            .context("one-time touchplate coordinates")?;

        let n = buf.read_u32::<LE>()
            .context("one-time touchplate attachment count")?;
        let mut attachments = Vec::with_capacity(n as usize);
        for i in 0..n {
            let cstring = buf.read_cstring().with_context(|| {
                format!("one-time touchplate attachment name {i} of {n}")
            })?;
            attachments.push(latin1_to_utf8(&cstring));
        }

        Ok(Self::OneTimeTouchplate {
            collision_type_2d,
            collision_type_3d,
            coordinates,
            attachments,
        })
    }

    /// Halo
    fn halo(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let count = buf.read_u32::<LE>().context("halo count")?;
        let mut halos = Vec::with_capacity(count as usize);
        for i in 0..count {
            let name = latin1_to_utf8(&buf.read_cstring().with_context(|| {
                format!("halo name {i} of {count}")
            })?);
            let vec = Vec8f::read(buf)
                .with_context(|| format!("halo vec {i} of {count}"))?;
            halos.push((name, vec));
        }
        Ok(Self::Halo { halos })
    }

    /// Static world effects like manhole steam and smoke stacks
    fn static_effect(_buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        // nothing?
        Ok(Self::StaticEffect)
    }
}

#[derive(Clone, Debug)]
pub struct Rooms {
    pub section_id: u32,
    pub section_name: String,
    pub rooms: Vec<Room>,
}

impl Rooms {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (section_id, section_name) = section_header(buf)
            .context("room list")?;

        let n = buf.read_u32::<LE>().context("room count")?;
        let mut rooms = Vec::with_capacity(n as usize);
        for i in 0..n {
            rooms.push(Room::read(buf).with_context(|| format!("room {i}"))?);
        }

        Ok(Self { section_id, section_name, rooms })
    }
}

#[derive(Clone, Debug)]
pub struct Room {
    pub section_id: u32,
    pub section_name: String,

    pub unknown1: u8,
    pub unknown2: u8,
    pub unknown3: u8,
    // Optionals below are influenced by the unknown values above
    /// Set when `unknown1 == 0`
    pub unknown4: Option<u8>,
    /// Set when `unknown3 == 1`
    pub unknown5: Option<[f32; 6]>,
    /// Set when `unknown4 == 1`
    pub unknown6: Option<[f32; 6]>,

    pub sherman_levels: Vec<ShermanLevel>,

    pub unknown7: f32,
    pub level_heights: Vec<LevelHeight>,
}

impl Room {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (section_id, section_name) = section_header_short(buf)
            .context("room section header short")?;

        let unknown1 = buf.read_u8().context("room unknown1")?;
        let unknown2 = buf.read_u8().context("room unknown2")?;
        let unknown3 = buf.read_u8().context("room unknown3")?;

        let unknown4 = if unknown1 == 0 {
            Some(buf.read_u8().context("room unknown4")?)
        } else {
            None
        };

        let unknown5 = if unknown3 == 1 {
            let mut array = [0f32; 6];
            for (i, x) in array.iter_mut().enumerate() {
                *x = buf.read_f32::<LE>().with_context(|| {
                    format!("room unknown5 {i}")
                })?;
            }
            Some(array)
        } else {
            None
        };

        let unknown6 = if unknown4.is_some_and(|x| x == 1) {
            let mut array = [0f32; 6];
            for (i, x) in array.iter_mut().enumerate() {
                *x = buf.read_f32::<LE>().with_context(|| {
                    format!("room unknown6 {i}")
                })?;
            }
            Some(array)
        } else {
            None
        };

        let n = buf.read_u32::<LE>().context("room level count")?;
        let mut levels = Vec::with_capacity(n as usize);
        for i in 0..n {
            levels.push(ShermanLevel::read(buf).with_context(|| {
                format!("room sherman level {i} of {n}")
            })?);
        }

        let n = buf.read_u32::<LE>().context("room level heights count")?;
        let unknown7 = buf.read_f32::<LE>().context("room unknown7")?;
        let mut heights = Vec::with_capacity(n as usize);
        for i in 0..n {
            heights.push(LevelHeight::read(buf).with_context(|| {
                format!("room sherman level heights {i} of {n}")
            })?);
        }

        Ok(Self {
            section_id,
            section_name,
            unknown1,
            unknown2,
            unknown3,
            unknown4,
            unknown5,
            unknown6,
            sherman_levels: levels,
            unknown7,
            level_heights: heights,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ShermanLevel {
    pub name: String,
    pub tm_with_aabb: Vec<TransformationWithAABB>,
    // TODO: always 1?
    pub unknown1: Vec<f32>,
    // TODO: has sherman level plan area?
    pub unknown2: u8,
}

impl ShermanLevel {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let name = latin1_to_utf8(&buf.read_cstring().context("level name")?);

        let n = buf.read_u32::<LE>().context("level TM + AABB count")?;
        let mut tm_with_aabb = Vec::with_capacity(n as usize);
        for i in 0..n {
            let tm = TransformationWithAABB::read(buf).with_context(|| {
                format!("level TM + AABBB {i}")
            })?;
            tm_with_aabb.push(tm);
        }

        let n = buf.read_u32::<LE>().context("unknown count")?;
        let mut unknown1 = Vec::with_capacity(n as usize);
        for i in 0..n {
            let value = buf.read_f32::<LE>().with_context(|| {
                format!("level unknown1 {i}")
            })?;
            unknown1.push(value);
        }

        let unknown2 = buf.read_u8().context("level unknown2")?;

        Ok(Self { name, tm_with_aabb, unknown1, unknown2 })
    }
}

#[derive(Clone, Debug)]
pub struct TransformationWithAABB {
    pub tm: TransformationMatrix,
    pub aabb: [f32; 6],
}

impl TransformationWithAABB {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let tm = TransformationMatrix::read(buf).context("TM + AABB")?;
        let mut aabb = [0f32; 6];
        for (i, side) in aabb.iter_mut().enumerate() {
            *side = buf.read_f32::<LE>().with_context(|| {
                format!("level TM + AABB side {i}")
            })?;
        }
        Ok(Self { tm, aabb })
    }
}

#[derive(Clone, Debug)]
pub struct LevelHeight {
    pub height: f32,
    pub unknown: f32,
    // TODO: there is an extra 4 bytes between the Room list and Transition list
    // with BT and CL maps. Figure out the conditional logic here. All RS, CO,
    // UT maps are fine. BT03 is an example of a broken map.
}

impl LevelHeight {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let height = buf.read_f32::<LE>().context("level height")?;
        let unknown = buf.read_f32::<LE>().context("level height unknown")?;
        Ok(Self { height, unknown })
    }
}

#[derive(Clone, Debug)]
pub struct Transitions {
    pub section_id: u32,
    pub section_name: String,
    pub transitions: Vec<Transition>,
}

impl Transitions {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (section_id, section_name) = section_header(buf)
            .context("transitions")?;

        let n = buf.read_u32::<LE>().context("transitions count")?;
        let mut transitions = Vec::with_capacity(n as usize);
        for i in 0..n {
            transitions.push(Transition::read(buf).with_context(|| {
                format!("transition {i} of {n}")
            })?);
        }

        Ok(Self { section_id, section_name, transitions })
    }
}

#[derive(Clone, Debug)]
pub struct Transition {
    pub name: String,
    pub coords: TransitionCoords,
}

impl Transition {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let name = latin1_to_utf8(&buf.read_cstring().context("transition")?);
        let coords = TransitionCoords::read(buf)?;
        Ok(Self { name, coords })
    }
}

#[derive(Clone, Debug)]
pub struct TransitionCoords {
    pub p1: Vec3f,
    pub p2: Vec3f,
}

impl TransitionCoords {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let p1 = Vec3f::read(buf).context("transition coords P1")?;
        let p2 = Vec3f::read(buf).context("transition coords P2")?;
        Ok(Self { p1, p2 })
    }
}

#[derive(Clone, Debug)]
pub struct PlanningLevels {
    pub section_id: u32,
    pub section_name: String,
    pub levels: Vec<PlanningLevel>,
}

impl PlanningLevels {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let (section_id, section_name) = section_header(buf)
            .context("planning levels")?;
        let n = buf.read_u32::<LE>().context("planning levels count")?;
        let mut levels = Vec::with_capacity(n as usize);
        for i in 0..n {
            levels.push(PlanningLevel::read(buf).with_context(|| {
                format!("planning level {i} of {n}")
            })?);
        }
        Ok(Self { section_id, section_name, levels })
    }
}

#[derive(Clone, Debug)]
pub struct PlanningLevel {
    pub level_number: f32,
    pub floor_height: f32,
    pub room_names: Vec<String>,
}

impl PlanningLevel {
    fn read(buf: &mut Cursor<Vec<u8>>) -> Result<Self> {
        let level_number = buf.read_f32::<LE>()
            .context("planning level number")?;
        let floor_height = buf.read_f32::<LE>()
            .context("planning level floor height")?;

        let n = buf.read_u32::<LE>().context("planning level room count")?;
        let mut room_names = Vec::with_capacity(n as usize);
        for i in 0..n {
            let room_name = latin1_to_utf8(&buf.read_cstring().with_context(|| {
                format!("planning level room name {i} of {n}")
            })?);
            room_names.push(room_name);
        }

        Ok(Self { level_number, floor_height, room_names })
    }
}

/// Read and parse a section header that precedes the section data. Discards the
/// section size in bytes and name. The total size isn't used in our reader
/// implementation and the section name is encoded in the return type.
fn section_header(buf: &mut Cursor<Vec<u8>>) -> Result<(u32, String)> {
    let _section_size = buf.read_u32::<LE>()
        .context("failed to read section size")?;
    section_header_short(buf)
}

/// Read the id and return the non-Version name
fn section_header_short(buf: &mut Cursor<Vec<u8>>) -> Result<(u32, String)> {
    let id = buf.read_u32::<LE>()
        .context("failed to read material id")?;

    // Read a name and if the value is "Version" then we need to read an
    // additional name, which is the in-game map texture short name. This
    // appears to be a convention.
    let mut name = buf.read_cstring().context("section header name")?;
    if name == b"Version" {
        let _version = buf.read_u32::<LE>().context("version number")?;
        name = buf.read_cstring().context("texture short name")?;
    }

    Ok((id, latin1_to_utf8(&name)))
}

/// Read primitive data types that are common in the MAP format
trait ReadMapBytes: ReadBytesExt {
    fn read_cstring(&mut self) -> Result<Vec<u8>>;
    fn read_bool(&mut self) -> Result<bool>;
    fn read_f32_xy(&mut self) -> Result<(f32, f32)>;
    fn read_f32_xyz(&mut self) -> Result<(f32, f32, f32)>;
}

impl<T> ReadMapBytes for Cursor<T>
where
    T: AsRef<[u8]>
{
    fn read_cstring(&mut self) -> Result<Vec<u8>> {
        let len = self.read_u32::<LE>()
            .context("could not read length")? as usize;
        anyhow::ensure!(len >= 1, "empty string");
        let mut buf = vec![0; len];
        self.read_exact(&mut buf)
            .with_context(|| format!("could not read {len} bytes"))?;
        // Do not need the null terminator
        buf.truncate(len - 1);
        Ok(buf)
    }

    fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_u8().context("could not read bool")? == 1)
    }

    fn read_f32_xy(&mut self) -> Result<(f32, f32)> {
        let x = self.read_f32::<LE>().context("x")?;
        let y = self.read_f32::<LE>().context("y")?;
        Ok((x, y))
    }

    fn read_f32_xyz(&mut self) -> Result<(f32, f32, f32)> {
        let (x, y) = self.read_f32_xy()?;
        let z = self.read_f32::<LE>().context("z")?;
        Ok((x, y, z))
    }
}

/// Strings are ISO-8859-1 (Latin1) and must be converted properly. For example,
/// "intérieur" 7th byte is 0xE9 in Latin1 (and in Rogue Spear MAP files) but
/// this is 0xC3 0xA9 byte sequence in UTF-8.
fn latin1_to_utf8(s: &[u8]) -> String {
    s.iter().map(|&c| c as char).collect()
}
