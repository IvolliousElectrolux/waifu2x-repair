//! PDF 渲染 / 抽图. pdfium 动态库查找方式对齐 score_sync.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use image::RgbImage;
use pdfium_render::prelude::*;

use crate::error::Error;

pub const DEFAULT_PDF_SCALE: f32 = 3.0;
pub const PDF_MAX_SIDE_PX: u32 = 8192;
pub const PDF_MIN_SCALE: f32 = 0.5;
pub const PDF_MAX_SCALE: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageKind {
    /// 页几乎就是一张嵌入图: 直接抽像素, 不光栅化.
    Image,
    /// 无有效位图, 矢量已足够清晰, 跳过修复.
    Vector,
    /// 混排/扫描需要光栅化: 导入时选分辨率.
    Raster,
}

#[derive(Clone, Debug)]
pub struct PdfPageInfo {
    pub page: u32,
    pub w_pt: f32,
    pub h_pt: f32,
    pub kind: PageKind,
    pub image_px: Option<(u32, u32)>,
}

#[derive(Clone, Debug)]
pub struct PdfSizeGroup {
    pub w_pt: f32,
    pub h_pt: f32,
    pub pages: Vec<u32>,
    pub image_px: Option<(u32, u32)>,
}

impl PdfSizeGroup {
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
}

#[derive(Clone, Debug)]
pub struct PdfInspect {
    pub path: PathBuf,
    pub name: String,
    pub page_count: usize,
    pub groups: Vec<PdfSizeGroup>,
    pub pages: Vec<PdfPageInfo>,
}

static PDF_TMP_DIRS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn cleanup_pdf_tmps() {
    if let Ok(mut dirs) = PDF_TMP_DIRS.lock() {
        for d in dirs.drain(..) {
            let _ = std::fs::remove_dir_all(&d);
        }
    }
}

fn lib_name() -> &'static str {
    #[cfg(windows)]
    {
        "pdfium.dll"
    }
    #[cfg(target_os = "macos")]
    {
        "libpdfium.dylib"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "libpdfium.so"
    }
}

fn find_in_path_env() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let name = lib_name();
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn find_pdfium_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PDFIUM_DYNAMIC_LIB_PATH") {
        let pb = PathBuf::from(&p);
        if pb.is_file() {
            return Some(pb);
        }
        let dll = pb.join(lib_name());
        if dll.is_file() {
            return Some(dll);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in [dir.join(lib_name()), dir.join("pdfium").join(lib_name())] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    find_in_path_env()
}

pub fn bind_pdfium() -> Result<Pdfium, Error> {
    let path = find_pdfium_path().ok_or_else(|| Error::PdfiumMissing {
        lib: lib_name().to_string(),
    })?;
    let dir = path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&dir))
        .or_else(|_| Pdfium::bind_to_library(&path))
        .map_err(|e| Error::PdfiumLoad {
            path: path.clone(),
            detail: e.to_string(),
        })?;
    Ok(Pdfium::new(bindings))
}

fn size_key(w: f32, h: f32) -> (i32, i32) {
    ((w * 2.0).round() as i32, (h * 2.0).round() as i32)
}

fn classify_page(page: &PdfPage<'_>) -> (PageKind, Option<(u32, u32)>) {
    let page_w = page.width().value.max(1.0);
    let page_h = page.height().value.max(1.0);
    let page_area = (page_w * page_h).max(1.0);
    let mut best: Option<(u32, u32, f32)> = None;
    let mut image_area = 0.0f32;
    let mut other = 0u32;
    for obj in page.objects().iter() {
        if let Some(img) = obj.as_image_object() {
            // 只读 metadata, 不要 get_raw_bitmap: 整本扫描 PDF 会把每一页都解码进内存.
            let (w, h) = match (img.width(), img.height()) {
                (Ok(w), Ok(h)) => (w.max(0) as u32, h.max(0) as u32),
                _ => {
                    other += 1;
                    continue;
                }
            };
            if w < 32 || h < 32 {
                continue;
            }
            let cover = obj
                .bounds()
                .map(|b| b.width().value.max(0.0) * b.height().value.max(0.0))
                .unwrap_or(0.0);
            image_area += cover;
            let better = best
                .map(|(_, _, a)| cover > a || (cover == a && w.saturating_mul(h) > 0))
                .unwrap_or(true);
            if better {
                best = Some((w, h, cover));
            }
        } else {
            other += 1;
        }
    }
    let image_px = best.map(|(w, h, _)| (w, h));
    if best.is_none() {
        return (PageKind::Vector, None);
    }
    let cover_ratio = image_area / page_area;
    if cover_ratio >= 0.82 && other < 12 {
        (PageKind::Image, image_px)
    } else {
        (PageKind::Raster, image_px)
    }
}

