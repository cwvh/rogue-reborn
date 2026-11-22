//! Drawing the level, characters, glass and weapons.

use std::cell::RefCell;
use std::collections::HashMap;
use rogue_reborn::{map, crp, glass, visibility};
use rogue_reborn::lighting::CharacterLight;
use crate::opengl::*;
use crate::textures::{ensure_texture, is_tga};

// The weapon model as flat triangle corner arrays, ready to draw
pub struct WeaponDraw {
    pub verts: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub texture: Option<u32>,
    pub muzzle: [f32; 3],
    /// Middle of the model's bounding box, and its longest side
    pub center: [f32; 3],
    pub length: f32,
    /// Zero means no muzzle flash
    pub flash_scale: f32,
}

/// `light` is the color the level's light gives the gun
pub fn draw_weapon(wd: &WeaponDraw, textured: bool, light: [f32; 3]) {
    unsafe {
        if let (true, Some(tex)) = (textured, wd.texture) {
            glEnable(GL_TEXTURE_2D);
            glBindTexture(GL_TEXTURE_2D, tex);
            glColor4f(light[0], light[1], light[2], 1.0);
        } else {
            glDisable(GL_TEXTURE_2D);
            glColor4f(0.7, 0.7, 0.75, 1.0);
        }
        glBegin(GL_TRIANGLES);
        for (v, uv) in wd.verts.iter().zip(&wd.uvs) {
            glTexCoord2f(uv[0], uv[1]);
            glVertex3f(v[0], v[1], v[2]);
        }
        glEnd();
    }
}

pub struct FlashDraw {
    pub parts: Vec<FlashPart>,
}

pub struct FlashPart {
    pub verts: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub texture: u32,
}

pub struct FlashSpot {
    pub matrix: [f32; 16],
    pub muzzle: [f32; 3],
    pub scale: f32,
}

impl FlashSpot {
    pub fn here(wd: &WeaponDraw, roll: f32) -> Self {
        let mut matrix = [0.0f32; 16];
        unsafe { glGetFloatv(GL_MODELVIEW_MATRIX, matrix.as_mut_ptr()); }
        Self { matrix, muzzle: wd.muzzle, scale: wd.flash_scale * (0.8 + 0.4 * roll) }
    }
}

pub fn draw_flash(flash: &FlashDraw, spot: &FlashSpot) {
    if spot.scale <= 0.0 {
        return;
    }
    unsafe {
        glPushMatrix();
        glLoadIdentity();
        glMultMatrixf(spot.matrix.as_ptr());
        draw_flash_here(flash, spot);
        glPopMatrix();
    }
}

// Needs the weapon's model-view matrix to be current
pub fn draw_flash_here(flash: &FlashDraw, spot: &FlashSpot) {
    if spot.scale <= 0.0 {
        return;
    }
    unsafe {
        glPushMatrix();
        glTranslatef(spot.muzzle[0], spot.muzzle[1], spot.muzzle[2]);
        glScalef(spot.scale, spot.scale, spot.scale);
        glDisable(GL_CULL_FACE);
        glEnable(GL_TEXTURE_2D);
        glEnable(GL_BLEND);
        glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        glDepthMask(0);
        glColor4f(1.0, 1.0, 1.0, 1.0);
        for part in &flash.parts {
            glBindTexture(GL_TEXTURE_2D, part.texture);
            glBegin(GL_TRIANGLES);
            for (v, uv) in part.verts.iter().zip(&part.uvs) {
                glTexCoord2f(uv[0], uv[1]);
                glVertex3f(v[0], v[1], v[2]);
            }
            glEnd();
        }
        glDepthMask(1);
        glDisable(GL_BLEND);
        glPopMatrix();
    }
}

// The original shows each shot's flash for at least one frame.
// It stays visible until FLASH_SECONDS after the shot.
#[derive(Default)]
pub struct Flashes {
    // Per gun, the last shot time seen and whether its flash has been shown
    seen: HashMap<usize, (f32, bool)>,
}

const FLASH_SECONDS: f32 = 0.02;
// A shot older than this is not flashed when first seen
const NEW_SHOT_SECONDS: f32 = 0.25;

impl Flashes {
    pub fn shows(&mut self, gun: usize, shot: f32, now: f32) -> bool {
        let e = self.seen.entry(gun).or_insert((f32::NEG_INFINITY, true));
        if shot != e.0 {
            *e = (shot, now - shot > NEW_SHOT_SECONDS);
        }
        let show = !e.1 || now - shot <= FLASH_SECONDS;
        e.1 = true;
        show
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
    // The same for the corners' colors, when the character is lit
    colors: RefCell<Vec<f32>>,
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
            colors: RefCell::new(Vec::new()),
        }
    }
}

