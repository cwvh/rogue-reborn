//! The results screen, which everyone sees after a round until they continue to the lobby (see
//! "Results" in docs/multiplayer.md).
//!
//! On the left is the round's table: each player who played it, their health at the end, their
//! kills, hit percentage, rounds fired and rounds taken, and the last column as this player saw
//! the round: a skull on each player they killed, a crosshair on the one who killed them. Each
//! side's totals follow. On the right are the banner, this player's name, what was played, and
//! the chat. It is laid out as the original's, from the table the server sent once the round was
//! decided (`RoundResults`), and gives back Continue, or for the host ToLobby once they have
//! continued.

use crate::lobby::{
    BACK, BLUE_BAR, Chat, ChatPlace, DIM_TEXT, FIELD, FIELD_EDGE, FIELD_TEXT, GOLD_BAR, LABEL, LABEL_TEXT, START, STATUS, WHITE, button, chat_box,
    side_name,
};
use crate::net::proto::{LobbyCommand, ResultRow, RoundResults};
use crate::net::round::Outcome;
use crate::sim::{Ending, RoundStats, Side};
use crate::ui::{Art, Color, FontId, Rect, Ui};

/// What the client knows that the screen shows
pub struct ResultsView<'a> {
    pub results: &'a RoundResults,
    /// The server's clock now, for the caret
    pub time: f32,
    /// This player, by slot and join number
    pub me: (usize, u32),
    pub host: bool,
    /// Who has continued, by slot and join number, as of the newest snapshot
    pub continued: &'a [(usize, u32)],
    /// This player has sent Continue for this screen, and is waiting or done
    pub sent: bool,
    /// The mission's name, as the level's mission file gives it
    pub mission: &'a str,
}

impl ResultsView<'_> {
    /// This player's row, if they played the round
    fn mine(&self) -> Option<usize> {
        self.results.rows.iter().position(|r| (r.slot as usize, r.joined) == self.me)
    }

    fn team_game(&self) -> bool {
        self.results.rows.iter().any(|r| r.side.is_some())
    }

    fn has_continued(&self) -> bool {
        self.continued.contains(&self.me)
    }
}

/// How the round went for this player
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Successful,
    Failed,
    Aborted,
    /// They did not play it
    Watched,
}

pub fn verdict(v: &ResultsView) -> Verdict {
    let Some(mine) = v.mine() else { return Verdict::Watched };
    let won = match v.results.outcome {
        Outcome::Won(side) => v.results.rows[mine].side == Some(side),
        Outcome::Survived(_) => v.results.winner == Some(mine as u8),
        Outcome::Draw => false,
        Outcome::CalledOff => return Verdict::Aborted,
    };
    if won { Verdict::Successful } else { Verdict::Failed }
}

/// The round's result in the original's words
pub fn result_text(results: &RoundResults) -> String {
    match results.outcome {
        Outcome::Won(Side::Blue) => "Blue Team Won".to_string(),
        Outcome::Won(Side::Gold) => "Gold Team Won".to_string(),
        Outcome::Survived(_) => match results.winner.and_then(|w| results.rows.get(w as usize)) {
            Some(row) => format!("{} Won", row.name),
            None => "Everybody Lost".to_string(),
        },
        Outcome::Draw => "Everybody Lost".to_string(),
        Outcome::CalledOff => "The round was called off".to_string(),
    }
}

