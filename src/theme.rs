//! Color themes. The first nine match macOS Terminal's built-in profiles.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex(v: u32) -> Rgb {
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
    pub fn to_u32(self) -> u32 {
        (self.0 as u32) << 16 | (self.1 as u32) << 8 | self.2 as u32
    }
    pub fn luma(self) -> f32 {
        0.2126 * self.0 as f32 + 0.7152 * self.1 as f32 + 0.0722 * self.2 as f32
    }
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }
    pub fn dim(self) -> Rgb {
        Rgb((self.0 as u32 * 2 / 3) as u8, (self.1 as u32 * 2 / 3) as u8, (self.2 as u32 * 2 / 3) as u8)
    }
}

/// Terminal.app's ANSI palette, shared by its profiles.
const MAC_ANSI: [u32; 16] = [
    0x000000, 0xC23621, 0x25BC24, 0xADAD27, 0x492EE1, 0xD338D3, 0x33BBC8, 0xCBCCCD, //
    0x818383, 0xFC391F, 0x31E722, 0xEAEC23, 0x5833FF, 0xF935F8, 0x14F0F0, 0xE9EBEB,
];

/// A softer palette for the Midnight theme.
const MIDNIGHT_ANSI: [u32; 16] = [
    0x1B1E28, 0xE5737A, 0x8CCF7E, 0xE5C76B, 0x6CB6EB, 0xC68AEE, 0x6CD0D5, 0xC9CED9, //
    0x5B6170, 0xF28B91, 0xA6E094, 0xF0D68A, 0x8CC8F2, 0xD6A4F5, 0x8ADDE1, 0xF0F3F8,
];

pub struct Theme {
    pub name: &'static str,
    pub bg: Rgb,
    pub fg: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    ansi: &'static [u32; 16],
}

macro_rules! theme {
    ($name:expr, $bg:expr, $fg:expr, $cursor:expr, $sel:expr, $ansi:expr) => {
        Theme {
            name: $name,
            bg: Rgb::hex($bg),
            fg: Rgb::hex($fg),
            cursor: Rgb::hex($cursor),
            selection: Rgb::hex($sel),
            ansi: $ansi,
        }
    };
}

pub static THEMES: [Theme; 10] = [
    theme!("Pro", 0x1A1A1A, 0xF2F2F2, 0x9A9A9A, 0x414141, &MAC_ANSI),
    theme!("Basic", 0xFFFFFF, 0x000000, 0x7F7F7F, 0xB4D5FE, &MAC_ANSI),
    theme!("Midnight", 0x14161D, 0xD5DAE3, 0x8CC8F2, 0x2E3442, &MIDNIGHT_ANSI),
    theme!("Homebrew", 0x000000, 0x28FE14, 0x38FE27, 0x083905, &MAC_ANSI),
    theme!("Ocean", 0x224FBC, 0xFFFFFF, 0x7F7F7F, 0x216DFF, &MAC_ANSI),
    theme!("Grass", 0x13773D, 0xFFF0A5, 0x8C2800, 0xB64926, &MAC_ANSI),
    theme!("Man Page", 0xFEF49C, 0x000000, 0x7F7F7F, 0xA4C9CD, &MAC_ANSI),
    theme!("Novel", 0xDFDBC3, 0x3B2322, 0x3A2322, 0xA4A390, &MAC_ANSI),
    theme!("Red Sands", 0x7A251E, 0xD7C9A7, 0xFFFFFF, 0x3F1B16, &MAC_ANSI),
    theme!("Silver Aerogel", 0x929292, 0x000000, 0x7F7F7F, 0x787878, &MAC_ANSI),
];

pub fn by_name(name: &str) -> Option<usize> {
    THEMES.iter().position(|t| t.name.eq_ignore_ascii_case(name))
}

