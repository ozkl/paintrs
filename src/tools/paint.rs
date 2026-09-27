//! Brush, pencil and eraser: a shared stroke engine with spacing,
//! smoothing (stabilizer), symmetry and shift-click straight lines.

use egui::{Pos2, Ui, pos2};

use super::{Btn, Ctx, OverlayCx, Symmetry, Tool, percent, size_slider};
use crate::doc::{EditSession, Paint};
use crate::raster::{IRect, Rgba};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PaintKind {
    Brush,
    Pencil,
    Eraser,
}

pub struct PaintTool {
    kind: PaintKind,
    session: Option<EditSession>,
    paint: Paint,
    /// Last dab position (after smoothing).
    last: Pos2,
    /// Smoothed pointer position.
    smooth: Pos2,
    /// Distance travelled since the last dab.
    carry: f32,
    /// End of the previous stroke, for shift-click lines.
    prev_end: Option<Pos2>,
    picking: bool,
}

impl PaintTool {
    pub fn new(kind: PaintKind) -> Self {
        Self {
            kind,
            session: None,
            paint: Paint::Erase { strength: 1.0 },
            last: Pos2::ZERO,
            smooth: Pos2::ZERO,
            carry: 0.0,
            prev_end: None,
            picking: false,
        }
    }

    fn radius(&self, cx: &Ctx) -> f32 {
        match self.kind {
            PaintKind::Brush => cx.s.brush_size / 2.0,
            PaintKind::Eraser => cx.s.eraser_size / 2.0,
            PaintKind::Pencil => cx.s.pencil_size as f32 / 2.0,
        }
    }

    fn spacing(&self, cx: &Ctx) -> f32 {
        match self.kind {
            PaintKind::Pencil => 1.0,
            _ => (self.radius(cx) * 2.0 * cx.s.brush_spacing).max(0.5),
        }
    }

    fn mirrored(&self, cx: &Ctx, p: Pos2) -> Vec<Pos2> {
        let (w, h) = (cx.doc.w as f32, cx.doc.h as f32);
        let mut v = vec![p];
        let sym = if self.kind == PaintKind::Pencil || self.kind == PaintKind::Brush || self.kind == PaintKind::Eraser {
            cx.s.symmetry
        } else {
            Symmetry::Off
        };
        if matches!(sym, Symmetry::Vertical | Symmetry::Both) {
            v.push(pos2(w - p.x, p.y));
        }
        if matches!(sym, Symmetry::Horizontal | Symmetry::Both) {
            v.push(pos2(p.x, h - p.y));
        }
        if sym == Symmetry::Both {
            v.push(pos2(w - p.x, h - p.y));
        }
        v
    }

    fn dab(&self, cx: &mut Ctx, p: Pos2) -> IRect {
        let mut r = IRect::EMPTY;
        for q in self.mirrored(cx, p) {
            let d = match self.kind {
                PaintKind::Brush => cx.cov.dab(q, cx.s.brush_size / 2.0, cx.s.brush_hardness, cx.s.brush_flow),
                PaintKind::Eraser => cx.cov.dab(q, cx.s.eraser_size / 2.0, cx.s.eraser_hardness, 1.0),
                PaintKind::Pencil => {
                    cx.cov.pixel_block(q.x.floor() as i32, q.y.floor() as i32, cx.s.pencil_size.max(1), cx.s.pencil_round)
                }
            };
            r = r.union(&d);
        }
        r
    }

