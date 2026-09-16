//! CHW float32 张量、padding、TTA. 对齐 unlimited:waifu2x 的预处理.

use crate::models::PadMode;

#[derive(Clone, Debug)]
pub struct Tensor {
    pub data: Vec<f32>,
    pub c: usize,
    pub h: usize,
    pub w: usize,
}

impl Tensor {
    pub fn zeros(c: usize, h: usize, w: usize) -> Self {
        Self {
            data: vec![0.0; c * h * w],
            c,
            h,
            w,
        }
    }

    pub fn from_rgb(rgb: &image::RgbImage) -> Self {
        let w = rgb.width() as usize;
        let h = rgb.height() as usize;
        let mut data = vec![0.0f32; 3 * h * w];
        let hw = h * w;
        let raw = rgb.as_raw();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 3;
                let j = y * w + x;
                data[j] = raw[i] as f32 / 255.0;
                data[hw + j] = raw[i + 1] as f32 / 255.0;
                data[2 * hw + j] = raw[i + 2] as f32 / 255.0;
            }
        }
        Self { data, c: 3, h, w }
    }

    pub fn from_rgba_premul_white(rgba: &image::RgbaImage) -> Self {
        let w = rgba.width() as usize;
        let h = rgba.height() as usize;
        let mut data = vec![0.0f32; 3 * h * w];
        let hw = h * w;
        let raw = rgba.as_raw();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let j = y * w + x;
                let a = raw[i + 3] as f32 / 255.0;
                for c in 0..3 {
                    data[c * hw + j] = a * (raw[i + c] as f32 / 255.0) + (1.0 - a);
                }
            }
        }
        Self { data, c: 3, h, w }
    }

    pub fn to_rgb(&self) -> image::RgbImage {
        let mut buf = vec![0u8; self.h * self.w * 3];
        let hw = self.h * self.w;
        for y in 0..self.h {
            for x in 0..self.w {
                let j = y * self.w + x;
                let i = j * 3;
                buf[i] = (self.data[j] * 255.0 + 0.49999).clamp(0.0, 255.0) as u8;
                buf[i + 1] = (self.data[hw + j] * 255.0 + 0.49999).clamp(0.0, 255.0) as u8;
                buf[i + 2] = (self.data[2 * hw + j] * 255.0 + 0.49999).clamp(0.0, 255.0) as u8;
            }
        }
        image::RgbImage::from_raw(self.w as u32, self.h as u32, buf).expect("rgb size")
    }

    fn idx(c: usize, y: usize, x: usize, h: usize, w: usize) -> usize {
        c * h * w + y * w + x
    }

    pub fn crop(&self, x: usize, y: usize, tw: usize, th: usize) -> Self {
        let mut out = Self::zeros(self.c, th, tw);
        for c in 0..self.c {
            for iy in 0..th {
                for ix in 0..tw {
                    let dst = Self::idx(c, iy, ix, th, tw);
                    let src = Self::idx(c, y + iy, x + ix, self.h, self.w);
                    out.data[dst] = self.data[src];
                }
            }
        }
        out
    }

    pub fn is_single_color(&self) -> Option<[f32; 3]> {
        if self.c < 3 || self.data.is_empty() {
            return None;
        }
        let hw = self.h * self.w;
        let r = self.data[0];
        let g = self.data[hw];
        let b = self.data[2 * hw];
        for i in 0..hw {
            if self.data[i] != r || self.data[hw + i] != g || self.data[2 * hw + i] != b {
                return None;
            }
        }
        Some([r, g, b])
    }

    pub fn solid(rgb: [f32; 3], h: usize, w: usize) -> Self {
        let mut t = Self::zeros(3, h, w);
        let hw = h * w;
        t.data[..hw].fill(rgb[0]);
        t.data[hw..2 * hw].fill(rgb[1]);
        t.data[2 * hw..].fill(rgb[2]);
        t
    }

    pub fn hflip(&self) -> Self {
        let mut out = Self::zeros(self.c, self.h, self.w);
        for c in 0..self.c {
            for y in 0..self.h {
                for x in 0..self.w {
                    let dst = Self::idx(c, y, x, self.h, self.w);
                    let src = Self::idx(c, y, self.w - 1 - x, self.h, self.w);
                    out.data[dst] = self.data[src];
                }
            }
        }
        out
    }

    pub fn vflip(&self) -> Self {
        let mut out = Self::zeros(self.c, self.h, self.w);
        for c in 0..self.c {
            for y in 0..self.h {
                for x in 0..self.w {
                    let dst = Self::idx(c, y, x, self.h, self.w);
                    let src = Self::idx(c, self.h - 1 - y, x, self.h, self.w);
                    out.data[dst] = self.data[src];
                }
            }
        }
        out
    }

    pub fn rot180(&self) -> Self {
        self.hflip().vflip()
    }

    pub fn add_assign(&mut self, other: &Self) {
        for (a, b) in self.data.iter_mut().zip(other.data.iter()) {
            *a += *b;
        }
    }

    pub fn scale_assign(&mut self, s: f32) {
        for v in &mut self.data {
            *v *= s;
        }
    }

    pub fn pad(&self, left: usize, right: usize, top: usize, bottom: usize, mode: PadMode) -> Self {
        let nh = self.h + top + bottom;
        let nw = self.w + left + right;
        let mut out = Self::zeros(self.c, nh, nw);
        for c in 0..self.c {
            for y in 0..nh {
                for x in 0..nw {
                    let (sx, sy) = match mode {
                        PadMode::Replication => {
                            let sy = y.saturating_sub(top).min(self.h.saturating_sub(1));
                            let sx = x.saturating_sub(left).min(self.w.saturating_sub(1));
                            (sx, sy)
                        }
                        PadMode::Reflection => {
                            (reflect(x as i32 - left as i32, self.w), reflect(y as i32 - top as i32, self.h))
                        }
                    };
                    let dst = Self::idx(c, y, x, nh, nw);
                    let src = Self::idx(c, sy, sx, self.h, self.w);
                    out.data[dst] = self.data[src];
                }
            }
        }
        out
    }

    pub fn nbytes(&self) -> u64 {
        (self.data.len() * 4) as u64
    }
}

fn reflect(mut v: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let n = len as i32;
    if n == 1 {
        return 0;
    }
    let period = 2 * n - 2;
    v = v.rem_euclid(period);
    if v < 0 {
        v += period;
    }
    if v >= n {
        (2 * n - 2 - v) as usize
    } else {
        v as usize
    }
}

/// TTA 0/2/4: 顺序跑各变换再平均, 避免把 batch 一次性塞进显存.
pub fn tta_variants(level: u32) -> &'static [TtaOp] {
    match level {
        2 => &[TtaOp::Id, TtaOp::HFlip],
        4 => &[TtaOp::Id, TtaOp::HFlip, TtaOp::VFlip, TtaOp::Rot180],
        _ => &[TtaOp::Id],
    }
}

#[derive(Clone, Copy)]
pub enum TtaOp {
    Id,
    HFlip,
    VFlip,
    Rot180,
}

impl TtaOp {
    pub fn apply(self, x: &Tensor) -> Tensor {
        match self {
            Self::Id => x.clone(),
            Self::HFlip => x.hflip(),
            Self::VFlip => x.vflip(),
            Self::Rot180 => x.rot180(),
        }
    }
}
