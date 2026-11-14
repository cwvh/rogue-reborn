//! A level's `.dmp` light file: the level's ambient light and the lights of each room.

use std::io::{Cursor, Read};
use std::path::Path;

use anyhow::{Context, Result, ensure};
use byteorder::{LE, ReadBytesExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Directional,
    Point,
    Spot,
}

#[derive(Clone, Debug)]
pub struct Light {
    /// The name of the room the light is in, as the rooms' objects are named
    pub room: String,
    pub name: String,
    pub kind: Kind,
    /// The way the light shines, for directional lights and spots
    pub direction: [f32; 3],
    pub position: [f32; 3],
    /// The angle across a spot's cone
    pub cone_degrees: f32,
    pub range: f32,
    pub diffuse: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Dmp {
    /// The level's ambient light, after the original's fix-up of it
    pub ambient: [f32; 3],
    pub lights: Vec<Light>,
}

impl Dmp {
    /// What a level with no file has: a grey ambient and no lights
    pub fn none() -> Dmp {
        Dmp { ambient: [0.5; 3], lights: Vec::new() }
    }
}

/// The ambient light as the original uses it: never darker than 0.15 in its brightest channel
fn fix_ambient(raw: [f32; 3]) -> [f32; 3] {
    if raw.iter().any(|&c| c >= 0.15) {
        return raw;
    }
    let top = raw.iter().fold(f32::MIN, |a, &c| a.max(c));
    if raw.iter().all(|c| c.abs() <= 0.0001) {
        [0.15; 3]
    } else {
        raw.map(|c| c * 0.15 / top)
    }
}

fn string(buf: &mut Cursor<&[u8]>) -> Result<String> {
    let len = buf.read_u32::<LE>().context("string length")? as usize;
    ensure!(len <= buf.get_ref().len() - buf.position() as usize, "string runs past the end");
    let mut bytes = vec![0u8; len];
    buf.read_exact(&mut bytes)?;
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    Ok(bytes.iter().map(|&b| b as char).collect())
}

fn floats<const N: usize>(buf: &mut Cursor<&[u8]>) -> Result<[f32; N]> {
    let mut v = [0.0; N];
    buf.read_f32_into::<LE>(&mut v)?;
    Ok(v)
}

pub fn parse(bytes: &[u8]) -> Result<Dmp> {
    let mut buf = Cursor::new(bytes);
    let _version = buf.read_u32::<LE>().context("version")?;
    let [r, g, b, _alpha] = floats::<4>(&mut buf).context("ambient")?;
    let _next_id = buf.read_u32::<LE>().context("next id")?;
    let count = buf.read_u32::<LE>().context("light count")?;
    let mut lights = Vec::new();
    for i in 0..count {
        let light = light(&mut buf).with_context(|| format!("light {i}"))?;
        lights.extend(light);
    }
    ensure!(buf.position() as usize == bytes.len(), "{} bytes left over", bytes.len() - buf.position() as usize);
    Ok(Dmp { ambient: fix_ambient([r, g, b]), lights })
}

/// One record, or none for a kind of light we do not draw
fn light(buf: &mut Cursor<&[u8]>) -> Result<Option<Light>> {
    let room = string(buf).context("room")?;
    let name = string(buf).context("name")?;
    let kind = buf.read_u32::<LE>().context("kind")?;
    let direction = floats::<3>(buf)?;
    let position = floats::<3>(buf)?;
    let cone_degrees = buf.read_f32::<LE>()?;
    // Five floats the original keeps and the lighting never reads
    floats::<5>(buf)?;
    let range = buf.read_f32::<LE>()?;
    let _ambient = floats::<4>(buf)?;
    let [r, g, b, _alpha] = floats::<4>(buf)?;
    let _specular = floats::<4>(buf)?;
    let _orientation = floats::<4>(buf)?;
    buf.read_u8()?;
    let kind = match kind {
        1 => Kind::Directional,
        2 | 3 => Kind::Point,
        6 => Kind::Spot,
        _ => return Ok(None),
    };
    Ok(Some(Light { room, name, kind, direction, position, cone_degrees, range, diffuse: [r, g, b] }))
}

pub fn read(path: &Path) -> Result<Dmp> {
    let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    parse(&bytes).with_context(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(name: &str) -> Option<Dmp> {
        let path = Path::new("data/map").join(name).join(format!("{name}.dmp"));
        path.exists().then(|| read(&path).expect("the file parses to its exact length"))
    }

    #[test]
    fn rm08_has_its_lights() {
        let Some(d) = load("rm08") else { return };
        assert_eq!(d.lights.len(), 265);
        let first = &d.lights[0];
        assert_eq!((first.room.as_str(), first.name.as_str(), first.kind), ("100", "light100_4591", Kind::Point));
        assert!((first.range - 338.42).abs() < 0.01);
        assert!((first.diffuse[0] - 0.9294).abs() < 0.001);
        assert!((d.ambient[2] - 0.1765).abs() < 0.001);
    }

    #[test]
    fn rm01_has_its_lights() {
        let Some(d) = load("rm01") else { return };
        assert!(!d.lights.is_empty());
        assert!(d.lights.iter().all(|l| l.range > 0.0 || l.kind == Kind::Directional));
    }

    #[test]
    fn every_shipped_file_parses() {
        let Ok(dirs) = std::fs::read_dir("data/map") else { return };
        for dir in dirs.flatten() {
            for file in std::fs::read_dir(dir.path()).into_iter().flatten().flatten() {
                if file.path().extension().is_some_and(|e| e.eq_ignore_ascii_case("dmp")) {
                    read(&file.path()).unwrap();
                }
            }
        }
    }

    #[test]
    fn ambient_is_never_dark() {
        assert_eq!(fix_ambient([0.5, 0.02, 0.0]), [0.5, 0.02, 0.0]);
        assert_eq!(fix_ambient([0.0, 0.0, 0.0]), [0.15; 3]);
        assert_eq!(fix_ambient([7.7e-44, 0.0, 9.4e-38]), [0.15; 3]);
        let a = fix_ambient([0.06, 0.03, 0.0]);
        assert!((a[0] - 0.15).abs() < 1e-6 && (a[1] - 0.075).abs() < 1e-6);
    }
}