/// A round's length as the screen shows it, as hours, minutes and seconds
pub fn elapsed_text(seconds: u16) -> String {
    format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

// Where things are, on the 640 x 480 canvas, as measured on the original's
const HEAD_Y: f32 = 42.0;
const ROWS_Y: f32 = 59.0;
const ROW: f32 = 19.0;
const ROW_H: f32 = 17.0;
const LIGHT_X: f32 = 25.0;
const HEALTH_X: f32 = 43.0;
const NAME_X: f32 = 60.0;
const NAME_W: f32 = 136.0;
const STATS_X: [f32; 4] = [197.0, 223.0, 249.0, 275.0];
const SHOT_X: f32 = 301.0;
/// The right side: the banner, the name box, the text box, what was played and the chat
const RIGHT: f32 = 323.0;
const BANNER: Rect = Rect::new(RIGHT, 42.0, 294.0, 35.0);
/// The picked player's box (`Art::PickedBlue` and the rest): their patch, name and clan. The
/// fields are clear in the art.
const PICKED: Rect = Rect::new(324.0, 78.0, 292.0, 50.0);
const PATCH: Rect = Rect::new(342.0, 87.0, 32.0, 32.0);
const NAME_ROWS: [Rect; 2] = [Rect::new(396.0, 83.0, 220.0, 19.0), Rect::new(396.0, 103.0, 220.0, 19.0)];
const TEXT: Rect = Rect::new(RIGHT, 128.0, 294.0, 70.0);
const INFO: Rect = Rect::new(RIGHT, 198.0, 294.0, 72.0);
const CHAT: ChatPlace = ChatPlace {
    art: Art::ResultsChat,
    frame: Rect::new(RIGHT, 276.0, 294.0, 124.0),
    lines: Rect::new(RIGHT + 4.0, 279.0, 272.0, 97.0),
    typing: Rect::new(RIGHT + 11.0, 383.0, 257.0, 16.0),
    up: Rect::new(RIGHT + 277.0, 285.0, 16.0, 14.0),
    down: Rect::new(RIGHT + 277.0, 356.0, 16.0, 14.0),
};
/// CONTINUE, where the lobby's START is
pub const CONTINUE: Rect = START;

const ID_CONTINUE: u32 = 20;
const ID_TO_LOBBY: u32 = 21;
const ID_ROW: u32 = 30;

/// The player picked on the table: whose name box shows, and whose kills the last column marks.
/// This player at first, until another row is clicked; a new round's table starts again.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Picked {
    round: u16,
    row: Option<usize>,
}

impl Picked {
    fn row(&mut self, v: &ResultsView) -> Option<usize> {
        if self.round != v.results.number {
            *self = Picked { round: v.results.number, row: v.mine() };
        }
        self.row.filter(|&r| r < v.results.rows.len())
    }
}

/// The lit state of a table's piece for the picked player's row, and the dark for anyone else's
fn lit(picked: bool) -> usize {
    picked as usize
}

