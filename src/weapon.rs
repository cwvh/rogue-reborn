//! Weapon definitions (`data/kit/*.wpn`).
//!
//! Text files with one value per line and no field names.
//! Only the fields worked out so far are read.
//! The first two integer-only lines give the magazine size and reserve ammunition.
//! The first `.wav` names the sound of a shot.
//! The three lines before the model give the kind of weapon, how it reloads and how it is held.
//! The first quoted `.qob` names the model file.
//! The first line of three numbers after it gives the muzzle offset in model space.
//! The eight lines after the ammunition's name give its spread (see `sim::aim`).
//! The line after those gives the size of the muzzle flash.

use std::path::Path;

use anyhow::{Context, Result};

use crate::sim::aim::Gun;

#[derive(Clone, Debug)]
pub struct Weapon {
    pub id: String,
    pub magazine: u32,
    pub reserve: u32,
    /// Model file name, e.g. `rsw_mp5k.qob`
    pub model: String,
    /// Where bullets leave the weapon, in model space
    pub muzzle: [f32; 3],
    /// The sound of one shot
    pub sound: Option<String>,
    /// "long arm", "compact", "machine pistol", "sniper" or "pistol"
    pub class: String,
    /// How it reloads: "subgun", "shotgun", "pistol" or "bolt action"
    pub reload_class: String,
    /// How it is held: "pistol", "compact", "subgun", "long arm" or "sniper" (0x5dc7d0)
    pub hold: String,
    /// The ammunition it fires, as the gunfire combat model names it (see `gfcm`)
    pub ammo: Option<String>,
    /// How many pellets a shot fires, and how far apart: each goes off the aim by up to
    /// `pellet_spread` in 100 across and up (0x588c70)
    pub pellets: u32,
    pub pellet_spread: f32,
    pub gun: Option<Gun>,
    pub flash_scale: f32,
}

impl Weapon {
    /// Seconds between shots with the trigger held. The file does not say it plainly, so these
    /// are guesses by the kind of weapon: the automatics at 600 rounds a minute.
    pub fn interval(&self) -> f32 {
        match (self.class.as_str(), self.reload_class.as_str()) {
            (_, "shotgun") => 0.8,
            (_, "bolt action") => 1.5,
            ("sniper", _) => 0.6,
            ("pistol", _) => 0.3,
            _ => 0.1,
        }
    }
}

pub fn read(path: &Path) -> Result<Weapon> {
    let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    parse(&String::from_utf8_lossy(&bytes)).with_context(|| format!("{}", path.display()))
}

