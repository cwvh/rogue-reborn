// mapdump [--full] MAP...
//
// Reads each .MAP file and prints a summary of it, or with --full every field
// it holds, with floats as their bits in hex: the same text that
// examples/reader-dump.rs prints from the Rust reader, to compare the two.
#include "map.h"

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif

static const char *kind_name(map_dynamic_kind k)
{
    switch (k) {
    case MAP_DYNAMIC: return "dynamic";
    case MAP_ANIMATION: return "animation";
    case MAP_REPEATABLE_TOUCHPLATE: return "repeatable touchplate";
    case MAP_GLASS: return "glass";
    case MAP_ONE_TIME_TOUCHPLATE: return "one-time touchplate";
    case MAP_HALO: return "halo";
    case MAP_STATIC_EFFECT: return "static effect";
    }
    return "?";
}

static void summary(const char *path, const map *m)
{
    time_t t = m->timestamp;
    char when[32];
    strftime(when, sizeof when, "%Y-%m-%d %H:%M:%S", gmtime(&t));
    printf("%s: written %s UTC\n", path, when);

    printf("  %-16s %6" PRIu32 "\n", "materials", m->materials.count);

    uint64_t vertices = 0, faces = 0, tags = 0, groups = 0;
    for (uint32_t i = 0; i < m->geometries.count; i++) {
        const map_object *o = &m->geometries.items[i];
        vertices += o->vertex_count;
        tags += o->tag_count;
        groups += o->group_count;
        for (uint32_t j = 0; j < o->data_count; j++)
            faces += o->datas[j].face_count;
    }
    printf("  %-16s %6" PRIu32 "  (%" PRIu64 " vertices, %" PRIu64 " triangles, %" PRIu64 " collision faces in %" PRIu64 " groups)\n",
           "objects", m->geometries.count, vertices, faces, tags, groups);
    printf("  %-16s %6" PRIu32 "\n", "portals", m->portals.count);
    printf("  %-16s %6" PRIu32 "\n", "lights", m->lights.count);

    printf("  %-16s %6" PRIu32, "dynamic objects", m->dynamic_objects.count);
    const map_dynamic_kind kinds[] = { MAP_DYNAMIC, MAP_ANIMATION, MAP_REPEATABLE_TOUCHPLATE, MAP_GLASS,
                                       MAP_ONE_TIME_TOUCHPLATE, MAP_HALO, MAP_STATIC_EFFECT };
    const char *sep = "  (";
    for (size_t k = 0; k < sizeof kinds / sizeof *kinds; k++) {
        uint32_t n = 0;
        for (uint32_t i = 0; i < m->dynamic_objects.count; i++)
            n += m->dynamic_objects.items[i].kind == kinds[k];
        if (n) {
            printf("%s%" PRIu32 " %s", sep, n, kind_name(kinds[k]));
            sep = ", ";
        }
    }
    printf("%s\n", *sep == ',' ? ")" : "");

    uint64_t boxes = 0;
    for (uint32_t i = 0; i < m->rooms.count; i++)
        for (uint32_t j = 0; j < m->rooms.items[i].level_count; j++)
            boxes += m->rooms.items[i].levels[j].box_count;
    printf("  %-16s %6" PRIu32 "  (%" PRIu64 " volumes)\n", "rooms", m->rooms.count, boxes);
    printf("  %-16s %6" PRIu32 "\n", "transitions", m->transitions.count);
    printf("  %-16s %6" PRIu32 "\n", "planning levels", m->planning_levels.count);
    for (uint32_t i = 0; i < m->planning_levels.count; i++) {
        const map_planning_level *l = &m->planning_levels.items[i];
        printf("    floor %g at height %g: %" PRIu32 " rooms\n", l->level_number, l->floor_height, l->room_count);
    }
}

// --- Everything, to compare with the Rust reader ----------------------------

static void f(float x)
{
    uint32_t bits;
    memcpy(&bits, &x, sizeof bits);
    printf(" %08" PRIx32, bits);
}

