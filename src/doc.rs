//! Document model: layers, selection, compositing and pixel edit sessions.

use std::sync::Arc;

use crate::history::{Edit, History};
use crate::raster::{Coverage, IRect, Image, Mask, Rgba, blend_color, lerp_rgba, union_opt};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Add,
}

impl BlendMode {
    pub const ALL: [BlendMode; 12] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Add,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Overlay => "Overlay",
            BlendMode::Darken => "Darken",
            BlendMode::Lighten => "Lighten",
            BlendMode::ColorDodge => "Color Dodge",
            BlendMode::ColorBurn => "Color Burn",
            BlendMode::HardLight => "Hard Light",
            BlendMode::SoftLight => "Soft Light",
            BlendMode::Difference => "Difference",
            BlendMode::Add => "Add",
        }
    }

    #[inline]
    fn apply(self, b: f32, s: f32) -> f32 {
        match self {
            BlendMode::Normal => s,
            BlendMode::Multiply => b * s,
            BlendMode::Screen => b + s - b * s,
            BlendMode::Overlay => BlendMode::HardLight.apply(s, b),
            BlendMode::Darken => b.min(s),
            BlendMode::Lighten => b.max(s),
            BlendMode::ColorDodge => {
                if b == 0.0 {
                    0.0
                } else if s >= 1.0 {
                    1.0
                } else {
                    (b / (1.0 - s)).min(1.0)
                }
            }
            BlendMode::ColorBurn => {
                if b >= 1.0 {
                    1.0
                } else if s <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - b) / s).min(1.0)
                }
            }
            BlendMode::HardLight => {
                if s <= 0.5 {
                    b * 2.0 * s
                } else {
                    let s2 = 2.0 * s - 1.0;
                    b + s2 - b * s2
                }
            }
            BlendMode::SoftLight => {
                if s <= 0.5 {
                    b - (1.0 - 2.0 * s) * b * (1.0 - b)
                } else {
                    let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
                    b + (2.0 * s - 1.0) * (d - b)
                }
            }
            BlendMode::Difference => (b - s).abs(),
            BlendMode::Add => (b + s).min(1.0),
        }
    }
}

#[derive(Clone)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    /// Copy-on-write so history snapshots are cheap.
    pub img: Arc<Image>,
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    /// Locked layers can't be painted on.
    pub locked: bool,
    /// Paint only affects existing pixels' color.
    pub alpha_lock: bool,
    /// Bumped on every pixel change (drives thumbnail refresh).
    pub rev: u64,
}

impl Layer {
    pub fn img_mut(&mut self) -> &mut Image {
        self.rev += 1;
        Arc::make_mut(&mut self.img)
    }
}

/// Everything that a structural undo step needs to restore.
#[derive(Clone)]
pub struct DocSnapshot {
    pub w: u32,
    pub h: u32,
    pub layers: Vec<Layer>,
    pub active: usize,
    pub selection: Option<Mask>,
}

impl DocSnapshot {
    pub fn byte_size(&self) -> usize {
        self.layers.iter().map(|l| l.img.byte_size()).sum::<usize>()
            + self.selection.as_ref().map_or(0, |m| m.data.len())
    }
}

pub struct Document {
    pub w: u32,
    pub h: u32,
    pub layers: Vec<Layer>,
    pub active: usize,
    pub selection: Option<Mask>,
    /// Region of the composite that needs refreshing.
    pub dirty: Option<IRect>,
    /// Selection outline must be recomputed.
    pub selection_changed: bool,
    pub next_id: u64,
    pub name: String,
    /// Native path of the project file, if saved as project.
    pub path: Option<std::path::PathBuf>,
    pub modified: bool,
}

impl Document {
    pub fn new(w: u32, h: u32, background: Option<Rgba>) -> Self {
        let mut d = Document {
            w,
            h,
            layers: Vec::new(),
            active: 0,
            selection: None,
            dirty: Some(IRect::from_size(w, h)),
            selection_changed: true,
            next_id: 1,
            name: "Untitled".into(),
            path: None,
            modified: false,
        };
        let img = match background {
            Some(c) => Image::filled(w, h, c),
            None => Image::new(w, h),
        };
        let l = d.make_layer(if background.is_some() { "Background" } else { "Layer 1" }, img);
        d.layers.push(l);
        d
    }

