// Reader for Rogue Spear .MAP files: see map.h
#include "map.h"

#include <inttypes.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

// --- Arena ------------------------------------------------------------------

constexpr int NOZERO = 1;

// `count` items of `size` bytes, zeroed unless NOZERO is given, or nullptr where they do not fit
static void *alloc(arena *a, ptrdiff_t size, ptrdiff_t align, ptrdiff_t count, int flags)
{
    ptrdiff_t pad = -(uintptr_t)a->beg & (align - 1);
    if (count > (a->end - a->beg - pad) / size)
        return nullptr;
    char *p = a->beg + pad;
    a->beg += pad + count * size;
    return flags & NOZERO ? p : memset(p, 0, count * size);
}

#define new(a, t, n, ...) (t *)alloc(a, sizeof(t), alignof(t), n, 0 __VA_OPT__(| __VA_ARGS__))

// --- Reading ----------------------------------------------------------------

typedef struct {
    const uint8_t *p, *end;
    arena *perm;
    char err[1024];
} reader;

// Fail with `fmt` as the innermost error
static bool fail(reader *r, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(r->err, sizeof r->err, fmt, ap);
    va_end(ap);
    return false;
}

// Put a context in front of the error, as anyhow's .context() does
static bool ctx(reader *r, const char *fmt, ...)
{
    char head[256], whole[sizeof r->err];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(head, sizeof head, fmt, ap);
    va_end(ap);
    if (snprintf(whole, sizeof whole, "%s: %s", head, r->err) < 0)
        return false;
    memcpy(r->err, whole, sizeof whole);
    return false;
}

// Return false, with a context when one is given
#define TRY(expr, ...) do { if (!(expr)) return ctx(r, __VA_ARGS__); } while (0)
#define TRY_(expr) do { if (!(expr)) return false; } while (0)

static const char EOF_ERROR[] = "failed to fill whole buffer";

static size_t left(const reader *r)
{
    return (size_t)(r->end - r->p);
}

static bool take(reader *r, size_t n, const uint8_t **out)
{
    if (left(r) < n) {
        r->p = r->end;
        return fail(r, EOF_ERROR);
    }
    *out = r->p;
    r->p += n;
    return true;
}

static bool read_u8(reader *r, uint8_t *out)
{
    const uint8_t *b;
    TRY_(take(r, 1, &b));
    *out = b[0];
    return true;
}

static bool read_u16(reader *r, uint16_t *out)
{
    const uint8_t *b;
    TRY_(take(r, 2, &b));
    *out = (uint16_t)(b[0] | b[1] << 8);
    return true;
}

static bool read_u32(reader *r, uint32_t *out)
{
    const uint8_t *b;
    TRY_(take(r, 4, &b));
    *out = (uint32_t)b[0] | (uint32_t)b[1] << 8 | (uint32_t)b[2] << 16 | (uint32_t)b[3] << 24;
    return true;
}

static bool read_f32(reader *r, float *out)
{
    uint32_t bits;
    TRY_(read_u32(r, &bits));
    memcpy(out, &bits, sizeof *out);
    return true;
}

static bool read_bool(reader *r, bool *out)
{
    uint8_t b;
    TRY(read_u8(r, &b), "could not read bool");
    *out = b == 1;
    return true;
}

static bool read_xyz(reader *r, float *x, float *y, float *z)
{
    TRY(read_f32(r, x), "x");
    TRY(read_f32(r, y), "y");
    TRY(read_f32(r, z), "z");
    return true;
}

static bool read_vec3(reader *r, map_vec3 *v)
{
    return read_xyz(r, &v->x, &v->y, &v->z);
}

static bool read_plane(reader *r, map_plane *p)
{
    TRY_(read_xyz(r, &p->x, &p->y, &p->z));
    TRY(read_f32(r, &p->distance), "distance origin to face");
    return true;
}

static bool read_color(reader *r, map_color *c)
{
    TRY(read_f32(r, &c->r), "red");
    TRY(read_f32(r, &c->g), "green");
    TRY(read_f32(r, &c->b), "blue");
    TRY(read_f32(r, &c->a), "alpha");
    return true;
}