/// Lay out the results screen, and say which commands this frame's clicks made. A click on a
/// player's row picks them (`picked`).
pub fn results(ui: &mut Ui, v: &ResultsView, chat: &mut Chat, picked: &mut Picked) -> Vec<LobbyCommand> {
    let mut commands = Vec::new();
    let screen = Rect::new(0.0, 0.0, 640.0, 480.0);
    ui.fill(screen, BACK);
    ui.tile(Art::Grid, screen, BACK);
    let sel = picked.row(v);

    // The table's header, then a row each: Blue's, Gold's, and those with no side, each side's
    // players who left after those who stayed
    for (x, w, art) in [(LIGHT_X, 17.0, Art::HeadReady), (HEALTH_X, 16.0, Art::HeadHealth), (NAME_X, NAME_W, Art::HeadName), (SHOT_X, 17.0, Art::HeadShot)] {
        ui.art(art, 0, Rect::new(x, HEAD_Y, w, 16.0), LABEL);
    }
    for (x, art) in STATS_X.into_iter().zip([Art::HeadKills, Art::HeadHits, Art::HeadFired, Art::HeadTaken]) {
        ui.art(art, 0, Rect::new(x, HEAD_Y, 25.0, 16.0), LABEL);
    }
    let mut order: Vec<usize> = (0..v.results.rows.len()).collect();
    order.sort_by_key(|&i| {
        let r = &v.results.rows[i];
        (r.side.map_or(2, |s| s.index()), r.left, r.slot)
    });
    let mut y = ROWS_Y;
    for i in order {
        let row = &v.results.rows[i];
        if ui.control(ID_ROW + i as u32, Rect::new(LIGHT_X, y, SHOT_X + 17.0 - LIGHT_X, ROW_H), true).1 {
            picked.row = Some(i);
        }
        // The picked player's row is lit
        let me = picked.row == Some(i);
        let done = v.continued.contains(&(row.slot as usize, row.joined));
        ui.art(if done { Art::Ready } else { Art::NotReady }, lit(me), Rect::new(LIGHT_X, y, 17.0, ROW_H), if done { [0.2, 0.8, 0.2, 1.0] } else { [0.6, 0.1, 0.1, 1.0] });
        let health = if row.end == Ending::Killed { Art::Killed } else { Art::Healthy };
        ui.art(health, lit(me), Rect::new(HEALTH_X, y, 16.0, ROW_H), LABEL);
        let (name, stat, shot, fallback) = match (row.left, row.side) {
            (true, _) => (Art::NameLeft, Art::StatLeft, Art::ShotLeft, [0.15, 0.5, 0.25, 1.0]),
            (false, Some(Side::Blue)) => (Art::NameBlue, Art::StatBlue, Art::ShotBlue, BLUE_BAR),
            (false, Some(Side::Gold)) => (Art::NameGold, Art::StatGold, Art::ShotGold, GOLD_BAR),
            (false, None) => (Art::NameNone, Art::StatTotals, Art::ShotTotals, LABEL),
        };
        let bar = Rect::new(NAME_X, y, NAME_W, ROW_H);
        ui.art(name, lit(me), bar, fallback);
        ui.centered_clipped(&row.name, FontId::Text, bar, 4.0, WHITE);
        numbers(ui, y, row.stats, stat, lit(me), fallback);
        // The last column: whom the picked player killed, and who killed them
        let marker = match sel {
            Some(m) if row.killed_by == Some(m as u8) => Art::YouKilled,
            Some(m) if v.results.rows[m].killed_by == Some(i as u8) => Art::KilledYou,
            _ => shot,
        };
        ui.art(marker, if matches!(marker, Art::YouKilled | Art::KilledYou) { 0 } else { lit(me) }, Rect::new(SHOT_X, y, 17.0, ROW_H), fallback);
        y += ROW;
    }
    // Each side's totals, of everyone who played for it
    if v.team_game() {
        for side in Side::BOTH {
            let rows: Vec<&ResultRow> = v.results.rows.iter().filter(|r| r.side == Some(side)).collect();
            let sum = rows.iter().fold(RoundStats::default(), |a, r| RoundStats {
                kills: a.kills + r.stats.kills, fired: a.fired + r.stats.fired, hits: a.hits + r.stats.hits, taken: a.taken + r.stats.taken,
            });
            ui.art(Art::PingBox, 0, Rect::new(LIGHT_X, y, 17.0, ROW_H), FIELD);
            ui.art(Art::HealthTotals, 0, Rect::new(HEALTH_X, y, 16.0, ROW_H), FIELD);
            let bar = Rect::new(NAME_X, y, NAME_W, ROW_H);
            ui.art(Art::NameTotals, 0, bar, FIELD);
            ui.centered(&format!("{} TEAM STATS", side_name(side).to_uppercase()), FontId::Text, bar, WHITE);
            numbers(ui, y, sum, Art::StatTotals, 0, FIELD);
            ui.art(Art::ShotTotals, 0, Rect::new(SHOT_X, y, 17.0, ROW_H), FIELD);
            y += ROW;
        }
    }

    // The banner, as the round went for this player
    let (title, fill, edge, ink) = match verdict(v) {
        Verdict::Successful => ("MISSION SUCCESSFUL".to_string(), [0.07, 0.3, 0.0, 1.0], [0.35, 0.8, 0.1, 1.0], [0.3, 1.0, 0.1, 1.0]),
        Verdict::Failed => ("MISSION FAILED".to_string(), [0.55, 0.0, 0.0, 1.0], [0.95, 0.25, 0.15, 1.0], [1.0, 0.1, 0.05, 1.0]),
        Verdict::Aborted => ("MISSION ABORTED".to_string(), [0.45, 0.22, 0.0, 1.0], [0.95, 0.55, 0.1, 1.0], [1.0, 0.6, 0.1, 1.0]),
        Verdict::Watched => (result_text(v.results).to_uppercase(), [0.0, 0.0, 0.0, 1.0], [0.6, 0.6, 0.6, 1.0], WHITE),
    };
    ui.fill(BANNER, fill);
    ui.frame(BANNER, edge);
    ui.centered(&title, FontId::Big, BANNER, ink);

    // Who is picked: their patch and name in their side's box, and the clan line, which no one
    // sets. With no one picked, an empty Blue box.
    let row = sel.map(|s| &v.results.rows[s]);
    let (side, left) = row.map_or((None, false), |r| (r.side, r.left));
    for rect in NAME_ROWS {
        ui.fill(rect, FIELD);
    }
    let art = match (left, side) {
        (true, _) => Art::PickedLeft,
        (false, Some(Side::Gold)) => Art::PickedGold,
        _ => Art::PickedBlue,
    };
    ui.art(art, 0, PICKED, FIELD_EDGE);
    if let Some(row) = row {
        ui.art(if side == Some(Side::Gold) { Art::PatchGold } else { Art::PatchBlue }, 0, PATCH, LABEL);
        ui.left_clipped(&row.name, FontId::Text, NAME_ROWS[0], 4.0, FIELD_TEXT);
    }

    // What was played, and how it ended
    ui.art(Art::ResultsText, 0, TEXT, FIELD);
    let rules = if v.team_game() { "Team Survival" } else { "Survival" };
    for (i, line) in [format!("Adversarial / {rules}"), result_text(v.results)].iter().enumerate() {
        ui.text(line, FontId::Text, [TEXT.x + 14.0, TEXT.y + 9.0 + i as f32 * 14.0], LABEL_TEXT);
    }
    ui.art(Art::ResultsInfo, 0, INFO, FIELD);
    let rows = [("MISSION", v.mission.to_string()), ("TYPE", "Adversarial".to_string()), ("RULES", rules.to_string()), ("ELAPSED TIME", elapsed_text(v.results.played))];
    for (i, (label, value)) in rows.iter().enumerate() {
        let y = INFO.y + 1.0 + i as f32 * 18.0;
        // In light blue, as in the original's screenshot at its own size (another shows gold)
        ui.centered(label, FontId::Text, Rect::new(INFO.x + 11.0, y, 78.0, 16.0), LABEL_TEXT);
        ui.left_clipped(value, FontId::Text, Rect::new(INFO.x + 91.0, y, 191.0, 16.0), 4.0, FIELD_TEXT);
    }
    chat_box(ui, v.time, chat, &CHAT);

    // What the player can do now
    let hint = match () {
        _ if v.host && v.has_continued() => "TO LOBBY takes everyone to the lobby now.",
        _ if v.sent => "Waiting for the others to continue.",
        _ => "Continue when you are ready for the next round.",
    };
    ui.fill(STATUS, LABEL);
    ui.left(hint, FontId::Text, STATUS, 4.0, LABEL_TEXT);
    let esc = "ESC LEAVES THE GAME";
    let w = ui.text.width(esc);
    ui.left(esc, FontId::Text, STATUS, STATUS.w - w - 4.0, DIM_TEXT);

    // CONTINUE, and the host's second click, a control of its own
    if v.host && v.has_continued() {
        let (look, clicked) = ui.control(ID_TO_LOBBY, CONTINUE, true);
        button(ui, CONTINUE, look, false, "TO LOBBY");
        if clicked {
            commands.push(LobbyCommand::ToLobby);
        }
    } else {
        let (look, clicked) = ui.control(ID_CONTINUE, CONTINUE, !v.sent);
        button(ui, CONTINUE, look, false, "CONTINUE");
        if clicked {
            commands.push(LobbyCommand::Continue);
        }
    }
    commands
}

