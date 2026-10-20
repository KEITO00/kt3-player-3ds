use std::collections::HashMap;

pub type Rgb = [u8; 3];

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    px: Vec<u8>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Self {
        Canvas { w, h, px: vec![0; w * h * 3] }
    }

    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        let (x0, y0) = (x.max(0) as usize, y.max(0) as usize);
        let (x1, y1) = (((x + w).max(0) as usize).min(self.w), ((y + h).max(0) as usize).min(self.h));
        for yy in y0..y1 {
            for xx in x0..x1 {
                let o = (yy * self.w + xx) * 3;
                self.px[o..o + 3].copy_from_slice(&c);
            }
        }
    }

    fn blend(&mut self, x: usize, y: usize, c: Rgb, a: u32) {
        let o = (y * self.w + x) * 3;
        for i in 0..3 {
            let d = self.px[o + i] as u32;
            self.px[o + i] = ((c[i] as u32 * a + d * (255 - a) + 127) / 255) as u8;
        }
    }

    pub fn image(&mut self, x: i32, y: i32, img: &Image) {
        for yy in 0..img.h {
            let ty = y + yy as i32;
            if ty < 0 || ty as usize >= self.h {
                continue;
            }
            for xx in 0..img.w {
                let tx = x + xx as i32;
                if tx < 0 || tx as usize >= self.w {
                    continue;
                }
                let s = (yy * img.w + xx) * 3;
                let o = (ty as usize * self.w + tx as usize) * 3;
                self.px[o..o + 3].copy_from_slice(&img.px[s..s + 3]);
            }
        }
    }

    pub fn present(&self, fb: *mut u8, fb_w: usize, x: i32, y: i32, w: i32, h: i32) {
        let (x0, y0) = (x.max(0) as usize, y.max(0) as usize);
        let (x1, y1) = (((x + w).max(0) as usize).min(self.w), ((y + h).max(0) as usize).min(self.h));
        let fb = unsafe { std::slice::from_raw_parts_mut(fb, self.w * fb_w * 3) };
        for xx in x0..x1 {
            for yy in y0..y1 {
                let s = (yy * self.w + xx) * 3;
                let o = (xx * fb_w + (fb_w - 1 - yy)) * 3;
                fb[o] = self.px[s + 2];
                fb[o + 1] = self.px[s + 1];
                fb[o + 2] = self.px[s];
            }
        }
    }
}

pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Image {
    pub fn webp(data: &[u8], size: usize) -> Result<Image, String> {
        let mut d = image_webp::WebPDecoder::new(std::io::Cursor::new(data)).map_err(|e| format!("Thumbnail: {e}"))?;
        let (w, h) = d.dimensions();
        let (w, h) = (w as usize, h as usize);
        let bpp = if d.has_alpha() { 4 } else { 3 };
        let mut buf = vec![0u8; d.output_buffer_size().ok_or("Thumbnail is too large")?];
        d.read_image(&mut buf).map_err(|e| format!("Thumbnail: {e}"))?;
        let mut px = vec![0u8; size * size * 3];
        for ty in 0..size {
            let (sy0, sy1) = (ty * h / size, ((ty + 1) * h / size).max(ty * h / size + 1));
            for tx in 0..size {
                let (sx0, sx1) = (tx * w / size, ((tx + 1) * w / size).max(tx * w / size + 1));
                let mut acc = [0u32; 3];
                for sy in sy0..sy1 {
                    for sx in sx0..sx1 {
                        let s = (sy * w + sx) * bpp;
                        for i in 0..3 {
                            acc[i] += buf[s + i] as u32;
                        }
                    }
                }
                let n = ((sy1 - sy0) * (sx1 - sx0)) as u32;
                let o = (ty * size + tx) * 3;
                for i in 0..3 {
                    px[o + i] = (acc[i] / n) as u8;
                }
            }
        }
        Ok(Image { w: size, h: size, px })
    }
}

struct Glyph {
    left: i32,
    advance: i32,
    w: usize,
    h: usize,
    alpha: Vec<u8>,
}

pub struct Font {
    font: *mut ctru_sys::CFNT_s,
    cell_w: usize,
    cell_h: usize,
    sheet_w: usize,
    sheet_size: usize,
    sheet_fmt: u16,
    per_row: usize,
    per_sheet: usize,
    sheets: *const u8,
    cache: HashMap<(i32, u8), Glyph>,
}

fn morton(x: usize, y: usize) -> usize {
    (x & 1) | ((y & 1) << 1) | ((x & 2) << 1) | ((y & 2) << 2) | ((x & 4) << 2) | ((y & 4) << 3)
}

