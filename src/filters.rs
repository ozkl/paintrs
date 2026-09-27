//! Image adjustments and filters. Each works on the active layer (limited to
//! the selection) and is previewed live before being applied.

use crate::raster::{IRect, Image, Rgba, box_blur_channel, hsv_to_rgb, rgb_to_hsv};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    Invert,
    Desaturate,
    Sepia,
    BrightnessContrast { brightness: f32, contrast: f32 },
    HueSaturation { hue: f32, saturation: f32, lightness: f32 },
    Posterize { levels: u32 },
    Threshold { level: f32 },
    Blur { radius: f32 },
    Sharpen { amount: f32, radius: f32 },
    Pixelate { size: u32 },
    Noise { amount: f32, mono: bool },
    Outline { radius: f32 },
}

impl Filter {
    pub fn name(&self) -> &'static str {
        match self {
            Filter::Invert => "Invert",
            Filter::Desaturate => "Desaturate",
            Filter::Sepia => "Sepia",
            Filter::BrightnessContrast { .. } => "Brightness / Contrast",
            Filter::HueSaturation { .. } => "Hue / Saturation",
            Filter::Posterize { .. } => "Posterize",
            Filter::Threshold { .. } => "Threshold",
            Filter::Blur { .. } => "Gaussian Blur",
            Filter::Sharpen { .. } => "Sharpen",
            Filter::Pixelate { .. } => "Pixelate",
            Filter::Noise { .. } => "Add Noise",
            Filter::Outline { .. } => "Edge Detect",
        }
    }

    pub fn has_params(&self) -> bool {
        !matches!(self, Filter::Invert | Filter::Desaturate | Filter::Sepia)
    }

    /// Parameter editor; returns true when something changed.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut c = false;
        let mut slider = |ui: &mut egui::Ui, label: &str, v: &mut f32, r: std::ops::RangeInclusive<f32>| {
            c |= ui.add(egui::Slider::new(v, r).text(label)).changed();
        };
        match self {
            Filter::BrightnessContrast { brightness, contrast } => {
                slider(ui, "Brightness", brightness, -100.0..=100.0);
                slider(ui, "Contrast", contrast, -100.0..=100.0);
            }
            Filter::HueSaturation { hue, saturation, lightness } => {
                slider(ui, "Hue", hue, -180.0..=180.0);
                slider(ui, "Saturation", saturation, -100.0..=100.0);
                slider(ui, "Lightness", lightness, -100.0..=100.0);
            }
            Filter::Posterize { levels } => {
                c |= ui.add(egui::Slider::new(levels, 2..=32).text("Levels")).changed();
            }
            Filter::Threshold { level } => slider(ui, "Level", level, 0.0..=255.0),
            Filter::Blur { radius } => slider(ui, "Radius", radius, 0.5..=100.0),
            Filter::Sharpen { amount, radius } => {
                slider(ui, "Amount", amount, 0.0..=5.0);
                slider(ui, "Radius", radius, 0.5..=20.0);
            }
            Filter::Pixelate { size } => {
                c |= ui.add(egui::Slider::new(size, 2..=128).text("Cell size")).changed();
            }
            Filter::Noise { amount, mono } => {
                slider(ui, "Amount", amount, 0.0..=100.0);
                c |= ui.checkbox(mono, "Monochrome").changed();
            }
            Filter::Outline { radius } => slider(ui, "Radius", radius, 1.0..=10.0),
            _ => {}
        }
        c
    }
}

pub const MENU: &[Filter] = &[
    Filter::BrightnessContrast { brightness: 0.0, contrast: 0.0 },
    Filter::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 0.0 },
    Filter::Invert,
    Filter::Desaturate,
    Filter::Sepia,
    Filter::Posterize { levels: 6 },
    Filter::Threshold { level: 128.0 },
    Filter::Blur { radius: 4.0 },
    Filter::Sharpen { amount: 1.0, radius: 2.0 },
    Filter::Pixelate { size: 8 },
    Filter::Noise { amount: 20.0, mono: false },
    Filter::Outline { radius: 1.0 },
];

