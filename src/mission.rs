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

/// How watchful a character is, which sets how wide and far they see (see `sim::terrorist`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alertness {
    #[default]
    Bored,
    Ready,
    Alert,
}

impl Alertness {
    pub fn parse(s: &str) -> Option<Alertness> {
        [Alertness::Bored, Alertness::Ready, Alertness::Alert].into_iter().find(|a| format!("{a:?}").eq_ignore_ascii_case(s))
    }
}

/// What a team does about a threat (a plan's `ROE`): close in on it, go and see, hold its
/// ground, or close in carefully; hostages are held, or are hostages
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Roe {
    Advance,
    Investigate,
    #[default]
    Defend,
    Cautious,
    HostageCaptive,
    Hostage,
}

/// How fast a team moves (a plan's other `ROE`): walking, or at a run
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Speed {
    #[default]
    Normal,
    Blitz,
}

/// What sets off a trigger plan
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// Any terrorist anywhere suspects, or knows of, a threat
    GloballySuspicious,
    GloballyAware,
    /// The team's threat has come to this
    SuspiciousThreat,
    DistantThreat,
    ImmediateThreat,
    DeadlyThreat,
    /// A threat in one of the mission's zones
    ThreatInZone(u32),
    /// A sound of a kind: `Alarm` or `Combat`
    Sound(String),
}

/// A plan: what the team does about threats, how fast it moves, its steps as written, and the
/// way they take it
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub roe: Roe,
    pub speed: Speed,
    pub steps: Vec<String>,
    pub route: Vec<Step>,
}

/// One step of the way a plan takes a team
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Go to a place (`MoveTo`, or a point of a `PatrolPath`)
    Move(V3),
    /// What is between these is done again and again (`PlanLoop`, `EndLoop`)
    Loop,
    EndLoop,
    /// Move at this many units a second from here on (`ChangeSpeed`)
    Speed(f32),
    /// Stay and guard where they are (`GuardSpot`)
    Guard,
}

/// The way a plan's steps take a team: where it goes, round again or once, and where it stops.
/// A track (`PlanTrack`) is a way taken once; the rest of the steps are not taken yet.
fn route(steps: &[String]) -> Vec<Step> {
    let mut route = Vec::new();
    let mut lines = steps.iter();
    let place = |words: &mut std::str::SplitWhitespace| -> Option<V3> {
        let v: Vec<f32> = words.take(3).filter_map(|n| n.parse().ok()).collect();
        (v.len() == 3).then(|| [v[0], v[1], v[2]])
    };
    while let Some(line) = lines.next() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("MoveTo") => route.extend(place(&mut words).map(Step::Move)),
            Some("PatrolPath") => {
                let n: usize = words.nth(1).and_then(|n| n.parse().ok()).unwrap_or(0);
                for point in lines.by_ref().take(n) {
                    route.extend(place(&mut point.split_whitespace()).map(Step::Move));
                }
            }
            Some("PlanLoop") => route.push(Step::Loop),
            Some("EndLoop") => route.push(Step::EndLoop),
            Some("ChangeSpeed") => route.extend(words.next().and_then(|v| v.parse().ok()).map(Step::Speed)),
            Some("GuardSpot") => route.push(Step::Guard),
            _ => {}
        }
    }
    route
}

/// A plan the team turns to when one of its conditions comes about
#[derive(Clone, Debug, PartialEq)]
pub struct Reaction {
    pub name: String,
    /// Lower first, when more than one could start
    pub priority: u32,
    pub when: Vec<Condition>,
    pub plan: Plan,
}

/// One of a team's characters
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub name: String,
    /// The actor file (`data/actor`): their skills and looks
    pub actor: String,
    /// The kit file (`data/kit`): their weapons and model
    pub kit: String,
    pub alertness: Alertness,
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
    /// Its plan, and the plans it turns to when they are set off
    pub plan: Plan,
    pub reactions: Vec<Reaction>,
}

/// A plan's lines between `BeginPlan` and `EndPlan`
fn plan(lines: &[String]) -> Plan {
    let mut plan = Plan::default();
    for line in lines {
        let mut words = line.split_whitespace();
        match (words.next(), words.next()) {
            (Some("ROE"), Some(word)) => match word.to_ascii_lowercase().as_str() {
                "normal" => plan.speed = Speed::Normal,
                "blitz" => plan.speed = Speed::Blitz,
                "advance" => plan.roe = Roe::Advance,
                "investigate" => plan.roe = Roe::Investigate,
                "defend" => plan.roe = Roe::Defend,
                "cautious" => plan.roe = Roe::Cautious,
                "hostagecaptive" => plan.roe = Roe::HostageCaptive,
                "hostage" => plan.roe = Roe::Hostage,
                _ => {}
            },
            _ => plan.steps.push(line.clone()),
        }
    }
    plan.route = route(&plan.steps);
    plan
}