pub fn inspect_pdf(pdf_path: &Path) -> Result<PdfInspect, Error> {
    let pdfium = bind_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(pdf_path, None)
        .map_err(|e| Error::PdfOpen(e.to_string()))?;
    let n = document.pages().len() as usize;
    if n == 0 {
        return Err(Error::PdfOpen(format!("{} 没有页面.", pdf_path.display())));
    }
    let sizes = document
        .pages()
        .page_sizes()
        .map_err(|e| Error::PdfOpen(e.to_string()))?;

    let mut pages = Vec::with_capacity(n);
    let mut buckets: Vec<((i32, i32), PdfSizeGroup)> = Vec::new();
    for (i, rect) in sizes.iter().enumerate() {
        let w = rect.width().value.max(1.0);
        let h = rect.height().value.max(1.0);
        let page_no = (i as u32) + 1;
        let (kind, image_px) = document
            .pages()
            .get(i as u16)
            .map(|p| classify_page(&p))
            .unwrap_or((PageKind::Raster, None));
        pages.push(PdfPageInfo {
            page: page_no,
            w_pt: w,
            h_pt: h,
            kind,
            image_px,
        });
        let key = size_key(w, h);
        if let Some((_, g)) = buckets.iter_mut().find(|(k, _)| *k == key) {
            g.pages.push(page_no);
            if g.image_px.is_none() {
                g.image_px = image_px;
            }
        } else {
            buckets.push((
                key,
                PdfSizeGroup {
                    w_pt: w,
                    h_pt: h,
                    pages: vec![page_no],
                    image_px,
                },
            ));
        }
    }
    buckets.sort_by(|a, b| b.1.page_count().cmp(&a.1.page_count()));
    let name = pdf_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("pdf")
        .to_string();
    Ok(PdfInspect {
        path: pdf_path.to_path_buf(),
        name,
        page_count: n,
        groups: buckets.into_iter().map(|(_, g)| g).collect(),
        pages,
    })
}

pub fn render_pdf_page_preview(
    pdf_path: &Path,
    page_1based: u32,
    max_side: u32,
) -> Result<RgbImage, Error> {
    let pdfium = bind_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(pdf_path, None)
        .map_err(|e| Error::PdfOpen(e.to_string()))?;
    let n = document.pages().len() as u32;
    if page_1based == 0 || page_1based > n {
        return Err(Error::PdfOpen(format!(
            "{} 没有第 {page_1based} 页 (共 {n} 页).",
            pdf_path.display()
        )));
    }
    let page = document
        .pages()
        .get((page_1based - 1) as u16)
        .map_err(|e| Error::msg(format!("读取第 {page_1based} 页失败: {e}")))?;
    let w_pt = page.width().value.max(1.0);
    let h_pt = page.height().value.max(1.0);
    let cap = max_side.max(64) as f32;
    let scale = (cap / w_pt).min(cap / h_pt).clamp(0.05, PDF_MAX_SCALE);
    let cfg = PdfRenderConfig::new()
        .scale_page_width_by_factor(scale)
        .scale_page_height_by_factor(scale);
    let image = page
        .render_with_config(&cfg)
        .map_err(|e| Error::msg(format!("渲染第 {page_1based} 页失败: {e}")))?
        .as_image()
        .into_rgb8();
    Ok(image)
}

