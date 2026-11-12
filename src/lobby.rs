//! The lobby screen, which players wait on between rounds (see "Lobby" in docs/multiplayer.md).
//!
//! It shows who is playing on which side and who is ready, and the session's settings. Each
//! player picks a side and says ready, and the host changes the settings and starts the
//! countdown. Under the settings is the chat, a line at a time as in IRC: what players say,
//! and what happens, such as the countdown. It is laid out as the original's session screen,
//! from what the client knows, and gives back the commands its clicks and typing make
//! (`LobbyCommand`), for the client to send.

use std::collections::{HashSet, VecDeque};
use std::hash::Hash;

use crate::net::proto::{CHAT_LENGTH, LobbyCommand, RoundResults, Session, chat_text};
use crate::net::round::{Outcome, Phase, RoundState};
use crate::sim::{MAX_PLAYERS, Side};
use crate::ui::font::Font;
use crate::ui::{Art, Color, FontId, Look, Rect, Ui};

/// The round lengths the host can choose from (seconds), shortest first, and none
pub const LENGTHS: [Option<u16>; 8] = [Some(60), Some(120), Some(180), Some(240), Some(300), Some(420), Some(600), None];

/// Someone in the game: their slot, join number (which no one else in the slot has), side and
/// name
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub slot: usize,
    pub joined: u32,
    pub side: Option<Side>,
    pub name: String,
}

/// What the client knows that the screen shows
pub struct LobbyView<'a> {
    pub round: RoundState,
    /// The server's clock now, for the countdown
    pub time: f32,
    pub session: Session,
    pub me: usize,
    /// Everyone in the game
    pub players: &'a [Player],
    /// Whether this player has continued from the round's results
    pub continued: bool,
    /// The slots of those who have said ready
    pub ready: &'a [usize],
    /// The last round's results, from its end until the lobby
    pub results: Option<&'a RoundResults>,
    pub level: &'a str,
    /// Each pair of bases, by name
    pub base_pairs: &'a [String],
    /// What each side wears
    pub uniforms: [&'a str; 2],
}

impl LobbyView<'_> {
    fn host(&self) -> bool {
        self.session.host == Some(self.me as u8)
    }

    fn team_game(&self) -> bool {
        self.players.iter().any(|p| p.side.is_some())
    }

    fn side(&self) -> Option<Side> {
        self.players.iter().find(|p| p.slot == self.me).and_then(|p| p.side)
    }

    fn name_of(&self, slot: usize) -> String {
        self.players.iter().find(|p| p.slot == slot).map_or_else(|| name(slot), |p| p.name.clone())
    }

    /// Whether there is anyone to play against: someone on each side, or two players
    fn two_sides(&self) -> bool {
        match self.team_game() {
            true => Side::BOTH.iter().all(|&s| self.players.iter().any(|p| p.side == Some(s))),
            false => self.players.len() >= 2,
        }
    }

    fn base_pair(&self, i: u8) -> String {
        self.base_pairs.get(i as usize).cloned().unwrap_or_else(|| "-".to_string())
    }

    /// Whole seconds left of the countdown, while it runs
    fn count(&self) -> Option<u32> {
        (self.round.phase == Phase::Countdown).then(|| self.round.ends.map_or(0.0, |e| (e - self.time).max(0.0)).ceil() as u32)
    }
}

/// A player as the screen names them when it does not know their name
pub fn name(slot: usize) -> String {
    format!("PLAYER {}", slot + 1)
}

pub fn side_name(side: Side) -> &'static str {
    match side {
        Side::Blue => "Blue",
        Side::Gold => "Gold",
    }
}

/// A uniform as the original names it: the two worn unless the server says otherwise by name,
/// and any other by its file
pub fn uniform_name(model: &str) -> String {
    match crate::net::proto::UNIFORMS.iter().position(|u| u.eq_ignore_ascii_case(model)) {
        Some(0) => "Blue Medium".to_string(),
        Some(_) => "Desert 1 Medium".to_string(),
        None => std::path::Path::new(model).file_stem().map_or(model.to_string(), |s| s.to_string_lossy().into_owned()),
    }
}

/// A round's length as the screen shows it
pub fn length_text(t: Option<u16>) -> String {
    match t {
        Some(t) => format!("{}:{:02}", t / 60, t % 60),
        None => "No limit".to_string(),
    }
}

/// The next length up or down from `now`, which need not be one of `LENGTHS`, if there is one
fn step_length(now: Option<u16>, up: bool) -> Option<Option<u16>> {
    let long = |t: Option<u16>| t.map_or(u32::MAX, u32::from);
    match up {
        true => LENGTHS.iter().find(|&&l| long(l) > long(now)).copied(),
        false => LENGTHS.iter().rev().find(|&&l| long(l) < long(now)).copied(),
    }
}

// The screen's colors, after the original's
pub const BACK: Color = [0.01, 0.02, 0.04, 1.0];
pub const LABEL: Color = [0.04, 0.16, 0.24, 1.0];
const LABEL_EDGE: Color = [0.16, 0.42, 0.55, 1.0];
pub const LABEL_TEXT: Color = [0.3, 0.75, 0.92, 1.0];
pub const FIELD: Color = [0.01, 0.02, 0.03, 1.0];
pub const FIELD_EDGE: Color = [0.85, 0.67, 0.15, 1.0];
pub const FIELD_TEXT: Color = [1.0, 1.0, 1.0, 1.0];
const HEADER: Color = [0.08, 0.3, 0.5, 1.0];
const GOLD_TEXT: Color = [0.95, 0.78, 0.2, 1.0];
pub const DIM_TEXT: Color = [0.45, 0.5, 0.52, 1.0];
/// The chat's lines for what happens, quieter than what players say
const NEWS_TEXT: Color = [0.42, 0.56, 0.64, 1.0];
pub const WHITE: Color = [1.0, 1.0, 1.0, 1.0];
pub const BLUE_BAR: Color = [0.15, 0.4, 0.7, 1.0];
pub const GOLD_BAR: Color = [0.75, 0.6, 0.15, 1.0];

