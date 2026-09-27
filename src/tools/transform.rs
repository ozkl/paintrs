//! Move / free transform. Lifts the selection (or the layer's content) into
//! a floating image that can be moved, scaled, rotated and flipped with live
//! preview, then applied as a single undo step (pixels + selection).

use egui::{Color32, Pos2, Stroke, Ui, Vec2, pos2, vec2};

use super::{Btn, Ctx, OverlayCx, Tool};
use crate::doc::EditSession;
use crate::history::Edit;
use crate::raster::{IRect, Image, Mask, over};

#[derive(Clone, Copy, PartialEq, Debug)]
struct Xf {
    t: Vec2,
    sx: f32,
    sy: f32,
    rot: f32,
}

impl Default for Xf {
    fn default() -> Self {
        Self { t: Vec2::ZERO, sx: 1.0, sy: 1.0, rot: 0.0 }
    }
}

struct Floating {
    session: EditSession,
    img: Image,
    mask: Option<Mask>,
    /// Top-left of the lifted pixels in canvas space.
    origin: Pos2,
    xf: Xf,
    last_rect: IRect,
    sel_before: Option<Mask>,
}

impl Floating {
    fn half(&self) -> Vec2 {
        vec2(self.img.w as f32, self.img.h as f32) / 2.0
    }

    fn center0(&self) -> Pos2 {
        self.origin + self.half()
    }

    fn center(&self) -> Pos2 {
        self.center0() + self.xf.t
    }

    /// Local (image) coordinates → canvas.
    fn map(&self, u: Pos2) -> Pos2 {
        let q = (u.to_vec2() - self.half()) * vec2(self.xf.sx, self.xf.sy);
        self.center() + rotate(q, self.xf.rot)
    }

    /// Canvas → local (image) coordinates.
    fn unmap(&self, p: Pos2) -> Pos2 {
        let q = rotate(p - self.center(), -self.xf.rot);
        (vec2(q.x / self.xf.sx, q.y / self.xf.sy) + self.half()).to_pos2()
    }

    fn corners(&self) -> [Pos2; 4] {
        let (w, h) = (self.img.w as f32, self.img.h as f32);
        [self.map(pos2(0.0, 0.0)), self.map(pos2(w, 0.0)), self.map(pos2(w, h)), self.map(pos2(0.0, h))]
    }

    /// 0..3 corners, 4..7 edge midpoints (top, right, bottom, left), 8 rotate.
    fn handles(&self, zoom: f32) -> [Pos2; 9] {
        let (w, h) = (self.img.w as f32, self.img.h as f32);
        let c = self.corners();
        let top = self.map(pos2(w / 2.0, 0.0));
        let up = rotate(vec2(0.0, -1.0), self.xf.rot) * self.xf.sy.signum();
        [
            c[0],
            c[1],
            c[2],
            c[3],
            top,
            self.map(pos2(w, h / 2.0)),
            self.map(pos2(w / 2.0, h)),
            self.map(pos2(0.0, h / 2.0)),
            top + up * (28.0 / zoom),
        ]
    }

    fn screen_bounds(&self) -> IRect {
        let c = self.corners();
        let min = c.iter().fold(pos2(f32::MAX, f32::MAX), |a, b| a.min(*b));
        let max = c.iter().fold(pos2(f32::MIN, f32::MIN), |a, b| a.max(*b));
        IRect::around(min, max).expand(1)
    }
}

