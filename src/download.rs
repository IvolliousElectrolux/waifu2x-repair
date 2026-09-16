//! 下载并缓存 unlimited.waifu2x.net 的 ONNX.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;

use crate::config;
use crate::error::Error;
use crate::models::{model_rel_path, model_url, MethodConfig};

pub fn ensure_model(cfg: &MethodConfig) -> Result<PathBuf, Error> {
    let rel = model_rel_path(cfg);
    let dest = config::cache_dir().join(&rel);
    if dest.is_file() && dest.metadata().map(|m| m.len() > 1024).unwrap_or(false) {
        return Ok(dest);
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::Model(e.to_string()))?;
    }
    let url = model_url(cfg);
    download(&url, &dest)?;
    Ok(dest)
}

fn download(url: &str, dest: &PathBuf) -> Result<(), Error> {
    let tmp = dest.with_extension("onnx.part");
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(600))
        .call()
        .map_err(|e| Error::Model(format!("下载 {url}: {e}")))?;
    if resp.status() != 200 {
        return Err(Error::Model(format!(
            "下载 {url} HTTP {}",
            resp.status()
        )));
    }
    let mut reader = resp.into_reader();
    let mut file = fs::File::create(&tmp).map_err(|e| Error::Model(e.to_string()))?;
    let mut buf = [0u8; 1 << 16];
    loop {
        let n = reader.read(&mut buf).map_err(|e| Error::Model(e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| Error::Model(e.to_string()))?;
    }
    file.flush().map_err(|e| Error::Model(e.to_string()))?;
    drop(file);
    fs::rename(&tmp, dest).map_err(|e| Error::Model(e.to_string()))?;
    Ok(())
}
