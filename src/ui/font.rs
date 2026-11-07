//! The original's bitmap fonts (`data/font`, described by `fonts.fdf`).
//!
//! Each font is one RSB, a column of cells from character 32 to 255. The plain fonts draw their
//! ink in magenta on black, and the antialiased ones in grey on black, so a pixel's brightness
//! is how much it covers. The text is proportional, as the original draws it: each glyph is as
//! wide as its lit columns, and the next starts the fdf's kerning after it. Cell 32 is blank in
//! use, but holds a short line as wide as a space.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::rsb;

/// A pixel this bright or more is ink when a glyph's width is measured. The plain fonts have
/// specks of near black (8 0 0) that are not.
const INK: u8 = 64;

/// Coverage below this is drawn as none
const FAINT: u8 = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Glyph {
    /// The first lit column, and how many columns from there to the last
    left: usize,
    width: usize,
}

/// A font, or the dot font (`hud::draw_glyph`) that stands in when the game's are not there
#[derive(Clone, Debug)]
pub struct Font {
    pub cell: [usize; 2],
    pub kerning: usize,
    glyphs: Vec<Glyph>,
    /// How much each pixel of the column of cells covers, 0 to 255, top row first
    pub coverage: Vec<u8>,
}

/// One font's line in `fonts.fdf`
#[derive(Clone, Debug, PartialEq)]
pub struct FontDef {
    pub name: String,
    pub file: String,
    pub cell: [usize; 2],
    pub first: u32,
    pub last: u32,
    pub kerning: usize,
}

/// The fonts `fonts.fdf` describes
pub fn read_fdf(text: &str) -> Vec<FontDef> {
    text.lines().filter_map(|line| {
        let line = line.split("//").next()?.trim();
        let mut quoted = line.split('"');
        let (_, name, _, file, rest) = (quoted.next()?, quoted.next()?, quoted.next()?, quoted.next()?, quoted.next()?);
        let n: Vec<usize> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        let &[w, h, first, last, kerning] = n.as_slice() else { return None };
        Some(FontDef { name: name.to_string(), file: file.to_string(), cell: [w, h], first: first as u32, last: last as u32, kerning })
    }).collect()
}

