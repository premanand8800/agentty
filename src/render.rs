#![allow(clippy::too_many_arguments)] // drawing helpers take a rect, a color and a scale
//! Software renderer: draws the window into a 0RGB pixel buffer. Used by the window and by
//! headless snapshots, so screenshots show exactly what the app draws.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::CursorShape;

use crate::font::Fonts;
use crate::session::{Dims, Proxy, Status};
use crate::theme::{Rgb, Theme};

pub struct Frame<'a> {
    pub buf: &'a mut [u32],
    pub w: usize,
    pub h: usize,
}

impl Frame<'_> {
    pub fn fill(&mut self, x: isize, y: isize, w: usize, h: usize, c: Rgb) {
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = ((x + w as isize).max(0) as usize).min(self.w);
        let y1 = ((y + h as isize).max(0) as usize).min(self.h);
        if x0 >= x1 {
            return;
        }
        let v = c.to_u32();
        for row in y0..y1 {
            self.buf[row * self.w + x0..row * self.w + x1].fill(v);
        }
    }

    pub fn blend_pub(&mut self, x: isize, y: isize, c: Rgb, a: u8) {
        self.blend(x, y, c, a);
    }

    fn blend(&mut self, x: isize, y: isize, c: Rgb, a: u8) {
        if a == 0 || x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let i = y as usize * self.w + x as usize;
        if a == 255 {
            self.buf[i] = c.to_u32();
            return;
        }
        let p = self.buf[i];
        let (a, ia) = (a as u32, 255 - a as u32);
        let ch = |shift: u32, v: u8| ((((p >> shift) & 0xFF) * ia + v as u32 * a) / 255) << shift;
        self.buf[i] = ch(16, c.0) | ch(8, c.1) | ch(0, c.2);
    }

    /// Anti-aliased filled circle.
    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgb) {
        let (x0, x1) = ((cx - r - 1.0).floor() as isize, (cx + r + 1.0).ceil() as isize);
        let (y0, y1) = ((cy - r - 1.0).floor() as isize, (cy + r + 1.0).ceil() as isize);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let a = (r + 0.5 - d).clamp(0.0, 1.0);
                self.blend(x, y, c, (a * 255.0) as u8);
            }
        }
    }

    /// Rounded rectangle, used for tabs.
    pub fn rounded(&mut self, x: isize, y: isize, w: usize, h: usize, r: f32, c: Rgb) {
        let (fw, fh) = (w as f32, h as f32);
        for yy in 0..h {
            for xx in 0..w {
                let (px, py) = (xx as f32 + 0.5, yy as f32 + 0.5);
                let dx = (r - px).max(px - (fw - r)).max(0.0);
                let dy = (r - py).max(py - (fh - r)).max(0.0);
                let d = (dx * dx + dy * dy).sqrt();
                let a = (r + 0.5 - d).clamp(0.0, 1.0);
                if a > 0.0 {
                    self.blend(x + xx as isize, y + yy as isize, c, (a * 255.0) as u8);
                }
            }
        }
    }

    fn glyph(&mut self, fonts: &mut Fonts, ch: char, bold: bool, ui: bool, x: f32, baseline: f32, c: Rgb) -> f32 {
        self.styled_glyph(fonts, ch, bold, false, ui, x, baseline, c)
    }

    fn styled_glyph(
        &mut self,
        fonts: &mut Fonts,
        ch: char,
        bold: bool,
        italic: bool,
        ui: bool,
        x: f32,
        baseline: f32,
        c: Rgb,
    ) -> f32 {
        let g = if ui { fonts.ui_glyph(ch) } else { fonts.styled_glyph(ch, bold, italic) };
        let m = g.metrics;
        let gx = (x + m.xmin as f32).round() as isize;
        let gy = (baseline - m.height as f32 - m.ymin as f32).round() as isize;
        for row in 0..m.height {
            for col in 0..m.width {
                let a = g.coverage[row * m.width + col];
                if a > 0 {
                    self.blend(gx + col as isize, gy + row as isize, c, a);
                }
            }
        }
        m.advance_width
    }

    pub fn ui_text(&mut self, fonts: &mut Fonts, text: &str, x: f32, baseline: f32, c: Rgb) -> f32 {
        let mut pen = x;
        for ch in text.chars() {
            pen += self.glyph(fonts, ch, false, true, pen, baseline, c);
        }
        pen - x
    }
}