// Where things are, on the 640 x 480 canvas
/// The player list: a header level with NAME, then a row each from level with SPECIALTY. A
/// row is three boxes: the ping, the ready light and the name.
const HEAD_Y: f32 = 42.0;
const ROWS_Y: f32 = 60.0;
const ROW: f32 = 19.0;
const ROW_H: f32 = 17.0;
/// Room for everyone a game can have, as the original has
const ROWS: usize = MAX_PLAYERS;
const PING_X: f32 = 25.0;
const LIGHT_X: f32 = 43.0;
const NAME_X: f32 = 61.0;
const NAME_W: f32 = 126.0;
/// The states the ready lights and the header are drawn in, as the original's look
const LIGHT: usize = 0;
const HEAD: usize = 1;
/// Below the last row, where the original has them
pub const BLUE: Rect = Rect::new(25.0, 365.0, 162.0, 16.0);
pub const GOLD: Rect = Rect::new(25.0, 383.0, 162.0, 16.0);
/// The settings' boxes (`Art::Panel`): five rows of the player's own, then seven of the session's
const PANEL: Rect = Rect::new(190.0, 41.0, 425.0, 233.0);
/// The chat's box (`Art::ChatBox`), the lines in it, and the line being typed
const CHAT: Rect = Rect::new(191.0, 275.0, 423.0, 122.0);
const LINES: Rect = Rect::new(195.0, 278.0, 400.0, 97.0);
const LINE: f32 = 14.0;
pub const TYPING: Rect = Rect::new(201.0, 381.0, 388.0, 16.0);
pub const SCROLL_UP: Rect = Rect::new(598.0, 285.0, 16.0, 14.0);
pub const SCROLL_DOWN: Rect = Rect::new(598.0, 355.0, 16.0, 14.0);
/// What the player can do now, across the bottom
pub const STATUS: Rect = Rect::new(25.0, 401.0, 590.0, 13.0);
pub const READY: Rect = Rect::new(30.0, 428.0, 110.0, 26.0);
pub const START: Rect = Rect::new(497.0, 428.0, 110.0, 26.0);

/// The rows of the settings: the player's name, kit and so on, then the type, rules, map,
/// insertions, length and the uniforms
pub const INSERTIONS: usize = 8;
pub const LENGTH: usize = 9;
const PLAYER_OPTIONS: usize = 0;
const SESSION_OPTIONS: usize = 5;
pub const FRIENDLY_FIRE: Rect = cells(6)[2];

/// A row of the settings: its label, its field, and its cell in the options on the right
const fn cells(row: usize) -> [Rect; 3] {
    let y = PANEL.y + if row < 5 { 1.0 + 18.0 * row as f32 } else { 108.0 + 18.0 * (row - 5) as f32 };
    [Rect::new(PANEL.x + 11.0, y, 78.0, 16.0), Rect::new(PANEL.x + 90.0, y, 159.0, 16.0), Rect::new(PANEL.x + 255.0, y, 159.0, 16.0)]
}

/// A setting's field
pub fn field(row: usize) -> Rect {
    cells(row)[1]
}

/// The arrows that step a setting's choice on (up) and back (down)
pub fn arrows(row: usize) -> [Rect; 2] {
    let f = field(row);
    [Rect::new(f.x + f.w - 32.0, f.y + 2.0, 15.0, 12.0), Rect::new(f.x + f.w - 17.0, f.y + 2.0, 15.0, 12.0)]
}

// Each control's identity
const ID_BLUE: u32 = 1;
const ID_GOLD: u32 = 2;
const ID_READY: u32 = 3;
const ID_START: u32 = 4;
const ID_FRIENDLY_FIRE: u32 = 5;
const ID_SCROLL_UP: u32 = 6;
const ID_SCROLL_DOWN: u32 = 7;
const ID_CHOICE: u32 = 10;

