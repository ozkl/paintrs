//! Scripting with Rhai (pure Rust, identical on native and web).
//! The API mirrors qpaint's JavaScript API names so existing scripts port
//! with minimal changes, and adds shapes, text, pixels reads and layers.
//! A whole script run is one undo step.

use std::cell::RefCell;
use std::rc::Rc;

use egui::pos2;
use rhai::{Dynamic, Engine, EvalAltResult, Map};

use crate::doc::Document;
use crate::history::History;
use crate::raster::{Coverage, IRect, Image, LineStyle, Mask, Rgba, blend_color, ellipse_points};

#[derive(Clone, Copy, Debug)]
pub struct Color {
    pub r: i64,
    pub g: i64,
    pub b: i64,
    pub a: i64,
}

impl Color {
    fn rgba(self) -> Rgba {
        [self.r.clamp(0, 255) as u8, self.g.clamp(0, 255) as u8, self.b.clamp(0, 255) as u8, self.a.clamp(0, 255) as u8]
    }
}

struct State {
    w: u32,
    h: u32,
    layers: Vec<(String, Image)>,
    active: usize,
    sel: Option<Mask>,
    cov: Coverage,
    log: String,
    font: Option<ab_glyph::FontArc>,
    primary: Rgba,
    secondary: Rgba,
    rng: u64,
}

impl State {
    fn paint(&mut self, rect: IRect, color: Rgba) {
        let rect = rect.clamp_to(self.w, self.h);
        let img = &mut self.layers[self.active].1;
        let a = color[3] as f32 / 255.0;
        let rgb = [color[0] as f32, color[1] as f32, color[2] as f32];
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let i = y as usize * self.w as usize + x as usize;
                let mut c = self.cov.data[i].min(1.0);
                if let Some(m) = &self.sel {
                    c *= m.data[i] as f32 / 255.0;
                }
                if c > 0.0 {
                    img.px[i] = blend_color(img.px[i], rgb, c * a);
                }
            }
        }
        self.cov.clear(rect);
    }
}

fn num(d: &Dynamic) -> f32 {
    d.as_float().map(|f| f as f32).or_else(|_| d.as_int().map(|i| i as f32)).unwrap_or(0.0)
}

pub const EXAMPLES: &[(&str, &str)] = &[
    (
        "Checkerboard",
        r#"// Checkerboard pattern (same as the qpaint example)
let cell = 32;
let white = rgb(255, 255, 255);
let black = rgb(0, 0, 0);
for y in range(0, height(), cell) {
    for x in range(0, width(), cell) {
        let c = if ((x / cell) + (y / cell)) % 2 == 0 { white } else { black };
        fillRect(x, y, cell, cell, c);
    }
}
print("Checkerboard created!");
"#,
    ),
    (
        "Spiral",
        r#"// Rainbow spiral from the canvas center
let cx = width() / 2.0;
let cy = height() / 2.0;
let px = cx;
let py = cy;
for i in 0..720 {
    let t = i.to_float() * 0.05;
    let r = t * 6.0;
    let x = cx + r * t.cos();
    let y = cy + r * t.sin();
    drawLine(px, py, x, y, hsv(i.to_float() * 2.0, 0.9, 1.0), 3);
    px = x;
    py = y;
}
"#,
    ),
    (
        "Bubbles",
        r#"// Random translucent circles on a new layer
addLayer("Bubbles");
for i in 0..150 {
    let x = random() * width();
    let y = random() * height();
    let r = 5.0 + random() * 40.0;
    fillCircle(x, y, r, rgba(rand_int(0, 255), rand_int(100, 255), 255, 90));
    drawCircle(x, y, r, rgba(255, 255, 255, 160), 2);
}
"#,
    ),
    (
        "Vignette",
        r#"// Darken the corners (respects the selection)
let cx = width() / 2.0;
let cy = height() / 2.0;
let maxd = (cx * cx + cy * cy).sqrt();
for y in 0..height() {
    for x in 0..width() {
        let dx = x - cx;
        let dy = y - cy;
        let d = (dx * dx + dy * dy).sqrt() / maxd;
        let a = ((d - 0.5) * 2.0).max(0.0).min(1.0) * 200.0;
        if a > 0.0 { blendPixel(x, y, rgba(0, 0, 0, a.to_int())); }
    }
}
"#,
    ),
    (
        "Title text",
        r#"// Centered text with a drop shadow
let size = height() / 8.0;
let x = width() / 2.0;
let y = height() / 2.0 - size / 2.0;
drawTextCentered(x + 4, y + 4, "paintrs", size, rgba(0, 0, 0, 120));
drawTextCentered(x, y, "paintrs", size, primary());
"#,
    ),
];

