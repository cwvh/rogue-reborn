// What a bitmap font of the original is made of: font-probe data/font/ocrb14.rsb CELL [out.png]
// Prints the pixel format, the colors found and how often, and each glyph's lit columns, and can
// write the glyphs as a 16-wide grid (scaled 4 times) to look at.

use std::collections::BTreeMap;
use std::path::Path;

use rogue_reborn::{png, rsb};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() >= 2, "usage: font-probe FONT.rsb CELL_HEIGHT [out.png]");
    let tex = rsb::read(Path::new(&args[0]))?;
    let cell: usize = args[1].parse()?;
    let (w, h) = (tex.width as usize, tex.height as usize);
    let rgba = tex.to_rgba8();
    println!("{w}x{h}, version {}, bits r{} g{} b{} a{}, {} cells", tex.version, tex.bitmask.r, tex.bitmask.g, tex.bitmask.b, tex.bitmask.a, h / cell);
    let mut colors: BTreeMap<[u8; 4], usize> = BTreeMap::new();
    for px in rgba.chunks(4) {
        *colors.entry([px[0], px[1], px[2], px[3]]).or_default() += 1;
    }
    let mut by_count: Vec<_> = colors.into_iter().collect();
    by_count.sort_by_key(|c| std::cmp::Reverse(c.1));
    println!("{} colors; the most common:", by_count.len());
    for (c, n) in by_count.iter().take(8) {
        println!("  {c:?} x{n}");
    }
    let background = by_count[0].0;
    let lit = |x: usize, y: usize| rgba[(y * w + x) * 4..][..3].iter().any(|&c| c >= 64);
    for g in 0..h / cell {
        let cols: Vec<usize> = (0..w).filter(|&x| (0..cell).any(|y| lit(x, g * cell + y))).collect();
        let ch = char::from_u32(32 + g as u32).unwrap_or('?');
        if g < 96 || !cols.is_empty() {
            match (cols.first(), cols.last()) {
                (Some(a), Some(b)) => println!("{:3} {ch:?}: columns {a}..={b}", 32 + g),
                _ => println!("{:3} {ch:?}: blank", 32 + g),
            }
        }
    }
    if let Some(out) = args.get(2) {
        let (n, scale) = (h / cell, 4);
        let rows = n.div_ceil(16);
        let (ow, oh) = (16 * w * scale, rows * cell * scale);
        let mut rgb = vec![40u8; ow * oh * 3];
        for g in 0..n {
            let (gx, gy) = (g % 16 * w * scale, g / 16 * cell * scale);
            for y in 0..cell * scale {
                for x in 0..w * scale {
                    let o = ((g * cell + y / scale) * w + x / scale) * 4;
                    let v = if rgba[o..o + 4] == background { 40 } else { rgba[o].max(rgba[o + 1]).max(rgba[o + 2]) };
                    let at = ((gy + y) * ow + gx + x) * 3;
                    rgb[at..at + 3].fill(v);
                }
            }
        }
        png::write_png(Path::new(out), ow as u32, oh as u32, &rgb)?;
        println!("wrote {out}");
    }
    Ok(())
}
