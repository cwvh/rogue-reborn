//! What a character carries and wears, from its kit file (`data/kit/*.kit`): the weapon in their
//! hands and the model. After the kit's kind and name come its weapons, a line each (`"Primary"
//! "tw_ak47.wpn" 5`, with how many magazines), then the directory of `data/character` the model
//! is in, how many models it has (the same body in less and less detail) and their names.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kit {
    /// The most detailed model the kit dresses its character in, if it is in the game
    pub model: Option<PathBuf>,
    /// The weapon in their hands: the primary, or else the secondary (`tw_ak47.wpn`), and how
    /// many magazines they carry for it
    pub weapon: Option<(String, u32)>,
    /// The secondary, when the primary is in their hands, and its magazines
    pub sidearm: Option<(String, u32)>,
}

pub fn read(kit: &Path) -> Result<Kit> {
    let bytes = std::fs::read(kit).with_context(|| format!("could not read {}", kit.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let (model, weapon) = parse(&text);
    let model = model.and_then(|(dir, name)| crate::util::find_file(&Path::new("data/character").join(dir), &name, "crp"));
    Ok(Kit { model, weapon, sidearm: sidearm(&text) })
}

/// The model's directory and the name of its first model, and the weapon in hand
pub fn parse(text: &str) -> (Option<(String, String)>, Option<(String, u32)>) {
    let lines = words(text);
    // The directory is a word alone on its line, then a count, then that many names
    let model = (0..lines.len()).find_map(|i| {
        let [dir] = lines[i][..] else { return None };
        let n: usize = lines.get(i + 1).filter(|l| l.len() == 1)?[0].parse().ok()?;
        let names = lines.get(i + 2..i + 2 + n)?;
        (n > 0 && names.iter().all(|l| l.len() == 1)).then(|| (dir.to_string(), names[0][0].to_string()))
    });
    (model, carried(&lines, "Primary").or_else(|| carried(&lines, "Secondary")))
}

/// The secondary weapon and its magazines, if the kit has a primary too
pub fn sidearm(text: &str) -> Option<(String, u32)> {
    let lines = words(text);
    carried(&lines, "Primary").and(carried(&lines, "Secondary"))
}

fn words(text: &str) -> Vec<Vec<&str>> {
    text.lines()
        .map(|l| l.split("//").next().unwrap_or("").split_whitespace().map(|w| w.trim_matches('"')).collect::<Vec<_>>())
        .filter(|l| !l.is_empty())
        .collect()
}

/// The weapon in a slot (`Primary` or `Secondary`) and its magazines
fn carried(lines: &[Vec<&str>], slot: &str) -> Option<(String, u32)> {
    lines.iter().find(|l| l.len() >= 2 && l[0].eq_ignore_ascii_case(slot))
        .map(|l| (l[1].to_string(), l.get(2).and_then(|n| n.parse().ok()).unwrap_or(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_model_after_the_weapons() {
        let kit = "\"TERRORIST\"\n\"TER_arab_cat1\"\n\"NONE\"\n\"NONE\"\n\"Primary\" \"tw_ak47.wpn\" 5\n\
            \"Secondary\" \"sw_de357.wpn\" 5\n\"TER_arabturk\"\n3\n\"arab_cat1_a\"\n\"arab_cat1_b\"\n\"arab_cat1_c\"\n1\n\"TR_ARM1.eq\" 1\n";
        assert_eq!(parse(kit), (Some(("TER_arabturk".to_string(), "arab_cat1_a".to_string())), Some(("tw_ak47.wpn".to_string(), 5))));
        assert_eq!(sidearm(kit), Some(("sw_de357.wpn".to_string(), 5)));
        // A hostage's kit has no weapons
        let kit = "\"OTHER\"\n\"hsm_waiter\"\n\"NONE\"\n\"NONE\"\n\"HOS_male\"\n3\n\"hsm_waiter_a\"\n\"hsm_waiter_b\"\n\"hsm_waiter_c\"\n1\n\"H_ARM1.eq\" 1\n";
        assert_eq!(parse(kit), (Some(("HOS_male".to_string(), "hsm_waiter_a".to_string())), None));
        // Some carry only a sidearm
        let kit = "\"TERRORIST\"\n\"TER_rus_gen_de\"\n\"NONE\"\n\"NONE\"\n\"Secondary\" \"sw_de357.wpn\" 6\n\"TER_Russian\"\n3\n\"Rus_gen_a\"\n\"Rus_gen_b\"\n\"Rus_gen_c\"\n";
        assert_eq!(parse(kit).1, Some(("sw_de357.wpn".to_string(), 6)));
        assert_eq!(sidearm(kit), None);
    }

    #[test]
    fn every_kit_a_mission_names_has_a_model_with_a_skeleton_and_a_weapon_that_is_there() {
        let Ok(dir) = std::fs::read_dir("data/mission") else { return };
        let mut kits = std::collections::BTreeSet::new();
        for path in dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mdf"))) {
            for team in crate::mission::read(&path).unwrap() {
                kits.extend(team.members.into_iter().map(|m| m.kit));
            }
        }
        assert!(kits.len() > 20, "{} kits", kits.len());
        for kit in &kits {
            let path = crate::util::find_file(Path::new("data/kit"), Path::new(kit).file_stem().unwrap().to_str().unwrap(), "kit")
                .unwrap_or_else(|| panic!("no kit {kit}"));
            let k = read(&path).unwrap();
            let model = k.model.unwrap_or_else(|| panic!("{kit} names no model that is there"));
            let stem = model.file_stem().unwrap().to_string_lossy().to_string();
            assert!(crate::util::find_file(model.parent().unwrap(), &stem, "skl").is_some(), "{} has no skeleton", model.display());
            if let Some((w, _)) = k.weapon {
                let w = crate::weapon::read(&Path::new("data/kit").join(&w)).unwrap_or_else(|e| panic!("{kit}: {e:#}"));
                assert!(Path::new("data/model").join(&w.model).exists(), "{kit}: no {}", w.model);
            }
        }
    }
}
