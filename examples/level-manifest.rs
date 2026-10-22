// Every file the viewer reads from data/ to play a level, one path per line:
//
//   level-manifest [MAP] [--model CRP] [--actor NAME]
//
// MAP defaults to citystreet_large, the model to the default soldier and the actor (the one
// face the characters wear) to Chavez. Each file is found the way the viewer finds it, so run
// from the directory holding data/. Anything that cannot be found is reported and the exit
// status is 1: run it from an unpacked copy to check the copy is complete.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rogue_reborn::sim::characters::REACTION_MOTIONS;
use rogue_reborn::util::find_file;
use rogue_reborn::{actor, ambience, crp, door, map, weapon};

/// The motions the player plays, as main.rs loads them for `PlayerMotions`
const PLAYER_MOTIONS: [&str; 24] = ["up_cyc_mp5k", "tmu_wk1", "tmu_rn", "tmu_bk1", "tms_l2", "tms_r2", "tms_l1", "tms_r1", "fire_up_mp5k", "twm_mp2", "twm_mp1", "gr_ov_up",
    "crouch_arms_mp5k", "tmc_fd", "rmc_rn", "tmc_bk", "tmc_lf", "tmc_rt", "up_cr", "cr_up", "twm_mp4", "twm_mp3", "tmu_plf", "tmc_plf"];
/// What other characters idle with, unless --anim says otherwise
const IDLE_MOTION: &str = "up_none_cyc";
/// The sprites main.rs loads from data/texture: bullet holes, dust, sparks and a blast
const SPRITES: [&str; 7] = ["SP_wood_hit_1", "Bullet_hole_simple_1", "TGAdirt_cloud", "Sparks1", "TGAgrenade_explosion_1", "TGAgrenade_blast_03", "TGAgrenade_smoke_ring_1"];
/// Every sound main.rs and the sim play by name, besides the level's ambience and doors
const SOUNDS: [&str; 27] = [
    "C_SB5KB.WAV", "C_SB5KR.WAV", "C_SBMAG1.WAV", "F_HWCON1.WAV", "F_HRCON1.WAV",
    "C_BLRIC1.WAV", "C_BLRIC2.WAV", "C_BLWOD.WAV", "C_BLMTTK.WAV", "C_BLMTTN.WAV", "C_BLEAR.WAV", "C_BLCON.WAV",
    "C_GBRKMD.WAV", "C_BHTHT.WAV", "C_GRMET.WAV", "C_GRWOD.WAV", "C_GREAR.WAV", "C_GRCON.WAV", "C_GRFRAG.WAV",
    "English/tm_wnd1.wav", "English/tm_wnd2.wav", "English/tm_wnd3.wav", "English/tm_wnd4.wav", "English/tm_wnd5.wav",
    "English/tm_inc1.wav", "English/tm_inc2.wav", "English/tm_inc3.wav",
];

struct Manifest {
    files: BTreeSet<PathBuf>,
    missing: Vec<String>,
}

impl Manifest {
    fn add(&mut self, what: &str, found: Option<PathBuf>) -> Option<PathBuf> {
        match &found {
            Some(p) => { self.files.insert(p.clone()); }
            None => self.missing.push(what.to_string()),
        }
        found
    }
}

fn main() {
    let mut map_path = PathBuf::from("data/map/m00/citystreet_large.map");
    let mut model = PathBuf::from("data/character/team_hrt/blue_M_a.CRP");
    let mut actor_name = "Chavez".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--model" => model = args.next().map(PathBuf::from).unwrap_or(model),
            "--actor" => actor_name = args.next().unwrap_or(actor_name),
            _ => map_path = PathBuf::from(a),
        }
    }
    let mut m = Manifest { files: BTreeSet::new(), missing: Vec::new() };

    // The level, its textures, ambience and spawn points
    let level = match map::read(&map_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}: {e:#}", map_path.display());
            std::process::exit(1);
        }
    };
    m.files.insert(map_path.clone());
    let map_num = rogue_reborn::levels::extract_map_num(&map_path).unwrap_or_else(|| "m00".to_string());
    for mat in &level.materials.materials {
        m.add(&format!("texture {}", mat.filename), level_texture(&map_num, &mat.filename));
    }
    // Which textures are keyed out, and which let bullets and grenades through
    m.add("texture properties", Some(PathBuf::from(rogue_reborn::cxp::PATH)).filter(|p| p.exists()));
    let ass = map_path.with_extension("ass");
    if let Some(def) = ambience::read(&ass) {
        m.files.insert(ass);
        let names = def.loops.iter().map(|l| l.file.clone()).chain(def.spots.iter().flat_map(|s| s.sounds.iter().map(|x| x.0.clone())));
        for name in names {
            m.add(&format!("ambient sound {name}"), sound(&name));
        }
    }
    for d in door::doors(&level) {
        for name in [&d.open_sound, &d.close_sound] {
            if !name.is_empty() && !name.eq_ignore_ascii_case("none") {
                m.add(&format!("door sound {name}"), sound(name));
            }
        }
    }
    let missions = missions_naming(&map_path);
    if missions.is_empty() {
        m.missing.push("a mission file naming the map (spawn points)".into());
    }
    m.files.extend(missions);

    // The character: model, skeleton, the texture of each part, and the motions it plays
    m.files.insert(model.clone());
    let dir = model.parent().unwrap_or(Path::new("."));
    let stem = model.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    m.add("skeleton", find_file(dir, &stem, "skl"));
    match crp::read(&model) {
        Ok(c) => {
            for part in &c.parts {
                m.add(&format!("character texture {}", part.material), character_texture(dir, &part.material));
            }
        }
        Err(e) => m.missing.push(format!("{}: {e:#}", model.display())),
    }
    for name in std::iter::once(IDLE_MOTION).chain(REACTION_MOTIONS).chain(PLAYER_MOTIONS) {
        m.add(&format!("motion {name}"), find_file(Path::new("data/motion"), name, "dam"));
    }

    // One team member, for the face the characters wear
    let actor_file = m.add(&format!("actor {actor_name}"), find_file(Path::new("data/actor"), &actor_name, "act"));
    if let Some(face) = actor_file.and_then(|p| actor::read(&p).ok()).and_then(|a| a.hrt_face) {
        match actor::find_face_textures(Path::new("data/texture/faces"), &face) {
            Some((main, blink)) => {
                m.files.insert(main);
                m.files.extend(blink);
            }
            None => m.missing.push(format!("face {face}")),
        }
    }

    // The gun and the grenade, each with the texture of its first material
    if let Some(wpn) = m.add("weapon", Some(PathBuf::from("data/kit/pw_mp5k.wpn")).filter(|p| p.exists())) {
        match weapon::read(&wpn) {
            Ok(w) => model_and_texture(&mut m, &w.model),
            Err(e) => m.missing.push(format!("{e:#}")),
        }
    }
    model_and_texture(&mut m, "rsw_smokegrenade.qob");
    for stem in SPRITES {
        m.add(&format!("sprite {stem}"), find_file(Path::new("data/texture"), stem, "rsb"));
    }

    // Sounds, and their volumes
    m.add("soundvol.txt", Some(PathBuf::from("data/sound/soundvol.txt")).filter(|p| p.exists()));
    for name in SOUNDS {
        m.add(&format!("sound {name}"), sound(name));
    }

    for f in &m.files {
        println!("{}", f.to_string_lossy().replace('\\', "/"));
    }
    for what in &m.missing {
        eprintln!("missing: {what}");
    }
    if !m.missing.is_empty() {
        std::process::exit(1);
    }
}