/// Lay out the lobby, and say which commands this frame's clicks made
pub fn lobby(ui: &mut Ui, v: &LobbyView, chat: &mut Chat) -> Vec<LobbyCommand> {
    let mut commands = Vec::new();
    let lobby = v.round.phase == Phase::Lobby;
    let host = v.host();
    let settings = host && lobby;

    let screen = Rect::new(0.0, 0.0, 640.0, 480.0);
    ui.fill(screen, BACK);
    ui.tile(Art::Grid, screen, BACK);

    // The players: Blue's, then Gold's, then anyone with no side. The ping is not known yet,
    // so its boxes are empty.
    ui.art(Art::HeadPing, HEAD, Rect::new(PING_X, HEAD_Y, 15.0, 16.0), LABEL);
    ui.art(Art::HeadReady, HEAD, Rect::new(LIGHT_X, HEAD_Y, 15.0, 16.0), LABEL);
    ui.art(Art::HeadName, HEAD, Rect::new(NAME_X, HEAD_Y, NAME_W, 16.0), LABEL);
    let mut players: Vec<&Player> = v.players.iter().collect();
    players.sort_by_key(|p| (p.side.map_or(2, |s| s.index()), p.slot));
    for (i, p) in players.iter().enumerate().take(ROWS) {
        let (slot, side) = (p.slot, p.side);
        let y = ROWS_Y + i as f32 * ROW;
        ui.art(Art::PingBox, 0, Rect::new(PING_X, y, 15.0, ROW_H), FIELD);
        let (light, fallback) = match () {
            _ if v.session.host == Some(slot as u8) => (Art::Host, LABEL_TEXT),
            _ if v.ready.contains(&slot) => (Art::Ready, [0.2, 0.8, 0.2, 1.0]),
            _ => (Art::NotReady, [0.6, 0.1, 0.1, 1.0]),
        };
        ui.art(light, LIGHT, Rect::new(LIGHT_X, y, 15.0, ROW_H), fallback);
        let bar = Rect::new(NAME_X, y, NAME_W, ROW_H);
        let (art, fallback) = match side {
            Some(Side::Blue) => (Art::NameBlue, BLUE_BAR),
            Some(Side::Gold) => (Art::NameGold, GOLD_BAR),
            None => (Art::NameNone, LABEL),
        };
        // Dark, but the player's own, which is lit
        ui.art(art, (slot == v.me) as usize, bar, fallback);
        ui.centered_clipped(&p.name, FontId::Text, bar, 4.0, WHITE);
    }

    // Choosing a side, in the lobby of a team game. The player's own is pressed in, and the
    // other is dark, with a ⊘, while it cannot be had.
    if v.team_game() {
        for (side, rect, id, art, fallback) in [(Side::Blue, BLUE, ID_BLUE, Art::SideBlue, BLUE_BAR), (Side::Gold, GOLD, ID_GOLD, Art::SideGold, GOLD_BAR)] {
            let mine = v.side() == Some(side);
            let (look, clicked) = ui.control(id, rect, lobby && !mine);
            let shown = if mine { 2 } else { state(look) };
            ui.art(art, shown, rect, fallback);
            ui.centered(if side == Side::Blue { "BLUE" } else { "GOLD" }, FontId::Text, rect, if shown == 3 { DIM_TEXT } else { WHITE });
            if clicked {
                commands.push(LobbyCommand::Side(side));
            }
        }
    }

    // The settings: the player's own, which are fixed for now, then what is played and how.
    // The host steps through the choices.
    ui.art(Art::Panel, 0, PANEL, LABEL);
    let rows: [(&str, String, usize); 12] = [
        ("NAME", v.name_of(v.me), 0),
        ("SPECIALTY", String::new(), 0),
        ("FIREARMS", "HK MP5K".to_string(), 0),
        ("AMMO", "9mm (FMJ)".to_string(), 0),
        ("KIT ITEMS", "Frag Grenade".to_string(), 0),
        ("TYPE", "Adversarial".to_string(), 0),
        ("RULES", if v.team_game() { "Team Survival" } else { "Survival" }.to_string(), 0),
        ("MAP", v.level.to_string(), 0),
        ("INSERTIONS", v.base_pair(v.session.base_pair), v.base_pairs.len()),
        ("LENGTH", length_text(v.session.time_limit), LENGTHS.len()),
        ("BLUE", v.uniforms[0].to_string(), 0),
        ("GOLD", v.uniforms[1].to_string(), 0),
    ];
    for (row, (label, value, choices)) in rows.into_iter().enumerate() {
        let [l, f, _] = cells(row);
        ui.left(label, FontId::Text, l, 4.0, LABEL_TEXT);
        // The values are edged in gold, whoever may change them, as in the original
        ui.frame(f, FIELD_EDGE);
        ui.left(&value, FontId::Text, f, 4.0, if row == LENGTH { GOLD_TEXT } else { FIELD_TEXT });
        if choices < 2 {
            continue;
        }
        // Up to the next and down to the one before: the lengths stop at either end, and the
        // insertions, which have no order, go round
        for (k, (rect, art)) in arrows(row).into_iter().zip([Art::Up, Art::Down]).enumerate() {
            let up = k == 0;
            let next = match row {
                INSERTIONS => {
                    let now = v.session.base_pair as usize;
                    let next = if up { now + 1 } else { now + choices - 1 };
                    Some(LobbyCommand::BasePair((next % choices) as u8))
                }
                _ => step_length(v.session.time_limit, up).map(LobbyCommand::TimeLimit),
            };
            let (look, clicked) = ui.control(ID_CHOICE + 2 * row as u32 + k as u32, rect, settings && next.is_some());
            ui.art(art, state(look), rect, LABEL_EDGE);
            if let (true, Some(next)) = (clicked, next) {
                commands.push(next);
            }
        }
    }
    ui.left("PLAYER OPTIONS", FontId::Text, cells(PLAYER_OPTIONS)[2], 4.0, LABEL_TEXT);
    ui.left("SESSION OPTIONS", FontId::Text, cells(SESSION_OPTIONS)[2], 4.0, LABEL_TEXT);
    let (look, clicked) = ui.control(ID_FRIENDLY_FIRE, FRIENDLY_FIRE, settings);
    let on = v.session.friendly_fire;
    ui.left("FRIENDLY FIRE", FontId::Text, FRIENDLY_FIRE, 4.0, if on { GOLD_TEXT } else { LABEL_TEXT });
    let boxed = Rect::new(FRIENDLY_FIRE.x + FRIENDLY_FIRE.w - 16.0, FRIENDLY_FIRE.y + 1.0, 15.0, 14.0);
    ui.art(Art::CheckBox, if on { 1 } else if look == Look::Disabled { 3 } else { 0 }, boxed, if on { FIELD_EDGE } else { FIELD });
    if clicked {
        commands.push(LobbyCommand::FriendlyFire(!on));
    }

    chat_box(ui, v.time, chat, &LOBBY_CHAT);

    // What the player can do now
    let hint = match v.round.phase {
        Phase::Countdown if host => "The round is about to start. STOP holds it.",
        Phase::Countdown => "The round is about to start.",
        _ if host && !v.two_sides() => if v.team_game() { "Each side needs someone before you can start." } else { "You need someone to play against." },
        _ if host => "You are the host. Start the round when everyone is ready.",
        _ if v.session.host.is_some() => "Waiting for the host to start the round.",
        _ => "The round starts when both sides have someone.",
    };
    ui.fill(STATUS, LABEL);
    ui.left(hint, FontId::Text, STATUS, 4.0, LABEL_TEXT);
    let esc = "ESC LEAVES THE GAME";
    let w = ui.text.width(esc);
    ui.left(esc, FontId::Text, STATUS, STATUS.w - w - 4.0, DIM_TEXT);

    // Ready, and the host's start and stop
    let ready = v.ready.contains(&v.me);
    let (look, clicked) = ui.control(ID_READY, READY, lobby);
    button(ui, READY, look, ready, "READY");
    if clicked {
        commands.push(LobbyCommand::Ready(!ready));
    }
    if host {
        let counting = v.round.phase == Phase::Countdown;
        let (look, clicked) = ui.control(ID_START, START, (lobby && v.two_sides()) || counting);
        button(ui, START, look, false, if counting { "STOP" } else { "START" });
        if clicked {
            commands.push(if counting { LobbyCommand::Stop } else { LobbyCommand::Start });
        }
    }
    commands
}

/// The art's state for how a control looks: up, lit, pressed and dark
fn state(look: Look) -> usize {
    match look {
        Look::Normal => 0,
        Look::Hover => 1,
        Look::Down => 2,
        Look::Disabled => 3,
    }
}

