//! Reader for Rogue Spear character meshes (`.CRP`), bind pose only.
//!
//! The format is undocumented; this was worked out from the files, so it only
//! extracts what has been verified: model-space vertex positions, and the
//! triangles, UVs and normals of each material part. Animation is in the
//! motion files.
//!
//! Layout as understood:
//! - `f32` version (1.8), then one group per bone (the count is not stored;
//!   the groups end where the model name begins): `u32 n`, `n` vertex
//!   indices, `n` XYZ positions in bone-local space.
//! - A length-prefixed model name, then, 53 bytes later, the `u32` vertex
//!   count followed by that many 32-byte records. The first 12 bytes of
//!   each are the vertex's model-space position. The records start 8 bytes
//!   after the count and can be at an unaligned offset.
//! - One chunk per material part, each introduced by the tag `0c 00 00 00`
//!   and a length-prefixed texture path (the root of the path varies between
//!   characters). Chunks are 47 or 79 bytes apart, in a header not worked out,
//!   so they are found by the tag and the path.
//! - A chunk, from the end of its path: a 0x72-byte material header (its
//!   first `u32` is 3, or 4 for the blended `.tga` materials), then `u32 V`
//!   (the part's vertex count), `V` RGBA colors of four `f32`, `u32 1`, `V`
//!   UV pairs of `f32`; the same three again (the same UVs in every file);
//!   `V` unit normals of three `f32`; `u32 T`, `T` face planes (normal and
//!   distance, four `f32`); and two lists of `3T` `u16` triangle indices:
//!   first into the model's vertices, then the same triangles in part-local
//!   indices, which index the UVs and normals. UVs run roughly -1.1 to 1.7,
//!   so textures wrap. The corners go counterclockwise seen from the front.
//!
//! Units are centimeter-like and a soldier is about 200 tall with the origin
//! near the hips. Coordinates are the same handedness as the MAP files.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

use crate::motion::quat_rotate;
use crate::skeleton::{Skeleton, Transform};

/// A group of triangles sharing one material
#[derive(Clone, Debug)]
pub struct Part {
    /// Texture file name, without its directory (e.g. `art_h_torso.bmp`)
    pub material: String,
    /// Triangles as indices into `Crp::vertices`
    pub triangles: Vec<[u32; 3]>,
    /// Texture coordinates of each triangle's three corners, parallel to
    /// `triangles`. A vertex shared between parts can have different UVs in
    /// each, so these are per corner.
    pub corner_uvs: Vec<[[f32; 2]; 3]>,
    /// Unit normals of each triangle's three corners in the bind pose, parallel
    /// to `triangles`. Per corner like the UVs: where a part meets another at a
    /// crease, each side has its own.
    pub corner_normals: Vec<[[f32; 3]; 3]>,
}

#[derive(Clone, Debug)]
pub struct Crp {
    pub name: String,
    /// Model-space bind-pose positions
    pub vertices: Vec<[f32; 3]>,
    pub parts: Vec<Part>,
    /// One entry per bone group in file order: the vertices that group
    /// moves, with their bone-local positions
    pub groups: Vec<Vec<(u32, [f32; 3])>>,
}

/// Rigid skinning: every vertex follows one bone
#[derive(Clone, Debug)]
pub struct Skin {
    /// Bone index into `Skeleton::bones` for each vertex
    pub bone: Vec<usize>,
    /// Vertex position relative to its bone's bind position
    pub local: Vec<[f32; 3]>,
}

impl Skin {
    /// Vertex positions for a posed skeleton (see `Skeleton::pose`)
    pub fn apply(&self, pose: &[Transform]) -> Vec<[f32; 3]> {
        self.bone.iter().zip(&self.local).map(|(&b, l)| {
            let t = &pose[b];
            let r = quat_rotate(t.rot, *l);
            [t.pos[0] + r[0], t.pos[1] + r[1], t.pos[2] + r[2]]
        }).collect()
    }
}

