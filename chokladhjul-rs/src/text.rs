//! Signed distance field text. Glyphs are rasterized once with fontdue at a
//! large size and turned into a distance field so text can be scaled,
//! outlined and shadowed cheaply in the shader.

use std::collections::HashMap;

use crate::gl_util::Texture;

#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub uv: [f32; 4],
    /// Quad offset from the pen position (baseline, y down), in raster pixels.
    pub offset: [f32; 2],
    pub size: [f32; 2],
    pub advance: f32,
}

pub struct SdfFont {
    pub texture: Texture,
    font: fontdue::Font,
    glyphs: HashMap<char, Glyph>,
    pub raster_px: f32,
    /// Distance range stored in the texture, in raster pixels (each way).
    pub spread: f32,
}

/// A glyph quad positioned in virtual pixels.
pub struct PlacedGlyph {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub uv: [f32; 4],
}

pub const FONT_CANDIDATES: &[&str] = &[
    "C:\\Windows\\Fonts\\ariblk.ttf",
    "C:\\Windows\\Fonts\\arialbd.ttf",
    "C:\\Windows\\Fonts\\segoeuib.ttf",
    "C:\\Windows\\Fonts\\arial.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    "/System/Library/Fonts/Supplemental/Arial Black.ttf",
    "/Library/Fonts/Arial Black.ttf",
];

pub fn load_system_font() -> Option<(String, Vec<u8>)> {
    FONT_CANDIDATES
        .iter()
        .find_map(|path| std::fs::read(path).ok().map(|data| (path.to_string(), data)))
}

impl SdfFont {
    pub fn new(gl: &glow::Context, data: &[u8], raster_px: f32, spread: usize) -> Result<Self, String> {
        let font = fontdue::Font::from_bytes(data, fontdue::FontSettings::default())?;
        let chars: Vec<char> = (0x21u8..0x7f).map(char::from).chain("åäöÅÄÖé".chars()).collect();

        struct Cell {
            ch: char,
            w: usize,
            h: usize,
            field: Vec<u8>,
            metrics: fontdue::Metrics,
        }
        let mut cells = Vec::new();
        for &ch in &chars {
            if font.lookup_glyph_index(ch) == 0 {
                continue;
            }
            let (metrics, coverage) = font.rasterize(ch, raster_px);
            let w = metrics.width + 2 * spread;
            let h = metrics.height + 2 * spread;
            let field = distance_field(&coverage, metrics.width, metrics.height, spread);
            cells.push(Cell { ch, w, h, field, metrics });
        }

        // Shelf packing, tallest first.
        cells.sort_by_key(|c| std::cmp::Reverse(c.h));
        let atlas_w = 1024usize;
        let (mut x, mut y, mut shelf_h) = (0usize, 0usize, 0usize);
        let mut placed = Vec::with_capacity(cells.len());
        for cell in &cells {
            if x + cell.w > atlas_w {
                x = 0;
                y += shelf_h + 1;
                shelf_h = 0;
            }
            placed.push((x, y));
            x += cell.w + 1;
            shelf_h = shelf_h.max(cell.h);
        }
        let atlas_h = (y + shelf_h).next_multiple_of(4).max(4);
        let mut pixels = vec![0u8; atlas_w * atlas_h];
        let mut glyphs = HashMap::new();
        for (cell, &(px, py)) in cells.iter().zip(&placed) {
            for row in 0..cell.h {
                let dst = (py + row) * atlas_w + px;
                pixels[dst..dst + cell.w].copy_from_slice(&cell.field[row * cell.w..(row + 1) * cell.w]);
            }
            let m = &cell.metrics;
            glyphs.insert(
                cell.ch,
                Glyph {
                    uv: [
                        px as f32 / atlas_w as f32,
                        py as f32 / atlas_h as f32,
                        (px + cell.w) as f32 / atlas_w as f32,
                        (py + cell.h) as f32 / atlas_h as f32,
                    ],
                    offset: [
                        m.xmin as f32 - spread as f32,
                        -(m.ymin as f32 + m.height as f32) - spread as f32,
                    ],
                    size: [cell.w as f32, cell.h as f32],
                    advance: m.advance_width,
                },
            );
        }
        let texture = Texture::r8(gl, atlas_w as u32, atlas_h as u32, &pixels)?;
        Ok(SdfFont { texture, font, glyphs, raster_px, spread: spread as f32 })
    }

    /// Lays out a single line. Returns the glyph quads with the pen starting
    /// at x = 0 on the baseline y = 0, plus the advance width.
    pub fn layout(&self, text: &str, size: f32) -> (Vec<PlacedGlyph>, f32) {
        let k = size / self.raster_px;
        let mut pen = 0.0f32;
        let mut out = Vec::new();
        let mut prev: Option<char> = None;
        for ch in text.chars() {
            if let Some(p) = prev {
                pen += self.font.horizontal_kern(p, ch, size).unwrap_or(0.0);
            }
            prev = Some(ch);
            match self.glyphs.get(&ch) {
                Some(g) => {
                    out.push(PlacedGlyph {
                        pos: [pen + g.offset[0] * k, g.offset[1] * k],
                        size: [g.size[0] * k, g.size[1] * k],
                        uv: g.uv,
                    });
                    pen += g.advance * k;
                }
                None => pen += self.font.metrics(ch, size).advance_width,
            }
        }
        (out, pen)
    }

