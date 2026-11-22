use std::collections::VecDeque;
use std::path::Path;
use std::time::Instant;
use glfw::Key;
use rogue_reborn::lobby::{self, Chat, Gate};
use rogue_reborn::{grenade, map, sim};
use rogue_reborn::net::round::{Outcome, Phase, RoundState};
use rogue_reborn::net::watch::Pose;
use rogue_reborn::net::proto::{LobbyCommand, Setup};
use rogue_reborn::results::Picked;
use rogue_reborn::ui::Ui;
use crate::args::Args;
use crate::half_view_tan;
use crate::hud::{draw_hud, draw_debug_text};
use crate::session::Session;
use crate::state::{Timing, Toggles};
use crate::ui_gl::UiGl;
use crate::view::PlayerView;

// What is drawn over the world and answers the pointer and the keys: the lobby, the results, the
// chat, the kill feed and the lines for a debug dump
pub struct Overlay {
    // The lobby between rounds (see lobby.rs), drawn over everything with the original's art.
    // While it shows, the keys and mouse are its, and the pointer is free.
    pub ui: Ui,
    pub ui_gl: UiGl,
    pub gate: Gate<Key>,
    pub pointer_down: bool,
    // The chat, for the whole game, what has been typed for it to say, and the wheel turned
    // over the lobby since the last frame
    pub chat: Chat,
    // Whom the results screen shows, by a click on their row
    pub picked: Picked,
    pub typed: Vec<LobbyCommand>,
    pub wheel: f32,
    pub last_cursor_pos: Option<(f64, f64)>,
    pub recent_log: VecDeque<String>,
    // The latest kills, with when they leave the screen and their color
    pub feed: Vec<(f32, String, [f32; 3])>,
    pub level_name: String,
    pub pair_names: Vec<String>,
    // The mission's name for the results, as the original's mission file has it
    pub mission: String,
    pub uniform_names: [String; 2],
}

impl Overlay {
    pub fn new(joined: Option<&Setup>) -> Overlay {
        let ui = Ui::load(Path::new("data/font"));
        let ui_gl = UiGl::load(&ui, Path::new("data/bitmap"));
        let level_name = joined.map(|s| Path::new(&s.map).file_stem().map_or(s.map.clone(), |n| n.to_string_lossy().into_owned())).unwrap_or_default();
        let pair_names: Vec<String> = joined.map(|s| s.base_pairs.iter().map(|p| rogue_reborn::levels::game_text(&p.name).unwrap_or_else(|| p.name.clone())).collect()).unwrap_or_default();
        let mission = joined.and_then(|s| rogue_reborn::levels::mission_name(&s.map)).unwrap_or_else(|| level_name.clone());
        let uniform_names: [String; 2] = joined.map(|s| s.uniforms.clone().map(|u| lobby::uniform_name(&u))).unwrap_or_default();
        Overlay {
            ui,
            ui_gl,
            gate: Gate::default(),
            pointer_down: false,
            chat: Chat::default(),
            picked: Picked::default(),
            typed: Vec::new(),
            wheel: 0.0,
            last_cursor_pos: None,
            recent_log: VecDeque::new(),
            feed: Vec::new(),
            level_name,
            pair_names,
            mission,
            uniform_names,
        }
    }
}

// What the HUD and the debug text read besides the loop's structs
pub struct HudPass<'a> {
    pub args: &'a Args,
    pub level: &'a map::Map,
    pub sim: &'a sim::Sim,
    pub me: usize,
    pub game_time: f32,
    pub w: i32,
    pub h: i32,
    pub fps: f32,
    pub render_time: Instant,
    pub watch_pose: Option<Pose>,
    pub current_rooms: &'a [&'a str],
    pub drawn_rooms: &'a [String],
}

