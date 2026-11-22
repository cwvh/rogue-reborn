mod args;
mod audio;
mod camera;
mod capture;
mod debug;
mod events;
mod frame;
mod hud;
mod input;
mod opengl;
mod overlay;
mod player;
mod render;
mod scene;
mod session;
mod state;
mod textures;
mod ui_gl;
mod view;

use std::collections::HashMap;
use std::time::Instant;
use glfw::{Context, CursorMode};
use rogue_reborn::motion::MOTION_FPS;
use rogue_reborn::{map, grenade, sim, glass, casings};
use rogue_reborn::net::client::Status;
use rogue_reborn::sim::movement::{Movement, animation_pace, hip_turn, movement_from_keys};
use rogue_reborn::impact;
use crate::opengl::*;
use crate::args::parse_args;
use crate::camera::listener_of;
use rogue_reborn::util::unit3;
use crate::render::characters::{CharacterPass, FrameOutput, draw_characters, standing_light};
use crate::render::level::LevelDraw;
use crate::render::world::{WorldPass, draw_world};
use crate::render::window;
use crate::audio::Audio;
use crate::capture::{CapturePass, capture_frame};
use crate::input::{InputPass, poll_window, script_keys};
use crate::frame::{FrameTicks, TickPass, run_ticks};
use crate::events::{EventPass, handle_events};
use crate::overlay::{HudPass, Overlay};
use crate::scene::{NPC_OWNERS, Scene};
use crate::session::{Game, Session};
use crate::view::{PlayerView, ViewPass, Watched};
use crate::state::{Pressed, Timing, Toggles};
use crate::render::draw::{draw_weapon, FlashSpot, draw_flash, draw_flash_here, draw_glass, draw_fracture, level_radius};

// The first-person view's height in degrees. The width follows the screen shape.
const VIEW_HIGH: f32 = 90.0;

fn half_view_tan() -> f32 {
    (VIEW_HIGH.to_radians() / 2.0).tan()
}

const MOST_CASINGS_A_FRAME: u32 = 4;
const SHELL_SOUND_GAP: f32 = 0.2;
const SHELL_SOUND_GAIN: f32 = 0.5;


/// The ways to draw the level's soft sprites (H cycles them, `--sprite-mode` starts one), for
/// finding why a shadow flickers: whether it writes depth, and whether it is pulled toward the
/// camera so that it wins against the surface it lies on. A shadow a fraction of a unit off its
/// wall flickered with no offset, so the first mode, which has one, is the default.
pub const SPRITE_MODES: [&str; 3] = ["writes depth, offset toward the camera", "writes depth", "no depth writes"];

