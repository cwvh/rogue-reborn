use glfw::{Action, Key, WindowEvent, CursorMode};
use rogue_reborn::map;
use rogue_reborn::net::client::Status;
use crate::args::Args;
use crate::capture::CLIP_SECONDS;
use crate::opengl::*;
use crate::overlay::Overlay;
use crate::scene::Scene;
use crate::session::Session;
use crate::state::{Pressed, Timing, Toggles};
use crate::view::PlayerView;
use crate::SPRITE_MODES;

// What the window events read besides the loop's structs
pub struct InputPass<'a> {
    pub args: &'a Args,
    pub level: &'a map::Map,
    pub base_speed: f32,
}

// Polls the window and acts on what it reports: the lobby's gate, the keys, the mouse
pub fn poll_window(pass: InputPass, glfw: &mut glfw::Glfw, events: &glfw::GlfwReceiver<(f64, glfw::WindowEvent)>, window: &mut glfw::PWindow,
                   toggles: &mut Toggles, overlay: &mut Overlay, pview: &mut PlayerView, pressed: &mut Pressed,
                   scene: &mut Scene, session: &mut Session) {
    let InputPass { args, level, base_speed } = pass;
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

    for (_, event) in glfw::flush_messages(events) {
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
}

// A scripted run (--walk) presses its own keys
pub fn script_keys(args: &Args, sim: &rogue_reborn::sim::Sim, me: usize, timing: &Timing, overlay: &mut Overlay,
                   pview: &mut PlayerView, pressed: &mut Pressed) {
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
}