pub fn parse(text: &str) -> Result<Weapon> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let id = lines.first().copied().unwrap_or_default().to_string();

    let ints: Vec<u32> = lines.iter().filter_map(|l| l.parse::<u32>().ok()).take(2).collect();
    let [magazine, reserve] = ints[..] else { anyhow::bail!("magazine and reserve not found") };

    let model_at = lines.iter().position(|l| {
        l.trim_matches('"').to_ascii_lowercase().ends_with(".qob") && l.starts_with('"')
    }).context("model not found")?;
    let model = lines[model_at].trim_matches('"').to_string();
    let word = |i: Option<usize>| i.and_then(|i| lines.get(i)).map_or_else(String::new, |l| l.trim_matches('"').to_ascii_lowercase());
    let (class, reload_class, hold) = (word(model_at.checked_sub(3)), word(model_at.checked_sub(2)), word(model_at.checked_sub(1)));
    let sound = lines.iter().find(|l| l.to_ascii_lowercase().ends_with(".wav\"")).map(|l| l.trim_matches('"').to_string());

    let (muzzle_at, muzzle) = lines.iter().enumerate().skip(model_at + 1).find_map(|(i, l)| {
        let v: Vec<f32> = l.split_whitespace().filter_map(|n| n.parse().ok()).collect();
        (v.len() == 3 && l.split_whitespace().count() == 3).then(|| (i, [v[0], v[1], v[2]]))
    }).context("muzzle not found")?;

    // After the muzzle, the ammunition's name, then the spread's numbers. A shotgun first has
    // its pellets' spread and a list of its ammunitions, each with how many pellets it fires
    // (0x5dabf0), then the name again.
    let ammo_at = (model_at + 1..lines.len().saturating_sub(1))
        .find(|&i| lines[i].starts_with('"') && lines[i + 1].parse::<f32>().is_ok());
    let ammo = ammo_at.and_then(|a| lines[a].split('"').nth(1)).map(str::to_string);
    let numbers: Vec<f32> = ammo_at.map_or_else(Vec::new, |a| lines[a + 1..].iter().map_while(|l| l.parse().ok()).take(8).collect());
    let gun = (numbers.len() == 8).then(|| Gun {
        moving: [numbers[0], numbers[1], numbers[2], numbers[3], numbers[4]],
        crouched: numbers[5],
        recoil: numbers[6],
        close: numbers[7],
    });

    let flash_scale = ammo_at.and_then(|a| lines.get(a + 9)).and_then(|l| l.parse().ok()).unwrap_or(0.0);

    let shot = (reload_class == "shotgun").then(|| {
        let spread: f32 = lines.get(muzzle_at + 1)?.parse().ok()?;
        let n: usize = lines.get(muzzle_at + 2)?.parse().ok()?;
        // As many as the ammunition it is loaded with fires, else one (0x5dc500)
        let pellets = lines.get(muzzle_at + 3..muzzle_at + 3 + n)?.iter().find_map(|l| {
            let mut parts = l.split('"');
            let name = parts.nth(1)?;
            (Some(name) == ammo.as_deref()).then(|| parts.next()?.trim().parse().ok()).flatten()
        }).unwrap_or(1);
        Some((pellets, spread))
    }).flatten();
    let (pellets, pellet_spread) = shot.unwrap_or((1, 0.0));

    Ok(Weapon { id, magazine, reserve, model, muzzle, sound, class, reload_class, hold, ammo, gun, pellets, pellet_spread, flash_scale })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_mp5k() {
        let p = Path::new("data/kit/pw_mp5k.wpn");
        if !p.exists() {
            return;
        }
        let w = read(p).unwrap();
        assert_eq!(w.id, "ST_EQP_MP5PDW");
        assert_eq!((w.magazine, w.reserve), (30, 900));
        assert_eq!(w.model, "rsw_mp5k.qob");
        assert_eq!(w.muzzle, [0.0, -43.0, 13.0]);
        assert_eq!(w.sound.as_deref(), Some("C_sb5ks.wav"));
        assert_eq!((w.class.as_str(), w.reload_class.as_str(), w.hold.as_str(), w.ammo.as_deref()), ("compact", "subgun", "compact", Some("9mmP(FMJ)")));
        assert_eq!((w.pellets, w.pellet_spread), (1, 0.0));
        assert_eq!(w.flash_scale, 0.09);
        // What sim::aim has for it
        let (gun, mp5k) = (w.gun.unwrap(), crate::sim::aim::MP5K);
        assert_eq!((gun.moving, gun.crouched, gun.recoil, gun.close), (mp5k.moving, mp5k.crouched, mp5k.recoil, mp5k.close));
    }

    #[test]
    fn muzzle_sits_at_the_end_of_the_model() {
        // The muzzle offset should be at the barrel end of the model's bounds
        let (w, m) = (Path::new("data/kit/pw_mp5k.wpn"), Path::new("data/model/rsw_mp5k.qob"));
        if !w.exists() || !m.exists() {
            return;
        }
        let w = read(w).unwrap();
        let model = crate::map::read_model(m).unwrap();
        let min_y = model.geometries.objects.iter().flat_map(|o| &o.vertices).map(|v| v.y).fold(f32::MAX, f32::min);
        assert!((w.muzzle[1] - min_y).abs() < 1.0, "muzzle {} vs barrel end {min_y}", w.muzzle[1]);
    }

    #[test]
    fn reads_most_weapons() {
        let Ok(dir) = std::fs::read_dir("data/kit") else { return };
        let (mut ok, mut bad) = (0, Vec::new());
        for f in dir.flatten().map(|e| e.path()) {
            if f.extension().is_some_and(|e| e.eq_ignore_ascii_case("wpn")) {
                match read(&f) {
                    Ok(_) => ok += 1,
                    Err(e) => bad.push(format!("{e:#}")),
                }
            }
        }
        eprintln!("{ok} weapons parsed, {} failed: {:?}", bad.len(), &bad[..bad.len().min(3)]);
        assert!(ok > bad.len(), "most weapons should parse");
    }

    #[test]
    fn flash_scales_follow_the_weapon() {
        for (file, scale) in [("pw_car15", 0.4), ("pw_mp5sd", 0.01), ("pw_shotgun", 0.5), ("tw_dragunov", 0.8)] {
            let Ok(w) = read(&Path::new("data/kit").join(format!("{file}.wpn"))) else { return };
            assert_eq!(w.flash_scale, scale, "{file}");
        }
    }

    #[test]
    fn a_shotgun_fires_its_ammunitions_pellets() {
        let Ok(w) = read(Path::new("data/kit/pw_shotgun.wpn")) else { return };
        assert_eq!((w.ammo.as_deref(), w.pellets, w.pellet_spread), (Some("12ga. 00 Buckshot"), 4, 2.16));
        assert!(w.gun.is_some());
        let spas = read(Path::new("data/kit/pw_spas12.wpn")).unwrap();
        assert_eq!((spas.pellets, spas.pellet_spread), (4, 3.81));
    }
}
