use std::path::Path;
use glfw::Window;
use rogue_reborn::sim;
use rogue_reborn::map;
use crate::args::Args;
use crate::debug::{DebugReport, write_debug_dump};
use crate::opengl::*;
use crate::overlay::Overlay;
use crate::scene::Scene;
use crate::session::Session;
use crate::state::{Timing, Toggles};
use crate::view::PlayerView;

// How long F4 records, in seconds
pub const CLIP_SECONDS: f32 = 2.0;

/// Frames to skip before timing, while textures load
const BENCH_WARMUP: u32 = 30;

// What the capture reads besides the loop's structs
pub struct CapturePass<'a> {
    pub args: &'a Args,
    pub level: &'a map::Map,
    pub sim: &'a sim::Sim,
    pub me: usize,
    pub fps: f32,
    pub render_ms: f32,
    pub raw_dt: f32,
    pub current_rooms: &'a [&'a str],
    pub drawn_rooms: &'a [String],
}

// Runs after the frame is drawn and before the buffers swap, so the readback sees the frame.
// Returns true when the loop is to end.
pub fn capture_frame(pass: CapturePass, toggles: &mut Toggles, timing: &mut Timing, overlay: &mut Overlay,
                     scene: &Scene, pview: &PlayerView, session: &Session, window: &mut Window) -> bool {
    let CapturePass { args, level, sim, me, fps, render_ms, raw_dt, current_rooms, drawn_rooms } = pass;
    // F2: a screenshot and a text log of where the player is and what they see
    if (toggles.dump_requested && (!args.dump || timing.frames_drawn >= 2)) || toggles.clip.is_some() {
        let one_dump = std::mem::take(&mut toggles.dump_requested);
        let stem = toggles.clip.as_mut().map(|(_, dir, n)| {
            *n += 1;
            format!("{dir}/{:04}", *n)
        });
        let (fw, fh) = window.get_framebuffer_size();
        let rgb = read_framebuffer(fw, fh);
        let eye = [pview.camera.position[0], pview.camera.position[1], -pview.camera.position[2]];
        let f = pview.camera.get_forward();
        let dir = [f[0], f[1], -f[2]];
        let hit = sim.world.raycast_hit(eye, dir, 20000.0).map(|hit| {
            let point = [eye[0] + dir[0] * hit.dist, eye[1] + dir[1] * hit.dist, eye[2] + dir[2] * hit.dist];
            let material = hit.material.and_then(|m| level.materials.materials.get(m)).map_or("?", |m| m.filename.as_str());
            (hit, point, material)
        });
        let feet = [eye[0], eye[1] - if toggles.first_person { pview.eye_now } else { 0.0 }, eye[2]];
        let nearby_doors: Vec<String> = sim.doors.iter().zip(&sim.door_states)
            .filter_map(|(d, s)| {
                let c = d.center(s.open);
                let dist = ((c[0] - eye[0]).powi(2) + (c[1] - eye[1]).powi(2) + (c[2] - eye[2]).powi(2)).sqrt();
                (dist <= 500.0).then(|| format!("{} {:.0} units away, {:.0}% open{}", d.name, dist, s.open * 100.0,
                    d.portal.map_or(String::new(), |p| format!(", portal {}", level.portals.portals[p].name))))
            })
            .collect();
        let depth_bits = {
            let mut bits = 0;
            unsafe { glGetIntegerv(GL_DEPTH_BITS, &mut bits) };
            bits
        };
        let report = DebugReport {
            map_path: &session.map_path, map_num: &session.map_num, level, eye, feet, yaw: pview.camera.yaw, pitch: pview.camera.pitch, dir,
            hit, first_person: toggles.first_person, third_person: toggles.third_person, textured: toggles.textured, room_debug: toggles.room_debug, show_portals: toggles.show_portals, show_collisions: toggles.show_collisions, sprite_mode: toggles.sprite_mode, depth_bits,
            fps, render_ms, culling: sim.culling, portal_paths: args.portal_paths, room: current_rooms.first().copied(), drawn: drawn_rooms, collision: &sim.players[me].collision_rooms, player_rooms: &sim.players[me].player_rooms, render_rooms: &sim.players[me].render_rooms, doors: &nearby_doors,
            terrorists: &sim.terrorist_report(me), log: overlay.recent_log.make_contiguous(),
        };
        let clipping = stem.is_some();
        if let Err(e) = std::fs::create_dir_all(stem.as_deref().and_then(|s| Path::new(s).parent()).unwrap_or(Path::new("debug"))) {
            eprintln!("debug dump failed: {e:#}");
        }
        match write_debug_dump(&report, &rgb, fw as u32, fh as u32, stem) {
            Ok(stem) if !clipping => println!("wrote {stem}.png and {stem}.txt"),
            Ok(_) => {}
            Err(e) => eprintln!("debug dump failed: {e:#}"),
        }
        if toggles.clip.as_ref().is_some_and(|(start, ..)| start.elapsed().as_secs_f32() >= CLIP_SECONDS) {
            if let Some((_, dir, n)) = toggles.clip.take() {
                println!("recorded {n} frames in {dir}");
            }
        }
        if args.dump && one_dump {
            return true;
        }
    }

    // Screenshot mode: capture after a couple of frames so textures are
    // uploaded and the context has settled, then exit.
    timing.frames_drawn += 1;
    // Timing for benchmarks (--walk N --hold X --max-fps 10000), after a warm-up
    if timing.frames_drawn > BENCH_WARMUP {
        timing.bench = (timing.bench.0 + 1, timing.bench.1 + render_ms as f64, timing.bench.2 + raw_dt as f64 * 1000.0);
    }
    // A replay ends when the recording does
    let replay_over = session.playback.as_ref().is_some_and(|p| p.finished());
    if replay_over && args.screenshot.is_none() {
        window.set_should_close(true);
    }
    if let Some(path) = &args.screenshot {
        let done = if session.playback.is_some() { replay_over } else { timing.frames_drawn >= args.walk.map_or(3, |n| n.max(3)) };
        if done {
            if timing.bench.0 > 0 {
                println!("timing over {} frames: render {:.3} ms, whole frame {:.3} ms", timing.bench.0, timing.bench.1 / timing.bench.0 as f64, timing.bench.2 / timing.bench.0 as f64);
            }
            let (fw, fh) = window.get_framebuffer_size();
            let rgb = read_framebuffer(fw, fh);
            match rogue_reborn::png::write_png(path, fw as u32, fh as u32, &rgb) {
                Ok(()) => {
                    println!("wrote {} ({fw}x{fh})", path.display());
                    // Live casings per owner, so the gate sees ejections that first person hides
                    let mut owners = std::collections::BTreeMap::<usize, usize>::new();
                    for c in scene.casings.iter() {
                        *owners.entry(c.owner).or_default() += 1;
                    }
                    let list: Vec<String> = owners.iter().map(|(o, n)| format!("{o}={n}")).collect();
                    println!("casings: {}", if list.is_empty() { "none".to_string() } else { list.join(" ") });
                }
                Err(e) => {
                    eprintln!("failed to write screenshot: {e}");
                    std::process::exit(1);
                }
            }
            return true;
        }
    }
    false
}