impl Font {
    /// The font named `name` in `dir`'s `fonts.fdf`
    pub fn read(dir: &Path, name: &str) -> Result<Font> {
        let fdf = std::fs::read(dir.join("fonts.fdf")).with_context(|| format!("no fonts.fdf in {}", dir.display()))?;
        let defs = read_fdf(&String::from_utf8_lossy(&fdf));
        let Some(def) = defs.iter().find(|d| d.name.eq_ignore_ascii_case(name)) else { bail!("fonts.fdf has no font {name}") };
        if def.first != 32 {
            bail!("font {name} starts at character {}, not 32", def.first);
        }
        // The fdf names a BMP, and the files are RSBs of the same name in any case
        let stem = Path::new(&def.file).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let file = std::fs::read_dir(dir)?.flatten().map(|e| e.path())
            .find(|p| p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == stem) && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("rsb")))
            .with_context(|| format!("no {stem}.rsb in {}", dir.display()))?;
        let tex = rsb::read(&file)?;
        if tex.width as usize != def.cell[0] {
            bail!("{} is {} wide, not the {} its cells are", file.display(), tex.width, def.cell[0]);
        }
        Ok(Font::from_pixels(&tex.to_rgba8(), def.cell, def.kerning))
    }

    /// A font from a column of cells as RGBA pixels, character 32 first
    pub fn from_pixels(rgba: &[u8], cell: [usize; 2], kerning: usize) -> Font {
        let [w, h] = cell;
        let mut coverage: Vec<u8> = rgba.chunks(4).map(|p| p[0].max(p[1]).max(p[2])).map(|c| if c < FAINT { 0 } else { c }).collect();
        let cells = coverage.len() / (w * h).max(1);
        let glyphs = (0..cells).map(|g| {
            let lit: Vec<usize> = (0..w).filter(|&x| (0..h).any(|y| coverage[(g * h + y) * w + x] >= INK)).collect();
            match (lit.first(), lit.last()) {
                (Some(&a), Some(&b)) => Glyph { left: a, width: b - a + 1 },
                _ => Glyph::default(),
            }
        }).collect();
        // The space's line marks its width and is not drawn
        coverage.iter_mut().take(w * h).for_each(|c| *c = 0);
        Font { cell, kerning, glyphs, coverage }
    }

    /// The dot font, this tall
    pub fn dots(height: usize) -> Font {
        Font { cell: [height * 4 / 5, height], kerning: 0, glyphs: Vec::new(), coverage: Vec::new() }
    }

    pub fn is_dots(&self) -> bool {
        self.glyphs.is_empty()
    }

    pub fn height(&self) -> f32 {
        self.cell[1] as f32
    }

    /// The cell a character is drawn from: its own, or the question mark's if it has none
    fn glyph_of(&self, ch: char) -> usize {
        let code = ch as u32;
        let i = if (32..32 + self.glyphs.len() as u32).contains(&code) { (code - 32) as usize } else { usize::MAX };
        match self.glyphs.get(i) {
            Some(g) if g.width > 0 => i,
            _ if ch == ' ' => 0,
            _ => ('?' as u32 - 32) as usize,
        }
    }

    /// Where each character goes: its cell, the first column of the cell to draw, how many
    /// columns, and how far along the line it starts
    pub fn layout(&self, text: &str) -> Vec<(usize, usize, usize, f32)> {
        let mut x = 0.0;
        text.chars().map(|ch| {
            let (cell, left, width, advance) = if self.is_dots() {
                (0, 0, 0, self.cell[0] as f32)
            } else {
                let g = self.glyph_of(ch);
                let glyph = self.glyphs[g];
                (g, glyph.left, glyph.width, (glyph.width + self.kerning) as f32)
            };
            let at = x;
            x += advance;
            (cell, left, width, at)
        }).collect()
    }

    /// How wide a line of text is
    pub fn width(&self, text: &str) -> f32 {
        if self.is_dots() {
            return text.chars().count() as f32 * self.cell[0] as f32;
        }
        let w: f32 = text.chars().map(|ch| (self.glyphs[self.glyph_of(ch)].width + self.kerning) as f32).sum();
        // No space after the last
        (w - self.kerning as f32).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font of 4 x 3 cells from 32 to 127, each glyph a bar of ink as wide as its code's last
    /// two bits plus one, a column in from the left; the space's line three wide
    fn bars() -> Font {
        let (w, h) = (4, 3);
        let mut rgba = vec![0u8; w * h * 96 * 4];
        for g in 0..96usize {
            let width = if g == 0 { 3 } else { (g + 32) % 4 + 1 };
            for x in 0..width.min(w - 1) {
                let o = ((g * h + 1) * w + 1 + x) * 4;
                rgba[o..o + 4].copy_from_slice(&[255, 0, 132, 255]);
            }
            // A speck of near black, which is not ink
            rgba[(g * h * w) * 4] = 8;
        }
        // Character 127 is blank
        let o = (95 * h + 1) * w * 4;
        rgba[o..o + w * 4].fill(0);
        Font::from_pixels(&rgba, [w, h], 1)
    }

    #[test]
    fn the_fdf_is_read_as_the_original_lays_it_out() {
        let text = "// Font Definition File\n\n33\t// number of fonts\n\"OCRB14\"\t\"ocrb14.bmp\"\t\t16\t16\t32\t255\t1\t\"NORMAL\"\n\"OCRB14_A\" \"ocrb14_aa.bmp\" 16 16 32 255 1 \"ANTIALIASED\"\n";
        let defs = read_fdf(text);
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[1], FontDef { name: "OCRB14_A".to_string(), file: "ocrb14_aa.bmp".to_string(), cell: [16, 16], first: 32, last: 255, kerning: 1 });
    }

    #[test]
    fn text_is_as_wide_as_its_glyphs_and_the_kerning_between_them() {
        let font = bars();
        // A is 65: 2 wide; B 3 wide (capped at 3 by the cell); space 3
        assert_eq!(font.width("A"), 2.0);
        assert_eq!(font.width("AB"), 2.0 + 1.0 + 3.0);
        assert_eq!(font.width("A B"), 2.0 + 1.0 + 3.0 + 1.0 + 3.0);
        let laid: Vec<f32> = font.layout("A B").iter().map(|l| l.3).collect();
        assert_eq!(laid, [0.0, 3.0, 7.0]);
        // The specks are not ink, and neither are they drawn
        assert_eq!(font.layout("A")[0].1, 1);
        assert_eq!(font.coverage[(65 - 32) * 12], 0);
        // The space's line is not drawn
        assert!(font.coverage[..12].iter().all(|&c| c == 0));
    }

    #[test]
    fn a_character_the_font_lacks_is_a_question_mark() {
        let font = bars();
        for ch in ['\u{7f}', 'é', '\u{2603}'] {
            assert_eq!(font.layout(&ch.to_string())[0].0, '?' as usize - 32, "{ch:?}");
        }
    }

    #[test]
    fn the_dot_font_stands_in_without_the_games_fonts() {
        let font = Font::read(Path::new("no/such/dir"), "OCRB12_A").unwrap_or(Font::dots(10));
        assert!(font.is_dots());
        assert_eq!(font.width("ABC"), 24.0);
    }

    #[test]
    fn the_games_fonts_are_proportional() {
        let Ok(font) = Font::read(Path::new("data/font"), "OCRB14_A") else { return };
        assert_eq!(font.cell, [16, 16]);
        assert!(font.width("i") < font.width("m"));
        assert!(font.width("Team Survival") < 13.0 * 16.0);
        assert!(font.width(" ") > 0.0);
        let plain = Font::read(Path::new("data/font"), "OCRB14").unwrap();
        assert!(plain.width("Team Survival") > 0.0);
    }
}
