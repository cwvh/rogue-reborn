// Draw the realistic bullet holes to a PNG contact sheet over a wall-colored
// background: hole-png out.png [first_seed]. One row per surface, four holes each.

use std::path::Path;

use rogue_reborn::impact::{hole_texture, Surface, TEXTURE_SIZE};
use rogue_reborn::png;

const SCALE: usize = 4;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(!args.is_empty(), "usage: hole-png out.png [first_seed]");
    let first: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);

    let rows = [
        (Surface::Concrete, [150u8, 145, 135]),
        (Surface::Wood, [140, 100, 60]),
        (Surface::Metal, [90, 100, 110]),
        (Surface::Earth, [100, 80, 55]),
    ];
    let tile = TEXTURE_SIZE * SCALE;
    let (w, h) = (tile * 4, tile * rows.len());
    let mut sheet = vec![0u8; w * h * 3];
    for (row, (surface, back)) in rows.iter().enumerate() {
        for col in 0..4 {
            let px = hole_texture(*surface, first + col as u64);
            for y in 0..tile {
                for x in 0..tile {
                    let o = ((y / SCALE) * TEXTURE_SIZE + x / SCALE) * 4;
                    let a = px[o + 3] as u32;
                    let dst = ((row * tile + y) * w + col * tile + x) * 3;
                    for c in 0..3 {
                        sheet[dst + c] = ((px[o + c] as u32 * a + back[c] as u32 * (255 - a)) / 255) as u8;
                    }
                }
            }
        }
    }
    png::write_png(Path::new(&args[0]), w as u32, h as u32, &sheet)?;
    println!("wrote {}", args[0]);
    Ok(())
}