impl Crp {
    /// Bind each vertex to a bone of `skeleton`. A group's bone is the one
    /// whose bind position equals the group's vertices' model-space position
    /// minus their bone-local position. Returns `None` if a group does not
    /// match any bone. A vertex listed in several groups follows the first.
    pub fn skin(&self, skeleton: &Skeleton) -> Option<Skin> {
        let n = self.vertices.len();
        let mut bone = vec![usize::MAX; n];
        let mut local = vec![[0.0f32; 3]; n];
        for group in &self.groups {
            // Some bones move no vertices
            let Some(&(first, l)) = group.first() else { continue };
            let m = *self.vertices.get(first as usize)?;
            let origin = [m[0] - l[0], m[1] - l[1], m[2] - l[2]];
            let (b, dist) = skeleton.nearest_bone(origin)?;
            if dist > 0.5 {
                return None;
            }
            for &(v, l) in group {
                let v = v as usize;
                if v < n && bone[v] == usize::MAX {
                    bone[v] = b;
                    local[v] = l;
                }
            }
        }
        // Vertices in no group stay rigid with the root
        let root = skeleton.bones.iter().position(|b| b.parent.is_none())?;
        for v in 0..n {
            if bone[v] == usize::MAX {
                let p = skeleton.bones[root].bind_pos;
                bone[v] = root;
                local[v] = [self.vertices[v][0] - p[0], self.vertices[v][1] - p[1], self.vertices[v][2] - p[2]];
            }
        }
        Some(Skin { bone, local })
    }

    /// The edges that only one triangle uses, as pairs of bind-pose positions, once vertices
    /// at the same position are merged. Where the mesh is closed there are none; a missing
    /// triangle leaves a loop of three.
    pub fn open_edges(&self) -> Vec<([f32; 3], [f32; 3])> {
        let key = |v: [f32; 3]| v.map(|x| (x * 100.0).round() as i64);
        let mut ids: HashMap<[i64; 3], usize> = HashMap::new();
        let welded: Vec<usize> = self.vertices.iter().map(|&v| {
            let n = ids.len();
            *ids.entry(key(v)).or_insert(n)
        }).collect();
        let mut uses: HashMap<(usize, usize), (u32, [u32; 2])> = HashMap::new();
        for t in self.parts.iter().flat_map(|p| &p.triangles) {
            for i in 0..3 {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                let (x, y) = (welded[a as usize], welded[b as usize]);
                uses.entry((x.min(y), x.max(y))).or_insert((0, [a, b])).0 += 1;
            }
        }
        let mut open: Vec<_> = uses.into_values().filter(|(n, _)| *n == 1)
            .map(|(_, [a, b])| (self.vertices[a as usize], self.vertices[b as usize]))
            .collect();
        open.sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
        open
    }

    /// (min, max) corners of the bind-pose vertices
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for v in &self.vertices {
            for i in 0..3 {
                min[i] = min[i].min(v[i]);
                max[i] = max[i].max(v[i]);
            }
        }
        (min, max)
    }
}

pub fn read(path: &Path) -> Result<Crp> {
    let data = std::fs::read(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    parse(&data)
}

fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    let b = d.get(o..o + 4).context("unexpected end of CRP")?;
    Ok(u32::from_le_bytes(b.try_into().unwrap()))
}

fn f32_at(d: &[u8], o: usize) -> Result<f32> {
    Ok(f32::from_bits(u32_at(d, o)?))
}

