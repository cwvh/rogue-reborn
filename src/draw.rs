//! Drawing the level, characters, glass and weapons.

use std::collections::HashMap;
use rogue_reborn::{map, crp, glass, visibility};
use crate::opengl::*;
use crate::textures::ensure_texture;

pub struct Mesh {
    pub vertices: Vec<[f32; 3]>,
    pub indices: Vec<[u32; 3]>,
    pub center: [f32; 3],
    pub radius: f32,
}

// The weapon model as flat triangle corner arrays, ready to draw
pub struct WeaponDraw {
    pub verts: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub texture: Option<u32>,
    pub muzzle: [f32; 3],
    /// Middle of the model's bounding box, and its longest side
    pub center: [f32; 3],
    pub length: f32,
}

// Draw the weapon model in the current matrix, with a muzzle flash if asked
pub fn draw_weapon(wd: &WeaponDraw, textured: bool, flash: bool) {
    unsafe {
        if let (true, Some(tex)) = (textured, wd.texture) {
            glEnable(0x0DE1);
            glBindTexture(0x0DE1, tex);
            glColor4f(1.0, 1.0, 1.0, 1.0);
        } else {
            glDisable(0x0DE1);
            glColor4f(0.7, 0.7, 0.75, 1.0);
        }
        glBegin(0x0004); // GL_TRIANGLES
        for (v, uv) in wd.verts.iter().zip(&wd.uvs) {
            glTexCoord2f(uv[0], uv[1]);
            glVertex3f(v[0], v[1], v[2]);
        }
        glEnd();
        // Muzzle flash: a short burst of lines out of the barrel
        if flash {
            glDisable(0x0DE1);
            glColor4f(1.0, 0.85, 0.3, 1.0);
            let m = wd.muzzle;
            glBegin(0x0001); // GL_LINES
            for (dx, dy, dz) in [(0.0, -22.0, 0.0), (7.0, -6.0, 0.0), (-7.0, -6.0, 0.0), (0.0, -6.0, 7.0), (0.0, -6.0, -7.0)] {
                glVertex3f(m[0], m[1], m[2]);
                glVertex3f(m[0] + dx, m[1] + dy, m[2] + dz);
            }
            glEnd();
        }
    }
}

// Draw one posed character: `verts` are its (possibly animated) vertex
// positions and `face` replaces the HRT placeholder face texture.
pub fn draw_character(c: &crp::Crp, verts: &[[f32; 3]], part_textures: &[Option<u32>], face: Option<u32>, textured: bool) {
    unsafe {
    // Opaque parts first, then glass (.tga materials) blended over them
    for glass_pass in [false, true] {
    for (pi, part) in c.parts.iter().enumerate() {
        let is_glass = part.material.to_ascii_lowercase().ends_with(".tga");
        if is_glass != glass_pass { continue; }
        // The HRT placeholder face is replaced by the team member's own
        let is_face = part.material.eq_ignore_ascii_case("hrt_face.bmp");
        let tex = if is_face { face } else { None }
            .or(part_textures.get(pi).copied().flatten());
        let uvs = part.corner_uvs.as_ref();
        if is_glass && textured { glEnable(0x0BE2); glBlendFunc(0x0302, 0x0303); } // GL_BLEND, SRC_ALPHA, ONE_MINUS_SRC_ALPHA
        // Solid mode shows the texture; wireframe shows flat color lines
        if let (true, Some(id), Some(_)) = (textured, tex, uvs) {
            glEnable(0x0DE1); // GL_TEXTURE_2D
            glBindTexture(0x0DE1, id);
            glColor4f(1.0, 1.0, 1.0, 1.0);
        } else {
            glDisable(0x0DE1);
            if textured && is_glass {
                // Glass with no texture file: faint tinted pane
                glColor4f(0.45, 0.55, 0.65, 0.2);
            } else if textured {
                glColor4f(0.3, 0.35, 0.4, 1.0);
            } else {
                let (r, g, b) = material_color(&part.material);
                glColor4f(r, g, b, 1.0);
            }
        }
        glBegin(0x0004); // GL_TRIANGLES
        for (k, t) in part.triangles.iter().enumerate() {
            for (j, &i) in t.iter().enumerate() {
                if let Some(uvs) = uvs {
                    glTexCoord2f(uvs[k][j][0], uvs[k][j][1]);
                }
                let v = &verts[i as usize];
                glVertex3f(v[0], v[1], v[2]);
            }
        }
        glEnd();
        if is_glass { glDisable(0x0BE2); }
    }
    }
    }
}