static bool read_transform(reader *r, map_transform *t)
{
    TRY(read_vec3(r, &t->x_axis), "transformation matrix x-axis");
    TRY(read_vec3(r, &t->y_axis), "transformation matrix y-axis");
    TRY(read_vec3(r, &t->z_axis), "transformation matrix z-axis");
    TRY(read_vec3(r, &t->position), "transformation matrix position");
    return true;
}

// A string: its length, counting a NUL at the end, then its bytes. The last
// byte is dropped whatever it is, as in the Rust.
static bool read_string_len(reader *r, char **out, uint32_t *out_len)
{
    uint32_t len;
    TRY(read_u32(r, &len), "could not read length");
    if (len < 1)
        return fail(r, "empty string");
    if (len > left(r))
        return fail(r, "could not read %" PRIu32 " bytes, only %zu left", len, left(r));
    char *s = new(r->perm, char, len, NOZERO);
    if (!s)
        return fail(r, "out of memory");
    memcpy(s, r->p, len - 1);
    r->p += len;
    s[len - 1] = 0;
    *out = s;
    if (out_len)
        *out_len = len - 1;
    return true;
}

static bool read_string(reader *r, char **out)
{
    return read_string_len(r, out, nullptr);
}

// Whether `s` is UTF-8, which the Rust asks of a few strings
static bool valid_utf8(const char *str)
{
    const unsigned char *s = (const unsigned char *)str;
    while (*s) {
        int n = *s < 0x80 ? 0 : (*s & 0xe0) == 0xc0 ? 1 : (*s & 0xf0) == 0xe0 ? 2 : (*s & 0xf8) == 0xf0 ? 3 : -1;
        if (n < 0 || (n == 1 && *s < 0xc2) || (n == 3 && *s > 0xf4))
            return false;
        uint32_t c = *s++ & (0x3f >> n);
        for (int i = 0; i < n; i++, s++) {
            if ((*s & 0xc0) != 0x80)
                return false;
            c = c << 6 | (*s & 0x3f);
        }
        if ((n == 2 && (c < 0x800 || (c >= 0xd800 && c <= 0xdfff))) || (n == 3 && (c < 0x10000 || c > 0x10ffff)))
            return false;
    }
    return true;
}

static bool read_utf8(reader *r, char *s)
{
    return valid_utf8(s) || fail(r, "invalid utf-8");
}

