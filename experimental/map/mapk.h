// Reader for Rogue Spear .MAP files in the style of Arthur Whitney: mapki.c
// interprets the schemas below, mapkd.c is the same schemas written out as code.
//
// A map is one vector of 32-bit words w. Each record is a row of words laid out
// by its schema; the map's own row is at w[0]. A reference is a word index into
// w: a string's bytes, NUL-terminated, are at (char*)(w+i), a u16 array's at
// (uint16_t*)(w+i). Floats are kept as their bits.
//
// op   reads                                  words in the row
// u f  u32, f32; kf is k of them              1, k
// b    byte                                   1
// s    string; ks is k of them                1, k: index of its bytes
// z    string that must be UTF-8              1
// S    section: size id [Version n] name      4: size id n name
// R    section with no size                   4: 0 id n name
// n    count c                                1: c
// k#   c rows of k words                      1: index
// k%   c rows of k u16s                       1: index
// $    c strings                              1: index of c string indices
// @X   c rows of schema X                     1: index
// a    address mode, 0 1 or 3                 1
// e    enclosed byte, then where 0 small      2: enclosed, small or -1
// w    where version > 1 a has-bounds byte,   7: has bounds or -1, min, max
//      then where not 0 min and max
// !    byte that must be 0                    1
// ?    row of the dynamic kind of section id  1: index
// d    count, then that many K, or one F      2: count, index
// < >  "BeginMapv2.1", "EndMap"               0
//
// The schemas and widths are also read by the assembly readers, mapkai.S and mapkad.S.
#ifndef RR_MAPK_H
#define RR_MAPK_H

#ifndef __ASSEMBLER__
#include <stddef.h>
#include <stdint.h>
#endif

// materials geometries portals lights dynamic objects rooms transitions planning levels
#define K_MAP "<uSn@MSn@OSn@PSuSn@YSn@QSn@TSn@L>"
#define K_M "Ssfua13fb"              // material
#define K_O "SSn3#n@Dn3#n4#n8%n@G"   // object: vertices datas, collision vertices planes tags groups
#define K_D "un4#3%3%n3#2#4#"        // data: material, planes tris texture tris, normals uvs colors
#define K_G "zun1%"                  // collision group
#define K_P "Sn3#uu"                 // portal
#define K_Y "Ss12f?"                 // dynamic object
#define K_Q "Rebwn@Vn2#n1#"          // room: levels, floors, unknown
#define K_V "sn18#n1#!"              // level: boxes, floors, has model
#define K_T "s6f"                    // transition
#define K_L "2fn$"                   // planning level
#define K_K "s9f2u"                  // dynamic param
#define K_F "n$4f"                   // dynamic flat names
#define K_H "s8f"                    // halo
#define K_C "12fsu11s"               // what kinds 14 15 16 share
#define K_14 K_C "d"
#define K_15 K_C "un$3fu3s5f"
#define K_16 K_C "un$3fn$3s5f"
#define K_20 "z"
#define K_25 "2s6fn$"
#define K_31 "n@H"
#define K_36 ""

// The widths of the rows of each schema
#define wMAP 48
#define wM 22
#define wO 20
#define wD 9
#define wG 4
#define wP 8
#define wY 18
#define wQ 20
#define wV 6
#define wT 7
#define wL 4
#define wK 12
#define wF 6
#define wH 9
#define w14 27
#define w15 40
#define w16 41
#define w20 1
#define w25 10
#define w31 2
#define w36 0

#ifndef __ASSEMBLER__
// Read the map in d[0..n) into w[0..m), returning the words used, or 0 where it fails
uint32_t mapk_parse(const void *d, ptrdiff_t n, uint32_t *w, ptrdiff_t m);
// Read the file at `path` into w[0..m), which also holds the file's bytes while it is read
uint32_t mapk_read(const char *path, uint32_t *w, ptrdiff_t m);
#endif

#endif
