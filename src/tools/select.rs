//! Rectangle / ellipse / lasso-polygon selection and magic wand.
//! All support replace, add (Shift), subtract (Alt) and intersect (Shift+Alt).

use egui::{Pos2, Ui, pos2};

use super::{Btn, Ctx, OverlayCx, Tool, drag_rect, flood_region, norm_rect, percent, sample_source, select_op_buttons};
use crate::raster::{Coverage, IRect, Mask, SelectOp, ellipse_points};

/// Combine freshly rasterized coverage with the current selection.
fn commit_selection(cx: &mut Ctx, rect: IRect, op: SelectOp, label: &str) {
    let (w, h) = (cx.doc.w, cx.doc.h);
    let mut m = match (&cx.doc.selection, op) {
        (Some(s), SelectOp::Add | SelectOp::Subtract | SelectOp::Intersect) => s.clone(),
        (None, SelectOp::Intersect) => Mask::new(w, h),
        _ => Mask::new(w, h),
    };
    m.combine(cx.cov, rect, op);
    cx.cov.clear(rect);
    cx.doc.set_selection(cx.history, label, Some(m));
}

fn deselect(cx: &mut Ctx) {
    cx.doc.set_selection(cx.history, "Deselect", None);
}

fn op_label(op: SelectOp) -> &'static str {
    match op {
        SelectOp::Replace => "Select",
        SelectOp::Add => "Add to selection",
        SelectOp::Subtract => "Subtract from selection",
        SelectOp::Intersect => "Intersect selection",
    }
}

pub struct SelectTool {
    ellipse: bool,
    dragging: bool,
    start: Pos2,
    a: Pos2,
    b: Pos2,
    op: SelectOp,
}

impl SelectTool {
    pub fn new(ellipse: bool) -> Self {
        Self { ellipse, dragging: false, start: Pos2::ZERO, a: Pos2::ZERO, b: Pos2::ZERO, op: SelectOp::Replace }
    }
}

impl Tool for SelectTool {
    fn press(&mut self, cx: &mut Ctx, p: Pos2, _btn: Btn) {
        cx.ensure_buffers();
        self.op = cx.select_op();
        // Selections snap to whole pixels.
        let p = pos2(p.x.round(), p.y.round());
        self.dragging = true;
        self.start = p;
        self.a = p;
        self.b = p;
    }

    fn drag(&mut self, cx: &mut Ctx, p: Pos2) {
        if !self.dragging {
            return;
        }
        let p = pos2(p.x.round(), p.y.round());
        // While dragging, Ctrl/Cmd constrains to square (Shift is taken by "add").
        let (a, b) = drag_rect(self.start, p, cx.mods.command, false);
        self.a = a;
        self.b = b;
    }

    fn release(&mut self, cx: &mut Ctx, _p: Pos2) {
        if !self.dragging {
            return;
        }
        self.dragging = false;
        let (a, b) = norm_rect(self.a, self.b);
        if b.x - a.x < 1.0 || b.y - a.y < 1.0 {
            if self.op == SelectOp::Replace {
                deselect(cx);
            }
            return;
        }
        let poly = if self.ellipse {
            let c = a + (b - a) / 2.0;
            ellipse_points(c, (b.x - a.x) / 2.0, (b.y - a.y) / 2.0)
        } else {
            vec![a, pos2(b.x, a.y), b, pos2(a.x, b.y)]
        };
        let rect = cx.cov.fill_polygon(&[poly], self.ellipse);
        commit_selection(cx, rect, self.op, op_label(self.op));
    }

