//! Drawing what the UI lays out (`rogue_reborn::ui`) with GL: the canvas scaled into the
//! window, bars either side, with the original's art and fonts as textures.

use std::collections::HashMap;
use std::path::Path;

use rogue_reborn::rsb;
use rogue_reborn::ui::font::Font;
use rogue_reborn::ui::{Art, Canvas, Color, Draw, FontId, Rect, Ui};

use crate::hud::draw_glyph;
use crate::opengl::*;

const TEXTURE_2D: u32 = 0x0DE1;

/// The UI's textures, uploaded once the window is open
pub struct UiGl {
    art: HashMap<Art, (u32, [u32; 2])>,
    text: Option<u32>,
    big: Option<u32>,
}

/// A font's cells as a texture: white, as opaque as each pixel covers
fn upload_font(font: &Font) -> Option<u32> {
    if font.is_dots() {
        return None;
    }
    let rgba: Vec<u8> = font.coverage.iter().flat_map(|&c| [255, 255, 255, c]).collect();
    let rows = font.coverage.len() / font.cell[0];
    upload_rgba_to_gl(font.cell[0] as u32, rows as u32, &rgba)
}

impl UiGl {
    pub fn load(ui: &Ui, bitmaps: &Path) -> UiGl {
        let mut art = HashMap::new();
        for a in Art::ALL {
            let (file, _) = a.file();
            match rsb::read(&bitmaps.join(file)) {
                Ok(tex) => {
                    if let Some(id) = upload_rgba_to_gl(tex.width, tex.height, &tex.to_rgba8_keyed(Art::KEY)) {
                        art.insert(a, (id, [tex.width, tex.height]));
                    }
                }
                Err(e) => eprintln!("{e:#}: drawing a plain box for it"),
            }
        }
        UiGl { art, text: upload_font(&ui.text), big: upload_font(&ui.big) }
    }

    /// Draw a frame of the UI over the whole framebuffer
    pub fn draw(&self, ui: &Ui, draws: &[Draw], framebuffer: [i32; 2]) {
        let canvas = Canvas::fit([framebuffer[0] as f32, framebuffer[1] as f32]);
        unsafe {
            glViewport(0, 0, framebuffer[0], framebuffer[1]);
            glClearColor(0.0, 0.0, 0.0, 1.0);
            glClear(0x4000 | 0x0100); // GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT
            glMatrixMode(0x1701); // GL_PROJECTION
            glLoadIdentity();
            // Down the screen as the canvas goes
            ortho_2d(0.0, framebuffer[0].max(1) as f32, framebuffer[1].max(1) as f32, 0.0);
            glMatrixMode(0x1700); // GL_MODELVIEW
            glLoadIdentity();
            glTranslatef(canvas.offset[0], canvas.offset[1], 0.0);
            glScalef(canvas.scale, canvas.scale, 1.0);
            glDisable(0x0B71); // GL_DEPTH_TEST
            glDisable(0x0B44); // GL_CULL_FACE
            // The level may be drawn in lines; the 3D view sets its own each frame
            glPolygonMode(0x0408, 0x1B02); // GL_FRONT_AND_BACK, GL_FILL
            glEnable(0x0BE2); // GL_BLEND
            glBlendFunc(0x0302, 0x0303); // GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA
            for d in draws {
                match d {
                    Draw::Fill { rect, color } => fill(*rect, *color),
                    Draw::Frame { rect, color } => {
                        let Rect { x, y, w, h } = *rect;
                        for edge in [Rect::new(x, y, w, 1.0), Rect::new(x, y + h - 1.0, w, 1.0), Rect::new(x, y, 1.0, h), Rect::new(x + w - 1.0, y, 1.0, h)] {
                            fill(edge, *color);
                        }
                    }
                    Draw::Art { art, state, rect, fallback } => match self.art.get(art) {
                        Some(&(id, [_, height])) => {
                            let states = art.file().1 as f32;
                            // Half a pixel in from each edge, so that the states either side do
                            // not bleed in
                            let half = 0.5 / height as f32;
                            let (v0, v1) = (*state as f32 / states + half, (*state + 1) as f32 / states - half);
                            textured(id, *rect, [0.0, v0, 1.0, v1], [1.0; 4]);
                        }
                        None => fill(*rect, *fallback),
                    },
                    // The texture repeats, so its coordinates run on past 1
                    Draw::Tile { art, rect, fallback } => match self.art.get(art) {
                        Some(&(id, [w, h])) => textured(id, *rect, [0.0, 0.0, rect.w / w as f32, rect.h / h as f32], [1.0; 4]),
                        None => fill(*rect, *fallback),
                    },
                    Draw::Text { text, font, at, color } => {
                        let (f, tex) = match font {
                            FontId::Text => (&ui.text, self.text),
                            FontId::Big => (&ui.big, self.big),
                        };
                        self.text(f, tex, text, *at, *color);
                    }
                }
            }
            glDisable(0x0BE2);
            glDisable(TEXTURE_2D);
            glEnable(0x0B71);
            glColor4f(1.0, 1.0, 1.0, 1.0);
        }
    }

