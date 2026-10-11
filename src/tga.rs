//! Minimal TGA reader: uncompressed true-color (type 2), 24 or 32 bits per
//! pixel, which is all the game's character textures use.

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

pub struct Tga {
    pub width: u32,
    pub height: u32,
    /// RGBA8, top row first
    pub rgba: Vec<u8>,
}

pub fn read(path: &Path) -> Result<Tga> {
    let d = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    parse(&d)
}

pub fn parse(d: &[u8]) -> Result<Tga> {
    ensure!(d.len() >= 18, "TGA header truncated");
    let id_len = d[0] as usize;
    let (color_map, image_type) = (d[1], d[2]);
    if image_type != 2 || color_map != 0 {
        bail!("unsupported TGA (type {image_type}, color map {color_map})");
    }
    let width = u16::from_le_bytes([d[12], d[13]]) as usize;
    let height = u16::from_le_bytes([d[14], d[15]]) as usize;
    let bpp = d[16];
    let top_left = d[17] & 0x20 != 0;
    if bpp != 24 && bpp != 32 {
        bail!("unsupported TGA depth {bpp}");
    }
    let bytes = bpp as usize / 8;
    let start = 18 + id_len;
    let pixels = d.get(start..start + width * height * bytes).context("TGA pixel data truncated")?;

    let mut rgba = vec![0u8; width * height * 4];
    for y in 0..height {
        // Rows are stored bottom-up unless the origin bit says otherwise
        let src_row = if top_left { y } else { height - 1 - y };
        for x in 0..width {
            let s = (src_row * width + x) * bytes;
            let o = (y * width + x) * 4;
            rgba[o] = pixels[s + 2];
            rgba[o + 1] = pixels[s + 1];
            rgba[o + 2] = pixels[s];
            rgba[o + 3] = if bytes == 4 { pixels[s + 3] } else { 255 };
        }
    }
    Ok(Tga { width: width as u32, height: height as u32, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_tiny_bottom_up_image() {
        // 2x1, 32-bit, bottom-left origin: BGRA red then BGRA green-with-alpha
        let mut d = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 32, 8];
        d.extend_from_slice(&[0, 0, 255, 255, 0, 255, 0, 128]);
        let t = parse(&d).unwrap();
        assert_eq!((t.width, t.height), (2, 1));
        assert_eq!(t.rgba, vec![255, 0, 0, 255, 0, 255, 0, 128]);
    }

    #[test]
    fn rejects_compressed_images() {
        let mut d = vec![0u8; 18];
        d[2] = 10;
        assert!(parse(&d).is_err());
    }

    #[test]
    fn reads_the_game_glass_texture() {
        let p = Path::new("data/character/team_hrt/HRT_M_glass.tga");
        if !p.exists() {
            return;
        }
        let t = read(p).unwrap();
        assert_eq!((t.width, t.height), (32, 32));
        assert_eq!(t.rgba.len(), 32 * 32 * 4);
    }
}