// Draw blended glass triangles (world space corners and UVs) with a material's texture
pub fn draw_glass(
    materials: &map::Materials,
    material: usize,
    map_num: &str,
    textures: &mut HashMap<String, u32>,
    textured: bool,
    alpha: f32,
    tris: impl Iterator<Item = ([[f32; 3]; 3], [[f32; 2]; 3])>,
) {
    unsafe {
        glEnable(0x0BE2); // GL_BLEND
        glBlendFunc(0x0302, 0x0303); // SRC_ALPHA, ONE_MINUS_SRC_ALPHA
        match (textured, ensure_texture(materials, material, map_num, textures)) {
            (true, Some(id)) => {
                glEnable(0x0DE1);
                glBindTexture(0x0DE1, id);
                glColor4f(1.0, 1.0, 1.0, alpha);
            }
            _ => {
                glDisable(0x0DE1);
                glColor4f(0.6, 0.8, 0.9, alpha.max(0.3));
            }
        }
        glBegin(0x0004); // GL_TRIANGLES
        for (p, uv) in tris {
            for k in 0..3 {
                glTexCoord2f(uv[k][0], uv[k][1]);
                glVertex3f(p[k][0], p[k][1], p[k][2]);
            }
        }
        glEnd();
        glDisable(0x0BE2);
        glColor4f(1.0, 1.0, 1.0, 1.0);
    }
}

// Draw a broken pane: its pieces with a glint along their edges, and the flying flecks
pub fn draw_fracture(materials: &map::Materials, map_num: &str, textures: &mut HashMap<String, u32>, textured: bool, f: &glass::Fracture) {
    unsafe {
        glEnable(0x0BE2); // GL_BLEND
        glBlendFunc(0x0302, 0x0303); // SRC_ALPHA, ONE_MINUS_SRC_ALPHA
        let tex = if textured { ensure_texture(materials, f.material, map_num, textures) } else { None };
        for s in &f.shards {
            let (corners, alpha) = (s.corners(), s.alpha());
            if let Some(id) = tex {
                glEnable(0x0DE1);
                glBindTexture(0x0DE1, id);
                glColor4f(1.0, 1.0, 1.0, alpha * f.opacity);
            } else {
                glDisable(0x0DE1);
                glColor4f(0.6, 0.8, 0.9, alpha * 0.4);
            }
            glBegin(0x0006); // GL_TRIANGLE_FAN
            for (c, uv) in corners.iter().zip(s.uvs()) {
                glTexCoord2f(uv[0], uv[1]);
                glVertex3f(c[0], c[1], c[2]);
            }
            glEnd();
            // Edges catch the light
            glDisable(0x0DE1);
            glColor4f(0.9, 0.97, 1.0, alpha * 0.75);
            glBegin(0x0002); // GL_LINE_LOOP
            for c in &corners {
                glVertex3f(c[0], c[1], c[2]);
            }
            glEnd();
        }
        glPointSize(2.5);
        glBegin(0x0000); // GL_POINTS
        for fl in &f.flecks {
            glColor4f(0.95, 0.98, 1.0, fl.alpha());
            glVertex3f(fl.pos[0], fl.pos[1], fl.pos[2]);
        }
        glEnd();
        glDisable(0x0BE2);
        glColor4f(1.0, 1.0, 1.0, 1.0);
    }
}

// Stable, distinguishable color for a material name
pub fn material_color(name: &str) -> (f32, f32, f32) {
    let h = name.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let c = |shift: u32| 0.35 + 0.65 * ((h >> shift) & 0xff) as f32 / 255.0;
    (c(0), c(8), c(16))
}

pub fn build_mesh(level: &map::Map) -> Mesh {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for object in &level.geometries.objects {
        let base = vertices.len() as u32;

        // Append object vertices
        for v in &object.vertices {
            let p = [v.x, v.y, v.z];
            for i in 0..3 {
                if p[i] < min[i] {
                    min[i] = p[i];
                }
                if p[i] > max[i] {
                    max[i] = p[i];
                }
            }
            vertices.push(p);
        }

        // Append triangle indices from all object data groups
        for od in &object.object_datas {
            for &(i0, i1, i2) in &od.faces.face_indices {
                indices.push([
                    base + i0 as u32,
                    base + i1 as u32,
                    base + i2 as u32,
                ]);
            }
        }
    }

    // Fallback if empty
    if vertices.is_empty() {
        return Mesh {
            vertices,
            indices,
            center: [0.0, 0.0, 0.0],
            radius: 10.0,
        };
    }

    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let dx = max[0] - center[0];
    let dy = max[1] - center[1];
    let dz = max[2] - center[2];
    let radius = (dx * dx + dy * dy + dz * dz).sqrt().max(1.0);

    Mesh {
        vertices,
        indices,
        center,
        radius,
    }
}

/// Which of the level's objects to draw
#[derive(Clone, Copy)]
pub enum Which {
    All,
    /// The objects of one room
    Room(u32),
    /// Those that belong to no room
    Unowned,
}

