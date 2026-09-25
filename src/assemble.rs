//! 把修好的页合成 PDF: 1-bit 用 DeviceGray Flate.
//! 封面在 JPEG 和源 PNG 的 IDAT 里选更小的一份嵌入, 避免合成后比单张图更大.
//! 不用 pdfium SetBitmap (会扩成 RGBA + SMask, 体积和页高都会炸).

use std::fs::File;
use std::io::{BufReader, Write};
use std::path::Path;

use image::RgbImage;
use miniz_oxide::deflate::{compress_to_vec_zlib, CompressionLevel};
use pdf_writer::types::Predictor;
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref};
use png::{BitDepth, ColorType, Transformations};

use crate::binarize;
use crate::error::Error;

/// q<90 时 jpeg-encoder 默认 4:2:0, 和 Acrobat 把 PNG 收成 PDF 同一档.
const JPEG_QUALITY: u8 = 85;

enum Embedded {
    Bitonal { w: u32, h: u32, data: Vec<u8> },
    Gray8 { w: u32, h: u32, data: Vec<u8> },
    Jpeg {
        w: u32,
        h: u32,
        gray: bool,
        data: Vec<u8>,
    },
    /// 源 PNG 的 IDAT (zlib), PDF Predictor 15 按行还原, 体积跟单张 PNG 同一量级.
    PngIdat {
        w: u32,
        h: u32,
        colors: i32,
        data: Vec<u8>,
    },
}

pub fn write_image_pdf(
    pages: &[(f32, f32, &Path)],
    dest: &Path,
) -> Result<(), Error> {
    if pages.is_empty() {
        return Err(Error::msg("没有可合成的页"));
    }
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let pages_id = Ref::new(2);
    let mut next = 3i32;
    let mut page_ids = Vec::with_capacity(pages.len());

    for (w_pt, h_pt, img_path) in pages {
        let embedded = load_embedded(img_path)?;
        let page_id = Ref::new(next);
        next += 1;
        let content_id = Ref::new(next);
        next += 1;
        let image_id = Ref::new(next);
        next += 1;
        page_ids.push(page_id);

        let (w_pt, h_pt) = page_size(*w_pt, *h_pt, &embedded);
        let image_name = Name(b"Im0");

        match &embedded {
            Embedded::Bitonal { w, h, data } => {
                let mut image = pdf.image_xobject(image_id, data);
                image.filter(Filter::FlateDecode);
                image.width(*w as i32);
                image.height(*h as i32);
                image.color_space().device_gray();
                image.bits_per_component(1);
                image.interpolate(false);
                image.finish();
            }
            Embedded::Gray8 { w, h, data } => {
                let mut image = pdf.image_xobject(image_id, data);
                image.filter(Filter::FlateDecode);
                image.width(*w as i32);
                image.height(*h as i32);
                image.color_space().device_gray();
                image.bits_per_component(8);
                image.interpolate(false);
                image.finish();
            }
            Embedded::Jpeg { w, h, gray, data } => {
                let mut image = pdf.image_xobject(image_id, data);
                image.filter(Filter::DctDecode);
                image.width(*w as i32);
                image.height(*h as i32);
                if *gray {
                    image.color_space().device_gray();
                } else {
                    image.color_space().device_rgb();
                }
                image.bits_per_component(8);
                image.interpolate(true);
                image.finish();
            }
            Embedded::PngIdat { w, h, colors, data } => {
                let mut image = pdf.image_xobject(image_id, data);
                image.filter(Filter::FlateDecode);
                image
                    .decode_parms()
                    .predictor(Predictor::PngOptimum)
                    .colors(*colors)
                    .bits_per_component(8)
                    .columns(*w as i32);
                image.width(*w as i32);
                image.height(*h as i32);
                if *colors == 1 {
                    image.color_space().device_gray();
                } else {
                    image.color_space().device_rgb();
                }
                image.bits_per_component(8);
                image.interpolate(false);
                image.finish();
            }
        }

        let mut page = pdf.page(page_id);
        page.media_box(Rect::new(0.0, 0.0, w_pt, h_pt));
        page.parent(pages_id);
        page.contents(content_id);
        page.resources().x_objects().pair(image_name, image_id);
        page.finish();

        let mut content = Content::new();
        content.save_state();
        content.transform([w_pt, 0.0, 0.0, h_pt, 0.0, 0.0]);
        content.x_object(image_name);
        content.restore_state();
        pdf.stream(content_id, &content.finish());
    }

    pdf.catalog(catalog_id).pages(pages_id);
    pdf.pages(pages_id)
        .kids(page_ids)
        .count(pages.len() as i32);

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::msg(e.to_string()))?;
    }
    let tmp = dest.with_extension("pdf.part");
    let bytes = pdf.finish();
    {
        let mut f = File::create(&tmp).map_err(|e| Error::msg(format!("写 {}: {e}", tmp.display())))?;
        f.write_all(&bytes)
            .map_err(|e| Error::msg(format!("写 {}: {e}", tmp.display())))?;
    }
    if dest.exists() {
        let _ = std::fs::remove_file(dest);
    }
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::msg(format!("写 {}: {e}", dest.display()))
    })?;
    Ok(())
}