/// A button with its label, lit while `on`
pub fn button(ui: &mut Ui, rect: Rect, look: Look, on: bool, text: &str) {
    let state = match look {
        Look::Normal if on => 1,
        _ => state(look),
    };
    ui.art(Art::Button, state, rect, if state == 1 { HEADER } else { LABEL });
    ui.frame(rect, LABEL_EDGE);
    ui.centered(text, FontId::Text, rect, if look == Look::Disabled { DIM_TEXT } else { WHITE });
}

/// Where a chat box's parts are: its art and the box it fills, where the lines go, the line
/// being typed, and the scroll arrows
pub struct ChatPlace {
    pub art: Art,
    pub frame: Rect,
    pub lines: Rect,
    pub typing: Rect,
    pub up: Rect,
    pub down: Rect,
}

const LOBBY_CHAT: ChatPlace = ChatPlace { art: Art::ChatBox, frame: CHAT, lines: LINES, typing: TYPING, up: SCROLL_UP, down: SCROLL_DOWN };

/// The chat's box: the newest lines that fit, or older ones as it is scrolled, the arrows and
/// wheel to scroll it, and the line being typed. `time` blinks the caret.
pub fn chat_box(ui: &mut Ui, time: f32, chat: &mut Chat, place: &ChatPlace) {
    let ChatPlace { art, frame, lines, typing, up, down } = *place;
    ui.art(art, 0, frame, FIELD);
    let wheel = ui.pointer().wheel;
    if wheel != 0.0 && frame.contains(ui.pointer().at) {
        chat.scroll(wheel.round() as i32);
    }
    for (rect, art, id, by) in [(up, Art::ScrollUp, ID_SCROLL_UP, 1), (down, Art::ScrollDown, ID_SCROLL_DOWN, -1)] {
        let can = if by > 0 { chat.scroll < chat.lines.len().saturating_sub(1) } else { chat.scroll > 0 };
        let (look, clicked) = ui.control(id, rect, can);
        ui.art(art, state(look), rect, LABEL_EDGE);
        if clicked {
            chat.scroll(by);
        }
    }
    // The knob is as far up as the box is scrolled back
    let track = (up.y + up.h + 1.0, down.y - 1.0 - 14.0);
    let back = chat.scroll as f32 / chat.lines.len().saturating_sub(1).max(1) as f32;
    ui.art(Art::ScrollKnob, 0, Rect::new(up.x, (track.1 - back * (track.1 - track.0)).round(), 15.0, 14.0), LABEL_EDGE);

    let fit = (lines.h / LINE) as usize;
    let mut rows: Vec<(String, Color)> = Vec::new();
    for line in chat.lines.range(..chat.lines.len() - chat.scroll).rev() {
        let (text, color) = match &line.who {
            Some(who) => (format!("{}: {}", speaker(who, line.dead), line.text), GOLD_TEXT),
            None => (line.text.clone(), NEWS_TEXT),
        };
        rows.extend(wrap(&ui.text, &text, lines.w).into_iter().rev().map(|r| (r, color)));
        if rows.len() >= fit {
            break;
        }
    }
    for (i, (text, color)) in rows.iter().take(fit).enumerate() {
        ui.text(text, FontId::Text, [lines.x, lines.y + lines.h - (i + 1) as f32 * LINE], *color);
    }

    // What is being typed, its end if it is too long to show, and a blinking caret
    let caret = if (time * 2.0).rem_euclid(2.0) < 1.0 { "_" } else { "" };
    let room = typing.w - 8.0 - ui.text.width("_");
    let mut shown = chat.typing.as_str();
    while ui.text.width(shown) > room {
        let mut chars = shown.chars();
        chars.next();
        shown = chars.as_str();
    }
    ui.left(&format!("{shown}{caret}"), FontId::Text, typing, 3.0, FIELD_TEXT);
}

/// Text in lines no wider than `width`, broken between words, or within a word too long for one
fn wrap(font: &Font, text: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let joined = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if font.width(&joined) <= width {
            line = joined;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        for c in word.chars() {
            line.push(c);
            if font.width(&line) > width && line.chars().count() > 1 {
                line.pop();
                lines.push(std::mem::replace(&mut line, c.to_string()));
            }
        }
    }
    lines.push(line);
    lines
}

/// The most lines the chat keeps
const KEPT: usize = 200;

/// The results screen's first line, in the original's words
pub const CONTINUE_LINE: &str = "Press the 'continue' button when you are ready to start next game";

/// A line of the chat: what a player said, or what happened (with no one's name). `dead` if
/// they said it dead in a round.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub who: Option<String>,
    pub text: String,
    pub dead: bool,
}

/// Who said a line, as the chat and the feed show them
pub fn speaker(name: &str, dead: bool) -> String {
    if dead { format!("{name} (dead)") } else { name.to_string() }
}

/// The chat, kept for the whole game: what players say (`Event::Chat`), and what happens, which
/// it sees by watching the game change, and the line the player is typing
#[derive(Default)]
pub struct Chat {
    lines: VecDeque<Line>,
    /// How many lines back from the newest the box is scrolled
    scroll: usize,
    typing: String,
    before: Option<Before>,
    /// The round whose result the chat has told
    told: u16,
}

/// What the chat saw of the game last, to tell what has changed
#[derive(Clone)]
struct Before {
    phase: Phase,
    count: Option<u32>,
    session: Session,
    players: Vec<Player>,
    continued: bool,
}

