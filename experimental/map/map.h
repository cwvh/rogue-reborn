// Reader for Rogue Spear .MAP files ("BeginMapv2.1" ... "EndMap").
//
// A transcription of src/map.rs: the same sections, read in the same order,
// with the same checks. Names follow the Rust where they are known; the
// Rust's name is given where a type here is named differently.
//
// Everything a map holds lives in the arena it is read into. Strings are the
// file's bytes (ISO-8859-1), NUL-terminated.
#ifndef RR_MAP_H
#define RR_MAP_H

#include <stddef.h>
#include <stdint.h>

// Memory handed out from `beg` up, never freed one piece at a time
typedef struct { char *beg, *end; } arena;

typedef struct { float x, y, z; } map_vec3;
typedef struct { float r, g, b, a; } map_color;
typedef struct { float u, v; } map_uv;

// A face's plane: its normal, and the distance from the origin to the face,
// signed along the normal (FaceNormal)
typedef struct { float x, y, z, distance; } map_plane;

// A triangle's three corners, as indices
typedef struct { uint16_t a, b, c; } map_tri;

typedef struct { map_vec3 x_axis, y_axis, z_axis, position; } map_transform;

// Every list in the file starts with one of these. Rooms leave out the size.
typedef struct {
    uint32_t size;    // bytes after this field, up to the end of the section
    uint32_t id;
    uint32_t version; // 0 unless the name is preceded by "Version" and a number
    char *name;
} map_section;

typedef enum {
    MAP_ADDRESS_OPAQUE = 0,
    MAP_ADDRESS_WRAP = 1,
    MAP_ADDRESS_CLAMP = 3,
} map_address_mode;

typedef struct {
    map_section section;
    char *filename;   // the texture, as in "wall_house_wood_01.bmp"
    float opacity;
    uint32_t emissive_strength;
    map_address_mode address_mode;
    map_color ambient, diffuse, specular;
    float specular_level;
    bool two_sided;
} map_material;

// The triangles of an object drawn with one material (ObjectData)
typedef struct {
    uint32_t material; // index into the materials

    uint32_t face_count;
    map_plane *face_planes;      // Faces::normals
    map_tri *faces;              // corners, into the object's vertices
    map_tri *face_texture_verts; // corners, into the texture vertices below

    uint32_t texture_vertex_count;
    map_vec3 *normals;
    map_uv *uvs;
    map_color *colors;
} map_object_data;

// One collision face. A drawn one is face `face_index_1` of the object's data
// `coord2[2]`, with `coord1` its corners in the object's vertices. Where
// `coord2[2]` is 0xffff it is not drawn: `coord1` indexes the collision
// vertices and `face_index_1` the collision planes.
typedef struct {
    uint16_t coord1[3];
    uint16_t face_index_1;
    uint16_t coord2[3];
    uint16_t face_index_2;
} map_tag;

// A group of an object's collision faces, "NONE" or a name such as
// "staticcaptainschair_02" (CollisionGroup)
typedef struct {
    char *name;
    uint32_t flags;   // 2 leaves the group out of the walls
    uint32_t index_count;
    uint16_t *indices; // into the object's tags
} map_collision_group;

typedef struct {
    map_section section;
    map_section object_section;

    uint32_t vertex_count;
    map_vec3 *vertices;

    uint32_t data_count;
    map_object_data *datas;

    // The collision faces that are not drawn
    uint32_t collision_vertex_count;
    map_vec3 *collision_vertices;
    uint32_t collision_plane_count;
    map_plane *collision_planes;

    uint32_t tag_count;
    map_tag *tags;

    uint32_t group_count;
    map_collision_group *groups;
} map_object;

typedef struct {
    map_section section;
    uint32_t vertex_count;
    map_vec3 *vertices;
    uint32_t room;
    uint32_t opposite_room;
} map_portal;

// Dynamic object kinds, by their section id
typedef enum {
    MAP_DYNAMIC = 14,               // with dynamic properties, such as a television
    MAP_ANIMATION = 15,             // with an animation attached
    MAP_REPEATABLE_TOUCHPLATE = 16, // a door that can be used more than once
    MAP_GLASS = 20,                 // breakable glass
    MAP_ONE_TIME_TOUCHPLATE = 25,   // a door that opens once
    MAP_HALO = 31,
    MAP_STATIC_EFFECT = 36,         // manhole steam, smoke stacks
} map_dynamic_kind;

// What the dynamic, animation and touchplate kinds share (DynamicObjectKindCommon)
typedef struct {
    map_transform transform;
    char *name;
    uint32_t unknown1;
    char *sounds[4];
    char *collision_type_2d;
    char *collision_type_3d;
    char *destruction_action;
    char *destruction_category;
    char *penetration_type;
    char *name2;
    char *destruction_category2;
} map_dynamic_common;

