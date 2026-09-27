//! Image import/export, the layered `.paintrs` project format and import of
//! legacy qpaint `.qpif` files.

use std::io::Cursor;

use serde::{Deserialize, Serialize};

use crate::doc::{BlendMode, Document};
use crate::raster::{Image, Mask};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    Png,
    Jpeg,
    Webp,
    Bmp,
    Tga,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 5] =
        [ExportFormat::Png, ExportFormat::Jpeg, ExportFormat::Webp, ExportFormat::Bmp, ExportFormat::Tga];

    pub fn ext(self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Jpeg => "jpg",
            ExportFormat::Webp => "webp",
            ExportFormat::Bmp => "bmp",
            ExportFormat::Tga => "tga",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ExportFormat::Png => "PNG",
            ExportFormat::Jpeg => "JPEG",
            ExportFormat::Webp => "WebP (lossless)",
            ExportFormat::Bmp => "BMP",
            ExportFormat::Tga => "TGA",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            ExportFormat::Png => "image/png",
            ExportFormat::Jpeg => "image/jpeg",
            ExportFormat::Webp => "image/webp",
            ExportFormat::Bmp => "image/bmp",
            ExportFormat::Tga => "image/x-tga",
        }
    }
}

pub const PROJECT_EXT: &str = "paintrs";
pub const OPEN_EXTS: &[&str] = &["paintrs", "qpif", "png", "jpg", "jpeg", "webp", "bmp", "tga", "gif", "ico"];

pub fn decode_image(bytes: &[u8]) -> Result<Image, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("Can't decode image: {e}"))?;
    let rgba = img.to_rgba8();
    Ok(Image::from_rgba8(rgba.width(), rgba.height(), rgba.as_raw()))
}

pub fn encode_image(img: &Image, fmt: ExportFormat, jpeg_quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    let buf = image::RgbaImage::from_raw(img.w, img.h, img.as_bytes().to_vec()).ok_or("bad image buffer")?;
    let res = match fmt {
        ExportFormat::Jpeg => {
            // JPEG has no alpha: composite onto white.
            let rgb = image::RgbImage::from_fn(img.w, img.h, |x, y| {
                let p = img.px[img.idx(x as i32, y as i32)];
                let a = p[3] as f32 / 255.0;
                let ch = |c: u8| (c as f32 * a + 255.0 * (1.0 - a)).round() as u8;
                image::Rgb([ch(p[0]), ch(p[1]), ch(p[2])])
            });
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, jpeg_quality.clamp(1, 100));
            rgb.write_with_encoder(enc)
        }
        ExportFormat::Png => buf.write_to(&mut out, image::ImageFormat::Png),
        ExportFormat::Webp => buf.write_to(&mut out, image::ImageFormat::WebP),
        ExportFormat::Bmp => buf.write_to(&mut out, image::ImageFormat::Bmp),
        ExportFormat::Tga => buf.write_to(&mut out, image::ImageFormat::Tga),
    };
    res.map_err(|e| format!("Encoding failed: {e}"))?;
    Ok(out.into_inner())
}

// ---------------------------------------------------------------------------
// .paintrs project format
//
//   "PAINTRS\0"  magic (8 bytes)
//   u32 LE       format version (1)
//   u32 LE       JSON metadata length
//   JSON         ProjectMeta
//   blobs        PNG data for each layer, then the optional selection mask,
//                lengths are in the metadata.

const MAGIC: &[u8; 8] = b"PAINTRS\0";

