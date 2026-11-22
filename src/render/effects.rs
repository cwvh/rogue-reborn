//! Drawing the light of blasts, bullet holes, debris, glass, casings, muzzle flashes and the
//! debug outlines.

use std::collections::HashMap;
use rogue_reborn::{blastlight, casings, glass, impact, map, sim};
use rogue_reborn::util::unit3;
use crate::audio::Audio;
use crate::camera::{Camera, listener_of};
use crate::opengl::*;
use crate::render::assets::Assets;
use crate::render::characters::FrameOutput;
use crate::render::draw::{draw_flash, draw_fracture, draw_glass};
use crate::render::world::WorldDrawn;
use crate::scene::Scene;
use crate::session::Session;
use crate::state::Toggles;

pub const MOST_CASINGS_A_FRAME: u32 = 4;
const SHELL_SOUND_GAP: f32 = 0.2;
const SHELL_SOUND_GAIN: f32 = 0.5;

// A blast's embers: how wide they are, and the width of a pixel for each unit of distance
// (a 60 degree view about a thousand pixels high)
const EMBER_SIZE: f32 = 2.0;
const ONE_PIXEL_PER_UNIT: f32 = 0.0012;

/// The light of a blast, over what it reaches
pub fn draw_blast_lights(lights: &mut Vec<blastlight::BlastLight>, assets: &Assets, textured: bool, step_dt: f32) {
    for l in lights.iter_mut() {
        l.update(step_dt);
    }
    lights.retain(|l| !l.done());
    if let (true, Some(id)) = (textured && !lights.is_empty(), assets.light_tex) {
        unsafe {
            set_clip_planes(&[]);
            glEnable(GL_TEXTURE_2D);
            glBindTexture(GL_TEXTURE_2D, id);
            glEnable(GL_BLEND);
            glBlendFunc(GL_ONE, GL_ONE); // It adds to what is there
            glDisable(GL_CULL_FACE);
            glEnable(GL_POLYGON_OFFSET_FILL); // The light lies on the surface it lights
            glPolygonOffset(-1.0, -1.0);
            glDepthFunc(GL_LEQUAL);
            glDepthMask(0);
            glBegin(GL_TRIANGLES);
            for l in lights.iter() {
                let (k, c) = (l.intensity(), l.color());
                for lit in &l.lit {
                    for ((corner, uv), strength) in lit.corners.iter().zip(&lit.uv).zip(&lit.strength) {
                        let s = k * strength;
                        glColor3f(c[0] * s, c[1] * s, c[2] * s);
                        glTexCoord2f(uv[0], uv[1]);
                        glVertex3f(corner[0], corner[1], corner[2]);
                    }
                }
            }
            glEnd();
            glDepthFunc(GL_LESS);
            glDepthMask(1);
            glDisable(GL_POLYGON_OFFSET_FILL);
            glDisable(GL_BLEND);
            glColor4f(1.0, 1.0, 1.0, 1.0);
        }
    }
}

