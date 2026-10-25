//! Tool framework and shared tool settings.

mod fill;
mod paint;
mod picker;
mod select;
mod shape;
mod text;
mod transform;

use egui::{Color32, Key, Modifiers, Painter, Pos2, Stroke, Ui, pos2};
use serde::{Deserialize, Serialize};

use crate::doc::Document;
use crate::history::History;
use crate::raster::{Coverage, Image, LineStyle, Mask, Rgba, SelectOp, color_distance};
use crate::text::FontStore;
use crate::view::View;

pub use fill::{GradientKind, GradientRepeat};
pub use text::TextAlign;
pub use transform::TransformTool;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolId {
    Brush,
    Pencil,
    Eraser,
    Fill,
    Gradient,
    Line,
    Rect,
    Ellipse,
    Text,
    Picker,
    SelectRect,
    SelectEllipse,
    Lasso,
    MagicWand,
    Move,
    Pan,
    Zoom,
}

impl ToolId {
    pub const ALL: [ToolId; 17] = [
        ToolId::Move,
        ToolId::Pan,
        ToolId::SelectRect,
        ToolId::SelectEllipse,
        ToolId::Lasso,
        ToolId::MagicWand,
        ToolId::Brush,
        ToolId::Pencil,
        ToolId::Eraser,
        ToolId::Fill,
        ToolId::Gradient,
        ToolId::Picker,
        ToolId::Line,
        ToolId::Rect,
        ToolId::Ellipse,
        ToolId::Text,
        ToolId::Zoom,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ToolId::Brush => "Brush",
            ToolId::Pencil => "Pencil",
            ToolId::Eraser => "Eraser",
            ToolId::Fill => "Fill",
            ToolId::Gradient => "Gradient",
            ToolId::Line => "Line / Curve",
            ToolId::Rect => "Rectangle",
            ToolId::Ellipse => "Ellipse",
            ToolId::Text => "Text",
            ToolId::Picker => "Color Picker",
            ToolId::SelectRect => "Rectangle Select",
            ToolId::SelectEllipse => "Ellipse Select",
            ToolId::Lasso => "Lasso / Polygon Select",
            ToolId::MagicWand => "Magic Wand",
            ToolId::Move => "Move / Transform",
            ToolId::Pan => "Pan",
            ToolId::Zoom => "Zoom",
        }
    }

    pub fn shortcut(self) -> Option<Key> {
        Some(match self {
            ToolId::Brush => Key::B,
            ToolId::Pencil => Key::N,
            ToolId::Eraser => Key::E,
            ToolId::Fill => Key::F,
            ToolId::Gradient => Key::G,
            ToolId::Line => Key::L,
            ToolId::Rect => Key::R,
            ToolId::Ellipse => Key::O,
            ToolId::Text => Key::T,
            ToolId::Picker => Key::I,
            ToolId::SelectRect => Key::M,
            ToolId::SelectEllipse => Key::J,
            ToolId::Lasso => Key::K,
            ToolId::MagicWand => Key::W,
            ToolId::Move => Key::V,
            ToolId::Pan => Key::H,
            ToolId::Zoom => Key::Z,
        })
    }

    pub fn hint(self) -> &'static str {
        match self {
            ToolId::Brush | ToolId::Pencil => {
                "Right-click: secondary color · Shift-click: straight line · Alt: pick color · [ ]: size"
            }
            ToolId::Eraser => "Shift-click: straight line · [ ]: size",
            ToolId::Fill => "Right-click fills with the secondary color",
            ToolId::Gradient => "Drag to place, then adjust the handles · Enter applies · Esc cancels",
            ToolId::Line => "Drag to draw · drag handles to edit · Shift snaps to 15° · Enter applies",
            ToolId::Rect | ToolId::Ellipse => "Shift: square/circle · Alt: from center · Enter applies · Esc cancels",
            ToolId::Text => "Click to place text, then type · drag the box to move · Ctrl/Cmd+Enter or Esc applies",
            ToolId::Picker => "Left: primary · Right: secondary",
            ToolId::SelectRect | ToolId::SelectEllipse => {
                "Shift: add · Alt: subtract · Shift+Alt: intersect · click to deselect"
            }
            ToolId::Lasso => "Drag for freehand · click for polygon points, Enter/double-click closes",
            ToolId::MagicWand => "Shift: add · Alt: subtract · Shift+Alt: intersect",
            ToolId::Move => "Drag to move · handles scale/rotate · Shift keeps ratio · arrows nudge · Enter applies",
            ToolId::Pan => "Drag to pan (Space or middle mouse works with any tool)",
            ToolId::Zoom => "Click: zoom in · Alt-click: zoom out",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeFill {
    Outline,
    Fill,
    OutlineFill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Symmetry {
    Off,
    Vertical,
    Horizontal,
    Both,
}

/// User-tweakable tool options (persisted between sessions).
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolSettings {
    pub primary: Rgba,
    pub secondary: Rgba,
    pub brush_size: f32,
    pub brush_hardness: f32,
    pub brush_opacity: f32,
    pub brush_flow: f32,
    pub brush_spacing: f32,
    pub smoothing: f32,
    pub symmetry: Symmetry,
    pub pencil_size: i32,
    pub pencil_round: bool,
    pub pencil_overwrite: bool,
    pub eraser_size: f32,
    pub eraser_hardness: f32,
    pub eraser_strength: f32,
    pub line_width: f32,
    pub line_style: LineStyle,
    pub antialias: bool,
    pub shape_fill: ShapeFill,
    pub corner_radius: f32,
    pub curve: bool,
    pub fill_tolerance: f32,
    pub fill_contiguous: bool,
    pub fill_sample_all: bool,
    pub gradient_kind: GradientKind,
    pub gradient_repeat: GradientRepeat,
    pub gradient_to_transparent: bool,
    pub wand_tolerance: f32,
    pub wand_contiguous: bool,
    pub wand_sample_all: bool,
    pub select_op: SelectOp,
    pub text_size: f32,
    pub text_font: usize,
    pub text_align: TextAlign,
    pub text_line_spacing: f32,
    pub smooth_transform: bool,
    pub picker_sample_all: bool,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            primary: [0, 0, 0, 255],
            secondary: [255, 255, 255, 255],
            brush_size: 24.0,
            brush_hardness: 0.6,
            brush_opacity: 1.0,
            brush_flow: 1.0,
            brush_spacing: 0.15,
            smoothing: 0.3,
            symmetry: Symmetry::Off,
            pencil_size: 1,
            pencil_round: true,
            pencil_overwrite: false,
            eraser_size: 32.0,
            eraser_hardness: 0.8,
            eraser_strength: 1.0,
            line_width: 4.0,
            line_style: LineStyle::Solid,
            antialias: true,
            shape_fill: ShapeFill::Outline,
            corner_radius: 0.0,
            curve: false,
            fill_tolerance: 0.1,
            fill_contiguous: true,
            fill_sample_all: false,
            gradient_kind: GradientKind::Linear,
            gradient_repeat: GradientRepeat::None,
            gradient_to_transparent: false,
            wand_tolerance: 0.15,
            wand_contiguous: true,
            wand_sample_all: false,
            select_op: SelectOp::Replace,
            text_size: 48.0,
            text_font: 0,
            text_align: TextAlign::Left,
            text_line_spacing: 1.2,
            smooth_transform: true,
            picker_sample_all: true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Btn {
    Primary,
    Secondary,
}

/// Everything a tool may touch while handling input.
pub struct Ctx<'a> {
    pub doc: &'a mut Document,
    pub history: &'a mut History,
    pub s: &'a mut ToolSettings,
    pub cov: &'a mut Coverage,
    pub cov2: &'a mut Coverage,
    pub fonts: &'a FontStore,
    pub zoom: f32,
    pub mods: Modifiers,
    pub status: &'a mut Option<String>,
}

impl Ctx<'_> {
    /// Canvas-space distance that corresponds to `px` screen points.
    pub fn screen_dist(&self, px: f32) -> f32 {
        px / self.zoom
    }

    pub fn near(&self, a: Pos2, b: Pos2) -> bool {
        (a - b).length() * self.zoom <= 9.0
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        *self.status = Some(msg.into());
    }

    /// Selection op implied by modifier keys (falls back to the toolbar choice).
    pub fn select_op(&self) -> SelectOp {
        let alt = self.mods.alt;
        let shift = self.mods.shift;
        match (shift, alt) {
            (true, true) => SelectOp::Intersect,
            (true, false) => SelectOp::Add,
            (false, true) => SelectOp::Subtract,
            _ => self.s.select_op,
        }
    }

    pub fn ensure_buffers(&mut self) {
        self.cov.ensure_size(self.doc.w, self.doc.h);
        self.cov2.ensure_size(self.doc.w, self.doc.h);
    }
}

pub struct OverlayCx<'a> {
    pub view: &'a View,
    pub painter: &'a Painter,
    pub hover: Option<Pos2>,
    pub s: &'a ToolSettings,
    pub doc: &'a Document,
    pub fonts: &'a FontStore,
    pub time: f64,
}

