use std::ffi::{CString, c_char};

#[repr(C)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

pub const KEY_ENTER: i32 = 257;

pub const RAYWHITE: Color = Color { r: 245, g: 245, b: 245, a: 255 };
pub const LIGHTGRAY: Color = Color { r: 200, g: 200, b: 200, a: 255 };
pub const GRAY: Color = Color { r: 130, g: 130, b: 130, a: 255 };
pub const GREEN: Color = Color { r: 0, g: 228, b: 48, a: 255 };
pub const DARKGREEN: Color = Color { r: 0, g: 117, b: 44, a: 255 };
pub const PURPLE: Color = Color { r: 200, g: 122, b: 255, a: 255 };
pub const MAROON: Color = Color { r: 190, g: 33, b: 55, a: 255 };
pub const BLUE: Color = Color { r: 0, g: 121, b: 241, a: 255 };
pub const DARKBLUE: Color = Color { r: 0, g: 82, b: 172, a: 255 };

mod ffi {
    use super::*;

    #[link(name = "raylib")]
    unsafe extern "C" {
        // Window-related functions
        pub fn InitWindow(width: i32, height: i32, title: *const c_char);
        pub fn CloseWindow();
        pub fn WindowShouldClose() -> bool;

        // Drawing-related functions
        pub fn ClearBackground(color: Color);

        // Timing-related functions
        pub fn SetTargetFPS(fps: i32);

        // Input-related functions: keyboard
        pub fn IsKeyPressed(key: i32) -> bool;
        pub fn IsKeyDown(key: i32) -> bool;

        // Drawing-related functions
        pub fn BeginDrawing();
        pub fn EndDrawing();

        pub fn DrawText(text: *const c_char, posX: i32, posY: i32, fontSize: i32, color: Color);
        pub fn DrawRectangle(posX: i32, posY: i32, width: i32, height: i32, color: Color);
    }
}

/// Initialize window and OpenGL context
pub fn InitWindow(width: i32, height: i32, title: &str) {
    let title = CString::new(title).unwrap();
    unsafe { ffi::InitWindow(width, height, title.as_ptr()) }
}

/// Close window and unload OpenGL context
pub fn CloseWindow() {
    unsafe { ffi::CloseWindow() }
}

/// Check if application should close (KEY_ESCAPE pressed or windows close icon
/// clicked)
pub fn WindowShouldClose() -> bool {
    unsafe { ffi::WindowShouldClose() }
}

/// Set target FPS (maximum)
pub fn SetTargetFPS(fps: i32) {
    unsafe { ffi::SetTargetFPS(fps) }
}

/// Set background color (framebuffer clear color)
pub fn ClearBackground(color: Color) {
    unsafe { ffi::ClearBackground(color) }
}

/// Setup cavnas (framebuffer) to start drawing
pub fn BeginDrawing() {
    unsafe { ffi::BeginDrawing() }
}

/// End canvas drawing and swap buffers (double buffering)
pub fn EndDrawing() {
    unsafe { ffi::EndDrawing() }
}

/// Draw text (using default font)
pub fn DrawText(text: &str, posX: i32, posY: i32, fontSize: i32, color: Color) {
    let s = CString::new(text).unwrap();
    unsafe { ffi::DrawText(s.as_ptr(), posX, posY, fontSize, color) }
}

/// Draw a color-filled rectangle
pub fn DrawRectangle(posX: i32, posY: i32, width: i32, height: i32, color: Color) {
    unsafe { ffi::DrawRectangle(posX, posY, width, height, color) }
}

/// Check if a key has been pressed once
pub fn IsKeyPressed(key: i32) -> bool {
    unsafe { ffi::IsKeyPressed(key) }
}

/// Check if a key is being pressed
pub fn IsKeyDown(key: i32) -> bool {
    unsafe { ffi::IsKeyDown(key) }
}