    /// Lay dabs from `self.last` to `to`.
    fn stroke_to(&mut self, cx: &mut Ctx, to: Pos2) {
        let mut rect = IRect::EMPTY;
        if self.kind == PaintKind::Pencil {
            // Bresenham for gap-free pixel lines.
            let (mut x0, mut y0) = (self.last.x.floor() as i32, self.last.y.floor() as i32);
            let (x1, y1) = (to.x.floor() as i32, to.y.floor() as i32);
            let dx = (x1 - x0).abs();
            let dy = -(y1 - y0).abs();
            let sx = if x0 < x1 { 1 } else { -1 };
            let sy = if y0 < y1 { 1 } else { -1 };
            let mut err = dx + dy;
            loop {
                rect = rect.union(&self.dab(cx, pos2(x0 as f32 + 0.5, y0 as f32 + 0.5)));
                if x0 == x1 && y0 == y1 {
                    break;
                }
                let e2 = 2 * err;
                if e2 >= dy {
                    err += dy;
                    x0 += sx;
                }
                if e2 <= dx {
                    err += dx;
                    y0 += sy;
                }
            }
        } else {
            let spacing = self.spacing(cx);
            let d = to - self.last;
            let len = d.length();
            if len > 0.0 {
                let dir = d / len;
                let mut t = spacing - self.carry;
                while t <= len {
                    rect = rect.union(&self.dab(cx, self.last + dir * t));
                    t += spacing;
                }
                self.carry = len - (t - spacing);
            }
        }
        self.last = to;
        if let Some(sess) = &mut self.session {
            sess.apply_coverage(cx.doc, cx.cov, rect, self.paint);
        }
    }

    fn finish(&mut self, cx: &mut Ctx) {
        if let Some(sess) = self.session.take() {
            let dirty = sess.dirty;
            let label = match self.kind {
                PaintKind::Brush => "Brush stroke",
                PaintKind::Pencil => "Pencil",
                PaintKind::Eraser => "Erase",
            };
            sess.commit(cx.doc, cx.history, label);
            if let Some(d) = dirty {
                cx.cov.clear(d.expand(2));
            }
        }
    }
}

fn pick(cx: &mut Ctx, p: Pos2, btn: Btn) {
    let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
    if !cx.doc.bounds().contains(x, y) {
        return;
    }
    let c: Rgba = cx.doc.composite_pixel(x, y);
    match btn {
        Btn::Primary => cx.s.primary = c,
        Btn::Secondary => cx.s.secondary = c,
    }
}

impl Tool for PaintTool {
    fn press(&mut self, cx: &mut Ctx, p: Pos2, btn: Btn) {
        if cx.mods.alt && self.kind != PaintKind::Eraser {
            self.picking = true;
            pick(cx, p, btn);
            return;
        }
        cx.ensure_buffers();
        match cx.doc.begin_edit() {
            Ok(s) => self.session = Some(s),
            Err(e) => {
                cx.warn(e);
                return;
            }
        }
        let color = if btn == Btn::Primary { cx.s.primary } else { cx.s.secondary };
        self.paint = match self.kind {
            PaintKind::Brush => Paint::Color { color, opacity: cx.s.brush_opacity },
            PaintKind::Pencil if cx.s.pencil_overwrite => Paint::Replace { color },
            PaintKind::Pencil => Paint::Color { color, opacity: 1.0 },
            PaintKind::Eraser => Paint::Erase { strength: cx.s.eraser_strength },
        };
        self.carry = 0.0;
        match (cx.mods.shift, self.prev_end) {
            (true, Some(prev)) => {
                // Straight line from the end of the previous stroke.
                self.last = prev;
                let r = self.dab(cx, prev);
                if let Some(s) = &mut self.session {
                    s.apply_coverage(cx.doc, cx.cov, r, self.paint);
                }
                self.stroke_to(cx, p);
                self.smooth = p;
            }
            _ => {
                self.last = p;
                self.smooth = p;
                let r = self.dab(cx, p);
                if let Some(s) = &mut self.session {
                    s.apply_coverage(cx.doc, cx.cov, r, self.paint);
                }
            }
        }
    }

    fn drag(&mut self, cx: &mut Ctx, p: Pos2) {
        if self.picking {
            pick(cx, p, Btn::Primary);
            return;
        }
        if self.session.is_none() {
            return;
        }
        let k = if self.kind == PaintKind::Pencil { 0.0 } else { cx.s.smoothing.clamp(0.0, 0.95) };
        self.smooth = self.smooth + (p - self.smooth) * (1.0 - k);
        let to = self.smooth;
        self.stroke_to(cx, to);
    }

    fn release(&mut self, cx: &mut Ctx, p: Pos2) {
        if self.picking {
            self.picking = false;
            return;
        }
        if self.session.is_some() {
            // Catch up with the pointer so smoothing doesn't shorten strokes.
            if (p - self.last).length() > 0.5 && cx.s.smoothing > 0.0 {
                self.stroke_to(cx, p);
            }
            self.prev_end = Some(self.last);
            self.finish(cx);
        }
    }