/// As textures::resolve_texture_path: the map's own directory, then the shared one, with
/// .bmp references stored as .rsb and .tga ones as TGA<name>.rsb, in any case
fn level_texture(map_num: &str, material: &str) -> Option<PathBuf> {
    let rel = Path::new(material);
    let stem = rel.file_stem()?.to_string_lossy().into_owned();
    let mut wanted = vec![format!("{stem}.rsb").to_lowercase()];
    if material.to_ascii_lowercase().ends_with(".tga") {
        wanted.push(format!("tga{stem}.rsb").to_lowercase());
    }
    let parent = rel.parent().unwrap_or(Path::new(""));
    for dir in [Path::new("data").join("map").join(map_num).join(parent), Path::new("data").join("texture").join(parent)] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let names: Vec<(String, PathBuf)> = entries.flatten().map(|e| (e.file_name().to_string_lossy().to_lowercase(), e.path())).collect();
        for want in &wanted {
            if let Some((_, p)) = names.iter().find(|(n, _)| n == want) {
                return Some(p.clone());
            }
        }
    }
    None
}

/// As textures::load_character_texture: beside the model, .tga materials as themselves and
/// the rest as .rsb
fn character_texture(dir: &Path, material: &str) -> Option<PathBuf> {
    let stem = Path::new(material).file_stem()?.to_string_lossy().into_owned();
    find_file(dir, &stem, if material.to_ascii_lowercase().ends_with(".tga") { "tga" } else { "rsb" })
}

/// A sound under data/sound. The viewer opens the name as given, so on a filesystem that
/// minds case the file must match it exactly; a file that only matches in another case is
/// listed but reported.
fn sound(name: &str) -> Option<PathBuf> {
    let want = Path::new("data/sound").join(name);
    if want.exists() {
        let dir = want.parent()?;
        let file = want.file_name()?.to_string_lossy().into_owned();
        let actual = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n.eq_ignore_ascii_case(&file))?;
        if actual != file {
            eprintln!("note: sound {name} is {actual} on disk, which a case-sensitive filesystem will not find");
        }
        return Some(dir.join(actual));
    }
    None
}

/// A model under data/model and the texture of its first material under data/texture
fn model_and_texture(m: &mut Manifest, file: &str) {
    let path = Path::new("data/model").join(file);
    let Some(path) = m.add(&format!("model {file}"), Some(path).filter(|p| p.exists())) else { return };
    match map::read_model(&path) {
        Ok(model) => {
            let stem = model.materials.materials.first()
                .and_then(|mat| Path::new(&mat.filename).file_stem().map(|s| s.to_string_lossy().into_owned()));
            if let Some(stem) = stem {
                m.add(&format!("texture {stem} of {file}"), find_file(Path::new("data/texture"), &stem, "rsb"));
            }
        }
        Err(e) => m.missing.push(format!("{}: {e:#}", path.display())),
    }
}

/// As levels::find_mission_spawns: the mission files whose first quoted string is the map's
/// file name
fn missions_naming(map_path: &Path) -> Vec<PathBuf> {
    let Some(name) = map_path.file_name().map(|n| n.to_string_lossy().into_owned()) else { return Vec::new() };
    let Ok(dir) = std::fs::read_dir("data/mission") else { return Vec::new() };
    dir.flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mis")))
        .filter(|p| {
            let text = String::from_utf8_lossy(&std::fs::read(p).unwrap_or_default()).into_owned();
            text.lines().find_map(|l| {
                let rest = l.trim().strip_prefix('"')?;
                Some(rest[..rest.find('"')?].to_string())
            }).is_some_and(|n| n.eq_ignore_ascii_case(&name))
        })
        .collect()
}