impl Chat {
    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.lines.iter()
    }

    fn add(&mut self, text: impl Into<String>) {
        self.line(Line { who: None, text: text.into(), dead: false });
    }

    fn line(&mut self, line: Line) {
        self.lines.push_back(line);
        // Scrolled back, the box stays on what it shows
        if self.scroll > 0 {
            self.scroll += 1;
        }
        while self.lines.len() > KEPT {
            self.lines.pop_front();
        }
        self.scroll = self.scroll.min(self.lines.len().saturating_sub(1));
    }

    /// A player said something (`Event::Chat`), named as they were when they said it
    pub fn said(&mut self, name: &str, text: &str, dead: bool) {
        self.line(Line { who: Some(name.to_string()), text: text.to_string(), dead });
    }

    /// Note what has changed since the last look: players coming, going and changing sides,
    /// the countdown, how the round ended, and the host's changes to the settings. The first
    /// look notes nothing.
    pub fn watch(&mut self, v: &LobbyView) {
        // The client's clock is eased toward the server's, so it can go back a little: the
        // countdown only ever counts down
        let count = match (self.before.as_ref().and_then(|b| b.count), v.count()) {
            (Some(was), Some(n)) => Some(was.min(n)),
            (_, n) => n,
        };
        let now = Before { phase: v.round.phase, count, session: v.session, players: v.players.to_vec(), continued: v.continued };
        let Some(before) = self.before.replace(now) else {
            // A result already shown when the client came is old news
            self.told = v.results.map_or(0, |r| r.number);
            return;
        };

        // By occupant, so that someone new in a slot is one going and another coming
        let same = |a: &Player, b: &Player| (a.slot, a.joined) == (b.slot, b.joined);
        for p in &before.players {
            if !v.players.iter().any(|q| same(p, q)) {
                self.add(format!("{} has disconnected", p.name));
            }
        }
        for p in v.players {
            match before.players.iter().find(|q| same(p, q)) {
                None => self.add(format!("{} has joined{}", p.name, p.side.map_or(String::new(), |s| format!(" {}", side_name(s))))),
                Some(was) if was.side != p.side => {
                    if let Some(s) = p.side {
                        self.add(format!("{} went to {}", p.name, side_name(s)));
                    }
                }
                _ => {}
            }
        }
        let s = (before.session, v.session);
        if let Some(host) = s.1.host.filter(|_| s.0.host != s.1.host) {
            self.add(format!("{} is the host", v.name_of(host as usize)));
        }
        if s.0.base_pair != s.1.base_pair {
            self.add(format!("INSERTIONS changed to {}", v.base_pair(s.1.base_pair)));
        }
        if s.0.time_limit != s.1.time_limit {
            self.add(format!("LENGTH changed to {}", length_text(s.1.time_limit)));
        }
        if s.0.friendly_fire != s.1.friendly_fire {
            self.add(format!("FRIENDLY FIRE changed to {}", if s.1.friendly_fire { "on" } else { "off" }));
        }

        // The countdown in the original's words, and the round's end
        match (before.phase, v.round.phase) {
            (Phase::Countdown, Phase::Countdown) => {}
            (_, Phase::Countdown) => self.add("--- Preparing to begin mission ---"),
            (Phase::Countdown, Phase::Lobby) => self.add("--- Start suspended ---"),
            (Phase::Countdown, _) => self.add("--- Away we go ---"),
            (was, Phase::Debrief) if was != Phase::Debrief => self.add(CONTINUE_LINE),
            _ => {}
        }
        if let Some(n) = count.filter(|&n| n > 0 && before.count != Some(n)) {
            self.add(format!("Starting mission in {n} second{}", if n == 1 { "" } else { "s" }));
        }
        if v.continued && !before.continued && v.round.phase == Phase::Debrief {
            self.add("Ready to continue");
        }
        // How the round ended, once, from its results, which keep the winner as they were
        // whoever has their slot now, and come however late the client sees them
        if let Some(results) = v.results.filter(|r| r.number != self.told) {
            self.told = results.number;
            self.ended(v, results);
        }
    }

    fn ended(&mut self, v: &LobbyView, results: &RoundResults) {
        let n = results.number;
        if results.outcome == Outcome::CalledOff {
            return self.add(format!("Round {n} was called off"));
        }
        let team_game = results.rows.iter().any(|r| r.side.is_some());
        let score = if team_game { format!(" (Blue {}, Gold {})", v.round.wins[0], v.round.wins[1]) } else { String::new() };
        self.add(format!("Round {n}: {}{score}", crate::results::result_text(results)));
    }

    /// A key typed on the line, if the game's fonts have it and the line has room
    pub fn typed(&mut self, c: char) {
        if matches!(c, ' '..='~' | '\u{a0}'..='\u{ff}') && self.typing.chars().count() < CHAT_LENGTH {
            self.typing.push(c);
        }
    }

    pub fn erase(&mut self) {
        self.typing.pop();
    }

    pub fn typing(&self) -> &str {
        &self.typing
    }

    /// Enter: what was typed, to say, if there is anything. The box goes to the newest line.
    pub fn send(&mut self) -> Option<LobbyCommand> {
        let text = chat_text(&std::mem::take(&mut self.typing))?;
        self.scroll = 0;
        Some(LobbyCommand::Say(text))
    }

    /// Escape: the line is cleared. Says whether there was anything on it.
    pub fn clear(&mut self) -> bool {
        !std::mem::take(&mut self.typing).is_empty()
    }

    /// Scroll back (more) or on (less), by lines
    pub fn scroll(&mut self, by: i32) {
        self.scroll = (self.scroll as i64 + by as i64).clamp(0, self.lines.len().saturating_sub(1) as i64) as usize;
    }
}

/// The keys and buttons that steer the player, which the lobby keeps from the game. Whatever is
/// held when the lobby comes or goes is let go, and counts again only once pressed again, so
/// that a held trigger, or the click on Start, does not fire in the round.
pub struct Gate<K> {
    showing: bool,
    held: HashSet<K>,
}

impl<K: Eq + Hash + Copy> Default for Gate<K> {
    fn default() -> Gate<K> {
        Gate { showing: false, held: HashSet::new() }
    }
}

impl<K: Eq + Hash + Copy> Gate<K> {
    /// Whether the lobby shows. Says whether that changed, which lets go of everything.
    pub fn show(&mut self, showing: bool) -> bool {
        let changed = showing != self.showing;
        if changed {
            self.showing = showing;
            self.held.clear();
        }
        changed
    }

    pub fn showing(&self) -> bool {
        self.showing
    }

    /// A key went down. Says whether it counts: never while the lobby shows.
    pub fn press(&mut self, k: K) -> bool {
        if !self.showing {
            self.held.insert(k);
        }
        !self.showing
    }

    pub fn release(&mut self, k: K) {
        self.held.remove(&k);
    }

    pub fn held(&self, k: K) -> bool {
        self.held.contains(&k)
    }

