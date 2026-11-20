//! Texture preparation: baking the rim and pegs onto the wheel, and procedural sprites.
//! All images are premultiplied RGBA8.

use crate::physics::{MAIN_SECTORS, WheelGeometry};
use std::f32::consts::{PI, TAU};

/// Peg ring and peg size as fractions of the wheel's outer radius. Shared with the physics.
pub const PEG_RING_FRAC: f32 = 498.0 / 512.0;
pub const PEG_RADIUS_FRAC: f32 = 9.0 / 512.0;
const RING_INNER_FRAC: f32 = 484.0 / 512.0;
/// The source wheel art extends slightly under the rim.
const ART_RADIUS_FRAC: f32 = 490.0 / 512.0;

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    fn new(width: u32, height: u32) -> Self {
        Image { width, height, pixels: vec![0; (width * height * 4) as usize] }
    }

    fn put(&mut self, x: u32, y: u32, c: [f32; 4]) {
        let i = ((y * self.width + x) * 4) as usize;
        for (dst, v) in self.pixels[i..i + 4].iter_mut().zip(c) {
            *dst = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }

    fn get(&self, x: i32, y: i32) -> [f32; 4] {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return [0.0; 4];
        }
        let i = ((y as u32 * self.width + x as u32) * 4) as usize;
        let p = &self.pixels[i..i + 4];
        [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0]
    }

    /// Bilinear sample at pixel coordinates (pixel centres at +0.5).
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let (x0, y0) = (x0 as i32, y0 as i32);
        let a = self.get(x0, y0);
        let b = self.get(x0 + 1, y0);
        let c = self.get(x0, y0 + 1);
        let d = self.get(x0 + 1, y0 + 1);
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bottom = c[k] + (d[k] - c[k]) * tx;
            top + (bottom - top) * ty
        })
    }

    /// Pixel data with straight (non-premultiplied) alpha, e.g. for window icons.
    pub fn unpremultiplied(&self) -> Vec<u8> {
        let mut out = self.pixels.clone();
        for p in out.as_chunks_mut::<4>().0 {
            let a = p[3] as f32 / 255.0;
            if a > 0.0 {
                for c in &mut p[..3] {
                    *c = ((*c as f32 / a).min(255.0) + 0.5) as u8;
                }
            }
        }
        out
    }
}

pub fn decode_png(bytes: &[u8]) -> Result<Image, String> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|e| format!("could not decode image: {e}"))?
        .to_rgba8();
    let (width, height) = img.dimensions();
    let mut pixels = img.into_raw();
    for p in pixels.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        for c in &mut p[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
    Ok(Image { width, height, pixels })
}

/// Downscales a square image with a box filter (used for the window icon).
pub fn downscale(src: &Image, size: u32) -> Image {
    let mut out = Image::new(size, size);
    let f = src.width as f32 / size as f32;
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0.0f32; 4];
            let mut n = 0.0;
            let (sx0, sy0) = ((x as f32 * f) as i32, (y as f32 * f) as i32);
            let (sx1, sy1) = (((x + 1) as f32 * f).ceil() as i32, ((y + 1) as f32 * f).ceil() as i32);
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let p = src.get(sx, sy);
                    for k in 0..4 {
                        acc[k] += p[k];
                    }
                    n += 1.0;
                }
            }
            out.put(x, y, acc.map(|v| v / f32::max(n, 1.0)));
        }
    }
    out
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

fn rgb(r: u8, g: u8, b: u8) -> [f32; 3] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

/// Premultiplied `src` over premultiplied `dst`.
fn over(dst: [f32; 4], src: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|k| src[k] + dst[k] * (1.0 - src[3]))
}

fn opaque(c: [f32; 3], coverage: f32) -> [f32; 4] {
    [c[0] * coverage, c[1] * coverage, c[2] * coverage, coverage]
}

