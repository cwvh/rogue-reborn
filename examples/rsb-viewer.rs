use std::path::Path;

use glfw::{Action, Context, Key, WindowEvent};
use rogue_reborn::rsb;

#[link(name = "GL")]
extern "C" {
    fn glViewport(x: i32, y: i32, width: i32, height: i32);
    fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    fn glClear(mask: u32);
    fn glEnable(cap: u32);
    fn glDisable(cap: u32);
    fn glMatrixMode(mode: u32);
    fn glLoadIdentity();
    fn glBegin(mode: u32);
    fn glEnd();
    fn glTexCoord2f(s: f32, t: f32);
    fn glVertex3f(x: f32, y: f32, z: f32);
    fn glTexParameteri(target: u32, pname: u32, param: i32);
    fn glGenTextures(n: i32, textures: *mut u32);
    fn glBindTexture(target: u32, texture: u32);
    fn glTexImage2D(
        target: u32,
        level: i32,
        internalformat: i32,
        width: i32,
        height: i32,
        border: i32,
        format: u32,
        type_: u32,
        pixels: *const std::ffi::c_void
    );
}

fn main() -> anyhow::Result<()> {
    // Load two sample textures
    let rsb_main = rsb::read(Path::new("data/texture/faces/Chavez_hrt_face.RSB"))?;
    let rsb_blink = rsb::read(Path::new("data/texture/faces/Chavez_hrt_face_blink.rsb"))?;
    println!("{rsb_main}");
    println!("{rsb_blink}");

    // Init window via GLFW
    let mut glfw = glfw::init_no_callbacks().unwrap();
    let (mut window, events) = glfw
        .create_window(800, 600, "RSB Viewer (GLFW)", glfw::WindowMode::Windowed)
        .expect("Failed to create GLFW window");
    window.make_current();
    window.set_key_polling(true);
    window.set_framebuffer_size_polling(true);
    gl::load_with(|s| window.get_proc_address(s) as *const _);

    // Upload textures to GL
    let tex_main = upload_rsb_to_gl(&rsb_main).expect("failed to upload main texture");
    let tex_blink = upload_rsb_to_gl(&rsb_blink).expect("failed to upload blink texture");
    let mut use_blink = false;

    // GL state
    unsafe {
        glViewport(0, 0, 800, 600);
        glClearColor(0.05, 0.05, 0.05, 1.0);
        glDisable(0x0B71); // GL_DEPTH_TEST
        glEnable(0x0DE1);  // GL_TEXTURE_2D
    }

    while !window.should_close() {
        glfw.poll_events();
        for (_, event) in glfw::flush_messages(&events) {
            match event {
                WindowEvent::Key(key, _, action, _) => match (key, action) {
                    (Key::Escape, Action::Press) | (Key::Q, Action::Press) => window.set_should_close(true),
                    (Key::F, Action::Press) => { use_blink = !use_blink; }
                    _ => {}
                },
                WindowEvent::FramebufferSize(w, h) => unsafe { glViewport(0, 0, w, h); },
                _ => {}
            }
        }

        let (w, h) = window.get_size();
        let win_aspect = (w.max(1) as f32) / (h.max(1) as f32);
        let (tex_w, tex_h) = (rsb_main.width as f32, rsb_main.height as f32);
        let tex_aspect = tex_w / tex_h;

        // Compute quad size in NDC preserving aspect ratio
        let (mut quad_w, mut quad_h) = (1.0f32, 1.0f32);
        if win_aspect > tex_aspect {
            quad_w = tex_aspect / win_aspect;
        } else {
            quad_h = win_aspect / tex_aspect;
        }

        unsafe {
            glClear(0x00004000); // GL_COLOR_BUFFER_BIT
            glMatrixMode(0x1701); // GL_PROJECTION
            glLoadIdentity();
            glMatrixMode(0x1700); // GL_MODELVIEW
            glLoadIdentity();

            // Bind selected texture
            glBindTexture(0x0DE1, if use_blink { tex_blink } else { tex_main });

            // Draw a full-screen quad (triangle fan) with V flipped to match RSB top-left origin
            glBegin(0x0006); // GL_TRIANGLE_FAN
            glTexCoord2f(0.0, 1.0); glVertex3f(-quad_w, -quad_h, 0.0);
            glTexCoord2f(1.0, 1.0); glVertex3f( quad_w, -quad_h, 0.0);
            glTexCoord2f(1.0, 0.0); glVertex3f( quad_w,  quad_h, 0.0);
            glTexCoord2f(0.0, 0.0); glVertex3f(-quad_w,  quad_h, 0.0);
            glEnd();
        }

        window.swap_buffers();
    }

    Ok(())
}

fn upload_rsb_to_gl(tex: &rsb::Rsb) -> Option<u32> {
    let rgba = rsb_to_rgba8(tex);
    if rgba.is_empty() {
        return None;
    }

    let mut id: u32 = 0;
    unsafe {
        glGenTextures(1, &mut id as *mut u32);
        glBindTexture(0x0DE1, id); // GL_TEXTURE_2D
        glTexParameteri(0x0DE1, 0x2801, 0x2601); // GL_TEXTURE_MIN_FILTER, GL_LINEAR
        glTexParameteri(0x0DE1, 0x2800, 0x2601); // GL_TEXTURE_MAG_FILTER, GL_LINEAR

        glTexImage2D(
            0x0DE1,              // GL_TEXTURE_2D
            0,                   // level
            0x1908,              // GL_RGBA
            tex.width as i32,
            tex.height as i32,
            0,                   // border
            0x1908,              // GL_RGBA
            0x1401,              // GL_UNSIGNED_BYTE
            rgba.as_ptr() as *const std::ffi::c_void
        );
    }
    Some(id)
}

fn rsb_to_rgba8(tex: &rsb::Rsb) -> Vec<u8> {
    let size = tex.size();
    let mut out = Vec::with_capacity(size * 4);

    if tex.palette_colors.is_some() {
        if let Some(colors) = &tex.palette_colors {
            for px in &tex.pixels {
                let idx = match px {
                    rsb::Pixel::PaletteColorIndex(i) => *i as usize,
                    _ => 0,
                };
                let c = colors.get(idx).cloned().unwrap_or(rsb::PaletteColor { r: 0, g: 0, b: 0, a: 255 });
                out.push(c.r);
                out.push(c.g);
                out.push(c.b);
                out.push(c.a);
            }
        }
        return out;
    }

    let bm = &tex.bitmask;
    let rbits = bm.r.max(1);
    let gbits = bm.g.max(1);
    let bbits = bm.b.max(1);
    let abits = bm.a;

    for px in &tex.pixels {
        let r = px.r(bm).unwrap_or(0);
        let g = px.g(bm).unwrap_or(0);
        let b = px.b(bm).unwrap_or(0);
        let a = px.a(bm).unwrap_or_else(|| if abits == 0 { 255 } else { 0 });

        out.push(scale_to_u8(r, rbits));
        out.push(scale_to_u8(g, gbits));
        out.push(scale_to_u8(b, bbits));
        out.push(if abits == 0 { a as u8 } else { scale_to_u8(a, abits) });
    }

    out
}

fn scale_to_u8(v: u32, bits: u32) -> u8 {
    if bits == 0 {
        return 255;
    }
    let max = (1u32 << bits) - 1;
    (((v * 255) + (max / 2)) / max) as u8
}
