//! Pixel buffers and CPU rasterization primitives.
//!
//! All layer pixels are straight (non-premultiplied) sRGB RGBA8. Tools never
//! write pixels directly: they rasterize shapes into a [`Coverage`] buffer and
//! the edit session turns coverage into pixels. That keeps stroke opacity
//! uniform (no darkening where dabs overlap) and makes selection clipping,
//! alpha lock and erasing work identically for every tool.

use egui::{Pos2, pos2, vec2};

pub type Rgba = [u8; 4];

pub const TRANSPARENT: Rgba = [0, 0, 0, 0];

/// Integer rectangle, `x1`/`y1` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    pub const EMPTY: IRect = IRect { x0: 0, y0: 0, x1: 0, y1: 0 };

    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn from_size(w: u32, h: u32) -> Self {
        Self::new(0, 0, w as i32, h as i32)
    }

    /// Smallest integer rect containing the float bounds.
    pub fn around(min: Pos2, max: Pos2) -> Self {
        Self::new(
            min.x.floor() as i32,
            min.y.floor() as i32,
            max.x.ceil() as i32 + 1,
            max.y.ceil() as i32 + 1,
        )
    }

    pub fn width(&self) -> i32 {
        (self.x1 - self.x0).max(0)
    }

    pub fn height(&self) -> i32 {
        (self.y1 - self.y0).max(0)
    }

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn area(&self) -> usize {
        self.width() as usize * self.height() as usize
    }

    pub fn union(&self, o: &IRect) -> IRect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        IRect::new(self.x0.min(o.x0), self.y0.min(o.y0), self.x1.max(o.x1), self.y1.max(o.y1))
    }

    pub fn intersect(&self, o: &IRect) -> IRect {
        let r = IRect::new(self.x0.max(o.x0), self.y0.max(o.y0), self.x1.min(o.x1), self.y1.min(o.y1));
        if r.is_empty() { IRect::EMPTY } else { r }
    }

    pub fn expand(&self, d: i32) -> IRect {
        IRect::new(self.x0 - d, self.y0 - d, self.x1 + d, self.y1 + d)
    }

    pub fn clamp_to(&self, w: u32, h: u32) -> IRect {
        self.intersect(&IRect::from_size(w, h))
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

pub fn union_opt(a: Option<IRect>, b: IRect) -> Option<IRect> {
    if b.is_empty() {
        return a;
    }
    Some(match a {
        Some(a) => a.union(&b),
        None => b,
    })
}

/// RGBA8 image with straight alpha.
#[derive(Clone, PartialEq)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub px: Vec<Rgba>,
}

impl Image {
    pub fn new(w: u32, h: u32) -> Self {
        Self::filled(w, h, TRANSPARENT)
    }

    pub fn filled(w: u32, h: u32, c: Rgba) -> Self {
        Self { w, h, px: vec![c; w as usize * h as usize] }
    }

