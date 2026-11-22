//! Drawing the level: the view, the rooms the portals show, and their objects.

use std::collections::HashMap;
use rogue_reborn::{casings, map, sim};
use rogue_reborn::net::watch::{BEHIND, Pose, RISE, behind};
use rogue_reborn::visibility::{PlanNode, RectRoom};
use crate::args::Args;
use crate::camera::{Camera, apply_camera};
use crate::opengl::*;
use crate::render::draw::{Which, draw_plan, wipe_opening, room_color};
use crate::render::level::LevelDraw;
use crate::session::Session;
use crate::state::Toggles;
use crate::textures::ensure_texture;
use crate::view::PlayerView;
use crate::VIEW_HIGH;

// What the world pass reads besides the toggles and the view
pub struct WorldPass<'a> {
    pub args: &'a Args,
    pub level: &'a map::Map,
    pub sim: &'a sim::Sim,
    pub session: &'a Session,
    pub level_draw: &'a LevelDraw,
    pub me: usize,
    pub game_time: f32,
    pub aspect: f32,
    pub z_near: f32,
    pub z_far: f32,
    pub watch_pose: Option<Pose>,
    pub camera_feet: [f32; 3],
    pub current_rooms: &'a [&'a str],
}

// What the rest of the frame needs from the world's drawing
pub struct WorldDrawn {
    pub view_to_map: Option<casings::Mat4>,
    pub visible_rooms: Vec<u32>,
    pub drawn_rooms: Vec<String>,
    // Culling is off, or the player is in no room
    pub plan_empty: bool,
}

impl WorldDrawn {
    /// Whether something at `feet` (in map space) shows: the level overlaps itself, and on rm05 a
    /// character on the plane's upper deck pokes out of the top of the hull. It shows only when
    /// a room it is in is drawn. With no rooms drawn (culling off) everyone shows.
    pub fn shows(&self, sim: &sim::Sim, level: &map::Map, feet: [f32; 3]) -> bool {
        self.visible_rooms.is_empty()
            || sim.visibility.rooms_holding(level, [feet[0], feet[1] + 10.0, feet[2]], None).iter().any(|r| self.visible_rooms.contains(r))
    }
}

// What drawing some of the level's objects reads, and the textures it loads
struct ObjectDraw<'a> {
    args: &'a Args,
    level: &'a map::Map,
    sim: &'a sim::Sim,
    session: &'a Session,
    level_draw: &'a LevelDraw,
    toggles: &'a Toggles,
    current_rooms: &'a [&'a str],
    textures: &'a mut HashMap<String, Option<u32>>,
}

