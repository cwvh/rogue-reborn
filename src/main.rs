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
use rogue_reborn::{sim, casings};
use rogue_reborn::net::client::Status;
use rogue_reborn::sim::movement::{Movement, animation_pace, hip_turn, movement_from_keys};
use crate::args::parse_args;
use crate::render::characters::{CharacterPass, FrameOutput, draw_characters};
use crate::render::effects::{CasingPass, GlassPass, OutlinePass, draw_blast_lights, draw_casings, draw_flashes, draw_marks, draw_outlines, draw_panes};
use crate::render::view_model::{ViewModelPass, draw_view_model};
use crate::render::level::LevelDraw;
use crate::render::world::{WorldPass, draw_world};
use crate::render::window;
use crate::audio::Audio;
use crate::capture::{CapturePass, capture_frame};
use crate::input::{InputPass, poll_window, script_keys};
use crate::frame::{FrameTicks, TickPass, run_ticks};
use crate::events::{EventPass, handle_events};
use crate::overlay::{HudPass, Overlay};
use crate::scene::Scene;
use crate::session::{Game, Session};
use crate::view::{PlayerView, ViewPass, Watched};
use crate::state::{Pressed, Timing, Toggles};
use crate::render::draw::level_radius;

// The first-person view's height in degrees. The width follows the screen shape.
const VIEW_HIGH: f32 = 90.0;

fn half_view_tan() -> f32 {
    (VIEW_HIGH.to_radians() / 2.0).tan()
}

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
        let characters = CharacterPass { args: &args, assets: &assets, level: &level, sim: &sim, session: &session, toggles: &toggles, timing: &timing, drawn: &drawn, frame: frame.as_ref(), me, game_time, step_dt, npc_actor: &npc_actor, player_flash: &player_flash, npc_flash: &npc_flash, watched_body, draw_feet, player_ejector: player_ejector.as_ref() };
        draw_characters(&characters, &pview, &mut scene, &mut out);
        let textured = toggles.textured;
        draw_blast_lights(&mut scene.blast_lights, &assets, textured, step_dt);
        draw_marks(&mut scene, &assets, textured, &pview.camera, step_dt);
        let glass = GlassPass { sim: &sim, level: &level, session: &session, drawn: &drawn, textured, step_dt };
        draw_panes(glass, &mut scene.fractures, &mut textures);
        let casings = CasingPass { sim: &sim, assets: &assets, level: &level, toggles: &toggles, drawn: &drawn, camera: &pview.camera, view_to_map: drawn.view_to_map, me, game_time, step_dt, gate_showing: overlay.gate.showing() };
        draw_casings(casings, &mut scene, &mut audio, &mut out);
        draw_flashes(&assets, textured, &mut out);
        draw_outlines(OutlinePass { level: &level, sim: &sim, frame: frame.as_ref(), me, draw_feet }, &toggles);
        let view_model = ViewModelPass { args: &args, assets: &assets, level: &level, sim: &sim, session: &session, toggles: &toggles, me, game_time, aspect, player_flash: &player_flash, player_ejector: player_ejector.as_ref(), view_to_map: drawn.view_to_map, gate_showing: overlay.gate.showing() };
        draw_view_model(view_model, &mut scene, &mut out);

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
