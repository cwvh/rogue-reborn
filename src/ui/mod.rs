//! A small immediate-mode UI for the menu screens, drawn with the original's fonts and art.
//!
//! A screen is laid out each frame on a 640 x 480 canvas, the original's, by calling the
//! widgets in turn. They say whether they were clicked, and add what they look like to a list
//! of things to draw, which the game draws with GL (`ui_gl` in rr). Nothing here needs a window,
//! so screens can be tested without one.

pub mod font;

use std::path::Path;

use font::Font;

pub type Color = [f32; 4];

/// The canvas everything is laid out on
pub const CANVAS: [f32; 2] = [640.0, 480.0];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.x && p[0] < self.x + self.w && p[1] >= self.y && p[1] < self.y + self.h
    }

    pub fn inset(&self, by: f32) -> Rect {
        Rect::new(self.x + by, self.y + by, self.w - 2.0 * by, self.h - 2.0 * by)
    }

    /// The part `w` wide at the right
    pub fn right(&self, w: f32) -> Rect {
        Rect::new(self.x + self.w - w, self.y, w, self.h)
    }
}

/// Where the canvas goes in a framebuffer: as large as fits, in the middle, bars either side
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canvas {
    pub scale: f32,
    pub offset: [f32; 2],
}

impl Canvas {
    pub fn fit(framebuffer: [f32; 2]) -> Canvas {
        let scale = (framebuffer[0] / CANVAS[0]).min(framebuffer[1] / CANVAS[1]).max(1e-3);
        Canvas { scale, offset: [(framebuffer[0] - CANVAS[0] * scale) / 2.0, (framebuffer[1] - CANVAS[1] * scale) / 2.0] }
    }

    /// The point of the canvas under the pointer, which the window gives in its own units. A
    /// window on a high-DPI screen has more framebuffer pixels than units.
    pub fn pointer(&self, cursor: [f64; 2], window: [i32; 2], framebuffer: [i32; 2]) -> [f32; 2] {
        let px = |i: usize| cursor[i] as f32 * framebuffer[i] as f32 / window[i].max(1) as f32;
        [(px(0) - self.offset[0]) / self.scale, (px(1) - self.offset[1]) / self.scale]
    }
}

/// The original's pieces of the session screen, each a strip of its states one above another
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Art {
    /// A player's bar, by side, or with none: dark, lit, dark, lit. Gold's file is "d" for
    /// desert.
    NameBlue,
    NameGold,
    NameNone,
    /// The box a player's ping shows in: dark, lit, dark, lit
    PingBox,
    /// The player list's header row: the ping's, the ready light's and the names': up, lit,
    /// pressed and dark
    HeadPing,
    HeadReady,
    HeadName,
    /// The buttons that change side: up, lit, pressed, and dark with a ⊘
    SideBlue,
    SideGold,
    /// The background, tiled
    Grid,
    /// A button: up, lit, framed and dark
    Button,
    /// Unchecked, checked in gold, checked in blue, and dark
    CheckBox,
    /// The ready lights, and the host's screen: dim and lit, twice
    Ready,
    NotReady,
    Host,
    /// Arrows to step a choice on and back: up, lit, pressed and dark
    Up,
    Down,
    /// The settings' boxes, the player's own above the session's, with the color key between
    Panel,
    /// The chat's box, with its scroll bar and the line to type on
    ChatBox,
    /// The chat's scroll arrows and the knob between them: up, lit, pressed and dark
    ScrollUp,
    ScrollDown,
    ScrollKnob,
}

impl Art {
    pub const ALL: [Art; 22] = [
        Art::NameBlue, Art::NameGold, Art::NameNone, Art::PingBox, Art::HeadPing, Art::HeadReady, Art::HeadName, Art::SideBlue, Art::SideGold,
        Art::Grid, Art::Button, Art::CheckBox, Art::Ready, Art::NotReady, Art::Host, Art::Up, Art::Down, Art::Panel, Art::ChatBox, Art::ScrollUp,
        Art::ScrollDown, Art::ScrollKnob,
    ];

    /// The color the art's clear pixels have
    pub const KEY: [u8; 3] = [255, 0, 132];