static void fs(const float *x, int n)
{
    for (int i = 0; i < n; i++)
        f(x[i]);
}

static void s(const char *str)
{
    putchar(' ');
    putchar('"');
    for (const unsigned char *c = (const unsigned char *)str; *c; c++) {
        if (*c == '"' || *c == '\\')
            printf("\\%c", *c);
        else if (*c < 0x20 || *c >= 0x7f)
            printf("\\x%02x", *c);
        else
            putchar(*c);
    }
    putchar('"');
}

static void strs(char *const *list, uint32_t n)
{
    printf(" %" PRIu32, n);
    for (uint32_t i = 0; i < n; i++)
        s(list[i]);
}

static void v3(map_vec3 v)
{
    f(v.x), f(v.y), f(v.z);
}

static void tm(map_transform t)
{
    v3(t.x_axis), v3(t.y_axis), v3(t.z_axis), v3(t.position);
}

static void color(map_color c)
{
    f(c.r), f(c.g), f(c.b), f(c.a);
}

static void plane(map_plane p)
{
    f(p.x), f(p.y), f(p.z), f(p.distance);
}

static void section(const char *what, map_section sec, uint32_t count)
{
    printf("%s %" PRIu32, what, sec.id);
    s(sec.name);
    printf(" %" PRIu32 "\n", count);
}

static void common(const map_dynamic_common *c)
{
    printf("common");
    tm(c->transform);
    s(c->name);
    printf(" %" PRIu32, c->unknown1);
    strs(c->sounds, 4);
    s(c->collision_type_2d), s(c->collision_type_3d), s(c->destruction_action), s(c->destruction_category);
    s(c->penetration_type), s(c->name2), s(c->destruction_category2);
    printf("\n");
}

static void dynamic_object(const map_dynamic_object *o)
{
    printf("dynamic %" PRIu32, o->section.id);
    s(o->section.name), s(o->name);
    tm(o->transform);
    printf("\n");
    switch (o->kind) {
    case MAP_DYNAMIC:
        common(&o->dynamic.common);
        if (o->dynamic.param_count) {
            for (uint32_t i = 0; i < o->dynamic.param_count; i++) {
                const map_dynamic_param *p = &o->dynamic.params[i];
                printf("param");
                s(p->name);
                fs(p->unknown1, 9);
                printf(" %" PRIu32 " %" PRIu32 "\n", p->unknown2[0], p->unknown2[1]);
            }
        } else {
            printf("flat");
            strs(o->dynamic.names, o->dynamic.name_count);
            fs(o->dynamic.unknown, 4);
            printf("\n");
        }
        break;
    case MAP_ANIMATION:
        common(&o->animation.common);
        printf("animation %" PRIu32, o->animation.unknown2);
        strs(o->animation.names, o->animation.name_count);
        fs(o->animation.unknown3, 3);
        printf(" %" PRIu32, o->animation.unknown4);
        s(o->animation.name3), s(o->animation.name4), s(o->animation.animation_type);
        v3(o->animation.direction), f(o->animation.distance), f(o->animation.velocity);
        printf("\n");
        break;
    case MAP_REPEATABLE_TOUCHPLATE:
        common(&o->repeatable_touchplate.common);
        printf("touchplate %" PRIu32, o->repeatable_touchplate.unknown1);
        strs(o->repeatable_touchplate.attachments, o->repeatable_touchplate.attachment_count);
        fs(o->repeatable_touchplate.unknown2, 3);
        strs(o->repeatable_touchplate.names, o->repeatable_touchplate.name_count);
        s(o->repeatable_touchplate.name2), s(o->repeatable_touchplate.name3), s(o->repeatable_touchplate.animation_type);
        v3(o->repeatable_touchplate.direction), f(o->repeatable_touchplate.distance), f(o->repeatable_touchplate.velocity);
        printf("\n");
        break;
    case MAP_GLASS:
        printf("glass");
        s(o->glass.name);
        printf("\n");
        break;
    case MAP_ONE_TIME_TOUCHPLATE:
        printf("once");
        s(o->one_time_touchplate.collision_type_2d), s(o->one_time_touchplate.collision_type_3d);
        fs(o->one_time_touchplate.coordinates, 6);
        strs(o->one_time_touchplate.attachments, o->one_time_touchplate.attachment_count);
        printf("\n");
        break;
    case MAP_HALO:
        for (uint32_t i = 0; i < o->halo.count; i++) {
            printf("halo");
            s(o->halo.halos[i].name);
            fs(o->halo.halos[i].values, 8);
            printf("\n");
        }
        break;
    case MAP_STATIC_EFFECT:
        break;
    }
}