/// Bullet holes on the walls, then the debris thrown from them
pub fn draw_marks(scene: &mut Scene, assets: &Assets, textured: bool, camera: &Camera, step_dt: f32) {
    scene.effects.update(step_dt);
    unsafe {
        if textured && !scene.decals.is_empty() {
            glEnable(GL_TEXTURE_2D);
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            glEnable(GL_POLYGON_OFFSET_FILL); // Keeps the hole from flickering against the wall
            glPolygonOffset(-2.0, -2.0);
            glDepthMask(0);
            glColor4f(1.0, 1.0, 1.0, 1.0);
            for d in &scene.decals {
                let Some(&id) = assets.hole_tex.get(&(d.surface, d.variant)) else { continue };
                glBindTexture(GL_TEXTURE_2D, id);
                glBegin(GL_QUADS);
                // The top row of the picture is its top edge
                for (c, uv) in d.corners().iter().zip([[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]) {
                    glTexCoord2f(uv[0], uv[1]);
                    glVertex3f(c[0], c[1], c[2]);
                }
                glEnd();
            }
            glDepthMask(1);
            glDisable(GL_POLYGON_OFFSET_FILL);
            glDisable(GL_BLEND);
        }
        if !scene.effects.particles.is_empty() || !scene.effects.puffs.is_empty() {
            let right = listener_of(camera).right;
            glEnable(GL_BLEND);
            glDepthMask(0);
            // Soft clouds and the flash of a spark, as sprites facing the camera
            for puff in &scene.effects.puffs {
                let (tex, additive, strength) = match puff.kind {
                    impact::PuffKind::Dust => (assets.cloud_tex, false, 0.55),
                    impact::PuffKind::Flash => (assets.flash_tex, true, 1.0),
                    impact::PuffKind::Glare => (assets.glare_tex, true, 1.0),
                    impact::PuffKind::Smoke => (assets.smoke_tex, false, 0.6),
                    impact::PuffKind::Ring => (assets.ring_tex, false, 0.9),
                };
                let Some(id) = tex else { continue };
                glEnable(GL_TEXTURE_2D);
                glBindTexture(GL_TEXTURE_2D, id);
                glBlendFunc(GL_SRC_ALPHA, if additive { GL_ONE } else { GL_ONE_MINUS_SRC_ALPHA });
                // A blast fades quickly at first, then lingers; the rest fade steadily
                let fade = if puff.kind == impact::PuffKind::Smoke { puff.alpha().sqrt() * puff.alpha() } else { puff.alpha() };
                glColor4f(puff.color[0], puff.color[1], puff.color[2], fade * strength);
                let h = puff.current_size() / 2.0;
                // Facing the camera, or lying in the surface for a shockwave ring
                let (r, u) = match puff.axis {
                    Some(n) => {
                        let across = if n[1].abs() > 0.95 { [1.0, 0.0, 0.0] } else { unit3([n[2], 0.0, -n[0]]) };
                        (across, [n[1] * across[2] - n[2] * across[1], n[2] * across[0] - n[0] * across[2], n[0] * across[1] - n[1] * across[0]])
                    }
                    None => (right, [0.0, 1.0, 0.0]),
                };
                // Dust hugs the ground as a low bank and smoke sits on it as a ball, so
                // neither is cut off by the floor where the flat sprite meets it
                let (squash, lift) = match (puff.kind, puff.axis) {
                    (_, Some(_)) => (1.0, 0.0),
                    (impact::PuffKind::Dust, _) => (0.55, 0.55),
                    (impact::PuffKind::Smoke, _) => (1.0, 0.85),
                    _ => (1.0, 0.0),
                };
                let p = [puff.pos[0], puff.pos[1] + lift * h, puff.pos[2]];
                glBegin(GL_QUADS);
                for (dx, dy, tu, tv) in [(-1.0, -1.0, 0.0, 1.0), (1.0, -1.0, 1.0, 1.0), (1.0, 1.0, 1.0, 0.0), (-1.0, 1.0, 0.0, 0.0)] {
                    glTexCoord2f(tu, tv);
                    let dy = dy * squash;
                    glVertex3f(p[0] + r[0] * dx * h + u[0] * dy * h, p[1] + r[1] * dx * h + u[1] * dy * h, p[2] + r[2] * dx * h + u[2] * dy * h);
                }
                glEnd();
            }
            // Splinters, then sparks that glow over everything, then chips
            glDisable(GL_TEXTURE_2D);
            for sparks in [false, true] {
                glBlendFunc(GL_SRC_ALPHA, if sparks { GL_ONE } else { GL_ONE_MINUS_SRC_ALPHA });
                glLineWidth(if sparks { 1.5 } else { 2.0 });
                glBegin(GL_LINES);
                for pt in scene.effects.particles.iter().filter(|p| matches!(p.kind, impact::Kind::Spark | impact::Kind::Flare) == sparks && p.kind != impact::Kind::Chip && p.kind != impact::Kind::Ember) {
                    let (a, b) = (pt.tail(), pt.pos);
                    // The line a fragment crossed since the last frame fades behind it
                    let tail = if pt.kind == impact::Kind::Flare { 0.12 } else { 1.0 };
                    glColor4f(pt.color[0], pt.color[1], pt.color[2], pt.alpha() * tail);
                    glVertex3f(a[0], a[1], a[2]);
                    glColor4f(pt.color[0], pt.color[1], pt.color[2], pt.alpha());
                    glVertex3f(b[0], b[1], b[2]);
                }
                glEnd();
                if sparks {
                    glPointSize(3.0);
                    glBegin(GL_POINTS);
                    for pt in scene.effects.particles.iter().filter(|p| p.kind == impact::Kind::Flare) {
                        glColor4f(pt.color[0], pt.color[1], pt.color[2], pt.alpha());
                        glVertex3f(pt.pos[0], pt.pos[1], pt.pos[2]);
                    }
                    glEnd();
                    // Embers are small things in the world, not a number of pixels: a couple
                    // of units across, but never less than a pixel or so
                    let eye = listener_of(camera).pos; // in the map's space, like the embers
                    glBegin(GL_QUADS);
                    for pt in scene.effects.particles.iter().filter(|p| p.kind == impact::Kind::Ember) {
                        let away = ((pt.pos[0] - eye[0]).powi(2) + (pt.pos[1] - eye[1]).powi(2) + (pt.pos[2] - eye[2]).powi(2)).sqrt();
                        let h = EMBER_SIZE.max(away * ONE_PIXEL_PER_UNIT) / 2.0;
                        glColor4f(pt.color[0], pt.color[1], pt.color[2], pt.alpha());
                        for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                            glVertex3f(pt.pos[0] + right[0] * dx * h, pt.pos[1] + dy * h, pt.pos[2] + right[2] * dx * h);
                        }
                    }
                    glEnd();
                }
            }
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            glPointSize(2.5);
            glBegin(GL_POINTS);
            for pt in scene.effects.particles.iter().filter(|p| p.kind == impact::Kind::Chip) {
                glColor4f(pt.color[0], pt.color[1], pt.color[2], pt.alpha());
                glVertex3f(pt.pos[0], pt.pos[1], pt.pos[2]);
            }
            glEnd();
            glLineWidth(1.0);
            glDepthMask(1);
            glDisable(GL_BLEND);
            glColor4f(1.0, 1.0, 1.0, 1.0);
        }
    }
}

