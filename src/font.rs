//! Glyph rasterization with a bounded cache. CPU only: no GPU context, no atlas textures.
//!
//! Memory: fonts are parsed lazily (ab_glyph reads an outline only when a glyph is first drawn),
//! and fallback fonts are loaded only when the primary font lacks a character. A large CJK
//! fallback costs nothing until CJK text actually appears.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A font file and the face index inside it (.ttc collections hold several faces).
#[derive(Clone)]
struct Face {
    path: PathBuf,
    index: u32,
}

fn face(path: impl Into<PathBuf>, index: u32) -> Face {
    Face { path: path.into(), index }
}

/// A monospace family: regular, bold, italic, bold italic.
struct Family(Face, Option<Face>, Option<Face>, Option<Face>);

#[cfg(windows)]
fn sys_fonts() -> PathBuf {
    PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts")
}

fn mono_families() -> Vec<Family> {
    #[cfg(target_os = "macos")]
    {
        let menlo = "/System/Library/Fonts/Menlo.ttc";
        vec![
            Family(face(menlo, 0), Some(face(menlo, 1)), Some(face(menlo, 2)), Some(face(menlo, 3))),
            Family(face("/System/Library/Fonts/Monaco.ttf", 0), None, None, None),
        ]
    }
    #[cfg(windows)]
    {
        let f = sys_fonts();
        vec![
            Family(
                face(f.join("consola.ttf"), 0),
                Some(face(f.join("consolab.ttf"), 0)),
                Some(face(f.join("consolai.ttf"), 0)),
                Some(face(f.join("consolaz.ttf"), 0)),
            ),
            Family(face(f.join("CascadiaMono.ttf"), 0), None, None, None),
            Family(
                face(f.join("cour.ttf"), 0),
                Some(face(f.join("courbd.ttf"), 0)),
                Some(face(f.join("couri.ttf"), 0)),
                Some(face(f.join("courbi.ttf"), 0)),
            ),
        ]
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        // Menlo (macOS Terminal's font) is derived from DejaVu Sans Mono.
        let mut out = Vec::new();
        for dir in ["/usr/share/fonts/truetype/dejavu", "/usr/share/fonts/TTF", "/usr/share/fonts/dejavu"] {
            let p = |n: &str| face(format!("{dir}/{n}"), 0);
            out.push(Family(
                p("DejaVuSansMono.ttf"),
                Some(p("DejaVuSansMono-Bold.ttf")),
                Some(p("DejaVuSansMono-Oblique.ttf")),
                Some(p("DejaVuSansMono-BoldOblique.ttf")),
            ));
        }
        let n = |f: &str| face(format!("/usr/share/fonts/truetype/{f}"), 0);
        out.push(Family(n("noto/NotoSansMono-Regular.ttf"), Some(n("noto/NotoSansMono-Bold.ttf")), None, None));
        out.push(Family(
            n("liberation/LiberationMono-Regular.ttf"),
            Some(n("liberation/LiberationMono-Bold.ttf")),
            Some(n("liberation/LiberationMono-Italic.ttf")),
            Some(n("liberation/LiberationMono-BoldItalic.ttf")),
        ));
        out
    }
}

fn ui_candidates() -> Vec<Face> {
    #[cfg(target_os = "macos")]
    {
        vec![face("/System/Library/Fonts/SFNS.ttf", 0), face("/System/Library/Fonts/Helvetica.ttc", 0)]
    }
    #[cfg(windows)]
    {
        vec![face(sys_fonts().join("segoeui.ttf"), 0), face(sys_fonts().join("arial.ttf"), 0)]
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        ]
        .into_iter()
        .map(|p| face(p, 0))
        .collect()
    }
}

/// Tried in order, each loaded only on the first character the earlier fonts lack.
fn fallback_candidates() -> Vec<Face> {
    #[cfg(target_os = "macos")]
    {
        [
            "/System/Library/Fonts/Apple Symbols.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
        ]
        .into_iter()
        .map(|p| face(p, 0))
        .collect()
    }
    #[cfg(windows)]
    {
        let f = sys_fonts();
        ["seguisym.ttf", "segoeui.ttf", "msyh.ttc", "YuGothR.ttc", "malgun.ttf"]
            .into_iter()
            .map(|n| face(f.join(n), 0))
            .collect()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansSymbols-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansMath-Regular.ttf",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        ]
        .into_iter()
        .map(|p| face(p, 0))
        .collect()
    }
}

const MAX_CACHED_GLYPHS: usize = 2048;

fn load(f: &Face) -> Option<FontVec> {
    let bytes = std::fs::read(&f.path).ok()?;
    FontVec::try_from_vec_and_index(bytes, f.index).ok()
}

fn variant_path(regular: &Path, style: &str) -> Option<PathBuf> {
    let s = regular.to_string_lossy();
    // DejaVu uses -Bold / -Oblique / -BoldOblique; Noto and Liberation use -Bold / -Italic / -BoldItalic.
    let alt = style.replace("Oblique", "Italic");
    for (a, b) in
        [("-Regular", format!("-{style}")), ("-Regular", format!("-{alt}")), (".ttf", format!("-{style}.ttf"))]
    {
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
    face: Face,
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
                metrics: Metrics {
                    xmin: b.min.x.floor() as i32,
                    ymin: -(b.min.y.floor() as i32) - height as i32,
                    width,
                    height,
                    advance_width,
                },
                coverage,
            }
        }
        None => {
            Glyph { metrics: Metrics { xmin: 0, ymin: 0, width: 0, height: 0, advance_width }, coverage: Vec::new() }
        }
    }
}

fn has(font: &FontVec, c: char) -> bool {
    font.glyph_id(c).0 != 0
}

impl Fonts {
    pub fn new(custom: Option<&Path>, px: f32, ui_px: f32) -> Result<Fonts, String> {
        let (regular, bold, italic, bold_italic) = match custom {
            Some(p) => {
                let regular = load(&face(p, 0)).ok_or_else(|| format!("cannot load font {}", p.display()))?;
                let variant = |style: &str| variant_path(p, style).and_then(|v| load(&face(v, 0)));
                (regular, variant("Bold"), variant("Oblique"), variant("BoldOblique"))
            }
            None => mono_families()
                .into_iter()
                .find_map(|Family(r, b, i, bi)| {
                    let regular = load(&r)?;
                    let opt = |f: Option<Face>| f.and_then(|f| load(&f));
                    Some((regular, opt(b), opt(i), opt(bi)))
                })
                .ok_or("no monospace font found; set `font` in config.toml")?,
        };
        let ui = ui_candidates().iter().find_map(load);
        let fallbacks = fallback_candidates().into_iter().map(|face| Fallback { face, font: None }).collect();
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
            let font = fb.font.get_or_insert_with(|| load(&fb.face));
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