    /// Its file in `data/bitmap`, and how many states it has
    pub fn file(self) -> (&'static str, usize) {
        match self {
            Art::NameBlue => ("ss_name_b.RSB", 4),
            Art::NameGold => ("ss_name_d.RSB", 4),
            Art::NameNone => ("ss_name_w.rsb", 4),
            Art::PingBox => ("ss_ping_team.rsb", 4),
            Art::HeadPing => ("ss_sortping.rsb", 4),
            Art::HeadReady => ("ss_sortready.rsb", 4),
            Art::HeadName => ("ss_sortname.rsb", 4),
            Art::SideBlue => ("mps_blueteam_btn.rsb", 4),
            Art::SideGold => ("mps_goldteam_btn.rsb", 4),
            Art::Grid => ("PDA_grid_blue.rsb", 1),
            Art::Button => ("mp_team.RSB", 4),
            Art::CheckBox => ("mp_sc_checkbox2.rsb", 4),
            Art::Ready => ("ss_ready_g.RSB", 4),
            Art::NotReady => ("ss_ready_r.RSB", 4),
            Art::Host => ("ss_ready_s.RSB", 4),
            Art::Up => ("mp_sc_uparrow.RSB", 4),
            Art::Down => ("mp_sc_dnarrow.RSB", 4),
            Art::Panel => ("mps_adv_bg.rsb", 1),
            Art::ChatBox => ("mps_adv_chatbox.rsb", 1),
            Art::ScrollUp => ("mps_up_arrow.RSB", 4),
            Art::ScrollDown => ("mps_dn_arrow.RSB", 4),
            Art::ScrollKnob => ("mps_scrollbar.RSB", 4),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontId {
    Text,
    Big,
}

/// Something to draw, on the canvas
#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    Fill { rect: Rect, color: Color },
    /// A line a pixel wide just inside the rectangle
    Frame { rect: Rect, color: Color },
    /// A state of a piece of art stretched over the rectangle, or `fallback` filling it if the
    /// art is not there
    Art { art: Art, state: usize, rect: Rect, fallback: Color },
    /// Art repeated across the rectangle at its own size
    Tile { art: Art, rect: Rect, fallback: Color },
    /// Text whose top left is at `at`, in the color given
    Text { text: String, font: FontId, at: [f32; 2], color: Color },
}

/// The pointer on the canvas, whether its main button is down, and how far its wheel turned
/// since the last frame (away from the player is more)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pointer {
    pub at: [f32; 2],
    pub down: bool,
    pub wheel: f32,
}

/// How a control looks
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Normal,
    /// The pointer is over it
    Hover,
    /// Pressed, with the pointer still over it
    Down,
    Disabled,
}

pub struct Ui {
    pub text: Font,
    pub big: Font,
    draws: Vec<Draw>,
    pointer: Pointer,
    was_down: bool,
    /// The control pressed and not yet let go, and the generation it was pressed in
    pressed: Option<(u32, u32)>,
    generation: u32,
}

impl Ui {
    pub fn new(text: Font, big: Font) -> Ui {
        Ui { text, big, draws: Vec::new(), pointer: Pointer::default(), was_down: false, pressed: None, generation: 0 }
    }

    /// With the game's fonts if `data/font` has them, or the dot font
    pub fn load(fonts: &Path) -> Ui {
        let font = |name: &str, height: usize| Font::read(fonts, name).unwrap_or_else(|e| {
            eprintln!("{e:#}: using the dot font");
            Font::dots(height)
        });
        // The original's session screen writes in the plain OCR-B 10, not the antialiased
        Ui::new(font("OCRB10", 11), font("OCRB24_A", 26))
    }

    pub fn font(&self, id: FontId) -> &Font {
        match id {
            FontId::Text => &self.text,
            FontId::Big => &self.big,
        }
    }

    /// A frame begins, with the pointer as it is now. A click counts only if pressed and let
    /// go in the same `generation`: one that spans a change of screen does nothing.
    pub fn begin(&mut self, pointer: Pointer, generation: u32) {
        self.pointer = pointer;
        self.generation = generation;
        self.draws.clear();
    }

    /// The frame is done: what to draw
    pub fn finish(&mut self) -> Vec<Draw> {
        if self.was_down && !self.pointer.down {
            self.pressed = None;
        }
        self.was_down = self.pointer.down;
        std::mem::take(&mut self.draws)
    }

    pub fn pointer(&self) -> Pointer {
        self.pointer
    }

    fn pressing(&self) -> bool {
        self.pointer.down && !self.was_down
    }

    fn releasing(&self) -> bool {
        !self.pointer.down && self.was_down
    }

    /// A control: how it looks, and whether it was clicked. It must be enabled both when it is
    /// pressed and when it is let go, with the pointer over it both times.
    pub fn control(&mut self, id: u32, rect: Rect, enabled: bool) -> (Look, bool) {
        let over = rect.contains(self.pointer.at);
        if self.pressing() && over && enabled {
            self.pressed = Some((id, self.generation));
        }
        let mine = self.pressed == Some((id, self.generation));
        let clicked = self.releasing() && mine && over && enabled;
        let look = match () {
            _ if !enabled => Look::Disabled,
            _ if mine && over && self.pointer.down => Look::Down,
            _ if over && (mine || self.pressed.is_none()) => Look::Hover,
            _ => Look::Normal,
        };
        (look, clicked)
    }

