//! The window and its OpenGL state.

use glfw::Context;
use crate::args::Args;
use crate::opengl::*;

const WIDTH: i32 = 1280;
const HEIGHT: i32 = 1024;

pub fn open(args: &Args) -> (glfw::Glfw, glfw::PWindow, glfw::GlfwReceiver<(f64, glfw::WindowEvent)>) {
    // Initialize GLFW. The portal drawing masks the screen with the stencil buffer, so ask
    // for one rather than hope the default has it.
    let mut glfw = glfw::init_no_callbacks().unwrap();
    glfw.window_hint(glfw::WindowHint::StencilBits(Some(8)));
    glfw.window_hint(glfw::WindowHint::DepthBits(Some(24)));
    // A screenshot run draws into a window that never shows, so it doesn't cover the desktop
    glfw.window_hint(glfw::WindowHint::Visible(args.screenshot.is_none() && !args.dump));
    let (mut window, events) = glfw
        .create_window(WIDTH as _, HEIGHT as _, "Wireframe Viewer", glfw::WindowMode::Windowed)
        .expect("Failed to create GLFW window");
    window.make_current();
    {
        let mut stencil_bits = 0;
        unsafe { glGetIntegerv(GL_STENCIL_BITS, &mut stencil_bits) };
        if stencil_bits == 0 {
            eprintln!("warning: no stencil buffer, so rooms seen through doorways will not be masked (use --no-cull)");
        }
    }
    // vsync gives steady pacing; --max-fps replaces it with an explicit cap
    glfw.set_swap_interval(if args.max_fps.is_some() { glfw::SwapInterval::None } else { glfw::SwapInterval::Sync(1) });
    window.set_key_polling(true);
    window.set_cursor_pos_polling(true);
    window.set_framebuffer_size_polling(true);
    window.set_scroll_polling(true);
    window.set_mouse_button_polling(true);
    window.set_char_polling(true);
    gl::load_with(|s| window.get_proc_address(s) as *const _);

    // OpenGL setup
    unsafe {
        glViewport(0, 0, WIDTH, HEIGHT);
        glClearColor(0.1, 0.1, 0.1, 1.0);
        glEnable(GL_DEPTH_TEST);
        glPolygonMode(GL_FRONT_AND_BACK, GL_LINE);
    }
    (glfw, window, events)
}
