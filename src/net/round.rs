//! Rounds: the phases a session goes through, how a round ends, and what every snapshot says
//! of them (see docs/multiplayer.md, "Our match flow").

use crate::sim::{Side, Sim};

/// Where the session is. `Open` is play with no rounds, coming back after dying.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Open,
    /// Waiting for the round to be started
    Lobby,
    /// The round starts when it runs out, unless it is stopped
    Countdown,
    /// Waiting for everyone to have the round's game
    Loading,
    /// Everyone has it: a moment before they are let go
    Hold,
    Live,
    /// Who won, seen in the level for a moment
    Results,
    /// Everyone on the results screen, until they continue to the lobby
    Debrief,
}

impl Phase {
    /// Whether players are held where they are: behind the lobby and the results screen, which
    /// hide the level, and from the round's start until they are let go. After the round, they
    /// move as they like for a moment, as in the original.
    pub fn frozen(self) -> bool {
        matches!(self, Phase::Lobby | Phase::Countdown | Phase::Loading | Phase::Hold | Phase::Debrief)
    }

    /// Whether a screen is up instead of the level
    pub fn screen(self) -> bool {
        matches!(self, Phase::Lobby | Phase::Countdown | Phase::Debrief)
    }

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(c: u8) -> Option<Phase> {
        [Phase::Open, Phase::Lobby, Phase::Countdown, Phase::Loading, Phase::Hold, Phase::Live, Phase::Results, Phase::Debrief].get(c as usize).copied()
    }
}

/// How a round ended
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Won(Side),
    /// The last one standing, with no sides
    Survived(u8),
    /// No one left, or the time ran out: "Everybody Lost"
    Draw,
    /// A side had no one when the round began, so it did not count
    CalledOff,
}

/// The rounds as a snapshot tells them
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RoundState {
    pub phase: Phase,
    /// The round's number, from 1, or 0 before the first
    pub number: u16,
    /// When (server time) the phase ends, if the clock ends it
    pub ends: Option<f32>,
    /// Rounds each side has won
    pub wins: [u16; 2],
    pub last: Option<Outcome>,
}

/// One side, or one player with no side
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Team {
    Side(Side),
    Player(usize),
}

/// The teams with a player in the game, or with one still alive
pub fn teams(sim: &Sim, alive: bool) -> Vec<Team> {
    let mut teams = Vec::new();
    for (i, p) in sim.players.iter().enumerate() {
        if !p.active || (alive && p.respawn_at.is_some()) {
            continue;
        }
        let team = p.side.map_or(Team::Player(i), Team::Side);
        if !teams.contains(&team) {
            teams.push(team);
        }
    }
    teams
}

/// How the round ends now, if it does: when no more than one team has anyone alive. A grenade
/// still about is waited for, since a dead player's last throw counts.
pub fn judge(sim: &Sim) -> Option<Outcome> {
    let alive = teams(sim, true);
    if alive.len() > 1 || !sim.thrown.is_empty() {
        return None;
    }
    Some(match alive.first() {
        Some(Team::Side(s)) => Outcome::Won(*s),
        Some(Team::Player(p)) => Outcome::Survived(*p as u8),
        None => Outcome::Draw,
    })
}
