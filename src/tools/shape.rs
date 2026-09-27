//! Line/curve, rectangle and ellipse. Shapes stay editable (handles, move,
//! option changes) until applied, and are rendered live into the layer.

use egui::{Pos2, Ui, pos2, vec2};

use super::{Btn, Ctx, OverlayCx, ShapeFill, Tool, drag_rect, line_style_combo, norm_rect, snap_angle};
use crate::doc::{EditSession, Paint, paint_pixel};
use crate::raster::{IRect, cubic_points, ellipse_points, rounded_rect_points};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    Line,
    Rect,
    Ellipse,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Grab {
    /// Line: 0/1 endpoints, 2/3 curve controls. Box: 0..3 corners, 4..7 edges.
    Handle(usize),
    Move,
    Create,
}

pub struct ShapeTool {
    kind: ShapeKind,
    session: Option<EditSession>,
    /// Line: p0, p1, c1, c2. Box shapes: two opposite corners in [0], [1].
    pts: [Pos2; 4],
    curve: bool,
    grab: Option<Grab>,
    grab_pos: Pos2,
    start: Pos2,
    last_rect: IRect,
    swap_colors: bool,
}

impl ShapeTool {
    pub fn new(kind: ShapeKind) -> Self {
        Self {
            kind,
            session: None,
            pts: [Pos2::ZERO; 4],
            curve: false,
            grab: None,
            grab_pos: Pos2::ZERO,
            start: Pos2::ZERO,
            last_rect: IRect::EMPTY,
            swap_colors: false,
        }
    }

    fn box_handles(&self) -> [Pos2; 8] {
        let (a, b) = norm_rect(self.pts[0], self.pts[1]);
        let m = a + (b - a) / 2.0;
        [
            a,
            pos2(b.x, a.y),
            b,
            pos2(a.x, b.y),
            pos2(m.x, a.y),
            pos2(b.x, m.y),
            pos2(m.x, b.y),
            pos2(a.x, m.y),
        ]
    }

    fn line_points(&self) -> Vec<Pos2> {
        if self.curve {
            cubic_points(self.pts[0], self.pts[2], self.pts[3], self.pts[1])
        } else {
            vec![self.pts[0], self.pts[1]]
        }
    }

    fn render(&mut self, cx: &mut Ctx) {
        if self.session.is_none() {
            return;
        }
        let line_pts = self.line_points();
        cx.cov.clear(self.last_rect);
        cx.cov2.clear(self.last_rect);
        let s = &*cx.s;
        let (mut c_primary, mut c_secondary) = (s.primary, s.secondary);
        if self.swap_colors {
            std::mem::swap(&mut c_primary, &mut c_secondary);
        }
        let aa = s.antialias;
        let mut rect = IRect::EMPTY;
        let (outline, fill) = match (self.kind, s.shape_fill) {
            (ShapeKind::Line, _) => (true, false),
            (_, ShapeFill::Outline) => (true, false),
            (_, ShapeFill::Fill) => (false, true),
            (_, ShapeFill::OutlineFill) => (true, true),
        };
        let fill_color = if outline { c_secondary } else { c_primary };
        match self.kind {
            ShapeKind::Line => {
                rect = cx.cov.polyline(&line_pts, false, s.line_width, s.line_style, aa);
            }
            ShapeKind::Rect | ShapeKind::Ellipse => {
                let (a, b) = norm_rect(self.pts[0], self.pts[1]);
                let poly = if self.kind == ShapeKind::Rect {
                    rounded_rect_points(a, b, s.corner_radius)
                } else {
                    let c = a + (b - a) / 2.0;
                    ellipse_points(c, (b.x - a.x) / 2.0, (b.y - a.y) / 2.0)
                };
                if fill {
                    rect = rect.union(&cx.cov2.fill_polygon(std::slice::from_ref(&poly), aa));
                }
                if outline {
                    rect = rect.union(&cx.cov.polyline(&poly, true, s.line_width, s.line_style, aa));
                }
            }
        }
        let area = rect.union(&self.last_rect);
        self.last_rect = rect;
        let lock = cx.doc.active_layer().alpha_lock;
        let stroke_paint = Paint::Color { color: c_primary, opacity: 1.0 };
        let fill_paint = Paint::Color { color: fill_color, opacity: 1.0 };
        let (cov, cov2) = (&*cx.cov, &*cx.cov2);
        let Some(sess) = &mut self.session else { return };
        sess.apply(cx.doc, area, |x, y, src, sel| {
            let f = cov2.get(x, y) * sel;
            let st = cov.get(x, y) * sel;
            if f <= 0.0 && st <= 0.0 {
                return src;
            }
            let a = paint_pixel(src, f, fill_paint, lock);
            paint_pixel(a, st, stroke_paint, lock)
        });
    }