// Draw one posed character: `verts` are its (possibly animated) vertex
// positions and `face` replaces the HRT placeholder face texture. `light` is the level's
// light on it, in the character's own space; with none it is drawn as it is.
pub fn draw_character(c: &crp::Crp, arrays: &CharacterArrays, verts: &[[f32; 3]], turns: &[[f32; 4]], part_textures: &[Option<u32>], face: Option<u32>, textured: bool, glow: [f32; 3], light: Option<&CharacterLight>) {
    // The posed position of every corner, gathered once for all the parts
    let mut positions = arrays.positions.borrow_mut();
    positions.clear();
    positions.extend(arrays.corner_vertex.iter().flat_map(|&i| verts[i as usize]));
    let mut colors = arrays.colors.borrow_mut();
    let lit = textured && light.is_some();
    if let (true, Some(light)) = (lit, light) {
        light.corner_colors(c, verts, turns, &mut colors);
    }
    unsafe {
    glEnableClientState(GL_VERTEX_ARRAY);
    glEnableClientState(GL_TEXTURE_COORD_ARRAY);
    glVertexPointer(3, GL_FLOAT, 0, positions.as_ptr().cast());
    glTexCoordPointer(2, GL_FLOAT, 0, arrays.uvs.as_ptr().cast());
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
        if has_alpha && textured { glEnable(GL_BLEND); glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA); }
        // Solid mode shows the texture; wireframe shows flat color lines
        if let (true, Some(id)) = (textured, tex) {
            glEnable(GL_TEXTURE_2D);
            glBindTexture(GL_TEXTURE_2D, id);
            glColor4f(1.0, 1.0, 1.0, 1.0);
            if lit {
                glEnableClientState(GL_COLOR_ARRAY);
                glColorPointer(3, GL_FLOAT, 0, colors.as_ptr().cast());
            }
        } else {
            glDisable(GL_TEXTURE_2D);
            glDisableClientState(GL_COLOR_ARRAY);
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
        glDrawArrays(GL_TRIANGLES, arrays.first[pi], part.triangles.len() as i32 * 3);
        if has_alpha { glDisable(GL_BLEND); }
    }
    }
    glDisableClientState(GL_COLOR_ARRAY);
    // A blast's light on the body: the opaque parts again, added over themselves
    if textured && glow.iter().any(|&g| g > 0.003) {
        glEnable(GL_BLEND);
        glBlendFunc(GL_ONE, GL_ONE);
        glDepthFunc(GL_LEQUAL); // The same triangles again
        glDepthMask(0);
        glEnable(GL_POLYGON_OFFSET_FILL);
        glPolygonOffset(-1.0, -1.0);
        glEnable(GL_TEXTURE_2D);
        glColor4f(glow[0], glow[1], glow[2], 1.0);
        for (pi, part) in c.parts.iter().enumerate() {
            if is_tga(&part.material) { continue; }
            let is_face = part.material.eq_ignore_ascii_case("hrt_face.bmp");
            let tex = if is_face { face } else { None }.or(part_textures.get(pi).copied().flatten());
            let Some(id) = tex else { continue };
            glBindTexture(GL_TEXTURE_2D, id);
            glDrawArrays(GL_TRIANGLES, arrays.first[pi], part.triangles.len() as i32 * 3);
        }
        glDisable(GL_POLYGON_OFFSET_FILL);
        glDepthMask(1);
        glDepthFunc(GL_LESS);
        glDisable(GL_BLEND);
        glColor4f(1.0, 1.0, 1.0, 1.0);
    }
    glDisableClientState(GL_TEXTURE_COORD_ARRAY);
    glDisableClientState(GL_VERTEX_ARRAY);
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
        glEnable(GL_BLEND);
        glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        match (textured, ensure_texture(materials, material, map_num, textures)) {
            (true, Some(id)) => {
                glEnable(GL_TEXTURE_2D);
                glBindTexture(GL_TEXTURE_2D, id);
                glColor4f(1.0, 1.0, 1.0, alpha);
            }
            _ => {
                glDisable(GL_TEXTURE_2D);
                glColor4f(0.6, 0.8, 0.9, alpha.max(0.3));
            }
        }
        glBegin(GL_TRIANGLES);
        for (p, uv) in tris {
            for k in 0..3 {
                glTexCoord2f(uv[k][0], uv[k][1]);
                glVertex3f(p[k][0], p[k][1], p[k][2]);
            }
        }
        glEnd();
        glDisable(GL_BLEND);
        glColor4f(1.0, 1.0, 1.0, 1.0);
    }
}