    pub fn from_layers(w: u32, h: u32, layers: Vec<(String, Image)>) -> Self {
        let mut d = Document::new(w, h, None);
        d.layers.clear();
        for (name, img) in layers {
            let l = d.make_layer(&name, img);
            d.layers.push(l);
        }
        if d.layers.is_empty() {
            let l = d.make_layer("Layer 1", Image::new(w, h));
            d.layers.push(l);
        }
        d.active = d.layers.len() - 1;
        d
    }

    pub fn make_layer(&mut self, name: &str, img: Image) -> Layer {
        let id = self.next_id;
        self.next_id += 1;
        Layer {
            id,
            name: name.to_string(),
            img: Arc::new(img),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            locked: false,
            alpha_lock: false,
            rev: 0,
        }
    }

    pub fn unique_layer_name(&self) -> String {
        let mut n = self.layers.len() + 1;
        loop {
            let name = format!("Layer {n}");
            if !self.layers.iter().any(|l| l.name == name) {
                return name;
            }
            n += 1;
        }
    }

    pub fn bounds(&self) -> IRect {
        IRect::from_size(self.w, self.h)
    }

    pub fn active_layer(&self) -> &Layer {
        &self.layers[self.active]
    }

    pub fn layer_index(&self, id: u64) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    pub fn mark_dirty(&mut self, r: IRect) {
        self.dirty = union_opt(self.dirty, r.clamp_to(self.w, self.h));
    }

    pub fn mark_all_dirty(&mut self) {
        self.dirty = Some(self.bounds());
        self.selection_changed = true;
    }

    pub fn snapshot(&self) -> DocSnapshot {
        DocSnapshot {
            w: self.w,
            h: self.h,
            layers: self.layers.clone(),
            active: self.active,
            selection: self.selection.clone(),
        }
    }

    pub fn restore(&mut self, s: DocSnapshot) {
        self.w = s.w;
        self.h = s.h;
        self.layers = s.layers;
        self.active = s.active.min(self.layers.len().saturating_sub(1));
        self.selection = s.selection;
        for l in &mut self.layers {
            l.rev += 1;
        }
        self.mark_all_dirty();
    }

    /// Run a structural change and record it as one undo step.
    pub fn structural(&mut self, history: &mut History, label: &str, f: impl FnOnce(&mut Document)) {
        let before = self.snapshot();
        f(self);
        let after = self.snapshot();
        history.push(label, Edit::Doc { before: Box::new(before), after: Box::new(after) });
        self.modified = true;
        self.mark_all_dirty();
    }

    /// Structural change merged with the previous one of the same label.
    pub fn structural_coalesce(&mut self, history: &mut History, label: &str, f: impl FnOnce(&mut Document)) {
        let before = self.snapshot();
        f(self);
        let after = self.snapshot();
        history.push_coalesce(label, Edit::Doc { before: Box::new(before), after: Box::new(after) });
        self.modified = true;
        self.mark_all_dirty();
    }

    pub fn set_selection(&mut self, history: &mut History, label: &str, sel: Option<Mask>) {
        let sel = sel.filter(|m| !m.is_empty());
        let before = self.selection.clone();
        if before == sel {
            return;
        }
        self.selection = sel.clone();
        self.selection_changed = true;
        history.push(label, Edit::Selection { before, after: sel });
    }

    // -----------------------------------------------------------------------
    // Compositing

    /// Composite visible layers into premultiplied egui colors for a rect.
    pub fn composite_rect(&self, r: IRect, out: &mut Vec<egui::Color32>) {
        out.clear();
        out.reserve(r.area());
        let layers: Vec<&Layer> = self.layers.iter().filter(|l| l.visible && l.opacity > 0.0).collect();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let c = composite_pixel(&layers, x, y);
                out.push(egui::Color32::from_rgba_premultiplied(
                    (c[0] * c[3] * 255.0 + 0.5) as u8,
                    (c[1] * c[3] * 255.0 + 0.5) as u8,
                    (c[2] * c[3] * 255.0 + 0.5) as u8,
                    (c[3] * 255.0 + 0.5) as u8,
                ));
            }
        }
    }

    /// Flatten visible layers into a straight-alpha image.
    pub fn flatten(&self) -> Image {
        let layers: Vec<&Layer> = self.layers.iter().collect();
        composite_layers(&layers, self.w, self.h)
    }

    pub fn composite_pixel(&self, x: i32, y: i32) -> Rgba {
        let layers: Vec<&Layer> = self.layers.iter().filter(|l| l.visible && l.opacity > 0.0).collect();
        let c = composite_pixel(&layers, x, y);
        [
            (c[0] * 255.0 + 0.5) as u8,
            (c[1] * 255.0 + 0.5) as u8,
            (c[2] * 255.0 + 0.5) as u8,
            (c[3] * 255.0 + 0.5) as u8,
        ]
    }

    // -----------------------------------------------------------------------
    // Pixel editing

    pub fn begin_edit(&mut self) -> Result<EditSession, String> {
        let layer = &self.layers[self.active];
        if layer.locked {
            return Err(format!("Layer \"{}\" is locked", layer.name));
        }
        if !layer.visible {
            return Err(format!("Layer \"{}\" is hidden", layer.name));
        }
        Ok(EditSession { layer_id: layer.id, base: layer.img.clone(), work: None, dirty: None })
    }
}