/// Draw a room of the plan, then the rooms seen through its portals. `level` is the
/// stencil value of the part of the screen this room owns. Each portal's opening is
/// marked in the stencil buffer where it can be seen, the depth there is wiped, and the
/// room beyond is drawn only into the mark: whatever the room in front has past the
/// portal (rooms overlap each other) cannot show in the doorway.
pub unsafe fn draw_plan(node: &visibility::PlanNode<'_>, level: i32, fill_mode: u32, textured: bool, draw: &mut dyn FnMut(u32, &[[f32; 4]])) {
    const EQUAL: u32 = 0x0202;
    const KEEP: u32 = 0x1E00;
    unsafe {
        glStencilFunc(EQUAL, level, !0);
        glStencilOp(KEEP, KEEP, KEEP);
        // Cut where the portal it is seen through begins, and at the portals the player is
        // standing in (on this room's side). Where the player stands in a doorway both rooms
        // are drawn whole and each is cut only where it overlaps the other: what is on the
        // near side of the plane, or outside the other room's floor plan, stays. That is a
        // union, which clip planes cannot say, so the room is drawn once for each way to stay.
        let outside = |b: [f32; 4]| [[-1.0, 0.0, 0.0, b[0]], [1.0, 0.0, 0.0, -b[2]], [0.0, 0.0, -1.0, b[1]], [0.0, 0.0, 1.0, -b[3]]];
        let mut fixed: Vec<[f32; 4]> = Vec::new();
        let mut ways: Vec<Vec<[f32; 4]>> = Vec::new();
        match (node.plane, node.cut_in_self) {
            // Entered through a doorway the player stands in
            (Some(p), Some(b)) if node.touched => {
                ways.push(vec![p]);
                ways.extend(outside(b).into_iter().map(|o| vec![o]));
            }
            (Some(p), _) => fixed.push(p),
            _ => {}
        }
        for child in node.children.iter().filter(|c| c.touched) {
            let Some(p) = child.plane else { continue };
            let near = [-p[0], -p[1], -p[2], -p[3]];
            match child.cut_in_parent {
                Some(b) if ways.is_empty() => {
                    ways.push(vec![near]);
                    ways.extend(outside(b).into_iter().map(|o| vec![o]));
                }
                _ => fixed.push(near),
            }
        }
        if ways.is_empty() {
            fixed.truncate(6);
            draw(node.room, &fixed);
        }
        for way in &ways {
            let mut planes = fixed.clone();
            planes.extend(way);
            planes.truncate(6);
            draw(node.room, &planes);
        }

        for child in &node.children {
            // Standing in the doorway: both rooms are drawn, each cut at the portal's plane
            if child.touched {
                draw_plan(child, level, fill_mode, textured, draw);
                continue;
            }
            let opening = || {
                glBegin(0x0006); // GL_TRIANGLE_FAN
                for p in child.polygon {
                    glVertex3f(p[0], p[1], p[2]);
                }
                glEnd();
            };
            set_clip_planes(&[]);
            glDisable(0x0DE1); // GL_TEXTURE_2D
            glPolygonMode(0x0408, 0x1B02);
            glColorMask(0, 0, 0, 0);
            glDepthMask(0);

            // Mark the part of the opening that is not behind something nearer. The polygon
            // is nudged deeper, so walls lying in the portal's own plane are not wiped.
            glStencilFunc(EQUAL, level, !0);
            glStencilOp(KEEP, KEEP, 0x1E02); // GL_INCR
            glEnable(0x8037); // GL_POLYGON_OFFSET_FILL
            glPolygonOffset(2.0, 2.0);
            opening();
            glDisable(0x8037);
            // Wipe the depth there, so the room beyond competes only with itself
            glStencilFunc(EQUAL, level + 1, !0);
            glStencilOp(KEEP, KEEP, KEEP);
            glDepthFunc(0x0207); // GL_ALWAYS
            glDepthMask(1);
            glDepthRange(1.0, 1.0);
            opening();
            glDepthRange(0.0, 1.0);
            glDepthFunc(0x0201); // GL_LESS
            glColorMask(1, 1, 1, 1);
            glPolygonMode(0x0408, fill_mode);
            if textured {
                glEnable(0x0DE1);
            }

            draw_plan(child, level + 1, fill_mode, textured, draw);

            // The opening is done: nothing else may draw into it
            set_clip_planes(&[]);
            glColorMask(0, 0, 0, 0);
            glDepthMask(0);
            glDisable(0x0B71); // GL_DEPTH_TEST
            glPolygonMode(0x0408, 0x1B02);
            glStencilFunc(EQUAL, level + 1, !0);
            glStencilOp(KEEP, KEEP, 0x0000); // GL_ZERO
            opening();
            glEnable(0x0B71);
            glDepthMask(1);
            glColorMask(1, 1, 1, 1);
            glPolygonMode(0x0408, fill_mode);
            if textured {
                glEnable(0x0DE1);
            }
            glStencilOp(KEEP, KEEP, KEEP);
        }
    }
}

/// A stable color for a room's name, so the same room looks the same every run.
/// The rooms the player is standing in are drawn white.
pub fn room_color(name: &str, current: bool) -> [f32; 3] {
    if current {
        return [1.0, 1.0, 1.0];
    }
    let hash = name.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let hue = (hash % 360) as f32 / 60.0;
    let x = 1.0 - (hue % 2.0 - 1.0).abs();
    let (r, g, b) = match hue as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    [r * 0.8 + 0.1, g * 0.8 + 0.1, b * 0.8 + 0.1]
}
