//! Canvas view: zoom/pan transform, tiled display textures with dirty-rect
//! uploads, and overlays (checkerboard, grids, selection outline).

use egui::{
    Color32, ColorImage, Painter, Pos2, Rect, Shape, Stroke, TextureHandle, TextureOptions, Vec2, pos2, vec2,
};

use crate::doc::Document;
use crate::raster::{IRect, Mask};

/// Display tiles keep every texture well under WebGL size limits and let us
/// upload only what changed.
const TILE: i32 = 512;

pub struct View {
    pub zoom: f32,
    /// Canvas origin, relative to the widget's top-left.
    pub offset: Vec2,
    /// Widget rect in screen points (updated every frame).
    pub rect: Rect,
    pub fit_pending: bool,
    tiles: Vec<Option<TextureHandle>>,
    cols: i32,
    rows: i32,
    size: (u32, u32),
    checker: Option<TextureHandle>,
    /// Selection outline edges in canvas coordinates.
    outline: Vec<[Pos2; 2]>,
    scratch: Vec<Color32>,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            offset: Vec2::ZERO,
            rect: Rect::ZERO,
            fit_pending: true,
            tiles: Vec::new(),
            cols: 0,
            rows: 0,
            size: (0, 0),
            checker: None,
            outline: Vec::new(),
            scratch: Vec::new(),
        }
    }
}

const TEX_OPTS: TextureOptions = TextureOptions {
    magnification: egui::TextureFilter::Nearest,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: None,
};

impl View {
    pub fn to_screen(&self, p: Pos2) -> Pos2 {
        self.rect.min + self.offset + p.to_vec2() * self.zoom
    }

    pub fn to_canvas(&self, s: Pos2) -> Pos2 {
        ((s - self.rect.min - self.offset) / self.zoom).to_pos2()
    }

    pub fn canvas_screen_rect(&self, w: u32, h: u32) -> Rect {
        Rect::from_min_max(self.to_screen(Pos2::ZERO), self.to_screen(pos2(w as f32, h as f32)))
    }

    pub fn fit(&mut self, w: u32, h: u32) {
        let avail = self.rect.size() - vec2(48.0, 48.0);
        if avail.x <= 0.0 || avail.y <= 0.0 {
            return;
        }
        let z = (avail.x / w as f32).min(avail.y / h as f32).min(1.0);
        self.zoom = snap_zoom(z);
        self.center(w, h);
        self.fit_pending = false;
    }

    pub fn center(&mut self, w: u32, h: u32) {
        let s = vec2(w as f32, h as f32) * self.zoom;
        self.offset = ((self.rect.size() - s) / 2.0).floor();
    }

    pub fn set_zoom_at(&mut self, zoom: f32, screen: Pos2) {
        let zoom = zoom.clamp(0.02, 64.0);
        let c = self.to_canvas(screen);
        self.zoom = zoom;
        // Keep the canvas point under the cursor fixed.
        self.offset = screen - self.rect.min - c.to_vec2() * zoom;
    }

    pub fn zoom_step(&mut self, dir: i32, screen: Pos2) {
        const STEPS: [f32; 22] = [
            0.02, 0.05, 0.0833, 0.125, 0.1667, 0.25, 0.3333, 0.5, 0.6667, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0,
            12.0, 16.0, 24.0, 32.0, 64.0,
        ];
        let z = if dir > 0 {
            STEPS.iter().copied().find(|&s| s > self.zoom * 1.001).unwrap_or(64.0)
        } else {
            STEPS.iter().rev().copied().find(|&s| s < self.zoom / 1.001).unwrap_or(0.02)
        };
        self.set_zoom_at(z, screen);
    }

