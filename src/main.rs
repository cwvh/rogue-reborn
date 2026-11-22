mod args;
mod audio;
mod camera;
mod capture;
mod debug;
mod hud;
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
use glfw::{Action, Context, Key, WindowEvent, CursorMode};
use rogue_reborn::motion::MOTION_FPS;
use rogue_reborn::{map, combat, crp, grenade, blastlight, sim, sound, glass, casings};
use rogue_reborn::net::client::{Seen, Status};
use rogue_reborn::lobby;
use rogue_reborn::net::animation::Doing;
use rogue_reborn::net::watch::{BEHIND, Pose, RISE, WatchContext, WatchMode, behind};
use rogue_reborn::sim::characters::{Characters, Npc, Rig};
use rogue_reborn::sim::movement::{Movement, Reloading, animation_pace, hip_turn, movement_from_keys};
use rogue_reborn::impact::{self, Surface};
use crate::opengl::*;
use crate::args::parse_args;
use crate::camera::{Camera, listener_of, apply_camera};
use rogue_reborn::util::unit3;
use crate::player::{BodyKey, BodyState};
use rogue_reborn::body;
use rogue_reborn::lighting::{CharacterLight, Lighting};
use rogue_reborn::visibility::Visibility;
use crate::render::assets::{Uniform, SCORCH_VARIANTS, HOLE_VARIANTS};
use crate::render::level::LevelDraw;
use crate::render::window;
use crate::audio::Audio;
use crate::capture::{CLIP_SECONDS, CapturePass, capture_frame};
use crate::overlay::{HudPass, Overlay};
use crate::scene::Scene;
use crate::session::{Game, Session};
use crate::view::PlayerView;
use crate::state::{Pressed, Timing, Toggles};
use crate::render::draw::{WeaponDraw, CharacterArrays, draw_weapon, FlashSpot, draw_flash, draw_flash_here, draw_character, draw_glass, draw_fracture, level_radius, Which, draw_plan, wipe_opening, room_color};
use crate::textures::ensure_texture;

// The first-person view's height in degrees. The width follows the screen shape.
const VIEW_HIGH: f32 = 90.0;

fn half_view_tan() -> f32 {
    (VIEW_HIGH.to_radians() / 2.0).tan()
}

/// Added to a terrorist's number so its casings are not taken for a player's
const NPC_OWNERS: usize = 1000;
const MOST_CASINGS_A_FRAME: u32 = 4;
const SHELL_SOUND_GAP: f32 = 0.2;
const SHELL_SOUND_GAIN: f32 = 0.5;

/// How many of the latest log lines a debug dump shows
const RECENT_LOG: usize = 60;

/// The ways to draw the level's soft sprites (H cycles them, `--sprite-mode` starts one), for
/// finding why a shadow flickers: whether it writes depth, and whether it is pulled toward the
/// camera so that it wins against the surface it lies on. A shadow a fraction of a unit off its
/// wall flickered with no offset, so the first mode, which has one, is the default.
pub const SPRITE_MODES: [&str; 3] = ["writes depth, offset toward the camera", "writes depth", "no depth writes"];

/// The light of the room at `room_at`, weighed at `weigh_at`, in the level's space
fn room_light(lighting: &Lighting, level: &map::Map, visibility: &Visibility, room_at: [f32; 3], weigh_at: [f32; 3]) -> CharacterLight {
    let room = level.rooms.locate(room_at, None).and_then(|i| visibility.number_of(i));
    lighting.character(room, weigh_at)
}

/// The light on a character standing at `feet`: its room is found just above the floor, and
/// the light is weighed at the chest
fn standing_light(lighting: &Lighting, level: &map::Map, visibility: &Visibility, feet: [f32; 3]) -> CharacterLight {
    room_light(lighting, level, visibility, [feet[0], feet[1] + 10.0, feet[2]], [feet[0], feet[1] + 100.0, feet[2]])
}

/// What a player on `side` is drawn with: their side's uniform, or else the players' model
fn dress<'a>(side: Option<sim::Side>, sides: &'a [Option<Uniform>], model: &'a crp::Crp, arrays: &'a CharacterArrays,
             textures: &'a [Option<u32>], skin: Option<&'a crp::Skin>) -> (&'a crp::Crp, &'a CharacterArrays, &'a [Option<u32>], Option<&'a crp::Skin>) {
    match side.and_then(|s| sides.get(s.index())).and_then(Option::as_ref) {
        Some(u) => (&u.model, &u.arrays, &u.textures, Some(&u.skin)),
        None => (model, arrays, textures, skin),
    }
}