// Point `ptr` at `n` new items, zeroed unless NOZERO is given
#define NEW(ptr, n, ...) \
    do { \
        if (!((ptr) = new(r->perm, typeof(*(ptr)), (n) __VA_OPT__(,) __VA_ARGS__))) \
            return fail(r, "out of memory for " #ptr); \
    } while (0)

// The id and name, with the "Version" and number that may come before the
// name (the map's own short name, where it has one)
static bool read_short_section(reader *r, map_section *s)
{
    uint32_t len;
    TRY(read_u32(r, &s->id), "failed to read material id");
    TRY(read_string_len(r, &s->name, &len), "section header name");
    if (len == 7 && memcmp(s->name, "Version", 7) == 0) {
        TRY(read_u32(r, &s->version), "version number");
        TRY(read_string(r, &s->name), "texture short name");
    }
    return true;
}

static bool read_section(reader *r, map_section *s)
{
    TRY(read_u32(r, &s->size), "failed to read section size");
    return read_short_section(r, s);
}

// --- Materials --------------------------------------------------------------

static bool read_material(reader *r, map_material *m)
{
    uint32_t mode;
    TRY(read_section(r, &m->section), "material section header");
    TRY(read_string(r, &m->filename), "texture filename");
    TRY(read_f32(r, &m->opacity), "opacity");
    TRY(read_u32(r, &m->emissive_strength), "emissive strength");
    TRY(read_u32(r, &mode), "texture address mode");
    if (mode != MAP_ADDRESS_OPAQUE && mode != MAP_ADDRESS_WRAP && mode != MAP_ADDRESS_CLAMP)
        return fail(r, "unknown texture mode address value: %" PRIu32, mode);
    m->address_mode = mode;
    TRY(read_color(r, &m->ambient), "ambient");
    TRY(read_color(r, &m->diffuse), "diffuse");
    TRY(read_color(r, &m->specular), "specular");
    TRY(read_f32(r, &m->specular_level), "specular level");
    TRY(read_bool(r, &m->two_sided), "two sided");
    return true;
}

static bool read_materials(reader *r, map *m)
{
    TRY(read_section(r, &m->materials.section), "material list section header");
    TRY(read_u32(r, &m->materials.count), "missing number of materials");
    NEW(m->materials.items, m->materials.count);
    for (uint32_t i = 0; i < m->materials.count; i++)
        TRY(read_material(r, &m->materials.items[i]), "material section header %" PRIu32, i);
    return true;
}

// --- Geometry ---------------------------------------------------------------

static bool read_tris(reader *r, map_tri *tris, uint32_t n)
{
    for (uint32_t i = 0; i < n; i++) {
        TRY(read_u16(r, &tris[i].a), "p1 of face index %" PRIu32, i);
        TRY(read_u16(r, &tris[i].b), "p2 of face index %" PRIu32, i);
        TRY(read_u16(r, &tris[i].c), "p3 of face index %" PRIu32, i);
    }
    return true;
}

static bool read_object_data(reader *r, map_object_data *d)
{
    uint32_t n;
    TRY(read_u32(r, &d->material), "material");

    // Faces: their planes, then their corners, then their texture corners
    TRY(read_u32(r, &d->face_count), "face count");
    n = d->face_count;
    NEW(d->face_planes, n, NOZERO);
    NEW(d->faces, n, NOZERO);
    NEW(d->face_texture_verts, n, NOZERO);
    for (uint32_t i = 0; i < n; i++)
        TRY(read_plane(r, &d->face_planes[i]), "face normal %" PRIu32, i);
    TRY_(read_tris(r, d->faces, n));
    TRY_(read_tris(r, d->face_texture_verts, n));

    // Texture vertices: normals, then UVs, then colors
    TRY(read_u32(r, &d->texture_vertex_count), "vertices count");
    n = d->texture_vertex_count;
    NEW(d->normals, n, NOZERO);
    NEW(d->uvs, n, NOZERO);
    NEW(d->colors, n, NOZERO);
    for (uint32_t i = 0; i < n; i++)
        TRY(read_vec3(r, &d->normals[i]), "normal coordinate %" PRIu32 " of %" PRIu32, i, n);
    for (uint32_t i = 0; i < n; i++) {
        TRY(read_f32(r, &d->uvs[i].u) || ctx(r, "u"), "UV texture coordinate %" PRIu32 " of %" PRIu32, i, n);
        TRY(read_f32(r, &d->uvs[i].v) || ctx(r, "v"), "UV texture coordinate %" PRIu32 " of %" PRIu32, i, n);
    }
    for (uint32_t i = 0; i < n; i++)
        TRY(read_color(r, &d->colors[i]), "face color %" PRIu32 " of %" PRIu32, i, n);
    return true;
}

static bool read_tag(reader *r, map_tag *t)
{
    TRY(read_u16(r, &t->coord1[0]), "coord1 p1");
    TRY(read_u16(r, &t->coord1[1]), "coord1 p2");
    TRY(read_u16(r, &t->coord1[2]), "coord1 p3");
    TRY(read_u16(r, &t->face_index_1), "face index 1");
    TRY(read_u16(r, &t->coord2[0]), "coord2 p1");
    TRY(read_u16(r, &t->coord2[1]), "coord2 p2");
    TRY(read_u16(r, &t->coord2[2]), "coord2 p3");
    TRY(read_u16(r, &t->face_index_2), "face index 2");
    return true;
}

static bool read_collision_group(reader *r, map_collision_group *g)
{
    TRY(read_string(r, &g->name), "collision group name");
    TRY(read_u32(r, &g->flags), "collision group flags");
    TRY(read_u32(r, &g->index_count), "collision group indices count");
    NEW(g->indices, g->index_count, NOZERO);
    for (uint32_t i = 0; i < g->index_count; i++)
        TRY_(read_u16(r, &g->indices[i]));
    return read_utf8(r, g->name);
}

static bool read_object(reader *r, map_object *o)
{
    // An object has two section headers, for reasons unknown
    TRY(read_section(r, &o->section), "section header");
    TRY(read_section(r, &o->object_section), "object section header");

    TRY(read_u32(r, &o->vertex_count), "vertex count");
    NEW(o->vertices, o->vertex_count, NOZERO);
    for (uint32_t i = 0; i < o->vertex_count; i++)
        TRY_(read_vec3(r, &o->vertices[i]));

    TRY(read_u32(r, &o->data_count), "objects data count");
    NEW(o->datas, o->data_count);
    for (uint32_t i = 0; i < o->data_count; i++)
        TRY_(read_object_data(r, &o->datas[i]));

    TRY(read_u32(r, &o->collision_vertex_count), "collision vertices count");
    NEW(o->collision_vertices, o->collision_vertex_count, NOZERO);
    for (uint32_t i = 0; i < o->collision_vertex_count; i++)
        TRY(read_vec3(r, &o->collision_vertices[i]), "collision vertex %" PRIu32, i);
    TRY(read_u32(r, &o->collision_plane_count), "collision faces count");
    NEW(o->collision_planes, o->collision_plane_count, NOZERO);
    for (uint32_t i = 0; i < o->collision_plane_count; i++)
        TRY(read_plane(r, &o->collision_planes[i]), "collision face normal %" PRIu32, i);

    TRY(read_u32(r, &o->tag_count), "object tag count");
    NEW(o->tags, o->tag_count, NOZERO);
    for (uint32_t i = 0; i < o->tag_count; i++)
        TRY_(read_tag(r, &o->tags[i]));

    TRY(read_u32(r, &o->group_count), "collision group count");
    NEW(o->groups, o->group_count);
    for (uint32_t i = 0; i < o->group_count; i++)
        TRY(read_collision_group(r, &o->groups[i]), "collision group %" PRIu32, i);
    return true;
}

static bool read_geometries(reader *r, map *m)
{
    TRY(read_section(r, &m->geometries.section), "geometry list section header");
    TRY(read_u32(r, &m->geometries.count), "missing number of objects");
    NEW(m->geometries.items, m->geometries.count);
    for (uint32_t i = 0; i < m->geometries.count; i++)
        TRY_(read_object(r, &m->geometries.items[i]));
    return true;
}

// --- Portals and lights -----------------------------------------------------

static bool read_portal(reader *r, map_portal *p)
{
    TRY(read_section(r, &p->section), "portal");
    TRY(read_u32(r, &p->vertex_count), "coordinates count");
    NEW(p->vertices, p->vertex_count, NOZERO);
    for (uint32_t i = 0; i < p->vertex_count; i++)
        TRY(read_vec3(r, &p->vertices[i]), "coordinate vertex %" PRIu32, i);
    TRY(read_u32(r, &p->room), "room");
    TRY(read_u32(r, &p->opposite_room), "opposite room");
    return true;
}

static bool read_portals(reader *r, map *m)
{
    TRY(read_section(r, &m->portals.section), "portals");
    TRY(read_u32(r, &m->portals.count), "portal count");
    NEW(m->portals.items, m->portals.count);
    for (uint32_t i = 0; i < m->portals.count; i++)
        TRY(read_portal(r, &m->portals.items[i]), "portal %" PRIu32, i);
    return true;
}

// No Rogue Spear map has any lights: only the count is read
static bool read_lights(reader *r, map *m)
{
    TRY(read_section(r, &m->lights.section), "lights");
    TRY(read_u32(r, &m->lights.count), "light count");
    return true;
}

// --- Dynamic objects --------------------------------------------------------

// A count, then that many strings. `item` is the context for one of them, given
// its index and the count, as in "ADT name %u" or "room name %u of %u".
static bool read_strings(reader *r, uint32_t *count, char ***out, const char *count_what, const char *item)
{
    TRY(read_u32(r, count), "%s", count_what);
    NEW(*out, *count, NOZERO);
    for (uint32_t i = 0; i < *count; i++)
        TRY(read_string(r, &(*out)[i]), item, i, *count);
    return true;
}

static bool read_floats(reader *r, float *out, int n, const char *what)
{
    for (int i = 0; i < n; i++)
        TRY(read_f32(r, &out[i]), "%s %d", what, i);
    return true;
}

static bool read_dynamic_common(reader *r, map_dynamic_common *c)
{
    TRY(read_transform(r, &c->transform), "transformation matrix");
    TRY(read_string(r, &c->name), "name");
    TRY(read_u32(r, &c->unknown1), "unknown1");
    for (int i = 0; i < 4; i++)
        TRY(read_string(r, &c->sounds[i]), "sound %d", i);
    TRY(read_string(r, &c->collision_type_2d), "2D collision type");
    TRY(read_string(r, &c->collision_type_3d), "3D collision type");
    TRY(read_string(r, &c->destruction_action), "destruction action");
    TRY(read_string(r, &c->destruction_category), "destruction category");
    TRY(read_string(r, &c->penetration_type), "penetration type");
    TRY(read_string(r, &c->name2), "name2");
    TRY(read_string(r, &c->destruction_category2), "destruction category 2");
    return true;
}

static bool read_dynamic_param(reader *r, map_dynamic_param *p)
{
    TRY(read_string(r, &p->name), "dynamic object kind struct name");
    TRY_(read_floats(r, p->unknown1, 9, "dynamic object kind struct unknown1"));
    for (int i = 0; i < 2; i++)
        TRY(read_u32(r, &p->unknown2[i]), "dynamic object kind struct unknown2 %d", i);
    return true;
}

static bool read_dynamic(reader *r, map_dynamic_object *o)
{
    TRY_(read_dynamic_common(r, &o->dynamic.common));
    uint32_t count;
    TRY(read_u32(r, &count), "dynamic object kind count");
    if (count > 0) {
        o->dynamic.param_count = count;
        NEW(o->dynamic.params, count);
        for (uint32_t i = 0; i < count; i++)
            TRY(read_dynamic_param(r, &o->dynamic.params[i]), "dynamic object kind struct %" PRIu32 " of %" PRIu32, i, count);
    } else {
        // A second count, for the names
        TRY_(read_strings(r, &o->dynamic.name_count, &o->dynamic.names, "dynamic object flat count", "dynamic object kind flat name %" PRIu32 " of %" PRIu32));
        TRY_(read_floats(r, o->dynamic.unknown, 4, "dynamic object kind flat unknown"));
    }
    return true;
}

static bool read_animation(reader *r, map_dynamic_object *o)
{
    TRY_(read_dynamic_common(r, &o->animation.common));
    TRY(read_u32(r, &o->animation.unknown2), "unknown2");
    TRY_(read_strings(r, &o->animation.name_count, &o->animation.names, "name count", "animation name %" PRIu32));
    TRY_(read_floats(r, o->animation.unknown3, 3, "animation unknown3"));
    TRY(read_u32(r, &o->animation.unknown4), "animation unknown4");
    TRY(read_string(r, &o->animation.name3), "animation name3");
    TRY(read_string(r, &o->animation.name4), "animation name4");
    TRY(read_string(r, &o->animation.animation_type), "animation type");
    TRY(read_vec3(r, &o->animation.direction), "animation direction");
    TRY(read_f32(r, &o->animation.distance), "animation distance");
    TRY(read_f32(r, &o->animation.velocity), "animation velocity");
    return true;
}

// Often named "ADT", perhaps for "automatic door touchplate"
static bool read_repeatable_touchplate(reader *r, map_dynamic_object *o)
{
    TRY_(read_dynamic_common(r, &o->repeatable_touchplate.common));
    TRY(read_u32(r, &o->repeatable_touchplate.unknown1), "ADT unknown1");
    TRY_(read_strings(r, &o->repeatable_touchplate.attachment_count, &o->repeatable_touchplate.attachments,
                      "ADT attachment count", "ADT attachment %" PRIu32));
    TRY_(read_floats(r, o->repeatable_touchplate.unknown2, 3, "ADT unknown2"));
    TRY_(read_strings(r, &o->repeatable_touchplate.name_count, &o->repeatable_touchplate.names, "name count", "ADT name %" PRIu32));
    TRY(read_string(r, &o->repeatable_touchplate.name2), "ADT name2");
    TRY(read_string(r, &o->repeatable_touchplate.name3), "ADT name3");
    TRY(read_string(r, &o->repeatable_touchplate.animation_type), "animation type");
    TRY(read_vec3(r, &o->repeatable_touchplate.direction), "animation direction");
    TRY(read_f32(r, &o->repeatable_touchplate.distance), "animation distance");
    TRY(read_f32(r, &o->repeatable_touchplate.velocity), "animation velocity");
    return true;
}

static bool read_glass(reader *r, map_dynamic_object *o)
{
    TRY_(read_string(r, &o->glass.name));
    return read_utf8(r, o->glass.name);
}

static bool read_one_time_touchplate(reader *r, map_dynamic_object *o)
{
    float *c = o->one_time_touchplate.coordinates;
    TRY(read_string(r, &o->one_time_touchplate.collision_type_2d), "one-time touchplate 2D collision type");
    TRY(read_string(r, &o->one_time_touchplate.collision_type_3d), "one-time touchplate 3D collision type");
    TRY(read_xyz(r, &c[0], &c[1], &c[2]) && read_xyz(r, &c[3], &c[4], &c[5]), "one-time touchplate coordinates");
    TRY_(read_strings(r, &o->one_time_touchplate.attachment_count, &o->one_time_touchplate.attachments,
                      "one-time touchplate attachment count", "one-time touchplate attachment name %" PRIu32 " of %" PRIu32));
    return true;
}

static bool read_halo(reader *r, map_dynamic_object *o)
{
    uint32_t n;
    TRY(read_u32(r, &n), "halo count");
    o->halo.count = n;
    NEW(o->halo.halos, n);
    for (uint32_t i = 0; i < n; i++) {
        float *v = o->halo.halos[i].values;
        TRY(read_string(r, &o->halo.halos[i].name), "halo name %" PRIu32 " of %" PRIu32, i, n);
        bool ok = read_xyz(r, &v[0], &v[1], &v[2]) && read_xyz(r, &v[3], &v[4], &v[5])
            && (read_f32(r, &v[6]) || ctx(r, "x")) && (read_f32(r, &v[7]) || ctx(r, "y"));
        TRY(ok, "halo vec %" PRIu32 " of %" PRIu32, i, n);
    }
    return true;
}

static bool read_dynamic_object(reader *r, map_dynamic_object *o)
{
    TRY(read_section(r, &o->section), "dynamic object section header");
    TRY(read_string(r, &o->name), "name");
    TRY(read_transform(r, &o->transform), "transformation matrix");
    o->kind = o->section.id;
    switch (o->kind) {
    case MAP_DYNAMIC: return read_dynamic(r, o);
    case MAP_ANIMATION: return read_animation(r, o);
    case MAP_REPEATABLE_TOUCHPLATE: return read_repeatable_touchplate(r, o);
    case MAP_GLASS: return read_glass(r, o);
    case MAP_ONE_TIME_TOUCHPLATE: return read_one_time_touchplate(r, o);
    case MAP_HALO: return read_halo(r, o);
    case MAP_STATIC_EFFECT: return true; // nothing more
    }
    return fail(r, "unhandled id value: %" PRIu32, o->section.id);
}

static bool read_dynamic_objects(reader *r, map *m)
{
    uint32_t n;
    TRY(read_section(r, &m->dynamic_objects.section), "dynamic objects section header");
    TRY(read_u32(r, &n), "dynamic object count");
    m->dynamic_objects.count = n;
    NEW(m->dynamic_objects.items, n);
    for (uint32_t i = 0; i < n; i++)
        TRY(read_dynamic_object(r, &m->dynamic_objects.items[i]), "dynamic object %" PRIu32 " of %" PRIu32, i, n);
    return true;
}

// --- Rooms ------------------------------------------------------------------

static bool read_box(reader *r, map_box *b)
{
    TRY(read_transform(r, &b->transform), "TM + AABB");
    for (int i = 0; i < 6; i++)
        TRY(read_f32(r, &b->aabb[i]), "level TM + AABB side %d", i);
    return true;
}

static bool read_level(reader *r, map_level *l)
{
    TRY(read_string(r, &l->name), "level name");
    TRY(read_u32(r, &l->box_count), "level TM + AABB count");
    NEW(l->boxes, l->box_count, NOZERO);
    for (uint32_t i = 0; i < l->box_count; i++)
        TRY(read_box(r, &l->boxes[i]), "level TM + AABBB %" PRIu32, i);
    TRY(read_u32(r, &l->floor_count), "level floor count");
    NEW(l->floors, l->floor_count, NOZERO);
    for (uint32_t i = 0; i < l->floor_count; i++)
        TRY(read_f32(r, &l->floors[i]), "level floor %" PRIu32, i);
    TRY(read_u8(r, &l->has_model), "level has model");
    if (l->has_model)
        return fail(r, "a level with a model, which is not read");
    return true;
}

static bool read_room(reader *r, map_room *room)
{
    TRY(read_short_section(r, &room->section), "room section header short");
    TRY(read_u8(r, &room->enclosed), "room enclosed");
    if (room->enclosed == 0) {
        room->has_small = true;
        TRY(read_u8(r, &room->small), "room small");
        if (room->small > 1)
            return fail(r, "unknown room small value: %u", room->small);
    } else if (room->enclosed > 1) {
        return fail(r, "unknown room enclosed value: %u", room->enclosed);
    }
    TRY(read_u8(r, &room->unknown_flag), "room unknown flag");
    if (room->section.version > 1) {
        room->has_weather_flag = true;
        TRY(read_u8(r, &room->has_weather_bounds), "room has weather bounds");
        if (room->has_weather_bounds) {
            TRY(read_vec3(r, &room->weather_min), "room weather bounds min");
            TRY(read_vec3(r, &room->weather_max), "room weather bounds max");
        }
    }

    uint32_t n;
    TRY(read_u32(r, &n), "room level count");
    room->level_count = n;
    NEW(room->levels, n);
    for (uint32_t i = 0; i < n; i++)
        TRY(read_level(r, &room->levels[i]), "room sherman level %" PRIu32 " of %" PRIu32, i, n);

    TRY(read_u32(r, &n), "room floor count");
    room->floor_count = n;
    NEW(room->floors, n, NOZERO);
    for (uint32_t i = 0; i < n; i++) {
        map_room_floor *f = &room->floors[i];
        bool ok = (read_f32(r, &f->number) || ctx(r, "number")) && (read_f32(r, &f->height) || ctx(r, "height"));
        TRY(ok, "room floor %" PRIu32 " of %" PRIu32, i, n);
    }

    TRY(read_u32(r, &n), "room unknown count");
    room->unknown_count = n;
    NEW(room->unknown, n, NOZERO);
    for (uint32_t i = 0; i < n; i++)
        TRY(read_u32(r, &room->unknown[i]), "room unknown %" PRIu32, i);
    return true;
}

static bool read_rooms(reader *r, map *m)
{
    TRY(read_section(r, &m->rooms.section), "room list");
    TRY(read_u32(r, &m->rooms.count), "room count");
    NEW(m->rooms.items, m->rooms.count);
    for (uint32_t i = 0; i < m->rooms.count; i++)
        TRY(read_room(r, &m->rooms.items[i]), "room %" PRIu32, i);
    return true;
}

// --- Transitions and planning levels ----------------------------------------

static bool read_transition(reader *r, map_transition *t)
{
    TRY(read_string(r, &t->name), "transition");
    TRY(read_vec3(r, &t->p1), "transition coords P1");
    TRY(read_vec3(r, &t->p2), "transition coords P2");
    return true;
}

static bool read_transitions(reader *r, map *m)
{
    uint32_t n;
    TRY(read_section(r, &m->transitions.section), "transitions");
    TRY(read_u32(r, &n), "transitions count");
    m->transitions.count = n;
    NEW(m->transitions.items, n);
    for (uint32_t i = 0; i < n; i++)
        TRY(read_transition(r, &m->transitions.items[i]), "transition %" PRIu32 " of %" PRIu32, i, n);
    return true;
}

static bool read_planning_level(reader *r, map_planning_level *l)
{
    TRY(read_f32(r, &l->level_number), "planning level number");
    TRY(read_f32(r, &l->floor_height), "planning level floor height");
    return read_strings(r, &l->room_count, &l->room_names, "planning level room count", "planning level room name %" PRIu32 " of %" PRIu32);
}

static bool read_planning_levels(reader *r, map *m)
{
    uint32_t n;
    TRY(read_section(r, &m->planning_levels.section), "planning levels");
    TRY(read_u32(r, &n), "planning levels count");
    m->planning_levels.count = n;
    NEW(m->planning_levels.items, n);
    for (uint32_t i = 0; i < n; i++)
        TRY(read_planning_level(r, &m->planning_levels.items[i]), "planning level %" PRIu32 " of %" PRIu32, i, n);
    return true;
}

// --- The file ---------------------------------------------------------------

static bool read_magic(reader *r, const char *what, const char *want, const char *bad)
{
    char *s;
    uint32_t len;
    TRY(read_string_len(r, &s, &len), "%s", what);
    if (len != strlen(want) || memcmp(s, want, len) != 0)
        return fail(r, "%s", bad);
    return true;
}

static bool read_header(reader *r, map *m)
{
    TRY_(read_magic(r, "missing magic", "BeginMapv2.1", "incorrect magic"));
    TRY(read_u32(r, &m->timestamp), "failed to read file creation timestamp");
    return true;
}

static bool read_map(reader *r, map *m)
{
    TRY(read_header(r, m), "MAP Header");
    TRY(read_materials(r, m), "Materials List");
    TRY(read_geometries(r, m), "Geometry List");
    TRY(read_portals(r, m), "Portal List");
    TRY(read_lights(r, m), "Light List");
    TRY(read_dynamic_objects(r, m), "Dynamic Object List");
    TRY(read_rooms(r, m), "Rooms List");
    TRY(read_transitions(r, m), "Transition List");
    TRY(read_planning_levels(r, m), "Planning Levels List");
    // Anything after the end is left alone
    return read_magic(r, "end", "EndMap", "missing MAP end");
}

map_result map_parse(const void *data, ptrdiff_t size, arena *perm)
{
    // Kept only if the whole map reads
    arena scratch = *perm;
    reader r = { .p = data, .end = (const uint8_t *)data + size, .perm = &scratch };
    map *m = new(&scratch, map, 1);
    if (!m)
        return (map_result){ .err = "out of memory" };
    if (!read_map(&r, m)) {
        // The message was built in the reader, so it is kept in the arena
        ptrdiff_t len = (ptrdiff_t)strlen(r.err) + 1;
        char *err = new(perm, char, len, NOZERO);
        if (!err)
            return (map_result){ .err = "out of memory" };
        memcpy(err, r.err, len);
        return (map_result){ .err = err };
    }
    perm->beg = scratch.beg;
    return (map_result){ .map = m };
}

map_result map_read(const char *path, arena *perm)
{
    FILE *f = fopen(path, "rb");
    if (!f)
        return (map_result){ .err = "could not open MAP file" };
    long size = fseek(f, 0, SEEK_END) ? -1 : ftell(f);
    if (size < 0 || fseek(f, 0, SEEK_SET)) {
        fclose(f);
        return (map_result){ .err = "failed to read MAP file" };
    }
    if (size > perm->end - perm->beg) {
        fclose(f);
        return (map_result){ .err = "MAP file is larger than the memory left" };
    }

    // The file comes off the top, so it is not kept when the map is
    arena scratch = *perm;
    scratch.end -= size;
    char *data = scratch.end;
    bool failed = fread(data, 1, size, f) != (size_t)size;
    fclose(f);
    if (failed)
        return (map_result){ .err = "failed to read MAP file" };
    // What the parse kept, the map or only its message, is kept here too
    map_result result = map_parse(data, size, &scratch);
    perm->beg = scratch.beg;
    return result;
}
