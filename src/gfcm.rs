//! The gunfire combat model (`data/kit/gfcm.txt`): what a bullet does to whoever it hits, as the
//! original works it out (RogueSpear.exe 0x5833e0, read by 0x5822c0). Each kind of ammunition
//! leaves the muzzle with so much kill energy, which falls off with the distance flown. The
//! energy that reaches the target picks a row of the kill chart, and a roll out of 100 picks, for
//! the part of the body hit, what it does: nothing, a wound, or incapacitating or killing them.
//!
//! Not read here yet: the stun energy and its chart, and armor, which turns some of the kill
//! energy into stun.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::combat::BodyPart;

/// One kind of ammunition, as the chart's list has it
#[derive(Clone, Debug, PartialEq)]
pub struct Ammo {
    /// As weapon files name it: `9mmP(FMJ)`
    pub name: String,
    pub kill_energy: f32,
    /// How far, in meters, it can kill
    pub max_kill_range: f32,
    /// The energy lost over `d` meters is `d * range_loss + d * d * range_loss_squared`
    pub range_loss: f32,
    pub range_loss_squared: f32,
}

/// What a bullet does to a person
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Effect {
    Nothing,
    Wounded,
    Incapacitated,
    Dead,
}

/// A row of the kill chart: for energy between `min` and `max`, the rolls out of 100 that do
/// each thing to each part of the body (head, chest, abdomen, legs, arms)
#[derive(Clone, Debug, PartialEq)]
struct Row {
    min: f32,
    max: f32,
    parts: [Vec<(Effect, u32, u32)>; 5],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gunfire {
    pub ammo: Vec<Ammo>,
    kill: Vec<Row>,
}

/// The chart's words, with quoted ones ("no effect") kept whole and comments left out
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("");
        let mut rest = line.trim();
        while !rest.is_empty() {
            if let Some(quoted) = rest.strip_prefix('"') {
                let end = quoted.find('"').unwrap_or(quoted.len());
                out.push(quoted[..end].to_string());
                rest = quoted.get(end + 1..).unwrap_or("").trim_start();
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                out.push(rest[..end].to_string());
                rest = rest[end..].trim_start();
            }
        }
    }
    out
}

pub fn read(path: &Path) -> Result<Gunfire> {
    let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    parse(&String::from_utf8_lossy(&bytes)).with_context(|| format!("{}", path.display()))
}

pub fn parse(text: &str) -> Result<Gunfire> {
    let words = words(text);
    let mut at = 0;
    let mut next = || -> Result<&str> {
        let w = words.get(at).context("the chart ends early")?;
        at += 1;
        Ok(w.as_str())
    };
    let number = |w: &str| -> Result<f32> { w.parse().with_context(|| format!("{w} is not a number")) };
    if next()? != "BeginList" {
        bail!("no list of ammunition");
    }
    // Each kind: its name, whether suppressed, a special effect, then (0x5822c0) a number not
    // read here, the kill and stun energy, the ranges they reach, and how they fall off
    let mut ammo = Vec::new();
    loop {
        let name = next()?.to_string();
        if name == "EndList" {
            break;
        }
        next()?;
        next()?;
        let mut v = [0.0; 10];
        for x in &mut v {
            *x = number(next()?)?;
        }
        ammo.push(Ammo { name, kill_energy: v[1], max_kill_range: v[3], range_loss: v[6], range_loss_squared: v[5] });
        // The armor list: lists within a list
        let mut depth = 0;
        loop {
            match next()? {
                "BeginList" => depth += 1,
                "EndList" if depth == 1 => break,
                "EndList" => depth -= 1,
                _ => {}
            }
        }
    }
    // The stun chart, not used yet, then the kill chart, whose rows have one more number
    chart(&mut next, false)?;
    let kill = chart(&mut next, true)?;
    Ok(Gunfire { ammo, kill })
}

fn chart<'a>(next: &mut impl FnMut() -> Result<&'a str>, kill: bool) -> Result<Vec<Row>> {
    if next()? != "BeginChart" {
        bail!("no chart");
    }
    let number = |w: &str| -> Result<f32> { w.parse().with_context(|| format!("{w} is not a number")) };
    let mut rows = Vec::new();
    loop {
        let first = next()?;
        if first == "EndChart" {
            return Ok(rows);
        }
        let (min, max) = (number(first)?, number(next()?)?);
        if kill {
            // What it does to something not a person
            next()?;
        }
        next()?; // BeginList
        let mut parts: [Vec<(Effect, u32, u32)>; 5] = Default::default();
        loop {
            let part = next()?;
            if part == "EndList" {
                break;
            }
            let i = ["head", "chest", "abdomen", "legs", "arms"].iter().position(|p| p.eq_ignore_ascii_case(part))
                .with_context(|| format!("no body part {part}"))?;
            next()?; // BeginList
            loop {
                let state = next()?;
                if state == "EndList" {
                    break;
                }
                let effect = match state.to_ascii_lowercase().as_str() {
                    "no effect" => Effect::Nothing,
                    "wounded" => Effect::Wounded,
                    "incapacitated" => Effect::Incapacitated,
                    "dead" => Effect::Dead,
                    // The stun chart's states, which are not used yet
                    _ => Effect::Nothing,
                };
                let (lo, hi) = (number(next()?)? as u32, number(next()?)? as u32);
                parts[i].push((effect, lo, hi));
            }
        }
        rows.push(Row { min, max, parts });
    }
}