fn map_pixels(src: &Image, rect: IRect, f: impl Fn(Rgba) -> Rgba) -> Image {
    let mut out = src.crop(rect);
    for p in &mut out.px {
        if p[3] != 0 {
            *p = f(*p);
        }
    }
    out
}

fn clamp8(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// Filter `rect` of `src`; returns an image of the rect's size.
pub fn apply(src: &Image, rect: IRect, f: Filter) -> Image {
    match f {
        Filter::Invert => map_pixels(src, rect, |p| [255 - p[0], 255 - p[1], 255 - p[2], p[3]]),
        Filter::Desaturate => map_pixels(src, rect, |p| {
            let l = clamp8(0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32);
            [l, l, l, p[3]]
        }),
        Filter::Sepia => map_pixels(src, rect, |p| {
            let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
            [
                clamp8(0.393 * r + 0.769 * g + 0.189 * b),
                clamp8(0.349 * r + 0.686 * g + 0.168 * b),
                clamp8(0.272 * r + 0.534 * g + 0.131 * b),
                p[3],
            ]
        }),
        Filter::BrightnessContrast { brightness, contrast } => {
            let b = brightness * 2.55;
            let c = (contrast / 100.0 + 1.0).max(0.0);
            let c = c * c;
            map_pixels(src, rect, move |p| {
                let m = |v: u8| clamp8((v as f32 + b - 128.0) * c + 128.0);
                [m(p[0]), m(p[1]), m(p[2]), p[3]]
            })
        }
        Filter::HueSaturation { hue, saturation, lightness } => map_pixels(src, rect, move |p| {
            let [h, s, v] = rgb_to_hsv(p);
            let s = (s * (1.0 + saturation / 100.0)).clamp(0.0, 1.0);
            let rgb = hsv_to_rgb(h + hue, s, v);
            let l = lightness / 100.0;
            let adj = |c: u8| {
                if l >= 0.0 {
                    clamp8(c as f32 + (255.0 - c as f32) * l)
                } else {
                    clamp8(c as f32 * (1.0 + l))
                }
            };
            [adj(rgb[0]), adj(rgb[1]), adj(rgb[2]), p[3]]
        }),
        Filter::Posterize { levels } => {
            let n = (levels.max(2) - 1) as f32;
            map_pixels(src, rect, move |p| {
                let q = |v: u8| clamp8((v as f32 / 255.0 * n).round() / n * 255.0);
                [q(p[0]), q(p[1]), q(p[2]), p[3]]
            })
        }
        Filter::Threshold { level } => map_pixels(src, rect, move |p| {
            let l = 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32;
            let v = if l >= level { 255 } else { 0 };
            [v, v, v, p[3]]
        }),
        Filter::Noise { amount, mono } => {
            let a = amount * 2.55;
            let mut out = src.crop(rect);
            for y in 0..out.h as i32 {
                for x in 0..out.w as i32 {
                    let i = out.idx(x, y);
                    let p = out.px[i];
                    if p[3] == 0 {
                        continue;
                    }
                    let seed = ((x + rect.x0) as u32).wrapping_mul(73856093) ^ ((y + rect.y0) as u32).wrapping_mul(19349663);
                    let n = |k: u32| (hash(seed.wrapping_add(k)) - 0.5) * 2.0 * a;
                    let (n0, n1, n2) = if mono { let v = n(0); (v, v, v) } else { (n(0), n(1), n(2)) };
                    out.px[i] = [clamp8(p[0] as f32 + n0), clamp8(p[1] as f32 + n1), clamp8(p[2] as f32 + n2), p[3]];
                }
            }
            out
        }
        Filter::Pixelate { size } => {
            let size = size.max(1) as i32;
            let mut out = src.crop(rect);
            // Cells aligned to the canvas grid so partial selections line up.
            let gx0 = rect.x0.div_euclid(size) * size;
            let gy0 = rect.y0.div_euclid(size) * size;
            let mut cy = gy0;
            while cy < rect.y1 {
                let mut cx = gx0;
                while cx < rect.x1 {
                    let cell = IRect::new(cx, cy, cx + size, cy + size).clamp_to(src.w, src.h);
                    let mut acc = [0.0f32; 4];
                    for y in cell.y0..cell.y1 {
                        for x in cell.x0..cell.x1 {
                            let p = src.get(x, y);
                            let a = p[3] as f32;
                            acc[0] += p[0] as f32 * a;
                            acc[1] += p[1] as f32 * a;
                            acc[2] += p[2] as f32 * a;
                            acc[3] += a;
                        }
                    }
                    let n = cell.area().max(1) as f32;
                    let c = if acc[3] > 0.0 {
                        [clamp8(acc[0] / acc[3]), clamp8(acc[1] / acc[3]), clamp8(acc[2] / acc[3]), clamp8(acc[3] / n)]
                    } else {
                        [0, 0, 0, 0]
                    };
                    let target = cell.intersect(&rect);
                    for y in target.y0..target.y1 {
                        for x in target.x0..target.x1 {
                            let i = out.idx(x - rect.x0, y - rect.y0);
                            out.px[i] = c;
                        }
                    }
                    cx += size;
                }
                cy += size;
            }
            out
        }
        Filter::Blur { radius } => blur(src, rect, radius),
        Filter::Sharpen { amount, radius } => {
            let blurred = blur(src, rect, radius);
            let mut out = src.crop(rect);
            for (o, b) in out.px.iter_mut().zip(&blurred.px) {
                for c in 0..3 {
                    o[c] = clamp8(o[c] as f32 + (o[c] as f32 - b[c] as f32) * amount);
                }
            }
            out
        }
        Filter::Outline { radius } => {
            // Sobel magnitude on luminance, drawn dark on white.
            let r = radius.round().max(1.0) as i32;
            let lum = |x: i32, y: i32| {
                let p = src.get(x.clamp(0, src.w as i32 - 1), y.clamp(0, src.h as i32 - 1));
                (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) * p[3] as f32 / 255.0
            };
            let mut out = src.crop(rect);
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    let gx = lum(x + r, y - r) + 2.0 * lum(x + r, y) + lum(x + r, y + r)
                        - lum(x - r, y - r)
                        - 2.0 * lum(x - r, y)
                        - lum(x - r, y + r);
                    let gy = lum(x - r, y + r) + 2.0 * lum(x, y + r) + lum(x + r, y + r)
                        - lum(x - r, y - r)
                        - 2.0 * lum(x, y - r)
                        - lum(x + r, y - r);
                    let m = clamp8(255.0 - (gx * gx + gy * gy).sqrt());
                    let i = out.idx(x - rect.x0, y - rect.y0);
                    out.px[i] = [m, m, m, 255];
                }
            }
            out
        }
    }
}

