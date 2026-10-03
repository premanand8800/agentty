//! Glyph rasterization with a bounded cache. CPU only: no GPU context, no atlas textures.
//!
//! Memory: fonts are parsed lazily (ab_glyph reads an outline only when a glyph is first drawn),
//! and fallback fonts are loaded only when the primary font lacks a character. A large CJK
//! fallback costs nothing until CJK text actually appears.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MONO_CANDIDATES: &[&str] = &[
    // Menlo (macOS Terminal's font) is derived from DejaVu Sans Mono.
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
];
const UI_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
];
/// Tried in order, each loaded only on the first character the earlier fonts lack.
const FALLBACK_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansSymbols-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansMath-Regular.ttf",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
];
const MAX_CACHED_GLYPHS: usize = 2048;

fn load(path: &Path) -> Option<FontVec> {
    let bytes = std::fs::read(path).ok()?;
    FontVec::try_from_vec(bytes).ok()
}

fn variant_path(regular: &Path, style: &str) -> Option<PathBuf> {
    let s = regular.to_string_lossy();
    // DejaVu uses -Bold / -Oblique / -BoldOblique; Noto and Liberation use -Bold / -Italic / -BoldItalic.
    let alt = style.replace("Oblique", "Italic");
    for (a, b) in [("-Regular", format!("-{style}")), ("-Regular", format!("-{alt}")), (".ttf", format!("-{style}.ttf"))] {
        if s.contains(a) {
            let p = PathBuf::from(s.replacen(a, &b, 1));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Glyph placement relative to the pen position on the baseline (same conventions as fontdue).
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub xmin: i32,
    /// Offset of the bitmap's bottom edge above the baseline.
    pub ymin: i32,
    pub width: usize,
    pub height: usize,
    pub advance_width: f32,
}

pub struct Glyph {
    pub metrics: Metrics,
    pub coverage: Vec<u8>,
}

struct Fallback {
    path: &'static str,
    font: Option<Option<FontVec>>, // None = not tried yet; Some(None) = failed to load
}

pub struct Fonts {
    regular: FontVec,
    bold: Option<FontVec>,
    italic: Option<FontVec>,
    bold_italic: Option<FontVec>,
    ui: Option<FontVec>,
    fallbacks: Vec<Fallback>,
    /// Which fallback covers a character (None = no font has it). Avoids rescanning.
    fallback_for: HashMap<char, Option<usize>>,
    pub px: f32,
    pub ui_px: f32,
    pub cell_w: usize,
    pub cell_h: usize,
    pub ascent: f32,
    cache: HashMap<(char, u8, bool), Glyph>,
}

/// Scale so `px` is the em size, as in CSS and most terminals. (ab_glyph's PxScale is the line height.)
fn em(font: &FontVec, px: f32) -> PxScale {
    let upem = font.units_per_em().unwrap_or(1000.0);
    PxScale::from(px * font.height_unscaled() / upem)
}

fn rasterize(font: &FontVec, c: char, px: f32) -> Glyph {
    let scale = em(font, px);
    let scaled = font.as_scaled(scale);
    let id = font.glyph_id(c);
    let advance_width = scaled.h_advance(id);
    let glyph = id.with_scale(scale);
    match font.outline_glyph(glyph) {
        Some(outlined) => {
            let b = outlined.px_bounds();
            let (width, height) = (b.width().ceil() as usize, b.height().ceil() as usize);
            let mut coverage = vec![0u8; width * height];
            outlined.draw(|x, y, cov| {
                let (x, y) = (x as usize, y as usize);
                if x < width && y < height {
                    coverage[y * width + x] = (cov.clamp(0.0, 1.0) * 255.0) as u8;
                }
            });
            Glyph {
                metrics: Metrics { xmin: b.min.x.floor() as i32, ymin: -(b.min.y.floor() as i32) - height as i32, width, height, advance_width },
                coverage,
            }
        }
        None => Glyph { metrics: Metrics { xmin: 0, ymin: 0, width: 0, height: 0, advance_width }, coverage: Vec::new() },
    }
}

fn has(font: &FontVec, c: char) -> bool {
    font.glyph_id(c).0 != 0
}

impl Fonts {
    pub fn new(custom: Option<&Path>, px: f32, ui_px: f32) -> Result<Fonts, String> {
        let regular_path = match custom {
            Some(p) => p.to_path_buf(),
            None => MONO_CANDIDATES
                .iter()
                .map(PathBuf::from)
                .find(|p| p.is_file())
                .ok_or("no monospace font found; set `font` in config.toml")?,
        };
        let regular = load(&regular_path).ok_or_else(|| format!("cannot load font {}", regular_path.display()))?;
        let variant = |style: &str| variant_path(&regular_path, style).and_then(|p| load(&p));
        let (bold, italic, bold_italic) = (variant("Bold"), variant("Oblique"), variant("BoldOblique"));
        let ui = UI_CANDIDATES.iter().find_map(|p| load(Path::new(p)));
        let fallbacks = FALLBACK_CANDIDATES.iter().map(|p| Fallback { path: p, font: None }).collect();
        let mut f = Fonts {
            regular,
            bold,
            italic,
            bold_italic,
            ui,
            fallbacks,
            fallback_for: HashMap::new(),
            px,
            ui_px,
            cell_w: 1,
            cell_h: 1,
            ascent: 0.0,
            cache: HashMap::new(),
        };
        f.set_size(px, ui_px);
        Ok(f)
    }

    pub fn set_size(&mut self, px: f32, ui_px: f32) {
        self.px = px;
        self.ui_px = ui_px;
        self.cache.clear();
        let s = self.regular.as_scaled(em(&self.regular, px));
        self.cell_w = s.h_advance(self.regular.glyph_id('M')).round().max(1.0) as usize;
        self.cell_h = (s.ascent() - s.descent() + s.line_gap()).ceil().max(1.0) as usize + 2;
        self.ascent = s.ascent().ceil() + 1.0;
    }

    fn primary(&self, bold: bool, italic: bool) -> &FontVec {
        match (bold, italic) {
            (true, true) => self.bold_italic.as_ref().or(self.bold.as_ref()).unwrap_or(&self.regular),
            (true, false) => self.bold.as_ref().unwrap_or(&self.regular),
            (false, true) => self.italic.as_ref().unwrap_or(&self.regular),
            _ => &self.regular,
        }
    }

    /// Index of a fallback font covering `c`, loading fallbacks lazily in order.
    fn fallback_index(&mut self, c: char) -> Option<usize> {
        if let Some(hit) = self.fallback_for.get(&c) {
            return *hit;
        }
        let mut found = None;
        for (i, fb) in self.fallbacks.iter_mut().enumerate() {
            let font = fb.font.get_or_insert_with(|| load(Path::new(fb.path)));
            if font.as_ref().is_some_and(|f| has(f, c)) {
                found = Some(i);
                break;
            }
        }
        if self.fallback_for.len() > 4096 {
            self.fallback_for.clear();
        }
        self.fallback_for.insert(c, found);
        found
    }

    pub fn styled_glyph(&mut self, c: char, bold: bool, italic: bool) -> &Glyph {
        self.glyph_inner(c, bold, italic, false)
    }

    pub fn ui_glyph(&mut self, c: char) -> &Glyph {
        self.glyph_inner(c, false, false, true)
    }

    fn glyph_inner(&mut self, c: char, bold: bool, italic: bool, ui: bool) -> &Glyph {
        if self.cache.len() >= MAX_CACHED_GLYPHS {
            self.cache.clear(); // bounded memory; re-rasterizing is cheap
        }
        let key = (c, bold as u8 | (italic as u8) << 1, ui);
        if !self.cache.contains_key(&key) {
            let px = if ui { self.ui_px } else { self.px };
            let in_ui = ui && self.ui.as_ref().is_some_and(|f| has(f, c));
            let glyph = if in_ui {
                rasterize(self.ui.as_ref().unwrap(), c, px)
            } else if has(self.primary(bold, italic), c) {
                rasterize(self.primary(bold, italic), c, px)
            } else if let Some(i) = self.fallback_index(c) {
                rasterize(self.fallbacks[i].font.as_ref().unwrap().as_ref().unwrap(), c, px)
            } else {
                rasterize(self.primary(bold, italic), c, px) // .notdef box
            };
            self.cache.insert(key, glyph);
        }
        &self.cache[&key]
    }

    pub fn ui_text_width(&mut self, text: &str) -> usize {
        text.chars().map(|c| self.ui_glyph(c).metrics.advance_width).sum::<f32>().ceil() as usize
    }

    pub fn ui_ascent(&self) -> f32 {
        let font = self.ui.as_ref().unwrap_or(&self.regular);
        font.as_scaled(em(font, self.ui_px)).ascent()
    }
}
