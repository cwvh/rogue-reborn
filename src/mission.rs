//! The teams a mission puts in a level, from its difficulty file (`data/mission/RM01Med.mdf` and
//! the like): who is in each, and where it starts. Each team has one or more characters, the
//! places it can start (one is picked), and its plans, which are kept as they are written.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

type V3 = [f32; 3];

/// How hard a mission is: each has a difficulty file of its own
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    #[default]
    Medium,
    Hard,
}

impl Difficulty {
    pub fn parse(s: &str) -> Option<Difficulty> {
        match s.to_ascii_lowercase().as_str() {
            "easy" => Some(Difficulty::Easy),
            "medium" | "med" => Some(Difficulty::Medium),
            "hard" => Some(Difficulty::Hard),
            _ => None,
        }
    }

    /// The name the difficulty file has after the level's
    fn suffix(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Medium => "Med",
            Difficulty::Hard => "Hard",
        }
    }
}

/// How a character waits until something happens, in the order the original numbers them
/// (RogueSpear.exe 0x7ff550)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Stance {
    Kneel,
    Cower,
    Sit,
    Squat,
    #[default]
    Stand,
    Lean,
    SitInChair,
    Slouch,
}

impl Stance {
    pub const ALL: [Stance; 8] = [Stance::Kneel, Stance::Cower, Stance::Sit, Stance::Squat, Stance::Stand, Stance::Lean, Stance::SitInChair, Stance::Slouch];

    /// As a mission writes it, in any case
    pub fn parse(s: &str) -> Option<Stance> {
        Stance::ALL.into_iter().find(|t| format!("{t:?}").eq_ignore_ascii_case(s))
    }
}

/// One of a team's characters
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub name: String,
    /// The actor file (`data/actor`): their skills and looks
    pub actor: String,
    /// The kit file (`data/kit`): their weapons and model
    pub kit: String,
    /// Bored, Ready or Alert
    pub alertness: String,
    pub stance: Stance,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Team {
    pub name: String,
    /// Terrorists, or hostages
    pub terrorists: bool,
    pub members: Vec<Member>,
    /// Where the team can start, and which way it faces there, in 256ths of a turn
    pub starts: Vec<(V3, f32)>,
    /// Its plan and the plans its triggers start, line by line, as written
    pub plans: Vec<String>,
}

/// The teams of the mission on the level at `map_path` at a difficulty, if it has one and it reads
pub fn teams(map_path: &Path, difficulty: Difficulty) -> Option<(PathBuf, Vec<Team>)> {
    let path = find(map_path, difficulty)?;
    match read(&path) {
        Ok(teams) => Some((path, teams)),
        Err(e) => {
            eprintln!("{e:#}");
            None
        }
    }
}

/// The difficulty file of the mission on the level at `map_path` (`RM01.map` is `RM01Med.mdf`),
/// if there is one
pub fn find(map_path: &Path, difficulty: Difficulty) -> Option<PathBuf> {
    let stem = map_path.file_stem()?.to_string_lossy().to_uppercase();
    let path = Path::new("data/mission").join(format!("{stem}{}.mdf", difficulty.suffix()));
    path.exists().then_some(path)
}

pub fn read(path: &Path) -> Result<Vec<Team>> {
    let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    parse(&String::from_utf8_lossy(&bytes)).with_context(|| format!("{}", path.display()))
}