fn page_size(w_pt: f32, h_pt: f32, img: &Embedded) -> (f32, f32) {
    if w_pt >= 1.0 && h_pt >= 1.0 {
        return (w_pt, h_pt);
    }
    let (w, h) = match img {
        Embedded::Bitonal { w, h, .. }
        | Embedded::Gray8 { w, h, .. }
        | Embedded::Jpeg { w, h, .. }
        | Embedded::PngIdat { w, h, .. } => (*w, *h),
    };
    (
        (w as f32) * 72.0 / 150.0,
        (h as f32) * 72.0 / 150.0,
    )
}

fn load_embedded(path: &Path) -> Result<Embedded, Error> {
    if let Some(bitonal) = try_read_png_l1(path)? {
        return Ok(bitonal);
    }
    let rgb = image::open(path)
        .map_err(|e| Error::ImageOpen {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })?
        .to_rgb8();
    if binarize::is_photo(&rgb) {
        let jpeg = encode_jpeg(&rgb)?;
        if let Some(png) = try_read_png_idat(path)? {
            let jpeg_len = match &jpeg {
                Embedded::Jpeg { data, .. } => data.len(),
                _ => usize::MAX,
            };
            let png_len = match &png {
                Embedded::PngIdat { data, .. } => data.len(),
                _ => usize::MAX,
            };
            if png_len <= jpeg_len {
                return Ok(png);
            }
        }
        Ok(jpeg)
    } else {
        encode_gray8(&rgb)
    }
}

/// 8-bit 非隔行 RGB / 灰度 PNG 的 IDAT 可直接当 FlateDecode + Predictor 15.
fn try_read_png_idat(path: &Path) -> Result<Option<Embedded>, Error> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return Err(Error::ImageOpen {
                path: path.to_path_buf(),
                detail: e.to_string(),
            });
        }
    };
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 8 || &bytes[..8] != SIG {
        return Ok(None);
    }
    let mut i = 8usize;
    let mut w = 0u32;
    let mut h = 0u32;
    let mut depth = 0u8;
    let mut color = 0u8;
    let mut interlace = 1u8;
    let mut saw_ihdr = false;
    let mut idat = Vec::new();
    while i + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let typ = &bytes[i + 4..i + 8];
        let start = i + 8;
        let end = start.saturating_add(len);
        if end + 4 > bytes.len() {
            return Ok(None);
        }
        if typ == b"IHDR" && len >= 13 {
            w = u32::from_be_bytes(bytes[start..start + 4].try_into().unwrap());
            h = u32::from_be_bytes(bytes[start + 4..start + 8].try_into().unwrap());
            depth = bytes[start + 8];
            color = bytes[start + 9];
            interlace = bytes[start + 12];
            saw_ihdr = true;
        } else if typ == b"IDAT" {
            idat.extend_from_slice(&bytes[start..end]);
        } else if typ == b"IEND" {
            break;
        }
        i = end + 4;
    }
    let colors = match color {
        0 => 1,
        2 => 3,
        _ => return Ok(None),
    };
    if !saw_ihdr || depth != 8 || interlace != 0 || w == 0 || h == 0 || idat.is_empty() {
        return Ok(None);
    }
    Ok(Some(Embedded::PngIdat {
        w,
        h,
        colors,
        data: idat,
    }))
}

