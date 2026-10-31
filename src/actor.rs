//! Reader for actor files: team members' (`data/actor/*.act`), and terrorists' and hostages'
//! (`*.ter`, `*.hst`).
//!
//! These are text files with one value per line and a `//` comment naming
//! the field. Fields are matched by their comment, not by line position, since
//! only some of them are needed here: name, sex, model scale, the team members'
//! per-uniform face textures and the textures the others' models wear.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sex {
    Male,
    Female,
}

#[derive(Clone, Debug)]
pub struct Actor {
    /// File stem, e.g. `Chavez`
    pub id: String,
    /// Display name, e.g. `Ding Chavez`
    pub name: String,
    pub sex: Sex,
    /// Per-axis model scale
    pub scale: [f32; 3],
    /// Face texture file names (`.bmp` in the file; the game data has them as
    /// `.rsb` in `data/texture/faces`)
    pub hrt_face: Option<String>,
    pub camo_face: Option<String>,
    pub desert_face: Option<String>,
    pub winter_face: Option<String>,
    /// The textures a terrorist's or hostage's model wears for its eyes, lips, chest and hair,
    /// in place of the model's own
    pub eyes: Option<String>,
    pub lips: Option<String>,
    pub chest: Option<String>,
    pub hair: Option<String>,
}

impl Actor {
    /// The texture this actor wears on a part of their model whose own is `material`, if not
    /// the part's own: the original matches the part's name against `_teyes`, `_heyes` and the
    /// like (RogueSpear.exe 0x5d3fb0)
    pub fn texture_for(&self, material: &str) -> Option<&str> {
        let name = material.to_ascii_lowercase();
        let slots = [("eyes", &self.eyes), ("lips", &self.lips), ("chest", &self.chest), ("hair", &self.hair)];
        slots.into_iter()
            .find(|(slot, _)| name.contains(&format!("_t{slot}")) || name.contains(&format!("_h{slot}")))
            .and_then(|(_, t)| t.as_deref())
    }
}

pub fn read(path: &Path) -> Result<Actor> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(parse(&id, &String::from_utf8_lossy(&bytes)))
}

pub fn parse(id: &str, text: &str) -> Actor {
    let mut actor = Actor {
        id: id.to_string(),
        name: id.to_string(),
        sex: Sex::Male,
        scale: [1.0; 3],
        hrt_face: None,
        camo_face: None,
        desert_face: None,
        winter_face: None,
        eyes: None,
        lips: None,
        chest: None,
        hair: None,
    };

    for line in text.lines() {
        let Some((value, comment)) = line.split_once("//") else { continue };
        let (value, comment) = (value.trim(), comment.trim().to_ascii_lowercase());
        let unquoted = value.trim_matches('"').to_string();

        if comment.starts_with("actor name") {
            actor.name = unquoted;
        } else if comment.starts_with("sex") {
            actor.sex = if unquoted.eq_ignore_ascii_case("female") { Sex::Female } else { Sex::Male };
        } else if comment.starts_with("scale") {
            // Team members' are written 1, .97, 1 and the others' 1.00 1.00 1.0
            let v: Vec<f32> = value.split([',', ' ', '\t']).filter_map(|n| n.trim().parse().ok()).collect();
            if v.len() == 3 {
                actor.scale = [v[0], v[1], v[2]];
            }
        } else if comment.contains("face texture") && !unquoted.is_empty() {
            if comment.starts_with("hrt") {
                actor.hrt_face = Some(unquoted);
            } else if comment.starts_with("camo") {
                actor.camo_face = Some(unquoted);
            } else if comment.starts_with("desert") {
                actor.desert_face = Some(unquoted);
            } else if comment.starts_with("winter") {
                actor.winter_face = Some(unquoted);
            }
        } else if let Some(slot) = ["eye", "lip", "chest", "hair"].iter().position(|s| comment.starts_with(&format!("{s} texture"))) {
            // none.bmp leaves the model's own
            let texture = (!unquoted.eq_ignore_ascii_case("none.bmp") && !unquoted.is_empty()).then_some(unquoted);
            *[&mut actor.eyes, &mut actor.lips, &mut actor.chest, &mut actor.hair][slot] = texture;
        }
    }
    actor
}

