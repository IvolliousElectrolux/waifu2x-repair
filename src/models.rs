//! 与 https://unlimited.waifu2x.net 面板一一对应的模型/参数.

use serde::{Deserialize, Serialize};

pub const MODEL_BASE: &str = "https://unlimited.waifu2x.net";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
    #[default]
    Auto,
    Gpu,
    Cpu,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Gpu => "WebGPU",
            Self::Cpu => "WebAssembly",
        }
    }
    pub const ALL: [Self; 3] = [Self::Auto, Self::Gpu, Self::Cpu];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelId {
    SwinUnetV3Art,
    SwinUnetArt,
    #[default]
    SwinUnetArtScan,
    SwinUnetPhoto,
    CunetArt,
}

impl ModelId {
    pub fn value(self) -> &'static str {
        match self {
            Self::SwinUnetV3Art => "swin_unet_v3.art",
            Self::SwinUnetArt => "swin_unet.art",
            Self::SwinUnetArtScan => "swin_unet.art_scan",
            Self::SwinUnetPhoto => "swin_unet.photo",
            Self::CunetArt => "cunet.art",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::SwinUnetV3Art => "🎨 swin_unet_v3 / art",
            Self::SwinUnetArt => "🎨 swin_unet / art",
            Self::SwinUnetArtScan => "🖨 swin_unet / art scan",
            Self::SwinUnetPhoto => "📷 swin_unet / photo",
            Self::CunetArt => "🎨 cunet / art (201811)",
        }
    }
    pub fn arch_style(self) -> (&'static str, &'static str) {
        match self {
            Self::SwinUnetV3Art => ("swin_unet_v3", "art"),
            Self::SwinUnetArt => ("swin_unet", "art"),
            Self::SwinUnetArtScan => ("swin_unet", "art_scan"),
            Self::SwinUnetPhoto => ("swin_unet", "photo"),
            Self::CunetArt => ("cunet", "art"),
        }
    }
    pub fn supports_4x(self) -> bool {
        matches!(
            self,
            Self::SwinUnetArt | Self::SwinUnetArtScan | Self::SwinUnetPhoto
        )
    }
    pub fn color_stability(self) -> bool {
        matches!(
            self,
            Self::SwinUnetV3Art | Self::SwinUnetArt | Self::CunetArt
        )
    }
    pub fn padding(self) -> PadMode {
        if matches!(self, Self::SwinUnetPhoto) {
            PadMode::Reflection
        } else {
            PadMode::Replication
        }
    }
    pub const ALL: [Self; 5] = [
        Self::SwinUnetV3Art,
        Self::SwinUnetArt,
        Self::SwinUnetArtScan,
        Self::SwinUnetPhoto,
        Self::CunetArt,
    ];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scale {
    #[default]
    X1,
    X2,
    X4,
}