fn try_read_png_l1(path: &Path) -> Result<Option<Embedded>, Error> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            return Err(Error::ImageOpen {
                path: path.to_path_buf(),
                detail: e.to_string(),
            });
        }
    };
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(Transformations::IDENTITY);
    let mut reader = match decoder.read_info() {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    let info = reader.info();
    if info.color_type != ColorType::Grayscale || info.bit_depth != BitDepth::One {
        return Ok(None);
    }
    let w = info.width;
    let h = info.height;
    let buf_len = reader
        .output_buffer_size()
        .ok_or_else(|| Error::msg(format!("读 1-bit PNG {}: 尺寸无效", path.display())))?;
    let mut buf = vec![0u8; buf_len];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| Error::msg(format!("读 1-bit PNG {}: {e}", path.display())))?;
    buf.truncate(frame.buffer_size());
    let data = compress_to_vec_zlib(&buf, CompressionLevel::BestCompression as u8);
    Ok(Some(Embedded::Bitonal { w, h, data }))
}

fn encode_gray8(rgb: &RgbImage) -> Result<Embedded, Error> {
    let luma = binarize::luma8(rgb);
    let data = compress_to_vec_zlib(&luma, CompressionLevel::BestCompression as u8);
    Ok(Embedded::Gray8 {
        w: rgb.width(),
        h: rgb.height(),
        data,
    })
}

fn encode_jpeg(rgb: &RgbImage) -> Result<Embedded, Error> {
    let w = rgb.width();
    let h = rgb.height();
    let wu = u16::try_from(w).map_err(|_| Error::msg(format!("封面宽 {w}px 超过 JPEG 上限")))?;
    let hu = u16::try_from(h).map_err(|_| Error::msg(format!("封面高 {h}px 超过 JPEG 上限")))?;
    let gray = !binarize::is_colorful(rgb);
    let mut data = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut data, JPEG_QUALITY);
    if gray {
        let luma = binarize::luma8(rgb);
        encoder
            .encode(&luma, wu, hu, jpeg_encoder::ColorType::Luma)
            .map_err(|e| Error::msg(format!("JPEG 编码: {e}")))?;
    } else {
        encoder
            .encode(rgb.as_raw(), wu, hu, jpeg_encoder::ColorType::Rgb)
            .map_err(|e| Error::msg(format!("JPEG 编码: {e}")))?;
    }
    Ok(Embedded::Jpeg { w, h, gray, data })
}