fn hash(mut x: u32) -> f32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;
    (x & 0xffffff) as f32 / 0xffffff as f32
}

/// Approximate gaussian blur (3 box passes) in premultiplied space.
fn blur(src: &Image, rect: IRect, radius: f32) -> Image {
    let r = (radius * 0.58).round().max(1.0) as usize;
    let pad = (r * 3 + 1) as i32;
    let area = rect.expand(pad).clamp_to(src.w, src.h);
    let crop = src.crop(area);
    let (w, h) = (crop.w as usize, crop.h as usize);
    let mut ch: Vec<Vec<f32>> = vec![vec![0.0; w * h]; 4];
    for (i, p) in crop.px.iter().enumerate() {
        let a = p[3] as f32 / 255.0;
        ch[0][i] = p[0] as f32 * a;
        ch[1][i] = p[1] as f32 * a;
        ch[2][i] = p[2] as f32 * a;
        ch[3][i] = p[3] as f32;
    }
    for c in &mut ch {
        box_blur_channel(c, w, h, r, 3);
    }
    let mut out = Image::new(rect.width() as u32, rect.height() as u32);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let i = (y - area.y0) as usize * w + (x - area.x0) as usize;
            let a = ch[3][i];
            let o = out.idx(x - rect.x0, y - rect.y0);
            out.px[o] = if a > 0.01 {
                let k = 255.0 / a;
                [clamp8(ch[0][i] * k), clamp8(ch[1][i] * k), clamp8(ch[2][i] * k), clamp8(a)]
            } else {
                [0, 0, 0, 0]
            };
        }
    }
    out
}