/// Composite the visible ones of `layers` into a straight-alpha image.
pub fn composite_layers(layers: &[&Layer], w: u32, h: u32) -> Image {
    let layers: Vec<&Layer> = layers.iter().copied().filter(|l| l.visible && l.opacity > 0.0).collect();
    let mut out = Image::new(w, h);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let c = composite_pixel(&layers, x, y);
            let i = out.idx(x, y);
            out.px[i] = [
                (c[0] * 255.0 + 0.5) as u8,
                (c[1] * 255.0 + 0.5) as u8,
                (c[2] * 255.0 + 0.5) as u8,
                (c[3] * 255.0 + 0.5) as u8,
            ];
        }
    }
    out
}

/// Composite a single pixel, returns straight RGBA in 0..1.
#[inline]
fn composite_pixel(layers: &[&Layer], x: i32, y: i32) -> [f32; 4] {
    let (mut cr, mut cg, mut cb, mut ca) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for l in layers {
        let p = l.img.px[l.img.idx(x, y)];
        if p[3] == 0 {
            continue;
        }
        let sa = p[3] as f32 / 255.0 * l.opacity;
        let s = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0];
        let (sr, sg, sb) = if l.blend == BlendMode::Normal || ca == 0.0 {
            (s[0], s[1], s[2])
        } else {
            // Mix blend result with the source where the backdrop is transparent.
            let m = |b: f32, s: f32| (1.0 - ca) * s + ca * l.blend.apply(b, s);
            (m(cr, s[0]), m(cg, s[1]), m(cb, s[2]))
        };
        let oa = sa + ca * (1.0 - sa);
        let k = ca * (1.0 - sa);
        cr = (sr * sa + cr * k) / oa;
        cg = (sg * sa + cg * k) / oa;
        cb = (sb * sa + cb * k) / oa;
        ca = oa;
    }
    [cr, cg, cb, ca]
}

/// How coverage is turned into pixels.
#[derive(Clone, Copy, Debug)]
pub enum Paint {
    /// Blend a color over the layer.
    Color { color: Rgba, opacity: f32 },
    /// Replace pixels, including alpha (pencil "overwrite").
    Replace { color: Rgba },
    /// Remove alpha.
    Erase { strength: f32 },
}

/// Apply `paint` with coverage `c` (0..1) to one pixel.
#[inline]
pub fn paint_pixel(src: Rgba, c: f32, paint: Paint, alpha_lock: bool) -> Rgba {
    match paint {
        Paint::Color { color, opacity } => {
            let a = c * opacity * color[3] as f32 / 255.0;
            if alpha_lock {
                let t = lerp_rgba(src, [color[0], color[1], color[2], src[3]], a);
                [t[0], t[1], t[2], src[3]]
            } else {
                blend_color(src, [color[0] as f32, color[1] as f32, color[2] as f32], a)
            }
        }
        Paint::Replace { color } => {
            if alpha_lock {
                let t = lerp_rgba(src, [color[0], color[1], color[2], src[3]], c);
                [t[0], t[1], t[2], src[3]]
            } else if src[3] == 0 {
                [color[0], color[1], color[2], (color[3] as f32 * c).round() as u8]
            } else {
                lerp_rgba(src, color, c)
            }
        }
        Paint::Erase { strength } => {
            if alpha_lock {
                return src;
            }
            let a = src[3] as f32 * (1.0 - c * strength);
            [src[0], src[1], src[2], a.round() as u8]
        }
    }
}

/// A pending pixel edit on one layer. The layer is rendered live from
/// `base` (or `work`) + tool output, so it can be redrawn any number of
/// times, cancelled, or committed as a compact undo patch.
pub struct EditSession {
    pub layer_id: u64,
    /// The layer before the edit (for undo and cancel).
    pub base: Arc<Image>,
    /// Optional alternative source to redraw from (e.g. layer with a hole
    /// cut out for a floating selection).
    pub work: Option<Image>,
    pub dirty: Option<IRect>,
}

