//! 按 unlimited:waifu2x 的 tiled_render 流程跑 ONNX (本地, 不上传图片).

use std::path::Path;
use std::sync::Mutex;

use image::RgbImage;
use ort::ep;
use ort::session::Session;
use ort::value::Tensor as OrtTensor;

use crate::error::Error;
use crate::models::{Backend, MethodConfig, PadMode};
use crate::seam::SeamBlending;
use crate::tensor::{tta_variants, Tensor, TtaOp};

pub struct OrtEngine {
    session: Mutex<Session>,
}

impl OrtEngine {
    pub fn load(path: &Path, backend: Backend) -> Result<Self, Error> {
        prepare_ort()?;
        let mut b = Session::builder().map_err(|e| Error::Infer(e.to_string()))?;
        let eps = execution_providers(backend);
        if !eps.is_empty() {
            b = b
                .with_execution_providers(eps)
                .map_err(|e| Error::Infer(e.to_string()))?;
        }
        let nthreads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        b = b
            .with_intra_threads(nthreads.max(1))
            .map_err(|e| Error::Infer(e.to_string()))?;
        let session = b
            .commit_from_file(path)
            .map_err(|e| Error::Infer(format!("加载 {}: {e}", path.display())))?;
        Ok(Self {
            session: Mutex::new(session),
        })
    }

    fn run_tile(&self, tile: &Tensor) -> Result<Tensor, Error> {
        let mut ses = self
            .session
            .lock()
            .map_err(|_| Error::Infer("session lock".into()))?;
        let input = OrtTensor::from_array((
            [1usize, tile.c, tile.h, tile.w],
            tile.data.clone(),
        ))
        .map_err(|e| Error::Infer(e.to_string()))?;
        let outputs = ses
            .run(ort::inputs!["x" => input])
            .map_err(|e| Error::Infer(e.to_string()))?;
        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| Error::Infer(e.to_string()))?;
        let dims: Vec<usize> = shape.iter().map(|d| *d as usize).collect();
        let (c, h, w) = match dims.as_slice() {
            [1, c, h, w] | [c, h, w] => (*c, *h, *w),
            _ => return Err(Error::Infer(format!("意外输出形状 {dims:?}"))),
        };
        Ok(Tensor {
            data: data.to_vec(),
            c,
            h,
            w,
        })
    }
}

fn prepare_ort() -> Result<(), Error> {
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        let dylib = find_ort_dylib().ok_or(Error::OrtMissing)?;
        ort::init_from(&dylib)
            .map_err(|e| Error::Infer(format!("加载 ONNX Runtime: {e}")))?
            .commit();
    }
    Ok(())
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn find_ort_dylib() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if let Ok(p) = std::env::var("ORT_DYLIB_PATH") {
        if !p.is_empty() {
            let pb = PathBuf::from(&p);
            if pb.is_file() {
                return Some(pb);
            }
            let joined = pb.join("libonnxruntime.dylib");
            if joined.is_file() {
                return Some(joined);
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let c = dir.join("libonnxruntime.dylib");
            if c.is_file() {
                return Some(c);
            }
        }
    }
    None
}

fn execution_providers(backend: Backend) -> Vec<ep::ExecutionProviderDispatch> {
    match backend {
        Backend::Cpu => Vec::new(),
        Backend::Gpu | Backend::Auto => {
            let mut v = Vec::new();
            #[cfg(windows)]
            {
                v.push(ep::DirectML::default().build());
            }
            #[cfg(target_os = "macos")]
            {
                v.push(ep::CoreML::default().build());
            }
            v
        }
    }
}

pub struct UpscaleSettings {
    pub cfg: MethodConfig,
    pub tile: u32,
    pub tta: u32,
    pub shuffle: bool,
}

pub fn estimate_job_bytes(w: u32, h: u32, settings: &UpscaleSettings) -> u64 {
    let scale = settings.cfg.scale as u64;
    let tile = settings.tile as u64;
    let tta = settings.tta.max(1) as u64;
    let in_hw = w as u64 * h as u64;
    let padded = in_hw.saturating_mul(2).saturating_add(tile * tile);
    let f32c = 4u64;
    let input = padded * 3 * f32c;
    let out = in_hw * scale * scale * 3 * f32c * 2;
    let tile_buf = tile * tile * 3 * f32c * tta;
    input.saturating_add(out).saturating_add(tile_buf).saturating_add(64 * 1024 * 1024)
}

pub fn upscale(
    rgb: &RgbImage,
    engine: &OrtEngine,
    settings: &UpscaleSettings,
    mut on_tile: impl FnMut(usize, usize),
) -> Result<RgbImage, Error> {
    let x = Tensor::from_rgb(rgb);
    let tile = settings.tile as usize;
    let mut seam = SeamBlending::new(x.h, x.w, settings.cfg.scale, settings.cfg.offset, settings.tile);
    let p = seam.param.clone();
    let padded = x.pad(p.pad[0], p.pad[1], p.pad[2], p.pad[3], settings.cfg.padding);
    let mut tiles = Vec::new();
    for h_i in 0..p.h_blocks {
        for w_i in 0..p.w_blocks {
            tiles.push((h_i, w_i));
        }
    }
    if settings.shuffle {
        use rand::seq::SliceRandom;
        tiles.shuffle(&mut rand::thread_rng());
    }
    let total = tiles.len();
    on_tile(0, total);
    for (k, (h_i, w_i)) in tiles.into_iter().enumerate() {
        let i = h_i * p.input_tile_step;
        let j = w_i * p.input_tile_step;
        let tile_x = padded.crop(j, i, tile, tile);
        let out_h = tile * settings.cfg.scale as usize - settings.cfg.offset as usize * 2;
        let out_w = out_h;
        let tile_y = if settings.cfg.color_stability {
            if let Some(rgb) = tile_x.is_single_color() {
                Tensor::solid(rgb, out_h, out_w)
            } else {
                infer_tta(engine, &tile_x, settings.tta)?
            }
        } else {
            infer_tta(engine, &tile_x, settings.tta)?
        };
        let _ = (out_w, PadMode::Replication);
        seam.update(&tile_y, h_i, w_i);
        on_tile(k + 1, total);
    }
    Ok(seam.finish().to_rgb())
}

fn infer_tta(engine: &OrtEngine, tile: &Tensor, tta: u32) -> Result<Tensor, Error> {
    let ops = tta_variants(tta);
    let n = ops.len() as f32;
    let mut acc: Option<Tensor> = None;
    for op in ops {
        let x = op.apply(tile);
        let y = engine.run_tile(&x)?;
        let y = match op {
            TtaOp::Id => y,
            TtaOp::HFlip => y.hflip(),
            TtaOp::VFlip => y.vflip(),
            TtaOp::Rot180 => y.rot180(),
        };
        match &mut acc {
            None => acc = Some(y),
            Some(a) => a.add_assign(&y),
        }
    }
    let mut out = acc.ok_or_else(|| Error::Infer("empty tta".into()))?;
    if n > 1.0 {
        out.scale_assign(1.0 / n);
    }
    Ok(out)
}
