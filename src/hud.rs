//! The heads-up display: status text, crosshair, and the bitmap font.

use crate::opengl::*;

// Heads-up display: crosshair at the centre, status lines bottom-left and a
// message under the crosshair. Leaves the 2D projection set; the debug text
// that follows re-enables depth testing.
pub fn draw_hud(win_w: i32, win_h: i32, status: &[String], message: &str, hit_marker: bool, veil: f32) {
    unsafe {
        glMatrixMode(0x1701); // GL_PROJECTION
        glLoadIdentity();
        ortho_2d(0.0, win_w.max(1) as f32, 0.0, win_h.max(1) as f32);
        glMatrixMode(0x1700); // GL_MODELVIEW
        glLoadIdentity();
        glDisable(0x0B71); // GL_DEPTH_TEST
        glDisable(0x0DE1); // GL_TEXTURE_2D

        // The white-out from a blast close by
        if veil > 0.01 {
            glEnable(0x0BE2); // GL_BLEND
            glBlendFunc(0x0302, 0x0303);
            glColor4f(1.0, 0.97, 0.92, veil.min(1.0) * 0.9);
            glBegin(0x0007); // GL_QUADS
            for (x, y) in [(0.0, 0.0), (win_w as f32, 0.0), (win_w as f32, win_h as f32), (0.0, win_h as f32)] {
                glVertex3f(x, y, 0.0);
            }
            glEnd();
            glDisable(0x0BE2);
        }

        let (cx, cy) = (win_w as f32 / 2.0, win_h as f32 / 2.0);
        if hit_marker { glColor3f(1.0, 0.2, 0.2); } else { glColor3f(1.0, 1.0, 1.0); }
        glBegin(0x0001); // GL_LINES
        for (dx0, dy0, dx1, dy1) in [(-12.0, 0.0, -4.0, 0.0), (4.0, 0.0, 12.0, 0.0), (0.0, -12.0, 0.0, -4.0), (0.0, 4.0, 0.0, 12.0)] {
            glVertex3f(cx + dx0, cy + dy0, 0.0);
            glVertex3f(cx + dx1, cy + dy1, 0.0);
        }
        glEnd();

        glColor3f(1.0, 1.0, 1.0);
        let scale = 14.0f32;
        let adv = scale * 0.8;
        for (i, line) in status.iter().enumerate() {
            let mut x = 12.0;
            for ch in line.chars() {
                draw_glyph(ch, x, 12.0 + i as f32 * scale * 1.5, scale);
                x += adv;
            }
        }
        if !message.is_empty() {
            glColor3f(1.0, 0.85, 0.3);
            let mut x = cx - adv * message.chars().count() as f32 / 2.0;
            for ch in message.chars() {
                draw_glyph(ch, x, cy - 50.0, scale);
                x += adv;
            }
        }
        glColor3f(1.0, 1.0, 1.0);
    }
}

pub fn draw_debug_text(win_w: i32, win_h: i32, lines: &[&str]) {
    unsafe {
        // Ortho for pixel coordinates with origin at bottom-left
        glMatrixMode(0x1701); // GL_PROJECTION
        glLoadIdentity();
        ortho_2d(0.0, win_w.max(1) as f32, 0.0, win_h.max(1) as f32);
        glMatrixMode(0x1700); // GL_MODELVIEW
        glLoadIdentity();

        // Render settings for HUD
        glDisable(0x0B71); // GL_DEPTH_TEST
        glDisable(0x0DE1); // GL_TEXTURE_2D
        glColor3f(1.0, 1.0, 1.0);

        let margin = 10.0f32;
        let scale = 14.0f32;
        let line_h = scale * 1.5;
        let adv = scale * 0.8;

        let w = win_w.max(1) as f32;
        let h = win_h.max(1) as f32;

        for (i, text) in lines.iter().enumerate() {
            let y = h - margin - (i as f32) * line_h;
            let len = text.chars().count() as f32;
            let tw = adv * len;
            let mut x = w - margin - tw;
            for ch in text.chars() {
                draw_glyph(ch, x, y, scale);
                x += adv;
            }
        }

        // Leave depth state restored for next frame's 3D
        glEnable(0x0B71); // GL_DEPTH_TEST
    }
}

pub fn draw_glyph(ch: char, x: f32, y: f32, scale: f32) {
    // Return a 5x7 bitmap for supported glyphs (top row first, bit 4 = leftmost)
    fn glyph_bitmap(c: char) -> Option<[u8; 7]> {
        match c {
            // Digits 0-9
            '0' => Some([0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E]),
            '1' => Some([0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E]),
            '2' => Some([0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F]),
            '3' => Some([0x0E, 0x11, 0x01, 0x06, 0x01, 0x11, 0x0E]),
            '4' => Some([0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02]),
            '5' => Some([0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E]),
            '6' => Some([0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E]),
            '7' => Some([0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08]),
            '8' => Some([0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E]),
            '9' => Some([0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C]),
            // Letters we commonly render
            'L' => Some([0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F]),
            'E' => Some([0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F]),
            'V' => Some([0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04]),
            'F' => Some([0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10]),
            'P' => Some([0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10]),
            'S' => Some([0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E]),
            'R' => Some([0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11]),
            'N' => Some([0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11]),
            'D' => Some([0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E]),
            'M' => Some([0x11, 0x1B, 0x15, 0x11, 0x11, 0x11, 0x11]),
            'A' => Some([0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]),
            'B' => Some([0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E]),
            'C' => Some([0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E]),
            'G' => Some([0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F]),
            'H' => Some([0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]),
            'I' => Some([0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E]),
            'J' => Some([0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C]),
            'K' => Some([0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11]),
            'O' => Some([0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E]),
            'Q' => Some([0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D]),
            'T' => Some([0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
            'U' => Some([0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E]),
            'W' => Some([0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11]),
            'X' => Some([0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11]),
            'Y' => Some([0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04]),
            'Z' => Some([0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F]),
            '/' => Some([0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10]),
            '-' => Some([0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00]),
            ':' => Some([0x00, 0x04, 0x00, 0x00, 0x04, 0x00, 0x00]),
            '.' => Some([0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00]),
            ' ' => Some([0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
            _ => None,
        }
    }

    let cu = ch.to_ascii_uppercase();
    if let Some(rows) = glyph_bitmap(cu) {
        let px = scale / 7.0; // pixel size based on 7 rows
        let py = px;
        unsafe {
            glBegin(0x0007); // GL_QUADS
            for (row_idx, row_bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if ((row_bits >> (4 - col)) & 1) != 0 {
                        let vx = x + (col as f32) * px;
                        let vy = y + ((6 - row_idx) as f32) * py;
                        glVertex3f(vx, vy, 0.0);
                        glVertex3f(vx + px, vy, 0.0);
                        glVertex3f(vx + px, vy + py, 0.0);
                        glVertex3f(vx, vy + py, 0.0);
                    }
                }
            }
            glEnd();
        }
    } else {
        // Fallback: outline box to visualize unsupported chars
        unsafe {
            glBegin(0x0001); // GL_LINES
            glVertex3f(x, y, 0.0);                 glVertex3f(x + scale, y, 0.0);
            glVertex3f(x + scale, y, 0.0);         glVertex3f(x + scale, y + scale, 0.0);
            glVertex3f(x + scale, y + scale, 0.0); glVertex3f(x, y + scale, 0.0);
            glVertex3f(x, y + scale, 0.0);         glVertex3f(x, y, 0.0);
            glEnd();
        }
    }
}

// Texture utilities