/// Pixel layout of the window, scaled for HiDPI.
#[derive(Clone, Copy)]
pub struct Layout {
    pub scale: f32,
    pub w: usize,
    pub h: usize,
    pub title_h: usize,
    pub tabs_h: usize,
    pub status_h: usize,
    pub pad: usize,
    /// The OS draws the title-bar buttons (macOS); leave their space empty.
    pub native_titlebar: bool,
}

impl Layout {
    pub fn new(w: usize, h: usize, scale: f32, show_tabs: bool) -> Layout {
        let s = |v: f32| (v * scale).round() as usize;
        Layout {
            scale,
            w,
            h,
            title_h: s(30.0),
            tabs_h: if show_tabs { s(30.0) } else { 0 },
            status_h: s(24.0),
            pad: s(10.0),
            native_titlebar: false,
        }
    }

    pub fn grid_origin(&self) -> (usize, usize) {
        (self.pad, self.title_h + self.tabs_h + self.pad)
    }

    pub fn grid_dims(&self, fonts: &Fonts) -> Dims {
        let w = self.w.saturating_sub(2 * self.pad);
        let h = self.h.saturating_sub(self.title_h + self.tabs_h + self.status_h + 2 * self.pad);
        Dims { cols: (w / fonts.cell_w).max(2), rows: (h / fonts.cell_h).max(1) }
    }

    /// Centers of the close / minimize / zoom buttons.
    pub fn traffic_lights(&self) -> [(f32, f32); 3] {
        let cy = self.title_h as f32 / 2.0;
        [20.0, 40.0, 60.0].map(|x| (x * self.scale, cy))
    }

    pub fn light_radius(&self) -> f32 {
        6.0 * self.scale
    }

    pub fn new_tab_button(&self) -> (usize, usize, usize) {
        let size = (22.0 * self.scale) as usize;
        (self.w - size - (10.0 * self.scale) as usize, (self.title_h - size) / 2, size)
    }