fn main() {
    let mut args = parse_args();
    let Some((mut session, world)) = Session::start(&mut args) else { return };
    let level = session.level.clone();
    let level_radius = level_radius(&level);

    let (mut glfw, mut window, events) = window::open(&args);

    let Game { camera, assets, mut sim, me, npc_actor, eye_height, crouch_eye_height } = session.start_game(&args, world);
    let mut scene = Scene::default();
    // A blast's embers: how wide they are, and the width of a pixel for each unit of distance
    // (a 60 degree view about a thousand pixels high)
    const EMBER_SIZE: f32 = 2.0;
    const ONE_PIXEL_PER_UNIT: f32 = 0.0012;
    let mut pview = PlayerView::new(camera, eye_height, sim.players[me].player.pos);
    window.set_cursor_mode(CursorMode::Disabled);
    // The lobby between rounds (see lobby.rs), drawn over everything with the original's art.
    // While it shows, the keys and mouse are its, and the pointer is free.
    let mut overlay = Overlay::new(session.joined.as_ref());
    let mut textures = HashMap::new();
    let mut toggles = Toggles::new(&args);
    let last_step = Instant::now();
    let level_draw = LevelDraw::new(&level, &sim, &session.map_num);
    let mut pressed = Pressed::default();
    let mut audio = Audio::new(&args, &session.map_path);
    // A scripted run, or a replay, takes one tick a frame instead of keeping time
    let mut timing = Timing::new(args.walk.is_some() || (session.playback.is_some() && args.screenshot.is_some()), last_step);
    let team_game = session.joined.as_ref().is_some_and(|s| s.bases.iter().all(|b| !b.is_empty()));
    let player_ejector = assets.weapon_def.as_ref().and_then(casings::Ejector::of);
    let mut out = FrameOutput::new();
    let base_speed = 7.5;

    while !window.should_close() {
        let input_pass = InputPass { args: &args, level: &level, base_speed };
        poll_window(input_pass, &mut glfw, &events, &mut window, &mut toggles, &mut overlay, &mut pview, &mut pressed, &mut scene, &mut session);

        script_keys(&args, &sim, me, &timing, &mut overlay, &mut pview, &mut pressed);

        let ticked = run_ticks(TickPass { me, base_speed }, &mut sim, &mut session, &mut toggles, &mut timing, &overlay, &mut pview, &mut pressed);
        let FrameTicks { raw_dt, step_dt, events, shown } = ticked;
        let game_time = sim.time;
        // On a server, what moves is shown a little in the past, between its snapshots, so that
        // it moves smoothly
        let frame = session.net.as_mut().map(|(client, _)| {
            let now = session.net_clock.elapsed().as_secs_f64();
            if let Some(rec) = session.net_recorder.as_mut() {
                rec.viewed(now);
            }
            client.view(now)
        });
        if let Some((client, _)) = &session.net {
            match &client.status {
                Status::Connected => {}
                Status::SentAway(why) => {
                    eprintln!("the server ended the game for us: {why}");
                    window.set_should_close(true);
                }
                _ => {
                    eprintln!("the connection to the server was lost: {:?}", client.status);
                    window.set_should_close(true);
                }
            }
            if !session.host_checked && client.snapshots > 0 {
                session.host_checked = true;
                if client.session.host != Some(me as u8) {
                    eprintln!("the server on that port is not the one we started: someone else is its host");
                    window.set_should_close(true);
                }
            }
        }

        let view_pass = ViewPass { sim: &sim, me, game_time, step_dt, eye_height, crouch_eye_height, team_game };
        let draw_feet = pview.eyes(&view_pass, &toggles, &timing);

        overlay.feed.retain(|(until, ..)| *until > game_time);
        // A new round leaves nothing of the last
        if let Some((client, _)) = &session.net {
            if client.round.number != session.round_seen {
                session.round_seen = client.round.number;
                scene.decals.clear();
                scene.corpses.clear();
                overlay.feed.clear();
            }
        }

        let watched = pview.dead_and_watching(&view_pass, &session, &toggles, &frame);
        let Watched { watch_pose, watched_body, camera_feet } = watched;

        pview.fade_blast(&mut audio, step_dt);

        // What happened in the ticks: sounds to play, effects to draw, lines for the log
        let pass = EventPass { args: &args, assets: &assets, me, game_time, watch_pose, player_ejector: player_ejector.as_ref() };
        handle_events(events, pass, &mut sim, &mut audio, &mut pview, &mut overlay, &mut scene);

        if toggles.first_person {
            // The animation follows what the player is trying to do, and plays at the pace
            // they actually move (a wall stops it)
            let dt = step_dt;
            let wanted = movement_from_keys(shown.forward, shown.back, shown.left, shown.right, shown.run);
            let moved = sim.players[me].moved_speed;
            // A wall stops the walking animation; standing still can fire or reload
            let walking = wanted != Movement::Idle && moved >= 5.0;
            let now = if walking {
                wanted
            } else if sim.players[me].throw.is_some() {
                Movement::Throw
            } else if sim.players[me].reload_until.is_some() {
                Movement::Reload
            } else if sim.players[me].gun_trigger && sim.players[me].ammo > 0 {
                Movement::Fire
            } else {
                Movement::Idle
            };
            if now != pview.movement {
                pview.movement = now;
                pview.player_frame = 0.0;
            }
            pview.player_hip_turn = hip_turn(shown.forward, shown.back, shown.left, shown.right);
            let crouched = sim.players[me].player.crouched;
            let limp = sim.players[me].leg_wounds > 0;
            let pace = animation_pace(pview.movement, crouched, limp);
            let rate = if matches!(pview.movement, Movement::Idle | Movement::Fire | Movement::Reload | Movement::Throw) { 1.0 } else { (moved / (pace * args.speed_scale.unwrap_or(1.0))).clamp(0.5, 3.0) * args.anim_scale.unwrap_or(1.0) };
            let frames = assets.player_motions.get(pview.movement, crouched, limp).map_or(1, |m| m.frames.max(1)) as f32;
            // A reload's frame follows its progress, and is picked where the body is drawn
            pview.player_frame = if let (Movement::Throw, Some(th)) = (pview.movement, sim.players[me].throw) {
                ((game_time - th.start) / sim::THROW_SECONDS).clamp(0.0, 1.0) * (frames - 1.0)
            } else {
                (pview.player_frame + rate * MOTION_FPS * dt) % frames
            };
            // A footstep at the start of each half of a walk or run cycle
            let phase = (pview.player_frame / (frames / 2.0).max(1.0)) as i32;
            if phase != audio.step_phase {
                audio.step_phase = phase;
                if walking && sim.players[me].player.on_ground {
                    if let Some(snd) = audio.sound.as_mut() {
                        snd.play(if matches!(pview.movement, Movement::Run | Movement::RunBack) { "F_HRCON1.WAV" } else { "F_HWCON1.WAV" }, 1.0);
                    }
                }
            }
        }

        // Reload, firing and ambient sound
        audio.play_frame(&sim, me, &level, &pview, overlay.gate.showing(), game_time, step_dt);

        let current_rooms: Vec<&str> = sim.players[me].current_room.map(|i| level.rooms.rooms[i].section_name.as_str()).into_iter().collect();
        if toggles.room_debug && current_rooms != toggles.last_rooms {
            println!("in rooms: {current_rooms:?}");
            toggles.last_rooms = current_rooms.iter().map(|s| s.to_string()).collect();
        }

        // Compute dynamic aspect and clip planes
        let (w, h) = window.get_size();
        let aspect = (w as f32 / h.max(1) as f32).max(0.1);
        // The near plane stays well inside the player's 30 units from a wall, and inside the
        // distance at which a doorway counts as one the player stands in: past it, the wall
        // is cut open, or the doorway's opening is cut and the room beyond goes missing
        const MAX_Z_NEAR: f32 = 4.0;
        let z_near = (level_radius / 1000.0).clamp(0.05, MAX_Z_NEAR);
        let z_far = (level_radius * 10.0 + 100.0).max(z_near + 1.0);

        // Frame timing for FPS
        let fps = if raw_dt > 0.0 { 1.0 / raw_dt } else { 0.0 };

        // The sim sets a player's flash_until to 0.05 s after the shot.
        // It is 0 before the first shot.
        let shot_at = |until: f32| if until > 0.0 { until - 0.05 } else { f32::NEG_INFINITY };
        let player_flash: Vec<bool> = sim.players.iter().enumerate().map(|(slot, p)| scene.player_flashes.shows(slot, shot_at(p.flash_until), game_time) && !p.equipped_grenade).collect();
        let npc_flash: Vec<bool> = sim.npcs.iter().enumerate().map(|(i, n)| scene.npc_flashes.shows(i, n.last_shot, game_time)).collect();

        let render_time = Instant::now();
        let world = WorldPass { args: &args, level: &level, sim: &sim, session: &session, level_draw: &level_draw, me, game_time, aspect, z_near, z_far, watch_pose, camera_feet, current_rooms: &current_rooms };
        let drawn = draw_world(world, &toggles, &mut pview, &mut textures);
        unsafe {
            let characters = CharacterPass { args: &args, assets: &assets, level: &level, sim: &sim, session: &session, toggles: &toggles, timing: &timing, drawn: &drawn, frame: frame.as_ref(), me, game_time, step_dt, npc_actor: &npc_actor, player_flash: &player_flash, npc_flash: &npc_flash, watched_body, draw_feet, player_ejector: player_ejector.as_ref() };
            draw_characters(&characters, &pview, &mut scene, &mut out);
            let shown = |feet: [f32; 3]| drawn.shows(&sim, &level, feet);

            // The light of a blast, over what it reaches
            for l in &mut scene.blast_lights {
                l.update(step_dt);
            }
            scene.blast_lights.retain(|l| !l.done());
            if let (true, Some(id)) = (toggles.textured && !scene.blast_lights.is_empty(), assets.light_tex) {
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
                for l in &scene.blast_lights {
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

            // Bullet holes on the walls, then the debris thrown from them
            scene.effects.update(step_dt);
            if toggles.textured && !scene.decals.is_empty() {
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
                let right = listener_of(&pview.camera).right;
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
                        let eye = listener_of(&pview.camera).pos; // in the map's space, like the embers
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

            // Glass: intact panes, then the pieces of broken ones falling away
            for f in scene.fractures.iter_mut() {
                f.update(step_dt, &|x, z, y| sim.world.ground_at(x, z, y + 30.0));
            }
            scene.fractures.retain(|f| !f.done());
            // A pane shows only with its room. Some maps glaze a window on the inside alone and
            // give the outside a solid one: from outside, the room and its glass are not drawn.
            let pane_shows = |pane: &glass::Pane| drawn.plan_empty
                || map::object_room(&pane.name).is_none_or(|n| drawn.visible_rooms.contains(&n));
            for (pane, _) in sim.panes.iter().zip(&sim.broken_at).filter(|(p, b)| b.is_none() && pane_shows(p)) {
                draw_glass(&level.materials, pane.material, &session.map_num, &mut textures, toggles.textured, pane.opacity, pane.tris.iter().map(|t| (t.p, t.uv)));
            }
            for f in &scene.fractures {
                draw_fracture(&level.materials, &session.map_num, &mut textures, toggles.textured, f);
            }

            if !overlay.gate.showing() {
                let spots = std::mem::take(&mut out.casing_spots);
                if let Some(to_map) = &drawn.view_to_map {
                    for (owner, ejector, gun) in spots {
                        let rounds = scene.casing_pending.get(&owner).copied().unwrap_or(0);
                        for _ in 0..rounds.min(MOST_CASINGS_A_FRAME) {
                            scene.casings.eject(&gun, to_map, &ejector, owner, 1.0);
                        }
                    }
                }
                for landing in scene.casings.update(step_dt, &sim.world) {
                    let weapon = if landing.owner >= NPC_OWNERS {
                        sim.npcs.get(landing.owner - NPC_OWNERS).and_then(|n| n.weapon).and_then(|w| sim.characters.as_ref()?.weapons.get(w)).map(|(_, w)| w)
                    } else {
                        assets.weapon_def.as_ref()
                    };
                    let last = audio.shell_sound_at.entry(landing.owner).or_insert(f32::NEG_INFINITY);
                    if let (Some(name), Some(snd)) = (weapon.and_then(|w| w.shell_sound.as_deref()), audio.sound.as_mut()) {
                        if game_time - *last >= SHELL_SOUND_GAP {
                            *last = game_time;
                            snd.play_at(name, landing.at, listener_of(&pview.camera), SHELL_SOUND_GAIN);
                        }
                    }
                }
                glDisable(GL_TEXTURE_2D);
                glDisable(GL_BLEND);
                glDisable(GL_CULL_FACE);
                glBegin(GL_QUADS);
                // In first person our own casings are only heard, not seen
                let own_hidden = toggles.first_person && !toggles.third_person;
                for c in scene.casings.iter().filter(|c| shown(c.pos) && !(own_hidden && c.owner == me)) {
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

            if let Some(flash) = assets.flash_draw.as_ref().filter(|_| toggles.textured) {
                for spot in out.flash_spots.iter() {
                    draw_flash(flash, spot);
                }
            }
            out.flash_spots.clear();
            glColor4f(1.0, 1.0, 1.0, 1.0);

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
                let others = frame.as_ref().map_or(&[][..], |v| &v.players[..]);
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

        // The held weapon in front of the camera, over the scene. A grenade is small, so it is held
        // further out in proportion, drawn back by the wind-up, and gone once it has been thrown.
        let held = if sim.players[me].equipped_grenade { assets.grenade_draw.as_ref().filter(|_| sim.players[me].throw.is_none_or(|t| !t.thrown)) } else { assets.weapon_draw.as_ref() };
        // Without the view model (the default) the gun is not drawn.
        // Its place still says where the casings leave, and only a firing gun needs it.
        let casing_rounds = scene.casing_pending.get(&me).copied().unwrap_or(0);
        let gun_shows = args.viewmodel;
        if let (true, true, false, Some(wd)) = ((gun_shows || (casing_rounds > 0 && !sim.players[me].equipped_grenade)) && sim.players[me].respawn_at.is_none(), toggles.first_person, toggles.third_person, held) {
            let l = wd.length;
            let [ox, oy, oz] = if sim.players[me].equipped_grenade {
                let held_for = sim.players[me].charge_from.map_or(0.0, |from| ((game_time - from) / grenade::CHARGE_SECONDS).clamp(0.0, 1.0));
                let swing = sim.players[me].throw.map_or(0.0, |t| t.charge.max(((game_time - t.start) / sim::THROW_SECONDS / sim::THROW_RELEASE).clamp(0.0, 1.0)));
                let wind = held_for.max(swing);
                [1.7 + 0.9 * wind, -1.2 - 0.6 * wind, 8.0 - 1.8 * wind]
            } else {
                args.viewmodel_offset.unwrap_or([0.45, -0.45, 1.5])
            };
            // Recoil pushes the gun back at the player; a reload dips it out of view
            let kick = ((sim.players[me].flash_until - game_time) / 0.05).clamp(0.0, 1.0) * 0.04 * l;
            let dip = sim.players[me].reload_until.map_or(0.0, |t| {
                let p = (1.0 - (t - game_time) / sim::RELOAD_SECONDS).clamp(0.0, 1.0);
                (p * std::f32::consts::PI).sin() * 0.8 * l
            });
            unsafe {
                if gun_shows {
                    // Its own depth range, so the gun never sinks into a wall
                    glClear(GL_DEPTH_BUFFER_BIT);
                    glMatrixMode(GL_PROJECTION);
                    glLoadIdentity();
                    glu_perspective(45.0, aspect, l * 0.05, l * 20.0); // narrower than the world, as in most shooters
                } else {
                    glMatrixMode(GL_MODELVIEW);
                    glPushMatrix();
                }
                glMatrixMode(GL_MODELVIEW);
                glLoadIdentity();
                glTranslatef(ox * l, oy * l - dip, -oz * l + kick);
                // Angle the barrel slightly in toward the middle of the view
                glRotatef(20.0, 0.0, 1.0, 0.0);
                // Model space (forward -Y, up +Z) to view space (forward -Z, up +Y)
                glRotatef(180.0, 0.0, 1.0, 0.0);
                glRotatef(-90.0, 1.0, 0.0, 0.0);
                glTranslatef(-wd.center[0], -wd.center[1], -wd.center[2]);
                if gun_shows {
                    // Lit by the room the player stands in
                    let light = session.lighting.as_ref().map_or([1.0; 3], |l| {
                        let lit = standing_light(l, &level, &sim.visibility, sim.players[me].player.pos);
                        if player_flash[me] && wd.flash_scale > 0.0 { lit.flash_tint() } else { lit.tint }
                    });
                    draw_weapon(wd, toggles.textured, light);
                    if let (true, Some(flash)) = (player_flash[me], assets.flash_draw.as_ref().filter(|_| toggles.textured)) {
                        draw_flash_here(flash, &FlashSpot::here(wd, out.flash_roll()));
                    }
                }
                // The casings leave the gun as it is placed here.
                // Its view is narrower than the world's, so they start further from the middle.
                if let (true, false, Some(e), Some(to_map)) = (casing_rounds > 0, overlay.gate.showing(), player_ejector.as_ref().filter(|_| !sim.players[me].equipped_grenade), &drawn.view_to_map) {
                    let mut gun = [0f32; 16];
                    glGetFloatv(GL_MODELVIEW_MATRIX, gun.as_mut_ptr());
                    let widen = half_view_tan() / 22.5f32.to_radians().tan();
                    for _ in 0..casing_rounds.min(MOST_CASINGS_A_FRAME) {
                        scene.casings.eject(&gun, to_map, e, me, widen);
                    }
                }
                if !gun_shows {
                    glPopMatrix();
                }
                glColor4f(1.0, 1.0, 1.0, 1.0);
            }
        }
        // Rounds fired by guns not drawn this frame leave no casings
        scene.casing_pending.clear();
        out.casing_spots.clear();

        // Debug text and the HUD
        let hud = HudPass { args: &args, level: &level, sim: &sim, me, game_time, w, h, fps, render_time, watch_pose, current_rooms: &current_rooms, drawn_rooms: &drawn.drawn_rooms };
        let render_ms = overlay.draw_hud_and_debug(hud, &toggles, &timing, &pview, &session);

        // The lobby, the results and the chat, over the game
        overlay.draw_lobby(&mut session, &sim, me, game_time, &window);

        // F2: a screenshot and a text log of where the player is and what they see, the
        // screenshot mode's capture, and the benchmark's timing
        let capture = CapturePass { args: &args, level: &level, sim: &sim, me, fps, render_ms, raw_dt, current_rooms: &current_rooms, drawn_rooms: &drawn.drawn_rooms };
        if capture_frame(capture, &mut toggles, &mut timing, &mut overlay, &scene, &pview, &session, &mut window) {
            break;
        }

        window.swap_buffers();
        if let Some(max) = args.max_fps {
            // Sleep most of the remaining frame time, then spin for accuracy
            let target = std::time::Duration::from_secs_f32(1.0 / max);
            loop {
                let left = target.saturating_sub(timing.last_step.elapsed());
                if left.is_zero() {
                    break;
                }
                if left > std::time::Duration::from_millis(2) {
                    std::thread::sleep(left - std::time::Duration::from_millis(1));
                } else {
                    std::hint::spin_loop();
                }
            }
        }
    }

    // Let go of the window and the sound device first when restarting, so the new one has them to itself
    let release = if toggles.reload { Some((window, audio.sound)) } else { None };
    session.finish(&args, &sim, release);
}