fn rotate(v: Vec2, a: f32) -> Vec2 {
    let (s, c) = a.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

#[derive(Clone, Copy, PartialEq)]
enum Grab {
    Move,
    Handle(usize),
    Rotate,
}

#[derive(Default)]
pub struct TransformTool {
    float: Option<Floating>,
    grab: Option<Grab>,
    grab_pos: Pos2,
    xf0: Xf,
}

impl TransformTool {
    /// Lift the selection (or the whole layer content) into a floating image.
    pub fn lift(&mut self, cx: &mut Ctx) -> bool {
        if self.float.is_some() {
            return true;
        }
        let mut session = match cx.doc.begin_edit() {
            Ok(s) => s,
            Err(e) => {
                cx.warn(e);
                return false;
            }
        };
        let sel = cx.doc.selection.clone();
        let bounds = match &sel {
            Some(m) => m.bounds(),
            None => session.base.opaque_bounds(),
        };
        let Some(bounds) = bounds else {
            cx.warn("Nothing to move on this layer");
            return false;
        };
        let base = session.base.clone();
        let mut img = base.crop(bounds);
        let mut work = (*base).clone();
        let mask = sel.as_ref().map(|m| {
            let mut c = Mask::new(bounds.width() as u32, bounds.height() as u32);
            for y in bounds.y0..bounds.y1 {
                for x in bounds.x0..bounds.x1 {
                    c.data[((y - bounds.y0) * bounds.width() + (x - bounds.x0)) as usize] = m.get(x, y);
                }
            }
            c
        });
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                let k = mask.as_ref().map_or(1.0, |m| m.getf(x - bounds.x0, y - bounds.y0));
                let li = img.idx(x - bounds.x0, y - bounds.y0);
                img.px[li][3] = (img.px[li][3] as f32 * k).round() as u8;
                let wi = work.idx(x, y);
                work.px[wi][3] = (work.px[wi][3] as f32 * (1.0 - k)).round() as u8;
            }
        }
        session.work = Some(work);
        cx.doc.selection = None;
        cx.doc.selection_changed = true;
        let mut f = Floating {
            session,
            img,
            mask,
            origin: pos2(bounds.x0 as f32, bounds.y0 as f32),
            xf: Xf::default(),
            last_rect: IRect::EMPTY,
            sel_before: sel,
        };
        render(&mut f, cx);
        self.float = Some(f);
        true
    }

    fn apply_pending(&mut self, cx: &mut Ctx) {
        let Some(f) = self.float.take() else { return };
        self.grab = None;
        let unchanged = f.xf == Xf::default();
        let new_sel = if unchanged {
            f.sel_before.clone()
        } else {
            f.mask.as_ref().map(|m| transform_mask(&f, m, cx.doc.w, cx.doc.h, cx.s.smooth_transform))
        };
        let sel_before = f.sel_before.clone();
        let pixels = if unchanged {
            f.session.cancel(cx.doc);
            None
        } else {
            f.session.finish(cx.doc)
        };
        cx.doc.selection = new_sel.clone().filter(|m| !m.is_empty());
        cx.doc.selection_changed = true;
        if let Some(p) = pixels {
            let mut group = vec![p];
            if sel_before != cx.doc.selection {
                group.push(Edit::Selection { before: sel_before, after: cx.doc.selection.clone() });
            }
            cx.history.push("Transform", Edit::Group(group));
        }
    }

    fn nudge(&mut self, cx: &mut Ctx, d: Vec2) {
        if !self.lift(cx) {
            return;
        }
        if let Some(f) = &mut self.float {
            f.xf.t += d;
            render(f, cx);
        }
    }
}

fn render(f: &mut Floating, cx: &mut Ctx) {
    let new_rect = f.screen_bounds().clamp_to(cx.doc.w, cx.doc.h);
    let area = new_rect.union(&f.last_rect);
    f.last_rect = new_rect;
    let smooth = cx.s.smooth_transform;
    let identity = f.xf == Xf::default();
    // Precompute the inverse mapping as an affine transform for speed.
    let o = f.unmap(pos2(0.0, 0.0));
    let dx = f.unmap(pos2(1.0, 0.0)) - o;
    let dy = f.unmap(pos2(0.0, 1.0)) - o;
    let img = &f.img;
    let origin = (f.origin.x as i32, f.origin.y as i32);
    f.session.apply(cx.doc, area, |x, y, src, _| {
        if !new_rect.contains(x, y) {
            return src;
        }
        if identity {
            let c = img.get(x - origin.0, y - origin.1);
            return over(src, c);
        }
        let px = x as f32 + 0.5;
        let py = y as f32 + 0.5;
        let u = o + dx * px + dy * py;
        let c = if smooth {
            let s = img.sample_bilinear(u.x, u.y);
            [s[0].round() as u8, s[1].round() as u8, s[2].round() as u8, s[3].round().clamp(0.0, 255.0) as u8]
        } else {
            img.get(u.x.floor() as i32, u.y.floor() as i32)
        };
        over(src, c)
    });
}

