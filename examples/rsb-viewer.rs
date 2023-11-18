use std::path::Path;

use anyhow::Context;
use minifb::{Key, Window, WindowOptions};
use rogue_reborn::rsb;

fn main() -> anyhow::Result<()> {
    let rsb = rsb::read(Path::new("data/texture/faces/Chavez_hrt_face.RSB"))?;
    let blink = rsb::read(Path::new("data/texture/faces/Chavez_hrt_face_blink.rsb"))?;
    println!("{rsb}");
    println!("{blink}");

    let width = rsb.width as usize;
    let height = rsb.height as usize;

    let mut buffer = vec![0u32; width * height];
    let mut window = Window::new(
        "RSB Viewer",
        width, height,
        WindowOptions {
            resize: true,
            scale: minifb::Scale::X8,
            ..WindowOptions::default()
        }
    )
    .context("window creation failed")?;

    window.limit_update_rate(Some(std::time::Duration::from_millis(40)));

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let rsb = if window.is_key_down(Key::F) { &blink } else { &rsb };
        let bitmask = &rsb.bitmask;

        for (buffer, rsb) in buffer.iter_mut().zip(rsb.pixels.iter()) {
            let r = rsb.r(bitmask).unwrap() * bitmask.r;
            let b = rsb.b(bitmask).unwrap() * bitmask.b;

            // Rogue Spear assets only have two pixel layouts:
            // RGBA 5/6/5/0 and 4/4/4/4
            // TODO: replace with proper scaling calculation in rsb.rs
            let scale = if bitmask.g == 6 { bitmask.g >> 1 } else { bitmask.g };
            let g = rsb.g(bitmask).unwrap() * scale;

            // 0RGB
            let rgb = (r << 16) | (g << 8) | b;
            *buffer = rgb;
        }

        window
            .update_with_buffer(&buffer, width, height)
            .unwrap()
    }

    Ok(())
}
