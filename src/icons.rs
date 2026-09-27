//! Tool icons: the qpaint icon atlas converted to a white mask (so it can be
//! tinted for light and dark themes), plus vector icons for new tools.

use egui::{Color32, Painter, Rect, Stroke, TextureHandle, pos2, vec2};

use crate::tools::ToolId;

const ATLAS: &[u8] = include_bytes!("../assets/icons.png");
const CELL: f32 = 32.0;

pub fn load_atlas(ctx: &egui::Context) -> Option<TextureHandle> {
    let img = image::load_from_memory(ATLAS).ok()?.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let px: Vec<Color32> = img
        .pixels()
        .map(|p| {
            // Dark line art → white with alpha, so tint controls the color.
            let lum = (p[0] as u32 + p[1] as u32 + p[2] as u32) / 3;
            let a = (p[3] as u32 * (255 - lum) / 255) as u8;
            Color32::from_white_alpha(a)
        })
        .collect();
    Some(ctx.load_texture("icons", egui::ColorImage::new([w, h], px), egui::TextureOptions::LINEAR))
}

fn atlas_cell(t: ToolId) -> Option<(u32, u32)> {
    Some(match t {
        ToolId::Pencil => (0, 0),
        ToolId::Brush => (0, 1),
        ToolId::Line => (0, 2),
        ToolId::Rect => (0, 3),
        ToolId::Text => (0, 4),
        ToolId::Eraser => (1, 0),
        ToolId::Picker => (1, 1),
        ToolId::SelectRect => (1, 2),
        ToolId::Lasso => (1, 3),
        ToolId::Ellipse => (1, 4),
        ToolId::Move => (3, 0),
        ToolId::Fill => (2, 2),
        ToolId::Pan => (2, 3),
        ToolId::Gradient => (2, 4),
        _ => return None,
    })
}

pub fn paint_tool_icon(painter: &Painter, atlas: Option<&TextureHandle>, t: ToolId, rect: Rect, color: Color32) {
    if let (Some(tex), Some((row, col))) = (atlas, atlas_cell(t)) {
        let size = tex.size_vec2();
        let uv = Rect::from_min_size(
            pos2(col as f32 * CELL / size.x, row as f32 * CELL / size.y),
            vec2(CELL / size.x, CELL / size.y),
        );
        painter.image(tex.id(), rect, uv, color);
        return;
    }
    let s = Stroke::new(1.6, color);
    let c = rect.center();
    let r = rect.width() * 0.34;
    match t {
        ToolId::SelectEllipse => {
            let pts = crate::raster::ellipse_points(c, r, r * 0.75);
            painter.extend(egui::Shape::dashed_line(&[pts.clone(), vec![pts[0]]].concat(), s, 3.0, 2.5));
        }
        ToolId::MagicWand => {
            painter.line_segment([c + vec2(-r, r), c + vec2(r * 0.3, -r * 0.3)], Stroke::new(2.2, color));
            let star = c + vec2(r * 0.55, -r * 0.55);
            for a in 0..4 {
                let ang = a as f32 * std::f32::consts::FRAC_PI_4 * 2.0 + 0.4;
                let d = vec2(ang.cos(), ang.sin()) * r * 0.45;
                painter.line_segment([star - d, star + d], Stroke::new(1.2, color));
            }
        }
        ToolId::Zoom => {
            let lc = c + vec2(-r * 0.2, -r * 0.2);
            painter.circle_stroke(lc, r * 0.6, Stroke::new(1.8, color));
            painter.line_segment([lc + vec2(r * 0.45, r * 0.45), c + vec2(r, r)], Stroke::new(2.5, color));
        }
        _ => {
            painter.text(c, egui::Align2::CENTER_CENTER, &t.name()[..1], egui::FontId::proportional(16.0), color);
        }
    }
}