impl ObjectDraw<'_> {
    // Draws some of the level's objects, and only the part beyond `plane`.
    // An object belongs to the room its name starts with ("204", "20_doormetal").
    fn draw(&mut self, which: Which, planes: &[[f32; 4]]) {
        let ObjectDraw { args, level, sim, session, level_draw, toggles, current_rooms, textures } = self;
        unsafe {
            set_clip_planes(planes);
            let indices: &[usize] = match which {
                Which::All => &level_draw.all_objects,
                Which::Room(n) => sim.visibility.objects_of(n),
                Which::Unowned => sim.visibility.unowned_objects(),
            };
            // The doors of these rooms are drawn along with them, each leaf where
            // its door is
            let room_doors: &[usize] = match which {
                Which::All => &level_draw.all_doors,
                Which::Room(n) => level_draw.doors_by_room.get(&n).map(Vec::as_slice).unwrap_or_default(),
                Which::Unowned => &[],
            };
            let items: Vec<(usize, Option<usize>)> = indices.iter().map(|&i| (i, None)).chain(room_doors.iter().map(|&d| (sim.doors[d].model_index, Some(d)))).collect();
            // Opaque and cut-out faces first, then the soft sprites over them
            for pass in 0..2 {
                for &(object_index, door) in &items {
                    let object = &level.geometries.objects[object_index];
                    // Panes are drawn in place below, and door leaves with their doors
                    if level_draw.pane_objects[object_index] || (level_draw.door_leaf[object_index] && door.is_none()) {
                        continue;
                    }
                    let room = sim.visibility.room_of_object(object_index);
                    if room.is_some_and(|n| args.hide_rooms.contains(&n)) {
                        continue;
                    }
                    if let Some(d) = door {
                        glPushMatrix();
                        glMultMatrixf(sim.doors[d].matrix(sim.door_states[d].open).as_ptr());
                    }
                    if toggles.room_debug {
                        let [r, g, b] = room_color(&object.name, current_rooms.iter().any(|n| *n == object.name));
                        glColor3f(r, g, b);
                    }
                    for (od_index, od) in object.object_datas.iter().enumerate() {
                        let mat_index = od.material as usize;
                        let blend = if toggles.textured { level_draw.sprite_blends.get(mat_index).copied().flatten() } else { None };
                        if blend.is_some() != (pass == 1) {
                            continue;
                        }
                        match blend {
                            Some(additive) => {
                                // The sprite writes depth, so that a room drawn again through another
                                // portal does not blend it a second time (a shadow twice as dark, a
                                // light twice as bright). The alpha test keeps its clear pixels out
                                // of the depth buffer.
                                glEnable(GL_ALPHA_TEST);
                                glAlphaFunc(GL_GREATER, 0.02);
                                glEnable(GL_BLEND);
                                glBlendFunc(GL_SRC_ALPHA, if additive { GL_ONE } else { GL_ONE_MINUS_SRC_ALPHA });
                                glDepthMask(u8::from(toggles.sprite_mode != 2));
                            }
                            None => {
                                glDepthMask(1);
                                glDisable(GL_BLEND);
                                if toggles.textured {
                                    glEnable(GL_ALPHA_TEST);
                                    glAlphaFunc(GL_GREATER, 0.5);
                                }
                            }
                        }
                        // A face is seen from the side its normal points to (its corners in
                        // order turn counterclockwise there), as in the original engine; the
                        // mirrored z makes that clockwise on screen. Two-sided materials (glass,
                        // foliage, nets) show both. Most impossible geometry is a room's surfaces
                        // seen from behind (rm05's cabin walls from outside, rm11's hall's stairs
                        // from inside 119), and this hides it.
                        if !args.no_cull_backs {
                            if level.materials.materials.get(mat_index).is_some_and(|m| m.two_sided) {
                                glDisable(GL_CULL_FACE);
                            } else {
                                glEnable(GL_CULL_FACE);
                                glCullFace(GL_BACK);
                                glFrontFace(GL_CW);
                            }
                        }
                        // Room colors replace the textures, so they show even where textures are missing
                        let tex_id = if toggles.room_debug { Some(0) } else { ensure_texture(&level.materials, mat_index, &session.map_num, textures) };
                        if let Some(tex_id) = tex_id {
                            // TODO put all faces with same tex-id together so I can bind once
                            if !toggles.room_debug {
                                glBindTexture(GL_TEXTURE_2D, tex_id);
                            }

                            // Faces and UVs are parallel arrays
                            let face_count = od.faces.face_indices.len().min(od.faces.texture_indices.len());
                            if face_count == 0 {
                                continue;
                            }

                            // Each face corner's light, baked at load; soft sprites and lamps are not lit
                            let shades = session.baked.as_ref().filter(|_| toggles.textured && !toggles.room_debug && blend.is_none() && !level_draw.glowing.get(mat_index).copied().unwrap_or(false))
                                .map(|b| &b.colors[object_index][od_index]);
                            if shades.is_none() && toggles.textured && !toggles.room_debug {
                                glColor3f(1.0, 1.0, 1.0);
                            }

                            // Faces over others in their plane come after, each layer nearer
                            let layers = &level_draw.coplanar_layers[object_index];
                            for layer in 0..=layers.max {
                                // Most groups have faces in only one of their object's layers
                                if !layers.per_face[od_index][..face_count].contains(&layer) {
                                    continue;
                                }
                                // A sprite's faces come nearer than the surface they lie on
                                let offset = f32::from(layer) + if blend.is_some() && toggles.sprite_mode == 0 { 2.0 } else { 0.0 };
                                if offset > 0.0 {
                                    glEnable(GL_POLYGON_OFFSET_FILL);
                                    glPolygonOffset(-offset, -offset);
                                }
                                glBegin(GL_TRIANGLES);
                                for i in 0..face_count {
                                    if layers.per_face[od_index][i] != layer {
                                        continue;
                                    }
                                    let (i0, i1, i2) = od.faces.face_indices[i];
                                    let (t0, t1, t2) = od.faces.texture_indices[i];

                                    let p0 = &object.vertices[i0 as usize];
                                    let p1 = &object.vertices[i1 as usize];
                                    let p2 = &object.vertices[i2 as usize];

                                    let uv0 = &od.texture_vertices.uv_coords[t0 as usize];
                                    let uv1 = &od.texture_vertices.uv_coords[t1 as usize];
                                    let uv2 = &od.texture_vertices.uv_coords[t2 as usize];

                                    if toggles.textured {
                                        let c = shades.map_or([[1.0; 3]; 3], |s| s[i]);
                                        if shades.is_some() { glColor3f(c[0][0], c[0][1], c[0][2]); }
                                        glTexCoord2f(uv0.u, uv0.v); glVertex3f(p0.x, p0.y, p0.z);
                                        if shades.is_some() { glColor3f(c[1][0], c[1][1], c[1][2]); }
                                        glTexCoord2f(uv1.u, uv1.v); glVertex3f(p1.x, p1.y, p1.z);
                                        if shades.is_some() { glColor3f(c[2][0], c[2][1], c[2][2]); }
                                        glTexCoord2f(uv2.u, uv2.v); glVertex3f(p2.x, p2.y, p2.z);
                                    } else {
                                        glVertex3f(p0.x, p0.y, p0.z);
                                        glVertex3f(p1.x, p1.y, p1.z);
                                        glVertex3f(p2.x, p2.y, p2.z);
                                    }
                                }
                                glEnd();
                            }
                            if layers.max > 0 || blend.is_some() {
                                glDisable(GL_POLYGON_OFFSET_FILL);
                            }
                        }
                    }
                    if door.is_some() {
                        glPopMatrix();
                    }
                }
            }
            glDisable(GL_CULL_FACE);
            glDisable(GL_ALPHA_TEST);
            glDisable(GL_BLEND);
            glDepthMask(1);
        }
    }
}

