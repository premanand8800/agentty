//! Optional ambient backgrounds (`Ctrl+Shift+B`): soft motion behind the text for long sessions.
//!
//! Every effect is a pure function of time: particle positions come from a hash of the particle
//! index, so there is no per-frame state and no memory growth. Effects only blend into pixels
//! that show the default background; text and colored cells are drawn on top. Off by default.

use std::f32::consts::TAU;

use crate::render::Frame;
use crate::theme::{Rgb, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Off,
    Starfield,
    Rain,
    Snow,
    Fireflies,
    Aurora,
}

impl Effect {
    pub const ALL: [Effect; 6] =
        [Effect::Off, Effect::Starfield, Effect::Rain, Effect::Snow, Effect::Fireflies, Effect::Aurora];

    pub fn name(self) -> &'static str {
        match self {
            Effect::Off => "off",
            Effect::Starfield => "starfield",
            Effect::Rain => "rain",
            Effect::Snow => "snow",
            Effect::Fireflies => "fireflies",
            Effect::Aurora => "aurora",
        }
    }

    pub fn parse(s: &str) -> Option<Effect> {
        Effect::ALL.into_iter().find(|e| e.name().eq_ignore_ascii_case(s.trim()))
    }

    pub fn next(self) -> Effect {
        let i = Effect::ALL.iter().position(|e| *e == self).unwrap_or(0);
        Effect::ALL[(i + 1) % Effect::ALL.len()]
    }
}