#[cfg(test)]
mod tests {
    use super::write_image_pdf;
    use crate::binarize::save_binary_png;
    use image::{Rgb, RgbImage};
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("w2x_assemble_{name}"));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn bitonal_png_stays_device_gray_1bit() {
        let mut img = RgbImage::new(32, 16);
        for p in img.pixels_mut() {
            *p = Rgb([255, 255, 255]);
        }
        img.put_pixel(0, 0, Rgb([0, 0, 0]));
        let png = tmp("l1.png");
        save_binary_png(&img, &png).unwrap();
        let dest = tmp("l1.pdf");
        write_image_pdf(&[(595.27, 841.89, png.as_path())], &dest).unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("/DeviceGray"), "{s}");
        assert!(s.contains("/BitsPerComponent 1"), "{s}");
        assert!(s.contains("/FlateDecode"), "{s}");
        assert!(!s.contains("/SMask"), "{s}");
        assert!(s.contains("595.27"), "{s}");
        assert!(s.contains("841.89"), "{s}");
        assert!(bytes.len() < 8 * 1024, "1-bit page should be tiny, got {}", bytes.len());
        let _ = std::fs::remove_file(png);
        let _ = std::fs::remove_file(dest);
    }

    #[test]
    fn mixed_cover_and_score_keep_source_page_boxes() {
        let mut cover = RgbImage::new(48, 64);
        for (i, p) in cover.pixels_mut().enumerate() {
            *p = Rgb([
                (i % 251) as u8,
                ((i * 5) % 251) as u8,
                ((i * 11) % 251) as u8,
            ]);
        }
        let mut score = RgbImage::new(48, 64);
        for p in score.pixels_mut() {
            *p = Rgb([255, 255, 255]);
        }
        score.put_pixel(2, 2, Rgb([0, 0, 0]));
        let cover_png = tmp("mix_cover.png");
        let score_png = tmp("mix_score.png");
        cover.save(&cover_png).unwrap();
        save_binary_png(&score, &score_png).unwrap();
        let dest = tmp("mix.pdf");
        write_image_pdf(
            &[
                (595.3, 841.9, cover_png.as_path()),
                (595.3, 841.9, score_png.as_path()),
            ],
            &dest,
        )
        .unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("/DCTDecode"), "{s}");
        assert!(s.contains("/BitsPerComponent 1"), "{s}");
        assert!(s.contains("595.3"), "{s}");
        assert!(s.contains("841.9"), "{s}");
        assert!(!s.contains("922"), "{s}");
        assert!(!s.contains("/SMask"), "{s}");
        let _ = std::fs::remove_file(cover_png);
        let _ = std::fs::remove_file(score_png);
        let _ = std::fs::remove_file(dest);
    }

    #[test]
    fn color_png_becomes_jpeg_on_source_page_size() {
        let mut img = RgbImage::new(64, 48);
        for (i, p) in img.pixels_mut().enumerate() {
            *p = Rgb([
                (i % 251) as u8,
                ((i * 7) % 251) as u8,
                ((i * 13) % 251) as u8,
            ]);
        }
        let png = tmp("color.png");
        img.save(&png).unwrap();
        let dest = tmp("color.pdf");
        write_image_pdf(&[(595.3, 841.9, png.as_path())], &dest).unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("/DCTDecode"), "{s}");
        assert!(s.contains("/DeviceRGB"), "{s}");
        assert!(!s.contains("/DeviceGray"), "{s}");
        assert!(!s.contains("/SMask"), "{s}");
        assert!(s.contains("595.3"), "{s}");
        assert!(s.contains("841.9"), "{s}");
        assert!(!s.contains("922"), "{s}");
        let _ = std::fs::remove_file(png);
        let _ = std::fs::remove_file(dest);
    }

    #[test]
    fn gray_photo_becomes_gray_jpeg() {
        let mut img = RgbImage::new(180, 240);
        for (i, p) in img.pixels_mut().enumerate() {
            let y = ((i * 13) % 220) as u8;
            *p = Rgb([y, y, y]);
        }
        let png = tmp("gray_photo.png");
        img.save(&png).unwrap();
        let dest = tmp("gray_photo.pdf");
        write_image_pdf(&[(595.3, 841.9, png.as_path())], &dest).unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("/DCTDecode"), "{s}");
        assert!(s.contains("/DeviceGray"), "{s}");
        assert!(!s.contains("/DeviceRGB"), "{s}");
        assert!(!s.contains("/BitsPerComponent 1"), "{s}");
        let raw = (img.width() * img.height()) as usize;
        assert!(bytes.len() < raw, "jpeg pdf {} should be under raw {raw}", bytes.len());
        let _ = std::fs::remove_file(png);
        let _ = std::fs::remove_file(dest);
    }

    #[test]
    fn line_art_rgb_stays_gray_flate() {
        let mut img = RgbImage::new(64, 48);
        for p in img.pixels_mut() {
            *p = Rgb([248, 244, 236]);
        }
        img.put_pixel(3, 3, Rgb([20, 16, 12]));
        let png = tmp("line.png");
        img.save(&png).unwrap();
        let dest = tmp("line.pdf");
        write_image_pdf(&[(595.3, 841.9, png.as_path())], &dest).unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("/FlateDecode"), "{s}");
        assert!(s.contains("/DeviceGray"), "{s}");
        assert!(s.contains("/BitsPerComponent 8"), "{s}");
        assert!(!s.contains("/DCTDecode"), "{s}");
        let _ = std::fs::remove_file(png);
        let _ = std::fs::remove_file(dest);
    }

    #[test]
    fn flat_cover_keeps_png_when_smaller_than_jpeg() {
        let mut img = RgbImage::new(360, 240);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let band = (x / 40) % 6;
            *p = match band {
                0 => Rgb([220, 40, 40]),
                1 => Rgb([40, 160, 60]),
                2 => Rgb([40, 80, 200]),
                3 => Rgb([230, 180, 40]),
                4 => Rgb([180, 40, 160]),
                _ => Rgb([240, 240, 235]),
            };
            let _ = y;
        }
        let png = tmp("flat_cover.png");
        img.save(&png).unwrap();
        let png_len = std::fs::metadata(&png).unwrap().len();
        let dest = tmp("flat_cover.pdf");
        write_image_pdf(&[(595.3, 841.9, png.as_path())], &dest).unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("/Predictor 15"), "{s}");
        assert!(s.contains("/FlateDecode"), "{s}");
        assert!(!s.contains("/DCTDecode"), "{s}");
        assert!(
            bytes.len() as u64 <= png_len + 4096,
            "pdf {} should stay near png {png_len}",
            bytes.len()
        );
        let _ = std::fs::remove_file(png);
        let _ = std::fs::remove_file(dest);
    }
}