pub fn clamp_pdf_scale(scale: f32) -> f32 {
    scale.clamp(PDF_MIN_SCALE, PDF_MAX_SCALE)
}

pub fn px_from_pt(pt: f32, scale: f32) -> u32 {
    let v = (pt * scale).round();
    (v as u32).clamp(1, PDF_MAX_SIDE_PX)
}

pub fn scale_from_target(pt: f32, px: u32) -> f32 {
    if pt < 0.5 {
        return DEFAULT_PDF_SCALE;
    }
    clamp_pdf_scale(px as f32 / pt)
}

pub fn parse_page_selection(input: &str, total: usize) -> Vec<u32> {
    if total == 0 {
        return Vec::new();
    }
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return (1..=total as u32).collect();
    }
    let normalized: String = trimmed
        .chars()
        .filter_map(|c| match c {
            c if c.is_whitespace() => None,
            '，' => Some(','),
            '－' | '—' | '～' | '~' => Some('-'),
            '\u{FF10}'..='\u{FF19}' => char::from_u32(c as u32 - 0xFF10 + '0' as u32),
            c => Some(c),
        })
        .collect();
    let mut set = std::collections::BTreeSet::new();
    for part in normalized.split(',') {
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) {
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                for i in lo..=hi {
                    if i >= 1 && (i as usize) <= total {
                        set.insert(i);
                    }
                }
            }
        } else if let Ok(a) = part.parse::<u32>() {
            if a >= 1 && (a as usize) <= total {
                set.insert(a);
            }
        }
    }
    set.into_iter().collect()
}

fn extract_largest_image(page: &PdfPage<'_>) -> Option<RgbImage> {
    let mut best: Option<(u32, RgbImage)> = None;
    for obj in page.objects().iter() {
        let Some(img) = obj.as_image_object() else {
            continue;
        };
        let rgb = img.get_raw_image().ok()?.into_rgb8();
        let area = rgb.width().saturating_mul(rgb.height());
        if area < 32 * 32 {
            continue;
        }
        let take = best.as_ref().map(|(a, _)| area > *a).unwrap_or(true);
        if take {
            best = Some((area, rgb));
        }
    }
    best.map(|(_, i)| i)
}

pub fn materialize_page(
    pdf_path: &Path,
    page_1based: u32,
    kind: PageKind,
    scale_x: f32,
    scale_y: f32,
) -> Result<RgbImage, Error> {
    let pdfium = bind_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(pdf_path, None)
        .map_err(|e| Error::PdfOpen(e.to_string()))?;
    let n = document.pages().len() as u32;
    if page_1based == 0 || page_1based > n {
        return Err(Error::PdfOpen(format!(
            "{} 没有第 {page_1based} 页.",
            pdf_path.display()
        )));
    }
    let page = document
        .pages()
        .get((page_1based - 1) as u16)
        .map_err(|e| Error::msg(format!("读取第 {page_1based} 页失败: {e}")))?;
    match kind {
        PageKind::Vector => Err(Error::msg("矢量页无需修复")),
        PageKind::Image => extract_largest_image(&page)
            .ok_or_else(|| Error::msg(format!("第 {page_1based} 页抽图失败"))),
        PageKind::Raster => {
            let sx = scale_x.max(PDF_MIN_SCALE);
            let sy = scale_y.max(PDF_MIN_SCALE);
            let cfg = PdfRenderConfig::new()
                .scale_page_width_by_factor(sx)
                .scale_page_height_by_factor(sy);
            Ok(page
                .render_with_config(&cfg)
                .map_err(|e| Error::msg(format!("渲染第 {page_1based} 页失败: {e}")))?
                .as_image()
                .into_rgb8())
        }
    }
}

pub fn register_tmp_dir(dir: PathBuf) {
    if let Ok(mut dirs) = PDF_TMP_DIRS.lock() {
        dirs.push(dir);
    }
}