    /// Hold or let go of a key by hand, as a scripted run does
    pub fn set(&mut self, k: K, on: bool) {
        if on && !self.showing {
            self.held.insert(k);
        } else {
            self.held.remove(&k);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Draw, Pointer};

    const LEVEL: &str = "citystreet_large";

    fn ui() -> Ui {
        Ui::new(Font::dots(10), Font::dots(20))
    }

    struct Seen {
        round: RoundState,
        time: f32,
        session: Session,
        me: usize,
        players: Vec<Player>,
        ready: Vec<usize>,
        pairs: Vec<String>,
        results: Option<RoundResults>,
    }

    impl Seen {
        /// Player 1 (slot 0) hosts on Blue, slot 1 is on Gold and ready, and this is slot `me`
        fn new(me: usize) -> Seen {
            Seen {
                round: RoundState { phase: Phase::Lobby, ..RoundState::default() },
                time: 0.0,
                session: Session { generation: 1, host: Some(0), time_limit: Some(180), friendly_fire: true, base_pair: 0 },
                me,
                players: vec![pl(0, Some(Side::Blue)), pl(1, Some(Side::Gold)), pl(2, Some(Side::Gold))],
                ready: vec![1],
                pairs: vec!["Upper Floors".to_string(), "Streets".to_string(), "Garage".to_string()],
                results: None,
            }
        }

        fn view(&self) -> LobbyView<'_> {
            LobbyView {
                round: self.round, time: self.time, session: self.session, me: self.me, players: &self.players, continued: false, ready: &self.ready,
                level: LEVEL, base_pairs: &self.pairs, uniforms: ["Blue Medium", "Desert 1 Medium"], results: self.results.as_ref(),
            }
        }
    }

    /// The player in `slot`, named as the screen names one it does not know
    fn pl(slot: usize, side: Option<Side>) -> Player {
        Player { slot, joined: slot as u32 + 1, side, name: name(slot) }
    }

    /// A click in the middle of `rect`: what it made, and what was drawn
    fn click(seen: &Seen, rect: Rect) -> (Vec<LobbyCommand>, Vec<Draw>) {
        let mut ui = ui();
        let mut chat = Chat::default();
        let at = [rect.x + rect.w / 2.0, rect.y + rect.h / 2.0];
        let mut made = Vec::new();
        for down in [false, true, false] {
            ui.begin(Pointer { at, down, ..Pointer::default() }, seen.session.generation);
            made.extend(lobby(&mut ui, &seen.view(), &mut chat));
            ui.finish();
        }
        ui.begin(Pointer::default(), seen.session.generation);
        lobby(&mut ui, &seen.view(), &mut chat);
        (made, ui.finish())
    }

    fn texts(draws: &[Draw]) -> Vec<&str> {
        draws.iter().filter_map(|d| match d {
            Draw::Text { text, .. } => Some(text.as_str()),
            _ => None,
        }).collect()
    }

    fn lines(chat: &Chat) -> Vec<String> {
        chat.lines().map(|l| match &l.who {
            Some(who) => format!("{}: {}", speaker(who, l.dead), l.text),
            None => l.text.clone(),
        }).collect()
    }

    #[test]
    fn a_full_game_of_players_all_fit_above_the_side_buttons() {
        let players: Vec<Player> = (0..MAX_PLAYERS).map(|i| pl(i, Some(if i % 2 == 0 { Side::Blue } else { Side::Gold }))).collect();
        let (_, draws) = click(&Seen { players, ..Seen::new(0) }, Rect::default());
        let bars: Vec<Rect> = draws.iter().filter_map(|d| match d {
            Draw::Art { art: Art::NameBlue | Art::NameGold, rect, .. } => Some(*rect),
            _ => None,
        }).collect();
        assert_eq!(bars.len(), MAX_PLAYERS);
        let lowest = bars.iter().map(|r| r.y + r.h).fold(0.0, f32::max);
        assert!(lowest <= BLUE.y, "the last row ends at {lowest}, under BLUE at {}", BLUE.y);
        assert!(GOLD.y + GOLD.h <= STATUS.y, "GOLD runs into the bar under it");
    }

    #[test]
    fn everyone_sees_the_players_by_side_who_is_ready_and_who_hosts() {
        let (_, draws) = click(&Seen::new(2), Rect::default());
        let lights: Vec<(Art, usize)> = draws.iter().filter_map(|d| match d {
            Draw::Art { art: a @ (Art::Host | Art::Ready | Art::NotReady), state, .. } => Some((*a, *state)),
            _ => None,
        }).collect();
        assert_eq!(lights, [(Art::Host, LIGHT), (Art::Ready, LIGHT), (Art::NotReady, LIGHT)]);
        let t = texts(&draws);
        for s in ["PLAYER 1", "PLAYER 2", "PLAYER 3", "Team Survival", LEVEL, "Upper Floors", "3:00", "Blue Medium", "Waiting for the host to start the round."] {
            assert!(t.contains(&s), "{s} is not shown");
        }
        // Their own bar is lit, and the others are dark
        let bars: Vec<(Art, usize)> = draws.iter().filter_map(|d| match d {
            Draw::Art { art: a @ (Art::NameBlue | Art::NameGold), state, .. } => Some((*a, *state)),
            _ => None,
        }).collect();
        assert_eq!(bars, [(Art::NameBlue, 0), (Art::NameGold, 0), (Art::NameGold, 1)]);
    }

    #[test]
    fn each_click_makes_its_command() {
        let host = Seen::new(0);
        let other = Seen::new(2);
        assert_eq!(click(&other, READY).0, [LobbyCommand::Ready(true)]);
        assert_eq!(click(&Seen { ready: vec![2], ..Seen::new(2) }, READY).0, [LobbyCommand::Ready(false)]);
        assert_eq!(click(&other, BLUE).0, [LobbyCommand::Side(Side::Blue)]);
        // Their own side is not a change
        assert_eq!(click(&other, GOLD).0, []);
        assert_eq!(click(&host, START).0, [LobbyCommand::Start]);
        assert_eq!(click(&host, FRIENDLY_FIRE).0, [LobbyCommand::FriendlyFire(false)]);
        // The insertions go round
        assert_eq!(click(&host, arrows(INSERTIONS)[0]).0, [LobbyCommand::BasePair(1)]);
        assert_eq!(click(&host, arrows(INSERTIONS)[1]).0, [LobbyCommand::BasePair(2)]);
    }

