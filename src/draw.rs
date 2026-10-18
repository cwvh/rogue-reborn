//! Drawing the level, characters, glass and weapons.

use std::cell::RefCell;
use std::collections::HashMap;
use rogue_reborn::{map, crp, glass, visibility};
use crate::opengl::*;
use crate::textures::{ensure_texture, is_tga};

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

// A character model's triangle corners laid out for glDrawArrays: one entry per corner, the
// parts one after another
#[derive(Default)]
pub struct CharacterArrays {
    // The vertex of `Crp::vertices` each corner uses
    corner_vertex: Box<[u32]>,
    // Texture coordinates, two per corner; they never change
    uvs: Box<[f32]>,
    // The first corner of each part
    first: Box<[i32]>,
    // Scratch space for the corners' positions, three per corner, refilled by each draw
    positions: RefCell<Vec<f32>>,
}

impl CharacterArrays {
    pub fn new(c: &crp::Crp) -> Self {
        let (mut corner_vertex, mut uvs, mut first) = (Vec::new(), Vec::new(), Vec::new());
        for part in &c.parts {
            first.push(corner_vertex.len() as i32);
            for (t, uv) in part.triangles.iter().zip(&part.corner_uvs) {
                for j in 0..3 {
                    corner_vertex.push(t[j]);
                    uvs.extend_from_slice(&uv[j]);
                }
            }
        }
        Self {
            corner_vertex: corner_vertex.into(),
            uvs: uvs.into(),
            first: first.into(),
            positions: RefCell::new(Vec::new()),
        }
    }
}

// Draw one posed character: `verts` are its (possibly animated) vertex
// positions and `face` replaces the HRT placeholder face texture.
pub fn draw_character(c: &crp::Crp, arrays: &CharacterArrays, verts: &[[f32; 3]], part_textures: &[Option<u32>], face: Option<u32>, textured: bool, glow: [f32; 3]) {
    // The posed position of every corner, gathered once for all the parts
    let mut positions = arrays.positions.borrow_mut();
    positions.clear();
    positions.extend(arrays.corner_vertex.iter().flat_map(|&i| verts[i as usize]));
    unsafe {
    glEnableClientState(0x8074); // GL_VERTEX_ARRAY
    glEnableClientState(0x8078); // GL_TEXTURE_COORD_ARRAY
    glVertexPointer(3, 0x1406, 0, positions.as_ptr().cast()); // GL_FLOAT
    glTexCoordPointer(2, 0x1406, 0, arrays.uvs.as_ptr().cast());
    // Opaque parts first, then the see-through ones (.tga materials) blended over them
    for alpha_pass in [false, true] {
    for (pi, part) in c.parts.iter().enumerate() {
        // See-through surfaces, like a visor or goggles
        let has_alpha = is_tga(&part.material);
        if has_alpha != alpha_pass { continue; }
        // The HRT placeholder face is replaced by the team member's own
        let is_face = part.material.eq_ignore_ascii_case("hrt_face.bmp");
        let tex = if is_face { face } else { None }
            .or(part_textures.get(pi).copied().flatten());
        if has_alpha && textured { glEnable(0x0BE2); glBlendFunc(0x0302, 0x0303); } // GL_BLEND, SRC_ALPHA, ONE_MINUS_SRC_ALPHA
        // Solid mode shows the texture; wireframe shows flat color lines
        if let (true, Some(id)) = (textured, tex) {
            glEnable(0x0DE1); // GL_TEXTURE_2D
            glBindTexture(0x0DE1, id);
            glColor4f(1.0, 1.0, 1.0, 1.0);
        } else {
            glDisable(0x0DE1);
            if textured && has_alpha {
                // See-through part with no texture file: faint tinted pane
                glColor4f(0.45, 0.55, 0.65, 0.2);
            } else if textured {
                glColor4f(0.3, 0.35, 0.4, 1.0);
            } else {
                let (r, g, b) = material_color(&part.material);
                glColor4f(r, g, b, 1.0);
            }
        }
        glDrawArrays(0x0004, arrays.first[pi], part.triangles.len() as i32 * 3); // GL_TRIANGLES
        if has_alpha { glDisable(0x0BE2); }
    }
    }
    // A blast's light on the body: the opaque parts again, added over themselves
    if textured && glow.iter().any(|&g| g > 0.003) {
        glEnable(0x0BE2); // GL_BLEND
        glBlendFunc(1, 1); // ONE, ONE
        glDepthFunc(0x0203); // GL_LEQUAL: the same triangles again
        glDepthMask(0);
        glEnable(0x8037); // GL_POLYGON_OFFSET_FILL
        glPolygonOffset(-1.0, -1.0);
        glEnable(0x0DE1); // GL_TEXTURE_2D
        glColor4f(glow[0], glow[1], glow[2], 1.0);
        for (pi, part) in c.parts.iter().enumerate() {
            if is_tga(&part.material) { continue; }
            let is_face = part.material.eq_ignore_ascii_case("hrt_face.bmp");
            let tex = if is_face { face } else { None }.or(part_textures.get(pi).copied().flatten());
            let Some(id) = tex else { continue };
            glBindTexture(0x0DE1, id);
            glDrawArrays(0x0004, arrays.first[pi], part.triangles.len() as i32 * 3);
        }
        glDisable(0x8037);
        glDepthMask(1);
        glDepthFunc(0x0201); // GL_LESS
        glDisable(0x0BE2);
        glColor4f(1.0, 1.0, 1.0, 1.0);
    }
    glDisableClientState(0x8078);
    glDisableClientState(0x8074);
    }
}