typedef struct {
    char *name;
    float unknown1[9];
    uint32_t unknown2[2];
} map_dynamic_param; // KindDynamicParamStruct

typedef struct {
    char *name;
    float values[8]; // Vec8f
} map_halo;

typedef struct {
    map_section section;
    char *name;
    map_transform transform;
    map_dynamic_kind kind;
    union {
        struct {
            map_dynamic_common common;
            // Params as structs, where the file's count is not 0
            uint32_t param_count;
            map_dynamic_param *params;
            // Otherwise names and four numbers (KindDynamicParams::Flat)
            uint32_t name_count;
            char **names;
            float unknown[4];
        } dynamic;
        struct {
            map_dynamic_common common;
            uint32_t unknown2;
            uint32_t name_count;
            char **names;
            float unknown3[3];
            uint32_t unknown4;
            char *name3;
            char *name4;
            char *animation_type;
            map_vec3 direction;
            float distance;
            float velocity;
        } animation;
        struct {
            map_dynamic_common common;
            uint32_t unknown1;
            uint32_t attachment_count;
            char **attachments;
            float unknown2[3];
            uint32_t name_count;
            char **names;
            char *name2;
            char *name3;
            char *animation_type;
            map_vec3 direction;
            float distance;
            float velocity;
        } repeatable_touchplate;
        struct {
            char *name;
        } glass;
        struct {
            char *collision_type_2d;
            char *collision_type_3d;
            float coordinates[6];
            uint32_t attachment_count;
            char **attachments;
        } one_time_touchplate;
        struct {
            uint32_t count;
            map_halo *halos;
        } halo;
    };
} map_dynamic_object;

// A box a room's level fills: `aabb` (min x, y, z, max x, y, z) placed by
// `transform` (TransformationWithAABB)
typedef struct {
    map_transform transform;
    float aabb[6];
} map_box;

typedef struct {
    char *name;
    uint32_t box_count;
    map_box *boxes;
    uint32_t floor_count;
    float *floors;     // planning level numbers
    uint8_t has_model; // where not 0 the original reads a model next; no map has one
} map_level; // ShermanLevel

// A planning level a room is on
typedef struct {
    float number;
    float height;
} map_room_floor;

// The original's room reader (0x0055e760) reads these in this order
typedef struct {
    map_section section; // with no size
    // How a flashbang in the room affects those in it. In an enclosed room it gets them
    // within its range even facing away; in a small one it fills the room. Neither is open.
    uint8_t enclosed;
    bool has_small;      // where not enclosed
    uint8_t small;
    uint8_t unknown_flag; // read by the damage code; the mission's RoomFlags bit 0x10 also sets it

    // Where the version is above 1. Without bounds the game works them out from the levels.
    bool has_weather_flag;
    uint8_t has_weather_bounds;
    map_vec3 weather_min, weather_max;

    uint32_t level_count;
    map_level *levels;

    uint32_t floor_count;
    map_room_floor *floors;

    uint32_t unknown_count; // 0 in every map
    uint32_t *unknown;
} map_room;

typedef struct {
    char *name;
    map_vec3 p1, p2;
} map_transition;

typedef struct {
    float level_number;
    float floor_height;
    uint32_t room_count;
    char **room_names;
} map_planning_level;

typedef struct map {
    uint32_t timestamp; // Unix time the file was written

    struct { map_section section; uint32_t count; map_material *items; } materials;
    struct { map_section section; uint32_t count; map_object *items; } geometries;
    struct { map_section section; uint32_t count; map_portal *items; } portals;
    struct { map_section section; uint32_t count; } lights;
    struct { map_section section; uint32_t count; map_dynamic_object *items; } dynamic_objects;
    struct { map_section section; uint32_t count; map_room *items; } rooms;
    struct { map_section section; uint32_t count; map_transition *items; } transitions;
    struct { map_section section; uint32_t count; map_planning_level *items; } planning_levels;
} map;

// What reading a map gave: the map, or where `err` is set, why it failed. A map
// that fails partway is described by a chain of contexts, such as
// "Portal List: portal 12: coordinates count: ...", kept in the arena. Every
// other message is a string literal.
typedef struct {
    map *map;
    const char *err;
} map_result;

// Read a .MAP file into `perm`, which also holds the file's bytes while it is
// read. A map that fails to read leaves at most its message in `perm`.
[[nodiscard]] map_result map_read(const char *path, arena *perm);

// Parse a .MAP file already in memory into `perm`. The map does not refer to `data`.
[[nodiscard]] map_result map_parse(const void *data, ptrdiff_t size, arena *perm);

#endif
