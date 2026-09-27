//! Bucket fill and the gradient tool.

use egui::{Pos2, Ui};
use serde::{Deserialize, Serialize};

use super::{Btn, Ctx, OverlayCx, Tool, flood_region, percent, sample_source, snap_angle};
use crate::doc::{EditSession, Paint};
use crate::raster::{Rgba, blend_color};

pub struct FillTool;

impl Tool for FillTool {
    fn press(&mut self, cx: &mut Ctx, p: Pos2, btn: Btn) {
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        if !cx.doc.bounds().contains(x, y) {
            return;
        }
        cx.ensure_buffers();
        let mut session = match cx.doc.begin_edit() {
            Ok(s) => s,
            Err(e) => return cx.warn(e),
        };
        let region = {
            let src = sample_source(cx.doc, cx.s.fill_sample_all);
            flood_region(&src, x, y, cx.s.fill_tolerance, cx.s.fill_contiguous)
        };
        let rect = cx.cov.fill_mask(&region);
        let color = if btn == Btn::Primary { cx.s.primary } else { cx.s.secondary };
        session.apply_coverage(cx.doc, cx.cov, rect, Paint::Replace { color });
        session.commit(cx.doc, cx.history, "Fill");
        cx.cov.clear(rect);
    }

    fn options(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        percent(ui, "Tolerance", &mut cx.s.fill_tolerance);
        ui.checkbox(&mut cx.s.fill_contiguous, "Contiguous");
        ui.checkbox(&mut cx.s.fill_sample_all, "Sample all layers");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GradientKind {
    Linear,
    Radial,
    Conic,
    Diamond,
    Bilinear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GradientRepeat {
    None,
    Repeat,
    Mirror,
}

#[derive(Default)]
pub struct GradientTool {
    session: Option<EditSession>,
    start: Pos2,
    end: Pos2,
    /// 0 = start, 1 = end, 2 = both.
    dragging: Option<u8>,
    grab: Pos2,
}

impl GradientTool {
    fn render(&mut self, cx: &mut Ctx) {
        let Some(sess) = &mut self.session else { return };
        let (a, b) = (self.start, self.end);
        let c0 = cx.s.primary;
        let c1 = if cx.s.gradient_to_transparent {
            [c0[0], c0[1], c0[2], 0]
        } else {
            cx.s.secondary
        };
        let kind = cx.s.gradient_kind;
        let repeat = cx.s.gradient_repeat;
        let d = b - a;
        let len2 = d.length_sq().max(1e-6);
        let len = len2.sqrt();
        let bounds = cx.doc.selection.as_ref().and_then(|m| m.bounds()).unwrap_or(cx.doc.bounds());
        sess.apply(cx.doc, bounds, |x, y, src, sel| {
            if sel <= 0.0 {
                return src;
            }
            let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
            let v = p - a;
            let t = match kind {
                GradientKind::Linear => v.dot(d) / len2,
                GradientKind::Bilinear => (v.dot(d) / len2).abs(),
                GradientKind::Radial => v.length() / len,
                GradientKind::Diamond => {
                    // Rotate into the gradient's frame.
                    let n = d / len;
                    let u = v.dot(n).abs();
                    let w = (v.x * -n.y + v.y * n.x).abs();
                    (u + w) / len
                }
                GradientKind::Conic => {
                    let ang = v.y.atan2(v.x) - d.y.atan2(d.x);
                    (ang / std::f32::consts::TAU).rem_euclid(1.0)
                }
            };
            let t = match repeat {
                GradientRepeat::None => t.clamp(0.0, 1.0),
                GradientRepeat::Repeat => t.rem_euclid(1.0),
                GradientRepeat::Mirror => {
                    let m = t.rem_euclid(2.0);
                    if m > 1.0 { 2.0 - m } else { m }
                }
            };
            let c = lerp_color(c0, c1, t);
            let alpha = c[3] * sel / 255.0;
            blend_color(src, [c[0], c[1], c[2]], alpha)
        });
    }

    fn apply_pending(&mut self, cx: &mut Ctx) {
        if let Some(s) = self.session.take() {
            s.commit(cx.doc, cx.history, "Gradient");
        }
        self.dragging = None;
    }
}

/// Interpolate in premultiplied space so fades to transparent stay clean.
fn lerp_color(a: Rgba, b: Rgba, t: f32) -> [f32; 4] {
    let (aa, ba) = (a[3] as f32, b[3] as f32);
    let alpha = aa + (ba - aa) * t;
    if alpha <= 0.0 {
        return [0.0; 4];
    }
    let ch = |i: usize| (a[i] as f32 * aa * (1.0 - t) + b[i] as f32 * ba * t) / alpha;
    [ch(0), ch(1), ch(2), alpha]
}

impl Tool for GradientTool {
    fn press(&mut self, cx: &mut Ctx, p: Pos2, _btn: Btn) {
        if self.session.is_some() {
            if cx.near(p, self.start) {
                self.dragging = Some(0);
                return;
            }
            if cx.near(p, self.end) {
                self.dragging = Some(1);
                return;
            }
            let mid = self.start + (self.end - self.start) / 2.0;
            if cx.near(p, mid) {
                self.dragging = Some(2);
                self.grab = p;
                return;
            }
            self.apply_pending(cx);
        }
        match cx.doc.begin_edit() {
            Ok(s) => self.session = Some(s),
            Err(e) => return cx.warn(e),
        }
        self.start = p;
        self.end = p;
        self.dragging = Some(1);
    }

    fn drag(&mut self, cx: &mut Ctx, p: Pos2) {
        let Some(which) = self.dragging else { return };
        match which {
            0 => self.start = if cx.mods.shift { snap_angle(self.end, p) } else { p },
            1 => self.end = if cx.mods.shift { snap_angle(self.start, p) } else { p },
            _ => {
                let d = p - self.grab;
                self.start += d;
                self.end += d;
                self.grab = p;
            }
        }
        if (self.end - self.start).length() > 0.5 {
            self.render(cx);
        }
    }

    fn release(&mut self, cx: &mut Ctx, _p: Pos2) {
        self.dragging = None;
        if (self.end - self.start).length() <= 0.5 {
            // A click without drag does nothing.
            self.cancel(cx);
        }
    }

    fn key(&mut self, cx: &mut Ctx, key: egui::Key) -> bool {
        match key {
            egui::Key::Enter if self.session.is_some() => {
                self.apply_pending(cx);
                true
            }
            egui::Key::Escape if self.session.is_some() => {
                self.cancel(cx);
                true
            }
            _ => false,
        }
    }

    fn commit(&mut self, cx: &mut Ctx) {
        self.apply_pending(cx);
    }

    fn cancel(&mut self, cx: &mut Ctx) {
        if let Some(s) = self.session.take() {
            s.cancel(cx.doc);
        }
        self.dragging = None;
    }

    fn busy(&self) -> bool {
        self.session.is_some()
    }

    fn overlay(&self, o: &OverlayCx) {
        if self.session.is_some() {
            o.guide_line(self.start, self.end);
            o.round_handle(self.start, super::to_color32(o.s.primary));
            o.round_handle(self.end, super::to_color32(o.s.secondary));
            o.handle(self.start + (self.end - self.start) / 2.0);
        }
    }

    fn options(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let mut changed = false;
        egui::ComboBox::from_id_salt("grad_kind")
            .width(90.0)
            .selected_text(format!("{:?}", cx.s.gradient_kind))
            .show_ui(ui, |ui| {
                for k in [
                    GradientKind::Linear,
                    GradientKind::Bilinear,
                    GradientKind::Radial,
                    GradientKind::Diamond,
                    GradientKind::Conic,
                ] {
                    changed |= ui.selectable_value(&mut cx.s.gradient_kind, k, format!("{k:?}")).changed();
                }
            });
        ui.label("Repeat");
        egui::ComboBox::from_id_salt("grad_rep")
            .width(80.0)
            .selected_text(format!("{:?}", cx.s.gradient_repeat))
            .show_ui(ui, |ui| {
                for k in [GradientRepeat::None, GradientRepeat::Repeat, GradientRepeat::Mirror] {
                    changed |= ui.selectable_value(&mut cx.s.gradient_repeat, k, format!("{k:?}")).changed();
                }
            });
        changed |= ui.checkbox(&mut cx.s.gradient_to_transparent, "Primary → transparent").changed();
        if ui.button("⬌ Reverse").clicked() {
            std::mem::swap(&mut self.start, &mut self.end);
            changed = true;
        }
        if changed && self.session.is_some() {
            self.render(cx);
        }
    }
}
