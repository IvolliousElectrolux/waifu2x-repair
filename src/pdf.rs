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

#[derive(Clone, Copy, Debug)]
struct PdfBox {
    left: f32,
    bottom: f32,
    right: f32,
    top: f32,
}

impl PdfBox {
    fn from_rect(r: &PdfRect) -> Self {
        Self {
            left: r.left().value,
            bottom: r.bottom().value,
            right: r.right().value,
            top: r.top().value,
        }
    }

    fn from_quad(q: &PdfQuadPoints) -> Self {
        Self {
            left: q.left().value,
            bottom: q.bottom().value,
            right: q.right().value,
            top: q.top().value,
        }
    }

    fn width(self) -> f32 {
        (self.right - self.left).max(0.0)
    }

    fn height(self) -> f32 {
        (self.top - self.bottom).max(0.0)
    }

    fn area(self) -> f32 {
        self.width() * self.height()
    }

    fn intersect(self, o: Self) -> Self {
        Self {
            left: self.left.max(o.left),
            bottom: self.bottom.max(o.bottom),
            right: self.right.min(o.right),
            top: self.top.min(o.top),
        }
    }
}

fn display_box(page: &PdfPage<'_>) -> PdfBox {
    PdfBox {
        left: 0.0,
        bottom: 0.0,
        right: page.width().value.max(1.0),
        top: page.height().value.max(1.0),
    }
}

fn abs_box_origin(page: &PdfPage<'_>) -> (f32, f32) {
    if let Ok(c) = page.boundaries().crop() {
        let b = c.bounds;
        if b.width().value > 1.0 && b.height().value > 1.0 {
            return (b.left().value, b.bottom().value);
        }
    }
    if let Ok(m) = page.boundaries().media() {
        let b = m.bounds;
        if b.width().value > 1.0 && b.height().value > 1.0 {
            return (b.left().value, b.bottom().value);
        }
    }
    (0.0, 0.0)
}

fn to_display(page: &PdfPage<'_>, abs: PdfBox) -> PdfBox {
    let (ox, oy) = abs_box_origin(page);
    display_box(page).intersect(PdfBox {
        left: abs.left - ox,
        bottom: abs.bottom - oy,
        right: abs.right - ox,
        top: abs.top - oy,
    })
}

fn view_box_abs(page: &PdfPage<'_>) -> PdfBox {
    if let Ok(c) = page.boundaries().crop() {
        let b = PdfBox::from_rect(&c.bounds);
        if b.width() > 1.0 && b.height() > 1.0 {
            return b;
        }
    }
    if let Ok(m) = page.boundaries().media() {
        let b = PdfBox::from_rect(&m.bounds);
        if b.width() > 1.0 && b.height() > 1.0 {
            return b;
        }
    }
    let d = display_box(page);
    let (ox, oy) = abs_box_origin(page);
    PdfBox {
        left: d.left + ox,
        bottom: d.bottom + oy,
        right: d.right + ox,
        top: d.top + oy,
    }
}

fn largest_image_box(page: &PdfPage<'_>) -> Option<PdfBox> {
    let mut best: Option<PdfBox> = None;
    let mut best_area = 0.0f32;
    for obj in page.objects().iter() {
        if obj.as_image_object().is_none() {
            continue;
        }
        let Ok(q) = obj.bounds() else {
            continue;
        };
        let b = PdfBox::from_quad(&q);
        let a = b.area();
        if a > best_area {
            best_area = a;
            best = Some(b);
        }
    }
    best
}

/// 实际谱面范围: CropBox (阅读器看到的), 若整页图没铺满再收到图块.
fn content_box(page: &PdfPage<'_>) -> PdfBox {
    let display = display_box(page);
    let mut vis = to_display(page, view_box_abs(page));
    if vis.width() < 1.0 || vis.height() < 1.0 {
        vis = display;
    }
    if let Some(img) = largest_image_box(page) {
        let mapped = to_display(page, img);
        let cover = mapped.area() / vis.area().max(1.0);
        if cover >= 0.40
            && (mapped.height() < vis.height() * 0.98 || mapped.width() < vis.width() * 0.98)
        {
            vis = mapped;
        }
    }
    if vis.width() < 1.0 || vis.height() < 1.0 {
        display
    } else {
        vis
    }
}

