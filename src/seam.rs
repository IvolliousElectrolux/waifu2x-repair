//! 与 nunif/utils/seam_blending.py 相同的累积缝合.

use crate::tensor::Tensor;

pub const BLEND_SIZE: u32 = 16;

#[derive(Clone, Debug)]
pub struct SeamParam {
    pub y_h: usize,
    pub y_w: usize,
    pub h_blocks: usize,
    pub w_blocks: usize,
    pub input_tile_step: usize,
    pub output_tile_step: usize,
    pub y_buffer_h: usize,
    pub y_buffer_w: usize,
    /// left, right, top, bottom (与 JS pad 数组一致)
    pub pad: [usize; 4],
}

pub fn calc_parameters(
    x_h: usize,
    x_w: usize,
    scale: u32,
    offset: u32,
    tile_size: u32,
    blend_size: u32,
) -> SeamParam {
    let scale = scale as usize;
    let offset = offset as usize;
    let tile_size = tile_size as usize;
    let blend_size = blend_size as usize;
    let input_offset = (offset as f32 / scale as f32).ceil() as usize;
    let input_blend_size = (blend_size as f32 / scale as f32).ceil() as usize;
    let input_tile_step = tile_size - (input_offset * 2 + input_blend_size);

    let mut h_blocks = 0usize;
    let mut w_blocks = 0usize;
    let mut input_h = 0usize;
    let mut input_w = 0usize;
    while input_h < x_h + input_offset * 2 {
        input_h = h_blocks * input_tile_step + tile_size;
        h_blocks += 1;
    }
    while input_w < x_w + input_offset * 2 {
        input_w = w_blocks * input_tile_step + tile_size;
        w_blocks += 1;
    }
    SeamParam {
        y_h: x_h * scale,
        y_w: x_w * scale,
        h_blocks,
        w_blocks,
        input_tile_step,
        output_tile_step: input_tile_step * scale,
        y_buffer_h: input_h * scale,
        y_buffer_w: input_w * scale,
        pad: [
            input_offset,
            input_w - (x_w + input_offset),
            input_offset,
            input_h - (x_h + input_offset),
        ],
    }
}

pub fn create_blend_filter(scale: u32, offset: u32, tile_size: u32, blend_size: u32) -> Tensor {
    let model_output_size = tile_size * scale - offset * 2;
    let inner = model_output_size.saturating_sub(blend_size * 2).max(1);
    let mut t = Tensor::zeros(3, inner as usize, inner as usize);
    t.data.fill(1.0);
    for i in 0..blend_size {
        let value = 1.0 - (1.0 / (blend_size as f32 + 1.0)) * (i as f32 + 1.0);
        t = pad_const(&t, 1, value);
    }
    t
}

fn pad_const(x: &Tensor, border: usize, v: f32) -> Tensor {
    let nh = x.h + border * 2;
    let nw = x.w + border * 2;
    let mut out = Tensor::zeros(x.c, nh, nw);
    out.data.fill(v);
    for c in 0..x.c {
        for y in 0..x.h {
            for xx in 0..x.w {
                let dst = c * nh * nw + (y + border) * nw + (xx + border);
                let src = c * x.h * x.w + y * x.w + xx;
                out.data[dst] = x.data[src];
            }
        }
    }
    out
}

pub struct SeamBlending {
    pub param: SeamParam,
    pixels: Tensor,
    weights: Tensor,
    filter: Tensor,
}

impl SeamBlending {
    pub fn new(x_h: usize, x_w: usize, scale: u32, offset: u32, tile_size: u32) -> Self {
        let param = calc_parameters(x_h, x_w, scale, offset, tile_size, BLEND_SIZE);
        let pixels = Tensor::zeros(3, param.y_buffer_h, param.y_buffer_w);
        let weights = Tensor::zeros(3, param.y_buffer_h, param.y_buffer_w);
        let filter = create_blend_filter(scale, offset, tile_size, BLEND_SIZE);
        Self {
            param,
            pixels,
            weights,
            filter,
        }
    }

    pub fn nbytes(&self) -> u64 {
        self.pixels.nbytes() + self.weights.nbytes() + self.filter.nbytes()
    }

    pub fn update(&mut self, tile: &Tensor, tile_i: usize, tile_j: usize) {
        let step = self.param.output_tile_step;
        let h_i = step * tile_i;
        let w_i = step * tile_j;
        let (c, h, w) = (tile.c, tile.h, tile.w);
        let bh = self.pixels.h;
        let bw = self.pixels.w;
        let bhw = bh * bw;
        let thw = h * w;
        for cc in 0..c {
            for i in 0..h {
                for j in 0..w {
                    let ti = cc * thw + i * w + j;
                    let bi = cc * bhw + (h_i + i) * bw + (w_i + j);
                    let old = self.weights.data[bi];
                    let next = old + self.filter.data[ti];
                    let old_n = if next > 0.0 { old / next } else { 0.0 };
                    let new_n = 1.0 - old_n;
                    self.pixels.data[bi] = self.pixels.data[bi] * old_n + tile.data[ti] * new_n;
                    self.weights.data[bi] += self.filter.data[ti];
                }
            }
        }
    }

    pub fn finish(self) -> Tensor {
        let y_h = self.param.y_h;
        let y_w = self.param.y_w;
        let src_h = self.pixels.h;
        let src_w = self.pixels.w;
        drop(self.weights);
        drop(self.filter);
        let mut pixels = self.pixels;
        if src_h == y_h && src_w == y_w {
            for v in &mut pixels.data {
                *v = (*v).clamp(0.0, 1.0);
            }
            return pixels;
        }
        let mut data = std::mem::take(&mut pixels.data);
        for c in 0..3 {
            for y in 0..y_h {
                let src = c * src_h * src_w + y * src_w;
                let dst = c * y_h * y_w + y * y_w;
                if dst != src {
                    data.copy_within(src..src + y_w, dst);
                }
            }
        }
        data.truncate(3 * y_h * y_w);
        data.shrink_to_fit();
        for v in &mut data {
            *v = (*v).clamp(0.0, 1.0);
        }
        Tensor {
            data,
            c: 3,
            h: y_h,
            w: y_w,
        }
    }
}