fn transform_mask(f: &Floating, m: &Mask, w: u32, h: u32, smooth: bool) -> Mask {
    let mut out = Mask::new(w, h);
    let r = f.screen_bounds().clamp_to(w, h);
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let u = f.unmap(pos2(x as f32 + 0.5, y as f32 + 0.5));
            let v = if smooth {
                let fx = u.x - 0.5;
                let fy = u.y - 0.5;
                let (x0, y0) = (fx.floor() as i32, fy.floor() as i32);
                let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
                let g = |a: i32, b: i32| m.getf(a, b);
                g(x0, y0) * (1.0 - tx) * (1.0 - ty)
                    + g(x0 + 1, y0) * tx * (1.0 - ty)
                    + g(x0, y0 + 1) * (1.0 - tx) * ty
                    + g(x0 + 1, y0 + 1) * tx * ty
            } else {
                m.getf(u.x.floor() as i32, u.y.floor() as i32)
            };
            out.data[(y as u32 * w + x as u32) as usize] = (v * 255.0).round() as u8;
        }
    }
    out
}

/// Handle sign pattern: (ex, ey) in local box space.
fn handle_signs(i: usize) -> (f32, f32) {
    match i {
        0 => (-1.0, -1.0),
        1 => (1.0, -1.0),
        2 => (1.0, 1.0),
        3 => (-1.0, 1.0),
        4 => (0.0, -1.0),
        5 => (1.0, 0.0),
        6 => (0.0, 1.0),
        _ => (-1.0, 0.0),
    }
}

impl Tool for TransformTool {
    fn press(&mut self, cx: &mut Ctx, p: Pos2, _btn: Btn) {
        if self.float.is_none() && !self.lift(cx) {
            return;
        }
        let Some(f) = &self.float else { return };
        let hs = f.handles(cx.zoom);
        self.grab = if cx.near(p, hs[8]) {
            Some(Grab::Rotate)
        } else if let Some(i) = (0..8).find(|&i| cx.near(p, hs[i])) {
            Some(Grab::Handle(i))
        } else {
            let u = f.unmap(p);
            let inside = u.x >= 0.0 && u.y >= 0.0 && u.x <= f.img.w as f32 && u.y <= f.img.h as f32;
            if inside {
                Some(Grab::Move)
            } else {
                // Outside the box: rotate (like most editors).
                Some(Grab::Rotate)
            }
        };
        self.grab_pos = p;
        self.xf0 = f.xf;
    }

    fn drag(&mut self, cx: &mut Ctx, p: Pos2) {
        let (Some(g), Some(f)) = (self.grab, &mut self.float) else { return };
        let xf0 = self.xf0;
        match g {
            Grab::Move => {
                let mut d = p - self.grab_pos;
                if cx.mods.shift {
                    // Constrain to horizontal/vertical.
                    if d.x.abs() > d.y.abs() { d.y = 0.0 } else { d.x = 0.0 }
                }
                f.xf.t = (xf0.t + d).round();
            }
            Grab::Rotate => {
                let c = f.center0() + xf0.t;
                let a0 = (self.grab_pos - c).angle();
                let a1 = (p - c).angle();
                let mut r = xf0.rot + (a1 - a0);
                if cx.mods.shift {
                    let step = 15f32.to_radians();
                    r = (r / step).round() * step;
                }
                f.xf.rot = r;
            }
            Grab::Handle(i) => {
                // Work in the box's rotated frame, centered at the original center.
                let (ex, ey) = handle_signs(i);
                let (w, h) = (f.img.w as f32, f.img.h as f32);
                let l = rotate(p - (f.center0() + xf0.t), -xf0.rot);
                // Opposite handle stays fixed.
                let anchor = vec2(-ex * w / 2.0 * xf0.sx, -ey * h / 2.0 * xf0.sy);
                let min_scale = |s: f32, size: f32, fallback: f32| {
                    if (s * size).abs() < 1.0 { fallback.signum() / size } else { s }
                };
                let mut sx = if ex != 0.0 { min_scale((l.x - anchor.x) / (ex * w), w, xf0.sx) } else { xf0.sx };
                let mut sy = if ey != 0.0 { min_scale((l.y - anchor.y) / (ey * h), h, xf0.sy) } else { xf0.sy };
                if cx.mods.shift && ex != 0.0 && ey != 0.0 {
                    let k = (sx / xf0.sx).abs().max((sy / xf0.sy).abs());
                    sx = sx.signum() * xf0.sx.abs() * k;
                    sy = sy.signum() * xf0.sy.abs() * k;
                }
                let center = vec2(
                    if ex != 0.0 { anchor.x + ex * w / 2.0 * sx } else { 0.0 },
                    if ey != 0.0 { anchor.y + ey * h / 2.0 * sy } else { 0.0 },
                );
                f.xf.sx = sx;
                f.xf.sy = sy;
                f.xf.t = xf0.t + rotate(center, xf0.rot);
            }
        }
        render(f, cx);
    }