impl OverlayCx<'_> {
    pub fn handle(&self, p: Pos2) {
        let s = self.view.to_screen(p);
        let r = egui::Rect::from_center_size(s, egui::vec2(8.0, 8.0));
        self.painter.rect_filled(r, 1.0, Color32::WHITE);
        self.painter.rect_stroke(r, 1.0, Stroke::new(1.0, Color32::from_gray(30)), egui::StrokeKind::Middle);
    }

    pub fn round_handle(&self, p: Pos2, color: Color32) {
        // White ring plus a dark outer ring so the handle shows on any
        // background, even when its fill matches the pixels underneath.
        let s = self.view.to_screen(p);
        self.painter.circle(s, 6.0, color, Stroke::new(1.5, Color32::WHITE));
        self.painter.circle_stroke(s, 7.5, Stroke::new(1.0, Color32::from_black_alpha(200)));
    }

    /// A line visible on any background.
    pub fn guide_line(&self, a: Pos2, b: Pos2) {
        let (a, b) = (self.view.to_screen(a), self.view.to_screen(b));
        self.painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(120)));
        self.painter.line_segment([a, b], Stroke::new(1.0, Color32::WHITE));
    }

    pub fn dashed_poly(&self, pts: &[Pos2], closed: bool) {
        let mut s: Vec<Pos2> = pts.iter().map(|p| self.view.to_screen(*p)).collect();
        if closed && s.len() > 2 {
            s.push(s[0]);
        }
        if s.len() < 2 {
            return;
        }
        self.painter.add(egui::Shape::line(s.clone(), Stroke::new(1.0, Color32::BLACK)));
        let off = (self.time * 12.0) as f32 % 8.0;
        self.painter.extend(egui::Shape::dashed_line_with_offset(
            &s,
            Stroke::new(1.0, Color32::WHITE),
            &[4.0],
            &[4.0],
            off,
        ));
    }

    pub fn brush_cursor(&self, radius: f32) {
        if let Some(h) = self.hover {
            let c = self.view.to_screen(h);
            let r = (radius * self.view.zoom).max(1.5);
            self.painter.circle_stroke(c, r, Stroke::new(2.5, Color32::from_black_alpha(140)));
            self.painter.circle_stroke(c, r, Stroke::new(1.0, Color32::WHITE));
            if r < 4.0 {
                self.painter.line_segment([c - egui::vec2(6.0, 0.0), c + egui::vec2(6.0, 0.0)], Stroke::new(1.0, Color32::WHITE));
                self.painter.line_segment([c - egui::vec2(0.0, 6.0), c + egui::vec2(0.0, 6.0)], Stroke::new(1.0, Color32::WHITE));
            }
        }
    }
}

