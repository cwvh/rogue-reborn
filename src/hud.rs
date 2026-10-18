//! The heads-up display: status text, crosshair, and the bitmap font.

use rogue_reborn::concussion;
use rogue_reborn::net::meter::{FIX, Meter, WINDOW};
use rogue_reborn::sim::TICK;

use crate::opengl::*;

// Heads-up display: crosshair at the center, status lines bottom-left, the latest kills top-left
// (each with its color, under the lag meter if it is shown, with the time now) and a message
// under the crosshair. A dead player's view is tinted red. Leaves the 2D projection set; the
// debug text that follows re-enables depth testing.
#[allow(clippy::too_many_arguments)]
pub fn draw_hud(win_w: i32, win_h: i32, status: &[String], message: &str, hit_marker: bool, veil: f32, daze: f32, feed: &[(String, [f32; 3])], dead: bool, meter: Option<(&Meter, f64)>) {
    unsafe {
        glMatrixMode(0x1701); // GL_PROJECTION
        glLoadIdentity();
        ortho_2d(0.0, win_w.max(1) as f32, 0.0, win_h.max(1) as f32);
        glMatrixMode(0x1700); // GL_MODELVIEW
        glLoadIdentity();
        glDisable(0x0B71); // GL_DEPTH_TEST
        glDisable(0x0DE1); // GL_TEXTURE_2D

        // The eyes squinting shut against a blast, to a dark slit
        if daze > 0.01 {
            glEnable(0x0BE2); // GL_BLEND
            glBlendFunc(0x0302, 0x0303);
            // Rings of dark from the edge of the clear middle out past the corners, coming on
            // quickly at first so the tunnel has a wall and not just a dim edge
            let (cx, cy) = (win_w as f32 / 2.0, win_h as f32 / 2.0);
            let corner = (cx * cx + cy * cy).sqrt();
            // The edge of the eye, scaled so that it reaches past every corner of the screen
            let corner_angle = cy.atan2(cx);
            let outer = corner * 1.05 / concussion::eye(corner_angle);
            let inner = outer * concussion::clear_radius(daze);
            let edge = concussion::edge_darkness(daze);
            const SEGMENTS: usize = 64;
            const RINGS: usize = 8;
            let ring = |k: usize| {
                let t = k as f32 / RINGS as f32;
                (inner + (outer - inner) * t, edge * (t / 0.5).min(1.0).powf(0.6))
            };
            for k in 0..RINGS {
                glBegin(0x0005); // GL_TRIANGLE_STRIP
                for i in 0..=SEGMENTS {
                    let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                    let (s, c) = a.sin_cos();
                    let reach = concussion::eye(a);
                    for (radius, alpha) in [ring(k), ring(k + 1)] {
                        glColor4f(0.0, 0.0, 0.0, alpha);
                        glVertex3f(cx + c * radius * reach, cy + s * radius * reach, 0.0);
                    }
                }
                glEnd();
            }
            glDisable(0x0BE2);
        }

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

        if dead {
            glEnable(0x0BE2); // GL_BLEND
            glBlendFunc(0x0302, 0x0303);
            glColor4f(0.45, 0.0, 0.0, 0.4);
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
        let feed_top = match meter {
            Some((meter, now)) => draw_lag_meter(win_h as f32 - 24.0, meter, now) - scale - 8.0,
            None => win_h as f32 - 24.0,
        };
        for (i, (line, color)) in feed.iter().enumerate() {
            glColor3f(color[0], color[1], color[2]);
            let mut x = 12.0;
            for ch in line.chars() {
                draw_glyph(ch, x, feed_top - i as f32 * scale * 1.5, scale);
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

/// The lag meter, its first line's foot at `top`: two lines of figures, then a graph of the last
/// few seconds, newest on the right. The upper band has a column a frame, for how much the view of
/// the other players has in hand: blue up while there is a later snapshot to move toward, yellow
/// down when there is none and they stall. The lower band has a column a snapshot: green up to its
/// ping (the top is 200 ms, or the next 100 above the worst, and is written beside it), red the
/// whole height for one that never came, and yellow down from the top for a fix of the player's
/// position (20 units the whole height). Returns where it ends.
fn draw_lag_meter(top: f32, meter: &Meter, now: f64) -> f32 {
    let f = meter.figures();
    let ms = |s: f32| (s * 1000.0).round();
    let scale = 14.0f32;
    let line = scale * 1.5;
    let ping_color = if f.ping < 0.06 { [0.4, 1.0, 0.4] } else if f.ping < 0.12 { [1.0, 0.85, 0.3] } else { [1.0, 0.3, 0.3] };
    let figures = [
        (format!("PING {} MS  JITTER {} MS  LOSS {:.0}%", ms(f.ping), ms(f.jitter), f.loss * 100.0), ping_color),
        (format!("VIEW {} MS  FIXES {} MAX {:.0}  IN {:.1} KB/S  OUT {:.1} KB/S", ms(f.view), f.fixes, f.worst_fix, f.bytes_in / 1000.0, f.bytes_out / 1000.0), [1.0, 1.0, 1.0]),
    ];
    for (i, (text, color)) in figures.iter().enumerate() {
        unsafe { glColor3f(color[0], color[1], color[2]) };
        draw_text(text, 12.0, top - i as f32 * line, scale);
    }

    let (left, width) = (12.0f32, 280.0f32);
    let (shown_h, ping_h) = (30.0f32, 50.0f32);
    let graph_top = top - line - 8.0;
    let middle = graph_top - shown_h / 2.0;
    let bottom = graph_top - shown_h - ping_h;
    let x = |at: f64| left + width * (1.0 - ((now - at) / WINDOW) as f32).clamp(0.0, 1.0);
    let quad = |x0: f32, y0: f32, x1: f32, y1: f32| unsafe {
        glVertex3f(x0, y0, 0.0);
        glVertex3f(x1, y0, 0.0);
        glVertex3f(x1, y1, 0.0);
        glVertex3f(x0, y1, 0.0);
    };
    unsafe {
        glEnable(0x0BE2); // GL_BLEND
        glBlendFunc(0x0302, 0x0303);
        glColor4f(0.0, 0.0, 0.0, 0.5);
        glBegin(0x0007); // GL_QUADS
        quad(left, bottom, left + width, graph_top);
        glEnd();
        glDisable(0x0BE2);

        // The view: blue up to 100 ms in hand, yellow down when it has run dry
        glBegin(0x0001); // GL_LINES
        for s in &meter.shown {
            let x = x(s.at);
            if s.ahead < TICK / 2.0 {
                glColor3f(1.0, 0.85, 0.2);
                glVertex3f(x, middle, 0.0);
                glVertex3f(x, middle - shown_h / 2.0, 0.0);
            } else {
                glColor3f(0.3, 0.5, 1.0);
                glVertex3f(x, middle, 0.0);
                glVertex3f(x, middle + (s.ahead / 0.1).min(1.0) * shown_h / 2.0, 0.0);
            }
        }
        // The line between the bands, and halfway up the lower one
        glColor3f(0.5, 0.5, 0.5);
        glVertex3f(left, bottom + ping_h, 0.0);
        glVertex3f(left + width, bottom + ping_h, 0.0);
        glColor3f(0.3, 0.3, 0.3);
        glVertex3f(left, bottom + ping_h / 2.0, 0.0);
        glVertex3f(left + width, bottom + ping_h / 2.0, 0.0);
        glEnd();

        let worst = meter.columns.iter().filter_map(|c| c.ping).fold(0.0f32, f32::max);
        let full = (worst * 10.0).ceil().max(2.0) / 10.0;
        glColor3f(0.6, 0.6, 0.6);
        draw_text(&format!("{} MS", (full * 1000.0).round()), left + width + 6.0, bottom + ping_h - 10.0, 10.0);
        draw_text(&format!("{} MS", (full * 500.0).round()), left + width + 6.0, bottom + ping_h / 2.0 - 5.0, 10.0);

        let column = (width * meter.interval().unwrap_or(2.0 * TICK) / WINDOW as f32).max(1.0);
        glBegin(0x0007); // GL_QUADS
        for c in &meter.columns {
            let x = x(c.at);
            if c.lost {
                glColor3f(1.0, 0.2, 0.2);
                quad(x - column, bottom, x, bottom + ping_h);
                continue;
            }
            if let Some(ping) = c.ping {
                glColor3f(0.3, 0.9, 0.3);
                quad(x - column, bottom, x, bottom + (ping / full).min(1.0) * ping_h);
            }
            if c.fix >= FIX {
                glColor3f(1.0, 0.85, 0.2);
                quad(x - column, bottom + ping_h * (1.0 - (c.fix / 20.0).min(1.0)), x, bottom + ping_h);
            }
        }
        glEnd();
        glColor3f(1.0, 1.0, 1.0);
    }
    bottom
}

fn draw_text(text: &str, x: f32, y: f32, scale: f32) {
    for (i, ch) in text.chars().enumerate() {
        draw_glyph(ch, x + i as f32 * scale * 0.8, y, scale);
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
            '%' => Some([0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03]),
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