pub fn parse(d: &[u8]) -> Result<Crp> {
    // Bone groups; only needed to learn the vertex count and where they end.
    // The count is not stored, so stop at the model-name string.
    let mut pos = 4;
    let mut group_total = 0usize;
    let mut groups: Vec<Vec<(u32, [f32; 3])>> = Vec::new();
    for g in 0.. {
        ensure!(g < 64, "no model name after bone groups");
        if string_at(d, pos).is_some() {
            break;
        }
        let n = u32_at(d, pos).with_context(|| format!("bone group {g}"))? as usize;
        ensure!(n < 5000, "bone group {g} has implausible size {n}");
        group_total += n;
        let mut group = Vec::with_capacity(n);
        for k in 0..n {
            let idx = u32_at(d, pos + 4 + 4 * k)?;
            let p = pos + 4 + 4 * n + 12 * k;
            group.push((idx, [f32_at(d, p)?, f32_at(d, p + 4)?, f32_at(d, p + 8)?]));
        }
        groups.push(group);
        pos += 4 + 16 * n;
    }
    ensure!(group_total > 0, "no vertices");

    let name_len = u32_at(d, pos).context("model name")? as usize;
    ensure!((1..256).contains(&name_len), "bad model name length {name_len}");
    let name_bytes = d.get(pos + 4..pos + 4 + name_len).context("model name")?;
    let name = String::from_utf8_lossy(&name_bytes[..name_len - 1]).into_owned();
    pos += 4 + name_len;

    // The vertex count sits 53 bytes after the name, in a short, partly
    // unaligned header. Bone groups may list a vertex under two bones, so it
    // is not always the sum of the group sizes.
    let count_at = pos + 53;
    let nverts = u32_at(d, count_at).context("vertex count")? as usize;
    ensure!((1..20000).contains(&nverts), "implausible vertex count {nverts}");
    let base = count_at + 8;
    let mut vertices = Vec::with_capacity(nverts);
    for v in 0..nverts {
        let o = base + 32 * v;
        vertices.push([f32_at(d, o)?, f32_at(d, o + 4)?, f32_at(d, o + 8)?]);
    }

    // Material chunks: tag 0c 00 00 00, then a length-prefixed texture path.
    // `marks` holds the offset of each length field.
    let marks: Vec<usize> = (4..d.len().saturating_sub(8))
        .filter(|&o| d[o - 4..o] == [0x0c, 0, 0, 0] && string_at(d, o).is_some_and(|s| {
            let l = s.to_ascii_lowercase();
            l.ends_with(".bmp") || l.ends_with(".tga")
        }))
        .collect();
    if marks.is_empty() {
        bail!("no material chunks");
    }
    let mut parts = Vec::new();
    for &p in &marks {
        let len = u32_at(d, p)? as usize;
        let path = string_at(d, p).context("material path")?;
        let material = path.rsplit('\\').next().unwrap_or("").to_string();
        let part = read_chunk(d, p + 4 + len, material, vertices.len())
            .with_context(|| format!("material chunk {path}"))?;
        parts.push(part);
    }

    Ok(Crp { name, vertices, parts, groups })
}

/// The bytes between a chunk's texture path and its vertex count
const CHUNK_HEADER: usize = 0x72;

/// Read a material chunk from `at`, just past its texture path (see the layout above).
/// `vertex_count` is the model's, which the triangles index.
fn read_chunk(d: &[u8], at: usize, material: String, vertex_count: usize) -> Result<Part> {
    let mut o = at + CHUNK_HEADER;
    let v = u32_at(d, o)? as usize;
    ensure!((1..20000).contains(&v), "implausible part vertex count {v}");
    // Colors, then UVs, twice over; the second UVs repeat the first
    let colors_and_uvs = 4 + 16 * v + 4 + 8 * v;
    ensure!(u32_at(d, o + 4 + 16 * v)? == 1, "no UVs after the colors");
    ensure!(u32_at(d, o + colors_and_uvs)? as usize == v, "the second colors are not for {v} vertices");
    let uvs: Vec<[f32; 2]> = (0..v).map(|i| {
        let u = o + 4 + 16 * v + 4 + 8 * i;
        Ok([f32_at(d, u)?, f32_at(d, u + 4)?])
    }).collect::<Result<_>>()?;
    o += 2 * colors_and_uvs;
    let normals: Vec<[f32; 3]> = (0..v).map(|i| {
        let n = o + 12 * i;
        Ok([f32_at(d, n)?, f32_at(d, n + 4)?, f32_at(d, n + 8)?])
    }).collect::<Result<_>>()?;
    o += 12 * v;
    let t = u32_at(d, o)? as usize;
    ensure!((1..20000).contains(&t), "implausible triangle count {t}");
    // Past the face planes, the triangles: into the model's vertices, then into the part's
    let global = o + 4 + 16 * t;
    let local = global + 6 * t;
    let u16_at = |at: usize| -> Result<usize> {
        let b = d.get(at..at + 2).context("unexpected end of CRP")?;
        Ok(u16::from_le_bytes([b[0], b[1]]) as usize)
    };
    let mut triangles = Vec::with_capacity(t);
    let mut corner_uvs = Vec::with_capacity(t);
    let mut corner_normals = Vec::with_capacity(t);
    for k in 0..t {
        let g: [usize; 3] = [u16_at(global + 6 * k)?, u16_at(global + 6 * k + 2)?, u16_at(global + 6 * k + 4)?];
        let l: [usize; 3] = [u16_at(local + 6 * k)?, u16_at(local + 6 * k + 2)?, u16_at(local + 6 * k + 4)?];
        ensure!(g.iter().all(|&i| i < vertex_count), "triangle {k} {g:?} is past the model's {vertex_count} vertices");
        ensure!(l.iter().all(|&i| i < v), "triangle {k} {l:?} is past the part's {v} vertices");
        triangles.push(g.map(|i| i as u32));
        corner_uvs.push(l.map(|i| uvs[i]));
        corner_normals.push(l.map(|i| normals[i]));
    }
    Ok(Part { material, triangles, corner_uvs, corner_normals })
}