/// Deterministic value in [0, 1) for particle `i` and property `salt`.
fn hash(i: u32, salt: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// The area to draw in, in pixels.
#[derive(Clone, Copy)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

struct Painter<'a, 'b> {
    f: &'a mut Frame<'b>,
    area: Area,
    bg: u32,
}

impl Painter<'_, '_> {
    /// Blend only over the plain background, so text and colored cells stay untouched.
    fn dot(&mut self, x: f32, y: f32, c: Rgb, a: f32) {
        if a <= 0.0
            || x < self.area.x
            || y < self.area.y
            || x >= self.area.x + self.area.w
            || y >= self.area.y + self.area.h
        {
            return;
        }
        let (xi, yi) = (x as usize, y as usize);
        if xi >= self.f.w || yi >= self.f.h || self.f.buf[yi * self.f.w + xi] != self.bg {
            return;
        }
        self.f.blend_pub(xi as isize, yi as isize, c, (a.min(1.0) * 255.0) as u8);
    }

    /// Soft round blob with a smooth falloff (glows, flakes).
    fn blob(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, a: f32) {
        let ri = r.ceil() as i32 + 1;
        for dy in -ri..=ri {
            for dx in -ri..=ri {
                let d = ((dx * dx + dy * dy) as f32).sqrt() / r;
                if d < 1.0 {
                    let k = (1.0 - d * d) * (1.0 - d * d);
                    self.dot(cx + dx as f32, cy + dy as f32, c, a * k);
                }
            }
        }
    }
}

/// Draw one frame of `effect` at time `t` seconds. `intensity` is 0..1.
pub fn draw(f: &mut Frame, effect: Effect, area: Area, t: f32, theme: &Theme, intensity: f32, scale: f32) {
    if effect == Effect::Off || area.w < 8.0 || area.h < 8.0 {
        return;
    }
    let dark = theme.is_dark();
    // Light themes get darker, quieter particles so the effect never fights the text.
    let k = intensity.clamp(0.0, 1.0) * if dark { 1.0 } else { 0.6 };
    let ink = if dark { Rgb(225, 232, 255) } else { theme.fg.mix(theme.bg, 0.35) };
    let mut p = Painter { bg: theme.bg.to_u32(), f, area };
    let Area { x, y, w, h } = area;
    match effect {
        Effect::Off => {}
        Effect::Starfield => {
            let n = ((w * h) / (3200.0 * scale * scale)).min(700.0) as u32;
            for i in 0..n {
                let speed = (1.5 + hash(i, 3) * 5.0) * scale;
                let sx = x + (hash(i, 1) * w + t * speed) % w;
                let sy = y + hash(i, 2) * h;
                let twinkle = 0.5 + 0.5 * (t * (0.6 + hash(i, 4) * 1.8) + hash(i, 5) * TAU).sin();
                let a = k * (0.12 + 0.38 * twinkle) * (0.4 + hash(i, 6));
                p.dot(sx, sy, ink, a);
                if hash(i, 7) > 0.85 {
                    // a few brighter stars get a soft cross
                    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                        p.dot(sx + dx * scale, sy + dy * scale, ink, a * 0.45);
                    }
                }
            }
        }
        Effect::Rain => {
            let n = (w / (9.0 * scale)).min(320.0) as u32;
            let drop = if dark { Rgb(150, 185, 230) } else { theme.fg.mix(Rgb(70, 110, 170), 0.5) };
            for i in 0..n {
                let len = (10.0 + hash(i, 2) * 16.0) * scale;
                let speed = (140.0 + hash(i, 3) * 180.0) * scale;
                let dx = x + hash(i, 1) * w;
                let head = (hash(i, 4) * (h + len) + t * speed) % (h + len);
                let a = k * (0.10 + 0.22 * hash(i, 5));
                let steps = len as i32;
                for s in 0..steps {
                    let fade = 1.0 - s as f32 / len; // bright head, fading tail
                    p.dot(dx - s as f32 * 0.12, y + head - s as f32, drop, a * fade);
                }
            }
        }
        Effect::Snow => {
            let n = ((w * h) / (8000.0 * scale * scale)).min(260.0) as u32;
            for i in 0..n {
                let r = (1.0 + hash(i, 2) * 1.8) * scale;
                let speed = (10.0 + hash(i, 3) * 26.0) * scale;
                let sway = (6.0 + hash(i, 4) * 16.0) * scale * (t * (0.3 + hash(i, 5) * 0.5) + hash(i, 6) * TAU).sin();
                let fx = x + (hash(i, 1) * w + sway).rem_euclid(w);
                let fy = y + (hash(i, 7) * h + t * speed) % h;
                p.blob(fx, fy, r, ink, k * (0.25 + 0.3 * hash(i, 8)));
            }
        }
        Effect::Fireflies => {
            let glow = if dark { Rgb(255, 214, 120) } else { Rgb(200, 140, 30) };
            for i in 0..16u32 {
                let fx = x
                    + (hash(i, 1) * w + (t * (0.05 + hash(i, 2) * 0.08) + hash(i, 3) * TAU).sin() * w * 0.12)
                        .rem_euclid(w);
                let fy = y
                    + (hash(i, 4) * h + (t * (0.04 + hash(i, 5) * 0.07) + hash(i, 6) * TAU).cos() * h * 0.12)
                        .rem_euclid(h);
                let pulse = (0.5 + 0.5 * (t * (0.5 + hash(i, 7) * 0.9) + hash(i, 8) * TAU).sin()).powi(2);
                p.blob(fx, fy, (5.0 + hash(i, 9) * 5.0) * scale, glow, k * 0.38 * pulse);
                p.blob(fx, fy, 1.6 * scale, glow, k * 0.8 * pulse);
            }
        }
        Effect::Aurora => {
            let (c1, c2) = (Rgb(70, 230, 170), Rgb(150, 110, 245));
            let step = (2.0 * scale).max(2.0);
            let mut cx = 0.0;
            while cx < w {
                let u = cx / scale;
                let center =
                    h * 0.24 + (u * 0.004 + t * 0.15).sin() * h * 0.07 + (u * 0.011 - t * 0.23).sin() * h * 0.03;
                let thick = h * 0.17 * (0.7 + 0.3 * (u * 0.006 + t * 0.1).sin());
                let color = c1.mix(c2, 0.5 + 0.5 * (u * 0.003 - t * 0.08).sin());
                let mut cy = (center - thick).max(0.0);
                while cy < (center + thick).min(h) {
                    let d = ((cy - center).abs() / thick).min(1.0);
                    let a = k * 0.22 * (1.0 - d) * (1.0 - d);
                    for (ox, oy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                        p.dot(x + cx + ox * step / 2.0, y + cy + oy * step / 2.0, color, a);
                    }
                    cy += step;
                }
                cx += step;
            }
        }
    }
}