    #[test]
    fn up_makes_the_round_longer_and_the_lengths_stop_at_either_end() {
        let with = |t| Seen { session: Session { time_limit: t, ..Seen::new(0).session }, ..Seen::new(0) };
        assert_eq!(click(&with(Some(180)), arrows(LENGTH)[0]).0, [LobbyCommand::TimeLimit(Some(240))]);
        assert_eq!(click(&with(Some(180)), arrows(LENGTH)[1]).0, [LobbyCommand::TimeLimit(Some(120))]);
        assert_eq!(click(&with(Some(600)), arrows(LENGTH)[0]).0, [LobbyCommand::TimeLimit(None)]);
        assert_eq!(click(&with(None), arrows(LENGTH)[0]).0, []);
        assert_eq!(click(&with(None), arrows(LENGTH)[1]).0, [LobbyCommand::TimeLimit(Some(600))]);
        assert_eq!(click(&with(Some(60)), arrows(LENGTH)[1]).0, []);
        // A length the server was started with, which is not one of the choices
        assert_eq!(click(&with(Some(200)), arrows(LENGTH)[0]).0, [LobbyCommand::TimeLimit(Some(240))]);
        assert_eq!(click(&with(Some(200)), arrows(LENGTH)[1]).0, [LobbyCommand::TimeLimit(Some(180))]);
    }

    #[test]
    fn only_the_host_has_the_settings_and_the_start() {
        let other = Seen::new(2);
        for rect in [START, FRIENDLY_FIRE, arrows(INSERTIONS)[1], arrows(LENGTH)[0]] {
            assert_eq!(click(&other, rect).0, [], "{rect:?}");
        }
        assert!(!texts(&click(&other, Rect::default()).1).contains(&"START"));
        // Nor can the host start with a side empty
        let alone = Seen { players: vec![pl(0, Some(Side::Blue))], ..Seen::new(0) };
        assert_eq!(click(&alone, START).0, []);
        assert!(texts(&click(&alone, Rect::default()).1).contains(&"Each side needs someone before you can start."));
    }

    #[test]
    fn in_the_countdown_all_is_locked_but_the_hosts_stop() {
        let counting = |me| Seen { round: RoundState { phase: Phase::Countdown, ends: Some(4.2), ..RoundState::default() }, ..Seen::new(me) };
        for me in [0, 2] {
            for rect in [READY, BLUE, GOLD, FRIENDLY_FIRE, arrows(INSERTIONS)[1], arrows(LENGTH)[0]] {
                assert_eq!(click(&counting(me), rect).0, [], "{rect:?}");
            }
        }
        assert_eq!(click(&counting(0), START).0, [LobbyCommand::Stop]);
    }

    #[test]
    fn the_countdown_and_the_rounds_end_go_in_the_chat() {
        let mut seen = Seen::new(2);
        let mut chat = Chat::default();
        chat.watch(&seen.view());
        assert!(lines(&chat).is_empty());
        seen.round = RoundState { phase: Phase::Countdown, number: 0, ends: Some(5.0), ..RoundState::default() };
        // The clock goes back a little now and then: no number is said twice
        for t in [0.0, 0.5, 1.0, 0.9, 1.5, 2.0, 1.8, 2.5, 3.0, 3.5, 4.0, 3.95, 4.5] {
            seen.time = t;
            chat.watch(&seen.view());
        }
        seen.round = RoundState { phase: Phase::Loading, number: 1, ..RoundState::default() };
        chat.watch(&seen.view());
        seen.round = RoundState { phase: Phase::Results, number: 1, wins: [0, 1], last: Some(Outcome::Won(Side::Gold)), ..RoundState::default() };
        seen.results = Some(table(1, Outcome::Won(Side::Gold), None, vec![row(0, "PLAYER 1", Some(Side::Blue))]));
        chat.watch(&seen.view());
        // Told once, though the table stays until the lobby
        seen.round.phase = Phase::Debrief;
        chat.watch(&seen.view());
        seen.round.phase = Phase::Lobby;
        seen.results = None;
        chat.watch(&seen.view());
        seen.round.phase = Phase::Countdown;
        seen.round.ends = Some(seen.time + 5.0);
        chat.watch(&seen.view());
        seen.round.phase = Phase::Lobby;
        chat.watch(&seen.view());
        assert_eq!(lines(&chat), [
            "--- Preparing to begin mission ---", "Starting mission in 5 seconds", "Starting mission in 4 seconds", "Starting mission in 3 seconds",
            "Starting mission in 2 seconds", "Starting mission in 1 second", "--- Away we go ---", "Round 1: Gold Team Won (Blue 0, Gold 1)",
            CONTINUE_LINE, "--- Preparing to begin mission ---", "Starting mission in 5 seconds", "--- Start suspended ---",
        ]);
    }

    fn row(slot: u8, name: &str, side: Option<Side>) -> crate::net::proto::ResultRow {
        crate::net::proto::ResultRow {
            slot, joined: slot as u32 + 1, name: name.to_string(), side, left: false, end: crate::sim::Ending::Alive,
            stats: crate::sim::RoundStats::default(), killed_by: None,
        }
    }

    fn table(number: u16, outcome: Outcome, winner: Option<u8>, rows: Vec<crate::net::proto::ResultRow>) -> RoundResults {
        RoundResults { number, outcome, winner, played: 60, rows }
    }

