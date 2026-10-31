//! What a character carries and wears, from its kit file (`data/kit/*.kit`): only the model is
//! read here. After the kit's kind, name and weapons comes the directory of `data/character` the
//! model is in, how many models it has (the same body in less and less detail) and their names.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The most detailed model a kit dresses its character in, if the file names one
pub fn model(kit: &Path) -> Result<Option<PathBuf>> {
    let bytes = std::fs::read(kit).with_context(|| format!("could not read {}", kit.display()))?;
    Ok(parse(&String::from_utf8_lossy(&bytes)).and_then(|(dir, name)| {
        crate::util::find_file(&Path::new("data/character").join(dir), &name, "crp")
    }))
}

/// The model's directory and the name of its first model
pub fn parse(text: &str) -> Option<(String, String)> {
    let lines: Vec<Vec<&str>> = text.lines()
        .map(|l| l.split("//").next().unwrap_or("").split_whitespace().map(|w| w.trim_matches('"')).collect::<Vec<_>>())
        .filter(|l| !l.is_empty())
        .collect();
    // The directory is a word alone on its line, then a count, then that many names
    (0..lines.len()).find_map(|i| {
        let [dir] = lines[i][..] else { return None };
        let n: usize = lines.get(i + 1).filter(|l| l.len() == 1)?[0].parse().ok()?;
        let names = lines.get(i + 2..i + 2 + n)?;
        (n > 0 && names.iter().all(|l| l.len() == 1)).then(|| (dir.to_string(), names[0][0].to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_model_after_the_weapons() {
        let kit = "\"TERRORIST\"\n\"TER_arab_cat1\"\n\"NONE\"\n\"NONE\"\n\"Primary\" \"tw_ak47.wpn\" 5\n\
            \"Secondary\" \"sw_de357.wpn\" 5\n\"TER_arabturk\"\n3\n\"arab_cat1_a\"\n\"arab_cat1_b\"\n\"arab_cat1_c\"\n1\n\"TR_ARM1.eq\" 1\n";
        assert_eq!(parse(kit), Some(("TER_arabturk".to_string(), "arab_cat1_a".to_string())));
        // A hostage's kit has no weapons
        let kit = "\"OTHER\"\n\"hsm_waiter\"\n\"NONE\"\n\"NONE\"\n\"HOS_male\"\n3\n\"hsm_waiter_a\"\n\"hsm_waiter_b\"\n\"hsm_waiter_c\"\n1\n\"H_ARM1.eq\" 1\n";
        assert_eq!(parse(kit), Some(("HOS_male".to_string(), "hsm_waiter_a".to_string())));
    }

    #[test]
    fn every_kit_a_mission_names_has_a_model_with_a_skeleton() {
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
            let model = model(&path).unwrap().unwrap_or_else(|| panic!("{kit} names no model that is there"));
            let stem = model.file_stem().unwrap().to_string_lossy().to_string();
            assert!(crate::util::find_file(model.parent().unwrap(), &stem, "skl").is_some(), "{} has no skeleton", model.display());
        }
    }
}