pub const DOCS: &str = r##"Scripts are written in Rhai (https://rhai.rs) — a small JavaScript-like language.
Numbers may be integers or floats. Drawing is clipped to the selection.

COLORS
  rgb(r, g, b) / rgba(r, g, b, a)   0–255 components
  hsv(h, s, v)                     h 0–360, s/v 0–1
  hex("#ff8800") / Color(r, g, b, a)
  primary() / secondary()          current palette colors
  c.r c.g c.b c.a                  components

CANVAS
  width() height()                 (also getCanvasWidth/Height)
  getPixel(x, y) -> Color
  setPixel(x, y, c)                replace a pixel
  blendPixel(x, y, c)              blend a pixel
  clear()                          clear active layer
  fill(c)                          fill active layer (or selection)

SHAPES (anti-aliased)
  drawLine(x1, y1, x2, y2, c, thickness)
  drawRect(x, y, w, h, c, thickness)   fillRect(x, y, w, h, c)
  drawCircle(x, y, r, c, thickness)    fillCircle(x, y, r, c)
  drawEllipse(x, y, rx, ry, c, thickness)  fillEllipse(x, y, rx, ry, c)
  fillPolygon([[x, y], [x, y], ...], c)
  drawText(x, y, text, size, c)        drawTextCentered(x, y, text, size, c)

LAYERS
  getLayerCount()  getActiveLayerIndex()  setActiveLayerIndex(i)
  addLayer(name) -> index (and makes it active)

SELECTION
  isInSelection(x, y)  hasSelection()  getSelectionCenter() -> #{x, y}
  selectRect(x, y, w, h)  selectAll()  deselect()

UTILITIES
  print(value)  random() -> 0..1  rand_int(lo, hi)
  math: sin cos sqrt abs floor round min max PI() ... (e.g. x.sin())
"##;

pub struct ScriptResult {
    pub output: String,
    pub ok: bool,
}

pub fn run(code: &str, doc: &mut Document, history: &mut History, fonts: &crate::text::FontStore, colors: (Rgba, Rgba)) -> ScriptResult {
    let state = Rc::new(RefCell::new(State {
        w: doc.w,
        h: doc.h,
        layers: doc.layers.iter().map(|l| (l.name.clone(), (*l.img).clone())).collect(),
        active: doc.active,
        sel: doc.selection.clone(),
        cov: Coverage::new(doc.w, doc.h),
        log: String::new(),
        font: fonts.get(0).cloned(),
        primary: colors.0,
        secondary: colors.1,
        rng: 0x9E3779B97F4A7C15,
    }));
    let engine = build_engine(state.clone());
    let result = engine.run(code);
    let mut st = state.borrow_mut();
    let ok = match &result {
        Ok(()) => true,
        Err(e) => {
            st.log.push_str(&format!("Error: {e}\n"));
            false
        }
    };
    // Apply even partial results: the user can undo in one step.
    let layers = std::mem::take(&mut st.layers);
    let active = st.active;
    let sel = st.sel.take();
    let changed = layers.len() != doc.layers.len()
        || layers.iter().zip(&doc.layers).any(|((_, a), l)| *a != *l.img)
        || sel != doc.selection;
    if changed {
        doc.structural(history, "Script", |d| {
            for (i, (name, img)) in layers.into_iter().enumerate() {
                if i < d.layers.len() {
                    if *d.layers[i].img != img {
                        d.layers[i].img = std::sync::Arc::new(img);
                        d.layers[i].rev += 1;
                    }
                } else {
                    let l = d.make_layer(&name, img);
                    d.layers.push(l);
                }
            }
            d.active = active.min(d.layers.len() - 1);
            d.selection = sel;
        });
    }
    ScriptResult { output: std::mem::take(&mut st.log), ok }
}