    fn overlay(&self, o: &OverlayCx) {
        if !self.dragging {
            return;
        }
        let (a, b) = norm_rect(self.a, self.b);
        if self.ellipse {
            let c = a + (b - a) / 2.0;
            o.dashed_poly(&ellipse_points(c, (b.x - a.x) / 2.0, (b.y - a.y) / 2.0), true);
        } else {
            o.dashed_poly(&[a, pos2(b.x, a.y), b, pos2(a.x, b.y)], true);
        }
        let label = format!("{} × {}", (b.x - a.x) as i32, (b.y - a.y) as i32);
        o.painter.text(
            o.view.to_screen(b) + egui::vec2(8.0, 8.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE,
        );
    }

    fn options(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        select_op_buttons(ui, &mut cx.s.select_op);
        ui.label("(Ctrl/Cmd while dragging: square)");
    }
}

#[derive(Default)]
pub struct LassoTool {
    pts: Vec<Pos2>,
    /// Freehand drag in progress.
    freehand: bool,
    /// Polygon clicks in progress.
    polygon: bool,
    press_pos: Pos2,
    moved: bool,
    op: Option<SelectOp>,
}

impl LassoTool {
    fn finish(&mut self, cx: &mut Ctx) {
        let pts = std::mem::take(&mut self.pts);
        self.freehand = false;
        self.polygon = false;
        let op = self.op.take().unwrap_or(SelectOp::Replace);
        if pts.len() < 3 {
            return;
        }
        cx.ensure_buffers();
        let rect = cx.cov.fill_polygon(&[pts], true);
        commit_selection(cx, rect, op, op_label(op));
    }
}

impl Tool for LassoTool {
    fn press(&mut self, cx: &mut Ctx, p: Pos2, _btn: Btn) {
        self.press_pos = p;
        self.moved = false;
        if self.polygon {
            // Clicking near the first point closes the polygon.
            if self.pts.len() >= 3 && cx.near(p, self.pts[0]) {
                self.finish(cx);
            } else {
                self.pts.push(p);
            }
            return;
        }
        self.op = Some(cx.select_op());
        self.pts = vec![p];
        self.freehand = true;
    }

    fn drag(&mut self, cx: &mut Ctx, p: Pos2) {
        if !self.freehand {
            return;
        }
        if (p - self.press_pos).length() * cx.zoom > 3.0 {
            self.moved = true;
        }
        if self.pts.last().is_none_or(|l| (*l - p).length() * cx.zoom >= 2.0) {
            self.pts.push(p);
        }
    }

    fn release(&mut self, cx: &mut Ctx, p: Pos2) {
        if self.freehand {
            self.freehand = false;
            if self.moved {
                self.finish(cx);
            } else {
                // A click: start polygon mode.
                self.polygon = true;
                self.pts = vec![p];
            }
        }
    }

    fn double_click(&mut self, cx: &mut Ctx, _p: Pos2) {
        if self.polygon {
            self.finish(cx);
        }
    }

    fn key(&mut self, cx: &mut Ctx, key: egui::Key) -> bool {
        if !self.polygon {
            return false;
        }
        match key {
            egui::Key::Enter => self.finish(cx),
            egui::Key::Escape => self.cancel(cx),
            egui::Key::Backspace => {
                self.pts.pop();
                if self.pts.is_empty() {
                    self.polygon = false;
                }
            }
            _ => return false,
        }
        true
    }

    fn cancel(&mut self, _cx: &mut Ctx) {
        self.pts.clear();
        self.polygon = false;
        self.freehand = false;
    }

    fn commit(&mut self, cx: &mut Ctx) {
        if self.polygon {
            self.finish(cx);
        }
    }

    fn busy(&self) -> bool {
        self.polygon
    }

    fn overlay(&self, o: &OverlayCx) {
        if self.pts.is_empty() {
            return;
        }
        let mut pts = self.pts.clone();
        if self.polygon
            && let Some(h) = o.hover {
                pts.push(h);
            }
        o.dashed_poly(&pts, self.polygon);
        if self.polygon {
            o.handle(self.pts[0]);
        }
    }

    fn options(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        select_op_buttons(ui, &mut cx.s.select_op);
    }
}

pub struct WandTool;

impl Tool for WandTool {
    fn press(&mut self, cx: &mut Ctx, p: Pos2, _btn: Btn) {
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        if !cx.doc.bounds().contains(x, y) {
            return;
        }
        cx.ensure_buffers();
        let op = cx.select_op();
        let region = {
            let src = sample_source(cx.doc, cx.s.wand_sample_all);
            flood_region(&src, x, y, cx.s.wand_tolerance, cx.s.wand_contiguous)
        };
        let rect = fill_cov_from_mask(cx.cov, &region);
        commit_selection(cx, rect, op, "Magic wand");
    }

    fn options(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        select_op_buttons(ui, &mut cx.s.select_op);
        ui.separator();
        percent(ui, "Tolerance", &mut cx.s.wand_tolerance);
        ui.checkbox(&mut cx.s.wand_contiguous, "Contiguous");
        ui.checkbox(&mut cx.s.wand_sample_all, "Sample all layers");
    }
}

fn fill_cov_from_mask(cov: &mut Coverage, m: &Mask) -> IRect {
    cov.fill_mask(m)
}