    /// Upload changed regions of the composite to the GPU.
    pub fn sync_textures(&mut self, ctx: &egui::Context, doc: &mut Document) {
        if self.size != (doc.w, doc.h) {
            self.size = (doc.w, doc.h);
            self.cols = (doc.w as i32 + TILE - 1) / TILE;
            self.rows = (doc.h as i32 + TILE - 1) / TILE;
            self.tiles = (0..self.cols * self.rows).map(|_| None).collect();
            doc.dirty = Some(doc.bounds());
        }
        let Some(dirty) = doc.dirty.take() else { return };
        let dirty = dirty.clamp_to(doc.w, doc.h);
        for ty in 0..self.rows {
            for tx in 0..self.cols {
                let tile_rect = IRect::new(tx * TILE, ty * TILE, (tx + 1) * TILE, (ty + 1) * TILE).clamp_to(doc.w, doc.h);
                let sub = tile_rect.intersect(&dirty);
                let slot = (ty * self.cols + tx) as usize;
                if sub.is_empty() && self.tiles[slot].is_some() {
                    continue;
                }
                match &mut self.tiles[slot] {
                    Some(tex) => {
                        doc.composite_rect(sub, &mut self.scratch);
                        let img = ColorImage::new(
                            [sub.width() as usize, sub.height() as usize],
                            std::mem::take(&mut self.scratch),
                        );
                        tex.set_partial([(sub.x0 - tile_rect.x0) as usize, (sub.y0 - tile_rect.y0) as usize], img, TEX_OPTS);
                    }
                    None => {
                        doc.composite_rect(tile_rect, &mut self.scratch);
                        let img = ColorImage::new(
                            [tile_rect.width() as usize, tile_rect.height() as usize],
                            std::mem::take(&mut self.scratch),
                        );
                        self.tiles[slot] = Some(ctx.load_texture(format!("tile-{tx}-{ty}"), img, TEX_OPTS));
                    }
                }
            }
        }
    }

    pub fn update_outline(&mut self, sel: Option<&Mask>) {
        self.outline = sel.map(selection_outline).unwrap_or_default();
    }

    pub fn has_outline(&self) -> bool {
        !self.outline.is_empty()
    }

    /// Draw checkerboard + canvas tiles.
    pub fn paint_canvas(&mut self, ctx: &egui::Context, painter: &Painter, w: u32, h: u32, dark: bool) {
        let canvas = self.canvas_screen_rect(w, h);
        // Drop shadow + border for depth.
        painter.rect_filled(canvas.translate(vec2(3.0, 3.0)), 0.0, Color32::from_black_alpha(60));
        let checker = self.checker.get_or_insert_with(|| {
            let (a, b) = (Color32::from_gray(255), Color32::from_gray(204));
            let px = (0..4).map(|i| if (i % 2) ^ (i / 2) == 0 { a } else { b }).collect();
            ctx.load_texture(
                "checker",
                ColorImage::new([2, 2], px),
                TextureOptions { wrap_mode: egui::TextureWrapMode::Repeat, ..TextureOptions::NEAREST },
            )
        });
        // Checker squares are 8 screen points regardless of zoom.
        let uv = Rect::from_min_size(Pos2::ZERO, canvas.size() / 16.0);
        painter.image(checker.id(), canvas, uv, Color32::WHITE);
        for ty in 0..self.rows {
            for tx in 0..self.cols {
                if let Some(tex) = &self.tiles[(ty * self.cols + tx) as usize] {
                    let x0 = tx * TILE;
                    let y0 = ty * TILE;
                    let x1 = ((tx + 1) * TILE).min(w as i32);
                    let y1 = ((ty + 1) * TILE).min(h as i32);
                    let r = Rect::from_min_max(
                        self.to_screen(pos2(x0 as f32, y0 as f32)),
                        self.to_screen(pos2(x1 as f32, y1 as f32)),
                    );
                    if r.intersects(painter.clip_rect()) {
                        painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                    }
                }
            }
        }
        let border = if dark { Color32::from_gray(90) } else { Color32::from_gray(140) };
        painter.rect_stroke(canvas, 0.0, Stroke::new(1.0, border), egui::StrokeKind::Outside);
    }