    fn text(&self, font: &Font, tex: Option<u32>, text: &str, at: [f32; 2], color: Color) {
        let h = font.height();
        let Some(id) = tex else {
            // The dot font draws up the screen, so it is turned over
            unsafe {
                glDisable(TEXTURE_2D);
                glColor4f(color[0], color[1], color[2], color[3]);
                for (ch, (_, _, _, x)) in text.chars().zip(font.layout(text)) {
                    glPushMatrix();
                    glTranslatef(at[0] + x, at[1] + h, 0.0);
                    glScalef(1.0, -1.0, 1.0);
                    draw_glyph(ch, 0.0, 0.0, h);
                    glPopMatrix();
                }
            }
            return;
        };
        let [cw, ch] = [font.cell[0] as f32, h];
        let total = (font.coverage.len() / font.cell[0]) as f32;
        for (cell, left, width, x) in font.layout(text) {
            if width == 0 {
                continue;
            }
            let (u0, u1) = (left as f32 / cw, (left + width) as f32 / cw);
            // Half a pixel in, so that the glyphs above and below do not bleed in
            let (v0, v1) = ((cell as f32 * ch + 0.5) / total, ((cell as f32 + 1.0) * ch - 0.5) / total);
            textured(id, Rect::new(at[0] + x, at[1], width as f32, ch), [u0, v0, u1, v1], color);
        }
    }
}

fn fill(rect: Rect, color: Color) {
    unsafe {
        glDisable(TEXTURE_2D);
        glColor4f(color[0], color[1], color[2], color[3]);
        glBegin(0x0007); // GL_QUADS
        glVertex3f(rect.x, rect.y, 0.0);
        glVertex3f(rect.x + rect.w, rect.y, 0.0);
        glVertex3f(rect.x + rect.w, rect.y + rect.h, 0.0);
        glVertex3f(rect.x, rect.y + rect.h, 0.0);
        glEnd();
    }
}

/// A texture's part `uv` (left, top, right, bottom) over the rectangle, tinted
fn textured(id: u32, rect: Rect, uv: [f32; 4], color: Color) {
    let [u0, v0, u1, v1] = uv;
    unsafe {
        glEnable(TEXTURE_2D);
        glBindTexture(TEXTURE_2D, id);
        glColor4f(color[0], color[1], color[2], color[3]);
        glBegin(0x0007); // GL_QUADS
        glTexCoord2f(u0, v0);
        glVertex3f(rect.x, rect.y, 0.0);
        glTexCoord2f(u1, v0);
        glVertex3f(rect.x + rect.w, rect.y, 0.0);
        glTexCoord2f(u1, v1);
        glVertex3f(rect.x + rect.w, rect.y + rect.h, 0.0);
        glTexCoord2f(u0, v1);
        glVertex3f(rect.x, rect.y + rect.h, 0.0);
        glEnd();
    }
}