pub trait Tool {
    fn press(&mut self, _cx: &mut Ctx, _p: Pos2, _btn: Btn) {}
    fn drag(&mut self, _cx: &mut Ctx, _p: Pos2) {}
    fn release(&mut self, _cx: &mut Ctx, _p: Pos2) {}
    fn double_click(&mut self, _cx: &mut Ctx, _p: Pos2) {}
    fn overlay(&self, _o: &OverlayCx) {}
    fn options(&mut self, _ui: &mut Ui, _cx: &mut Ctx) {}
    /// Returns true if the key was consumed.
    fn key(&mut self, _cx: &mut Ctx, _key: Key) -> bool {
        false
    }
    /// Finish any pending (still editable) operation.
    fn commit(&mut self, _cx: &mut Ctx) {}
    /// Abort any pending operation.
    fn cancel(&mut self, _cx: &mut Ctx) {}
    /// True while there is something pending to commit/cancel.
    fn busy(&self) -> bool {
        false
    }
    fn cursor(&self, _cx_hover: Option<Pos2>) -> egui::CursorIcon {
        egui::CursorIcon::Crosshair
    }
}

/// All tool instances. Tools keep their own state between activations.
pub struct Tools {
    pub brush: paint::PaintTool,
    pub pencil: paint::PaintTool,
    pub eraser: paint::PaintTool,
    pub fill: fill::FillTool,
    pub gradient: fill::GradientTool,
    pub line: shape::ShapeTool,
    pub rect: shape::ShapeTool,
    pub ellipse: shape::ShapeTool,
    pub text: text::TextTool,
    pub picker: picker::PickerTool,
    pub select_rect: select::SelectTool,
    pub select_ellipse: select::SelectTool,
    pub lasso: select::LassoTool,
    pub wand: select::WandTool,
    pub transform: TransformTool,
    pub none: NoTool,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            brush: paint::PaintTool::new(paint::PaintKind::Brush),
            pencil: paint::PaintTool::new(paint::PaintKind::Pencil),
            eraser: paint::PaintTool::new(paint::PaintKind::Eraser),
            fill: fill::FillTool,
            gradient: fill::GradientTool::default(),
            line: shape::ShapeTool::new(shape::ShapeKind::Line),
            rect: shape::ShapeTool::new(shape::ShapeKind::Rect),
            ellipse: shape::ShapeTool::new(shape::ShapeKind::Ellipse),
            text: text::TextTool::default(),
            picker: picker::PickerTool,
            select_rect: select::SelectTool::new(false),
            select_ellipse: select::SelectTool::new(true),
            lasso: select::LassoTool::default(),
            wand: select::WandTool,
            transform: TransformTool::default(),
            none: NoTool,
        }
    }
}