/// The teams between `BeginTeams` and `EndTeams`
pub fn parse(text: &str) -> Result<Vec<Team>> {
    // What a line says, without its comment
    let mut lines = text.lines().map(|l| l.split("//").next().unwrap_or("").trim()).filter(|l| !l.is_empty()).peekable();
    let mut next = |what: &str| lines.next().with_context(|| format!("the file ends before {what}"));
    while next("BeginTeams")? != "BeginTeams" {}
    let unquote = |l: &str| l.trim_matches('"').to_string();
    let mut teams = Vec::new();
    loop {
        let first = next("a team or EndTeams")?;
        if first == "EndTeams" {
            return Ok(teams);
        }
        let name = unquote(first);
        let terrorists = match next("the team's type")? {
            "Terrorist" => true,
            "Hostage" => false,
            other => bail!("team {name} is of no type the game has: {other}"),
        };
        // Starts active, whether its place is known, whether its icon is shown
        for what in ["whether it starts active", "whether its place is known", "whether its icon is shown"] {
            next(what)?;
        }
        let mut members = Vec::new();
        loop {
            let line = next("a character or BeginTeamPlan")?;
            if line == "BeginTeamPlan" {
                break;
            }
            // The character's type comes first, then the rest of them
            let mut field = |what: &str| next(what).map(unquote);
            let name = field("the character's name")?;
            let actor = field("the actor file")?;
            let kit = field("the kit file")?;
            field("whether they are a primary objective")?;
            let alertness = field("their alertness")?;
            field("their radio")?;
            let stance = field("their stance")?;
            // The original stands a character whose stance it does not know (0x598cd0)
            let stance = Stance::parse(&stance).unwrap_or_default();
            members.push(Member { name, actor, kit, alertness, stance });
        }
        let count: usize = next("how many places it starts at")?.parse().with_context(|| format!("team {name}'s count of starting places"))?;
        let mut starts = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            let v: Vec<f32> = next("a starting place")?.split_whitespace().map(|n| n.parse()).collect::<Result<_, _>>()
                .with_context(|| format!("team {name}'s starting place"))?;
            let [x, y, z, heading] = v[..] else { bail!("team {name}'s starting place is not x y z and a heading") };
            starts.push(([x, y, z], heading));
        }
        let mut plans = Vec::new();
        loop {
            let line = next("EndTeamPlan")?;
            if line == "EndTeamPlan" {
                break;
            }
            plans.push(line.to_string());
        }
        teams.push(Team { name, terrorists, members, starts, plans });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEAM: &str = "BeginTeams\n// a comment\n\"Response Team\"\t// Team Name\nTerrorist\t// Team Type\n1\n0\n0\n\n\
        \"Terrorist\"\n\"Response #1\"\n\"TER_arab_cat2_02.ter\"\n\"TER_arab_cat2_AK47.kit\"\n0\nBored\nCellPhone\nStand\n\
        \"Terrorist\"\n\"Response #2\"\n\"TER_arab_cat1_01.ter\"\n\"TER_arab_cat1_AK47.kit\"\n0\nReady\nNone\nKneel\n\
        BeginTeamPlan\n2\t// Number Starting Locations\n\t29809.5 32777 34509.1 0\n\t31424.4 32777 34336 64\n\
        BeginPlan\nROE Investigate\nEndPlan\nEndTeamPlan\nEndTeams\n";

    #[test]
    fn reads_a_team_of_two_and_where_it_can_start() {
        let teams = parse(TEAM).unwrap();
        assert_eq!(teams.len(), 1);
        let t = &teams[0];
        assert!(t.terrorists);
        assert_eq!(t.name, "Response Team");
        assert_eq!(t.members.iter().map(|m| (m.name.as_str(), m.stance, m.alertness.as_str())).collect::<Vec<_>>(),
            [("Response #1", Stance::Stand, "Bored"), ("Response #2", Stance::Kneel, "Ready")]);
        assert_eq!(t.members[1].kit, "TER_arab_cat1_AK47.kit");
        assert_eq!(t.starts, [([29809.5, 32777.0, 34509.1], 0.0), ([31424.4, 32777.0, 34336.0], 64.0)]);
        assert_eq!(t.plans, ["BeginPlan", "ROE Investigate", "EndPlan"]);
    }

    #[test]
    fn reads_every_mission_in_the_game() {
        let Ok(dir) = std::fs::read_dir("data/mission") else { return };
        let mut read = 0;
        for path in dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mdf"))) {
            let teams = super::read(&path).unwrap_or_else(|e| panic!("{e:#}"));
            assert!(teams.iter().any(|t| t.terrorists), "{} has no terrorists", path.display());
            assert!(teams.iter().all(|t| !t.members.is_empty() && !t.starts.is_empty()), "{}", path.display());
            read += 1;
        }
        assert!(read == 0 || read == 54, "{read} missions");
    }
}