impl EditSession {
    pub fn source(&self) -> &Image {
        self.work.as_ref().unwrap_or(&self.base)
    }

    /// Recompute `rect` of the layer from the source through
    /// `f(x, y, src_px, selection_factor) -> px`.
    pub fn apply(&mut self, doc: &mut Document, rect: IRect, mut f: impl FnMut(i32, i32, Rgba, f32) -> Rgba) {
        let rect = rect.clamp_to(doc.w, doc.h);
        if rect.is_empty() {
            return;
        }
        let Some(li) = doc.layer_index(self.layer_id) else { return };
        let src = self.work.as_ref().unwrap_or(&self.base);
        let sel = doc.selection.as_ref();
        let layer = &mut doc.layers[li];
        layer.rev += 1;
        let img = Arc::make_mut(&mut layer.img);
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let i = img.idx(x, y);
                let s = sel.map_or(1.0, |m| m.data[i] as f32 / 255.0);
                img.px[i] = f(x, y, src.px[i], s);
            }
        }
        self.dirty = union_opt(self.dirty, rect);
        doc.mark_dirty(rect);
    }

    /// Render coverage into the layer over `rect` using `paint`, honoring
    /// selection and alpha lock.
    pub fn apply_coverage(&mut self, doc: &mut Document, cov: &Coverage, rect: IRect, paint: Paint) {
        let Some(li) = doc.layer_index(self.layer_id) else { return };
        let alpha_lock = doc.layers[li].alpha_lock;
        self.apply(doc, rect, |x, y, src, sel| {
            let c = cov.get(x, y).min(1.0) * sel;
            if c <= 0.0 {
                return src;
            }
            paint_pixel(src, c, paint, alpha_lock)
        });
    }

    /// Turn the session into an undo patch (None if nothing changed).
    pub fn finish(self, doc: &mut Document) -> Option<Edit> {
        let rect = self.dirty?;
        let li = doc.layer_index(self.layer_id)?;
        let before = self.base.read_rect(rect);
        let after = doc.layers[li].img.read_rect(rect);
        if before == after {
            return None;
        }
        doc.modified = true;
        Some(Edit::Pixels { layer_id: self.layer_id, rect, before, after })
    }

    pub fn commit(self, doc: &mut Document, history: &mut History, label: &str) -> bool {
        match self.finish(doc) {
            Some(e) => {
                history.push(label, e);
                true
            }
            None => false,
        }
    }

    pub fn cancel(self, doc: &mut Document) {
        if let (Some(rect), Some(li)) = (self.dirty, doc.layer_index(self.layer_id)) {
            let data = self.base.read_rect(rect);
            doc.layers[li].img_mut().write_rect(rect, &data);
            doc.mark_dirty(rect);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_normal_over() {
        let mut d = Document::new(1, 1, Some([0, 0, 0, 255]));
        let l = d.make_layer("top", Image::filled(1, 1, [255, 255, 255, 128]));
        d.layers.push(l);
        let c = d.composite_pixel(0, 0);
        assert_eq!(c[3], 255);
        assert!((c[0] as i32 - 128).abs() <= 1);
    }

    #[test]
    fn multiply_blend() {
        let mut d = Document::new(1, 1, Some([200, 200, 200, 255]));
        let mut l = d.make_layer("top", Image::filled(1, 1, [128, 128, 128, 255]));
        l.blend = BlendMode::Multiply;
        d.layers.push(l);
        let c = d.composite_pixel(0, 0);
        assert!((c[0] as i32 - 100).abs() <= 1, "{c:?}");
    }

    #[test]
    fn session_commit_and_undo() {
        let mut d = Document::new(4, 4, Some([255, 255, 255, 255]));
        let mut h = History::default();
        let mut cov = Coverage::new(4, 4);
        let r = cov.pixel_block(1, 1, 1, false);
        let mut s = d.begin_edit().unwrap();
        s.apply_coverage(&mut d, &cov, r, Paint::Color { color: [255, 0, 0, 255], opacity: 1.0 });
        assert!(s.commit(&mut d, &mut h, "dot"));
        assert_eq!(d.layers[0].img.get(1, 1), [255, 0, 0, 255]);
        h.undo(&mut d);
        assert_eq!(d.layers[0].img.get(1, 1), [255, 255, 255, 255]);
        h.redo(&mut d);
        assert_eq!(d.layers[0].img.get(1, 1), [255, 0, 0, 255]);
    }
}