// What drawing the glass reads
pub struct GlassPass<'a> {
    pub sim: &'a sim::Sim,
    pub level: &'a map::Map,
    pub session: &'a Session,
    pub drawn: &'a WorldDrawn,
    pub textured: bool,
    pub step_dt: f32,
}

/// Glass: intact panes, then the pieces of broken ones falling away
pub fn draw_panes(pass: GlassPass, fractures: &mut Vec<glass::Fracture>, textures: &mut HashMap<String, Option<u32>>) {
    let GlassPass { sim, level, session, drawn, textured, step_dt } = pass;
    for f in fractures.iter_mut() {
        f.update(step_dt, &|x, z, y| sim.world.ground_at(x, z, y + 30.0));
    }
    fractures.retain(|f| !f.done());
    // A pane shows only with its room. Some maps glaze a window on the inside alone and
    // give the outside a solid one: from outside, the room and its glass are not drawn.
    let pane_shows = |pane: &glass::Pane| drawn.plan_empty
        || map::object_room(&pane.name).is_none_or(|n| drawn.visible_rooms.contains(&n));
    for (pane, _) in sim.panes.iter().zip(&sim.broken_at).filter(|(p, b)| b.is_none() && pane_shows(p)) {
        draw_glass(&level.materials, pane.material, &session.map_num, textures, textured, pane.opacity, pane.tris.iter().map(|t| (t.p, t.uv)));
    }
    for f in fractures.iter() {
        draw_fracture(&level.materials, &session.map_num, textures, textured, f);
    }
}

// What the casings' ejecting, falling and drawing read
pub struct CasingPass<'a> {
    pub sim: &'a sim::Sim,
    pub assets: &'a Assets,
    pub level: &'a map::Map,
    pub toggles: &'a Toggles,
    pub drawn: &'a WorldDrawn,
    pub camera: &'a Camera,
    pub view_to_map: Option<casings::Mat4>,
    pub me: usize,
    pub game_time: f32,
    pub step_dt: f32,
    // The lobby is up over the game
    pub gate_showing: bool,
}

/// Ejects the casings of the guns drawn this frame, lets them fall, and draws them
pub fn draw_casings(pass: CasingPass, scene: &mut Scene, audio: &mut Audio, out: &mut FrameOutput) {
    let CasingPass { sim, assets, level, toggles, drawn, camera, view_to_map, me, game_time, step_dt, gate_showing } = pass;
    if gate_showing {
        return;
    }
    let spots = std::mem::take(&mut out.casing_spots);
    if let Some(to_map) = &view_to_map {
        for (owner, ejector, gun) in spots {
            let rounds = scene.casing_pending.get(&owner).copied().unwrap_or(0);
            for _ in 0..rounds.min(MOST_CASINGS_A_FRAME) {
                scene.casings.eject(&gun, to_map, &ejector, owner, 1.0);
            }
        }
    }
    for landing in scene.casings.update(step_dt, &sim.world) {
        let weapon = if landing.owner >= crate::scene::NPC_OWNERS {
            sim.npcs.get(landing.owner - crate::scene::NPC_OWNERS).and_then(|n| n.weapon).and_then(|w| sim.characters.as_ref()?.weapons.get(w)).map(|(_, w)| w)
        } else {
            assets.weapon_def.as_ref()
        };
        let last = audio.shell_sound_at.entry(landing.owner).or_insert(f32::NEG_INFINITY);
        if let (Some(name), Some(snd)) = (weapon.and_then(|w| w.shell_sound.as_deref()), audio.sound.as_mut()) {
            if game_time - *last >= SHELL_SOUND_GAP {
                *last = game_time;
                snd.play_at(name, landing.at, listener_of(camera), SHELL_SOUND_GAIN);
            }
        }
    }
    unsafe {
        glDisable(GL_TEXTURE_2D);
        glDisable(GL_BLEND);
        glDisable(GL_CULL_FACE);
        glBegin(GL_QUADS);
        // In first person our own casings are only heard, not seen
        let own_hidden = toggles.first_person && !toggles.third_person;
        for c in scene.casings.iter().filter(|c| drawn.shows(sim, level, c.pos) && !(own_hidden && c.owner == me)) {
            for (corners, normal) in c.faces() {
                // Brass, darker on faces that look down
                let shade = 0.55 + 0.45 * (normal[1] * 0.5 + 0.5);
                glColor3f(0.80 * shade, 0.62 * shade, 0.25 * shade);
                for p in corners {
                    glVertex3f(p[0], p[1], p[2]);
                }
            }
        }
        glEnd();
        glColor4f(1.0, 1.0, 1.0, 1.0);
    }
}