impl Tools {
    pub fn get(&mut self, id: ToolId) -> &mut dyn Tool {
        match id {
            ToolId::Brush => &mut self.brush,
            ToolId::Pencil => &mut self.pencil,
            ToolId::Eraser => &mut self.eraser,
            ToolId::Fill => &mut self.fill,
            ToolId::Gradient => &mut self.gradient,
            ToolId::Line => &mut self.line,
            ToolId::Rect => &mut self.rect,
            ToolId::Ellipse => &mut self.ellipse,
            ToolId::Text => &mut self.text,
            ToolId::Picker => &mut self.picker,
            ToolId::SelectRect => &mut self.select_rect,
            ToolId::SelectEllipse => &mut self.select_ellipse,
            ToolId::Lasso => &mut self.lasso,
            ToolId::MagicWand => &mut self.wand,
            ToolId::Move => &mut self.transform,
            ToolId::Pan | ToolId::Zoom => &mut self.none,
        }
    }

    pub fn get_ref(&self, id: ToolId) -> &dyn Tool {
        match id {
            ToolId::Brush => &self.brush,
            ToolId::Pencil => &self.pencil,
            ToolId::Eraser => &self.eraser,
            ToolId::Fill => &self.fill,
            ToolId::Gradient => &self.gradient,
            ToolId::Line => &self.line,
            ToolId::Rect => &self.rect,
            ToolId::Ellipse => &self.ellipse,
            ToolId::Text => &self.text,
            ToolId::Picker => &self.picker,
            ToolId::SelectRect => &self.select_rect,
            ToolId::SelectEllipse => &self.select_ellipse,
            ToolId::Lasso => &self.lasso,
            ToolId::MagicWand => &self.wand,
            ToolId::Move => &self.transform,
            ToolId::Pan | ToolId::Zoom => &self.none,
        }
    }
}