impl Scale {
    pub fn factor(self) -> u32 {
        match self {
            Self::X1 => 1,
            Self::X2 => 2,
            Self::X4 => 4,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::X1 => "1x",
            Self::X2 => "2x",
            Self::X4 => "4x",
        }
    }
    pub const ALL: [Self; 3] = [Self::X1, Self::X2, Self::X4];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TileSize {
    T64,
    #[default]
    T256,
    T400,
    T640,
}

impl TileSize {
    pub fn px(self) -> u32 {
        match self {
            Self::T64 => 64,
            Self::T256 => 256,
            Self::T400 => 400,
            Self::T640 => 640,
        }
    }
    pub fn smaller(self) -> Option<Self> {
        match self {
            Self::T640 => Some(Self::T400),
            Self::T400 => Some(Self::T256),
            Self::T256 => Some(Self::T64),
            Self::T64 => None,
        }
    }
    pub const ALL: [Self; 4] = [Self::T64, Self::T256, Self::T400, Self::T640];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TtaLevel {
    #[default]
    Off,
    L2,
    L4,
}

impl TtaLevel {
    pub fn n(self) -> u32 {
        match self {
            Self::Off => 0,
            Self::L2 => 2,
            Self::L4 => 4,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "0",
            Self::L2 => "2",
            Self::L4 => "4",
        }
    }
    pub const ALL: [Self; 3] = [Self::Off, Self::L2, Self::L4];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlphaMode {
    #[default]
    Auto,
    Disable,
}

impl AlphaMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Disable => "Disable",
        }
    }
    pub const ALL: [Self; 2] = [Self::Auto, Self::Disable];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadMode {
    Replication,
    Reflection,
}

#[derive(Clone, Debug)]
pub struct MethodConfig {
    pub arch: &'static str,
    pub style: &'static str,
    pub method: String,
    pub scale: u32,
    pub offset: u32,
    pub color_stability: bool,
    pub padding: PadMode,
}

pub fn method_name(scale: Scale, noise: i32) -> Option<String> {
    if scale == Scale::X1 {
        if noise < 0 {
            return None;
        }
        return Some(format!("noise{noise}"));
    }
    let sx = scale.factor();
    if noise < 0 {
        Some(format!("scale{sx}x"))
    } else {
        Some(format!("noise{noise}_scale{sx}x"))
    }
}

pub fn offset_for(arch: &str, scale: u32) -> u32 {
    match (arch, scale) {
        ("cunet", 1) => 28,
        ("cunet", _) => 36,
        (_, 1) => 8,
        (_, 2) => 16,
        (_, 4) => 32,
        _ => 8,
    }
}

pub fn calc_tile_size(model: ModelId, requested: u32, scale: u32, offset: u32) -> u32 {
    match model {
        ModelId::SwinUnetV3Art => {
            let mut t = requested.max(64);
            if t % 32 != 0 {
                t += 32 - t % 32;
            }
            t
        }
        ModelId::CunetArt => {
            let adj = if scale == 1 { 16u32 } else { 32 };
            let mut t = ((requested * scale + offset * 2) - adj) / scale;
            t -= t % 4;
            t.max(4)
        }
        _ => {
            let mut t = requested;
            loop {
                if (t as i32 - 16) % 12 == 0 && (t as i32 - 16) % 16 == 0 {
                    break;
                }
                t += 1;
            }
            t
        }
    }
}

pub fn resolve(
    model: ModelId,
    scale: Scale,
    noise: i32,
    tile: TileSize,
) -> Result<(MethodConfig, u32), crate::error::Error> {
    if scale == Scale::X4 && !model.supports_4x() {
        return Err(crate::error::Error::msg("该模型没有 4x"));
    }
    let method = method_name(scale, noise).ok_or_else(|| {
        crate::error::Error::msg("1x 必须选择降噪 (DeNoise 不能是 None)")
    })?;
    let (arch, style) = model.arch_style();
    let scale_n = scale.factor();
    let offset = offset_for(arch, scale_n);
    let tile_px = calc_tile_size(model, tile.px(), scale_n, offset);
    Ok((
        MethodConfig {
            arch,
            style,
            method,
            scale: scale_n,
            offset,
            color_stability: model.color_stability(),
            padding: model.padding(),
        },
        tile_px,
    ))
}

pub fn model_url(cfg: &MethodConfig) -> String {
    format!(
        "{MODEL_BASE}/models/{}/{}/{}.onnx",
        cfg.arch, cfg.style, cfg.method
    )
}

pub fn model_rel_path(cfg: &MethodConfig) -> String {
    format!("models/{}/{}/{}.onnx", cfg.arch, cfg.style, cfg.method)
}

pub fn noise_label(v: i32) -> &'static str {
    match v {
        -1 => "(-) None",
        0 => "(0) Low",
        1 => "(1) Medium",
        2 => "(2) High",
        3 => "(3) Highest",
        _ => "?",
    }
}

pub const NOISE_LEVELS: [i32; 5] = [-1, 0, 1, 2, 3];