    /// Height of capital letters, for vertical centring.
    pub fn cap_height(&self, size: f32) -> f32 {
        self.font.metrics('H', size).height as f32
    }
}

/// Converts glyph coverage to an 8-bit distance field (0.5 = edge, higher = inside).
fn distance_field(coverage: &[u8], w: usize, h: usize, spread: usize) -> Vec<u8> {
    let pw = w + 2 * spread;
    let ph = h + 2 * spread;
    let cov = |x: usize, y: usize| -> f32 {
        if x < spread || y < spread || x >= spread + w || y >= spread + h {
            0.0
        } else {
            coverage[(y - spread) * w + (x - spread)] as f32 / 255.0
        }
    };
    const INF: f32 = 1e20;
    let mut to_inside = vec![INF; pw * ph];
    let mut to_outside = vec![INF; pw * ph];
    for y in 0..ph {
        for x in 0..pw {
            if cov(x, y) >= 0.5 {
                to_inside[y * pw + x] = 0.0;
            } else {
                to_outside[y * pw + x] = 0.0;
            }
        }
    }
    edt(&mut to_inside, pw, ph);
    edt(&mut to_outside, pw, ph);

    let mut out = vec![0u8; pw * ph];
    for y in 0..ph {
        for x in 0..pw {
            let i = y * pw + x;
            let c = cov(x, y);
            // Signed distance in pixels, positive inside.
            let d = if c > 0.0 && c < 1.0 {
                c - 0.5
            } else if c >= 0.5 {
                to_outside[i].sqrt() - 0.5
            } else {
                -(to_inside[i].sqrt() - 0.5)
            };
            let v = 0.5 + d / (2.0 * spread as f32);
            out[i] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    out
}

/// Exact squared Euclidean distance transform (Felzenszwalb & Huttenlocher).
fn edt(grid: &mut [f32], w: usize, h: usize) {
    let n = w.max(h);
    let mut f = vec![0.0f32; n];
    let mut d = vec![0.0f32; n];
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f32; n + 1];
    for x in 0..w {
        for y in 0..h {
            f[y] = grid[y * w + x];
        }
        edt_1d(&f[..h], &mut d[..h], &mut v, &mut z);
        for y in 0..h {
            grid[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&grid[y * w..(y + 1) * w]);
        edt_1d(&f[..w], &mut d[..w], &mut v, &mut z);
        grid[y * w..(y + 1) * w].copy_from_slice(&d[..w]);
    }
}

fn edt_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        let qf = q as f32;
        loop {
            let p = v[k];
            let pf = p as f32;
            let s = ((f[q] + qf * qf) - (f[p] + pf * pf)) / (2.0 * qf - 2.0 * pf);
            if s <= z[k] && k > 0 {
                k -= 1;
                continue;
            }
            if s <= z[k] {
                // k == 0: replace the only parabola.
                v[0] = q;
                z[0] = f32::NEG_INFINITY;
                z[1] = f32::INFINITY;
                break;
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f32::INFINITY;
            break;
        }
    }
    k = 0;
    for (q, dq) in d.iter_mut().enumerate() {
        let qf = q as f32;
        while z[k + 1] < qf {
            k += 1;
        }
        let p = v[k] as f32;
        *dq = (qf - p) * (qf - p) + f[v[k]];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edt_matches_brute_force() {
        let (w, h) = (13usize, 9usize);
        let seeds = [(2usize, 3usize), (10, 1), (7, 7)];
        let mut grid = vec![1e20f32; w * h];
        for &(x, y) in &seeds {
            grid[y * w + x] = 0.0;
        }
        edt(&mut grid, w, h);
        for y in 0..h {
            for x in 0..w {
                let brute = seeds
                    .iter()
                    .map(|&(sx, sy)| (x as f32 - sx as f32).powi(2) + (y as f32 - sy as f32).powi(2))
                    .fold(f32::MAX, f32::min);
                assert_eq!(grid[y * w + x], brute, "at {x},{y}");
            }
        }
    }

    #[test]
    fn distance_field_edge_is_half() {
        // A filled square: inside > 0.5, outside < 0.5.
        let (w, h) = (8usize, 8usize);
        let cov = vec![255u8; w * h];
        let field = distance_field(&cov, w, h, 4);
        let pw = w + 8;
        assert!(field[(4 + 4) * pw + 4 + 4] > 140);
        assert!(field[0] < 10);
    }
}