    pub fn from_rgba8(w: u32, h: u32, bytes: &[u8]) -> Self {
        Self { w, h, px: bytemuck::cast_slice(bytes).to_vec() }
    }

    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.px)
    }

    pub fn byte_size(&self) -> usize {
        self.px.len() * 4
    }

    #[inline]
    pub fn idx(&self, x: i32, y: i32) -> usize {
        y as usize * self.w as usize + x as usize
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Rgba {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            TRANSPARENT
        } else {
            self.px[self.idx(x, y)]
        }
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, c: Rgba) {
        if x >= 0 && y >= 0 && x < self.w as i32 && y < self.h as i32 {
            let i = self.idx(x, y);
            self.px[i] = c;
        }
    }

    /// Copy of a sub-rectangle (must lie inside the image).
    pub fn crop(&self, r: IRect) -> Image {
        let r = r.clamp_to(self.w, self.h);
        let mut out = Image::new(r.width() as u32, r.height() as u32);
        for y in r.y0..r.y1 {
            let src = self.idx(r.x0, y);
            let dst = ((y - r.y0) * r.width()) as usize;
            out.px[dst..dst + r.width() as usize].copy_from_slice(&self.px[src..src + r.width() as usize]);
        }
        out
    }

    /// Raw rows of a rect (used by history patches).
    pub fn read_rect(&self, r: IRect) -> Vec<Rgba> {
        self.crop(r).px
    }

    pub fn write_rect(&mut self, r: IRect, data: &[Rgba]) {
        let w = r.width() as usize;
        for (row, y) in (r.y0..r.y1).enumerate() {
            let dst = self.idx(r.x0, y);
            self.px[dst..dst + w].copy_from_slice(&data[row * w..row * w + w]);
        }
    }

    /// Place `src` with its top-left at (dx, dy), replacing pixels.
    pub fn blit(&mut self, src: &Image, dx: i32, dy: i32) {
        let target = IRect::new(dx, dy, dx + src.w as i32, dy + src.h as i32).clamp_to(self.w, self.h);
        for y in target.y0..target.y1 {
            for x in target.x0..target.x1 {
                let c = src.px[src.idx(x - dx, y - dy)];
                let i = self.idx(x, y);
                self.px[i] = c;
            }
        }
    }

    /// Bilinear sample with transparent outside (coordinates are pixel centers at +0.5).
    pub fn sample_bilinear(&self, x: f32, y: f32) -> [f32; 4] {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = fx.floor() as i32;
        let y0 = fy.floor() as i32;
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        // Interpolate premultiplied to avoid dark fringes.
        let mut acc = [0.0f32; 4];
        for (dx, dy, wgt) in [
            (0, 0, (1.0 - tx) * (1.0 - ty)),
            (1, 0, tx * (1.0 - ty)),
            (0, 1, (1.0 - tx) * ty),
            (1, 1, tx * ty),
        ] {
            if wgt <= 0.0 {
                continue;
            }
            let c = self.get(x0 + dx, y0 + dy);
            let a = c[3] as f32 / 255.0 * wgt;
            acc[0] += c[0] as f32 * a;
            acc[1] += c[1] as f32 * a;
            acc[2] += c[2] as f32 * a;
            acc[3] += a;
        }
        if acc[3] > 1e-6 {
            [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3] * 255.0]
        } else {
            [0.0; 4]
        }
    }

    /// Bounding box of pixels with non-zero alpha.
    pub fn opaque_bounds(&self) -> Option<IRect> {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                if self.px[self.idx(x, y)][3] != 0 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        (x0 < x1).then(|| IRect::new(x0, y0, x1, y1))
    }
}

/// Straight-alpha "source over".
#[inline]
pub fn over(dst: Rgba, src: Rgba) -> Rgba {
    if src[3] == 255 {
        return src;
    }
    if src[3] == 0 {
        return dst;
    }
    let sa = src[3] as f32 / 255.0;
    blend_color(dst, [src[0] as f32, src[1] as f32, src[2] as f32], sa)
}

