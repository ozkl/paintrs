//! On-canvas text: click to place, type in the options bar (focus moves
//! there automatically), drag to move, change font/size live, then apply.

use egui::{Pos2, Ui};
use serde::{Deserialize, Serialize};

use super::{Btn, Ctx, OverlayCx, Tool};
use crate::doc::{EditSession, Paint};
use crate::raster::IRect;
use crate::text;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

pub const TEXT_EDIT_ID: &str = "paintrs_text_edit";

#[derive(Default)]
pub struct TextTool {
    session: Option<EditSession>,
    anchor: Pos2,
    pub text: String,
    last_rect: IRect,
    dragging: bool,
    grab: Pos2,
    swap: bool,
    /// Ask the options bar to focus the text field.
    pub want_focus: bool,
}

impl TextTool {
    fn bounds(&self, cx_fonts: &crate::text::FontStore, s: &super::ToolSettings) -> Option<(Pos2, Pos2)> {
        let font = cx_fonts.get(s.text_font)?;
        let l = text::layout(font, if self.text.is_empty() { " " } else { &self.text }, s.text_size, s.text_line_spacing, s.text_align, self.anchor);
        Some((l.min, l.max))
    }

    fn render(&mut self, cx: &mut Ctx) {
        let Some(sess) = &mut self.session else { return };
        cx.cov.clear(self.last_rect);
        let Some(font) = cx.fonts.get(cx.s.text_font) else { return };
        let r = text::rasterize(cx.cov, font, &self.text, cx.s.text_size, cx.s.text_line_spacing, cx.s.text_align, self.anchor);
        let area = r.union(&self.last_rect);
        self.last_rect = r;
        let color = if self.swap { cx.s.secondary } else { cx.s.primary };
        sess.apply_coverage(cx.doc, cx.cov, area, Paint::Color { color, opacity: 1.0 });
    }

    fn apply_pending(&mut self, cx: &mut Ctx) {
        if let Some(s) = self.session.take() {
            s.commit(cx.doc, cx.history, "Text");
        }
        cx.cov.clear(self.last_rect);
        self.last_rect = IRect::EMPTY;
        self.text.clear();
    }
}

impl Tool for TextTool {
    fn press(&mut self, cx: &mut Ctx, p: Pos2, btn: Btn) {
        if self.session.is_some() {
            if let Some((a, b)) = self.bounds(cx.fonts, cx.s)
                && egui::Rect::from_min_max(a, b).expand(cx.screen_dist(6.0)).contains(p) {
                    self.dragging = true;
                    self.grab = p;
                    return;
                }
            self.apply_pending(cx);
        }
        cx.ensure_buffers();
        match cx.doc.begin_edit() {
            Ok(s) => self.session = Some(s),
            Err(e) => return cx.warn(e),
        }
        self.swap = btn == Btn::Secondary;
        self.anchor = p.round();
        self.text.clear();
        self.want_focus = true;
    }

    fn drag(&mut self, cx: &mut Ctx, p: Pos2) {
        if self.dragging {
            self.anchor += p - self.grab;
            self.grab = p;
            self.render(cx);
        }
    }

    fn release(&mut self, _cx: &mut Ctx, _p: Pos2) {
        if self.dragging {
            self.dragging = false;
            self.anchor = self.anchor.round();
            self.want_focus = true;
        }
    }

    fn key(&mut self, cx: &mut Ctx, key: egui::Key) -> bool {
        if self.session.is_none() {
            return false;
        }
        match key {
            egui::Key::Escape => self.apply_pending(cx),
            egui::Key::Enter if cx.mods.command => self.apply_pending(cx),
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
        self.last_rect = IRect::EMPTY;
        self.text.clear();
    }

    fn busy(&self) -> bool {
        self.session.is_some()
    }

    fn cursor(&self, _hover: Option<Pos2>) -> egui::CursorIcon {
        egui::CursorIcon::Text
    }

    fn overlay(&self, o: &OverlayCx) {
        if self.session.is_none() {
            return;
        }
        let Some((a, b)) = self.bounds(o.fonts, o.s) else { return };
        o.dashed_poly(&[a, egui::pos2(b.x, a.y), b, egui::pos2(a.x, b.y)], true);
        // Blinking caret at the end of the text.
        if (o.time * 2.0) as i64 % 2 == 0
            && let Some(font) = o.fonts.get(o.s.text_font) {
                let last = self.text.rsplit('\n').next().unwrap_or("");
                let lines = self.text.split('\n').count().max(1);
                let l = text::layout(font, last, o.s.text_size, o.s.text_line_spacing, o.s.text_align, self.anchor);
                let lh = (b.y - a.y) / lines as f32;
                let x = match o.s.text_align {
                    TextAlign::Left | TextAlign::Center => l.max.x,
                    TextAlign::Right => self.anchor.x,
                };
                let y0 = a.y + lh * (lines - 1) as f32;
                o.guide_line(egui::pos2(x, y0), egui::pos2(x, y0 + lh));
            }
    }

    fn options(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let mut changed = false;
        let names: Vec<String> = cx.fonts.fonts.iter().map(|(n, _)| n.clone()).collect();
        egui::ComboBox::from_id_salt("font")
            .width(130.0)
            .selected_text(names.get(cx.s.text_font).cloned().unwrap_or_default())
            .show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() {
                    changed |= ui.selectable_value(&mut cx.s.text_font, i, n).changed();
                }
            });
        ui.label("Size");
        changed |= ui.add(egui::DragValue::new(&mut cx.s.text_size).range(4.0..=1000.0).speed(0.5).suffix(" px")).changed();
        for (a, l) in [(TextAlign::Left, "⬅"), (TextAlign::Center, "↔"), (TextAlign::Right, "➡")] {
            changed |= ui.selectable_value(&mut cx.s.text_align, a, l).on_hover_text(format!("Align {a:?}")).changed();
        }
        ui.label("Line spacing");
        changed |= ui.add(egui::DragValue::new(&mut cx.s.text_line_spacing).range(0.5..=4.0).speed(0.01)).changed();
        if self.session.is_some() {
            ui.separator();
            let edit = egui::TextEdit::multiline(&mut self.text)
                .id(egui::Id::new(TEXT_EDIT_ID))
                .desired_rows(1)
                .desired_width(220.0)
                .hint_text("Type your text…");
            let r = ui.add(edit);
            if self.want_focus {
                r.request_focus();
                self.want_focus = false;
            }
            changed |= r.changed();
            // The text field has keyboard focus, so handle apply keys here:
            // Ctrl/Cmd+Enter applies, and Esc applies too (it's undoable,
            // unlike silently discarding what was typed).
            let (apply_key, esc) = ui.input(|i| (i.modifiers.command && i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            if (r.has_focus() && apply_key) || (r.lost_focus() && esc) {
                // The multiline field may have just inserted the newline.
                if apply_key && self.text.ends_with('\n') {
                    self.text.pop();
                    changed = true;
                }
                if changed {
                    self.render(cx);
                }
                self.apply_pending(cx);
                return;
            }
            if ui.button("✔ Apply").on_hover_text("Ctrl/Cmd+Enter").clicked() {
                self.apply_pending(cx);
                return;
            }
            if ui.button("✖").on_hover_text("Discard this text").clicked() {
                self.cancel(cx);
                return;
            }
        } else {
            ui.label("Click on the canvas to place text");
        }
        if changed && self.session.is_some() {
            self.render(cx);
        }
    }
}
