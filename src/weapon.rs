//! Weapon definitions (`data/kit/*.wpn`).
//!
//! Text files, one value per line, with no field names. Only the fields
//! worked out so far are read: the magazine size and reserve ammunition
//! (the first two integer-only lines), the model file (the first quoted
//! `.qob`) and the muzzle offset in model space (the first line of three
//! numbers after it).

use std::path::Path;

use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub struct Weapon {
    pub id: String,
    pub magazine: u32,
    pub reserve: u32,
    /// Model file name, e.g. `rsw_mp5k.qob`
    pub model: String,
    /// Where bullets leave the weapon, in model space
    pub muzzle: [f32; 3],
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

    let muzzle = lines[model_at + 1..].iter().find_map(|l| {
        let v: Vec<f32> = l.split_whitespace().filter_map(|n| n.parse().ok()).collect();
        (v.len() == 3 && l.split_whitespace().count() == 3).then(|| [v[0], v[1], v[2]])
    }).context("muzzle not found")?;

    Ok(Weapon { id, magazine, reserve, model, muzzle })
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
}