/// Blend a straight color with alpha `sa` (0..1) over `dst`.
#[inline]
pub fn blend_color(dst: Rgba, src: [f32; 3], sa: f32) -> Rgba {
    if sa <= 0.0 {
        return dst;
    }
    let da = dst[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 1e-6 {
        return TRANSPARENT;
    }
    let k = da * (1.0 - sa);
    let ch = |i: usize| ((src[i] * sa + dst[i] as f32 * k) / oa).round().clamp(0.0, 255.0) as u8;
    [ch(0), ch(1), ch(2), (oa * 255.0).round() as u8]
}

#[inline]
pub fn lerp_rgba(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let mut o = [0u8; 4];
    for i in 0..4 {
        o[i] = (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round().clamp(0.0, 255.0) as u8;
    }
    o
}

/// Single-channel 8-bit mask, used for selections.
#[derive(Clone, PartialEq)]
pub struct Mask {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl Mask {
    pub fn new(w: u32, h: u32) -> Self {
        Self { w, h, data: vec![0; w as usize * h as usize] }
    }

    pub fn full(w: u32, h: u32) -> Self {
        Self { w, h, data: vec![255; w as usize * h as usize] }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            0
        } else {
            self.data[y as usize * self.w as usize + x as usize]
        }
    }

    #[inline]
    pub fn getf(&self, x: i32, y: i32) -> f32 {
        self.get(x, y) as f32 / 255.0
    }

    pub fn bounds(&self) -> Option<IRect> {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        let w = self.w as usize;
        for y in 0..self.h as usize {
            let row = &self.data[y * w..(y + 1) * w];
            if let Some(first) = row.iter().position(|&v| v != 0) {
                let last = row.iter().rposition(|&v| v != 0).unwrap();
                x0 = x0.min(first as i32);
                x1 = x1.max(last as i32 + 1);
                y0 = y0.min(y as i32);
                y1 = y1.max(y as i32 + 1);
            }
        }
        (x0 < x1).then(|| IRect::new(x0, y0, x1, y1))
    }

    pub fn is_empty(&self) -> bool {
        self.data.iter().all(|&v| v == 0)
    }

    pub fn invert(&mut self) {
        for v in &mut self.data {
            *v = 255 - *v;
        }
    }

    /// Combine a coverage buffer into this mask.
    pub fn combine(&mut self, cov: &Coverage, rect: IRect, op: SelectOp) {
        if op == SelectOp::Replace {
            self.data.fill(0);
        }
        let rect = rect.clamp_to(self.w, self.h);
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let i = y as usize * self.w as usize + x as usize;
                let c = (cov.data[i].clamp(0.0, 1.0) * 255.0).round() as u8;
                let m = &mut self.data[i];
                *m = match op {
                    SelectOp::Replace | SelectOp::Add => (*m).max(c),
                    SelectOp::Subtract => ((*m as u16 * (255 - c) as u16) / 255) as u8,
                    SelectOp::Intersect => ((*m as u16 * c as u16) / 255) as u8,
                };
            }
        }
        if op == SelectOp::Intersect {
            // Pixels outside the rect have zero coverage.
            for y in 0..self.h as i32 {
                for x in 0..self.w as i32 {
                    if !rect.contains(x, y) {
                        self.data[y as usize * self.w as usize + x as usize] = 0;
                    }
                }
            }
        }
    }

    /// Grow (positive) or shrink (negative) the selection by `r` pixels.
    pub fn morph(&mut self, r: i32) {
        if r == 0 {
            return;
        }
        let grow = r > 0;
        let r = r.unsigned_abs() as i32;
        let (w, h) = (self.w as i32, self.h as i32);
        // Separable square min/max filter, good enough for grow/shrink.
        let mut tmp = self.data.clone();
        for y in 0..h {
            for x in 0..w {
                let mut v = if grow { 0u8 } else { 255u8 };
                for dx in -r..=r {
                    let s = self.get(x + dx, y);
                    v = if grow { v.max(s) } else { v.min(if x + dx < 0 || x + dx >= w { 0 } else { s }) };
                }
                tmp[(y * w + x) as usize] = v;
            }
        }
        let src = Mask { w: self.w, h: self.h, data: tmp };
        for y in 0..h {
            for x in 0..w {
                let mut v = if grow { 0u8 } else { 255u8 };
                for dy in -r..=r {
                    let s = src.get(x, y + dy);
                    v = if grow { v.max(s) } else { v.min(if y + dy < 0 || y + dy >= h { 0 } else { s }) };
                }
                self.data[(y * w + x) as usize] = v;
            }
        }
    }

    /// Soften edges with a box blur of radius `r`.
    pub fn feather(&mut self, r: u32) {
        let mut f: Vec<f32> = self.data.iter().map(|&v| v as f32).collect();
        box_blur_channel(&mut f, self.w as usize, self.h as usize, r as usize, 3);
        for (d, s) in self.data.iter_mut().zip(f) {
            *d = s.round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SelectOp {
    Replace,
    Add,
    Subtract,
    Intersect,
}

/// Float coverage buffer the size of the canvas. Tools rasterize into it;
/// only the touched area is ever cleared.
pub struct Coverage {
    pub w: u32,
    pub h: u32,
    pub data: Vec<f32>,
}

impl Coverage {
    pub fn new(w: u32, h: u32) -> Self {
        Self { w, h, data: vec![0.0; w as usize * h as usize] }
    }

    pub fn ensure_size(&mut self, w: u32, h: u32) {
        if self.w != w || self.h != h {
            *self = Coverage::new(w, h);
        }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> f32 {
        self.data[y as usize * self.w as usize + x as usize]
    }

    pub fn clear(&mut self, r: IRect) {
        let r = r.clamp_to(self.w, self.h);
        for y in r.y0..r.y1 {
            let i = y as usize * self.w as usize;
            self.data[i + r.x0 as usize..i + r.x1 as usize].fill(0.0);
        }
    }

    #[inline]
    fn put_max(&mut self, x: i32, y: i32, v: f32) {
        let i = y as usize * self.w as usize + x as usize;
        if v > self.data[i] {
            self.data[i] = v;
        }
    }

    #[inline]
    fn put_flow(&mut self, x: i32, y: i32, v: f32) {
        let i = y as usize * self.w as usize + x as usize;
        let c = self.data[i];
        self.data[i] = c + (1.0 - c) * v;
    }

    /// A soft round brush dab. `hardness` 1 = crisp anti-aliased edge.
    /// Returns the touched rect.
    pub fn dab(&mut self, c: Pos2, radius: f32, hardness: f32, flow: f32) -> IRect {
        let radius = radius.max(0.5);
        let rect = IRect::around(c - vec2(radius + 1.0, radius + 1.0), c + vec2(radius + 1.0, radius + 1.0))
            .clamp_to(self.w, self.h);
        let inner = (hardness.clamp(0.0, 1.0) * radius).min(radius - 0.5).max(0.0);
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let d = ((x as f32 + 0.5 - c.x).powi(2) + (y as f32 + 0.5 - c.y).powi(2)).sqrt();
                let edge = (radius - d + 0.5).clamp(0.0, 1.0);
                if edge <= 0.0 {
                    continue;
                }
                let soft = if d <= inner {
                    1.0
                } else {
                    let t = ((d - inner) / (radius - inner).max(1e-3)).clamp(0.0, 1.0);
                    let s = 1.0 - t;
                    s * s * (3.0 - 2.0 * s)
                };
                let v = edge.min(soft) * flow;
                if flow >= 0.999 { self.put_max(x, y, v) } else { self.put_flow(x, y, v) }
            }
        }
        rect
    }

    /// Hard aliased square/round block for the pencil (pixel-perfect).
    pub fn pixel_block(&mut self, x: i32, y: i32, size: i32, round: bool) -> IRect {
        let half = (size - 1) / 2;
        let r = IRect::new(x - half, y - half, x - half + size, y - half + size);
        let clipped = r.clamp_to(self.w, self.h);
        let rad = size as f32 / 2.0;
        for py in clipped.y0..clipped.y1 {
            for px in clipped.x0..clipped.x1 {
                if round && size > 2 {
                    let dx = px as f32 + 0.5 - (r.x0 as f32 + rad);
                    let dy = py as f32 + 0.5 - (r.y0 as f32 + rad);
                    if dx * dx + dy * dy > rad * rad {
                        continue;
                    }
                }
                self.put_max(px, py, 1.0);
            }
        }
        clipped
    }

    /// Anti-aliased thick segment (capsule) with round caps.
    pub fn segment(&mut self, a: Pos2, b: Pos2, width: f32, aa: bool) -> IRect {
        let r = (width / 2.0).max(0.5);
        let rect = IRect::around(a.min(b) - vec2(r + 1.0, r + 1.0), a.max(b) + vec2(r + 1.0, r + 1.0))
            .clamp_to(self.w, self.h);
        let ab = b - a;
        let len2 = ab.length_sq();
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
                let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let d = (p - (a + ab * t)).length();
                let v = if aa { (r - d + 0.5).clamp(0.0, 1.0) } else if d <= r { 1.0 } else { 0.0 };
                if v > 0.0 {
                    self.put_max(x, y, v);
                }
            }
        }
        rect
    }

    /// Stroke a polyline with a dash pattern.
    pub fn polyline(&mut self, pts: &[Pos2], closed: bool, width: f32, style: LineStyle, aa: bool) -> IRect {
        let mut rect = IRect::EMPTY;
        if pts.is_empty() {
            return rect;
        }
        let mut segs: Vec<(Pos2, Pos2)> = pts.windows(2).map(|w| (w[0], w[1])).collect();
        if closed && pts.len() > 2 {
            segs.push((*pts.last().unwrap(), pts[0]));
        }
        if segs.is_empty() {
            return self.segment(pts[0], pts[0], width, aa);
        }
        let w = width.max(1.0);
        let mut dist = 0.0f32;
        match style {
            LineStyle::Solid => {
                for (a, b) in segs {
                    rect = rect.union(&self.segment(a, b, width, aa));
                }
            }
            LineStyle::Dotted => {
                let spacing = w * 2.0;
                let mut next = 0.0f32;
                for (a, b) in segs {
                    let len = (b - a).length();
                    let dir = if len > 0.0 { (b - a) / len } else { vec2(0.0, 0.0) };
                    while next <= dist + len {
                        let p = a + dir * (next - dist);
                        rect = rect.union(&self.segment(p, p, width, aa));
                        next += spacing;
                    }
                    dist += len;
                }
            }
            LineStyle::Dashed => {
                let (on, off) = (w * 3.0 + 2.0, w * 2.0 + 2.0);
                let period = on + off;
                for (a, b) in segs {
                    let len = (b - a).length();
                    if len <= 0.0 {
                        continue;
                    }
                    let dir = (b - a) / len;
                    let mut t = 0.0f32;
                    while t < len {
                        let phase = (dist + t) % period;
                        if phase < on {
                            let e = (t + on - phase).min(len);
                            rect = rect.union(&self.segment(a + dir * t, a + dir * e, width, aa));
                            t = e.max(t + 1e-3);
                        } else {
                            t = (t + period - phase).min(len).max(t + 1e-3);
                        }
                    }
                    dist += len;
                }
            }
        }
        rect
    }

    /// Anti-aliased polygon fill (even-odd) using 4x vertical supersampling and
    /// exact horizontal span coverage.
    pub fn fill_polygon(&mut self, polys: &[Vec<Pos2>], aa: bool) -> IRect {
        let mut min = pos2(f32::MAX, f32::MAX);
        let mut max = pos2(f32::MIN, f32::MIN);
        for p in polys.iter().flatten() {
            min = min.min(*p);
            max = max.max(*p);
        }
        if min.x > max.x {
            return IRect::EMPTY;
        }
        let rect = IRect::around(min, max).clamp_to(self.w, self.h);
        if rect.is_empty() {
            return rect;
        }
        let edges: Vec<(Pos2, Pos2)> = polys
            .iter()
            .filter(|p| p.len() >= 3)
            .flat_map(|p| (0..p.len()).map(move |i| (p[i], p[(i + 1) % p.len()])))
            .filter(|(a, b)| a.y != b.y)
            .collect();
        let samples: &[f32] = if aa { &[0.125, 0.375, 0.625, 0.875] } else { &[0.5] };
        let weight = 1.0 / samples.len() as f32;
        let width = rect.width() as usize;
        let mut row = vec![0.0f32; width + 2];
        let mut xs: Vec<f32> = Vec::new();
        for y in rect.y0..rect.y1 {
            row.fill(0.0);
            for s in samples {
                let sy = y as f32 + s;
                xs.clear();
                for (a, b) in &edges {
                    let (lo, hi) = if a.y < b.y { (a, b) } else { (b, a) };
                    if sy >= lo.y && sy < hi.y {
                        xs.push(lo.x + (sy - lo.y) / (hi.y - lo.y) * (hi.x - lo.x));
                    }
                }
                xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                for pair in xs.chunks(2) {
                    if pair.len() < 2 {
                        break;
                    }
                    let (mut x0, mut x1) = (pair[0] - rect.x0 as f32, pair[1] - rect.x0 as f32);
                    if !aa {
                        x0 = (x0 - 0.5).ceil() + 0.0;
                        x1 = (x1 - 0.5).ceil();
                    }
                    x0 = x0.clamp(0.0, width as f32);
                    x1 = x1.clamp(0.0, width as f32);
                    if x1 <= x0 {
                        continue;
                    }
                    let (i0, i1) = (x0.floor() as usize, x1.floor() as usize);
                    if i0 == i1 {
                        row[i0] += (x1 - x0) * weight;
                    } else {
                        row[i0] += (1.0 - (x0 - i0 as f32)) * weight;
                        for v in &mut row[i0 + 1..i1] {
                            *v += weight;
                        }
                        row[i1] += (x1 - i1 as f32) * weight;
                    }
                }
            }
            for (i, v) in row[..width].iter().enumerate() {
                if *v > 0.0 {
                    self.put_max(rect.x0 + i as i32, y, v.min(1.0));
                }
            }
        }
        rect
    }

    /// Fill all pixels where `mask` is set (hard).
    pub fn fill_mask(&mut self, mask: &Mask) -> IRect {
        let mut rect = IRect::EMPTY;
        if let Some(b) = mask.bounds() {
            for y in b.y0..b.y1 {
                for x in b.x0..b.x1 {
                    let v = mask.getf(x, y);
                    if v > 0.0 {
                        self.put_max(x, y, v);
                    }
                }
            }
            rect = b;
        }
        rect
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LineStyle {
    Solid,
    Dashed,
    Dotted,
}

// ---------------------------------------------------------------------------
// Geometry helpers producing polygons

pub fn ellipse_points(c: Pos2, rx: f32, ry: f32) -> Vec<Pos2> {
    let n = (((rx.abs() + ry.abs()) * 1.2) as usize).clamp(16, 720);
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            pos2(c.x + rx * t.cos(), c.y + ry * t.sin())
        })
        .collect()
}

