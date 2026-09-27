use egui::{Color32, Pos2, Stroke, Ui, vec2};

use super::{Btn, Ctx, OverlayCx, Tool, to_color32};

pub struct PickerTool;

fn sample(cx: &mut Ctx, p: Pos2, btn: Btn) {
    let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
    if !cx.doc.bounds().contains(x, y) {
        return;
    }
    let c = if cx.s.picker_sample_all { cx.doc.composite_pixel(x, y) } else { cx.doc.active_layer().img.get(x, y) };
    match btn {
        Btn::Primary => cx.s.primary = c,
        Btn::Secondary => cx.s.secondary = c,
    }
}

thread_local! {
    static LAST_BTN: std::cell::Cell<Btn> = const { std::cell::Cell::new(Btn::Primary) };
}

impl Tool for PickerTool {
    fn press(&mut self, cx: &mut Ctx, p: Pos2, btn: Btn) {
        LAST_BTN.with(|b| b.set(btn));
        sample(cx, p, btn);
    }

    fn drag(&mut self, cx: &mut Ctx, p: Pos2) {
        sample(cx, p, LAST_BTN.with(|b| b.get()));
    }

    fn overlay(&self, o: &OverlayCx) {
        // Loupe swatch showing the color under the cursor.
        let Some(h) = o.hover else { return };
        let (x, y) = (h.x.floor() as i32, h.y.floor() as i32);
        if !o.doc.bounds().contains(x, y) {
            return;
        }
        let c = if o.s.picker_sample_all { o.doc.composite_pixel(x, y) } else { o.doc.active_layer().img.get(x, y) };
        let center = o.view.to_screen(h) + vec2(28.0, -28.0);
        o.painter.circle(center, 16.0, to_color32([c[0], c[1], c[2], 255]), Stroke::new(2.0, Color32::WHITE));
        o.painter.circle_stroke(center, 17.5, Stroke::new(1.0, Color32::BLACK));
    }

    fn options(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        ui.checkbox(&mut cx.s.picker_sample_all, "Sample all layers");
        ui.label("(Alt-click with Brush or Pencil also picks)");
    }
}