/// Clears the frame, sets the view, and draws the level through the portals the player sees
pub fn draw_world(pass: WorldPass, toggles: &Toggles, pview: &mut PlayerView, textures: &mut HashMap<String, Option<u32>>) -> WorldDrawn {
    let WorldPass { args, level, sim, session, level_draw, me, game_time, aspect, z_near, z_far, watch_pose, camera_feet, current_rooms } = pass;
    let view_to_map: Option<casings::Mat4>;
    let drawn_rooms: Vec<String>;
    let visible_rooms: Vec<u32>;
    let plan_empty: bool;
    unsafe {
        glClearStencil(1);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        // The HUD leaves the depth test off
        glEnable(GL_DEPTH_TEST);
        glMatrixMode(GL_PROJECTION);
        glLoadIdentity();
        glu_perspective(VIEW_HIGH, aspect, z_near, z_far);

        glMatrixMode(GL_MODELVIEW);
        glLoadIdentity();
        // A blast shakes the view: a quick tremble that dies away
        if pview.shake > 0.002 && watch_pose.is_none() {
            let t = game_time;
            let s = pview.shake.min(1.0);
            glRotatef(5.0 * s * (t * 47.0).sin(), 0.0, 0.0, 1.0);
            glTranslatef(9.0 * s * (t * 61.0 + 1.0).sin(), 7.0 * s * (t * 53.0 + 2.0).sin(), 0.0);
            glRotatef(2.5 * s * (t * 39.0 + 3.0).sin(), 1.0, 0.0, 0.0);
        }
        // Third person: pull the view back behind and above the player's eyes
        let view = if toggles.first_person && toggles.third_person && watch_pose.is_none() {
            // Orbited round the player to see them from the side, if asked, and kept out
            // of the walls as the camera behind a watched player is
            let eye = [pview.camera.position[0], pview.camera.position[1], -pview.camera.position[2]];
            let from = Pose { eye, yaw: pview.camera.yaw, pitch: (pview.camera.pitch + toggles.orbit.1).clamp(-89.0, 89.0) };
            let p = behind(sim, from, args.view_dist.unwrap_or(BEHIND), RISE, toggles.orbit.0);
            Camera { position: [p.eye[0], p.eye[1], -p.eye[2]], yaw: p.yaw, pitch: p.pitch }
        } else {
            Camera { position: pview.camera.position, yaw: pview.camera.yaw, pitch: pview.camera.pitch }
        };
        apply_camera(&view);
        glScalef(1.0, 1.0, -1.0);
        let mut world_view = [0f32; 16];
        glGetFloatv(GL_MODELVIEW_MATRIX, world_view.as_mut_ptr());
        view_to_map = casings::invert_affine(&world_view);

        // What the portals show from here: the rooms the player is in, and each room
        // seen through the previous one's portals (see visibility::plan)
        let cam_eye = [view.position[0], view.position[1], -view.position[2]];
        let mut plan: Vec<PlanNode> = Vec::new();
        // Each visible room once, with its screen rectangle (unless --portal-paths)
        let mut rect_plan: Vec<RectRoom> = Vec::new();
        // Dead on a server, drawn from the rooms of whoever the camera is on: the player's own
        // rooms are not followed while they are dead
        let (cull_rooms, cull_body) = if session.net.is_some() && sim.players[me].respawn_at.is_some() {
            let (rooms, room) = sim.standing_rooms(camera_feet, pview.camera_room);
            pview.camera_room = room;
            (rooms, [[camera_feet[0], camera_feet[1] + 10.0, camera_feet[2]], cam_eye])
        } else {
            (sim.players[me].render_rooms.clone(), sim.players[me].body)
        };
        if sim.culling && !cull_rooms.is_empty() {
            let (mut modelview, mut projection) = ([0f32; 16], [0f32; 16]);
            glGetFloatv(GL_MODELVIEW_MATRIX, modelview.as_mut_ptr());
            glGetFloatv(GL_PROJECTION_MATRIX, projection.as_mut_ptr());
            let clip = |v: [f32; 3]| mul_mat4(&projection, mul_mat4(&modelview, [v[0], v[1], v[2], 1.0]));
            if args.portal_paths {
                plan = cull_rooms.iter().map(|&r| sim.visibility.plan(level, r, &cull_body, cam_eye, &sim.blocked_portals, &clip)).collect();
            } else {
                rect_plan = sim.visibility.rect_plan(level, &cull_rooms, &cull_body, cam_eye, &sim.blocked_portals, &clip);
                // The player's rooms and the doorways they stand in are still cut where
                // they overlap
                plan = cull_rooms.iter().map(|&r| sim.visibility.touched_plan(level, r, &cull_body, cam_eye, &sim.blocked_portals)).collect();
            }
        }
        let mut rooms: Vec<u32> = rect_plan.iter().map(|rr| rr.room).collect();
        for node in &plan {
            node.rooms(&mut rooms);
        }
        rooms.sort();
        rooms.dedup();
        drawn_rooms = rooms.iter().map(|n| n.to_string()).collect();
        visible_rooms = rooms;
        plan_empty = plan.is_empty();

        // Combined
        {
            // Textured mode
            if toggles.textured {
                glEnable(GL_TEXTURE_2D);
                glPolygonMode(GL_FRONT_AND_BACK, GL_FILL);
            } else {
                glDisable(GL_TEXTURE_2D);
                glPolygonMode(GL_FRONT_AND_BACK, GL_LINE);
            }
            let level_mode = if toggles.textured { GL_FILL } else { GL_LINE };

            // Iterate map geometry by material groups
            if toggles.room_debug {
                glDisable(GL_TEXTURE_2D);
            }

            let mut objects = ObjectDraw { args, level, sim, session, level_draw, toggles, current_rooms, textures };

            if !rect_plan.is_empty() {
                // The player's rooms and the doorways they stand in, cut where they
                // overlap, then every other room whole, within its rectangle, nearest
                // first, sorted out by depth as the original engine does. Inside the
                // openings of the player's rooms' own doorways the room beyond shows in
                // front of them: rm11's hall's wall, one step past its door into 119 (its
                // stairs there are seen from behind, and culled). Deeper, overlaps show,
                // as in the original (the hall's wall in 120's doorway, see BUGS.txt)
                objects.draw(Which::Unowned, &[]);
                let mut cut_rooms: Vec<u32> = Vec::new();
                glEnable(GL_STENCIL_TEST);
                for node in &plan {
                    node.rooms(&mut cut_rooms);
                    draw_plan(node, 1, level_mode, toggles.textured && !toggles.room_debug, &mut |room, planes| objects.draw(Which::Room(room), planes));
                }
                glDisable(GL_STENCIL_TEST);
                set_clip_planes(&[]);
                let mut viewport = [0i32; 4];
                glGetIntegerv(GL_VIEWPORT, viewport.as_mut_ptr());
                glEnable(GL_SCISSOR_TEST);
                for rr in rect_plan.iter().filter(|rr| !cut_rooms.contains(&rr.room)) {
                    let r = rr.rect;
                    let x0 = viewport[0] + ((r[0].max(-1.0) + 1.0) / 2.0 * viewport[2] as f32).floor() as i32;
                    let y0 = viewport[1] + ((r[1].max(-1.0) + 1.0) / 2.0 * viewport[3] as f32).floor() as i32;
                    let x1 = viewport[0] + ((r[2].min(1.0) + 1.0) / 2.0 * viewport[2] as f32).ceil() as i32;
                    let y1 = viewport[1] + ((r[3].min(1.0) + 1.0) / 2.0 * viewport[3] as f32).ceil() as i32;
                    glScissor(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0));
                    // Through the player's room's own doorways, as the original engine does
                    for &(p, plan) in &rr.doorways {
                        let height = sim.visibility.height(rr.room);
                        wipe_opening(sim.visibility.polygon(p), plan, height, cam_eye, level_mode, toggles.textured && !toggles.room_debug);
                    }
                    objects.draw(Which::Room(rr.room), &[]);
                }
                glDisable(GL_SCISSOR_TEST);
            } else if plan.is_empty() {
                objects.draw(Which::All, &[]);
            } else {
                // Things that belong to no room first, then each room through its portals
                objects.draw(Which::Unowned, &[]);
                glEnable(GL_STENCIL_TEST);
                for node in &plan {
                    draw_plan(node, 1, level_mode, toggles.textured && !toggles.room_debug, &mut |room, planes| objects.draw(Which::Room(room), planes));
                }
                glDisable(GL_STENCIL_TEST);
            }
            set_clip_planes(&[]);
            glColor3f(1.0, 1.0, 1.0);
        }
    }
    WorldDrawn { view_to_map, visible_rooms, drawn_rooms, plan_empty }
}
