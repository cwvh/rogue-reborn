use rogue_reborn::{blastlight, casings, glass, impact, lobby, sim, sound};
use rogue_reborn::impact::Surface;
use rogue_reborn::net::watch::Pose;
use rogue_reborn::util::unit3;
use crate::NPC_OWNERS;
use crate::args::Args;
use crate::audio::Audio;
use crate::camera::listener_of;
use crate::overlay::Overlay;
use crate::render::assets::{Assets, SCORCH_VARIANTS, HOLE_VARIANTS};
use crate::scene::Scene;
use crate::view::PlayerView;

const MAX_DECALS: usize = 300;
/// How many of the latest log lines a debug dump shows
const RECENT_LOG: usize = 60;

// What the events pass reads besides the loop's structs
pub struct EventPass<'a> {
    pub args: &'a Args,
    pub assets: &'a Assets,
    pub me: usize,
    pub game_time: f32,
    pub watch_pose: Option<Pose>,
    pub player_ejector: Option<&'a casings::Ejector>,
}

pub fn handle_events(events: Vec<sim::Event>, pass: EventPass, sim: &mut sim::Sim, audio: &mut Audio, pview: &mut PlayerView,
                     overlay: &mut Overlay, scene: &mut Scene) {
    let EventPass { args, assets, me, game_time, watch_pose, player_ejector } = pass;
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
}
