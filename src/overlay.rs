use std::collections::VecDeque;
use std::path::Path;
use glfw::Key;
use rogue_reborn::lobby::{self, Chat, Gate};
use rogue_reborn::net::proto::{LobbyCommand, Setup};
use rogue_reborn::results::Picked;
use rogue_reborn::ui::Ui;
use crate::ui_gl::UiGl;

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