fn content_size(page: &PdfPage<'_>) -> (f32, f32) {
    let b = content_box(page);
    (b.width().max(1.0), b.height().max(1.0))
}

fn crop_px(img_w: u32, img_h: u32, src: PdfBox, vis: PdfBox) -> (u32, u32, u32, u32) {
    let vis = vis.intersect(src);
    let sw = src.width().max(1e-3);
    let sh = src.height().max(1e-3);
    let mut x0 = ((vis.left - src.left) / sw * img_w as f32).round() as i32;
    let mut x1 = ((vis.right - src.left) / sw * img_w as f32).round() as i32;
    let mut y0 = ((src.top - vis.top) / sh * img_h as f32).round() as i32;
    let mut y1 = ((src.top - vis.bottom) / sh * img_h as f32).round() as i32;
    let iw = img_w as i32;
    let ih = img_h as i32;
    x0 = x0.clamp(0, (iw - 1).max(0));
    y0 = y0.clamp(0, (ih - 1).max(0));
    x1 = x1.clamp(x0 + 1, iw.max(1));
    y1 = y1.clamp(y0 + 1, ih.max(1));
    (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32)
}

fn crop_rgb_to_box(rgb: RgbImage, src: PdfBox, vis: PdfBox) -> RgbImage {
    let (x, y, w, h) = crop_px(rgb.width(), rgb.height(), src, vis);
    if x == 0 && y == 0 && w == rgb.width() && h == rgb.height() {
        return rgb;
    }
    image::imageops::crop_imm(&rgb, x, y, w, h).to_image()
}

fn crop_rgb_display(rgb: RgbImage, page: &PdfPage<'_>, vis: PdfBox) -> RgbImage {
    crop_rgb_to_box(rgb, display_box(page), vis)
}

fn visible_image_px(w: u32, h: u32, img_box: Option<PdfBox>, vis_abs: PdfBox) -> (u32, u32) {
    let Some(src) = img_box else {
        return (w, h);
    };
    let (_, _, cw, ch) = crop_px(w, h, src, vis_abs);
    (cw.max(1), ch.max(1))
}

fn classify_page(page: &PdfPage<'_>) -> (PageKind, Option<(u32, u32)>) {
    let vis_abs = view_box_abs(page);
    let vis = to_display(page, vis_abs);
    let page_area = vis.area().max(1.0);
    let mut best: Option<(u32, u32, f32, Option<PdfBox>)> = None;
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
            let ib = obj.bounds().ok().map(|q| PdfBox::from_quad(&q));
            let cover = ib.map(PdfBox::area).unwrap_or(0.0);
            image_area += cover;
            let better = best
                .as_ref()
                .map(|(_, _, a, _)| cover > *a || (cover == *a && w.saturating_mul(h) > 0))
                .unwrap_or(true);
            if better {
                best = Some((w, h, cover, ib));
            }
        } else {
            other += 1;
        }
    }
    let image_px = best.map(|(w, h, _, ib)| visible_image_px(w, h, ib, vis_abs));
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
    let mut pages = Vec::with_capacity(n);
    let mut buckets: Vec<((i32, i32), PdfSizeGroup)> = Vec::new();
    for i in 0..n {
        let page_no = (i as u32) + 1;
        let (w, h, kind, image_px) = match document.pages().get(i as u16) {
            Ok(p) => {
                let (kind, image_px) = classify_page(&p);
                let (w, h) = content_size(&p);
                (w, h, kind, image_px)
            }
            Err(_) => (595.0, 842.0, PageKind::Raster, None),
        };
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
    render_visible(&page, preview_scale(&page, max_side))
        .map_err(|e| Error::msg(format!("渲染第 {page_1based} 页失败: {e}")))
}

fn preview_scale(page: &PdfPage<'_>, max_side: u32) -> (f32, f32) {
    let (w_pt, h_pt) = content_size(page);
    let cap = max_side.max(64) as f32;
    let scale = (cap / w_pt).min(cap / h_pt).clamp(0.05, PDF_MAX_SCALE);
    (scale, scale)
}

