// Dump RSB textures to a PNG contact sheet: rsb-png out.png a.rsb [b.rsb ...]
// Textures are scaled (nearest) to 256 pixels tall and placed side by side.

use std::path::Path;

use rogue_reborn::{png, rsb};

const HEIGHT: usize = 256;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() >= 2, "usage: rsb-png out.png a.rsb [b.rsb ...]");

    let mut tiles: Vec<(usize, Vec<u8>)> = Vec::new();
    for path in &args[1..] {
        let tex = rsb::read(Path::new(path))?;
        let (w, h) = (tex.width as usize, tex.height as usize);
        println!("{path}: {w}x{h}");
        let rgba = tex.to_rgba8();
        anyhow::ensure!(rgba.len() == w * h * 4, "{path}: could not decode pixels");

        let tw = (w * HEIGHT / h).max(1);
        let mut rgb = Vec::with_capacity(tw * HEIGHT * 3);
        for y in 0..HEIGHT {
            for x in 0..tw {
                let o = ((y * h / HEIGHT) * w + x * w / tw) * 4;
                // Over a checkerboard, so transparent pixels show
                let back = if (x / 16 + y / 16) % 2 == 0 { 90u32 } else { 150 };
                let a = rgba[o + 3] as u32;
                for c in 0..3 {
                    rgb.push(((rgba[o + c] as u32 * a + back * (255 - a)) / 255) as u8);
                }
            }
        }
        tiles.push((tw, rgb));
    }

    let gap = 4;
    let total: usize = tiles.iter().map(|t| t.0).sum::<usize>() + gap * (tiles.len() - 1);
    let mut sheet = vec![40u8; total * HEIGHT * 3];
    let mut x0 = 0;
    for (tw, rgb) in &tiles {
        for y in 0..HEIGHT {
            let dst = (y * total + x0) * 3;
            sheet[dst..dst + tw * 3].copy_from_slice(&rgb[y * tw * 3..(y + 1) * tw * 3]);
        }
        x0 += tw + gap;
    }
    png::write_png(Path::new(&args[0]), total as u32, HEIGHT as u32, &sheet)?;
    println!("wrote {}", args[0]);
    Ok(())
}