    fn commit(&mut self, cx: &mut Ctx) {
        self.finish(cx);
    }

    fn cancel(&mut self, cx: &mut Ctx) {
        if let Some(s) = self.session.take() {
            if let Some(d) = s.dirty {
                cx.cov.clear(d.expand(2));
            }
            s.cancel(cx.doc);
        }
    }

    fn busy(&self) -> bool {
        false
    }

    fn overlay(&self, o: &OverlayCx) {
        let r = match self.kind {
            PaintKind::Brush => o.s.brush_size / 2.0,
            PaintKind::Eraser => o.s.eraser_size / 2.0,
            PaintKind::Pencil => o.s.pencil_size as f32 / 2.0,
        };
        if self.kind == PaintKind::Pencil && o.view.zoom >= 4.0 {
            // Show the exact pixel block that will be painted.
            if let Some(h) = o.hover {
                let size = o.s.pencil_size.max(1);
                let half = (size - 1) / 2;
                let x0 = h.x.floor() - half as f32;
                let y0 = h.y.floor() - half as f32;
                let r = egui::Rect::from_min_max(
                    o.view.to_screen(pos2(x0, y0)),
                    o.view.to_screen(pos2(x0 + size as f32, y0 + size as f32)),
                );
                o.painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, egui::Color32::WHITE), egui::StrokeKind::Outside);
                o.painter.rect_stroke(r.expand(1.0), 0.0, egui::Stroke::new(1.0, egui::Color32::BLACK), egui::StrokeKind::Outside);
            }
        } else {
            o.brush_cursor(r);
        }
        // Symmetry axes.
        let (w, h) = (o.doc.w as f32, o.doc.h as f32);
        if matches!(o.s.symmetry, Symmetry::Vertical | Symmetry::Both) {
            o.guide_line(pos2(w / 2.0, 0.0), pos2(w / 2.0, h));
        }
        if matches!(o.s.symmetry, Symmetry::Horizontal | Symmetry::Both) {
            o.guide_line(pos2(0.0, h / 2.0), pos2(w, h / 2.0));
        }
    }

    fn options(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let s = &mut *cx.s;
        match self.kind {
            PaintKind::Brush => {
                size_slider(ui, "Size", &mut s.brush_size, 1.0..=1000.0);
                percent(ui, "Hardness", &mut s.brush_hardness);
                percent(ui, "Opacity", &mut s.brush_opacity).on_hover_text("Maximum opacity of one stroke");
                percent(ui, "Flow", &mut s.brush_flow).on_hover_text("How quickly paint builds up within a stroke");
                percent(ui, "Spacing", &mut s.brush_spacing).on_hover_text("Distance between dabs, relative to size");
                s.brush_spacing = s.brush_spacing.max(0.02);
                percent(ui, "Smoothing", &mut s.smoothing).on_hover_text("Stabilizes shaky strokes");
            }
            PaintKind::Pencil => {
                ui.label("Size");
                ui.add(egui::DragValue::new(&mut s.pencil_size).range(1..=64).suffix(" px"));
                ui.checkbox(&mut s.pencil_round, "Round");
                ui.checkbox(&mut s.pencil_overwrite, "Overwrite")
                    .on_hover_text("Replace pixels including alpha instead of blending");
            }
            PaintKind::Eraser => {
                size_slider(ui, "Size", &mut s.eraser_size, 1.0..=1000.0);
                percent(ui, "Hardness", &mut s.eraser_hardness);
                percent(ui, "Strength", &mut s.eraser_strength);
                percent(ui, "Smoothing", &mut s.smoothing);
            }
        }
        ui.separator();
        ui.label("Symmetry");
        egui::ComboBox::from_id_salt("symmetry").width(90.0).selected_text(format!("{:?}", s.symmetry)).show_ui(ui, |ui| {
            for v in [Symmetry::Off, Symmetry::Vertical, Symmetry::Horizontal, Symmetry::Both] {
                ui.selectable_value(&mut s.symmetry, v, format!("{v:?}"));
            }
        });
    }
}