    fn release(&mut self, _cx: &mut Ctx, _p: Pos2) {
        self.grab = None;
    }

    fn key(&mut self, cx: &mut Ctx, key: egui::Key) -> bool {
        let step = if cx.mods.shift { 10.0 } else { 1.0 };
        match key {
            egui::Key::ArrowLeft => self.nudge(cx, vec2(-step, 0.0)),
            egui::Key::ArrowRight => self.nudge(cx, vec2(step, 0.0)),
            egui::Key::ArrowUp => self.nudge(cx, vec2(0.0, -step)),
            egui::Key::ArrowDown => self.nudge(cx, vec2(0.0, step)),
            egui::Key::Enter if self.float.is_some() => self.apply_pending(cx),
            egui::Key::Escape if self.float.is_some() => self.cancel(cx),
            _ => return false,
        }
        true
    }

    fn commit(&mut self, cx: &mut Ctx) {
        self.apply_pending(cx);
    }

    fn cancel(&mut self, cx: &mut Ctx) {
        if let Some(f) = self.float.take() {
            f.session.cancel(cx.doc);
            cx.doc.selection = f.sel_before;
            cx.doc.selection_changed = true;
        }
        self.grab = None;
    }

    fn busy(&self) -> bool {
        self.float.is_some()
    }

    fn cursor(&self, _hover: Option<Pos2>) -> egui::CursorIcon {
        match self.grab {
            Some(Grab::Move) => egui::CursorIcon::Grabbing,
            Some(_) => egui::CursorIcon::Crosshair,
            None => egui::CursorIcon::Move,
        }
    }

    fn overlay(&self, o: &OverlayCx) {
        let Some(f) = &self.float else { return };
        let c = f.corners();
        let pts: Vec<Pos2> = c.iter().chain(std::iter::once(&c[0])).map(|p| o.view.to_screen(*p)).collect();
        o.painter.add(egui::Shape::line(pts.clone(), Stroke::new(3.0, Color32::from_black_alpha(100))));
        o.painter.add(egui::Shape::line(pts, Stroke::new(1.0, Color32::from_rgb(80, 160, 255))));
        let hs = f.handles(o.view.zoom);
        o.guide_line(hs[4], hs[8]);
        for h in &hs[..8] {
            o.handle(*h);
        }
        o.round_handle(hs[8], Color32::from_rgb(80, 160, 255));
    }

    fn options(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let mut changed = ui.checkbox(&mut cx.s.smooth_transform, "Smooth").on_hover_text("Bilinear resampling").changed();
        match &mut self.float {
            None => {
                if ui.button("Transform selection").on_hover_text("Ctrl/Cmd+T").clicked() {
                    self.lift(cx);
                }
            }
            Some(f) => {
                ui.separator();
                ui.label("X");
                changed |= ui.add(egui::DragValue::new(&mut f.xf.t.x).speed(1.0)).changed();
                ui.label("Y");
                changed |= ui.add(egui::DragValue::new(&mut f.xf.t.y).speed(1.0)).changed();
                let mut w = f.xf.sx * 100.0;
                let mut h = f.xf.sy * 100.0;
                ui.label("W");
                if ui.add(egui::DragValue::new(&mut w).speed(1.0).suffix("%")).changed() {
                    f.xf.sx = if w.abs() < 1.0 { 0.01 } else { w / 100.0 };
                    changed = true;
                }
                ui.label("H");
                if ui.add(egui::DragValue::new(&mut h).speed(1.0).suffix("%")).changed() {
                    f.xf.sy = if h.abs() < 1.0 { 0.01 } else { h / 100.0 };
                    changed = true;
                }
                let mut deg = f.xf.rot.to_degrees();
                ui.label("Angle");
                if ui.add(egui::DragValue::new(&mut deg).speed(1.0).suffix("°")).changed() {
                    f.xf.rot = deg.to_radians();
                    changed = true;
                }
                if ui.button("⬌").on_hover_text("Flip horizontal").clicked() {
                    f.xf.sx = -f.xf.sx;
                    changed = true;
                }
                if ui.button("⬍").on_hover_text("Flip vertical").clicked() {
                    f.xf.sy = -f.xf.sy;
                    changed = true;
                }
                if ui.button("⟳ 90°").clicked() {
                    f.xf.rot += std::f32::consts::FRAC_PI_2;
                    changed = true;
                }
            }
        }
        if changed
            && let Some(f) = &mut self.float {
                render(f, cx);
            }
    }
}