    fn apply_pending(&mut self, cx: &mut Ctx) {
        if let Some(s) = self.session.take() {
            let label = match self.kind {
                ShapeKind::Line => "Line",
                ShapeKind::Rect => "Rectangle",
                ShapeKind::Ellipse => "Ellipse",
            };
            s.commit(cx.doc, cx.history, label);
        }
        cx.cov.clear(self.last_rect);
        cx.cov2.clear(self.last_rect);
        self.last_rect = IRect::EMPTY;
        self.grab = None;
    }

    fn hit(&self, cx: &Ctx, p: Pos2) -> Option<Grab> {
        match self.kind {
            ShapeKind::Line => {
                let n = if self.curve { 4 } else { 2 };
                if let Some(i) = (0..n).find(|&i| cx.near(p, self.pts[i])) {
                    return Some(Grab::Handle(i));
                }
                let pts = self.line_points();
                let tol = cx.screen_dist(8.0) + cx.s.line_width / 2.0;
                let near_line = pts.windows(2).any(|w| dist_to_segment(p, w[0], w[1]) <= tol);
                near_line.then_some(Grab::Move)
            }
            _ => {
                let hs = self.box_handles();
                if let Some(i) = (0..8).find(|&i| cx.near(p, hs[i])) {
                    return Some(Grab::Handle(i));
                }
                let (a, b) = norm_rect(self.pts[0], self.pts[1]);
                egui::Rect::from_min_max(a, b).expand(cx.screen_dist(4.0)).contains(p).then_some(Grab::Move)
            }
        }
    }
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).length()
}

impl Tool for ShapeTool {
    fn press(&mut self, cx: &mut Ctx, p: Pos2, btn: Btn) {
        if self.session.is_some() {
            if let Some(g) = self.hit(cx, p) {
                self.grab = Some(g);
                self.grab_pos = p;
                return;
            }
            self.apply_pending(cx);
        }
        cx.ensure_buffers();
        match cx.doc.begin_edit() {
            Ok(s) => self.session = Some(s),
            Err(e) => return cx.warn(e),
        }
        self.swap_colors = btn == Btn::Secondary;
        self.curve = self.kind == ShapeKind::Line && cx.s.curve;
        self.start = p;
        self.pts = [p; 4];
        self.grab = Some(Grab::Create);
        self.last_rect = IRect::EMPTY;
    }

    fn drag(&mut self, cx: &mut Ctx, p: Pos2) {
        let Some(g) = self.grab else { return };
        let shift = cx.mods.shift;
        match (self.kind, g) {
            (ShapeKind::Line, Grab::Create) => {
                self.pts[1] = if shift { snap_angle(self.pts[0], p) } else { p };
                let d = self.pts[1] - self.pts[0];
                self.pts[2] = self.pts[0] + d / 3.0;
                self.pts[3] = self.pts[0] + d * 2.0 / 3.0;
            }
            (ShapeKind::Line, Grab::Handle(i)) => {
                let anchor = match i {
                    0 => self.pts[1],
                    1 => self.pts[0],
                    2 => self.pts[0],
                    _ => self.pts[1],
                };
                self.pts[i] = if shift { snap_angle(anchor, p) } else { p };
            }
            (_, Grab::Create) => {
                let (a, b) = drag_rect(self.start, p, shift, cx.mods.alt);
                self.pts[0] = a;
                self.pts[1] = b;
            }
            (_, Grab::Handle(i)) => {
                let (mut a, mut b) = norm_rect(self.pts[0], self.pts[1]);
                match i {
                    0 => a = p,
                    1 => {
                        b.x = p.x;
                        a.y = p.y;
                    }
                    2 => b = p,
                    3 => {
                        a.x = p.x;
                        b.y = p.y;
                    }
                    4 => a.y = p.y,
                    5 => b.x = p.x,
                    6 => b.y = p.y,
                    _ => a.x = p.x,
                }
                if shift && i < 4 {
                    // Keep square: size from the dragged corner.
                    let s = (b.x - a.x).abs().max((b.y - a.y).abs());
                    match i {
                        0 => a = b - vec2(s, s),
                        1 => {
                            b.x = a.x + s;
                            a.y = b.y - s;
                        }
                        2 => b = a + vec2(s, s),
                        _ => {
                            a.x = b.x - s;
                            b.y = a.y + s;
                        }
                    }
                }
                self.pts[0] = a;
                self.pts[1] = b;
            }
            (_, Grab::Move) => {
                let d = p - self.grab_pos;
                for q in &mut self.pts {
                    *q += d;
                }
                self.grab_pos = p;
            }
        }
        self.render(cx);
    }