pub struct NoTool;
impl Tool for NoTool {
    fn cursor(&self, _: Option<Pos2>) -> egui::CursorIcon {
        egui::CursorIcon::Grab
    }
}

// ---------------------------------------------------------------------------
// Shared helpers

/// Region of similar color around (x, y) — or everywhere, if not contiguous.
pub fn flood_region(img: &Image, x: i32, y: i32, tolerance: f32, contiguous: bool) -> Mask {
    let mut mask = Mask::new(img.w, img.h);
    if x < 0 || y < 0 || x >= img.w as i32 || y >= img.h as i32 {
        return mask;
    }
    let target = img.get(x, y);
    let matches = |c: Rgba| color_distance(c, target) <= tolerance + 1e-4;
    if !contiguous {
        for (m, c) in mask.data.iter_mut().zip(&img.px) {
            if matches(*c) {
                *m = 255;
            }
        }
        return mask;
    }
    let w = img.w as i32;
    let mut stack = vec![(x, y)];
    while let Some((sx, sy)) = stack.pop() {
        let row = sy as usize * w as usize;
        if mask.data[row + sx as usize] != 0 {
            continue;
        }
        // Scanline: extend left and right.
        let mut l = sx;
        while l > 0 && mask.data[row + (l - 1) as usize] == 0 && matches(img.px[row + (l - 1) as usize]) {
            l -= 1;
        }
        let mut r = sx;
        while r + 1 < w && mask.data[row + (r + 1) as usize] == 0 && matches(img.px[row + (r + 1) as usize]) {
            r += 1;
        }
        for i in l..=r {
            mask.data[row + i as usize] = 255;
        }
        for ny in [sy - 1, sy + 1] {
            if ny < 0 || ny >= img.h as i32 {
                continue;
            }
            let nrow = ny as usize * w as usize;
            let mut i = l;
            while i <= r {
                if mask.data[nrow + i as usize] == 0 && matches(img.px[nrow + i as usize]) {
                    stack.push((i, ny));
                    // Skip the rest of this run.
                    while i <= r && matches(img.px[nrow + i as usize]) {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
        }
    }
    mask
}

/// Image used for sampling: the active layer or the flattened composite.
pub fn sample_source(doc: &Document, all_layers: bool) -> std::borrow::Cow<'_, Image> {
    if all_layers {
        std::borrow::Cow::Owned(doc.flatten())
    } else {
        std::borrow::Cow::Borrowed(&doc.active_layer().img)
    }
}

pub fn to_color32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// Snap the vector a→b to multiples of 15°.
pub fn snap_angle(a: Pos2, b: Pos2) -> Pos2 {
    let d = b - a;
    let len = d.length();
    let ang = d.y.atan2(d.x);
    let step = 15f32.to_radians();
    let s = (ang / step).round() * step;
    pos2(a.x + len * s.cos(), a.y + len * s.sin())
}

/// Rect from drag points with Shift (square) and Alt (from center) modifiers.
pub fn drag_rect(start: Pos2, p: Pos2, square: bool, centered: bool) -> (Pos2, Pos2) {
    let mut d = p - start;
    if square {
        let m = d.x.abs().max(d.y.abs());
        d = egui::vec2(m * d.x.signum(), m * d.y.signum());
    }
    if centered {
        (start - d, start + d)
    } else {
        (start, start + d)
    }
}

pub fn norm_rect(a: Pos2, b: Pos2) -> (Pos2, Pos2) {
    (a.min(b), a.max(b))
}

pub fn size_slider(ui: &mut Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Response {
    ui.label(label);
    ui.add(egui::DragValue::new(v).range(range).speed(0.5).suffix(" px"))
}

pub fn percent(ui: &mut Ui, label: &str, v: &mut f32) -> egui::Response {
    ui.label(label);
    let mut p = (*v * 100.0).round();
    let r = ui.add(egui::DragValue::new(&mut p).range(0.0..=100.0).speed(1.0).suffix("%"));
    if r.changed() {
        *v = p / 100.0;
    }
    r
}

pub fn line_style_combo(ui: &mut Ui, s: &mut LineStyle) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt("line_style")
        .width(80.0)
        .selected_text(format!("{s:?}"))
        .show_ui(ui, |ui| {
            for v in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted] {
                changed |= ui.selectable_value(s, v, format!("{v:?}")).changed();
            }
        });
    changed
}

pub fn select_op_buttons(ui: &mut Ui, op: &mut SelectOp) {
    for (v, label, tip) in [
        (SelectOp::Replace, "New", "Replace selection"),
        (SelectOp::Add, "Add", "Add to selection (Shift)"),
        (SelectOp::Subtract, "Sub", "Subtract from selection (Alt)"),
        (SelectOp::Intersect, "Int", "Intersect with selection (Shift+Alt)"),
    ] {
        ui.selectable_value(op, v, label).on_hover_text(tip);
    }
}

/// "Clear selection" button for the selection tools' options bar.
pub fn clear_selection_button(ui: &mut Ui, cx: &mut Ctx) {
    let has = cx.doc.selection.is_some();
    if ui
        .add_enabled(has, egui::Button::new("✖ Clear selection"))
        .on_hover_text("Ctrl/Cmd+D or Esc")
        .clicked()
    {
        cx.doc.set_selection(cx.history, "Clear selection", None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    struct Env {
        doc: Document,
        history: History,
        s: ToolSettings,
        cov: Coverage,
        cov2: Coverage,
        fonts: FontStore,
        status: Option<String>,
    }

    impl Env {
        fn new(w: u32, h: u32) -> Self {
            Self {
                doc: Document::new(w, h, Some([255, 255, 255, 255])),
                history: History::default(),
                s: ToolSettings::default(),
                cov: Coverage::new(w, h),
                cov2: Coverage::new(w, h),
                fonts: FontStore::default(),
                status: None,
            }
        }

        fn cx(&mut self) -> Ctx<'_> {
            Ctx {
                doc: &mut self.doc,
                history: &mut self.history,
                s: &mut self.s,
                cov: &mut self.cov,
                cov2: &mut self.cov2,
                fonts: &self.fonts,
                zoom: 1.0,
                mods: Modifiers::NONE,
                status: &mut self.status,
            }
        }
    }

    fn select_rect(env: &mut Env, x0: i32, y0: i32, x1: i32, y1: i32) {
        let mut m = Mask::new(env.doc.w, env.doc.h);
        for y in y0..y1 {
            for x in x0..x1 {
                m.data[(y as u32 * env.doc.w + x as u32) as usize] = 255;
            }
        }
        env.doc.selection = Some(m);
    }

    #[test]
    fn flood_fill_region() {
        let mut img = Image::filled(5, 5, [255, 255, 255, 255]);
        for y in 0..5 {
            img.set(2, y, [0, 0, 0, 255]);
        }
        let m = flood_region(&img, 0, 0, 0.1, true);
        assert_eq!(m.data.iter().filter(|&&v| v == 255).count(), 10);
        let all = flood_region(&img, 0, 0, 0.1, false);
        assert_eq!(all.data.iter().filter(|&&v| v == 255).count(), 20);
    }

    #[test]
    fn brush_is_clipped_to_selection() {
        let mut env = Env::new(40, 20);
        env.s.primary = [255, 0, 0, 255];
        env.s.smoothing = 0.0;
        select_rect(&mut env, 0, 0, 20, 20);
        let mut brush = paint::PaintTool::new(paint::PaintKind::Brush);
        let mut cx = env.cx();
        brush.press(&mut cx, pos2(5.0, 10.0), Btn::Primary);
        brush.drag(&mut cx, pos2(35.0, 10.0));
        brush.release(&mut cx, pos2(35.0, 10.0));
        let img = &env.doc.layers[0].img;
        assert_eq!(img.get(10, 10), [255, 0, 0, 255]);
        assert_eq!(img.get(30, 10), [255, 255, 255, 255], "painted outside the selection");
        assert!(env.history.can_undo());
        // The coverage buffer must be clean for the next tool.
        assert!(env.cov.data.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn transform_moves_pixels_in_one_undo_step() {
        let mut env = Env::new(20, 10);
        env.doc.layers[0].img_mut().set(2, 2, [0, 0, 255, 255]);
        select_rect(&mut env, 0, 0, 5, 5);
        let steps_before = env.history.labels().0.len();
        let mut t = TransformTool::default();
        {
            let mut cx = env.cx();
            assert!(t.lift(&mut cx));
            for _ in 0..10 {
                t.key(&mut cx, Key::ArrowRight);
            }
            t.commit(&mut cx);
        }
        let img = &env.doc.layers[0].img;
        assert_eq!(img.get(12, 2), [0, 0, 255, 255]);
        // The old spot is now a hole.
        assert_eq!(img.get(2, 2)[3], 0);
        // Selection moved along.
        assert!(env.doc.selection.as_ref().unwrap().get(12, 2) == 255);
        assert_eq!(env.history.labels().0.len(), steps_before + 1);
        env.history.undo(&mut env.doc);
        assert_eq!(env.doc.layers[0].img.get(2, 2), [0, 0, 255, 255]);
        assert!(env.doc.selection.as_ref().unwrap().get(2, 2) == 255);
    }

    #[test]
    fn soft_selection_erase_is_partial() {
        let mut env = Env::new(4, 4);
        let mut m = Mask::new(4, 4);
        m.data.fill(128);
        env.doc.selection = Some(m);
        let mut e = paint::PaintTool::new(paint::PaintKind::Eraser);
        env.s.eraser_hardness = 1.0;
        env.s.eraser_size = 20.0;
        let mut cx = env.cx();
        e.press(&mut cx, pos2(2.0, 2.0), Btn::Primary);
        e.release(&mut cx, pos2(2.0, 2.0));
        let a = env.doc.layers[0].img.get(1, 1)[3];
        assert!((120..=135).contains(&a), "alpha {a}");
    }

    #[test]
    fn shape_cancel_restores_layer() {
        let mut env = Env::new(30, 30);
        let before = env.doc.layers[0].img.clone();
        let mut r = shape::ShapeTool::new(shape::ShapeKind::Rect);
        let mut cx = env.cx();
        r.press(&mut cx, pos2(5.0, 5.0), Btn::Primary);
        r.drag(&mut cx, pos2(25.0, 25.0));
        r.release(&mut cx, pos2(25.0, 25.0));
        assert!(r.busy());
        r.cancel(&mut cx);
        assert!(*env.doc.layers[0].img == *before);
        assert!(!env.history.can_undo());
    }
}