// Draw a broken pane: its pieces with a glint along their edges, and the flying flecks
pub fn draw_fracture(materials: &map::Materials, map_num: &str, textures: &mut HashMap<String, Option<u32>>, textured: bool, f: &glass::Fracture) {
    unsafe {
        glEnable(GL_BLEND);
        glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        let tex = if textured { ensure_texture(materials, f.material, map_num, textures) } else { None };
        for s in &f.shards {
            let (corners, alpha) = (s.corners(), s.alpha());
            if let Some(id) = tex {
                glEnable(GL_TEXTURE_2D);
                glBindTexture(GL_TEXTURE_2D, id);
                glColor4f(1.0, 1.0, 1.0, alpha * f.opacity);
            } else {
                glDisable(GL_TEXTURE_2D);
                glColor4f(0.6, 0.8, 0.9, alpha * 0.4);
            }
            glBegin(GL_TRIANGLE_FAN);
            for (c, uv) in corners.iter().zip(s.uvs()) {
                glTexCoord2f(uv[0], uv[1]);
                glVertex3f(c[0], c[1], c[2]);
            }
            glEnd();
            // Edges catch the light
            glDisable(GL_TEXTURE_2D);
            glColor4f(0.9, 0.97, 1.0, alpha * 0.75);
            glBegin(GL_LINE_LOOP);
            for c in &corners {
                glVertex3f(c[0], c[1], c[2]);
            }
            glEnd();
        }
        glPointSize(2.5);
        glBegin(GL_POINTS);
        for fl in &f.flecks {
            glColor4f(0.95, 0.98, 1.0, fl.alpha());
            glVertex3f(fl.pos[0], fl.pos[1], fl.pos[2]);
        }
        glEnd();
        glDisable(GL_BLEND);
        glColor4f(1.0, 1.0, 1.0, 1.0);
    }
}

// Stable, distinguishable color for a material name
pub fn material_color(name: &str) -> (f32, f32, f32) {
    let h = name.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let c = |shift: u32| 0.35 + 0.65 * ((h >> shift) & 0xff) as f32 / 255.0;
    (c(0), c(8), c(16))
}