static void full(const map *m)
{
    printf("timestamp %" PRIu32 "\n", m->timestamp);

    printf("materials %" PRIu32 " %" PRIu32 "\n", m->materials.section.id, m->materials.count);
    for (uint32_t i = 0; i < m->materials.count; i++) {
        const map_material *t = &m->materials.items[i];
        printf("material %" PRIu32, t->section.id);
        s(t->section.name), s(t->filename), f(t->opacity);
        printf(" %" PRIu32 " %d", t->emissive_strength, (int)t->address_mode);
        color(t->ambient), color(t->diffuse), color(t->specular), f(t->specular_level);
        printf(" %d\n", t->two_sided);
    }

    printf("geometries %" PRIu32 " %" PRIu32 "\n", m->geometries.section.id, m->geometries.count);
    for (uint32_t i = 0; i < m->geometries.count; i++) {
        const map_object *o = &m->geometries.items[i];
        printf("object %" PRIu32, o->section.id);
        s(o->section.name);
        printf(" %" PRIu32, o->object_section.id);
        s(o->object_section.name);
        printf("\n");
        for (uint32_t j = 0; j < o->vertex_count; j++)
            printf("v"), v3(o->vertices[j]), printf("\n");
        for (uint32_t j = 0; j < o->data_count; j++) {
            const map_object_data *d = &o->datas[j];
            printf("data %" PRIu32 " %" PRIu32 " %" PRIu32 "\n", d->material, d->face_count, d->texture_vertex_count);
            for (uint32_t k = 0; k < d->face_count; k++) {
                printf("face"), plane(d->face_planes[k]);
                printf(" %u %u %u", d->faces[k].a, d->faces[k].b, d->faces[k].c);
                printf(" %u %u %u\n", d->face_texture_verts[k].a, d->face_texture_verts[k].b, d->face_texture_verts[k].c);
            }
            for (uint32_t k = 0; k < d->texture_vertex_count; k++) {
                printf("tv"), v3(d->normals[k]), f(d->uvs[k].u), f(d->uvs[k].v), color(d->colors[k]);
                printf("\n");
            }
        }
        for (uint32_t j = 0; j < o->collision_vertex_count; j++)
            printf("cv"), v3(o->collision_vertices[j]), printf("\n");
        for (uint32_t j = 0; j < o->collision_plane_count; j++)
            printf("cp"), plane(o->collision_planes[j]), printf("\n");
        for (uint32_t j = 0; j < o->tag_count; j++) {
            const map_tag *t = &o->tags[j];
            printf("tag %u %u %u %u %u %u %u %u\n", t->coord1[0], t->coord1[1], t->coord1[2], t->face_index_1,
                   t->coord2[0], t->coord2[1], t->coord2[2], t->face_index_2);
        }
        for (uint32_t j = 0; j < o->group_count; j++) {
            const map_collision_group *g = &o->groups[j];
            printf("group");
            s(g->name);
            printf(" %" PRIu32 " %" PRIu32, g->flags, g->index_count);
            for (uint32_t k = 0; k < g->index_count; k++)
                printf(" %u", g->indices[k]);
            printf("\n");
        }
    }

    section("portals", m->portals.section, m->portals.count);
    for (uint32_t i = 0; i < m->portals.count; i++) {
        const map_portal *p = &m->portals.items[i];
        printf("portal %" PRIu32, p->section.id);
        s(p->section.name);
        printf(" %" PRIu32 " %" PRIu32 " %" PRIu32, p->room, p->opposite_room, p->vertex_count);
        for (uint32_t j = 0; j < p->vertex_count; j++)
            v3(p->vertices[j]);
        printf("\n");
    }

    section("lights", m->lights.section, m->lights.count);

    section("dynamic_objects", m->dynamic_objects.section, m->dynamic_objects.count);
    for (uint32_t i = 0; i < m->dynamic_objects.count; i++)
        dynamic_object(&m->dynamic_objects.items[i]);

    section("rooms", m->rooms.section, m->rooms.count);
    for (uint32_t i = 0; i < m->rooms.count; i++) {
        const map_room *r = &m->rooms.items[i];
        printf("room %" PRIu32, r->section.id);
        s(r->section.name);
        printf(" %u", r->enclosed);
        if (r->has_small)
            printf(" %u", r->small);
        else
            printf(" -");
        printf(" %u", r->unknown_flag);
        if (r->has_weather_flag)
            printf(" %u", r->has_weather_bounds);
        else
            printf(" -");
        if (r->has_weather_bounds)
            v3(r->weather_min), v3(r->weather_max);
        else
            printf(" -");
        printf("\n");
        for (uint32_t j = 0; j < r->level_count; j++) {
            const map_level *l = &r->levels[j];
            printf("level");
            s(l->name);
            printf(" %u %" PRIu32, l->has_model, l->floor_count);
            fs(l->floors, (int)l->floor_count);
            printf("\n");
            for (uint32_t k = 0; k < l->box_count; k++)
                printf("box"), tm(l->boxes[k].transform), fs(l->boxes[k].aabb, 6), printf("\n");
        }
        for (uint32_t j = 0; j < r->floor_count; j++)
            printf("floor"), f(r->floors[j].number), f(r->floors[j].height), printf("\n");
        printf("unknown %" PRIu32, r->unknown_count);
        for (uint32_t j = 0; j < r->unknown_count; j++)
            printf(" %08" PRIx32, r->unknown[j]);
        printf("\n");
    }

    section("transitions", m->transitions.section, m->transitions.count);
    for (uint32_t i = 0; i < m->transitions.count; i++) {
        const map_transition *t = &m->transitions.items[i];
        printf("transition");
        s(t->name), v3(t->p1), v3(t->p2);
        printf("\n");
    }

    section("planning_levels", m->planning_levels.section, m->planning_levels.count);
    for (uint32_t i = 0; i < m->planning_levels.count; i++) {
        const map_planning_level *l = &m->planning_levels.items[i];
        printf("planning"), f(l->level_number), f(l->floor_height);
        strs(l->room_names, l->room_count);
        printf("\n");
    }
}

int main(int argc, char **argv)
{
    bool all = false;
    int status = 0;
#ifdef _WIN32
    // Lines end in LF, not CRLF, so the output is the same everywhere
    _setmode(_fileno(stdout), _O_BINARY);
    _setmode(_fileno(stderr), _O_BINARY);
#endif
    // The biggest map, rm16, takes 3.6 MB read from a 3.3 MB file
    constexpr ptrdiff_t ARENA_SIZE = 32 << 20;
    char *mem = malloc(ARENA_SIZE);
    if (!mem) {
        fprintf(stderr, "mapdump: out of memory\n");
        return 1;
    }
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--full") == 0) {
            all = true;
            continue;
        }
        arena perm = { mem, mem + ARENA_SIZE };
        map_result r = map_read(argv[i], &perm);
        if (r.err) {
            fprintf(stderr, "%s: %s\n", argv[i], r.err);
            status = 1;
            continue;
        }
        if (all)
            full(r.map);
        else
            summary(argv[i], r.map);
    }
    if (argc < 2) {
        fprintf(stderr, "usage: mapdump [--full] MAP...\n");
        return 2;
    }
    return status;
}