    pub fn paint_grid(&self, painter: &Painter, w: u32, h: u32, step: u32, color: Color32) {
        let spacing = step as f32 * self.zoom;
        if spacing < 4.0 {
            return;
        }
        let canvas = self.canvas_screen_rect(w, h);
        let vis = canvas.intersect(painter.clip_rect());
        if vis.width() <= 0.0 || vis.height() <= 0.0 {
            return;
        }
        let c0 = self.to_canvas(vis.min);
        let c1 = self.to_canvas(vis.max);
        let stroke = Stroke::new(1.0, color);
        let mut x = (c0.x / step as f32).floor() as i64 * step as i64;
        while (x as f32) <= c1.x {
            if x > 0 && x < w as i64 {
                let sx = self.to_screen(pos2(x as f32, 0.0)).x.round() + 0.5;
                painter.line_segment([pos2(sx, vis.min.y), pos2(sx, vis.max.y)], stroke);
            }
            x += step as i64;
        }
        let mut y = (c0.y / step as f32).floor() as i64 * step as i64;
        while (y as f32) <= c1.y {
            if y > 0 && y < h as i64 {
                let sy = self.to_screen(pos2(0.0, y as f32)).y.round() + 0.5;
                painter.line_segment([pos2(vis.min.x, sy), pos2(vis.max.x, sy)], stroke);
            }
            y += step as i64;
        }
    }

    /// Marching ants.
    pub fn paint_selection(&self, painter: &Painter, time: f64) {
        if self.outline.is_empty() {
            return;
        }
        let clip = painter.clip_rect();
        let offset = (time * 12.0) as f32 % 8.0;
        let mut shapes = Vec::new();
        let black = Stroke::new(1.0, Color32::BLACK);
        let white = Stroke::new(1.0, Color32::WHITE);
        for [a, b] in &self.outline {
            let (sa, sb) = (self.to_screen(*a), self.to_screen(*b));
            let r = Rect::from_two_pos(sa, sb).expand(1.0);
            if !r.intersects(clip) {
                continue;
            }
            let pts = [pos2(sa.x.round() + 0.5, sa.y.round() + 0.5), pos2(sb.x.round() + 0.5, sb.y.round() + 0.5)];
            shapes.push(Shape::line_segment(pts, black));
            // Phase along the diagonal so dashes flow continuously around corners.
            let phase = offset + (pts[0].x + pts[0].y) % 8.0;
            Shape::dashed_line_many_with_offset(&pts, white, &[4.0], &[4.0], phase, &mut shapes);
        }
        painter.extend(shapes);
    }
}

pub fn snap_zoom(z: f32) -> f32 {
    if z >= 1.0 { z.floor() } else { z }
}

/// Boundary edges of the selection (threshold 50%), merged into runs.
fn selection_outline(m: &Mask) -> Vec<[Pos2; 2]> {
    let (w, h) = (m.w as i32, m.h as i32);
    let inside = |x: i32, y: i32| m.get(x, y) >= 128;
    let mut out = Vec::new();
    let Some(b) = m.bounds() else { return out };
    // Horizontal edges between rows y-1 and y.
    for y in b.y0..=b.y1.min(h) {
        let mut run: Option<i32> = None;
        for x in b.x0..=b.x1 {
            let edge = x < b.x1 && inside(x, y - 1) != inside(x, y);
            match (edge, run) {
                (true, None) => run = Some(x),
                (false, Some(s)) => {
                    out.push([pos2(s as f32, y as f32), pos2(x as f32, y as f32)]);
                    run = None;
                }
                _ => {}
            }
        }
    }
    for x in b.x0..=b.x1.min(w) {
        let mut run: Option<i32> = None;
        for y in b.y0..=b.y1 {
            let edge = y < b.y1 && inside(x - 1, y) != inside(x, y);
            match (edge, run) {
                (true, None) => run = Some(y),
                (false, Some(s)) => {
                    out.push([pos2(x as f32, s as f32), pos2(x as f32, y as f32)]);
                    run = None;
                }
                _ => {}
            }
        }
    }
    if out.len() > 200_000 {
        // Very noisy selections: show the bounding box only.
        let (x0, y0, x1, y1) = (b.x0 as f32, b.y0 as f32, b.x1 as f32, b.y1 as f32);
        return vec![
            [pos2(x0, y0), pos2(x1, y0)],
            [pos2(x1, y0), pos2(x1, y1)],
            [pos2(x1, y1), pos2(x0, y1)],
            [pos2(x0, y1), pos2(x0, y0)],
        ];
    }
    out
}