    #[test]
    fn the_winner_is_told_as_they_were_though_they_left_and_the_client_missed_the_results() {
        // Survival: the winner leaves as the round ends, and a newcomer takes their slot. The
        // client sees the round under way, then nothing until the results screen.
        let mut seen = Seen { players: vec![Player { name: "OriginalWinner".to_string(), ..pl(0, None) }, pl(1, None)], ..Seen::new(1) };
        seen.round = RoundState { phase: Phase::Live, number: 1, ..RoundState::default() };
        let mut chat = Chat::default();
        chat.watch(&seen.view());
        seen.players[0] = Player { joined: 50, name: "Newcomer".to_string(), ..pl(0, None) };
        seen.round = RoundState { phase: Phase::Debrief, number: 1, last: Some(Outcome::Survived(0)), ..RoundState::default() };
        seen.results = Some(table(1, Outcome::Survived(0), Some(0), vec![row(0, "OriginalWinner", None), row(1, "PLAYER 2", None)]));
        chat.watch(&seen.view());
        assert!(lines(&chat).contains(&"Round 1: OriginalWinner Won".to_string()), "{:?}", lines(&chat));
        // A client that comes while a result is up does not tell it as news
        let mut late = Chat::default();
        late.watch(&seen.view());
        seen.time += 1.0;
        late.watch(&seen.view());
        assert!(!lines(&late).iter().any(|l| l.starts_with("Round 1")));
    }

    #[test]
    fn players_coming_going_and_the_hosts_changes_go_in_the_chat() {
        let mut seen = Seen::new(2);
        let mut chat = Chat::default();
        chat.watch(&seen.view());
        seen.players.push(Player { name: "Villico".to_string(), ..pl(3, Some(Side::Blue)) });
        seen.players.retain(|p| p.slot != 1);
        seen.players[1].side = Some(Side::Blue);
        seen.session = Session { time_limit: Some(300), friendly_fire: false, base_pair: 1, ..seen.session };
        chat.watch(&seen.view());
        chat.said("Villico", "hello", false);
        chat.said("PLAYER 3", "gg", true);
        // Someone new in a slot is the last one going and another coming
        seen.players[0] = Player { joined: 99, name: "Raithat".to_string(), ..pl(0, Some(Side::Blue)) };
        chat.watch(&seen.view());
        assert_eq!(lines(&chat), [
            "PLAYER 2 has disconnected", "PLAYER 3 went to Blue", "Villico has joined Blue", "INSERTIONS changed to Streets", "LENGTH changed to 5:00",
            "FRIENDLY FIRE changed to off", "Villico: hello", "PLAYER 3 (dead): gg", "PLAYER 1 has disconnected", "Raithat has joined Blue",
        ]);
    }

    #[test]
    fn what_is_typed_is_said_and_shown_with_its_name() {
        let seen = Seen::new(2);
        let mut chat = Chat::default();
        for c in " gg\u{7}, één more ".chars() {
            chat.typed(c);
        }
        chat.erase();
        assert_eq!(chat.typing(), " gg, één more");
        assert_eq!(chat.send(), Some(LobbyCommand::Say("gg, één more".to_string())));
        assert_eq!(chat.typing(), "");
        // Nothing to say is not said; Escape clears the line first
        chat.typed(' ');
        assert_eq!(chat.send(), None);
        chat.typed('x');
        assert!(chat.clear());
        assert!(!chat.clear());
        for _ in 0..CHAT_LENGTH + 5 {
            chat.typed('a');
        }
        assert_eq!(chat.typing().len(), CHAT_LENGTH);
        // What players say is drawn as "NAME: text"
        chat.said("PLAYER 2", "on my way", false);
        let mut ui = ui();
        ui.begin(Pointer::default(), 1);
        lobby(&mut ui, &seen.view(), &mut chat);
        assert!(texts(&ui.finish()).contains(&"PLAYER 2: on my way"));
    }

    #[test]
    fn the_chat_scrolls_back_and_stays_there_as_lines_come() {
        let seen = Seen::new(2);
        let mut chat = Chat::default();
        for i in 0..30 {
            chat.said("PLAYER 1", &format!("line {i}"), false);
        }
        let shown = |chat: &mut Chat, pointer: Pointer| {
            let mut ui = ui();
            ui.begin(pointer, 1);
            lobby(&mut ui, &seen.view(), chat);
            texts(&ui.finish()).iter().filter(|t| t.starts_with("PLAYER 1: ")).map(|t| t.to_string()).collect::<Vec<_>>()
        };
        assert_eq!(shown(&mut chat, Pointer::default()).first().map(String::as_str), Some("PLAYER 1: line 29"));
        // The wheel over the box, away from the player, goes back
        shown(&mut chat, Pointer { at: [300.0, 300.0], wheel: 3.0, ..Pointer::default() });
        assert_eq!(shown(&mut chat, Pointer::default())[0], "PLAYER 1: line 26");
        chat.said("PLAYER 1", "line 30", false);
        assert_eq!(shown(&mut chat, Pointer::default())[0], "PLAYER 1: line 26");
        // Saying something goes back to the newest
        chat.typed('x');
        chat.send();
        assert_eq!(shown(&mut chat, Pointer::default())[0], "PLAYER 1: line 30");
        // The up arrow goes back a line
        let mut ui = ui();
        for down in [false, true, false] {
            let at = [SCROLL_UP.x + 8.0, SCROLL_UP.y + 7.0];
            ui.begin(Pointer { at, down, ..Pointer::default() }, 1);
            lobby(&mut ui, &seen.view(), &mut chat);
            ui.finish();
        }
        assert_eq!(shown(&mut chat, Pointer::default())[0], "PLAYER 1: line 29");
    }

    #[test]
    fn long_lines_are_wrapped_between_words() {
        let font = Font::dots(10);
        let text = "the quick brown fox jumps over the lazy dog";
        let lines = wrap(&font, text, font.width("the quick brown"));
        assert_eq!(lines, ["the quick brown", "fox jumps over", "the lazy dog"]);
        let lines = wrap(&font, "abcdefghij", font.width("abcd"));
        assert_eq!(lines, ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn a_held_key_counts_only_once_pressed_again_after_the_lobby_comes_or_goes() {
        let mut gate: Gate<char> = Gate::default();
        assert!(gate.press('F'));
        assert!(gate.held('F'));
        // The lobby shows: nothing counts
        assert!(gate.show(true));
        assert!(!gate.held('F'));
        assert!(!gate.press('W'));
        assert!(!gate.held('W'));
        // The click on Start is held as the round begins: it is not a shot
        assert!(gate.show(false));
        assert!(!gate.held('F'));
        gate.release('F');
        assert!(gate.press('F'));
        assert!(gate.held('F'));
        assert!(!gate.show(false));
        assert!(gate.held('F'));
    }
}