#[derive(Serialize, Deserialize)]
struct ProjectMeta {
    width: u32,
    height: u32,
    active: usize,
    layers: Vec<LayerMeta>,
    selection_len: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct LayerMeta {
    name: String,
    visible: bool,
    opacity: f32,
    blend: BlendMode,
    locked: bool,
    alpha_lock: bool,
    png_len: u64,
}

pub fn save_project(doc: &Document) -> Result<Vec<u8>, String> {
    let mut blobs = Vec::new();
    let mut layers = Vec::new();
    for l in &doc.layers {
        let png = encode_image(&l.img, ExportFormat::Png, 100)?;
        layers.push(LayerMeta {
            name: l.name.clone(),
            visible: l.visible,
            opacity: l.opacity,
            blend: l.blend,
            locked: l.locked,
            alpha_lock: l.alpha_lock,
            png_len: png.len() as u64,
        });
        blobs.push(png);
    }
    let selection_len = if let Some(m) = &doc.selection {
        let gray = image::GrayImage::from_raw(m.w, m.h, m.data.clone()).ok_or("bad mask")?;
        let mut out = Cursor::new(Vec::new());
        gray.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
        let v = out.into_inner();
        let len = v.len() as u64;
        blobs.push(v);
        Some(len)
    } else {
        None
    };
    let meta = ProjectMeta { width: doc.w, height: doc.h, active: doc.active, layers, selection_len };
    let json = serde_json::to_vec(&meta).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    for b in blobs {
        out.extend_from_slice(&b);
    }
    Ok(out)
}

pub fn is_project(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

pub fn load_project(bytes: &[u8]) -> Result<Document, String> {
    let err = || "Corrupt project file".to_string();
    if !is_project(bytes) || bytes.len() < 16 {
        return Err(err());
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if version != 1 {
        return Err(format!("Unsupported project version {version}"));
    }
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let json = bytes.get(16..16 + json_len).ok_or_else(err)?;
    let meta: ProjectMeta = serde_json::from_slice(json).map_err(|e| format!("Bad project metadata: {e}"))?;
    let mut pos = 16 + json_len;
    let mut take = |len: u64| -> Result<&[u8], String> {
        let s = bytes.get(pos..pos + len as usize).ok_or_else(err)?;
        pos += len as usize;
        Ok(s)
    };
    let mut doc = Document::new(meta.width, meta.height, None);
    doc.layers.clear();
    for lm in &meta.layers {
        let mut img = decode_image(take(lm.png_len)?)?;
        if img.w != meta.width || img.h != meta.height {
            let mut fixed = Image::new(meta.width, meta.height);
            fixed.blit(&img, 0, 0);
            img = fixed;
        }
        let mut l = doc.make_layer(&lm.name, img);
        l.visible = lm.visible;
        l.opacity = lm.opacity.clamp(0.0, 1.0);
        l.blend = lm.blend;
        l.locked = lm.locked;
        l.alpha_lock = lm.alpha_lock;
        doc.layers.push(l);
    }
    if doc.layers.is_empty() {
        return Err("Project has no layers".into());
    }
    if let Some(len) = meta.selection_len
        && let Ok(g) = image::load_from_memory(take(len)?) {
            let g = g.to_luma8();
            if g.width() == meta.width && g.height() == meta.height {
                doc.selection = Some(Mask { w: meta.width, h: meta.height, data: g.into_raw() });
            }
        }
    doc.active = meta.active.min(doc.layers.len() - 1);
    Ok(doc)
}

// ---------------------------------------------------------------------------
// Legacy qpaint QPIF import (52-byte header, 64-byte layer entries, PNGs).

pub fn is_qpif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"QPIF")
}

pub fn load_qpif(bytes: &[u8]) -> Result<Document, String> {
    let u32_at = |o: usize| -> Result<u32, String> {
        Ok(u32::from_le_bytes(bytes.get(o..o + 4).ok_or("Truncated QPIF")?.try_into().unwrap()))
    };
    if !is_qpif(bytes) {
        return Err("Not a QPIF file".into());
    }
    if u32_at(4)? != 1 {
        return Err("Unsupported QPIF version".into());
    }
    let (w, h, count) = (u32_at(40)?, u32_at(44)?, u32_at(48)? as usize);
    if w == 0 || h == 0 || w > 32768 || h > 32768 || count > 4096 {
        return Err("Invalid QPIF header".into());
    }
    let table = 52;
    let data_start = table + 64 * count;
    let offsets: Vec<usize> = (0..count).map(|i| u32_at(table + 64 * i).map(|v| v as usize)).collect::<Result<_, _>>()?;
    let mut layers = Vec::new();
    for i in 0..count {
        let start = data_start + offsets[i];
        let end = if i + 1 < count { data_start + offsets[i + 1] } else { bytes.len() };
        let png = bytes.get(start..end).ok_or("Truncated QPIF layer")?;
        let img = decode_image(png)?;
        let mut fixed = Image::new(w, h);
        fixed.blit(&img, 0, 0);
        layers.push((format!("L{i}"), fixed));
    }
    Ok(Document::from_layers(w, h, layers))
}

/// Result of opening a file.
pub enum Opened {
    Document(Document),
    Image(Image),
}

pub fn open_bytes(name: &str, bytes: &[u8]) -> Result<Opened, String> {
    if is_project(bytes) {
        let mut d = load_project(bytes)?;
        d.name = stem(name);
        return Ok(Opened::Document(d));
    }
    if is_qpif(bytes) {
        let mut d = load_qpif(bytes)?;
        d.name = stem(name);
        return Ok(Opened::Document(d));
    }
    decode_image(bytes).map(Opened::Image)
}

pub fn stem(name: &str) -> String {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file.rsplit_once('.').map_or(file, |(s, _)| s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_roundtrip() {
        let mut d = Document::new(8, 6, Some([10, 20, 30, 255]));
        let mut l = d.make_layer("Top", Image::filled(8, 6, [1, 2, 3, 4]));
        l.blend = BlendMode::Screen;
        l.opacity = 0.5;
        d.layers.push(l);
        d.selection = Some(Mask::full(8, 6));
        let bytes = save_project(&d).unwrap();
        let back = load_project(&bytes).unwrap();
        assert_eq!(back.layers.len(), 2);
        assert_eq!(back.layers[1].name, "Top");
        assert_eq!(back.layers[1].blend, BlendMode::Screen);
        assert_eq!(back.layers[1].img.get(3, 3), [1, 2, 3, 4]);
        assert_eq!(back.layers[0].img.get(0, 0), [10, 20, 30, 255]);
        assert!(back.selection.is_some());
    }

    #[test]
    fn qpif_import() {
        // Build a QPIF the way qpaint writes it.
        let layers = [Image::filled(4, 3, [255, 0, 0, 255]), Image::filled(4, 3, [0, 0, 255, 128])];
        let pngs: Vec<Vec<u8>> = layers.iter().map(|l| encode_image(l, ExportFormat::Png, 100).unwrap()).collect();
        let mut out = Vec::new();
        out.extend_from_slice(b"QPIF");
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 32]);
        for v in [4u32, 3, 2] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        let mut off = 0u32;
        for p in &pngs {
            out.extend_from_slice(&off.to_le_bytes());
            out.extend_from_slice(&[0u8; 60]);
            off += p.len() as u32;
        }
        for p in &pngs {
            out.extend_from_slice(p);
        }
        let d = load_qpif(&out).unwrap();
        assert_eq!((d.w, d.h, d.layers.len()), (4, 3, 2));
        assert_eq!(d.layers[1].img.get(1, 1), [0, 0, 255, 128]);
    }

    #[test]
    fn encode_all_formats() {
        let img = Image::filled(5, 5, [200, 100, 50, 255]);
        for f in ExportFormat::ALL {
            let bytes = encode_image(&img, f, 90).unwrap();
            if f != ExportFormat::Tga {
                let back = decode_image(&bytes).unwrap();
                assert_eq!((back.w, back.h), (5, 5));
            }
        }
    }
}
