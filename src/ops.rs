//! Whole-image geometric operations used by the Image and Layer menus.

use crate::raster::{Image, Mask};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ResampleMethod {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos,
}

impl ResampleMethod {
    pub const ALL: [ResampleMethod; 4] =
        [ResampleMethod::Nearest, ResampleMethod::Bilinear, ResampleMethod::Bicubic, ResampleMethod::Lanczos];

    fn filter(self) -> image::imageops::FilterType {
        match self {
            ResampleMethod::Nearest => image::imageops::FilterType::Nearest,
            ResampleMethod::Bilinear => image::imageops::FilterType::Triangle,
            ResampleMethod::Bicubic => image::imageops::FilterType::CatmullRom,
            ResampleMethod::Lanczos => image::imageops::FilterType::Lanczos3,
        }
    }
}

pub fn rotate90(img: &Image, clockwise: bool) -> Image {
    let (w, h) = (img.w as i32, img.h as i32);
    let mut out = Image::new(img.h, img.w);
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = if clockwise { (h - 1 - y, x) } else { (y, w - 1 - x) };
            out.set(nx, ny, img.get(x, y));
        }
    }
    out
}

pub fn rotate180(img: &Image) -> Image {
    let mut out = img.clone();
    out.px.reverse();
    out
}

pub fn flip(img: &Image, horizontal: bool) -> Image {
    let mut out = img.clone();
    let w = img.w as usize;
    if horizontal {
        for row in out.px.chunks_mut(w) {
            row.reverse();
        }
    } else {
        for y in 0..img.h as usize {
            let src = (img.h as usize - 1 - y) * w;
            out.px[y * w..(y + 1) * w].copy_from_slice(&img.px[src..src + w]);
        }
    }
    out
}

/// Change canvas size without scaling; content is offset by (ox, oy).
pub fn resize_canvas(img: &Image, w: u32, h: u32, ox: i32, oy: i32) -> Image {
    let mut out = Image::new(w, h);
    out.blit(img, ox, oy);
    out
}

/// Resample in premultiplied space to avoid dark halos at transparent edges.
pub fn resample(img: &Image, w: u32, h: u32, method: ResampleMethod) -> Image {
    if method == ResampleMethod::Nearest {
        let mut out = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let sx = ((x as f32 + 0.5) * img.w as f32 / w as f32) as i32;
                let sy = ((y as f32 + 0.5) * img.h as f32 / h as f32) as i32;
                out.set(x as i32, y as i32, img.get(sx, sy));
            }
        }
        return out;
    }
    let pre: Vec<f32> = img
        .px
        .iter()
        .flat_map(|p| {
            // The image crate clamps float pixels to 0..1, so stay in that range.
            let a = p[3] as f32 / 255.0;
            [p[0] as f32 / 255.0 * a, p[1] as f32 / 255.0 * a, p[2] as f32 / 255.0 * a, a]
        })
        .collect();
    let buf = image::ImageBuffer::<image::Rgba<f32>, Vec<f32>>::from_raw(img.w, img.h, pre).expect("buffer");
    let scaled = image::imageops::resize(&buf, w, h, method.filter());
    let mut out = Image::new(w, h);
    for (o, p) in out.px.iter_mut().zip(scaled.pixels()) {
        let a = p[3].clamp(0.0, 1.0);
        *o = if a > 1e-4 {
            let k = 255.0 / a;
            [
                (p[0] * k).round().clamp(0.0, 255.0) as u8,
                (p[1] * k).round().clamp(0.0, 255.0) as u8,
                (p[2] * k).round().clamp(0.0, 255.0) as u8,
                (a * 255.0).round() as u8,
            ]
        } else {
            [0, 0, 0, 0]
        };
    }
    out
}

/// Apply an image operation to a mask by round-tripping through the alpha channel.
pub fn map_mask(m: &Mask, f: impl Fn(&Image) -> Image) -> Mask {
    let img = Image { w: m.w, h: m.h, px: m.data.iter().map(|&v| [255, 255, 255, v]).collect() };
    let out = f(&img);
    Mask { w: out.w, h: out.h, data: out.px.iter().map(|p| p[3]).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Image {
        let mut img = Image::new(3, 2);
        img.set(0, 0, [1, 0, 0, 255]);
        img.set(2, 1, [2, 0, 0, 255]);
        img
    }

    #[test]
    fn rotations() {
        let img = sample();
        let cw = rotate90(&img, true);
        assert_eq!((cw.w, cw.h), (2, 3));
        assert_eq!(cw.get(1, 0), [1, 0, 0, 255]);
        let back = rotate90(&cw, false);
        assert!(back == img);
        assert!(rotate180(&rotate180(&img)) == img);
    }

    #[test]
    fn flips() {
        let img = sample();
        assert_eq!(flip(&img, true).get(2, 0), [1, 0, 0, 255]);
        assert_eq!(flip(&img, false).get(0, 1), [1, 0, 0, 255]);
    }

    #[test]
    fn resample_keeps_color_at_edges() {
        let mut img = Image::new(4, 4);
        img.set(1, 1, [255, 0, 0, 255]);
        let out = resample(&img, 8, 8, ResampleMethod::Bilinear);
        // Semi-transparent edge pixels keep the pure red color (no dark halo).
        for p in &out.px {
            if p[3] > 0 {
                assert!(p[0] >= 250 && p[1] == 0, "{p:?}");
            }
        }
    }

    #[test]
    fn resample_keeps_opaque_images_opaque() {
        let img = Image::filled(8, 8, [200, 100, 50, 255]);
        for m in [ResampleMethod::Bilinear, ResampleMethod::Bicubic, ResampleMethod::Lanczos] {
            for (w, h) in [(4, 4), (16, 16), (5, 11)] {
                let out = resample(&img, w, h, m);
                assert!(out.px.iter().all(|p| *p == [200, 100, 50, 255]), "{m:?} {w}x{h}: {:?}", out.px[0]);
            }
        }
    }
}
