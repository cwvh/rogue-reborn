//! Reader for Rogue Spear character meshes (`.CRP`), bind pose only.
//!
//! The format is undocumented; this was worked out from the files, so it only
//! extracts what has been verified: model-space vertex positions and the
//! triangles of each material part. UVs, normals, bone weights and animation
//! are not read.
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
//!   characters). A chunk ends with two lists of `u16` triangle indices:
//!   first the global vertex indices, then the same triangles in part-local
//!   indices (which start `0,1,2`).
//! - Inside a chunk, at offset `0x72` from the end of the path: `u32 V` (the
//!   part's vertex count), `V` 16-byte color entries (not read), `u32 1`,
//!   then `V` UV pairs of `f32`, indexed by the part-local vertex index.
//!   Values run roughly -1.1 to 1.7, so textures wrap.
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
    /// each, so these are per corner. `None` if the chunk's UVs were not found.
    pub corner_uvs: Option<Vec<[[f32; 2]; 3]>>,
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
    for (i, &p) in marks.iter().enumerate() {
        let len = u32_at(d, p)? as usize;
        let path = string_at(d, p).context("material path")?;
        let material = path.rsplit('\\').next().unwrap_or("").to_string();

        let body_start = p + 4 + len;
        let body_end = marks.get(i + 1).map_or(d.len(), |&n| n - 4);
        let Some(body) = d.get(body_start..body_end) else { continue };
        let Some((global, local)) = find_triangle_lists(body) else { continue };
        let uvs = part_uvs(body, &local);
        // A single triangle could be a false match on a chunk with no lists, so it must have
        // three distinct vertices of the mesh and the chunk its three UVs. Parts of one
        // triangle are real: the back of a helmet, the top of a vest.
        if let [t] = global[..] {
            let real = t[0] != t[1] && t[1] != t[2] && t[0] != t[2] && t.iter().all(|&i| (i as usize) < vertices.len());
            if !real || uvs.is_none() {
                continue;
            }
        }

        let mut triangles = Vec::with_capacity(global.len());
        let mut corner_uvs = uvs.as_ref().map(|_| Vec::with_capacity(global.len()));
        for (k, t) in global.iter().enumerate() {
            if !t.iter().all(|&i| (i as usize) < vertices.len()) {
                continue;
            }
            triangles.push([t[0] as u32, t[1] as u32, t[2] as u32]);
            if let (Some(out), Some(uvs)) = (corner_uvs.as_mut(), uvs.as_ref()) {
                out.push(local[k].map(|l| uvs[l as usize]));
            }
        }
        parts.push(Part { material, triangles, corner_uvs });
    }

    Ok(Crp { name, vertices, parts, groups })
}

/// The part's UV pairs, indexed by part-local vertex. Located by the vertex
/// count, which appears near the start of the chunk, followed `16 * V` bytes
/// later by a `1`.
fn part_uvs(body: &[u8], local: &[[u16; 3]]) -> Option<Vec<[f32; 2]>> {
    let v = local.iter().flatten().map(|&l| l as usize).max()? + 1;
    let at = (0..0x200).find(|&o| {
        u32_at(body, o).is_ok_and(|n| n as usize == v)
            && u32_at(body, o + 4 + 16 * v).is_ok_and(|t| t == 1)
            && o + 8 + 24 * v <= body.len()
    })?;
    let base = at + 4 + 16 * v + 4;
    (0..v)
        .map(|i| Some([f32_at(body, base + 8 * i).ok()?, f32_at(body, base + 8 * i + 4).ok()?]))
        .collect()
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

/// Find the pair of triangle index lists in a chunk and return the global and
/// local lists. The local list starts `0,1,2` and the global list immediately
/// precedes it; they must describe the same triangles, so the
/// global-to-local mapping has to be one-to-one and cover every local index.
/// The longest consistent pair wins.
fn find_triangle_lists(body: &[u8]) -> Option<(Vec<[u16; 3]>, Vec<[u16; 3]>)> {
    let mut best: Option<(usize, usize, Vec<u16>)> = None; // (T, q, u16 data)

    for parity in 0..2 {
        if body.len() < parity {
            continue;
        }
        let n = (body.len() - parity) / 2;
        let u: Vec<u16> = (0..n)
            .map(|i| u16::from_le_bytes([body[parity + 2 * i], body[parity + 2 * i + 1]]))
            .collect();

        for q in 3..n.saturating_sub(3) {
            if u[q..q + 3] != [0, 1, 2] {
                continue;
            }
            let max_t = (q / 3).min((n - q) / 3).min(2500);
            for t in (1..=max_t).rev() {
                if best.as_ref().is_some_and(|b| t <= b.0) {
                    break;
                }
                if consistent(&u, q, t) {
                    best = Some((t, q, u.clone()));
                    break;
                }
            }
        }
    }

    let (t, q, u) = best?;
    let tri = |o: usize| [u[o], u[o + 1], u[o + 2]];
    Some((
        (0..t).map(|k| tri(q - 3 * t + 3 * k)).collect(),
        (0..t).map(|k| tri(q + 3 * k)).collect(),
    ))
}

fn consistent(u: &[u16], q: usize, t: usize) -> bool {
    let mut to_global: HashMap<u16, u16> = HashMap::new();
    let mut to_local: HashMap<u16, u16> = HashMap::new();
    for i in 0..3 * t {
        let local = u[q + i];
        let global = u[q - 3 * t + i];
        if *to_global.entry(local).or_insert(global) != global
            || *to_local.entry(global).or_insert(local) != local
        {
            return false;
        }
    }
    // Local indices must be exactly 0..V
    (0..to_global.len() as u16).all(|l| to_global.contains_key(&l))
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
            let uvs = part.corner_uvs.as_ref()
                .unwrap_or_else(|| panic!("{} has no UVs", part.material));
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
                        Ok(c) if c.parts.iter().any(|p| !p.triangles.is_empty())
                            && c.parts.iter().all(|p| p.corner_uvs.is_some()) => ok += 1,
                        Ok(_) => bad.push(format!("{}: no triangles or UVs", p.display())),
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
