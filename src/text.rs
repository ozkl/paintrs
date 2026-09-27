//! Font management and text rasterization into coverage.

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use egui::{Pos2, pos2};

use crate::raster::{Coverage, IRect};
use crate::tools::TextAlign;

pub struct FontStore {
    pub fonts: Vec<(String, FontArc)>,
}

impl Default for FontStore {
    fn default() -> Self {
        // Reuse the fonts egui already ships with, so there's always something.
        let defs = egui::FontDefinitions::default();
        let mut fonts = Vec::new();
        let mut names: Vec<&String> = defs.font_data.keys().collect();
        // Proportional font first so it is the default.
        names.sort_by_key(|n| (!n.starts_with("Ubuntu"), n.to_string()));
        for name in names {
            let data = &defs.font_data[name];
            if name.to_lowercase().contains("emoji") {
                continue;
            }
            if let Ok(f) = FontArc::try_from_vec(data.font.to_vec()) {
                fonts.push((name.replace('-', " "), f));
            }
        }
        Self { fonts }
    }
}

impl FontStore {
    pub fn add(&mut self, name: &str, bytes: Vec<u8>) -> Result<usize, String> {
        let f = FontArc::try_from_vec(bytes).map_err(|e| format!("Invalid font: {e}"))?;
        let name = name.rsplit_once('.').map_or(name, |(n, _)| n).to_string();
        self.fonts.push((name, f));
        Ok(self.fonts.len() - 1)
    }

    pub fn get(&self, i: usize) -> Option<&FontArc> {
        self.fonts.get(i).or_else(|| self.fonts.first()).map(|(_, f)| f)
    }
}

pub struct TextLayout {
    /// Bounds of the text block in canvas space (top-left = anchor for Left).
    pub min: Pos2,
    pub max: Pos2,
}

/// Measure a block of text anchored at `anchor` (top-left/center/right of first line).
pub fn layout(font: &FontArc, text: &str, size: f32, spacing: f32, align: TextAlign, anchor: Pos2) -> TextLayout {
    let sf = font.as_scaled(PxScale::from(size));
    let line_h = (sf.ascent() - sf.descent() + sf.line_gap()) * spacing;
    let widths: Vec<f32> = text.split('\n').map(|l| line_width(&sf, l)).collect();
    let w = widths.iter().copied().fold(0.0, f32::max);
    let h = line_h * widths.len().max(1) as f32;
    let x0 = match align {
        TextAlign::Left => anchor.x,
        TextAlign::Center => anchor.x - w / 2.0,
        TextAlign::Right => anchor.x - w,
    };
    TextLayout { min: pos2(x0, anchor.y), max: pos2(x0 + w, anchor.y + h) }
}

fn line_width<F: Font, SF: ScaleFont<F>>(sf: &SF, line: &str) -> f32 {
    let mut w = 0.0;
    let mut prev = None;
    for ch in line.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            w += sf.kern(p, id);
        }
        w += sf.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Rasterize text into coverage; returns touched rect.
pub fn rasterize(
    cov: &mut Coverage,
    font: &FontArc,
    text: &str,
    size: f32,
    spacing: f32,
    align: TextAlign,
    anchor: Pos2,
) -> IRect {
    let sf = font.as_scaled(PxScale::from(size));
    let line_h = (sf.ascent() - sf.descent() + sf.line_gap()) * spacing;
    let mut rect = IRect::EMPTY;
    let (cw, ch) = (cov.w as i32, cov.h as i32);
    for (i, line) in text.split('\n').enumerate() {
        let lw = line_width(&sf, line);
        let mut x = match align {
            TextAlign::Left => anchor.x,
            TextAlign::Center => anchor.x - lw / 2.0,
            TextAlign::Right => anchor.x - lw,
        };
        let baseline = anchor.y + sf.ascent() + line_h * i as f32;
        let mut prev = None;
        for c in line.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            let glyph = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(x, baseline));
            x += sf.h_advance(id);
            prev = Some(id);
            if let Some(outlined) = font.outline_glyph(glyph) {
                let b = outlined.px_bounds();
                let (ox, oy) = (b.min.x as i32, b.min.y as i32);
                outlined.draw(|gx, gy, v| {
                    let px = ox + gx as i32;
                    let py = oy + gy as i32;
                    if px >= 0 && py >= 0 && px < cw && py < ch && v > 0.0 {
                        let idx = py as usize * cov.w as usize + px as usize;
                        cov.data[idx] = cov.data[idx].max(v.min(1.0));
                    }
                });
                rect = rect.union(&IRect::new(ox, oy, b.max.x.ceil() as i32 + 1, b.max.y.ceil() as i32 + 1));
            }
        }
    }
    rect.clamp_to(cov.w, cov.h)
}
