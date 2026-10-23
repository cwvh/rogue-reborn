//! A made-up level for trying things out: one plain room, built in memory rather than read
//! from a MAP file. The player and up to eight characters are placed in it close together.

use crate::map::{
    Collisions, Color4f, DynamicObjects, Faces, FaceNormal, Geometries, Lights, LevelHeight, Map, MapHeader,
    Material, Materials, NormalCoord, Object, ObjectData, PlanningLevel, PlanningLevels, Portals, Room, Rooms,
    ShermanLevel, TextureAddressMode, TextureVertices, TransformationMatrix, TransformationWithAABB, Transitions,
    UvCoord, Vec3f, Vertex,
};

/// The most characters the demo room places
pub const MAX_NPCS: usize = 8;

/// The room's inside measurements, in map units (about a centimeter each)
pub const WIDTH: f32 = 1000.0;
pub const DEPTH: f32 = 1000.0;
pub const HEIGHT: f32 = 300.0;

/// How far a texture repeats, in map units
const TILE: f32 = 200.0;

/// The room is named like the rooms of a real level, so it is drawn and walked like one
const ROOM: &str = "1";

/// A box with its floor at y = 0, centered on the origin, with every face pointing inward
pub fn map() -> Map {
    let (hx, hz) = (WIDTH / 2.0, DEPTH / 2.0);

    // Each face has its own four corners so it can be textured on its own. Corners go round
    // counter-clockwise seen from inside, which makes the face's normal point into the room.
    let faces: [([[f32; 3]; 4], [f32; 3]); 6] = [
        ([[-hx, 0.0, -hz], [-hx, 0.0, hz], [hx, 0.0, hz], [hx, 0.0, -hz]], [0.0, 1.0, 0.0]), // floor
        ([[-hx, HEIGHT, -hz], [hx, HEIGHT, -hz], [hx, HEIGHT, hz], [-hx, HEIGHT, hz]], [0.0, -1.0, 0.0]), // ceiling
        ([[-hx, 0.0, hz], [-hx, HEIGHT, hz], [hx, HEIGHT, hz], [hx, 0.0, hz]], [0.0, 0.0, -1.0]), // north wall
        ([[hx, 0.0, -hz], [hx, HEIGHT, -hz], [-hx, HEIGHT, -hz], [-hx, 0.0, -hz]], [0.0, 0.0, 1.0]), // south wall
        ([[-hx, 0.0, -hz], [-hx, HEIGHT, -hz], [-hx, HEIGHT, hz], [-hx, 0.0, hz]], [1.0, 0.0, 0.0]), // west wall
        ([[hx, 0.0, hz], [hx, HEIGHT, hz], [hx, HEIGHT, -hz], [hx, 0.0, -hz]], [-1.0, 0.0, 0.0]), // east wall
    ];

    let mut vertices = Vec::new();
    let mut uvs = Vec::new();
    let mut normals = Vec::new();
    let mut face_normals = Vec::new();
    let mut face_indices = Vec::new();
    let mut texture_indices = Vec::new();
    for (corners, normal) in &faces {
        let base = vertices.len() as u16;
        for p in corners {
            vertices.push(Vertex { x: p[0], y: p[1], z: p[2] });
            // The texture runs along the face's two directions in the world
            let (u, v) = if normal[1] != 0.0 { (p[0], p[2]) } else if normal[0] != 0.0 { (p[2], p[1]) } else { (p[0], p[1]) };
            uvs.push(UvCoord { u: u / TILE, v: -v / TILE });
            normals.push(NormalCoord { x: normal[0], y: normal[1], z: normal[2] });
        }
        for tri in [(0, 1, 2), (0, 2, 3)] {
            face_indices.push((base + tri.0, base + tri.1, base + tri.2));
            texture_indices.push((base + tri.0, base + tri.1, base + tri.2));
            face_normals.push(FaceNormal {
                x: normal[0],
                y: normal[1],
                z: normal[2],
                distance_origin_to_face: -(normal[0] * corners[0][0] + normal[1] * corners[0][1] + normal[2] * corners[0][2]),
            });
        }
    }
    let white = || Color4f { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    let face_colors = vertices.iter().map(|_| white()).collect();

    let object = Object {
        id: 1,
        name: ROOM.to_string(),
        object_id: 1,
        object_name: ROOM.to_string(),
        vertices,
        object_datas: vec![ObjectData {
            material: 0,
            faces: Faces { normals: face_normals, face_indices, texture_indices },
            texture_vertices: TextureVertices { normals, uv_coords: uvs, face_colors },
        }],
        collisions: Collisions { vertices: Vec::new(), faces: Vec::new() },
        tags: Vec::new(),
        collision_groups: Vec::new(),
    };

    let at = |x: f32, y: f32, z: f32| Vec3f { x, y, z };
    let room = Room {
        section_id: 1,
        section_name: ROOM.to_string(),
        unknown1: 0,
        unknown2: 0,
        unknown3: 0,
        unknown4: Some(0),
        unknown5: None,
        unknown6: None,
        sherman_levels: vec![ShermanLevel {
            name: ROOM.to_string(),
            tm_with_aabb: vec![TransformationWithAABB {
                tm: TransformationMatrix { x_axis: at(1.0, 0.0, 0.0), y_axis: at(0.0, 1.0, 0.0), z_axis: at(0.0, 0.0, 1.0), position: at(0.0, 0.0, 0.0) },
                aabb: [-hx, 0.0, -hz, hx, HEIGHT, hz],
            }],
            unknown1: Vec::new(),
            unknown2: 0,
        }],
        unknown7: 0.0,
        level_heights: vec![LevelHeight { height: HEIGHT, unknown: 0.0 }],
    };

    Map {
        header: MapHeader { timestamp: 0 },
        materials: Materials {
            id: 0,
            materials: vec![Material {
                id: 0,
                filename: "bunker_slab_01.bmp".to_string(),
                name: "slab".to_string(),
                opacity: 1.0,
                emissive_strength: 0,
                address_mode: TextureAddressMode::Wrap,
                ambient: white(),
                diffuse: white(),
                specular: white(),
                specular_level: 0.0,
                two_sided: false,
            }],
        },
        geometries: Geometries { id: 0, objects: vec![object] },
        portals: Portals { id: 0, name: "Portals".to_string(), portals: Vec::new() },
        lights: Lights { id: 0, name: "Lights".to_string(), light_count: 0 },
        dynamic_objects: DynamicObjects { id: 0, name: "DynamicObjects".to_string(), dynamic_objects: Vec::new() },
        rooms: Rooms { section_id: 0, section_name: "Rooms".to_string(), rooms: vec![room] },
        transitions: Transitions { section_id: 0, section_name: "Transitions".to_string(), transitions: Vec::new() },
        planning_levels: PlanningLevels {
            section_id: 0,
            section_name: "PlanningLevels".to_string(),
            levels: vec![PlanningLevel { level_number: 1.0, floor_height: 0.0, room_names: vec![ROOM.to_string()] }],
        },
    }
}

/// Where everyone starts, on the floor, in map space: the player first, standing near the
/// south wall, then `npcs` characters (at most `MAX_NPCS`) in rows ahead of them, an arm's
/// length or so apart.
pub fn spawns(npcs: usize) -> Vec<[f32; 3]> {
    const PER_ROW: usize = 4;
    const SPACING: f32 = 110.0;
    let mut points = vec![[0.0, 0.0, -DEPTH / 2.0 + 150.0]];
    for i in 0..npcs.min(MAX_NPCS) {
        let (row, col) = (i / PER_ROW, i % PER_ROW);
        let in_row = (npcs.min(MAX_NPCS) - row * PER_ROW).min(PER_ROW);
        let x = (col as f32 - (in_row as f32 - 1.0) / 2.0) * SPACING;
        points.push([x, 0.0, -50.0 + row as f32 * 220.0]);
    }
    points
}

/// Where up to `MAX_PLAYERS` players start, side by side near the south wall
pub fn player_spawns(count: usize) -> Vec<[f32; 3]> {
    let count = count.clamp(1, crate::sim::MAX_PLAYERS);
    (0..count).map(|i| [(i as f32 - (count as f32 - 1.0) / 2.0) * 90.0, 0.0, -DEPTH / 2.0 + 150.0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{CollisionWorld, Params};

    #[test]
    fn the_floor_is_underfoot_and_the_walls_are_where_they_should_be() {
        let world = CollisionWorld::from_map(&map(), Params::default());
        assert_eq!(world.ground_at(0.0, 0.0, 100.0), Some(0.0));
        let center = [0.0, 150.0, 0.0];
        let wall = |dir: [f32; 3]| world.raycast_hit(center, dir, 5000.0).map(|h| h.dist);
        assert!((wall([1.0, 0.0, 0.0]).unwrap() - WIDTH / 2.0).abs() < 0.5);
        assert!((wall([0.0, 0.0, -1.0]).unwrap() - DEPTH / 2.0).abs() < 0.5);
        assert!((wall([0.0, 1.0, 0.0]).unwrap() - (HEIGHT - 150.0)).abs() < 0.5);
    }

    #[test]
    fn everyone_starts_inside_the_room_and_apart() {
        let points = spawns(MAX_NPCS + 5);
        assert_eq!(points.len(), 1 + MAX_NPCS);
        for (i, p) in points.iter().enumerate() {
            assert!(p[0].abs() < WIDTH / 2.0 - 50.0 && p[2].abs() < DEPTH / 2.0 - 50.0, "{p:?}");
            for q in &points[..i] {
                assert!((p[0] - q[0]).hypot(p[2] - q[2]) >= 100.0, "{p:?} and {q:?} are too close");
            }
        }
    }
}