/// A length-prefixed, NUL-terminated printable ASCII string at `o`
fn string_at(d: &[u8], o: usize) -> Option<String> {
    let len = u32_at(d, o).ok()? as usize;
    if !(4..=260).contains(&len) {
        return None;
    }
    let b = d.get(o + 4..o + 4 + len)?;
    let (last, text) = b.split_last()?;
    (*last == 0 && text.iter().all(|c| (0x20..0x7f).contains(c)))
        .then(|| String::from_utf8_lossy(text).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOLDIER: &str = "data/character/team_hrt/Arctic1_H_a.CRP";

    fn load() -> Option<Crp> {
        let p = Path::new(SOLDIER);
        if !p.exists() {
            eprintln!("skipping: {SOLDIER} not found");
            return None;
        }
        Some(read(p).expect("CRP parses"))
    }

    #[test]
    fn reads_vertices_and_name() {
        let Some(c) = load() else { return };
        assert_eq!(c.name, "arctic1_H_a");
        assert_eq!(c.vertices.len(), 363);
    }

    #[test]
    fn is_a_person_sized_upright_body() {
        let Some(c) = load() else { return };
        let (min, max) = c.bounds();
        let height = max[1] - min[1];
        assert!((180.0..220.0).contains(&height), "height {height}");
        // Shoulders wider than the body is deep
        assert!(max[0] - min[0] > max[2] - min[2]);
    }

    #[test]
    fn extracts_triangles_that_cover_the_body() {
        let Some(c) = load() else { return };
        let total: usize = c.parts.iter().map(|p| p.triangles.len()).sum();
        assert!((600..800).contains(&total), "{total} triangles");
        let mut used = vec![false; c.vertices.len()];
        for t in c.parts.iter().flat_map(|p| &p.triangles) {
            for &i in t {
                used[i as usize] = true;
            }
        }
        let covered = used.iter().filter(|&&u| u).count();
        assert!(covered >= 350, "only {covered} vertices used");
    }

    #[test]
    fn every_part_has_uvs_in_a_sane_range() {
        let Some(c) = load() else { return };
        for part in &c.parts {
            let uvs = &part.corner_uvs;
            assert_eq!(uvs.len(), part.triangles.len());
            for uv in uvs.iter().flatten().flatten() {
                assert!((-5.0..5.0).contains(uv), "{} uv {uv}", part.material);
            }
        }
    }

    // The player's mesh has parts of a single triangle: the back of the helmet (two) and the
    // top of the vest below the neck. Dropped, they left holes to see into the model through.
    #[test]
    fn the_players_mesh_has_no_holes() {
        let p = Path::new("data/character/team_hrt/blue_M_a.CRP");
        if !p.exists() {
            eprintln!("skipping: {} not found", p.display());
            return;
        }
        let c = read(p).expect("CRP parses");
        let open = c.open_edges();
        assert!(open.is_empty(), "{} open edges: {open:?}", open.len());
    }

    // Each corner's normal is a unit vector, and faces the way the triangle does (its corners
    // go counterclockwise seen from the front) but where a vertex sits on a crease and its
    // normal is the average of the faces around it
    #[test]
    fn normals_are_unit_length_and_face_out() {
        let Some(c) = load() else { return };
        let (mut corners, mut out) = (0, 0);
        for part in &c.parts {
            assert_eq!(part.corner_normals.len(), part.triangles.len());
            for (t, normals) in part.triangles.iter().zip(&part.corner_normals) {
                let [a, b, p] = t.map(|i| c.vertices[i as usize]);
                let (e, f) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [p[0] - a[0], p[1] - a[1], p[2] - a[2]]);
                let face = [e[1] * f[2] - e[2] * f[1], e[2] * f[0] - e[0] * f[2], e[0] * f[1] - e[1] * f[0]];
                for n in normals {
                    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    assert!((length - 1.0).abs() < 0.01, "{} normal {n:?}", part.material);
                    corners += 1;
                    if n[0] * face[0] + n[1] * face[1] + n[2] * face[2] > 0.0 {
                        out += 1;
                    }
                }
            }
        }
        assert!(out as f32 > 0.9 * corners as f32, "{out} of {corners} corners face out");
    }

    #[test]
    fn triangles_are_not_degenerate() {
        let Some(c) = load() else { return };
        for t in c.parts.iter().flat_map(|p| &p.triangles) {
            assert!(t[0] != t[1] && t[1] != t[2] && t[0] != t[2], "{t:?}");
        }
    }

    #[test]
    fn skinning_in_the_bind_pose_reproduces_the_mesh() {
        let Some(c) = load() else { return };
        let skel = crate::skeleton::read(Path::new("data/character/team_hrt/Arctic1_H_a.SKL"))
            .expect("skeleton");
        let skin = c.skin(&skel).expect("groups match bones");
        let ident = vec![[0.0, 0.0, 0.0, 1.0]; 18];
        let hips = skel.bones.iter().find(|b| b.name == "Hips").unwrap().bind_pos;
        let moved = skin.apply(&skel.pose(hips, &ident));
        let worst = moved.iter().zip(&c.vertices)
            .map(|(a, b)| (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0f32, f32::max))
            .fold(0.0f32, f32::max);
        assert!(worst < 0.01, "worst error {worst}");
    }

    #[test]
    fn every_character_binds_to_its_skeleton() {
        let Ok(dirs) = std::fs::read_dir("data/character") else { return };
        let (mut ok, mut bad) = (0, Vec::new());
        for dir in dirs.flatten().filter(|d| d.path().is_dir()) {
            for f in std::fs::read_dir(dir.path()).unwrap().flatten() {
                let p = f.path();
                if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("crp")) {
                    continue;
                }
                // The skeleton sits beside the "_a" mesh with the same name
                let skl = p.with_extension("SKL");
                let skl = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.path()).find(|q| {
                    q.extension().is_some_and(|e| e.eq_ignore_ascii_case("skl"))
                        && q.file_stem().map(|s| s.to_ascii_lowercase()) == skl.file_stem().map(|s| s.to_ascii_lowercase())
                });
                let Some(skl) = skl else { continue };
                let (Ok(c), Ok(sk)) = (read(&p), crate::skeleton::read(&skl)) else {
                    bad.push(format!("{}: parse", p.display()));
                    continue;
                };
                match c.skin(&sk) {
                    Some(_) => ok += 1,
                    None => bad.push(format!("{}: groups do not match bones", p.display())),
                }
            }
        }
        eprintln!("{ok} meshes bound, {} failed", bad.len());
        for b in bad.iter().take(10) {
            eprintln!("  {b}");
        }
        assert!(bad.is_empty());
    }

    #[test]
    fn parses_every_character_file() {
        let Ok(dirs) = std::fs::read_dir("data/character") else { return };
        let (mut ok, mut bad) = (0, Vec::new());
        for dir in dirs.flatten().filter(|d| d.path().is_dir()) {
            for f in std::fs::read_dir(dir.path()).unwrap().flatten() {
                let p = f.path();
                if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("crp")) {
                    match read(&p) {
                        Ok(c) if c.parts.iter().any(|p| !p.triangles.is_empty()) => ok += 1,
                        Ok(_) => bad.push(format!("{}: no triangles", p.display())),
                        Err(e) => bad.push(format!("{}: {e:#}", p.display())),
                    }
                }
            }
        }
        eprintln!("{ok} parsed, {} failed", bad.len());
        for b in bad.iter().take(10) {
            eprintln!("  {b}");
        }
        assert!(bad.is_empty(), "{} files failed", bad.len());
    }
}