impl Overlay {
    // Returns the milliseconds the frame took to draw so far
    pub fn draw_hud_and_debug(&self, pass: HudPass, toggles: &Toggles, timing: &Timing, pview: &PlayerView, session: &Session) -> f32 {
        let overlay = self;
        let HudPass { args, level, sim, me, game_time, w, h, fps, render_time, watch_pose, current_rooms, drawn_rooms } = pass;
        // Draw simple debug text overlay (upper-right)
        let render_ms = (render_time.elapsed().as_secs_f64() * 1000.0) as f32;
        let level_text = if !level.planning_levels.levels.is_empty() {
            format!("LEVEL: {}", toggles.planning_level_index)
        } else {
            String::from("LEVEL: -")
        };
        // Fixed steps with a screenshot show fixed numbers, so the picture repeats
        let steady = timing.fixed_step && args.screenshot.is_some();
        let fps_text = format!("FPS: {:.1}", if steady { 0.0 } else { fps });
        let render_text = format!("RENDER: {:.2} MS", if steady { 0.0 } else { render_ms });
        if toggles.first_person {
            let mut status = vec![
                format!("KILLS: {}", sim.players[me].kills),
                if sim.players[me].equipped_grenade { format!("GRENADES: {}", sim.players[me].grenades_left) }
                    else if sim.players[me].reload_until.is_some() { "RELOADING".to_string() } else { format!("AMMO: {}/{}", sim.players[me].ammo, sim.players[me].reserve) },
            ];
            if let Some(from) = sim.players[me].charge_from {
                status.insert(0, format!("POWER: {:.0}", ((game_time - from) / grenade::CHARGE_SECONDS).clamp(0.0, 1.0) * 100.0));
            }
            if let Some(hp) = sim.players[me].target_hp {
                status.insert(0, format!("TARGET: {hp:.0}"));
            }
            if session.net.is_some() {
                let p = &sim.players[me];
                status.insert(0, if p.respawn_at.is_some() { "DEAD".to_string() } else { format!("HEALTH: {:.0}", p.hp.ceil()) });
                status.insert(1, format!("DEATHS: {}", p.deaths));
            }
            // Watching, who it is and how they are, in place of the player's own
            if pview.watch.watching() {
                status = vec![match pview.watch.target() {
                    Some(t) => format!("WATCHING {}  HEALTH: {:.0}", session.net.as_ref().map_or_else(|| format!("PLAYER {}", t + 1), |(c, _)| c.name_of(t)), sim.players[t].hp.ceil()),
                    None if watch_pose.is_some() => "WATCHING".to_string(),
                    None => "NO ONE TO WATCH".to_string(),
                }];
            }
            let round = session.net.as_ref().map(|(c, _)| c.round).unwrap_or_default();
            if round.phase != Phase::Open {
                status.insert(0, format!("ROUND {}  BLUE {}  GOLD {}", round.number.max(1), round.wins[0], round.wins[1]));
                if let (Phase::Live, Some(ends)) = (round.phase, round.ends) {
                    let left = (ends - game_time).max(0.0).ceil() as u32;
                    status.insert(1, format!("TIME: {}:{:02}", left / 60, left % 60));
                }
            }
            let results = session.net.as_ref().and_then(|(c, _)| c.results.as_ref());
            let winner = results.and_then(|r| r.rows.get(r.winner? as usize)).map(|row| row.name.as_str());
            let banner = round_banner(&round, game_time, winner);
            let msg = if game_time < sim.players[me].message.1 { sim.players[me].message.0.as_str() } else { banner.as_str() };
            let marker = game_time < pview.hit_marker_until;
            let lines: Vec<(String, [f32; 3])> = overlay.feed.iter().map(|(_, text, color)| (text.clone(), *color)).collect();
            let dead = session.net.is_some() && sim.players[me].respawn_at.is_some();
            let meter = session.net.as_ref().filter(|_| toggles.lag_meter).map(|(client, _)| (&client.meter, session.net_clock.elapsed().as_secs_f64()));
            // The crosshair opens as wide as the gun's spread, in the world view
            let p = &sim.players[me];
            let spread = if p.equipped_grenade { 0.0 } else { p.aim.angle.tan() / half_view_tan() * h as f32 / 2.0 };
            // Watching, none of the player's own: no crosshair, red, whiteout or daze
            let own = watch_pose.is_none();
            let (veil, daze) = if own { (pview.veil, pview.concussion.daze()) } else { (0.0, 0.0) };
            draw_hud(w, h, &status, msg, marker && own, spread, veil, daze, &lines, dead && own, own, meter);
        }
        let rooms_text = if sim.culling {
            format!("ROOM: {} DRAWN: {}", current_rooms.join(" "), drawn_rooms.join(" "))
        } else {
            format!("ROOM: {} (CULLING OFF)", current_rooms.join(" "))
        };
        let demo_text = match (&session.recorder, &session.playback) {
            (Some(r), _) => format!("REC {:.0} S", r.ticks() as f32 * sim::TICK),
            (_, Some(p)) => {
                let (done, all) = p.progress();
                format!("DEMO {:.0}/{:.0} S", done as f32 * sim::TICK, all as f32 * sim::TICK)
            }
            _ => String::new(),
        };
        let net_text = session.net.as_ref().map_or(String::new(), |(c, _)| format!("NET: {:?} {} PLAYERS", c.status, sim.players.iter().filter(|p| p.active).count()).to_uppercase());
        let mut debug_lines: Vec<&str> = vec![&level_text, &fps_text, &render_text];
        if !net_text.is_empty() {
            debug_lines.push(&net_text);
        }
        if toggles.room_debug {
            debug_lines.push(&rooms_text);
        }
        if !demo_text.is_empty() {
            debug_lines.push(&demo_text);
        }
        // With --cinematic, captured pictures leave it out; the dump's text has it
        if !(args.cinematic && (args.screenshot.is_some() || toggles.dump_requested || toggles.clip.is_some())) {
            draw_debug_text(w, h, &debug_lines);
        }
        render_ms
    }
}