/// A row's four numbers in their boxes: kills, hit percentage, rounds fired and rounds taken
fn numbers(ui: &mut Ui, y: f32, s: RoundStats, art: Art, state: usize, fallback: Color) {
    for (x, n) in STATS_X.into_iter().zip([s.kills, s.hit_percent(), s.fired, s.taken]) {
        let rect = Rect::new(x, y, 25.0, ROW_H);
        ui.art(art, state, rect, fallback);
        ui.centered(&n.to_string(), FontId::Text, rect, WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::font::Font;
    use crate::ui::{Draw, Pointer};

    fn ui() -> Ui {
        Ui::new(Font::dots(10), Font::dots(20))
    }

    fn row(slot: u8, side: Side, name: &str) -> ResultRow {
        ResultRow { slot, joined: 10 + slot as u32, name: name.to_string(), side: Some(side), left: false, end: Ending::Alive, stats: RoundStats::default(), killed_by: None }
    }

    /// Gold won: 7ony (Blue, slot 0) was killed by Raithat (Gold, slot 2), who also killed
    /// BobbyBoy (Blue), and Villico (Gold) left
    fn table() -> RoundResults {
        let mut rows = vec![row(0, Side::Blue, "7ony"), row(1, Side::Blue, "BobbyBoyGaming"), row(2, Side::Gold, "Raithat"), row(3, Side::Gold, "Villico")];
        rows[0].stats = RoundStats { kills: 0, fired: 12, hits: 0, taken: 6 };
        rows[1].stats = RoundStats { kills: 2, fired: 31, hits: 7, taken: 2 };
        rows[2].stats = RoundStats { kills: 1, fired: 4, hits: 2, taken: 1 };
        rows[3].stats = RoundStats { kills: 0, fired: 15, hits: 0, taken: 2 };
        (rows[0].end, rows[0].killed_by) = (Ending::Killed, Some(2));
        (rows[1].end, rows[1].killed_by) = (Ending::Killed, Some(2));
        rows[3].left = true;
        RoundResults { number: 3, outcome: Outcome::Won(Side::Gold), winner: None, played: 116, rows }
    }

    struct Seen {
        results: RoundResults,
        me: (usize, u32),
        host: bool,
        continued: Vec<(usize, u32)>,
        sent: bool,
    }

    impl Seen {
        fn new(me: usize) -> Seen {
            Seen { results: table(), me: (me, 10 + me as u32), host: false, continued: Vec::new(), sent: false }
        }

        fn view(&self) -> ResultsView<'_> {
            ResultsView { results: &self.results, time: 0.0, me: self.me, host: self.host, continued: &self.continued, sent: self.sent, mission: "City Street Large" }
        }
    }

    /// A click in the middle of `rect`: what it made, and what was drawn
    fn click(seen: &Seen, rect: Rect) -> (Vec<LobbyCommand>, Vec<Draw>) {
        click_picked(seen, rect, &mut Picked::default())
    }

    fn click_picked(seen: &Seen, rect: Rect, picked: &mut Picked) -> (Vec<LobbyCommand>, Vec<Draw>) {
        let (mut ui, mut chat) = (ui(), Chat::default());
        let at = [rect.x + rect.w / 2.0, rect.y + rect.h / 2.0];
        let mut made = Vec::new();
        for down in [false, true, false] {
            ui.begin(Pointer { at, down, ..Pointer::default() }, 1);
            made.extend(results(&mut ui, &seen.view(), &mut chat, picked));
            ui.finish();
        }
        ui.begin(Pointer::default(), 1);
        results(&mut ui, &seen.view(), &mut chat, picked);
        (made, ui.finish())
    }

    /// The table's row `n`, counted from the top
    fn row_at(n: usize) -> Rect {
        Rect::new(LIGHT_X, ROWS_Y + n as f32 * ROW, SHOT_X + 17.0 - LIGHT_X, ROW_H)
    }

    fn picked_name(draws: &[Draw]) -> Option<&str> {
        draws.iter().find_map(|d| match d {
            Draw::Text { text, at, .. } if NAME_ROWS[0].contains(*at) => Some(text.as_str()),
            _ => None,
        })
    }

    fn texts(draws: &[Draw]) -> Vec<&str> {
        draws.iter().filter_map(|d| match d {
            Draw::Text { text, .. } => Some(text.as_str()),
            _ => None,
        }).collect()
    }

    fn arts(draws: &[Draw], which: &[Art]) -> Vec<(Art, usize, f32)> {
        draws.iter().filter_map(|d| match d {
            Draw::Art { art, state, rect, .. } if which.contains(art) => Some((*art, *state, rect.y)),
            _ => None,
        }).collect()
    }

    #[test]
    fn the_banner_is_how_the_round_went_for_this_player() {
        let banner = |seen: &Seen| verdict(&seen.view());
        assert_eq!(banner(&Seen::new(0)), Verdict::Failed);
        assert_eq!(banner(&Seen::new(2)), Verdict::Successful);
        // One who left still lost or won with their side
        assert_eq!(banner(&Seen::new(3)), Verdict::Successful);
        assert_eq!(banner(&Seen { me: (2, 99), ..Seen::new(2) }), Verdict::Watched, "someone new in a player's slot did not play");
        let mut called = Seen::new(0);
        called.results.outcome = Outcome::CalledOff;
        assert_eq!(banner(&called), Verdict::Aborted);
        // In Survival, by the winner's row
        let mut alone = Seen::new(1);
        alone.results.outcome = Outcome::Survived(1);
        alone.results.winner = Some(1);
        assert_eq!(banner(&alone), Verdict::Successful);
        assert_eq!(result_text(&alone.results), "BobbyBoyGaming Won");
        assert_eq!(banner(&Seen { results: alone.results.clone(), ..Seen::new(0) }), Verdict::Failed);
        let t = texts(&click(&Seen::new(0), Rect::default()).1).join("|");
        for s in ["MISSION FAILED", "7ony", "Adversarial / Team Survival", "Gold Team Won", "City Street Large", "0:01:56", "BLUE TEAM STATS", "GOLD TEAM STATS"] {
            assert!(t.contains(s), "{s} is not shown");
        }
    }

    #[test]
    fn the_table_has_each_side_in_turn_those_who_left_last_and_the_totals_of_all() {
        let draws = click(&Seen::new(0), Rect::default()).1;
        let t = texts(&draws);
        let order: Vec<&str> = t.iter().copied().filter(|s| ["7ony", "BobbyBoyGaming", "Raithat", "Villico", "BLUE TEAM STATS", "GOLD TEAM STATS"].contains(s)).take(6).collect();
        assert_eq!(order, ["7ony", "BobbyBoyGaming", "Raithat", "Villico", "BLUE TEAM STATS", "GOLD TEAM STATS"]);
        // Villico left: green. Gold's totals count them: 1 kill, 2 hits of 19 fired, 3 taken
        assert_eq!(arts(&draws, &[Art::NameLeft]).len(), 1);
        let gold: Vec<&str> = t.iter().copied().skip_while(|s| *s != "GOLD TEAM STATS").skip(1).take(4).collect();
        assert_eq!(gold, ["1", "10", "19", "3"]);
        // The player's own row is lit, and only theirs
        let names = arts(&draws, &[Art::NameBlue, Art::NameGold, Art::NameLeft]);
        assert_eq!(names.iter().filter(|n| n.1 == 1).map(|n| n.2).collect::<Vec<_>>(), [ROWS_Y]);
    }

    #[test]
    fn the_last_column_marks_whom_this_player_killed_and_who_killed_them() {
        // 7ony was killed by Raithat
        let marks = arts(&click(&Seen::new(0), Rect::default()).1, &[Art::YouKilled, Art::KilledYou]);
        assert_eq!(marks, [(Art::KilledYou, 0, ROWS_Y + 2.0 * ROW)]);
        // Raithat killed both of Blue's
        let marks = arts(&click(&Seen::new(2), Rect::default()).1, &[Art::YouKilled, Art::KilledYou]);
        assert_eq!(marks, [(Art::YouKilled, 0, ROWS_Y), (Art::YouKilled, 0, ROWS_Y + ROW)]);
        // One who did not play sees no marks, and an empty name box
        let (_, draws) = click(&Seen { me: (2, 99), ..Seen::new(2) }, Rect::default());
        assert!(arts(&draws, &[Art::YouKilled, Art::KilledYou]).is_empty());
        assert_eq!(picked_name(&draws), None);
    }

    #[test]
    fn a_click_on_a_row_shows_whom_that_player_killed_and_who_killed_them() {
        // 7ony starts on themselves, and clicks Raithat's row, the third
        let seen = Seen::new(0);
        let mut picked = Picked::default();
        let (made, draws) = click_picked(&seen, row_at(2), &mut picked);
        assert_eq!(made, [], "a row is not a command");
        assert_eq!(picked_name(&draws), Some("Raithat"));
        let marks = arts(&draws, &[Art::YouKilled, Art::KilledYou]);
        assert_eq!(marks, [(Art::YouKilled, 0, ROWS_Y), (Art::YouKilled, 0, ROWS_Y + ROW)]);
        assert!(arts(&draws, &[Art::PickedGold]).len() == 1);
        // Raithat's row is the lit one now; the banner is still 7ony's
        let lit: Vec<f32> = arts(&draws, &[Art::NameBlue, Art::NameGold, Art::NameLeft]).iter().filter(|n| n.1 == 1).map(|n| n.2).collect();
        assert_eq!(lit, [ROWS_Y + 2.0 * ROW]);
        assert!(texts(&draws).contains(&"MISSION FAILED"));
        // BobbyBoy: killed by Raithat, killed no one
        let (_, draws) = click_picked(&seen, row_at(1), &mut picked);
        assert_eq!(arts(&draws, &[Art::YouKilled, Art::KilledYou]), [(Art::KilledYou, 0, ROWS_Y + 2.0 * ROW)]);
        // Villico, who left: their box is green
        let (_, draws) = click_picked(&seen, row_at(3), &mut picked);
        assert_eq!(picked_name(&draws), Some("Villico"));
        assert!(arts(&draws, &[Art::PickedLeft]).len() == 1);
        // The next round's table starts again on the player
        let next = Seen { results: RoundResults { number: 4, ..table() }, ..Seen::new(0) };
        let (_, draws) = click_picked(&next, Rect::default(), &mut picked);
        assert_eq!(picked_name(&draws), Some("7ony"));
        // A watcher may pick someone too
        let watcher = Seen { me: (2, 99), ..Seen::new(2) };
        let (_, draws) = click_picked(&watcher, row_at(0), &mut Picked::default());
        assert_eq!(picked_name(&draws), Some("7ony"));
        assert_eq!(arts(&draws, &[Art::YouKilled, Art::KilledYou]), [(Art::KilledYou, 0, ROWS_Y + 2.0 * ROW)]);
    }

    #[test]
    fn continue_is_clicked_once_and_the_host_then_goes_to_the_lobby() {
        assert_eq!(click(&Seen::new(1), CONTINUE).0, [LobbyCommand::Continue]);
        // Sent, and not yet taken: dark
        assert_eq!(click(&Seen { sent: true, ..Seen::new(1) }, CONTINUE).0, []);
        // The host, once the server says they have continued, has TO LOBBY there
        let host = Seen { host: true, sent: true, continued: vec![(1, 11)], ..Seen::new(1) };
        let (made, draws) = click(&host, CONTINUE);
        assert_eq!(made, [LobbyCommand::ToLobby]);
        assert!(texts(&draws).contains(&"TO LOBBY"));
        // Before then, still CONTINUE, and dark
        assert_eq!(click(&Seen { host: true, sent: true, ..Seen::new(1) }, CONTINUE).0, []);
        // Who has continued is lit green
        let lights = arts(&click(&Seen { continued: vec![(2, 12)], ..Seen::new(0) }, Rect::default()).1, &[Art::Ready]);
        assert_eq!(lights.iter().map(|l| l.2).collect::<Vec<_>>(), [ROWS_Y + 2.0 * ROW]);
    }

    #[test]
    fn a_press_on_continue_does_not_click_to_lobby_when_it_takes_its_place() {
        let mut ui = ui();
        let mut chat = Chat::default();
        let at = [CONTINUE.x + 5.0, CONTINUE.y + 5.0];
        let player = Seen { host: true, ..Seen::new(1) };
        let mut picked = Picked::default();
        ui.begin(Pointer { at, down: true, ..Pointer::default() }, 1);
        results(&mut ui, &player.view(), &mut chat, &mut picked);
        ui.finish();
        // The press was on CONTINUE; by the release, TO LOBBY is there
        let host = Seen { host: true, sent: true, continued: vec![(1, 11)], ..Seen::new(1) };
        ui.begin(Pointer { at, down: false, ..Pointer::default() }, 1);
        assert_eq!(results(&mut ui, &host.view(), &mut chat, &mut picked), []);
        ui.finish();
    }

    #[test]
    fn long_names_are_cut_to_their_box_and_elapsed_time_reads_as_the_original() {
        let mut seen = Seen::new(0);
        seen.results.rows[1].name = "W".repeat(24);
        let t = texts(&click(&seen, Rect::default()).1).into_iter().map(String::from).collect::<Vec<_>>();
        assert!(t.iter().any(|s| s.starts_with("WWW") && s.len() < 24), "{t:?}");
        assert_eq!(elapsed_text(116), "0:01:56");
        assert_eq!(elapsed_text(3725), "1:02:05");
    }
}
