use std::collections::HashMap;
use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::time::{Instant, Duration};

use rogue_reborn::rsb;

fn main() -> anyhow::Result<()> {
    // TODO: take CLI path to RSB file(s)

    let mut total = 0;
    let mut bytes = 0;
    let mut elapsed = Duration::ZERO;
    let mut counts = HashMap::new();
    let mut depths = HashMap::new();
    let mut layouts = HashMap::new();

    let opts = glob::MatchOptions {
        case_sensitive: false,
        ..Default::default()
    };

    for path in glob::glob_with("data/**/*.rsb", opts)? {
        let filename = path?;

        let mut file = File::open(&filename)?;
        bytes += file.seek(SeekFrom::End(0))?;

        let now = Instant::now();
        let rsb = rsb::read(&filename)?;
        elapsed += now.elapsed();
        total += 1;

        *counts.entry((rsb.version, rsb.palette)).or_insert(0) += 1;

        println!("{}", filename.display());
        println!("  version = {} ; palette = {:?}", rsb.version, rsb.palette);
        if let Some(colors) = &rsb.palette_colors {
            println!("  palette colors: {}", colors.len());
        }
        println!("  height, width: ({}, {})", rsb.height, rsb.width);
        println!("  bitmask: {:?}", rsb.bitmask);
        println!("  pixels: {}", rsb.pixels.len());
        if let Some(masked) = &rsb.masked_pixels {
            println!("  masked: {}", masked.len());
        }

        let bits = rsb.bitmask.bits();
        if bits == 32 {
            println!("  32-bit, is ARGB: {}", rsb.bitmask.is_argb());
        } else {
            println!("  bits: {}", bits);
        }
        *depths.entry(bits).or_insert(0) += 1;
        let key = (rsb.bitmask.r, rsb.bitmask.g, rsb.bitmask.b, rsb.bitmask.a);
        *layouts.entry(key).or_insert(0) += 1;
    }
    println!("{:-<50}", "");
    println!("Read {total} RSB files in {:?} ({bytes} bytes)\n", elapsed);
    for ((version, palette), count) in counts {
        let palette = if let Some(palette) = palette {
            palette.to_string()
        } else {
            "nil".to_string()
        };
        println!("version={version}, palette={palette:<5}: {count:>5} files");
    }
    for (depth, count) in depths {
        println!("bits={depth:<19}: {count:>5} files");
    }
    for ((r, g, b, a), count) in layouts {
        println!("RGBA {r}/{g}/{b}/{a} : {count:>5} files");
    }

    Ok(())
}