// Draw blended glass triangles (world space corners and UVs) with a material's texture
pub fn draw_glass(
    materials: &map::Materials,
    material: usize,
    map_num: &str,
    textures: &mut HashMap<String, Option<u32>>,
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
pub fn draw_fracture(materials: &map::Materials, map_num: &str, textures: &mut HashMap<String, Option<u32>>, textured: bool, f: &glass::Fracture) {
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

/// How far a room's cut at a portal's plane is moved toward the camera. Surfaces lying in the
/// portal's own plane (the riser of a stair's first step, a wall's face) must stay, and a cut
/// exactly through them keeps or loses them by rounding, one triangle or one pixel at a time.
const PLANE_SLACK: f32 = 1.0;

/// How far the box a doorway cut is limited to (see draw_plan) is moved out, for surfaces lying
/// on its edge
const BOX_SLACK: f32 = 0.25;

/// A cut that keeps the side of `plane` its normal points to, and `PLANE_SLACK` more
fn slackened(plane: [f32; 4]) -> [f32; 4] {
    [plane[0], plane[1], plane[2], plane[3] + PLANE_SLACK]
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
        // Slackened a little too: rm08's railing over room 521's stairwell lies exactly on the
        // edge of 521's volume, and rounding cut it. A whole PLANE_SLACK brings back a hairline
        // of 121's stairs in the ceiling of rm11's 119.
        // Outside the box: past one of its sides, or above or below it (a unit short of its top,
        // as in wipe_opening)
        let outside = |b: [f32; 4], h: Option<[f32; 2]>| {
            let mut sides = vec![[-1.0, 0.0, 0.0, b[0]], [1.0, 0.0, 0.0, -b[2]], [0.0, 0.0, -1.0, b[1]], [0.0, 0.0, 1.0, -b[3]]];
            if let Some([low, high]) = h {
                sides.push([0.0, 1.0, 0.0, -(high - 1.0)]);
                sides.push([0.0, -1.0, 0.0, low]);
            }
            sides.into_iter().map(|p| [p[0], p[1], p[2], p[3] + BOX_SLACK]).collect::<Vec<[f32; 4]>>()
        };
        let mut fixed: Vec<[f32; 4]> = Vec::new();
        let mut ways: Vec<Vec<[f32; 4]>> = Vec::new();
        match (node.plane, node.cut_in_self) {
            // Entered through a doorway the player stands in
            (Some(p), Some(b)) if node.touched => {
                ways.push(vec![slackened(p)]);
                ways.extend(outside(b, node.cut_in_self_height).into_iter().map(|o| vec![o]));
            }
            (Some(p), _) => fixed.push(slackened(p)),
            _ => {}
        }
        for child in node.children.iter().filter(|c| c.touched) {
            let Some(p) = child.plane else { continue };
            let near = slackened([-p[0], -p[1], -p[2], -p[3]]);
            match child.cut_in_parent {
                Some(b) if ways.is_empty() => {
                    ways.push(vec![near]);
                    ways.extend(outside(b, child.cut_in_parent_height).into_iter().map(|o| vec![o]));
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

        // Standing in the doorway: both rooms are drawn, each cut at the portal's plane. They
        // go first, as one room: the openings of the other portals must be hidden behind
        // their walls too, and nothing can draw into an opening once it is done.
        for child in node.children.iter().filter(|c| c.touched) {
            draw_plan(child, level, fill_mode, textured, draw);
        }
        for child in node.children.iter().filter(|c| !c.touched) {
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

/// Wipe the depth in the part of one of the player's room's doorways that is not behind
/// something nearer, where what is drawn there lies inside the floor plan `plan` of the room
/// beyond (min x, min z, max x, max z, map space) and within its `height` (low, high), seen
/// from `eye`, so that room, drawn next, shows there in front of it. Not above or below the
/// room beyond: it has nothing there to show, and the wipe would open a hole in the player's
/// room (rm08's nook between the crates is 280 high, and 105's wall above it is inside its
/// floor plan). Rooms overlap: this is what hides rm11's hall's wall one step into room 119,
/// as the original engine does. The stencil is expected to be 1 everywhere, and is left so.
pub unsafe fn wipe_opening(polygon: &[[f32; 3]], plan: [f32; 4], height: Option<[f32; 2]>, eye: [f32; 3], fill_mode: u32, textured: bool) {
    const ALWAYS: u32 = 0x0207;
    const EQUAL: u32 = 0x0202;
    const KEEP: u32 = 0x1E00;
    const REPLACE: u32 = 0x1E01;
    unsafe {
        let opening = || {
            glBegin(0x0006); // GL_TRIANGLE_FAN
            for p in polygon {
                glVertex3f(p[0], p[1], p[2]);
            }
            glEnd();
        };
        // The room's box, as the six planes bounding it: an axis, where along it, and which
        // way is out
        let [x0, z0, x1, z1] = plan;
        // Tall enough for anything seen through the opening, when the room's height is not known
        let tall = (polygon.iter().map(|p| p[1]).fold(f32::MAX, f32::min) - 2000.0, polygon.iter().map(|p| p[1]).fold(f32::MIN, f32::max) + 2000.0);
        // A unit short of the room's top: walls resting on it (rm08's 105 over the nook) meet
        // the plane there, and along that line the depth test speckles
        let (y0, y1) = height.map_or(tall, |h| (h[0] - BOX_SLACK, h[1] - 1.0));
        let planes = [(0, x0, -1.0), (0, x1, 1.0), (1, y0, -1.0), (1, y1, 1.0), (2, z0, -1.0), (2, z1, 1.0)];
        // A plane as a quad big enough to cover the view, centered on the eye
        const FAR: f32 = 100_000.0;
        let plane_quad = |axis: usize, at: f32| {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            glBegin(0x0007); // GL_QUADS
            for (du, dv) in [(-FAR, -FAR), (FAR, -FAR), (FAR, FAR), (-FAR, FAR)] {
                let mut p = [0.0f32; 3];
                p[axis] = at;
                p[u] = eye[u] + du;
                p[v] = eye[v] + dv;
                glVertex3f(p[0], p[1], p[2]);
            }
            glEnd();
        };
        glEnable(0x0B90); // GL_STENCIL_TEST
        glDisable(0x0DE1); // GL_TEXTURE_2D
        glPolygonMode(0x0408, 0x1B02);
        glColorMask(0, 0, 0, 0);
        glDepthMask(0);
        // Mark the visible part. The polygon is nudged deeper, so walls lying in the portal's
        // own plane are not wiped.
        glStencilFunc(ALWAYS, 3, !0);
        glStencilOp(KEEP, KEEP, REPLACE);
        glEnable(0x8037); // GL_POLYGON_OFFSET_FILL
        glPolygonOffset(2.0, 2.0);
        opening();
        glDisable(0x8037);
        // Unmark where what is drawn lies outside the box, on the outer side of one of its
        // planes: past the plane where the eye is inside it (the plane is nearer), in front of
        // it where the eye is outside (the plane is farther). Not only the box's far faces: a
        // line of sight can pass over a low room's box without going through it.
        glStencilFunc(EQUAL, 3, !0);
        glStencilOp(KEEP, KEEP, 0x1E03); // GL_DECR
        for (axis, at, out) in planes {
            let eye_out = (eye[axis] - at) * out > 0.0;
            glDepthFunc(if eye_out { 0x0204 } else { 0x0201 }); // GL_GREATER, GL_LESS
            plane_quad(axis, at);
        }
        glDepthFunc(0x0201); // GL_LESS
        // Wipe the depth where the mark is left
        glStencilFunc(EQUAL, 3, !0);
        glStencilOp(KEEP, KEEP, KEEP);
        glDepthFunc(ALWAYS);
        glDepthMask(1);
        glDepthRange(1.0, 1.0);
        opening();
        glDepthRange(0.0, 1.0);
        glDepthFunc(0x0201); // GL_LESS
        // And put the stencil back
        glDisable(0x0B71); // GL_DEPTH_TEST
        glDepthMask(0);
        glStencilFunc(ALWAYS, 1, !0);
        glStencilOp(REPLACE, REPLACE, REPLACE);
        opening();
        glEnable(0x0B71);
        glDepthMask(1);
        glStencilOp(KEEP, KEEP, KEEP);
        glDisable(0x0B90);
        glColorMask(1, 1, 1, 1);
        glPolygonMode(0x0408, fill_mode);
        if textured {
            glEnable(0x0DE1);
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