fn render_visible(page: &PdfPage<'_>, (sx, sy): (f32, f32)) -> Result<RgbImage, PdfiumError> {
    let sx = sx.max(PDF_MIN_SCALE);
    let sy = sy.max(PDF_MIN_SCALE);
    let cfg = PdfRenderConfig::new()
        .scale_page_width_by_factor(sx)
        .scale_page_height_by_factor(sy);
    let rgb = page.render_with_config(&cfg)?.as_image().into_rgb8();
    Ok(crop_rgb_display(rgb, page, content_box(page)))
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
    let vis = view_box_abs(page);
    let mut best: Option<(u32, PdfBox, RgbImage)> = None;
    for obj in page.objects().iter() {
        let Some(img) = obj.as_image_object() else {
            continue;
        };
        let rgb = img.get_raw_image().ok()?.into_rgb8();
        let area = rgb.width().saturating_mul(rgb.height());
        if area < 32 * 32 {
            continue;
        }
        let ib = obj
            .bounds()
            .ok()
            .map(|q| PdfBox::from_quad(&q))
            .unwrap_or(vis);
        let take = best.as_ref().map(|(a, _, _)| area > *a).unwrap_or(true);
        if take {
            best = Some((area, ib, rgb));
        }
    }
    best.map(|(_, ib, rgb)| crop_rgb_to_box(rgb, ib, vis))
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
            .or_else(|| render_visible(&page, (scale_x, scale_y)).ok())
            .ok_or_else(|| Error::msg(format!("第 {page_1based} 页抽图失败"))),
        PageKind::Raster => render_visible(&page, (scale_x, scale_y))
            .map_err(|e| Error::msg(format!("渲染第 {page_1based} 页失败: {e}"))),
    }
}

/// 按原 PDF 页序把修好的图合成一份 PDF, 纸张尺寸跟源页一致.
/// 1-bit 谱面保持 DeviceGray, 彩色封面走 JPEG, 不再经 pdfium 扩成 RGBA.
pub fn assemble_repaired_pdf(
    source_pdf: &Path,
    pages: &[(u32, PathBuf)],
    dest: &Path,
) -> Result<(), Error> {
    if pages.is_empty() {
        return Err(Error::msg("没有可合成的页"));
    }
    let sizes = read_page_sizes(source_pdf, pages)?;
    let plan: Vec<(f32, f32, &Path)> = pages
        .iter()
        .zip(sizes.iter())
        .map(|((_, png), (w, h))| (*w, *h, png.as_path()))
        .collect();
    crate::assemble::write_image_pdf(&plan, dest)
}

fn read_page_sizes(source_pdf: &Path, pages: &[(u32, PathBuf)]) -> Result<Vec<(f32, f32)>, Error> {
    let pdfium = bind_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(source_pdf, None)
        .map_err(|e| Error::PdfOpen(e.to_string()))?;
    let n = document.pages().len() as u32;
    let mut sizes = Vec::with_capacity(pages.len());
    for &(page_1based, _) in pages {
        if page_1based == 0 || page_1based > n {
            sizes.push((595.0, 842.0));
            continue;
        }
        match document.pages().get((page_1based - 1) as u16) {
            Ok(page) => sizes.push(content_size(&page)),
            Err(_) => sizes.push((595.0, 842.0)),
        }
    }
    Ok(sizes)
}

pub fn register_tmp_dir(dir: PathBuf) {
    if let Ok(mut dirs) = PDF_TMP_DIRS.lock() {
        dirs.push(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::{crop_px, PdfBox};

    fn box_xy(l: f32, b: f32, r: f32, t: f32) -> PdfBox {
        PdfBox {
            left: l,
            bottom: b,
            right: r,
            top: t,
        }
    }

    #[test]
    fn crop_px_keeps_full_image_when_boxes_match() {
        let src = box_xy(0.0, 0.0, 595.0, 760.0);
        let (x, y, w, h) = crop_px(3516, 4500, src, src);
        assert_eq!((x, y, w, h), (0, 0, 3516, 4500));
    }

    #[test]
    fn crop_px_drops_mediabox_bottom_margin() {
        // A4 MediaBox, CropBox 约 760pt, 图铺满 MediaBox.
        let src = box_xy(0.0, 0.0, 595.276, 841.89);
        let vis = box_xy(0.0, 42.52, 595.276, 802.205);
        let (x, y, w, h) = crop_px(3516, 4975, src, vis);
        assert_eq!(x, 0);
        assert_eq!(w, 3516);
        // 顶底各约 40pt → 约 236px, 留下 ~4500
        assert!(y > 180 && y < 280, "top crop {y}");
        assert!(h > 4300 && h < 4600, "visible height {h}");
        assert!(y + h <= 4975);
    }
}
