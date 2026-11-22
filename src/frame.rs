use std::time::Instant;
use glfw::Key;
use rogue_reborn::sim;
use crate::overlay::Overlay;
use crate::session::Session;
use crate::state::{Pressed, Timing, Toggles};
use crate::view::PlayerView;

// What the frame's clock and ticks read besides the loop's structs
pub struct TickPass {
    pub me: usize,
    pub base_speed: f32,
}

// What a frame's clock and ticks leave for the rest of the frame
pub struct FrameTicks {
    // The time since the last frame, as it was, and as the game takes it
    pub raw_dt: f32,
    pub step_dt: f32,
    // What happened in the ticks
    pub events: Vec<sim::Event>,
    // What the player did in the last tick, which the animation follows
    pub shown: sim::Input,
}

// The frame's time, the free view's movement, and the fixed ticks that feed the game. A key
// press counts for the first tick only, and the latches clear after the ticks only if one ran.
pub fn run_ticks(pass: TickPass, sim: &mut sim::Sim, session: &mut Session, toggles: &mut Toggles, timing: &mut Timing,
                 overlay: &Overlay, pview: &mut PlayerView, pressed: &mut Pressed) -> FrameTicks {
    let TickPass { me, base_speed } = pass;
    let mut shown;
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
                let news = client.tick(now, link, sim, tick_input);
                if let Some(rec) = session.net_recorder.as_mut() {
                    rec.ticked(now, tick_input, client, sim);
                }
                news
            }
            None => sim.tick(&[tick_input], sim::TICK),
        });
        if let Some(rec) = session.recorder.as_mut() {
            rec.after_tick(&tick_input, sim);
        }
        if let Some(pb) = session.playback.as_mut() {
            if let Some(d) = pb.after_tick(sim) {
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

    FrameTicks { raw_dt, step_dt, events, shown }
}