pub fn rounded_rect_points(min: Pos2, max: Pos2, radius: f32) -> Vec<Pos2> {
    let r = radius.min((max.x - min.x) / 2.0).min((max.y - min.y) / 2.0).max(0.0);
    if r < 0.5 {
        return vec![min, pos2(max.x, min.y), max, pos2(min.x, max.y)];
    }
    let mut pts = Vec::new();
    let n = ((r * 0.8) as usize).clamp(4, 64);
    let corners = [
        (pos2(max.x - r, min.y + r), -90.0f32),
        (pos2(max.x - r, max.y - r), 0.0),
        (pos2(min.x + r, max.y - r), 90.0),
        (pos2(min.x + r, min.y + r), 180.0),
    ];
    for (c, start) in corners {
        for i in 0..=n {
            let a = (start + 90.0 * i as f32 / n as f32).to_radians();
            pts.push(pos2(c.x + r * a.cos(), c.y + r * a.sin()));
        }
    }
    pts
}

pub fn cubic_points(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2) -> Vec<Pos2> {
    let approx = (p1 - p0).length() + (p2 - p1).length() + (p3 - p2).length();
    let n = ((approx / 3.0) as usize).clamp(8, 512);
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let v = p0.to_vec2() * (u * u * u)
                + p1.to_vec2() * (3.0 * u * u * t)
                + p2.to_vec2() * (3.0 * u * t * t)
                + p3.to_vec2() * (t * t * t);
            v.to_pos2()
        })
        .collect()
}