/// A team's plan and its trigger plans, from the lines after its starting places
fn orders(lines: &[String]) -> Result<(Plan, Vec<Reaction>)> {
    let mut base = Plan::default();
    let mut reactions = Vec::new();
    let mut i = 0;
    let between = |i: &mut usize, end: &str| -> Result<Vec<String>> {
        let from = *i;
        while lines.get(*i).context("a plan ends early")? != end {
            *i += 1;
        }
        *i += 1;
        Ok(lines[from..*i - 1].to_vec())
    };
    while i < lines.len() {
        match lines[i].as_str() {
            "BeginPlan" => {
                i += 1;
                base = plan(&between(&mut i, "EndPlan")?);
            }
            "BeginTriggerPlan" => {
                i += 1;
                let (mut name, mut priority, mut when) = (String::new(), 0, Vec::new());
                while lines.get(i).context("a trigger plan ends early")? != "BeginPlan" {
                    let mut words = lines[i].splitn(2, char::is_whitespace);
                    let (key, rest) = (words.next().unwrap_or(""), words.next().unwrap_or("").trim());
                    match key {
                        "Name" => name = rest.trim_matches('"').to_string(),
                        "Priority" => priority = rest.parse().unwrap_or(0),
                        "Trigger" => {
                            let mut w = rest.split_whitespace();
                            let condition = match w.next().unwrap_or("") {
                                "GloballySuspicious" => Some(Condition::GloballySuspicious),
                                "GloballyAware" => Some(Condition::GloballyAware),
                                "SuspiciousThreat" => Some(Condition::SuspiciousThreat),
                                "DistantThreat" => Some(Condition::DistantThreat),
                                "ImmediateThreat" => Some(Condition::ImmediateThreat),
                                "DeadlyThreat" => Some(Condition::DeadlyThreat),
                                "ThreatInZone" => Some(Condition::ThreatInZone(w.next().and_then(|z| z.parse().ok()).unwrap_or(0))),
                                "Sound" => Some(Condition::Sound(w.next().unwrap_or("").to_string())),
                                _ => None,
                            };
                            when.extend(condition);
                        }
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
                let plan = plan(&between(&mut i, "EndPlan")?);
                if lines.get(i).map(String::as_str) == Some("EndTriggerPlan") {
                    i += 1;
                }
                reactions.push(Reaction { name, priority, when, plan });
            }
            _ => i += 1,
        }
    }
    Ok((base, reactions))
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

/// How far the mission's characters see, from the mission file beside a difficulty file
/// (`RM01Med.mdf` has `RM01.mis`): its Max LOS Distance, 4000 when it does not say
pub fn max_los(difficulty_file: &Path) -> f32 {
    let stem = difficulty_file.file_stem().map_or_else(String::new, |s| s.to_string_lossy().to_string());
    let level = ["Easy", "Med", "Hard"].iter().find_map(|d| stem.strip_suffix(d)).unwrap_or(&stem);
    let dir = difficulty_file.parent().unwrap_or(Path::new("."));
    crate::util::find_file(dir, level, "mis")
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| {
            String::from_utf8_lossy(&bytes).lines()
                .find(|l| l.to_ascii_lowercase().contains("// max los distance"))
                .and_then(|l| l.split("//").next()?.trim().parse().ok())
        })
        .unwrap_or(4000.0)
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
            let alertness = Alertness::parse(&alertness).unwrap_or_default();
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
        let mut lines = Vec::new();
        loop {
            let line = next("EndTeamPlan")?;
            if line == "EndTeamPlan" {
                break;
            }
            lines.push(line.to_string());
        }
        let (plan, reactions) = orders(&lines).with_context(|| format!("team {name}'s plans"))?;
        teams.push(Team { name, terrorists, members, starts, plan, reactions });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEAM: &str = "BeginTeams\n// a comment\n\"Response Team\"\t// Team Name\nTerrorist\t// Team Type\n1\n0\n0\n\n\
        \"Terrorist\"\n\"Response #1\"\n\"TER_arab_cat2_02.ter\"\n\"TER_arab_cat2_AK47.kit\"\n0\nBored\nCellPhone\nStand\n\
        \"Terrorist\"\n\"Response #2\"\n\"TER_arab_cat1_01.ter\"\n\"TER_arab_cat1_AK47.kit\"\n0\nReady\nNone\nKneel\n\
        BeginTeamPlan\n2\t// Number Starting Locations\n\t29809.5 32777 34509.1 0\n\t31424.4 32777 34336 64\n\
        BeginPlan\nROE Investigate\nPlanLoop\nMoveTo 1 2 3 0\nEndLoop\nEndPlan\n\
        BeginTriggerPlan\nName \"React\"\nPriority 2\nTrigger ImmediateThreat\nTrigger ThreatInZone 3\nEndTriggers\n\
        BeginPlan\nROE Advance\nROE Blitz\nEndPlan\nEndTriggerPlan\nEndTeamPlan\nEndTeams\n";

    #[test]
    fn reads_a_team_of_two_and_where_it_can_start() {
        let teams = parse(TEAM).unwrap();
        assert_eq!(teams.len(), 1);
        let t = &teams[0];
        assert!(t.terrorists);
        assert_eq!(t.name, "Response Team");
        assert_eq!(t.members.iter().map(|m| (m.name.as_str(), m.stance, m.alertness)).collect::<Vec<_>>(),
            [("Response #1", Stance::Stand, Alertness::Bored), ("Response #2", Stance::Kneel, Alertness::Ready)]);
        assert_eq!(t.members[1].kit, "TER_arab_cat1_AK47.kit");
        assert_eq!(t.starts, [([29809.5, 32777.0, 34509.1], 0.0), ([31424.4, 32777.0, 34336.0], 64.0)]);
        assert_eq!((t.plan.roe, t.plan.speed), (Roe::Investigate, Speed::Normal));
        assert_eq!(t.plan.steps, ["PlanLoop", "MoveTo 1 2 3 0", "EndLoop"]);
        assert_eq!(t.plan.route, [Step::Loop, Step::Move([1.0, 2.0, 3.0]), Step::EndLoop]);
        let r = &t.reactions[0];
        assert_eq!((r.name.as_str(), r.priority, r.plan.roe, r.plan.speed), ("React", 2, Roe::Advance, Speed::Blitz));
        assert_eq!(r.when, [Condition::ImmediateThreat, Condition::ThreatInZone(3)]);
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

    #[test]
    fn a_mission_says_how_far_its_characters_see() {
        let path = Path::new("data/mission/RM02Med.mdf");
        if path.exists() {
            assert_eq!(max_los(path), 4000.0);
        }
    }
}