impl Gunfire {
    pub fn ammo(&self, name: &str) -> Option<&Ammo> {
        self.ammo.iter().find(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// The kill energy `ammo` still has after `meters`, none past its reach
    pub fn energy(ammo: &Ammo, meters: f32) -> f32 {
        if meters > ammo.max_kill_range {
            return 0.0;
        }
        ammo.kill_energy - (meters * ammo.range_loss + meters * meters * ammo.range_loss_squared)
    }

    /// What a bullet of `ammo` that flew `meters` does to the `part` of a person it hit, for a
    /// `roll` from 1 to 100: higher rolls do worse
    pub fn effect(&self, ammo: &Ammo, part: BodyPart, meters: f32, roll: u32) -> Effect {
        let energy = Self::energy(ammo, meters);
        let lowest = self.kill.iter().map(|r| r.min).fold(f32::INFINITY, f32::min);
        let highest = self.kill.iter().map(|r| r.max).fold(0.0, f32::max);
        if energy <= 0.0 {
            return Effect::Nothing;
        }
        let energy = energy.clamp(lowest, highest);
        let part = match part {
            BodyPart::Head => 0,
            BodyPart::Torso => 1,
            BodyPart::Abdomen => 2,
            BodyPart::LeftLeg | BodyPart::RightLeg => 3,
            BodyPart::LeftArm | BodyPart::RightArm => 4,
        };
        self.kill.iter().find(|r| (r.min..=r.max).contains(&energy))
            .and_then(|r| r.parts[part].iter().find(|(_, lo, hi)| (*lo..=*hi).contains(&roll)))
            .map_or(Effect::Nothing, |(e, _, _)| *e)
    }
}

/// How many wounds a person takes before they are incapacitated, by their Stamina out of 100
/// (0x5ae470): one more than 1 up to 30, 2 up to 60, 3 up to 80, 4 up to 90 and 5 above
pub fn wounds_to_fall(stamina: f32) -> u32 {
    1 + match stamina {
        s if s <= 30.0 => 1,
        s if s <= 60.0 => 2,
        s if s <= 80.0 => 3,
        s if s <= 90.0 => 4,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> Option<Gunfire> {
        read(Path::new("data/kit/gfcm.txt")).ok()
    }

    #[test]
    fn reads_every_kind_of_ammunition_and_the_kill_chart() {
        let Some(g) = game() else { return };
        let nine = g.ammo("9mmP(FMJ)").unwrap();
        assert_eq!((nine.kill_energy, nine.max_kill_range, nine.range_loss, nine.range_loss_squared), (550.0, 240.0, 1.0, 0.005435));
        // Every weapon's ammunition is there
        for f in std::fs::read_dir("data/kit").unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wpn"))) {
            if let Some(name) = crate::weapon::read(&f).ok().and_then(|w| w.ammo) {
                assert!(g.ammo(&name).is_some(), "{}: no {name}", f.display());
            }
        }
        assert_eq!(g.kill.len(), 10);
        assert!(g.kill.iter().all(|r| r.parts.iter().all(|p| !p.is_empty())));
    }

    #[test]
    fn a_bullet_does_less_the_further_it_flies_and_the_head_is_worst() {
        let Some(g) = game() else { return };
        let nine = g.ammo("9mmP(FMJ)").unwrap();
        // It runs out just about at its reach
        assert!(Gunfire::energy(nine, 240.0).abs() < 5.0);
        assert!(Gunfire::energy(nine, 10.0) > Gunfire::energy(nine, 50.0));
        let worst = |part, meters| (1..=100).map(|roll| g.effect(nine, part, meters, roll)).filter(|e| *e == Effect::Dead).count();
        assert!(worst(BodyPart::Head, 10.0) > worst(BodyPart::Torso, 10.0));
        assert!(worst(BodyPart::Torso, 10.0) > worst(BodyPart::Torso, 150.0));
        // A leg is wounded, never more
        assert!((1..=100).all(|roll| g.effect(nine, BodyPart::LeftLeg, 10.0, roll) <= Effect::Wounded));
    }

    #[test]
    fn hardier_people_take_more_wounds() {
        assert_eq!([20.0, 50.0, 70.0, 85.0, 95.0].map(wounds_to_fall), [2, 3, 4, 5, 6]);
    }
}