    pub fn tab_rects(&self, n: usize) -> Vec<(usize, usize)> {
        if n == 0 || self.tabs_h == 0 {
            return Vec::new();
        }
        let margin = (8.0 * self.scale) as usize;
        let avail = self.w.saturating_sub(2 * margin);
        let width = (avail / n).min((240.0 * self.scale) as usize);
        (0..n).map(|i| (margin + i * width, width)).collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Hover {
    #[default]
    None,
    Lights,
    NewTab,
    Tab(usize),
}

pub struct TabInfo {
    pub title: String,
    pub status: Status,
}

pub struct Chrome<'a> {
    pub window_title: String,
    pub tabs: &'a [TabInfo],
    pub active: usize,
    pub hover: Hover,
    pub focused: bool,
    pub status_left: String,
    pub status_right: String,
    pub pulse: f32,
}

pub fn status_color(s: Status) -> Rgb {
    match s {
        Status::Working => Rgb::hex(0x30D158),
        Status::NeedsYou => Rgb::hex(0xFF9F0A),
        Status::Idle => Rgb::hex(0x8E8E93),
        Status::Exited(0) => Rgb::hex(0x636366),
        Status::Exited(_) => Rgb::hex(0xFF453A),
    }
}

fn truncate(fonts: &mut Fonts, text: &str, max_w: usize) -> String {
    if fonts.ui_text_width(text) <= max_w {
        return text.to_string();
    }
    let mut s: String = text.to_string();
    while !s.is_empty() && fonts.ui_text_width(&(s.clone() + "…")) > max_w {
        s.pop();
    }
    s + "…"
}

pub fn draw_chrome(f: &mut Frame, fonts: &mut Fonts, theme: &Theme, l: &Layout, ui: &Chrome) {
    let ch = theme.chrome();
    let s = l.scale;
    let ui_base = |top: usize, height: usize, fonts: &Fonts| top as f32 + (height as f32 + fonts.ui_ascent()) / 2.0 - s;

    // Title bar.
    f.fill(0, 0, l.w, l.title_h, ch.titlebar);
    f.fill(0, l.title_h as isize - 1, l.w, 1, ch.border);
    let lights = [(0xFF5F57, 0xE0443E, '×'), (0xFEBC2E, 0xDEA123, '−'), (0x28C840, 0x1AAB29, '+')];
    for (i, (cx, cy)) in l.traffic_lights().into_iter().enumerate() {
        if l.native_titlebar {
            break;
        }
        let (fill, rim, sym) = lights[i];
        let (fill, rim) = if ui.focused || ui.hover == Hover::Lights {
            (Rgb::hex(fill), Rgb::hex(rim))
        } else if theme.is_dark() {
            (Rgb::hex(0x4A4A4C), Rgb::hex(0x3A3A3C))
        } else {
            (Rgb::hex(0xCDCDCD), Rgb::hex(0xB5B5B5))
        };
        f.circle(cx, cy, l.light_radius(), rim);
        f.circle(cx, cy, l.light_radius() - 0.6 * s, fill);
        if ui.hover == Hover::Lights {
            let w = fonts.ui_text_width(&sym.to_string()) as f32;
            f.ui_text(
                fonts,
                &sym.to_string(),
                cx - w / 2.0,
                cy + fonts.ui_ascent() / 2.0 - 1.5 * s,
                Rgb::hex(0x4D0000),
            );
        }
    }
    let title_max = l.w.saturating_sub((180.0 * s) as usize);
    let title = truncate(fonts, &ui.window_title, title_max);
    let tw = fonts.ui_text_width(&title);
    f.ui_text(fonts, &title, (l.w.saturating_sub(tw) / 2) as f32, ui_base(0, l.title_h, fonts), ch.titlebar_text);

    // New-tab button.
    let (bx, by, bs) = l.new_tab_button();
    if ui.hover == Hover::NewTab {
        f.rounded(bx as isize, by as isize, bs, bs, 5.0 * s, ch.tab_active);
    }
    let pw = fonts.ui_text_width("+") as f32;
    f.ui_text(fonts, "+", bx as f32 + (bs as f32 - pw) / 2.0, ui_base(by, bs, fonts), ch.titlebar_text);

    // Tab strip.
    if l.tabs_h > 0 {
        let top = l.title_h;
        f.fill(0, top as isize, l.w, l.tabs_h, ch.tab_inactive);
        f.fill(0, (top + l.tabs_h) as isize - 1, l.w, 1, ch.border);
        for (i, (x, w)) in l.tab_rects(ui.tabs.len()).into_iter().enumerate() {
            let tab = &ui.tabs[i];
            let inset = (3.0 * s) as usize;
            let bg = if i == ui.active {
                Some(ch.tab_active)
            } else if ui.hover == Hover::Tab(i) {
                Some(ch.tab_active.mix(ch.tab_inactive, 0.5))
            } else {
                None
            };
            if let Some(bg) = bg {
                f.rounded(
                    (x + inset) as isize,
                    (top + inset) as isize,
                    w - 2 * inset,
                    l.tabs_h - 2 * inset,
                    6.0 * s,
                    bg,
                );
            }
            let dot_x = x as f32 + 16.0 * s;
            let dot_y = top as f32 + l.tabs_h as f32 / 2.0;
            let mut color = status_color(tab.status);
            if tab.status == Status::NeedsYou {
                color = color.mix(ch.tab_inactive, 0.45 * ui.pulse);
            }
            f.circle(dot_x, dot_y, 4.0 * s, color);
            let text_color = if i == ui.active { ch.tab_text } else { ch.tab_text_dim };
            let label = truncate(fonts, &tab.title, w.saturating_sub((40.0 * s) as usize));
            f.ui_text(fonts, &label, dot_x + 12.0 * s, ui_base(top, l.tabs_h, fonts), text_color);
        }
    }

    // Status bar.
    let top = l.h.saturating_sub(l.status_h);
    f.fill(0, top as isize, l.w, l.status_h, ch.status_bg);
    let base = ui_base(top, l.status_h, fonts);
    if let Some(tab) = ui.tabs.get(ui.active) {
        f.circle(14.0 * s, top as f32 + l.status_h as f32 / 2.0, 3.5 * s, status_color(tab.status));
    }
    f.ui_text(fonts, &ui.status_left, 24.0 * s, base, ch.status_text);
    let rw = fonts.ui_text_width(&ui.status_right);
    f.ui_text(fonts, &ui.status_right, l.w.saturating_sub(rw + (12.0 * s) as usize) as f32, base, ch.status_text);
}

/// Draw the visible part of a terminal at the layout's grid origin.
/// An ambient background to draw behind the text: effect, time in seconds, intensity 0..1.
#[derive(Clone, Copy)]
pub struct Ambient {
    pub effect: crate::ambient::Effect,
    pub time: f32,
    pub intensity: f32,
}

pub fn draw_terminal(
    f: &mut Frame,
    fonts: &mut Fonts,
    theme: &Theme,
    l: &Layout,
    term: &Term<Proxy>,
    focused: bool,
    ambient: Option<Ambient>,
) {
    let (ox, oy) = l.grid_origin();
    let grid_bottom = l.h.saturating_sub(l.status_h);
    f.fill(0, (l.title_h + l.tabs_h) as isize, l.w, grid_bottom.saturating_sub(l.title_h + l.tabs_h), theme.bg);

    let (cw, chh) = (fonts.cell_w, fonts.cell_h);
    let rows = term.screen_lines() as i32;
    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let colors = content.colors;
    let selection = content.selection;
    let cursor = content.cursor;

    for cell in content.display_iter {
        let vrow = cell.point.line.0 + offset;
        if vrow < 0 || vrow >= rows {
            continue;
        }
        let flags = cell.flags;
        if flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let bold = flags.contains(Flags::BOLD);
        let mut fg = theme.resolve(cell.fg, colors, bold);
        let mut bg = theme.resolve(cell.bg, colors, false);
        if flags.contains(Flags::DIM) {
            fg = fg.dim();
        }
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if selection.is_some_and(|s| s.contains(cell.point)) {
            bg = theme.selection;
        }
        let x = ox + cell.point.column.0 * cw;
        let y = oy + vrow as usize * chh;
        let width = if flags.contains(Flags::WIDE_CHAR) { 2 * cw } else { cw };
        if bg != theme.bg {
            f.fill(x as isize, y as isize, width, chh, bg);
        }
        let c = cell.c;
        if c != ' '
            && c != '\0'
            && !flags.contains(Flags::HIDDEN)
            && !crate::boxdraw::draw(f, c, x, y, width, chh, fg, l.scale)
        {
            let italic = flags.contains(Flags::ITALIC);
            f.styled_glyph(fonts, c, bold, italic, false, x as f32, y as f32 + fonts.ascent, fg);
        }
        if flags.contains(Flags::UNDERLINE) {
            f.fill(x as isize, (y as f32 + fonts.ascent + 2.0) as isize, width, 1.max(l.scale as usize), fg);
        }
        if flags.contains(Flags::STRIKEOUT) {
            f.fill(x as isize, (y + chh / 2) as isize, width, 1.max(l.scale as usize), fg);
        }
    }

    // Ambient background, after the text so it only touches plain background pixels.
    if let Some(a) = ambient {
        let top = (l.title_h + l.tabs_h) as f32;
        let area = crate::ambient::Area { x: 0.0, y: top, w: l.w as f32, h: grid_bottom as f32 - top };
        crate::ambient::draw(f, a.effect, area, a.time, theme, a.intensity, l.scale);
    }

    // Cursor.
    let vrow = cursor.point.line.0 + offset;
    if cursor.shape != CursorShape::Hidden && vrow >= 0 && vrow < rows {
        let x = (ox + cursor.point.column.0 * cw) as isize;
        let y = (oy + vrow as usize * chh) as isize;
        let color = theme.resolve(
            alacritty_terminal::vte::ansi::Color::Named(alacritty_terminal::vte::ansi::NamedColor::Cursor),
            colors,
            false,
        );
        let t = (1.5 * l.scale).max(1.0) as usize;
        if !focused {
            f.fill(x, y, cw, t, color);
            f.fill(x, y + chh as isize - t as isize, cw, t, color);
            f.fill(x, y, t, chh, color);
            f.fill(x + cw as isize - t as isize, y, t, chh, color);
        } else {
            match cursor.shape {
                CursorShape::Beam => f.fill(x, y, (2.0 * l.scale) as usize, chh, color),
                CursorShape::Underline => {
                    f.fill(x, y + chh as isize - (2.0 * l.scale) as isize, cw, (2.0 * l.scale) as usize, color)
                }
                _ => {
                    f.fill(x, y, cw, chh, color);
                    let cell = &term.grid()[cursor.point];
                    if cell.c != ' ' {
                        f.glyph(
                            fonts,
                            cell.c,
                            cell.flags.contains(Flags::BOLD),
                            false,
                            x as f32,
                            y as f32 + fonts.ascent,
                            theme.bg,
                        );
                    }
                }
            }
        }
    }
}