/// The muzzle flashes of the guns drawn this frame
pub fn draw_flashes(assets: &Assets, textured: bool, out: &mut FrameOutput) {
    if let Some(flash) = assets.flash_draw.as_ref().filter(|_| textured) {
        for spot in out.flash_spots.iter() {
            draw_flash(flash, spot);
        }
    }
    out.flash_spots.clear();
    unsafe { glColor4f(1.0, 1.0, 1.0, 1.0) };
}

// What the debug outlines read
pub struct OutlinePass<'a> {
    pub level: &'a map::Map,
    pub sim: &'a sim::Sim,
    pub frame: Option<&'a rogue_reborn::net::client::Frame>,
    pub me: usize,
    pub draw_feet: [f32; 3],
}

/// The portals, what bullets hit and the collision hints, as lines, when asked for
pub fn draw_outlines(pass: OutlinePass, toggles: &Toggles) {
    let OutlinePass { level, sim, frame: view, me, draw_feet } = pass;
    unsafe {
        // Render portals overlay when toggled
        if toggles.show_portals {
            glDisable(GL_TEXTURE_2D);
            glColor3f(0.0, 1.0, 0.0);
            for portal in &level.portals.portals {
                if portal.coordinates.len() >= 2 {
                    glBegin(GL_LINE_LOOP);
                    for v in &portal.coordinates {
                        glVertex3f(v.x, v.y, v.z);
                    }
                    glEnd();
                }
            }
            glColor3f(1.0, 1.0, 1.0); // restore default for next pass
        }

        // What bullets hit of each player, as yellow outlines: the player's own only when
        // their body is seen, in third person or flying free. With the players' model, that
        // is the posed body, outlined where it is drawn; without, these cylinders.
        if toggles.show_hitboxes && sim.characters.as_ref().is_none_or(|c| c.rig.is_none()) {
            glDisable(GL_TEXTURE_2D);
            glColor3f(1.0, 0.85, 0.0);
            let others = view.map_or(&[][..], |v| &v.players[..]);
            let mut standing: Vec<(usize, [f32; 3])> = others.iter().filter(|s| s.slot != me && s.dead.is_none()).map(|s| (s.slot, s.feet)).collect();
            if sim.players[me].respawn_at.is_none() && (toggles.third_person || !toggles.first_person) {
                standing.push((me, draw_feet));
            }
            const SIDES: usize = 24;
            let ring = |c: [f32; 3], y: f32| {
                glBegin(GL_LINE_LOOP);
                for k in 0..SIDES {
                    let a = k as f32 / SIDES as f32 * std::f32::consts::TAU;
                    glVertex3f(c[0] + a.cos() * sim::HIT_RADIUS, y, c[2] + a.sin() * sim::HIT_RADIUS);
                }
                glEnd();
            };
            for (slot, feet) in standing {
                let p = &sim.players[slot];
                for part in sim.hit_parts(feet, p.player.crouched, sim.peek_offset(slot)) {
                    let b = part.bottom;
                    ring(b, b[1]);
                    ring(b, part.top);
                    glBegin(GL_LINES);
                    for k in 0..4 {
                        let a = k as f32 * std::f32::consts::FRAC_PI_2;
                        let (x, z) = (b[0] + a.cos() * sim::HIT_RADIUS, b[2] + a.sin() * sim::HIT_RADIUS);
                        glVertex3f(x, b[1], z);
                        glVertex3f(x, part.top, z);
                    }
                    glEnd();
                }
            }
            glColor3f(1.0, 1.0, 1.0);
        }

        // Brushes and some collision hints that augment level geometry
        if toggles.show_collisions {
            glDisable(GL_TEXTURE_2D);
            glColor3f(1.0, 0.0, 0.0);
            for object in &level.geometries.objects {
                glPointSize(7.5);
                glBegin(GL_POINTS);
                for v in &object.collisions.vertices {
                    glVertex3f(v.x, v.y, v.z)
                }
                glEnd();
            }
            glColor3f(1.0, 1.0, 1.0); // restore default for next pass
        }
    }
}
