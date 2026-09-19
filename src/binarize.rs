//! 修复后二值化并写成 1-bit PNG, 用来压体积.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use image::RgbImage;
use png::{BitDepth, ColorType, Compression, Encoder};

use crate::error::Error;

/// Rec.601 亮度, Otsu 阈值, 墨迹为 0 / 纸为 1, 打包成 PNG 1-bit.
pub fn save_binary_png(rgb: &RgbImage, path: &Path) -> Result<(), Error> {
    let w = rgb.width();
    let h = rgb.height();
    let luma = luma8(rgb);
    let thr = otsu(&luma);
    let packed = pack_l1(&luma, w, h, thr);
    let file = File::create(path).map_err(|e| Error::msg(format!("写 {}: {e}", path.display())))?;
    let mut encoder = Encoder::new(BufWriter::new(file), w, h);
    encoder.set_color(ColorType::Grayscale);
    encoder.set_depth(BitDepth::One);
    encoder.set_compression(Compression::Fast);
    let mut writer = encoder
        .write_header()
        .map_err(|e| Error::msg(format!("写 1-bit PNG {}: {e}", path.display())))?;
    writer
        .write_image_data(&packed)
        .map_err(|e| Error::msg(format!("写 1-bit PNG {}: {e}", path.display())))?;
    Ok(())
}

/// 封面/照片色度方差大, 谱面 (含发黄纸) 几乎是同一色偏.
pub fn is_colorful(rgb: &RgbImage) -> bool {
    let n_px = rgb.width().saturating_mul(rgb.height()) as usize;
    if n_px == 0 {
        return false;
    }
    let step = (n_px / 12_000).max(1);
    let mut n = 0i64;
    let mut sum_cr = 0i64;
    let mut sum_cb = 0i64;
    let mut samples = Vec::new();
    for px in rgb.as_raw().chunks_exact(3).step_by(step) {
        let cr = px[0] as i32 - px[1] as i32;
        let cb = px[1] as i32 - px[2] as i32;
        sum_cr += cr as i64;
        sum_cb += cb as i64;
        samples.push((cr, cb));
        n += 1;
    }
    if n == 0 {
        return false;
    }
    let mcr = sum_cr / n;
    let mcb = sum_cb / n;
    let mut var = 0i64;
    for (cr, cb) in samples {
        let dcr = cr as i64 - mcr;
        let dcb = cb as i64 - mcb;
        var += dcr * dcr + dcb * dcb;
    }
    var / n > 150
}

pub(crate) fn luma8(rgb: &RgbImage) -> Vec<u8> {
    let src = rgb.as_raw();
    let mut out = Vec::with_capacity((rgb.width() * rgb.height()) as usize);
    for px in src.chunks_exact(3) {
        let y = (77u32 * px[0] as u32 + 150 * px[1] as u32 + 29 * px[2] as u32) >> 8;
        out.push(y as u8);
    }
    out
}

fn otsu(luma: &[u8]) -> u8 {
    let mut hist = [0u32; 256];
    for &p in luma {
        hist[p as usize] += 1;
    }
    let n = luma.len().max(1) as f64;
    let mut sum_all = 0.0;
    for (i, &c) in hist.iter().enumerate() {
        sum_all += i as f64 * c as f64;
    }
    let mut w_b = 0.0;
    let mut sum_b = 0.0;
    let mut best = 0.0;
    let mut thr = 128u8;
    for t in 0..256 {
        w_b += hist[t] as f64;
        if w_b <= 0.0 {
            continue;
        }
        let w_f = n - w_b;
        if w_f <= 0.0 {
            break;
        }
        sum_b += t as f64 * hist[t] as f64;
        let m_b = sum_b / w_b;
        let m_f = (sum_all - sum_b) / w_f;
        let var = w_b * w_f * (m_b - m_f) * (m_b - m_f);
        if var > best {
            best = var;
            thr = t as u8;
        }
    }
    thr
}

fn pack_l1(luma: &[u8], w: u32, h: u32, thr: u8) -> Vec<u8> {
    let row_bytes = ((w as usize) + 7) / 8;
    let mut out = vec![0u8; row_bytes * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            // PNG 灰度 1=浅, 0=深: 高于阈值当纸 (白).
            if luma[y * w as usize + x] > thr {
                let bit = 7 - (x % 8);
                out[y * row_bytes + x / 8] |= 1 << bit;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{otsu, save_binary_png};
    use image::RgbImage;

    #[test]
    fn otsu_splits_two_peaks() {
        let mut luma = vec![10u8; 100];
        luma.extend(std::iter::repeat(200u8).take(100));
        let t = otsu(&luma);
        assert!(10 <= t && 200 > t, "thr={t} should separate 10 and 200");
    }

    #[test]
    fn colorful_cover_vs_yellow_score() {
        let mut cover = RgbImage::new(32, 32);
        for (i, p) in cover.pixels_mut().enumerate() {
            *p = image::Rgb([
                (i % 251) as u8,
                ((i * 9) % 251) as u8,
                ((i * 17) % 251) as u8,
            ]);
        }
        assert!(super::is_colorful(&cover));

        let mut score = RgbImage::new(32, 32);
        for (i, p) in score.pixels_mut().enumerate() {
            let ink = i % 17 == 0;
            *p = if ink {
                image::Rgb([40, 32, 20])
            } else {
                image::Rgb([236, 228, 200])
            };
        }
        assert!(!super::is_colorful(&score));
    }

    #[test]
    fn writes_one_bit_png() {
        let mut img = RgbImage::new(16, 8);
        for p in img.pixels_mut() {
            *p = image::Rgb([255, 255, 255]);
        }
        img.put_pixel(0, 0, image::Rgb([0, 0, 0]));
        let path = std::env::temp_dir().join("waifu2x_repair_l1_test.png");
        save_binary_png(&img, &path).expect("write 1-bit png");
        let decoded = image::open(&path).expect("reopen").to_luma8();
        assert_eq!(decoded.dimensions(), (16, 8));
        assert!(decoded.get_pixel(0, 0)[0] < 128);
        assert!(decoded.get_pixel(1, 0)[0] > 128);
        let _ = std::fs::remove_file(path);
    }
}
