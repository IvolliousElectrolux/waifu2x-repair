//! 拖入文件夹时递归收集图片 / PDF, 只记路径不解码.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_FILES: usize = 20_000;
const MAX_DEPTH: u32 = 32;

#[derive(Clone, Debug)]
pub struct FoundFile {
    pub path: PathBuf,
    pub rel: PathBuf,
}

pub fn is_pdf_path(path: &Path) -> bool {
    ext_is(path, &["pdf"])
}

pub fn is_image_path(path: &Path) -> bool {
    ext_is(path, &["png", "jpg", "jpeg", "tif", "tiff", "bmp", "webp"])
}

pub fn is_output_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|n| n.to_ascii_lowercase().contains("_waifu2x."))
        .unwrap_or(false)
}

pub fn rel_display(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

pub fn expand_paths(inputs: &[PathBuf]) -> Result<Vec<FoundFile>, String> {
    let mut out = Vec::new();
    for p in inputs {
        let meta = match fs::metadata(p) {
            Ok(m) => m,
            Err(e) => return Err(format!("{}: {e}", p.display())),
        };
        if meta.is_dir() {
            walk(p, p, 0, &mut out)?;
        } else if is_pdf_path(p) || is_image_path(p) {
            if is_output_name(p) {
                continue;
            }
            let rel = PathBuf::from(p.file_name().unwrap_or_default());
            out.push(FoundFile {
                path: p.clone(),
                rel,
            });
        }
        if out.len() > MAX_FILES {
            return Err(format!("文件太多 (上限 {MAX_FILES})"));
        }
    }
    Ok(out)
}

fn walk(root: &Path, dir: &Path, depth: u32, out: &mut Vec<FoundFile>) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if out.len() > MAX_FILES {
            return Err(format!("文件太多 (上限 {MAX_FILES})"));
        }
        let name = e.file_name();
        let name_s = name.to_string_lossy();
        if name_s.starts_with('.') {
            continue;
        }
        let path = e.path();
        let ft = match e.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk(root, &path, depth + 1, out)?;
            continue;
        }
        if !(is_pdf_path(&path) || is_image_path(&path)) || is_output_name(&path) {
            continue;
        }
        out.push(FoundFile {
            path: path.clone(),
            rel: rel_under_root(root, &path),
        });
    }
    Ok(())
}

fn rel_under_root(root: &Path, file: &Path) -> PathBuf {
    let folder = root
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("folder"));
    match file.strip_prefix(root) {
        Ok(r) => folder.join(r),
        Err(_) => folder.join(file.file_name().unwrap_or_default()),
    }
}

fn ext_is(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            exts.iter().any(|x| e == *x)
        })
        .unwrap_or(false)
}

pub fn job_out_path(
    overwrite: bool,
    out_dir: &Path,
    source_file: &Path,
    rel: &Path,
    pdf_page: Option<u32>,
) -> PathBuf {
    let stem = source_file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("page");
    let stem: String = stem
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c })
        .collect();
    let stem = if stem.is_empty() {
        "page".to_string()
    } else {
        stem
    };
    let name = match pdf_page {
        Some(p) if overwrite => format!("{stem}_p{p:03}.png"),
        Some(p) => format!("{stem}_p{p:03}_waifu2x.png"),
        None if overwrite => format!("{stem}.png"),
        None => format!("{stem}_waifu2x.png"),
    };
    if overwrite {
        source_file
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(name)
    } else {
        let parent = rel.parent().unwrap_or_else(|| Path::new(""));
        out_dir.join(parent).join(name)
    }
}

/// 由某一页修好的 PNG 推出合成 PDF 路径.
pub fn assembled_pdf_path(page_png: &Path, source_pdf: &Path) -> PathBuf {
    let parent = page_png.parent().unwrap_or_else(|| Path::new("."));
    let stem = source_pdf
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("score");
    let waifu = page_png
        .file_name()
        .and_then(|s| s.to_str())
        .map(|n| n.to_ascii_lowercase().contains("_waifu2x"))
        .unwrap_or(false);
    let name = if waifu {
        format!("{stem}_waifu2x.pdf")
    } else {
        format!("{stem}.pdf")
    };
    parent.join(name)
}