fn main() {
    let mut args = parse_args();
    let Some((mut session, world)) = Session::start(&mut args) else { return };
    let level = session.level.clone();
    let level_radius = level_radius(&level);

    let (mut glfw, mut window, events) = window::open(&args);

    let Game { camera, assets, mut sim, me, npc_actor, eye_height, crouch_eye_height } = session.start_game(&args, world);
    let mut scene = Scene::default();
    const MAX_DECALS: usize = 300;
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
    // What the player did in the last tick, which the animation follows
    let mut shown;
    let mut audio = Audio::new(&args, &session.map_path);
    // A scripted run, or a replay, takes one tick a frame instead of keeping time
    let mut timing = Timing::new(args.walk.is_some() || (session.playback.is_some() && args.screenshot.is_some()), last_step);
    let team_game = session.joined.as_ref().is_some_and(|s| s.bases.iter().all(|b| !b.is_empty()));
    let player_ejector = assets.weapon_def.as_ref().and_then(casings::Ejector::of);
    let flash_rng = std::cell::Cell::new(0x9E37_79B9_7F4A_7C15u64);
    let flash_roll = || {
        let mut r = flash_rng.get();
        let x = rogue_reborn::util::next_rng(&mut r);
        flash_rng.set(r);
        (x >> 40) as f32 / (1u64 << 24) as f32
    };
    let base_speed = 7.5;

    while !window.should_close() {
        glfw.poll_events();

        // Between rounds the lobby shows. Whatever was held when it comes or goes is let go.
        let showing = session.net.as_ref().is_some_and(|(c, _)| c.status == Status::Connected && c.round.phase.screen());
        if overlay.gate.show(showing) {
            (pview.trigger, pview.walk_held, overlay.pointer_down, overlay.wheel) = (false, false, false, 0.0);
            (pressed.use_key, pressed.reload, pressed.gun, pressed.grenade, pressed.culling) = (false, false, false, false, false);
            // The first movement after it does not turn the view
            overlay.last_cursor_pos = None;
            window.set_cursor_mode(if showing { CursorMode::Normal } else { CursorMode::Disabled });
            scene.casings.clear();
            scene.casing_pending.borrow_mut().clear();
        }

        for (_, event) in glfw::flush_messages(&events) {
            match event {
                WindowEvent::Key(key, _, action, _) => {
                    match (key, action) {
                        // In the lobby, Escape clears the chat's line first
                        (Key::Escape, Action::Press) => {
                            if !(overlay.gate.showing() && overlay.chat.clear()) {
                                window.set_should_close(true)
                            }
                        }
                        // The lobby's keys type on the chat's line, and only the keys for
                        // dumps go through to the game
                        (k, action) if overlay.gate.showing() && !matches!(k, Key::F2 | Key::F3 | Key::F4) => {
                            match (k, action) {
                                (Key::Enter | Key::KpEnter, Action::Press) => overlay.typed.extend(overlay.chat.send()),
                                (Key::Backspace, Action::Press | Action::Repeat) => overlay.chat.erase(),
                                (_, Action::Release) => overlay.gate.release(k),
                                _ => {}
                            }
                        }
                        (Key::T, Action::Press) => {
                            toggles.textured = !toggles.textured;
                        }
                        (Key::L, Action::Press) if session.net.is_some() => {
                            toggles.lag_meter = !toggles.lag_meter;
                        }
                        (Key::M, Action::Press) if session.net_recorder.is_some() => {
                            let now = session.net_clock.elapsed().as_secs_f64();
                            session.net_recorder.as_mut().unwrap().mark(now);
                            println!("marked this moment in the recording, {now:.1} s in");
                        }
                        (Key::L, Action::Press) => {
                            if !level.planning_levels.levels.is_empty() {
                                toggles.planning_level_index = (toggles.planning_level_index + 1) % level.planning_levels.levels.len();
                            }
                        }
                        (Key::P, Action::Press) => {
                            toggles.show_portals = !toggles.show_portals;
                        }
                        (Key::H, Action::Press) => {
                            toggles.sprite_mode = (toggles.sprite_mode + 1) % SPRITE_MODES.len() as u8;
                            println!("sprite mode {}: {}", toggles.sprite_mode, SPRITE_MODES[toggles.sprite_mode as usize]);
                        }
                        (Key::F2, Action::Press) => {
                            toggles.dump_requested = true;
                        }
                        (Key::F4, Action::Press) if toggles.clip.is_none() => {
                            let dir = format!("debug/clip-{}", crate::debug::stamp());
                            println!("recording {CLIP_SECONDS} seconds to {dir}");
                            toggles.clip = Some((std::time::Instant::now(), dir, 0));
                        }
                        (Key::F3, Action::Press) => {
                            toggles.reload = true;
                            window.set_should_close(true);
                        }
                        (Key::Space, Action::Press) | (Key::Backspace, Action::Press) => {
                            pressed.use_key = true;
                        }
                        (Key::V, Action::Press) => {
                            pressed.culling = true;
                        }
                        (Key::G, Action::Press) => {
                            toggles.room_debug = !toggles.room_debug;
                            toggles.last_rooms.clear();
                        }
                        (Key::C, Action::Press) => {
                            toggles.show_collisions = !toggles.show_collisions;
                        }
                        (Key::B, Action::Press) => {
                            toggles.show_hitboxes = !toggles.show_hitboxes;
                        }
                        (Key::N, Action::Press) => {
                            toggles.blink_enabled = !toggles.blink_enabled;
                        }
                        // Dead in a round, F goes between their eyes and behind them once
                        // watching, and leaves the player's own view as it was
                        (Key::F, Action::Press) if pview.watch.in_round() => {
                            pview.watch_mode |= pview.watch.watching();
                        }
                        (Key::F, Action::Press) => {
                            toggles.third_person = !toggles.third_person;
                        }
                        (Key::Num1, Action::Press) => {
                            pressed.gun = true;
                        }
                        (Key::Num3, Action::Press) => {
                            pressed.grenade = true;
                        }
                        (Key::R, Action::Press) => {
                            pressed.reload = true;
                        }
                        (Key::LeftControl, Action::Press) => {
                            pview.crouch_wanted = !pview.crouch_wanted;
                        }
                        (Key::F1, Action::Press) if session.playback.is_none() && session.net.is_none() => {
                            // Into the player's body, who drops in from wherever the view is, and out again
                            toggles.first_person = !toggles.first_person;
                        }
                        (k, Action::Press) => {
                            overlay.gate.press(k);
                        }
                        (k, Action::Release) => {
                            overlay.gate.release(k);
                        }
                        _ => {}
                    }
                }
                WindowEvent::Char(c) if overlay.gate.showing() => overlay.chat.typed(c),
                WindowEvent::Scroll(_, y) if overlay.gate.showing() => overlay.wheel += y as f32,
                WindowEvent::CursorPos(..) if overlay.gate.showing() => {}
                WindowEvent::CursorPos(xpos, ypos) => {
                    // A scripted run (--walk) or a replay is not steered by a real mouse
                    if let (Some((last_x, last_y)), None, false) = (overlay.last_cursor_pos, args.walk, session.playback.is_some()) {
                        let dx = xpos - last_x;
                        let dy = ypos - last_y;
                        if args.cinematic && toggles.first_person && toggles.third_person && overlay.gate.held(Key::LeftShift) {
                            toggles.orbit.0 -= dx as f32 * 0.2;
                            toggles.orbit.1 = (toggles.orbit.1 - dy as f32 * 0.1).clamp(-60.0, 60.0);
                            overlay.last_cursor_pos = Some((xpos, ypos));
                            continue;
                        }
                        pview.camera.yaw -= dx as f32 * 0.1;
                        pview.camera.pitch -= dy as f32 * 0.1;
                        pview.camera.pitch = pview.camera.pitch.clamp(-89.0, 89.0);
                    }
                    overlay.last_cursor_pos = Some((xpos, ypos));
                }
                WindowEvent::FramebufferSize(width, height) => {
                    unsafe { glViewport(0, 0, width, height); }
                }
                // In the lobby, the main button clicks on it
                WindowEvent::MouseButton(glfw::MouseButton::Button1, action, _) if overlay.gate.showing() => {
                    overlay.pointer_down = action != Action::Release;
                }
                WindowEvent::MouseButton(_, _, _) if overlay.gate.showing() => {}
                // Watching, the buttons go to the next teammate and back, on a press
                WindowEvent::MouseButton(glfw::MouseButton::Button1, action, _) => {
                    pview.trigger = action != Action::Release;
                    pview.watch_next |= action == Action::Press && pview.watch.watching();
                }
                WindowEvent::MouseButton(glfw::MouseButton::Button2, action, _) => {
                    pview.walk_held = action != Action::Release;
                    pview.watch_previous |= action == Action::Press && pview.watch.watching();
                }
                WindowEvent::Scroll(_x, y) => {
                    if !toggles.first_person {
                        let scroll = y as f32;
                        if scroll != 0.0 {
                            let dir = pview.camera.get_forward();
                            let zoom_speed = base_speed * 10.0;
                            pview.camera.position[0] += dir[0] * zoom_speed * scroll;
                            pview.camera.position[1] += dir[1] * zoom_speed * scroll;
                            pview.camera.position[2] += dir[2] * zoom_speed * scroll;
                        }
                    }
                }
                _ => {}
            }
        }

        if let Some(n) = args.walk {
            // --hold picks the keys to press: W A S D, and R for walk (right mouse button)
            let held = args.hold.as_deref().unwrap_or("W").to_ascii_uppercase();
            // B fires a single round at frame 60, once the others are in view, to see one hit land
            pview.trigger = (timing.frames_drawn < n && held.contains('F')) || (held.contains('B') && (60..64).contains(&timing.frames_drawn));
            // T takes out a grenade and throws it after holding for half a second
            if held.contains('T') {
                pressed.grenade = true;
                pview.trigger = timing.frames_drawn < 30;
            }
            // U presses the use key (Space) on frame 5, to open or close the door in front
            if held.contains('U') && timing.frames_drawn == 5 {
                pressed.use_key = true;
            }
            // L starts a reload on the first frame (for testing the animation)
            if held.contains('L') && timing.frames_drawn == 0 {
                pressed.reload = true;
            }
            pview.walk_held = timing.frames_drawn < n && held.contains('R');
            // Y turns to the right, 2 degrees a frame, to see the feet step round
            if timing.frames_drawn < n && held.contains('Y') {
                pview.camera.yaw -= 2.0;
            }
            // C crouches for the whole run; K switches stance at frame 30, to see the change
            pview.crouch_wanted = held.contains('C') != (held.contains('K') && timing.frames_drawn >= 30);
            for (ch, key) in [('W', Key::W), ('A', Key::A), ('S', Key::S), ('D', Key::D)] {
                overlay.gate.set(key, timing.frames_drawn < n && held.contains(ch));
            }
            if timing.frames_drawn % 10 == 0 {
                println!("frame {frames_drawn:4} crouch_wanted={crouch_wanted} crouched={} feet=({:.1}, {:.1}, {:.1}) on_ground={} movement={movement:?} anim_frame={player_frame:.1}",
                    sim.players[me].player.crouched, sim.players[me].player.pos[0], sim.players[me].player.pos[1], sim.players[me].player.pos[2], sim.players[me].player.on_ground, frames_drawn = timing.frames_drawn, crouch_wanted = pview.crouch_wanted, movement = pview.movement, player_frame = pview.player_frame);
            }
        }

        // Time since the previous step, measured here so it covers render and vsync
        let raw_dt = timing.last_step.elapsed().as_secs_f32();
        let real_dt = raw_dt.min(0.05);
        timing.last_step = Instant::now();
        let step_dt = if timing.fixed_step { sim::TICK } else { real_dt };

        // Free-fly: WASD moves the view (hold LeftShift to move faster)
        if !toggles.first_person && session.playback.is_none() {
            let mut speed = base_speed;
            if overlay.gate.held(Key::LeftShift) {
                speed *= 5.0;
            }
            let right = pview.camera.get_right();
            let yaw_rad = pview.camera.yaw.to_radians();
            let fwd_flat = [-yaw_rad.sin(), 0.0, -yaw_rad.cos()];
            // Desired horizontal displacement for this frame, in camera space
            let mut mv = [0.0f32; 2];
            if overlay.gate.held(Key::W) {
                mv[0] += fwd_flat[0] * speed;
                mv[1] += fwd_flat[2] * speed;
            }
            if overlay.gate.held(Key::S) {
                mv[0] -= fwd_flat[0] * speed;
                mv[1] -= fwd_flat[2] * speed;
            }
            if overlay.gate.held(Key::A) {
                mv[0] -= (right[0] * speed) / 2.0;
                mv[1] -= (right[2] * speed) / 2.0;
            }
            if overlay.gate.held(Key::D) {
                mv[0] += (right[0] * speed) / 2.0;
                mv[1] += (right[2] * speed) / 2.0;
            }
            // mv is per-frame at 60 FPS; scale so free-fly speed does not depend on frame rate
            pview.camera.position[0] += mv[0] * step_dt * 60.0;
            pview.camera.position[2] += mv[1] * step_dt * 60.0;
        }

        // Held before and after a round, the view stays where the game has it
        if session.net.is_some() && sim.frozen && toggles.first_person && sim.players[me].respawn_at.is_none() {
            let (yaw, pitch) = sim.players[me].look;
            (pview.camera.yaw, pview.camera.pitch) = sim::Input { yaw, pitch, ..sim::Input::default() }.look();
        }

        // The game runs in fixed ticks, however fast the frames come
        let mut input = sim::Input {
            forward: overlay.gate.held(Key::W),
            back: overlay.gate.held(Key::S),
            left: overlay.gate.held(Key::A),
            right: overlay.gate.held(Key::D),
            run: !pview.walk_held,
            crouch: pview.crouch_wanted,
            peek_left: overlay.gate.held(Key::Q),
            peek_right: overlay.gate.held(Key::E),
            fire: pview.trigger,
            use_key: pressed.use_key,
            reload: pressed.reload,
            gun: pressed.gun,
            grenade: pressed.grenade,
            toggle_culling: pressed.culling,
            observer: (!toggles.first_person).then(|| [pview.camera.position[0], pview.camera.position[1], -pview.camera.position[2]]),
            ..sim::Input::default()
        };
        input.set_look(pview.camera.yaw, pview.camera.pitch);
        shown = input;
        timing.tick_budget += step_dt;
        let mut events: Vec<sim::Event> = Vec::new();
        let mut ticks = 0;
        while timing.tick_budget >= sim::TICK && ticks < 5 {
            // What the player does comes from the keys, or from the recording
            let tick_input = match session.playback.as_mut() {
                Some(pb) => match pb.next_input() {
                    Some(i) => i,
                    None => break,
                },
                None => input,
            };
            pview.prev_feet = sim.players[me].player.pos;
            events.extend(match session.net.as_mut() {
                // On a server the game is its to run: send it what the player did, and put
                // what it says into this copy
                Some((client, link)) => {
                    let now = session.net_clock.elapsed().as_secs_f64();
                    link.set_time(now);
                    let news = client.tick(now, link, &mut sim, tick_input);
                    if let Some(rec) = session.net_recorder.as_mut() {
                        rec.ticked(now, tick_input, client, &sim);
                    }
                    news
                }
                None => sim.tick(&[tick_input], sim::TICK),
            });
            if let Some(rec) = session.recorder.as_mut() {
                rec.after_tick(&tick_input, &sim);
            }
            if let Some(pb) = session.playback.as_mut() {
                if let Some(d) = pb.after_tick(&sim) {
                    eprintln!("DESYNC: {d}");
                }
                // The view follows the recorded player
                (pview.camera.yaw, pview.camera.pitch) = tick_input.look();
                toggles.first_person = tick_input.observer.is_none();
                if let Some(o) = tick_input.observer {
                    pview.camera.position = [o[0], o[1], -o[2]];
                }
            }
            // Dropping in from the free view is a jump, not something to glide through
            if pview.was_free && tick_input.observer.is_none() {
                pview.prev_feet = sim.players[me].player.pos;
            }
            pview.was_free = tick_input.observer.is_some();
            shown = tick_input;
            // A key press counts once
            (input.use_key, input.reload, input.gun, input.grenade, input.toggle_culling) = (false, false, false, false, false);
            timing.tick_budget -= sim::TICK;
            ticks += 1;
        }
        if ticks > 0 {
            (pressed.use_key, pressed.reload, pressed.gun, pressed.grenade, pressed.culling) = (false, false, false, false, false);
        }
        // Past the limit the rest is dropped rather than caught up
        if timing.tick_budget >= sim::TICK {
            timing.tick_budget = 0.0;
        }
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

        // The view is at the player's eyes, between where they were a tick ago and now
        let draw_feet = if toggles.first_person {
            let alpha = if timing.fixed_step { 1.0 } else { (timing.tick_budget / sim::TICK).clamp(0.0, 1.0) };
            let (a, b) = (pview.prev_feet, sim.players[me].player.pos);
            let feet = [a[0] * (1.0 - alpha) + b[0] * alpha, a[1] * (1.0 - alpha) + b[1] * alpha, a[2] * (1.0 - alpha) + b[2] * alpha];
            let eye_target = if sim.players[me].player.crouched { crouch_eye_height } else { eye_height };
            pview.eye_now += (eye_target - pview.eye_now) * (1.0 - (-step_dt * 15.0).exp());
            let peek = sim.peek_offset(me);
            pview.camera.position = [feet[0] + peek[0], feet[1] + pview.eye_now + peek[1], -(feet[2] + peek[2])];
            feet
        } else {
            sim.players[me].player.pos
        };

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

        // Dead on a server: the view sinks to the floor and turns to whoever did it, and is given
        // back the way it was when the player comes back
        match session.net.as_ref().and_then(|_| sim.players[me].respawn_at) {
            Some(until) if toggles.first_person => {
                let (yaw0, pitch0) = *pview.alive_look.get_or_insert((pview.camera.yaw, pview.camera.pitch));
                let fall = ((game_time - (until - sim::RESPAWN_SECONDS)) / 0.6).clamp(0.0, 1.0);
                pview.camera.position[1] -= (pview.eye_now - 25.0) * fall * fall * (3.0 - 2.0 * fall);
                let (want_yaw, want_pitch) = match pview.killed_by.flatten().filter(|&k| k != me && sim.players[k].active) {
                    Some(k) => {
                        let t = sim.players[k].player.pos;
                        let d = [t[0] - pview.camera.position[0], t[1] + 100.0 - pview.camera.position[1], -t[2] - pview.camera.position[2]];
                        let flat = d[0].hypot(d[2]);
                        ((-d[0]).atan2(-d[2]).to_degrees(), d[1].atan2(flat).to_degrees())
                    }
                    None => (yaw0, pitch0),
                };
                let k = 1.0 - (-5.0 * step_dt).exp();
                pview.camera.yaw += ((want_yaw - pview.camera.yaw + 540.0).rem_euclid(360.0) - 180.0) * k;
                pview.camera.pitch += (want_pitch - pview.camera.pitch) * k;
            }
            Some(_) => {}
            None => {
                if let Some((yaw, pitch)) = pview.alive_look.take() {
                    (pview.camera.yaw, pview.camera.pitch) = (yaw, pitch);
                }
                pview.killed_by = None;
            }
        }

        // Dead in a round, the view is a teammate's (see net::watch)
        let watch_pose = match (&session.net, &frame) {
            (Some((client, _)), Some(f)) => {
                let cx = WatchContext { sim: &sim, frame: f, me, round: client.round, team_game, connected: client.status == Status::Connected };
                pview.watch.update(&cx);
                if std::mem::take(&mut pview.watch_next) {
                    pview.watch.next(&cx);
                }
                if std::mem::take(&mut pview.watch_previous) {
                    pview.watch.previous(&cx);
                }
                if std::mem::take(&mut pview.watch_mode) {
                    pview.watch.toggle_mode();
                }
                pview.watch.camera(&cx)
            }
            _ => None,
        };
        match watch_pose {
            Some(pose) => {
                // Through their eyes the height eases as they crouch and stand, as the player's
                // own does, and starts afresh with each one watched
                let target = pview.watch.target().filter(|_| pview.watch.mode == WatchMode::Eyes);
                let y = match pview.watch_eye {
                    Some((t, y)) if t == target && target.is_some() => y + (pose.eye[1] - y) * (1.0 - (-step_dt * 15.0).exp()),
                    _ => pose.eye[1],
                };
                pview.watch_eye = Some((target, y));
                pview.camera.position = [pose.eye[0], y, -pose.eye[2]];
                (pview.camera.yaw, pview.camera.pitch) = (pose.yaw, pose.pitch);
            }
            None => pview.watch_eye = None,
        }
        let watched_body = pview.watch.target().filter(|_| watch_pose.is_some() && pview.watch.mode == WatchMode::Eyes);
        // Dead, the view is drawn from the rooms of whoever the camera is on, the one watched (or
        // last watched, while their view is held) or the player's own body, where they stand
        let (on, camera_feet) = watch_pose.and(pview.watch.standing()).unwrap_or((me, sim.players[me].player.pos));
        if pview.camera_on != Some(on) {
            (pview.camera_on, pview.camera_room) = (Some(on), None);
        }

        // A blast leaves the view shaken and whited out, and the sound muffled, for a while
        pview.shake = (pview.shake - 9.0 * step_dt).max(0.0);
        pview.veil = (pview.veil - 5.0 * step_dt).max(0.0);
        pview.deaf = (pview.deaf - 0.2 * step_dt).max(0.0);
        pview.concussion.update(step_dt);
        if let Some(snd) = audio.sound.as_mut() {
            snd.master = audio.master_volume * (1.0 - 0.5 * pview.deaf.min(1.0));
            pview.stun.1 += step_dt;
            snd.set_muffle(sound::stunned_after(pview.stun.0, pview.stun.1));
        }

        // What happened in the ticks: sounds to play, effects to draw, lines for the log
        for event in events {
            match event {
                sim::Event::Sound { name, at, gain } => {
                    if let Some(snd) = audio.sound.as_mut() {
                        match at {
                            Some(p) => snd.play_at(&name, p, listener_of(&pview.camera), gain),
                            None => snd.play(&name, gain),
                        }
                    }
                }
                sim::Event::Log(line) => {
                    println!("{line}");
                    // The latest, for the debug dump
                    if overlay.recent_log.len() >= RECENT_LOG {
                        overlay.recent_log.pop_front();
                    }
                    overlay.recent_log.push_back(line);
                }
                sim::Event::Shot { player, hit_character, .. } => {
                    if player == me {
                        pview.hit_marker_until = if hit_character { game_time + 0.1 } else { 0.0 };
                    }
                    // The gun's place is known only when it is drawn, so the casings wait for that
                    if player_ejector.is_some() && !overlay.gate.showing() {
                        *scene.casing_pending.borrow_mut().entry(player).or_default() += 1;
                    }
                    if let (Some((name, at)), Some(snd)) = (sim::shot_sound(&event, me), audio.sound.as_mut()) {
                        snd.play_at(name, at, listener_of(&pview.camera), 1.0);
                    }
                }
                // A character fired: its muzzle flashes, here as on the server
                sim::Event::CharacterShot { npc, .. } => {
                    if let Some(n) = sim.npcs.get_mut(npc) {
                        n.last_shot = game_time;
                        if n.weapon.and_then(|w| assets.held_ejectors.get(w)).is_some_and(Option::is_some) && !overlay.gate.showing() {
                            *scene.casing_pending.borrow_mut().entry(NPC_OWNERS + npc).or_default() += 1;
                        }
                    }
                }
                sim::Event::BulletHit { point, normal, dir, surface, material, dist, seed } => {
                    // The bullet struck the level: leave a hole and throw some debris
                    let surface = args.decal_surface.unwrap_or(surface);
                    let mut sparked = false;
                    if args.realistic_decals {
                        sparked = scene.effects.impact(surface, point, normal, dir, seed);
                    }
                    println!("t={game_time:.2} shot hit {surface:?} ({material}) at {dist:.0} units{}", if sparked { ", sparks" } else { "" });
                    if surface != Surface::Glass {
                        let (art, variant) = if args.realistic_decals {
                            (surface, (seed >> 33) as usize % HOLE_VARIANTS)
                        } else {
                            (if surface == Surface::Wood { Surface::Wood } else { Surface::Concrete }, 0)
                        };
                        if assets.hole_tex.contains_key(&(art, variant)) {
                            let roll = ((seed >> 20) & 0xFFFF) as f32 / 65536.0;
                            scene.decals.push(impact::Decal::new(point, normal, art, args.realistic_decals, variant, roll));
                            if scene.decals.len() > MAX_DECALS {
                                scene.decals.remove(0);
                            }
                        }
                    }
                    if let Some(snd) = audio.sound.as_mut() {
                        let name = match surface {
                            _ if sparked => if seed & 1 == 0 { "C_BLRIC1.WAV" } else { "C_BLRIC2.WAV" },
                            Surface::Wood => "C_BLWOD.WAV",
                            Surface::Metal => if seed & 1 == 0 { "C_BLMTTK.WAV" } else { "C_BLMTTN.WAV" },
                            Surface::Earth => "C_BLEAR.WAV",
                            _ => "C_BLCON.WAV",
                        };
                        snd.play_at(name, point, listener_of(&pview.camera), 1.0);
                    }
                }
                sim::Event::Glass { pane, at, dir, seed } => {
                    scene.fractures.push(glass::shatter(&sim.panes[pane], at, dir, seed));
                }
                sim::Event::Blast { point, normal, surface, on_surface, seed } => {
                    let reach = sim.world.local_raycaster(point, 1500.0);
                    scene.effects.explosion(point, normal, seed, on_surface, &*reach);
                    // The light sits a little off the surface, as far as there is room
                    let n = unit3(normal);
                    let lift = reach(point, n, 60.0).map_or(60.0, |hit| hit * 0.5);
                    let at = [point[0] + n[0] * lift, point[1] + n[1] * lift, point[2] + n[2] * lift];
                    let blocked = |from: [f32; 3], to: [f32; 3]| {
                        let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
                        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                        dist > 4.0 && reach(from, [d[0] / dist, d[1] / dist, d[2] / dist], dist - 4.0).is_some()
                    };
                    scene.blast_lights.push(blastlight::BlastLight::new(at, &sim.world.solid_tris_near(at, blastlight::RADIUS), &blocked));
                    if scene.blast_lights.len() > blastlight::MOST {
                        scene.blast_lights.remove(0);
                    }
                    if on_surface && surface != Surface::Glass {
                        let roll = ((seed >> 20) & 0xFFFF) as f32 / 65536.0;
                        scene.decals.push(impact::Decal::scorch(point, normal, 100.0 + (seed >> 50) as f32 % 40.0, (seed >> 33) as usize % SCORCH_VARIANTS, roll));
                        if scene.decals.len() > MAX_DECALS {
                            scene.decals.remove(0);
                        }
                    }
                }
                // Not while watching: the view and the ears are elsewhere
                sim::Event::PlayerBlast { player, shake: s, veil: v } if player == me && watch_pose.is_none() => {
                    pview.shake = pview.shake.max(s);
                    pview.veil = pview.veil.max(v);
                    // The blast knocks the head: where the player points jumps, and drops
                    if let Some((turn, drop)) = pview.concussion.hit(s, (game_time * 1000.0) as u64) {
                        pview.camera.yaw += turn;
                        pview.camera.pitch = (pview.camera.pitch - drop).clamp(-89.0, 89.0);
                    }
                    if s > 0.1 {
                        pview.deaf = pview.deaf.max(s.min(1.0));
                        if let Some(snd) = audio.sound.as_mut() {
                            snd.ring(3.5 * s.min(1.0) + 1.0, 0.2 * s.min(1.0));
                            snd.thump(s.min(1.0));
                            // Never less stunned than what is left of an earlier blast. It takes
                            // less of a blast to stun the ears than to shake the view.
                            let stunned = (s * 2.0).min(1.0);
                            if stunned >= sound::stunned_after(pview.stun.0, pview.stun.1) {
                                pview.stun = (stunned, 0.0);
                            }
                        }
                    }
                }
                // What happened to other players, or near the body of one watching, is not felt
                // here
                sim::Event::PlayerBlast { .. } => {}
                // Named as they were when it happened, whoever has their slots now
                sim::Event::Kill { killer, victim, cause, killer_name, victim_name, .. } => {
                    let text = match (killer, cause) {
                        (Some(_), sim::Cause::HeadShot) => format!("{killer_name} HEAD SHOT {victim_name}"),
                        (Some(_), sim::Cause::Shot) => format!("{killer_name} SHOT {victim_name}"),
                        (Some(_), sim::Cause::Grenade) => format!("{killer_name} BLEW UP {victim_name}"),
                        (_, sim::Cause::Terrorist) => format!("A TERRORIST SHOT {victim_name}"),
                        (None, _) => format!("{victim_name} KILLED THEMSELVES"),
                    };
                    let color = if victim == me { [1.0, 0.35, 0.3] } else if killer == Some(me) { [1.0, 0.85, 0.3] } else { [1.0, 1.0, 1.0] };
                    overlay.feed.push((game_time + 6.0, text, color));
                    if overlay.feed.len() > 5 {
                        overlay.feed.remove(0);
                    }
                    if victim == me {
                        pview.killed_by = Some(killer);
                    }
                }
                // Said in the lobby or the round: in the chat, and in the feed for a while
                sim::Event::Chat { name, text, dead, .. } => {
                    let line = format!("{}: {text}", lobby::speaker(&name, dead));
                    println!("{line}");
                    overlay.feed.push((game_time + 8.0, line, [0.95, 0.78, 0.2]));
                    if overlay.feed.len() > 5 {
                        overlay.feed.remove(0);
                    }
                    overlay.chat.said(&name, &text, dead);
                }
            }
        }

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
        let view_to_map: Option<casings::Mat4>;
        let drawn_rooms: Vec<String>;
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
                let p = behind(&sim, from, args.view_dist.unwrap_or(BEHIND), RISE, toggles.orbit.0);
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
            let mut plan: Vec<rogue_reborn::visibility::PlanNode> = Vec::new();
            // Each visible room once, with its screen rectangle (unless --portal-paths)
            let mut rect_plan: Vec<rogue_reborn::visibility::RectRoom> = Vec::new();
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
                    plan = cull_rooms.iter().map(|&r| sim.visibility.plan(&level, r, &cull_body, cam_eye, &sim.blocked_portals, &clip)).collect();
                } else {
                    rect_plan = sim.visibility.rect_plan(&level, &cull_rooms, &cull_body, cam_eye, &sim.blocked_portals, &clip);
                    // The player's rooms and the doorways they stand in are still cut where
                    // they overlap
                    plan = cull_rooms.iter().map(|&r| sim.visibility.touched_plan(&level, r, &cull_body, cam_eye, &sim.blocked_portals)).collect();
                }
            }
            let mut visible_rooms: Vec<u32> = rect_plan.iter().map(|rr| rr.room).collect();
            for node in &plan {
                node.rooms(&mut visible_rooms);
            }
            visible_rooms.sort();
            visible_rooms.dedup();
            drawn_rooms = visible_rooms.iter().map(|n| n.to_string()).collect();

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

                // Draws some of the level's objects, and only the part beyond `plane`.
                // An object belongs to the room its name starts with ("204", "20_doormetal").
                let mut draw_objects = |which: Which, planes: &[[f32; 4]]| {
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
                        let tex_id = if toggles.room_debug { Some(0) } else { ensure_texture(&level.materials, mat_index, &session.map_num, &mut textures) };
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
                };

                if !rect_plan.is_empty() {
                    // The player's rooms and the doorways they stand in, cut where they
                    // overlap, then every other room whole, within its rectangle, nearest
                    // first, sorted out by depth as the original engine does. Inside the
                    // openings of the player's rooms' own doorways the room beyond shows in
                    // front of them: rm11's hall's wall, one step past its door into 119 (its
                    // stairs there are seen from behind, and culled). Deeper, overlaps show,
                    // as in the original (the hall's wall in 120's doorway, see BUGS.txt)
                    draw_objects(Which::Unowned, &[]);
                    let mut cut_rooms: Vec<u32> = Vec::new();
                    glEnable(GL_STENCIL_TEST);
                    for node in &plan {
                        node.rooms(&mut cut_rooms);
                        draw_plan(node, 1, level_mode, toggles.textured && !toggles.room_debug, &mut |room, planes| draw_objects(Which::Room(room), planes));
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
                        draw_objects(Which::Room(rr.room), &[]);
                    }
                    glDisable(GL_SCISSOR_TEST);
                } else if plan.is_empty() {
                    draw_objects(Which::All, &[]);
                } else {
                    // Things that belong to no room first, then each room through its portals
                    draw_objects(Which::Unowned, &[]);
                    glEnable(GL_STENCIL_TEST);
                    for node in &plan {
                        draw_plan(node, 1, level_mode, toggles.textured && !toggles.room_debug, &mut |room, planes| draw_objects(Which::Room(room), planes));
                    }
                    glDisable(GL_STENCIL_TEST);
                }
                set_clip_planes(&[]);
                glColor3f(1.0, 1.0, 1.0);
            }

            let view = frame.clone();

            // A character shows only when a room it is in is drawn: the level overlaps itself,
            // and on rm05 a character on the plane's upper deck pokes out of the top of the hull.
            // `feet` in map space; with no rooms drawn (culling off) everyone shows.
            let shown = |feet: [f32; 3]| visible_rooms.is_empty()
                || sim.visibility.rooms_holding(&level, [feet[0], feet[1] + 10.0, feet[2]], None).iter().any(|r| visible_rooms.contains(r));

            // The light of a blast on a character standing at `feet`: nothing when there is none
            let glow_on = |feet: [f32; 3]| -> [f32; 3] {
                let mut sum = [0.0f32; 3];
                let chest = [feet[0], feet[1] + 100.0, feet[2]];
                let blocked = |from: [f32; 3], to: [f32; 3]| {
                    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
                    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    dist > 10.0 && sim.world.raycast(from, [d[0] / dist, d[1] / dist, d[2] / dist], dist - 10.0).is_some()
                };
                for l in &scene.blast_lights {
                    let g = l.glow_at(chest, &blocked);
                    for k in 0..3 {
                        sum[k] += g[k];
                    }
                }
                sum
            };

            // The level's light on a character standing at `feet`, turned `yaw` and scaled, in its own
            // space: its room's ambient light and strongest lights. None with --flat-light.
            let lit_at = |feet: [f32; 3], yaw: f32, scale: [f32; 3]| -> Option<CharacterLight> {
                Some(standing_light(session.lighting.as_ref()?, &level, &sim.visibility, feet).local(feet, yaw, scale))
            };
            // A flashing gun lights its shooter. With no falloff only the direction matters, so the
            // light sits ahead of the chest, at gun height, in the character's own space.
            let flashing = |lit: Option<CharacterLight>, on: bool| lit.map(|l| if on { l.with_muzzle([0.0, 100.0, 60.0]) } else { l });
            let tint = |lit: &Option<CharacterLight>| lit.as_ref().map_or([1.0; 3], |l| l.tint);

            let note_flash = |wd: &WeaponDraw| scene.flash_spots.borrow_mut().push(FlashSpot::here(wd, flash_roll()));
            let note_casings = |owner: usize, ejector: &casings::Ejector| {
                if scene.casing_pending.borrow().contains_key(&owner) {
                    let mut matrix = [0f32; 16];
                    glGetFloatv(GL_MODELVIEW_MATRIX, matrix.as_mut_ptr());
                    scene.casing_spots.borrow_mut().push((owner, *ejector, matrix));
                }
            };

            // Characters at the spawn points, posed for what they are doing
            if let Some(chars) = &sim.characters {
                let c = &chars.model;
                glDisable(GL_TEXTURE_2D);
                let clock = if timing.fixed_step { game_time as f32 } else { timing.start_time.elapsed().as_secs_f32() };
                for (i, spot) in sim.npcs.iter().enumerate() {
                    let mut moved;
                    let spot = match view.as_ref().and_then(|v| v.npcs.get(i)) {
                        Some(&(pos, yaw)) => {
                            moved = spot.clone();
                            (moved.pos[0], moved.pos[2], moved.yaw) = (pos[0], pos[2], yaw);
                            &moved
                        }
                        None => spot,
                    };
                    if !shown([spot.pos[0], spot.floor, spot.pos[2]]) {
                        continue;
                    }
                    let idle = args.anim_frame.unwrap_or_else(|| game_time * args.anim_fps.unwrap_or(30.0) + spot.stagger * 9.0);
                    let (model, arrays, textures) = match spot.body.and_then(|b| chars.bodies.get(b).zip(assets.body_draws.get(b))) {
                        Some((b, (arrays, textures))) => (&b.model, arrays, textures.as_slice()),
                        None => (c, &assets.character_arrays, assets.part_textures.as_slice()),
                    };
                    let bones = chars.bones(spot, game_time, idle);
                    let (posed, turns) = match (&bones, chars.body(spot).1) {
                        (Some(bones), Some(rig)) => (rig.skin.apply(bones), rig.skin.turns(bones)),
                        (None, Some(_)) => (model.vertices.clone(), Vec::new()),
                        (_, None) => (Vec::new(), Vec::new()),
                    };
                    let verts: &[[f32; 3]] = if posed.is_empty() { &model.vertices } else { &posed };
                    let y = if posed.is_empty() { spot.pos[1] } else { spot.floor };
                    glPushMatrix();
                    glTranslatef(spot.pos[0], y, spot.pos[2]);
                    glRotatef(spot.yaw, 0.0, 1.0, 0.0);
                    glScalef(spot.scale[0], spot.scale[1], spot.scale[2]);
                    // Eyes shut for 0.15 s every 4 s, staggered per model; the dead keep them shut
                    let blinking = args.eyes_closed || spot.health.is_dead()
                        || (toggles.blink_enabled && (clock + spot.stagger) % 4.0 < 0.15);
                    let face = npc_actor[i].map(|a| assets.face_textures[a]).and_then(|(main, blink)| {
                        if blinking { blink.or(main) } else { main }
                    });
                    let held = spot.weapon.and_then(|w| assets.held_draws.get(w)).and_then(Option::as_ref);
                    let glowing = npc_flash.get(i).copied().unwrap_or(false) && held.is_some_and(|w| w.flash_scale > 0.0) && !spot.health.is_dead();
                    let lit = flashing(lit_at([spot.pos[0], y, spot.pos[2]], spot.yaw, spot.scale), glowing);
                    draw_character(model, arrays, verts, &turns, textures, face, toggles.textured, glow_on([spot.pos[0], y, spot.pos[2]]), lit.as_ref());
                    // Their weapon in the right hand, as the players hold theirs, or on the
                    // floor once they have dropped it dying
                    if let (Some(bones), Some(wd), Some((hand, off))) = (&bones, held, &assets.weapon_attach) {
                        glPushMatrix();
                        let lying = match chars.dropped_weapon(spot, game_time, *hand, off) {
                            Some(lying) => {
                                glMultMatrixf(lying.as_ptr());
                                true
                            }
                            None => {
                                glMultMatrixf(bones[*hand].matrix().as_ptr());
                                glMultMatrixf(off.matrix().as_ptr());
                                false
                            }
                        };
                        draw_weapon(wd, toggles.textured, tint(&lit));
                        if !lying && npc_flash.get(i).copied().unwrap_or(false) {
                            note_flash(wd);
                        }
                        if let (false, Some(e)) = (lying, spot.weapon.and_then(|w| assets.held_ejectors.get(w)).and_then(Option::as_ref)) {
                            note_casings(NPC_OWNERS + i, e);
                        }
                        glPopMatrix();
                    }
                    glPopMatrix();
                }
                glColor4f(1.0, 1.0, 1.0, 1.0);
            }

            // How a posed body is drawn, for the other players and for the player's own
            // `reloading` is the reload they are in the middle of, if any: its motion and frame
            // follow its progress, on the beats of its sound. `firing` says the gun is firing,
            // which the arms show whatever the legs do. `crouched` kneels the body for what is
            // done in place, and a change of stance plays crouching down or standing up. `flinch`
            // is a wound's flinch, its frame and when it began. It takes the whole body standing
            // in place and the upper body walking. `hip_turn` is how far the hips would turn
            // toward a side step (see `movement::hip_turn`). `peek` takes the whole body while it
            // lasts.
            // `body` is what the drawing remembers of this body from frame to frame.
            let draw_body = |chars: &Characters, side: Option<sim::Side>, movement: Movement, frame: f32, feet: [f32; 3], yaw_deg: f32, scale: [f32; 3],
                             face: Option<u32>, hand: Option<&Option<WeaponDraw>>, grenade: bool, flash: bool, owner: usize, reloading: Option<Reloading>,
                             firing: bool, crouched: bool, limp: bool, hip_turn: f32, flinch: Option<(&rogue_reborn::motion::Motion, f32, f32)>,
                             peek: Option<rogue_reborn::sim::peek::Peek>, body: &mut BodyState| -> bool {
                let Some(Rig { skeleton: skel, skin }) = &chars.rig else { return false };
                let moving = assets.player_motions.moving(movement, peek, flinch.is_some());
                // Crouching down or standing up plays in place, at its own pace; moving cuts it short
                let since = body.since_stance_change(crouched, game_time);
                let changing = since.filter(|_| !moving)
                    .and_then(|t| assets.player_motions.stance_change(crouched).map(|m| (m, t * 30.0)))
                    .filter(|(m, f)| *f < m.frames.saturating_sub(1) as f32);
                // Standing in place, the feet step round once the body has turned far enough
                let shuffle = body.shuffle(yaw_deg, !moving && changing.is_none(), game_time)
                    .and_then(|at| assets.player_motions.turn(crouched).map(|m| (m, (game_time - at) * 30.0, at)))
                    .filter(|(m, f, _)| *f < m.frames.saturating_sub(1) as f32);
                let turn = body.turn_hips(if moving { hip_turn } else { 0.0 }, game_time);
                let doing = body::Doing {
                    movement, frame, crouched, limp, reloading, firing, flinch: flinch.map(|(m, f, _)| (m, f)), peek, hip_turn: turn,
                    changing, shuffle: shuffle.map(|(m, f, _)| (m, f)), clock: game_time,
                };
                let Some(body::Pose { mut root, rotations: mut rots, aiming, peeking, shuffling }) = body::pose(&assets.player_motions, skel, &doing) else { return false };
                // Raising the gun to fire is a cut, as in the original, so the first shots never
                // leave a gun still on its way up. Once firing, the legs still ease.
                let raising = aiming && !body.fade.key().is_some_and(|k| k.aiming);
                let ease = if raising { 0.0 } else { rogue_reborn::motion::CROSSFADE_SECONDS };
                let key = BodyKey { movement, reload: reloading.map(|r| r.empty), aiming, crouched, changing: changing.is_some(), flinch: flinch.map(|f| f.2.to_bits()), shuffle: shuffle.filter(|_| shuffling).map(|s| s.2.to_bits()), peek: peeking };
                body.fade.apply(key, game_time, ease, &mut root, &mut rots);
                let pose = skel.pose([0.0, root[1], 0.0], &rots);
                let (model, arrays, textures, worn) = dress(side, &assets.side_dress, &chars.model, &assets.character_arrays, &assets.part_textures, Some(skin));
                let verts = worn.unwrap_or(skin).apply(&pose);
                let turns = worn.unwrap_or(skin).turns(&pose);
                glPushMatrix();
                glTranslatef(feet[0], feet[1], feet[2]);
                // The body faces where the player looks (model forward is +Z)
                glRotatef(yaw_deg, 0.0, 1.0, 0.0);
                glScalef(scale[0], scale[1], scale[2]);
                let glowing = flash && hand.and_then(Option::as_ref).is_some_and(|w| w.flash_scale > 0.0);
                let lit = flashing(lit_at(feet, yaw_deg, scale), glowing);
                draw_character(model, arrays, &verts, &turns, textures, face, toggles.textured, glow_on(feet), lit.as_ref());
                // What bullets hit (B): the players' model posed alike, whatever the uniform,
                // outlined over it
                if toggles.show_hitboxes {
                    let verts = skin.apply(&pose);
                    glDisable(GL_TEXTURE_2D);
                    glDisable(GL_DEPTH_TEST);
                    glColor3f(1.0, 0.85, 0.0);
                    glBegin(GL_LINES);
                    for t in chars.model.parts.iter().flat_map(|p| &p.triangles) {
                        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                            let (a, b) = (verts[a as usize], verts[b as usize]);
                            glVertex3f(a[0], a[1], a[2]);
                            glVertex3f(b[0], b[1], b[2]);
                        }
                    }
                    glEnd();
                    glEnable(GL_DEPTH_TEST);
                    glColor4f(1.0, 1.0, 1.0, 1.0);
                }

                // The MP5K in the right hand, or a grenade until it is thrown
                if let (Some(Some(wd)), Some((hand, off))) = (hand, &assets.weapon_attach) {
                    glPushMatrix();
                    glMultMatrixf(pose[*hand].matrix().as_ptr());
                    glMultMatrixf(off.matrix().as_ptr());
                    if grenade {
                        glTranslatef(-wd.center[0], -wd.center[1], -wd.center[2]);
                    }
                    draw_weapon(wd, toggles.textured, tint(&lit));
                    if flash {
                        note_flash(wd);
                    }
                    if let (false, Some(e)) = (grenade, player_ejector.as_ref()) {
                        note_casings(owner, e);
                    }
                    glPopMatrix();
                }
                glPopMatrix();
                glColor4f(1.0, 1.0, 1.0, 1.0);
                true
            };

            // The flinch a living player's latest wound plays at `now`, while it lasts
            fn flinch_at<'a>(chars: &'a Characters, p: &rogue_reborn::sim::PlayerState, now: f32) -> Option<(&'a rogue_reborn::motion::Motion, f32, f32)> {
                let hit = p.hit.filter(|_| p.respawn_at.is_none())?;
                let m = chars.motions.get(hit.motion)?;
                let frame = (now - hit.start) * 30.0;
                (0.0..m.frames as f32).contains(&frame).then_some((m, frame, hit.start))
            }

            // The other players on the server, moving and handling their weapons as they are seen to
            if let Some(chars) = &sim.characters {
                glDisable(GL_TEXTURE_2D);
                let others = view.as_ref().map_or(&[][..], |v| &v.players[..]);
                // Alive or dead as the view has them, at the moment it shows, not as the newest snapshot says
                let view_time = view.as_ref().map_or(game_time, |v| v.time);
                // Not the one watched, through whose eyes the view is
                for &Seen { slot, feet: pos, look, dead, crouched, peek, .. } in others.iter().filter(|s| s.slot != me && Some(s.slot) != watched_body) {
                    let visible = shown(pos);
                    let p = &sim.players[slot];
                    let yaw = sim::dequantize(look.0);
                    let face = (!assets.actors.is_empty()).then(|| assets.face_textures[slot % assets.actors.len()].0).flatten();
                    if dead.is_none() {
                        let doing = Doing {
                            throwing: p.throw.map(|t| (game_time - t.start) / sim::THROW_SECONDS),
                            reloading: p.reloading(game_time),
                            // The server sends others' trigger only while their gun has rounds
                            firing: p.gun_trigger,
                            crouched,
                            wounds: p.wounds,
                            leg_wounds: p.leg_wounds,
                        };
                        let limp = p.leg_wounds > 0;
                        let frames = |m: Movement| assets.player_motions.get(m, doing.crouched, limp).map_or(1, |m| m.frames.max(1)) as f32;
                        let (movement, frame) = scene.remote_anim.update(slot, pos, yaw, step_dt, doing, frames, args.speed_scale.unwrap_or(1.0), args.anim_scale.unwrap_or(1.0));
                        let scale = assets.actors.get(slot % assets.actors.len().max(1)).map_or([1.0; 3], |a| a.scale);
                        let hand = if p.equipped_grenade { p.throw.is_none_or(|t| !t.thrown).then_some(&assets.grenade_draw) } else { Some(&assets.weapon_draw) };
                        let flash = player_flash[slot];
                        // Hidden, the animation still follows them
                        let flinch = flinch_at(chars, p, view_time);
                        let hip_turn = scene.remote_anim.hip_turn(slot, yaw);
                        if !visible || draw_body(chars, p.side, movement, frame, pos, -yaw, scale, face, hand, p.equipped_grenade, flash, slot, doing.reloading, doing.firing, doing.crouched, limp, hip_turn, flinch, peek, scene.bodies.entry(slot).or_default()) {
                            continue;
                        }
                    } else {
                        // Dead, they are drawn where they fell, with the bodies
                        scene.remote_anim.forget(slot);
                        scene.bodies.remove(&slot);
                        continue;
                    }
                    if !visible {
                        continue;
                    }
                    // Without the player animations: a character, idling where they are
                    let ghost = Npc {
                        health: combat::Health::new(),
                        pos,
                        floor: pos[1],
                        yaw: -yaw,
                        scale: [1.0; 3],
                        stagger: slot as f32 * 0.73,
                        ..Npc::default()
                    };
                    let idle = game_time * 30.0 + ghost.stagger * 9.0;
                    let (model, arrays, textures, skin) = dress(p.side, &assets.side_dress, &chars.model, &assets.character_arrays, &assets.part_textures, chars.rig.as_ref().map(|r| &r.skin));
                    let (posed, turns) = chars.bones(&ghost, view_time, idle).zip(skin).map(|(b, s)| (s.apply(&b), s.turns(&b))).unwrap_or_default();
                    let verts: &[[f32; 3]] = if posed.is_empty() { &model.vertices } else { &posed };
                    glPushMatrix();
                    glTranslatef(ghost.pos[0], ghost.pos[1], ghost.pos[2]);
                    glRotatef(ghost.yaw, 0.0, 1.0, 0.0);
                    let lit = lit_at(ghost.pos, ghost.yaw, ghost.scale);
                    draw_character(model, arrays, verts, &turns, textures, face, toggles.textured, glow_on(ghost.pos), lit.as_ref());
                    glPopMatrix();
                }
                glColor4f(1.0, 1.0, 1.0, 1.0);
            }

            // The player's own body, shown in third person
            if let (true, true, Some(chars)) = (toggles.first_person, toggles.third_person, &sim.characters) {
                let clock = if timing.fixed_step { game_time as f32 } else { timing.start_time.elapsed().as_secs_f32() };
                let blinking = args.eyes_closed || (toggles.blink_enabled && clock % 4.0 < 0.15);
                let face = (!assets.actors.is_empty()).then(|| assets.face_textures[0]).and_then(|(main, blink)| {
                    if blinking { blink.or(main) } else { main }
                });
                let scale = assets.actors.first().map_or([1.0; 3], |a| a.scale);
                let mine = &sim.players[me];
                let hand = if mine.equipped_grenade { mine.throw.is_none_or(|t| !t.thrown).then_some(&assets.grenade_draw) } else { Some(&assets.weapon_draw) };
                let flash = player_flash[me];
                if mine.respawn_at.is_none() {
                    let flinch = flinch_at(chars, mine, game_time);
                    draw_body(chars, mine.side, pview.movement, pview.player_frame, draw_feet, -pview.camera.yaw, scale, face, hand, mine.equipped_grenade, flash, me, mine.reloading(game_time), sim.firing(me), mine.player.crouched, mine.leg_wounds > 0, pview.player_hip_turn, flinch, mine.peek, scene.bodies.entry(me).or_default());
                }
            }

            // The bodies of the players who died, falling where they did with the motion the
            // server picked, and lying there for as long as the fall plays or they wait to come back
            if let Some(chars) = &sim.characters {
                for (slot, p) in sim.players.iter().enumerate().filter(|(_, p)| p.active) {
                    if let (Some(_), Some(hit)) = (p.respawn_at, p.hit) {
                        if scene.corpses.get(&slot).is_none_or(|(start, _)| (start - hit.start).abs() > 0.1) {
                            let health = combat::Health { hp: 0.0, state: combat::NpcState::Dead { motion: hit.motion, start: hit.start }, ..Default::default() };
                            let yaw = -sim::dequantize(p.look.0);
                            let scale = assets.actors.get(slot % assets.actors.len().max(1)).map_or([1.0; 3], |a| a.scale);
                            let pos = p.player.pos;
                            scene.corpses.insert(slot, (hit.start, Npc { health, pos, floor: pos[1], yaw, scale, stagger: 0.0, body: None, ..Npc::default() }));
                        }
                    }
                }
                let view_time = view.as_ref().map_or(game_time, |v| v.time);
                // The others as the view shows them
                let now_for = |slot: usize| if slot == me { game_time } else { view_time };
                // A body stays while its player is dead, and for a while after they come back
                scene.corpses.retain(|&slot, (start, body)| {
                    let combat::NpcState::Dead { motion, .. } = body.health.state else { return false };
                    sim.players.get(slot).is_some_and(|p| p.active && p.respawn_at.is_some())
                        || now_for(slot) - *start < sim::RESPAWN_SECONDS.max(chars.frames(motion) / 30.0)
                });
                glDisable(GL_TEXTURE_2D);
                for (&slot, (start, body)) in &scene.corpses {
                    // The player's own body only from outside it
                    let now = now_for(slot);
                    if (slot == me && !(toggles.first_person && toggles.third_person)) || now < *start || !shown(body.pos) {
                        continue;
                    }
                    let face = (!assets.actors.is_empty()).then(|| assets.face_textures[slot % assets.actors.len()].0).flatten();
                    let side = sim.players.get(slot).and_then(|p| p.side);
                    let (model, arrays, textures, skin) = dress(side, &assets.side_dress, &chars.model, &assets.character_arrays, &assets.part_textures, chars.rig.as_ref().map(|r| &r.skin));
                    let (posed, turns) = chars.bones(body, now, 0.0).zip(skin).map(|(b, s)| (s.apply(&b), s.turns(&b))).unwrap_or_default();
                    let verts: &[[f32; 3]] = if posed.is_empty() { &model.vertices } else { &posed };
                    glPushMatrix();
                    glTranslatef(body.pos[0], body.pos[1], body.pos[2]);
                    glRotatef(body.yaw, 0.0, 1.0, 0.0);
                    glScalef(body.scale[0], body.scale[1], body.scale[2]);
                    let lit = lit_at(body.pos, body.yaw, body.scale);
                    draw_character(model, arrays, verts, &turns, textures, face, toggles.textured, glow_on(body.pos), lit.as_ref());
                    glPopMatrix();
                }
                glColor4f(1.0, 1.0, 1.0, 1.0);
            }

            // Grenades in the air
            if let Some(gd) = &assets.grenade_draw {
                let flying: Vec<([f32; 3], f32)> = match &view {
                    Some(v) => v.grenades.iter().map(|&(_, pos, spin)| (pos, spin)).collect(),
                    None => sim.thrown.iter().map(|t| (t.grenade.pos, t.grenade.spin)).collect(),
                };
                for (pos, spin) in flying {
                    glPushMatrix();
                    glTranslatef(pos[0], pos[1], pos[2]);
                    glRotatef(spin, 1.0, 0.3, 0.0);
                    // Model space (forward -Y, up +Z) turned so its length runs the way it spins
                    glTranslatef(-gd.center[0], -gd.center[1], -gd.center[2]);
                    let light = session.lighting.as_ref().map_or([1.0; 3], |l| room_light(l, &level, &sim.visibility, pos, pos).tint);
                    draw_weapon(gd, toggles.textured, light);
                    glPopMatrix();
                }
                glColor4f(1.0, 1.0, 1.0, 1.0);
            }

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
            let pane_shows = |pane: &glass::Pane| plan.is_empty()
                || map::object_room(&pane.name).is_none_or(|n| visible_rooms.contains(&n));
            for (pane, _) in sim.panes.iter().zip(&sim.broken_at).filter(|(p, b)| b.is_none() && pane_shows(p)) {
                draw_glass(&level.materials, pane.material, &session.map_num, &mut textures, toggles.textured, pane.opacity, pane.tris.iter().map(|t| (t.p, t.uv)));
            }
            for f in &scene.fractures {
                draw_fracture(&level.materials, &session.map_num, &mut textures, toggles.textured, f);
            }

            if !overlay.gate.showing() {
                let spots = scene.casing_spots.take();
                if let Some(to_map) = &view_to_map {
                    for (owner, ejector, gun) in spots {
                        let rounds = scene.casing_pending.borrow().get(&owner).copied().unwrap_or(0);
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
                for spot in scene.flash_spots.borrow().iter() {
                    draw_flash(flash, spot);
                }
            }
            scene.flash_spots.borrow_mut().clear();
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
                let others = view.as_ref().map_or(&[][..], |v| &v.players[..]);
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
        let casing_rounds = scene.casing_pending.borrow().get(&me).copied().unwrap_or(0);
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
                        draw_flash_here(flash, &FlashSpot::here(wd, flash_roll()));
                    }
                }
                // The casings leave the gun as it is placed here.
                // Its view is narrower than the world's, so they start further from the middle.
                if let (true, false, Some(e), Some(to_map)) = (casing_rounds > 0, overlay.gate.showing(), player_ejector.as_ref().filter(|_| !sim.players[me].equipped_grenade), &view_to_map) {
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
        scene.casing_pending.borrow_mut().clear();
        scene.casing_spots.borrow_mut().clear();

        // Debug text and the HUD
        let hud = HudPass { args: &args, level: &level, sim: &sim, me, game_time, w, h, fps, render_time, watch_pose, current_rooms: &current_rooms, drawn_rooms: &drawn_rooms };
        let render_ms = overlay.draw_hud_and_debug(hud, &toggles, &timing, &pview, &session);

        // The lobby, the results and the chat, over the game
        overlay.draw_lobby(&mut session, &sim, me, game_time, &window);

        // F2: a screenshot and a text log of where the player is and what they see, the
        // screenshot mode's capture, and the benchmark's timing
        let capture = CapturePass { args: &args, level: &level, sim: &sim, me, fps, render_ms, raw_dt, current_rooms: &current_rooms, drawn_rooms: &drawn_rooms };
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