/// Separable box blur, `passes` times (3 passes ~ gaussian).
pub fn box_blur_channel(data: &mut [f32], w: usize, h: usize, r: usize, passes: usize) {
    if r == 0 || w == 0 || h == 0 {
        return;
    }
    let mut tmp = vec![0.0f32; data.len()];
    for _ in 0..passes {
        blur_1d(data, &mut tmp, w, h, r, true);
        blur_1d(&tmp, data, w, h, r, false);
    }
}

fn blur_1d(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize, horizontal: bool) {
    let (len, lines) = if horizontal { (w, h) } else { (h, w) };
    let at = |line: usize, i: usize| if horizontal { line * w + i } else { i * w + line };
    let norm = 1.0 / (2 * r + 1) as f32;
    for line in 0..lines {
        let get = |i: isize| src[at(line, i.clamp(0, len as isize - 1) as usize)];
        let mut acc: f32 = (-(r as isize)..=r as isize).map(get).sum();
        for i in 0..len {
            dst[at(line, i)] = acc * norm;
            acc += get(i as isize + r as isize + 1) - get(i as isize - r as isize);
        }
    }
}

// ---------------------------------------------------------------------------
// Color helpers

pub fn rgb_to_hsv(c: Rgba) -> [f32; 3] {
    let (r, g, b) = (c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    [h, s, max]
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let f = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [f(r), f(g), f(b)]
}

pub fn hex_string(c: Rgba) -> String {
    if c[3] == 255 {
        format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
    } else {
        format!("#{:02X}{:02X}{:02X}{:02X}", c[0], c[1], c[2], c[3])
    }
}

pub fn parse_hex(s: &str) -> Option<Rgba> {
    let s = s.trim().trim_start_matches('#');
    let v = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
    match s.len() {
        3 => {
            let d = |i: usize| u8::from_str_radix(&s[i..i + 1], 16).ok().map(|v| v * 17);
            Some([d(0)?, d(1)?, d(2)?, 255])
        }
        6 => Some([v(0)?, v(2)?, v(4)?, 255]),
        8 => Some([v(0)?, v(2)?, v(4)?, v(6)?]),
        _ => None,
    }
}

pub fn color_distance(a: Rgba, b: Rgba) -> f32 {
    // Max channel difference, alpha-aware (so transparent pixels of any color match).
    let aa = a[3] as f32 / 255.0;
    let ba = b[3] as f32 / 255.0;
    let mut d = (a[3] as f32 - b[3] as f32).abs();
    for i in 0..3 {
        d = d.max((a[i] as f32 * aa - b[i] as f32 * ba).abs());
    }
    d / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_fill_covers_square() {
        let mut c = Coverage::new(10, 10);
        c.fill_polygon(&[vec![pos2(2.0, 2.0), pos2(6.0, 2.0), pos2(6.0, 6.0), pos2(2.0, 6.0)]], true);
        assert!((c.get(3, 3) - 1.0).abs() < 1e-4);
        assert_eq!(c.get(7, 7), 0.0);
        let total: f32 = c.data.iter().sum();
        assert!((total - 16.0).abs() < 0.01, "{total}");
    }

    #[test]
    fn half_pixel_edge() {
        let mut c = Coverage::new(10, 10);
        c.fill_polygon(&[vec![pos2(2.5, 2.0), pos2(6.0, 2.0), pos2(6.0, 6.0), pos2(2.5, 6.0)]], true);
        assert!((c.get(2, 3) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(parse_hex(&hex_string([1, 2, 3, 255])), Some([1, 2, 3, 255]));
        assert_eq!(parse_hex("#ff000080"), Some([255, 0, 0, 128]));
        assert_eq!(parse_hex("f00"), Some([255, 0, 0, 255]));
    }

    #[test]
    fn hsv_roundtrip() {
        for c in [[255u8, 0, 0, 255], [10, 200, 30, 255], [128, 128, 128, 255]] {
            let [h, s, v] = rgb_to_hsv(c);
            let r = hsv_to_rgb(h, s, v);
            assert_eq!([r[0], r[1], r[2]], [c[0], c[1], c[2]]);
        }
    }

    #[test]
    fn over_opaque() {
        assert_eq!(over([0, 0, 0, 255], [255, 0, 0, 255]), [255, 0, 0, 255]);
        assert_eq!(over([0, 0, 0, 0], [255, 0, 0, 128]), [255, 0, 0, 128]);
    }
}