// Bounding radius of the level's geometry, for the clip planes
pub fn level_radius(level: &map::Map) -> f32 {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for v in level.geometries.objects.iter().flat_map(|o| &o.vertices) {
        for (i, c) in [v.x, v.y, v.z].into_iter().enumerate() {
            min[i] = min[i].min(c);
            max[i] = max[i].max(c);
        }
    }
    if min[0] > max[0] {
        return 10.0;
    }
    let d: f32 = (0..3).map(|i| (max[i] - min[i]) * 0.5).map(|h| h * h).sum();
    d.sqrt().max(1.0)
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
    unsafe {
        glStencilFunc(GL_EQUAL, level, !0);
        glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
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
                glBegin(GL_TRIANGLE_FAN);
                for p in child.polygon {
                    glVertex3f(p[0], p[1], p[2]);
                }
                glEnd();
            };
            set_clip_planes(&[]);
            glDisable(GL_TEXTURE_2D);
            glPolygonMode(GL_FRONT_AND_BACK, GL_FILL);
            glColorMask(0, 0, 0, 0);
            glDepthMask(0);

            // Mark the part of the opening that is not behind something nearer. The polygon
            // is nudged deeper, so walls lying in the portal's own plane are not wiped.
            glStencilFunc(GL_EQUAL, level, !0);
            glStencilOp(GL_KEEP, GL_KEEP, GL_INCR);
            glEnable(GL_POLYGON_OFFSET_FILL);
            glPolygonOffset(2.0, 2.0);
            opening();
            glDisable(GL_POLYGON_OFFSET_FILL);
            // Wipe the depth there, so the room beyond competes only with itself
            glStencilFunc(GL_EQUAL, level + 1, !0);
            glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
            glDepthFunc(GL_ALWAYS);
            glDepthMask(1);
            glDepthRange(1.0, 1.0);
            opening();
            glDepthRange(0.0, 1.0);
            glDepthFunc(GL_LESS);
            glColorMask(1, 1, 1, 1);
            glPolygonMode(GL_FRONT_AND_BACK, fill_mode);
            if textured {
                glEnable(GL_TEXTURE_2D);
            }

            draw_plan(child, level + 1, fill_mode, textured, draw);

            // The opening is done: nothing else may draw into it
            set_clip_planes(&[]);
            glColorMask(0, 0, 0, 0);
            glDepthMask(0);
            glDisable(GL_DEPTH_TEST);
            glPolygonMode(GL_FRONT_AND_BACK, GL_FILL);
            glStencilFunc(GL_EQUAL, level + 1, !0);
            glStencilOp(GL_KEEP, GL_KEEP, GL_ZERO);
            opening();
            glEnable(GL_DEPTH_TEST);
            glDepthMask(1);
            glColorMask(1, 1, 1, 1);
            glPolygonMode(GL_FRONT_AND_BACK, fill_mode);
            if textured {
                glEnable(GL_TEXTURE_2D);
            }
            glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
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
    unsafe {
        let opening = || {
            glBegin(GL_TRIANGLE_FAN);
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
            glBegin(GL_QUADS);
            for (du, dv) in [(-FAR, -FAR), (FAR, -FAR), (FAR, FAR), (-FAR, FAR)] {
                let mut p = [0.0f32; 3];
                p[axis] = at;
                p[u] = eye[u] + du;
                p[v] = eye[v] + dv;
                glVertex3f(p[0], p[1], p[2]);
            }
            glEnd();
        };
        glEnable(GL_STENCIL_TEST);
        glDisable(GL_TEXTURE_2D);
        glPolygonMode(GL_FRONT_AND_BACK, GL_FILL);
        glColorMask(0, 0, 0, 0);
        glDepthMask(0);
        // Mark the visible part. The polygon is nudged deeper, so walls lying in the portal's
        // own plane are not wiped.
        glStencilFunc(GL_ALWAYS, 3, !0);
        glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
        glEnable(GL_POLYGON_OFFSET_FILL);
        glPolygonOffset(2.0, 2.0);
        opening();
        glDisable(GL_POLYGON_OFFSET_FILL);
        // Unmark where what is drawn lies outside the box, on the outer side of one of its
        // planes: past the plane where the eye is inside it (the plane is nearer), in front of
        // it where the eye is outside (the plane is farther). Not only the box's far faces: a
        // line of sight can pass over a low room's box without going through it.
        glStencilFunc(GL_EQUAL, 3, !0);
        glStencilOp(GL_KEEP, GL_KEEP, GL_DECR);
        for (axis, at, out) in planes {
            let eye_out = (eye[axis] - at) * out > 0.0;
            glDepthFunc(if eye_out { GL_GREATER } else { GL_LESS });
            plane_quad(axis, at);
        }
        glDepthFunc(GL_LESS);
        // Wipe the depth where the mark is left
        glStencilFunc(GL_EQUAL, 3, !0);
        glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
        glDepthFunc(GL_ALWAYS);
        glDepthMask(1);
        glDepthRange(1.0, 1.0);
        opening();
        glDepthRange(0.0, 1.0);
        glDepthFunc(GL_LESS);
        // And put the stencil back
        glDisable(GL_DEPTH_TEST);
        glDepthMask(0);
        glStencilFunc(GL_ALWAYS, 1, !0);
        glStencilOp(GL_REPLACE, GL_REPLACE, GL_REPLACE);
        opening();
        glEnable(GL_DEPTH_TEST);
        glDepthMask(1);
        glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
        glDisable(GL_STENCIL_TEST);
        glColorMask(1, 1, 1, 1);
        glPolygonMode(GL_FRONT_AND_BACK, fill_mode);
        if textured {
            glEnable(GL_TEXTURE_2D);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flash_shows_once_then_for_a_short_while() {
        let mut f = Flashes::default();
        assert!(!f.shows(0, f32::NEG_INFINITY, 1.0));
        assert!(!f.shows(0, 0.5, 1.0), "an old shot is not new");
        // A slow frame still shows the flash once
        assert!(f.shows(0, 1.0, 1.1));
        assert!(!f.shows(0, 1.0, 1.2));
        assert!(f.shows(0, 2.0, 2.005));
        assert!(f.shows(0, 2.0, 2.015));
        assert!(!f.shows(0, 2.0, 2.025));
        assert!(f.shows(0, 2.075, 2.08));
        assert!(f.shows(1, 2.075, 2.08));
    }
}