impl Font {
    pub fn system() -> Result<Font, String> {
        unsafe {
            if ctru_sys::fontEnsureMapped() < 0 {
                return Err("Cannot load the system font".into());
            }
            let font = ctru_sys::fontGetSystemFont();
            if font.is_null() {
                return Err("Cannot load the system font".into());
            }
            let t = &*(*font).finf.tglp;
            Ok(Font {
                font,
                cell_w: t.cellWidth as usize,
                cell_h: t.cellHeight as usize,
                sheet_w: t.sheetWidth as usize,
                sheet_size: t.sheetSize as usize,
                sheet_fmt: t.sheetFmt,
                per_row: t.nRows as usize,
                per_sheet: t.nRows as usize * t.nLines as usize,
                sheets: t.sheetData,
                cache: HashMap::new(),
            })
        }
    }

    fn texel(&self, sheet: usize, x: usize, y: usize) -> u8 {
        let i = ((y >> 3) * (self.sheet_w >> 3) + (x >> 3)) * 64 + morton(x & 7, y & 7);
        let base = unsafe { self.sheets.add(sheet * self.sheet_size) };
        match self.sheet_fmt {
            0xB => {
                let b = unsafe { *base.add(i >> 1) };
                let v = if i & 1 == 1 { b >> 4 } else { b & 15 };
                v * 17
            }
            0x8 => unsafe { *base.add(i) },
            _ => 0,
        }
    }

    fn glyph(&mut self, c: char, px: u8) -> &Glyph {
        let idx = unsafe { ctru_sys::fontGlyphIndexFromCodePoint(self.font, c as u32) };
        if !self.cache.contains_key(&(idx, px)) {
            let cwi = unsafe { &*ctru_sys::fontGetCharWidthInfo(self.font, idx) };
            let scale = px as f32 / self.cell_h as f32;
            let (sheet, at) = (idx as usize / self.per_sheet, idx as usize % self.per_sheet);
            let (gx, gy) = ((at % self.per_row) * (self.cell_w + 1) + 1, (at / self.per_row) * (self.cell_h + 1) + 1);
            let sw = cwi.glyphWidth as usize;
            let (w, h) = (((sw as f32 * scale).ceil() as usize).max(1), px as usize);
            let mut alpha = vec![0u8; w * h];
            for ty in 0..h {
                let (y0, y1) = (ty as f32 / scale, (ty + 1) as f32 / scale);
                for tx in 0..w {
                    let (x0, x1) = (tx as f32 / scale, (tx + 1) as f32 / scale);
                    let (mut acc, mut area) = (0.0f32, 0.0f32);
                    let mut sy = y0.floor() as usize;
                    while (sy as f32) < y1 && sy < self.cell_h {
                        let fy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
                        let mut sx = x0.floor() as usize;
                        while (sx as f32) < x1 && sx < sw {
                            let fx = (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
                            acc += self.texel(sheet, gx + sx, gy + sy) as f32 * fx * fy;
                            area += fx * fy;
                            sx += 1;
                        }
                        sy += 1;
                    }
                    alpha[ty * w + tx] = if area > 0.0 { (acc / area).round().min(255.0) as u8 } else { 0 };
                }
            }
            let g = Glyph { left: (cwi.left as f32 * scale).round() as i32, advance: (cwi.charWidth as f32 * scale).round() as i32, w, h, alpha };
            self.cache.insert((idx, px), g);
        }
        &self.cache[&(idx, px)]
    }

    pub fn width(&mut self, s: &str, px: u8) -> i32 {
        s.chars().map(|c| self.glyph(c, px).advance).sum()
    }

    pub fn text(&mut self, cv: &mut Canvas, x: i32, y: i32, s: &str, px: u8, color: Rgb, max_w: i32) -> i32 {
        let s = self.fit(s, px, max_w);
        let mut pen = x;
        for c in s.chars() {
            let g = self.glyph(c, px);
            for gy in 0..g.h {
                let ty = y + gy as i32;
                if ty < 0 || ty as usize >= cv.h {
                    continue;
                }
                for gx in 0..g.w {
                    let tx = pen + g.left + gx as i32;
                    let a = g.alpha[gy * g.w + gx];
                    if a == 0 || tx < 0 || tx as usize >= cv.w {
                        continue;
                    }
                    cv.blend(tx as usize, ty as usize, color, a as u32);
                }
            }
            pen += g.advance;
        }
        pen - x
    }

    fn fit(&mut self, s: &str, px: u8, max_w: i32) -> String {
        if self.width(s, px) <= max_w {
            return s.to_string();
        }
        let ell = self.width("…", px);
        let mut out = String::new();
        let mut w = 0;
        for c in s.chars() {
            let cw = self.glyph(c, px).advance;
            if w + cw + ell > max_w {
                break;
            }
            out.push(c);
            w += cw;
        }
        out.push('…');
        out
    }
}