/// Colors for the window chrome (title bar, tabs, status bar), derived from the theme.
pub struct Chrome {
    pub titlebar: Rgb,
    pub titlebar_text: Rgb,
    pub tab_active: Rgb,
    pub tab_inactive: Rgb,
    pub tab_text: Rgb,
    pub tab_text_dim: Rgb,
    pub border: Rgb,
    pub status_bg: Rgb,
    pub status_text: Rgb,
}

impl Theme {
    pub fn is_dark(&self) -> bool {
        self.bg.luma() < 110.0
    }

    pub fn chrome(&self) -> Chrome {
        let mut c = self.base_chrome();
        // Pick status text by the bar's own brightness: mid-gray themes (Silver Aerogel) need dark text.
        c.status_text = if c.status_bg.luma() > 120.0 { Rgb::hex(0x2C2C2E) } else { Rgb::hex(0xA1A1A6) };
        c
    }

    fn base_chrome(&self) -> Chrome {
        if self.is_dark() {
            Chrome {
                titlebar: Rgb::hex(0x2C2C2E),
                titlebar_text: Rgb::hex(0xC7C7CC),
                tab_active: Rgb::hex(0x3A3A3C),
                tab_inactive: Rgb::hex(0x242426),
                tab_text: Rgb::hex(0xE5E5EA),
                tab_text_dim: Rgb::hex(0x8E8E93),
                border: Rgb::hex(0x000000),
                status_bg: self.bg.mix(Rgb::hex(0x000000), 0.35),
                status_text: Rgb::hex(0x9A9AA0),
            }
        } else {
            Chrome {
                titlebar: Rgb::hex(0xE7E5E7),
                titlebar_text: Rgb::hex(0x4D4D4F),
                tab_active: Rgb::hex(0xF6F5F6),
                tab_inactive: Rgb::hex(0xD5D3D5),
                tab_text: Rgb::hex(0x1C1C1E),
                tab_text_dim: Rgb::hex(0x6E6E73),
                border: Rgb::hex(0xB8B6B8),
                status_bg: self.bg.mix(Rgb::hex(0x000000), 0.06),
                status_text: Rgb::hex(0x6E6E73),
            }
        }
    }

    pub fn palette(&self, index: usize) -> Rgb {
        match index {
            0..=15 => Rgb::hex(self.ansi[index]),
            16..=231 => {
                let i = index - 16;
                let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
                Rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
            }
            232..=255 => {
                let v = (8 + (index - 232) * 10) as u8;
                Rgb(v, v, v)
            }
            _ => self.fg,
        }
    }

    /// Resolve a cell color, honoring palette changes the program made (OSC 4/10/11).
    pub fn resolve(&self, color: Color, overrides: &Colors, bold: bool) -> Rgb {
        let ov = |i: usize| overrides[i].map(|c| Rgb(c.r, c.g, c.b));
        match color {
            Color::Spec(c) => Rgb(c.r, c.g, c.b),
            Color::Indexed(i) => {
                let i = if bold && i < 8 { i + 8 } else { i } as usize;
                ov(i).unwrap_or_else(|| self.palette(i))
            }
            Color::Named(n) => {
                let idx = n as usize;
                let named_bright = bold && idx < 8;
                let idx = if named_bright { idx + 8 } else { idx };
                if let Some(c) = ov(idx) {
                    return c;
                }
                match n {
                    NamedColor::Foreground | NamedColor::BrightForeground => self.fg,
                    NamedColor::Background => self.bg,
                    NamedColor::Cursor => self.cursor,
                    NamedColor::DimForeground => self.fg.dim(),
                    NamedColor::DimBlack
                    | NamedColor::DimRed
                    | NamedColor::DimGreen
                    | NamedColor::DimYellow
                    | NamedColor::DimBlue
                    | NamedColor::DimMagenta
                    | NamedColor::DimCyan
                    | NamedColor::DimWhite => self.palette(idx - NamedColor::DimBlack as usize).dim(),
                    _ => self.palette(idx),
                }
            }
        }
    }
}