#[cfg(test)]
mod banner_tests {
    use super::*;

    #[test]
    fn the_last_one_standing_is_named_as_the_results_have_them() {
        let r = RoundState { phase: Phase::Results, number: 1, ends: None, wins: [0, 0], last: Some(Outcome::Survived(0)) };
        // Whoever has slot 0 now, the banner names the winner the results kept
        assert_eq!(round_banner(&r, 0.0, Some("OriginalWinner")), "OriginalWinner WINS");
        // Before the results come, the slot
        assert_eq!(round_banner(&r, 0.0, None), "PLAYER 1 WINS");
    }
}

/// The line on screen for where the rounds are, or none while one is played. `winner` is the
/// last one standing in Survival, named as the round's results have them, whoever has their
/// slot by now.
fn round_banner(r: &RoundState, now: f32, winner: Option<&str>) -> String {
    let left = r.ends.map_or(0.0, |e| (e - now).max(0.0));
    match (r.phase, r.last) {
        (Phase::Open | Phase::Live, _) => String::new(),
        (Phase::Lobby, _) => "WAITING FOR THE ROUND TO START".to_string(),
        (Phase::Countdown, _) => format!("ROUND {} STARTS IN {}", r.number + 1, left.ceil() as u32),
        (Phase::Loading, _) => "WAITING FOR EVERYONE TO LOAD".to_string(),
        (Phase::Hold, _) => "GET READY".to_string(),
        (Phase::Results, Some(Outcome::Won(s))) => format!("{} WINS", format!("{s:?}").to_uppercase()),
        (Phase::Results, Some(Outcome::Survived(p))) => format!("{} WINS", winner.map_or_else(|| format!("PLAYER {}", p + 1), str::to_string)),
        (Phase::Results, Some(Outcome::Draw)) => "EVERYBODY LOST".to_string(),
        (Phase::Results, Some(Outcome::CalledOff)) => "ROUND CALLED OFF".to_string(),
        // The results screen is up
        (Phase::Results, None) | (Phase::Debrief, _) => String::new(),
    }
}