/// Puts the wheel art inside a golden rim with pegs between the sectors.
/// Shading is rotation invariant so it looks right at any wheel angle.
pub fn bake_wheel(art: &Image, size: u32) -> Image {
    let mut out = Image::new(size, size);
    let half = size as f32 / 2.0;
    let r_in = RING_INNER_FRAC * half;
    let r_peg = PEG_RING_FRAC * half;
    let peg = PEG_RADIUS_FRAC * half;
    let art_scale = (art.width.min(art.height) as f32 / 2.0) / (ART_RADIUS_FRAC * half);
    let art_c = [art.width as f32 / 2.0, art.height as f32 / 2.0];
    let step = TAU / MAIN_SECTORS as f32;

    let gold_dark = rgb(112, 70, 14);
    let gold_mid = rgb(222, 168, 58);
    let gold_light = rgb(255, 240, 170);

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - half;
            let dy = y as f32 + 0.5 - half;
            let r = (dx * dx + dy * dy).sqrt();
            if r > half + 1.0 {
                continue;
            }

            let mut c = if r < ART_RADIUS_FRAC * half + 1.0 {
                art.sample(art_c[0] + dx * art_scale, art_c[1] + dy * art_scale)
            } else {
                [0.0; 4]
            };
            // The rim casts a soft shadow onto the wheel face.
            let shade = 1.0 - 0.45 * smoothstep(r_in - 16.0, r_in, r);
            for v in &mut c[..3] {
                *v *= shade;
            }

            let ring_cov = (half - r + 0.5).clamp(0.0, 1.0) * (r - r_in + 0.5).clamp(0.0, 1.0);
            if ring_cov > 0.0 {
                let t = ((r - r_in) / (half - r_in)).clamp(0.0, 1.0);
                let mut col = mix3(gold_dark, gold_mid, (PI * t).sin().powf(0.6));
                let spec = (-((t - 0.32) / 0.1).powi(2)).exp();
                col = mix3(col, gold_light, spec * 0.85);
                let edge = t.min(1.0 - t);
                col = col.map(|v| v * (0.6 + 0.4 * smoothstep(0.0, 0.14, edge)));
                c = over(c, opaque(col, ring_cov));
            }

            let a = dx.atan2(-dy);
            let i = (a / step - 0.5).round();
            let pa = (i + 0.5) * step;
            let (px, py) = (r_peg * pa.sin(), -r_peg * pa.cos());
            let dp = ((dx - px).powi(2) + (dy - py).powi(2)).sqrt();

            let contact = 1.0 - smoothstep(peg, peg + 5.0, dp);
            for v in &mut c[..3] {
                *v *= 1.0 - 0.5 * contact;
            }
            let peg_cov = (peg - dp + 0.5).clamp(0.0, 1.0);
            if peg_cov > 0.0 {
                let dn = (dp / peg).min(1.0);
                let mut col = mix3(rgb(250, 248, 240), rgb(118, 118, 130), dn.powf(2.2));
                col = mix3(col, [1.0; 3], 0.55 * (-(dn / 0.38).powi(2)).exp());
                col = col.map(|v| v * (1.0 - 0.55 * smoothstep(0.8, 1.0, dn)));
                c = over(c, opaque(col, peg_cov));
            }
            out.put(x, y, c);
        }
    }
    out
}

fn sd_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let pa = [p[0] - a[0], p[1] - a[1]];
    let ba = [b[0] - a[0], b[1] - a[1]];
    let h = ((pa[0] * ba[0] + pa[1] * ba[1]) / (ba[0] * ba[0] + ba[1] * ba[1])).clamp(0.0, 1.0);
    ((pa[0] - ba[0] * h).powi(2) + (pa[1] - ba[1] * h).powi(2)).sqrt()
}

/// Signed distance to the flapper outline (negative inside): a triangle from
/// the pivot to the tip, with a rounded top around the pivot.
fn flapper_sd(p: [f32; 2], hw: f32, len: f32) -> f32 {
    let a = [-hw, 0.0];
    let b = [hw, 0.0];
    let t = [0.0, len];
    let d = sd_segment(p, a, b).min(sd_segment(p, b, t)).min(sd_segment(p, t, a));
    let inside_tri = p[1] >= 0.0 && p[0].abs() <= hw * (1.0 - p[1] / len);
    let tri = if inside_tri { -d } else { d };
    let circle = (p[0] * p[0] + p[1] * p[1]).sqrt() - hw;
    tri.min(circle)
}

pub struct FlapperSprites {
    pub body: Image,
    pub shadow: Image,
    /// Pivot position in 0..1 texture space.
    pub pivot: [f32; 2],
    /// Sprite size in virtual pixels.
    pub size: [f32; 2],
}

