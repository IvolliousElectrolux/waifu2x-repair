//! 记住导入分辨率与 waifu2x 面板参数.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::models::{AlphaMode, Backend, ModelId, Scale, TileSize, TtaLevel};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_lock")]
    pub pdf_import_lock_aspect: bool,
    #[serde(default = "default_scale")]
    pub pdf_import_scale: f32,
    #[serde(default)]
    pub backend: Backend,
    #[serde(default)]
    pub model: ModelId,
    #[serde(default)]
    pub noise_level: i32,
    #[serde(default)]
    pub scale: Scale,
    #[serde(default)]
    pub tile_size: TileSize,
    #[serde(default)]
    pub tile_shuffle: bool,
    #[serde(default)]
    pub tta: TtaLevel,
    #[serde(default)]
    pub alpha: AlphaMode,
    #[serde(default)]
    pub binarize: bool,
    #[serde(default)]
    pub out_dir: String,
}

fn default_lock() -> bool {
    true
}
fn default_scale() -> f32 {
    crate::pdf::DEFAULT_PDF_SCALE
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pdf_import_lock_aspect: true,
            pdf_import_scale: crate::pdf::DEFAULT_PDF_SCALE,
            backend: Backend::default(),
            model: ModelId::default(),
            noise_level: 2,
            scale: Scale::X1,
            tile_size: TileSize::T256,
            tile_shuffle: false,
            tta: TtaLevel::Off,
            alpha: AlphaMode::Auto,
            binarize: false,
            out_dir: String::new(),
        }
    }
}

fn path() -> PathBuf {
    dirs_config().join("config.json")
}

fn dirs_config() -> PathBuf {
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(base).join("waifu2x_repair");
    }
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("waifu2x_repair");
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config").join("waifu2x_repair");
    }
    std::env::temp_dir().join("waifu2x_repair")
}

pub fn cache_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Caches")
                .join("waifu2x_repair");
        }
    }
    dirs_config().join("cache")
}

pub fn load() -> Config {
    let p = path();
    let Ok(s) = fs::read_to_string(&p) else {
        return Config::default();
    };
    serde_json::from_str(&s).unwrap_or_default()
}

pub fn save(cfg: &Config) {
    let dir = dirs_config();
    let _ = fs::create_dir_all(&dir);
    if let Ok(s) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(path(), s);
    }
}

pub fn remember_pdf_import(scale: f32, lock: bool) {
    let mut cfg = load();
    cfg.pdf_import_scale = scale;
    cfg.pdf_import_lock_aspect = lock;
    save(&cfg);
}
