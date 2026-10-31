// mapbench MAP...
//
// Times reading each map, printing one line per measurement:
//   c STEP MAP BYTES RUNS MIN_NS MEDIAN_NS
// where STEP is "read" (map_read from the path), "parse" (map_parse of the
// file's bytes already in memory) or "file" (only reading the file).
// examples/reader-bench.rs prints the same lines for the Rust reader.
//
// Built with MAPK defined to a label, as in -DMAPK='"ki"', it times the reader
// of mapk.h instead and prints that label in place of "c". LABEL alone changes
// only the label, as for map2.c.
#ifdef MAPK
#include "mapk.h"
#define LABEL MAPK
#else
#include "map.h"
#ifndef LABEL
#define LABEL "c"
#endif
#endif

#include <stdio.h>
#include <stdlib.h>

#ifdef _WIN32
#include <windows.h>
#else
#include <time.h>
#endif

static int64_t now_ns(void)
{
#ifdef _WIN32
    static LARGE_INTEGER freq;
    LARGE_INTEGER t;
    if (!freq.QuadPart)
        QueryPerformanceFrequency(&freq);
    QueryPerformanceCounter(&t);
    return (int64_t)((double)t.QuadPart * 1e9 / (double)freq.QuadPart);
#else
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (int64_t)t.tv_sec * 1000000000 + t.tv_nsec;
#endif
}

constexpr ptrdiff_t ARENA_SIZE = 32 << 20;
constexpr int MAX_RUNS = 100000;

typedef struct {
    const char *path;
    char *mem;       // the arena's memory
    char *data;      // the file's bytes, for "parse"
    ptrdiff_t size;
} job;

static int64_t times[MAX_RUNS];

static int by_value(const void *a, const void *b)
{
    int64_t x = *(const int64_t *)a, y = *(const int64_t *)b;
    return (x > y) - (x < y);
}

#ifdef MAPK
static bool read_step(job *j)
{
    return mapk_read(j->path, (uint32_t *)j->mem, ARENA_SIZE / 4);
}

static bool parse_step(job *j)
{
    return mapk_parse(j->data, j->size, (uint32_t *)j->mem, ARENA_SIZE / 4);
}
#else
static bool read_step(job *j)
{
    arena perm = { j->mem, j->mem + ARENA_SIZE };
    map_result r = map_read(j->path, &perm);
    return !r.err;
}

static bool parse_step(job *j)
{
    arena perm = { j->mem, j->mem + ARENA_SIZE };
    map_result r = map_parse(j->data, j->size, &perm);
    return !r.err;
}
#endif

// The file's reading as map_read does it
static bool file_step(job *j)
{
    FILE *f = fopen(j->path, "rb");
    if (!f)
        return false;
    long size = fseek(f, 0, SEEK_END) ? -1 : ftell(f);
    bool ok = size == j->size && !fseek(f, 0, SEEK_SET) && fread(j->mem + ARENA_SIZE - size, 1, size, f) == (size_t)size;
    fclose(f);
    return ok;
}

// Runs `step` twice to warm up, then until it has taken 300 ms and run 20 times
static bool bench(const char *name, bool (*step)(job *), job *j)
{
    for (int i = 0; i < 2; i++)
        if (!step(j))
            return false;
    int runs = 0;
    int64_t total = 0;
    while ((total < 300000000 || runs < 20) && runs < MAX_RUNS) {
        int64_t t0 = now_ns();
        if (!step(j))
            return false;
        times[runs] = now_ns() - t0;
        total += times[runs++];
    }
    qsort(times, runs, sizeof *times, by_value);
    printf(LABEL " %s %s %td %d %lld %lld\n", name, j->path, j->size, runs, (long long)times[0], (long long)times[runs / 2]);
    fflush(stdout);
    return true;
}

int main(int argc, char **argv)
{
    char *mem = malloc(ARENA_SIZE), *data = malloc(ARENA_SIZE);
    if (!mem || !data) {
        fprintf(stderr, "mapbench: out of memory\n");
        return 1;
    }
    for (int i = 1; i < argc; i++) {
        FILE *f = fopen(argv[i], "rb");
        if (!f) {
            fprintf(stderr, "%s: could not open\n", argv[i]);
            return 1;
        }
        job j = { .path = argv[i], .mem = mem, .data = data, .size = (ptrdiff_t)fread(data, 1, ARENA_SIZE, f) };
        fclose(f);
        if (!bench("read", read_step, &j) || !bench("parse", parse_step, &j) || !bench("file", file_step, &j)) {
            fprintf(stderr, "%s: failed to read\n", argv[i]);
            return 1;
        }
    }
    return 0;
}