    fn release(&mut self, cx: &mut Ctx, _p: Pos2) {
        if self.grab == Some(Grab::Create) {
            let d = self.pts[1] - self.pts[0];
            let tiny = match self.kind {
                ShapeKind::Line => d.length() < 1.0,
                _ => d.x.abs() < 1.0 || d.y.abs() < 1.0,
            };
            if tiny {
                self.cancel(cx);
            }
        }
        self.grab = None;
    }

    fn key(&mut self, cx: &mut Ctx, key: egui::Key) -> bool {
        if self.session.is_none() {
            return false;
        }
        match key {
            egui::Key::Enter => self.apply_pending(cx),
            egui::Key::Escape => self.cancel(cx),
            _ => return false,
        }
        true
    }

    fn commit(&mut self, cx: &mut Ctx) {
        self.apply_pending(cx);
    }

    fn cancel(&mut self, cx: &mut Ctx) {
        if let Some(s) = self.session.take() {
            s.cancel(cx.doc);
        }
        cx.cov.clear(self.last_rect);
        cx.cov2.clear(self.last_rect);
        self.last_rect = IRect::EMPTY;
        self.grab = None;
    }

    fn busy(&self) -> bool {
        self.session.is_some()
    }

    fn overlay(&self, o: &OverlayCx) {
        if self.session.is_none() {
            return;
        }
        match self.kind {
            ShapeKind::Line => {
                if self.curve {
                    o.guide_line(self.pts[0], self.pts[2]);
                    o.guide_line(self.pts[1], self.pts[3]);
                    o.round_handle(self.pts[2], egui::Color32::from_rgb(60, 140, 255));
                    o.round_handle(self.pts[3], egui::Color32::from_rgb(60, 140, 255));
                }
                o.handle(self.pts[0]);
                o.handle(self.pts[1]);
            }
            _ => {
                let (a, b) = norm_rect(self.pts[0], self.pts[1]);
                o.dashed_poly(&[a, pos2(b.x, a.y), b, pos2(a.x, b.y)], true);
                for h in self.box_handles() {
                    o.handle(h);
                }
            }
        }
    }

    fn cursor(&self, _hover: Option<Pos2>) -> egui::CursorIcon {
        egui::CursorIcon::Crosshair
    }

    fn options(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let s = &mut *cx.s;
        let mut changed = false;
        if self.kind != ShapeKind::Line {
            egui::ComboBox::from_id_salt("shape_fill")
                .width(110.0)
                .selected_text(match s.shape_fill {
                    ShapeFill::Outline => "Outline",
                    ShapeFill::Fill => "Fill",
                    ShapeFill::OutlineFill => "Outline + Fill",
                })
                .show_ui(ui, |ui| {
                    changed |= ui.selectable_value(&mut s.shape_fill, ShapeFill::Outline, "Outline").changed();
                    changed |= ui.selectable_value(&mut s.shape_fill, ShapeFill::Fill, "Fill").changed();
                    changed |= ui
                        .selectable_value(&mut s.shape_fill, ShapeFill::OutlineFill, "Outline + Fill")
                        .on_hover_text("Outline with primary, fill with secondary color")
                        .changed();
                });
        }
        ui.label("Width");
        changed |= ui.add(egui::DragValue::new(&mut s.line_width).range(1.0..=200.0).speed(0.2).suffix(" px")).changed();
        ui.label("Style");
        changed |= line_style_combo(ui, &mut s.line_style);
        if self.kind == ShapeKind::Rect {
            ui.label("Corner radius");
            changed |= ui.add(egui::DragValue::new(&mut s.corner_radius).range(0.0..=500.0).speed(0.5)).changed();
        }
        if self.kind == ShapeKind::Line
            && ui.checkbox(&mut s.curve, "Curve").on_hover_text("Bézier curve with two control handles").changed() {
                if self.session.is_some() {
                    self.curve = s.curve;
                    let d = self.pts[1] - self.pts[0];
                    self.pts[2] = self.pts[0] + d / 3.0;
                    self.pts[3] = self.pts[0] + d * 2.0 / 3.0;
                }
                changed = true;
            }
        changed |= ui.checkbox(&mut s.antialias, "Anti-alias").changed();
        if changed && self.session.is_some() {
            self.render(cx);
        }
    }
}