/// All actors in a directory, sorted by file name
pub fn read_all(dir: &Path) -> Vec<Actor> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("act")))
        .collect();
    files.sort();
    files.iter().filter_map(|f| read(f).ok()).collect()
}

/// The `.rsb` for a face texture name such as `chavez_hrt_face.bmp` in `dir`,
/// found without regard to case, and the matching `_blink` texture if one
/// exists.
pub fn find_face_textures(dir: &Path, face: &str) -> Option<(PathBuf, Option<PathBuf>)> {
    let stem = Path::new(face).file_stem()?.to_string_lossy().to_lowercase();
    let blink_stem = format!("{stem}_blink");
    let mut main = None;
    let mut blink = None;
    for p in std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()) {
        if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("rsb")) {
            continue;
        }
        let s = p.file_stem()?.to_string_lossy().to_lowercase();
        if s == stem {
            main = Some(p);
        } else if s == blink_stem {
            blink = Some(p);
        }
    }
    main.map(|m| (m, blink))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fields_by_comment() {
        let text = "\"Ding Chavez\"\t// Actor Name\n\"MALE\"\t// Sex, MALE or FEMALE\n\
                    1, .97, 1\t// Scale\n\"chavez_hrt_face.bmp\"\t// HRT light face texture\n\
                    \"chavez_camo_head.bmp\"\t// Camo light face texture\n\
                    \"chavez_des_head.bmp\"\t// Desert light face texture\n";
        let a = parse("Chavez", text);
        assert_eq!(a.name, "Ding Chavez");
        assert_eq!(a.sex, Sex::Male);
        assert_eq!(a.scale, [1.0, 0.97, 1.0]);
        assert_eq!(a.hrt_face.as_deref(), Some("chavez_hrt_face.bmp"));
        assert_eq!(a.camo_face.as_deref(), Some("chavez_camo_head.bmp"));
        assert_eq!(a.desert_face.as_deref(), Some("chavez_des_head.bmp"));
        assert_eq!(a.winter_face, None);
    }

    #[test]
    fn a_terrorist_wears_their_own_eyes_and_lips() {
        let text = "1.00 1.00 1.0\t// Scale\n\"arab_cat1_teyes_2.rsb\"\t// Eye texture\n\
                    \"none.bmp\"\t// Lip texture\n\"arab_cat1_tchest.rsb\"\t// Chest texture\n\
                    \"arab_cat1_thair_2.rsb\"\t// Hair texture\n";
        let a = parse("TER_arab_cat1_03", text);
        assert_eq!(a.scale, [1.0; 3]);
        assert_eq!(a.texture_for("arab_cat1_teyes.bmp"), Some("arab_cat1_teyes_2.rsb"));
        // none.bmp keeps the model's own, and other parts keep theirs
        assert_eq!(a.texture_for("arab_cat1_tlips.bmp"), None);
        assert_eq!(a.texture_for("arab_cat1_head.bmp"), None);
        assert_eq!(a.texture_for("rus_gen_hhair.bmp"), Some("arab_cat1_thair_2.rsb"));
    }

    #[test]
    fn every_actor_file_has_an_hrt_face_that_exists() {
        let actors = read_all(Path::new("data/actor"));
        if actors.is_empty() {
            eprintln!("skipping: data/actor not found");
            return;
        }
        assert_eq!(actors.len(), 30);
        for a in &actors {
            let face = a.hrt_face.as_deref().unwrap_or_else(|| panic!("{} has no HRT face", a.id));
            assert!(
                find_face_textures(Path::new("data/texture/faces"), face).is_some(),
                "{}: {face} not found", a.id
            );
        }
    }

    #[test]
    fn chavez_has_a_blink_texture() {
        let Some((main, blink)) = find_face_textures(Path::new("data/texture/faces"), "chavez_hrt_face.bmp")
        else { return };
        assert!(main.exists());
        assert!(blink.is_some_and(|b| b.exists()));
    }
}
