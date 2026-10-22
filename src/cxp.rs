//! The texture properties in data/texture/Rommel.CXP: which textures have a color keyed out,
//! are blended, let bullets or grenades through, scroll or animate. A text file of entries
//!
//!   Material cha_rail_01.bmp   colorkey 184 160 104   gunpass   grenadepass   End
//!
//! with `//` comments. RogueSpear.exe reads it at 0x6723b0; a texture listed twice keeps its
//! first entry, as the original's lookup (0x672bb0) finds that one first.

use std::collections::HashMap;
use std::path::Path;

/// Where the game keeps the file, from the directory holding data/
pub const PATH: &str = "data/texture/Rommel.CXP";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Properties {
    /// The color drawn as clear, in 8-bit RGB (`colorkey`)
    pub color_key: Option<[u8; 3]>,
    /// Blended by its alpha (`alphablend`)
    pub alpha_blend: bool,
    /// Bullets go through it (`gunpass`)
    pub gun_pass: bool,
    /// Grenades go through it (`grenadepass`)
    pub grenade_pass: bool,
    /// `softwarealpha`, on glass
    pub software_alpha: bool,
    /// `nosubsample`
    pub no_subsample: bool,
    /// `mipmap`: a number, then the names of the smaller levels
    pub mipmap: Option<(i32, Vec<String>)>,
    /// `scrolling`: a number and two rates, presumably a mode and how fast u and v move
    pub scrolling: Option<(i32, f32, f32)>,
    /// `animated`
    pub animated: Option<Animated>,
}

/// A texture that cycles through others
#[derive(Clone, Debug, PartialEq)]
pub struct Animated {
    /// 0 cycles, 1 oscillates
    pub kind: i32,
    /// Seconds each texture shows
    pub interval: f32,
    /// The textures that follow this one
    pub frames: Vec<String>,
}

/// Every entry, looked up by texture file name in any case
#[derive(Clone, Debug, Default)]
pub struct TextureProperties {
    entries: HashMap<String, Properties>,
}

impl TextureProperties {
    /// The game's file, or no properties at all when it cannot be read
    pub fn load() -> Self {
        Self::read(Path::new(PATH)).unwrap_or_default()
    }

    pub fn read(path: &Path) -> std::io::Result<Self> {
        // The file is Latin-1 like the rest of the data; the names that matter are ASCII
        Ok(Self::parse(&String::from_utf8_lossy(&std::fs::read(path)?)))
    }

    pub fn parse(text: &str) -> Self {
        let mut tokens = text.lines()
            .map(|l| l.find("//").map_or(l, |i| &l[..i]))
            .flat_map(str::split_whitespace);
        let mut entries = HashMap::new();
        while let Some(kind) = tokens.next() {
            // Material or Surface, then the texture's name
            if !kind.eq_ignore_ascii_case("material") && !kind.eq_ignore_ascii_case("surface") {
                continue;
            }
            let Some(name) = tokens.next() else { break };
            let mut p = Properties::default();
            let int = |t: Option<&str>| t.and_then(|t| t.parse::<i32>().ok()).unwrap_or(0);
            let float = |t: Option<&str>| t.and_then(|t| t.parse::<f32>().ok()).unwrap_or(0.0);
            while let Some(word) = tokens.next() {
                match word.to_ascii_lowercase().as_str() {
                    "end" => break,
                    "colorkey" => {
                        let [r, g, b] = [int(tokens.next()), int(tokens.next()), int(tokens.next())];
                        p.color_key = Some([r as u8, g as u8, b as u8]);
                    }
                    "alphablend" => p.alpha_blend = true,
                    "gunpass" => p.gun_pass = true,
                    "grenadepass" => p.grenade_pass = true,
                    "softwarealpha" => p.software_alpha = true,
                    "nosubsample" => p.no_subsample = true,
                    "mipmap" => {
                        let first = int(tokens.next());
                        let n = int(tokens.next()).max(0);
                        p.mipmap = Some((first, (0..n).filter_map(|_| tokens.next().map(String::from)).collect()));
                    }
                    "scrolling" => p.scrolling = Some((int(tokens.next()), float(tokens.next()), float(tokens.next()))),
                    "animated" => {
                        let kind = int(tokens.next());
                        let interval = float(tokens.next());
                        let n = int(tokens.next()).max(0);
                        let frames = (0..n).filter_map(|_| tokens.next().map(String::from)).collect();
                        p.animated = Some(Animated { kind, interval, frames });
                    }
                    _ => {}
                }
            }
            entries.entry(name.to_ascii_lowercase()).or_insert(p);
        }
        Self { entries }
    }

    /// The properties of a texture, by its file name ("cha_rail_01.bmp"); a directory before it
    /// is ignored
    pub fn get(&self, texture: &str) -> Option<&Properties> {
        let name = texture.rsplit(['/', '\\']).next().unwrap_or(texture);
        self.entries.get(&name.to_ascii_lowercase())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
//M06 Chalet I
Material cha_rail_01.bmp\t\t\t\tcolorkey 184 160 104\tgunpass\tgrenadepass\tEnd
Material glass_reinforced.tga\t\talphablend\tsoftwarealpha mipmap\t4\t0\t\tEnd
Material cha_rail_01.bmp\t\t\t\tcolorkey 184 160 104\tgunpass\t\t\tEnd
Material sp_drum_flicker_1.tga
\tanimated
\t0\t\t// type (0 = cycle, 1 = oscillate)
\t0.4\t\t// interval
\t1\t\t// # of additional textures
\tsp_drum_flicker_2.tga
End
Surface  raindrop.tga \t\t\t\t\talphablend\t\t\t End
";

    #[test]
    fn reads_each_entry_and_keeps_the_first_of_a_name() {
        let t = TextureProperties::parse(SAMPLE);
        assert_eq!(t.len(), 4);
        let rail = t.get("CHA_RAIL_01.BMP").expect("found in any case");
        assert_eq!(rail.color_key, Some([184, 160, 104]));
        assert!(rail.gun_pass && rail.grenade_pass, "the first entry wins: {rail:?}");
        let glass = t.get("glass_reinforced.tga").unwrap();
        assert!(glass.alpha_blend && glass.software_alpha && !glass.gun_pass);
        assert_eq!(glass.mipmap, Some((4, Vec::new())));
        let drum = t.get("textures/sp_drum_flicker_1.tga").unwrap();
        assert_eq!(drum.animated, Some(Animated { kind: 0, interval: 0.4, frames: vec!["sp_drum_flicker_2.tga".into()] }));
        assert!(t.get("raindrop.tga").unwrap().alpha_blend);
        assert!(t.get("wall_house_wood_01.bmp").is_none());
    }

    #[test]
    fn the_games_file_keys_the_railing_and_the_chain_fence() {
        let Ok(t) = TextureProperties::read(Path::new(PATH)) else { return };
        assert!(t.len() > 400, "{} entries", t.len());
        let rail = t.get("cha_rail_01.bmp").unwrap();
        assert_eq!(rail.color_key, Some([184, 160, 104]));
        assert!(rail.gun_pass && rail.grenade_pass);
        let fence = t.get("rm13_chainfence.bmp").unwrap();
        assert_eq!(fence.color_key, Some([216, 216, 216]));
        assert!(fence.gun_pass && !fence.grenade_pass);
    }
}