    pub fn fill(&mut self, rect: Rect, color: Color) {
        self.draws.push(Draw::Fill { rect, color });
    }

    pub fn frame(&mut self, rect: Rect, color: Color) {
        self.draws.push(Draw::Frame { rect, color });
    }

    pub fn art(&mut self, art: Art, state: usize, rect: Rect, fallback: Color) {
        self.draws.push(Draw::Art { art, state, rect, fallback });
    }

    pub fn tile(&mut self, art: Art, rect: Rect, fallback: Color) {
        self.draws.push(Draw::Tile { art, rect, fallback });
    }

    /// Text with its top left at `at`
    pub fn text(&mut self, text: &str, font: FontId, at: [f32; 2], color: Color) {
        self.draws.push(Draw::Text { text: text.to_string(), font, at, color });
    }

    /// Text in the middle of a rectangle
    pub fn centered(&mut self, text: &str, font: FontId, rect: Rect, color: Color) {
        let f = self.font(font);
        let at = [(rect.x + (rect.w - f.width(text)) / 2.0).round(), (rect.y + (rect.h - f.height()) / 2.0).round()];
        self.text(text, font, at, color);
    }

    /// Text from the left of a rectangle, `pad` in, in the middle up and down
    pub fn left(&mut self, text: &str, font: FontId, rect: Rect, pad: f32, color: Color) {
        let f = self.font(font);
        let at = [rect.x + pad, (rect.y + (rect.h - f.height()) / 2.0).round()];
        self.text(text, font, at, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        Ui::new(Font::dots(10), Font::dots(20))
    }

    /// One frame with the pointer at `at`, down or not, and a control over the left half
    fn frame(ui: &mut Ui, at: [f32; 2], down: bool, generation: u32, enabled: bool) -> (Look, bool) {
        ui.begin(Pointer { at, down, ..Pointer::default() }, generation);
        let r = ui.control(1, Rect::new(0.0, 0.0, 100.0, 100.0), enabled);
        ui.finish();
        r
    }

    const IN: [f32; 2] = [50.0, 50.0];
    const OUT: [f32; 2] = [150.0, 50.0];

    #[test]
    fn a_click_is_a_press_and_a_release_over_the_control() {
        let mut ui = ui();
        assert_eq!(frame(&mut ui, IN, false, 0, true), (Look::Hover, false));
        assert_eq!(frame(&mut ui, IN, true, 0, true), (Look::Down, false));
        assert_eq!(frame(&mut ui, IN, false, 0, true), (Look::Hover, true));
        // Let go elsewhere, it is not a click; nor is pressing elsewhere and letting go over it
        frame(&mut ui, IN, true, 0, true);
        assert_eq!(frame(&mut ui, OUT, true, 0, true).0, Look::Normal);
        assert!(!frame(&mut ui, OUT, false, 0, true).1);
        frame(&mut ui, OUT, true, 0, true);
        assert!(!frame(&mut ui, IN, false, 0, true).1);
    }

    #[test]
    fn a_control_must_be_enabled_when_pressed_and_when_let_go() {
        let mut ui = ui();
        // Pressed while disabled, enabled before the release
        frame(&mut ui, IN, true, 0, false);
        assert!(!frame(&mut ui, IN, false, 0, true).1);
        // Pressed while enabled, disabled before the release
        frame(&mut ui, IN, true, 0, true);
        assert_eq!(frame(&mut ui, IN, false, 0, false), (Look::Disabled, false));
    }

    #[test]
    fn a_click_that_spans_a_change_of_screen_does_nothing() {
        let mut ui = ui();
        frame(&mut ui, IN, true, 4, true);
        assert!(!frame(&mut ui, IN, false, 5, true).1);
        // And the next click works
        frame(&mut ui, IN, true, 5, true);
        assert!(frame(&mut ui, IN, false, 5, true).1);
    }

    #[test]
    fn the_canvas_fills_the_window_as_far_as_it_can_and_the_pointer_lands_on_it() {
        // A wide window: bars at the sides
        let c = Canvas::fit([1280.0, 720.0]);
        assert_eq!((c.scale, c.offset), (1.5, [160.0, 0.0]));
        assert_eq!(c.pointer([160.0, 0.0], [1280, 720], [1280, 720]), [0.0, 0.0]);
        assert_eq!(c.pointer([1120.0, 720.0], [1280, 720], [1280, 720]), [640.0, 480.0]);
        // A screen with twice the pixels: the window's units are half the framebuffer's
        let c = Canvas::fit([2560.0, 1440.0]);
        assert_eq!(c.pointer([640.0, 360.0], [1280, 720], [2560, 1440]), [320.0, 240.0]);
    }
}