type S = Rc<RefCell<State>>;

fn build_engine(state: S) -> Engine {
    let mut e = Engine::new();
    e.set_max_operations(80_000_000);
    e.set_max_call_levels(64);

    e.register_type_with_name::<Color>("Color")
        .register_get("r", |c: &mut Color| c.r)
        .register_get("g", |c: &mut Color| c.g)
        .register_get("b", |c: &mut Color| c.b)
        .register_get("a", |c: &mut Color| c.a)
        .register_fn("to_string", |c: &mut Color| format!("Color({}, {}, {}, {})", c.r, c.g, c.b, c.a));

    let mk = |r: Dynamic, g: Dynamic, b: Dynamic, a: Dynamic| Color {
        r: num(&r) as i64,
        g: num(&g) as i64,
        b: num(&b) as i64,
        a: num(&a) as i64,
    };
    e.register_fn("rgb", move |r: Dynamic, g: Dynamic, b: Dynamic| mk(r, g, b, Dynamic::from_int(255)));
    e.register_fn("rgba", mk);
    e.register_fn("Color", mk);
    e.register_fn("Color", move |r: Dynamic, g: Dynamic, b: Dynamic| mk(r, g, b, Dynamic::from_int(255)));
    e.register_fn("hsv", |h: Dynamic, s: Dynamic, v: Dynamic| {
        let c = crate::raster::hsv_to_rgb(num(&h), num(&s), num(&v));
        Color { r: c[0] as i64, g: c[1] as i64, b: c[2] as i64, a: 255 }
    });
    e.register_fn("hex", |s: &str| -> Result<Color, Box<EvalAltResult>> {
        let c = crate::raster::parse_hex(s).ok_or_else(|| format!("invalid hex color '{s}'"))?;
        Ok(Color { r: c[0] as i64, g: c[1] as i64, b: c[2] as i64, a: c[3] as i64 })
    });

    let st = state.clone();
    e.register_fn("primary", move || {
        let c = st.borrow().primary;
        Color { r: c[0] as i64, g: c[1] as i64, b: c[2] as i64, a: c[3] as i64 }
    });
    let st = state.clone();
    e.register_fn("secondary", move || {
        let c = st.borrow().secondary;
        Color { r: c[0] as i64, g: c[1] as i64, b: c[2] as i64, a: c[3] as i64 }
    });

    // Output
    let st = state.clone();
    e.on_print(move |s| {
        let mut st = st.borrow_mut();
        if st.log.len() < 100_000 {
            st.log.push_str(s);
            st.log.push('\n');
        }
    });
    let st = state.clone();
    e.on_debug(move |s, _, _| {
        st.borrow_mut().log.push_str(&format!("[debug] {s}\n"));
    });

    // Canvas queries
    for name in ["width", "getCanvasWidth"] {
        let st = state.clone();
        e.register_fn(name, move || st.borrow().w as i64);
    }
    for name in ["height", "getCanvasHeight"] {
        let st = state.clone();
        e.register_fn(name, move || st.borrow().h as i64);
    }

    // Random numbers (xorshift, deterministic per run).
    let st = state.clone();
    e.register_fn("random", move || {
        let mut s = st.borrow_mut();
        s.rng ^= s.rng << 13;
        s.rng ^= s.rng >> 7;
        s.rng ^= s.rng << 17;
        (s.rng >> 11) as f64 / (1u64 << 53) as f64
    });
    let st = state.clone();
    e.register_fn("rand_int", move |lo: i64, hi: i64| {
        let mut s = st.borrow_mut();
        s.rng ^= s.rng << 13;
        s.rng ^= s.rng >> 7;
        s.rng ^= s.rng << 17;
        if hi <= lo { lo } else { lo + (s.rng % (hi - lo + 1) as u64) as i64 }
    });

    // Pixels
    let st = state.clone();
    e.register_fn("getPixel", move |x: Dynamic, y: Dynamic| {
        let s = st.borrow();
        let c = s.layers[s.active].1.get(num(&x).floor() as i32, num(&y).floor() as i32);
        Color { r: c[0] as i64, g: c[1] as i64, b: c[2] as i64, a: c[3] as i64 }
    });
    let st = state.clone();
    e.register_fn("setPixel", move |x: Dynamic, y: Dynamic, c: Color| {
        let mut s = st.borrow_mut();
        let (x, y) = (num(&x).floor() as i32, num(&y).floor() as i32);
        let inside = s.sel.as_ref().is_none_or(|m| m.get(x, y) >= 128);
        if inside {
            let active = s.active;
            s.layers[active].1.set(x, y, c.rgba());
        }
    });
    let st = state.clone();
    e.register_fn("blendPixel", move |x: Dynamic, y: Dynamic, c: Color| {
        let mut s = st.borrow_mut();
        let (x, y) = (num(&x).floor() as i32, num(&y).floor() as i32);
        if x < 0 || y < 0 || x >= s.w as i32 || y >= s.h as i32 {
            return;
        }
        let k = s.sel.as_ref().map_or(1.0, |m| m.getf(x, y));
        let c = c.rgba();
        let active = s.active;
        let img = &mut s.layers[active].1;
        let i = img.idx(x, y);
        img.px[i] = blend_color(img.px[i], [c[0] as f32, c[1] as f32, c[2] as f32], c[3] as f32 / 255.0 * k);
    });
    let st = state.clone();
    e.register_fn("clear", move || {
        let mut s = st.borrow_mut();
        let (w, h, active) = (s.w, s.h, s.active);
        let sel = s.sel.clone();
        let img = &mut s.layers[active].1;
        match sel {
            None => *img = Image::new(w, h),
            Some(m) => {
                for (p, k) in img.px.iter_mut().zip(&m.data) {
                    p[3] = ((p[3] as u16 * (255 - *k as u16)) / 255) as u8;
                }
            }
        }
    });
    let st = state.clone();
    e.register_fn("fill", move |c: Color| {
        let mut s = st.borrow_mut();
        let r = IRect::from_size(s.w, s.h);
        s.cov.data.fill(1.0);
        s.paint(r, c.rgba());
    });

    // Shapes
    let st = state.clone();
    e.register_fn(
        "drawLine",
        move |x1: Dynamic, y1: Dynamic, x2: Dynamic, y2: Dynamic, c: Color, t: Dynamic| {
            let mut s = st.borrow_mut();
            let r = s.cov.segment(pos2(num(&x1), num(&y1)), pos2(num(&x2), num(&y2)), num(&t).max(1.0), true);
            s.paint(r, c.rgba());
        },
    );
    let st = state.clone();
    e.register_fn("drawLine", move |x1: Dynamic, y1: Dynamic, x2: Dynamic, y2: Dynamic, c: Color| {
        let mut s = st.borrow_mut();
        let r = s.cov.segment(pos2(num(&x1), num(&y1)), pos2(num(&x2), num(&y2)), 1.0, true);
        s.paint(r, c.rgba());
    });
    let st = state.clone();
    e.register_fn("drawRect", move |x: Dynamic, y: Dynamic, w: Dynamic, h: Dynamic, c: Color, t: Dynamic| {
        let mut s = st.borrow_mut();
        let (x, y, w, h) = (num(&x), num(&y), num(&w), num(&h));
        let t = num(&t).max(1.0);
        // Stroke inside the rect like qpaint.
        let i = t / 2.0;
        let pts = [pos2(x + i, y + i), pos2(x + w - i, y + i), pos2(x + w - i, y + h - i), pos2(x + i, y + h - i)];
        let r = s.cov.polyline(&pts, true, t, LineStyle::Solid, false);
        s.paint(r, c.rgba());
    });
    let st = state.clone();
    e.register_fn("fillRect", move |x: Dynamic, y: Dynamic, w: Dynamic, h: Dynamic, c: Color| {
        let mut s = st.borrow_mut();
        let (x, y, w, h) = (num(&x), num(&y), num(&w), num(&h));
        let r = s.cov.fill_polygon(&[vec![pos2(x, y), pos2(x + w, y), pos2(x + w, y + h), pos2(x, y + h)]], true);
        s.paint(r, c.rgba());
    });
    let st = state.clone();
    e.register_fn("drawEllipse", move |x: Dynamic, y: Dynamic, rx: Dynamic, ry: Dynamic, c: Color, t: Dynamic| {
        let mut s = st.borrow_mut();
        let pts = ellipse_points(pos2(num(&x), num(&y)), num(&rx), num(&ry));
        let r = s.cov.polyline(&pts, true, num(&t).max(1.0), LineStyle::Solid, true);
        s.paint(r, c.rgba());
    });
    let st = state.clone();
    e.register_fn("fillEllipse", move |x: Dynamic, y: Dynamic, rx: Dynamic, ry: Dynamic, c: Color| {
        let mut s = st.borrow_mut();
        let pts = ellipse_points(pos2(num(&x), num(&y)), num(&rx), num(&ry));
        let r = s.cov.fill_polygon(&[pts], true);
        s.paint(r, c.rgba());
    });
    let st = state.clone();
    e.register_fn("drawCircle", move |x: Dynamic, y: Dynamic, rad: Dynamic, c: Color, t: Dynamic| {
        let mut s = st.borrow_mut();
        let pts = ellipse_points(pos2(num(&x), num(&y)), num(&rad), num(&rad));
        let r = s.cov.polyline(&pts, true, num(&t).max(1.0), LineStyle::Solid, true);
        s.paint(r, c.rgba());
    });
    let st = state.clone();
    e.register_fn("fillCircle", move |x: Dynamic, y: Dynamic, rad: Dynamic, c: Color| {
        let mut s = st.borrow_mut();
        let pts = ellipse_points(pos2(num(&x), num(&y)), num(&rad), num(&rad));
        let r = s.cov.fill_polygon(&[pts], true);
        s.paint(r, c.rgba());
    });
    let st = state.clone();
    e.register_fn("fillPolygon", move |pts: rhai::Array, c: Color| {
        let mut s = st.borrow_mut();
        let poly: Vec<egui::Pos2> = pts
            .iter()
            .filter_map(|p| {
                let a = p.clone().try_cast::<rhai::Array>()?;
                Some(pos2(num(a.first()?), num(a.get(1)?)))
            })
            .collect();
        let r = s.cov.fill_polygon(&[poly], true);
        s.paint(r, c.rgba());
    });
    for (name, align) in [("drawText", crate::tools::TextAlign::Left), ("drawTextCentered", crate::tools::TextAlign::Center)] {
        let st = state.clone();
        e.register_fn(name, move |x: Dynamic, y: Dynamic, text: &str, size: Dynamic, c: Color| {
            let mut s = st.borrow_mut();
            let Some(font) = s.font.clone() else { return };
            let r = crate::text::rasterize(&mut s.cov, &font, text, num(&size).max(1.0), 1.2, align, pos2(num(&x), num(&y)));
            s.paint(r, c.rgba());
        });
    }

    // Layers
    let st = state.clone();
    e.register_fn("getLayerCount", move || st.borrow().layers.len() as i64);
    let st = state.clone();
    e.register_fn("getActiveLayerIndex", move || st.borrow().active as i64);
    let st = state.clone();
    e.register_fn("setActiveLayerIndex", move |i: i64| -> Result<(), Box<EvalAltResult>> {
        let mut s = st.borrow_mut();
        if i < 0 || i as usize >= s.layers.len() {
            return Err(format!("layer index {i} out of range").into());
        }
        s.active = i as usize;
        Ok(())
    });
    let st = state.clone();
    e.register_fn("addLayer", move |name: &str| {
        let mut s = st.borrow_mut();
        let (w, h) = (s.w, s.h);
        s.layers.push((name.to_string(), Image::new(w, h)));
        s.active = s.layers.len() - 1;
        s.active as i64
    });

    // Selection
    let st = state.clone();
    e.register_fn("isInSelection", move |x: Dynamic, y: Dynamic| {
        let s = st.borrow();
        s.sel.as_ref().is_some_and(|m| m.get(num(&x).floor() as i32, num(&y).floor() as i32) >= 128)
    });
    let st = state.clone();
    e.register_fn("hasSelection", move || st.borrow().sel.is_some());
    let st = state.clone();
    e.register_fn("getSelectionCenter", move || {
        let s = st.borrow();
        let b = s.sel.as_ref().and_then(|m| m.bounds()).unwrap_or(IRect::from_size(s.w, s.h));
        let mut m = Map::new();
        m.insert("x".into(), Dynamic::from_float((b.x0 + b.x1) as f64 / 2.0));
        m.insert("y".into(), Dynamic::from_float((b.y0 + b.y1) as f64 / 2.0));
        m
    });
    let st = state.clone();
    e.register_fn("selectRect", move |x: Dynamic, y: Dynamic, w: Dynamic, h: Dynamic| {
        let mut s = st.borrow_mut();
        let mut m = Mask::new(s.w, s.h);
        let r = IRect::new(num(&x) as i32, num(&y) as i32, (num(&x) + num(&w)) as i32, (num(&y) + num(&h)) as i32)
            .clamp_to(s.w, s.h);
        for yy in r.y0..r.y1 {
            for xx in r.x0..r.x1 {
                m.data[yy as usize * s.w as usize + xx as usize] = 255;
            }
        }
        s.sel = (!r.is_empty()).then_some(m);
    });
    let st = state.clone();
    e.register_fn("selectAll", move || {
        let mut s = st.borrow_mut();
        s.sel = Some(Mask::full(s.w, s.h));
    });
    let st = state.clone();
    e.register_fn("deselect", move || st.borrow_mut().sel = None);

    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_run() {
        let fonts = crate::text::FontStore::default();
        for (name, code) in EXAMPLES {
            let mut doc = Document::new(64, 48, Some([255, 255, 255, 255]));
            let mut h = History::default();
            let r = run(code, &mut doc, &mut h, &fonts, ([0, 0, 0, 255], [255, 255, 255, 255]));
            assert!(r.ok, "{name}: {}", r.output);
            assert!(h.can_undo(), "{name} changed nothing");
        }
    }

    #[test]
    fn qpaint_style_script() {
        let fonts = crate::text::FontStore::default();
        let mut doc = Document::new(10, 10, Some([255, 255, 255, 255]));
        let mut h = History::default();
        let code = "let c = Color(255, 0, 0); fillRect(0, 0, 5, 5, c); setPixel(9, 9, Color(0, 0, 255, 255)); print(getLayerCount());";
        let r = run(code, &mut doc, &mut h, &fonts, ([0; 4], [0; 4]));
        assert!(r.ok, "{}", r.output);
        assert_eq!(r.output.trim(), "1");
        assert_eq!(doc.layers[0].img.get(2, 2), [255, 0, 0, 255]);
        assert_eq!(doc.layers[0].img.get(9, 9), [0, 0, 255, 255]);
    }
}