/// Renders the flapper so its edges match the collision shape in `physics`.
pub fn flapper(geo: &WheelGeometry, px_per_unit: f32) -> FlapperSprites {
    let hw = geo.pointer_half_width as f32;
    let len = geo.pointer_len as f32;
    let margin = 8.0;
    let min = [-hw - margin, -hw - margin];
    let size = [2.0 * (hw + margin), len + hw + 2.0 * margin];
    let w = (size[0] * px_per_unit).ceil() as u32;
    let h = (size[1] * px_per_unit).ceil() as u32;
    let mut body = Image::new(w, h);
    let mut mask = vec![0.0f32; (w * h) as usize];
    let outline = 2.4;

    for y in 0..h {
        for x in 0..w {
            let p = [min[0] + (x as f32 + 0.5) / px_per_unit, min[1] + (y as f32 + 0.5) / px_per_unit];
            let sd = flapper_sd(p, hw, len);
            let cov = (0.5 - sd * px_per_unit).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            mask[(y * w + x) as usize] = cov;
            let light = (0.5 - 0.5 * p[0] / hw).clamp(0.0, 1.0);
            let mut col = mix3(rgb(214, 84, 0), rgb(255, 196, 64), light);
            col = mix3(col, rgb(255, 245, 200), 0.45 * (-((p[0] + 1.6) / 1.8).powi(2)).exp());
            col = col.map(|v| v * (1.0 - 0.18 * (p[1] / len).clamp(0.0, 1.0)));
            let edge = smoothstep(-outline - 0.6, -outline + 0.6, sd);
            col = mix3(col, rgb(40, 22, 6), edge);
            body.put(x, y, opaque(col, cov));
        }
    }

    let blurred = box_blur(&mask, w, h, (2.5 * px_per_unit) as usize, 3);
    let mut shadow = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            shadow.put(x, y, [0.0, 0.0, 0.0, blurred[(y * w + x) as usize]]);
        }
    }

    FlapperSprites {
        body,
        shadow,
        pivot: [-min[0] / size[0], -min[1] / size[1]],
        size: [w as f32 / px_per_unit, h as f32 / px_per_unit],
    }
}

fn box_blur(src: &[f32], w: u32, h: u32, radius: usize, passes: usize) -> Vec<f32> {
    let (w, h) = (w as usize, h as usize);
    let mut a = src.to_vec();
    let mut b = vec![0.0; a.len()];
    let norm = 1.0 / (2 * radius + 1) as f32;
    for _ in 0..passes {
        for y in 0..h {
            for x in 0..w {
                let s: f32 = (x.saturating_sub(radius)..(x + radius + 1).min(w)).map(|k| a[y * w + k]).sum();
                b[y * w + x] = s * norm;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let s: f32 = (y.saturating_sub(radius)..(y + radius + 1).min(h)).map(|k| b[k * w + x]).sum();
                a[y * w + x] = s * norm;
            }
        }
    }
    a
}

/// Polished brass knob for pivots.
pub fn knob(size: u32) -> Image {
    let mut out = Image::new(size, size);
    let half = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let p = [(x as f32 + 0.5 - half) / half, (y as f32 + 0.5 - half) / half];
            let r = (p[0] * p[0] + p[1] * p[1]).sqrt();
            let cov = ((1.0 - r) * half + 0.5).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let hl = ((p[0] + 0.35).powi(2) + (p[1] + 0.4).powi(2)).sqrt();
            let mut col = mix3(rgb(255, 246, 200), rgb(150, 98, 24), smoothstep(0.0, 1.3, hl));
            col = mix3(col, [1.0; 3], 0.7 * (-(hl / 0.22).powi(2)).exp());
            col = col.map(|v| v * (1.0 - 0.6 * smoothstep(0.78, 1.0, r)));
            out.put(x, y, opaque(col, cov));
        }
    }
    out
}

fn radial(size: u32, f: impl Fn(f32, [f32; 2]) -> f32) -> Image {
    let mut out = Image::new(size, size);
    let half = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let p = [(x as f32 + 0.5 - half) / half, (y as f32 + 0.5 - half) / half];
            let r = (p[0] * p[0] + p[1] * p[1]).sqrt();
            let a = f(r, p).clamp(0.0, 1.0);
            out.put(x, y, [a, a, a, a]);
        }
    }
    out
}

/// Disc with a wide soft edge, used for drop shadows.
pub fn soft_disc(size: u32) -> Image {
    radial(size, |r, _| 1.0 - smoothstep(0.7, 1.0, r))
}

pub fn glow(size: u32) -> Image {
    radial(size, |r, _| (-r * r * 4.5).exp() * (1.0 - smoothstep(0.85, 1.0, r)))
}

/// Four pointed twinkle.
pub fn sparkle(size: u32) -> Image {
    radial(size, |r, p| {
        let (x, y) = (p[0].abs(), p[1].abs());
        let rays = (-x * 16.0).exp() * (-y * 2.6).exp() + (-y * 16.0).exp() * (-x * 2.6).exp();
        (rays + 0.9 * (-r * r * 40.0).exp()) * (1.0 - smoothstep(0.75, 1.0, r))
    })
}

pub fn white() -> Image {
    Image { width: 4, height: 4, pixels: vec![255; 64] }
}
