use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error(
        "找不到 {lib} — 请把它放在程序同目录下, 或加入系统 PATH, \
         也可设置环境变量 PDFIUM_DYNAMIC_LIB_PATH 指定路径."
    )]
    PdfiumMissing { lib: String },
    #[error("无法加载 pdfium ({}): {detail}", .path.display())]
    PdfiumLoad { path: PathBuf, detail: String },
    #[error(
        "找不到 libonnxruntime.dylib — 请把它放在程序同目录下, \
         或设置环境变量 ORT_DYLIB_PATH 指定路径."
    )]
    OrtMissing,
    #[error("打开 PDF 失败: {0}")]
    PdfOpen(String),
    #[error("无法打开图片 ({}): {detail}", .path.display())]
    ImageOpen { path: PathBuf, detail: String },
    #[error("模型: {0}")]
    Model(String),
    #[error("推理: {0}")]
    Infer(String),
}

impl Error {
    pub fn msg(s: impl Into<String>) -> Self {
        Self::Message(s.into())
    }

    pub fn is_oom(&self) -> bool {
        let s = self.to_string().to_ascii_lowercase();
        s.contains("out of memory")
            || s.contains("oom")
            || s.contains("failed to allocate")
            || s.contains("std::bad_alloc")
            || s.contains("memory allocation")
            || s.contains("not enough memory")
            || s.contains("cuda_error_out_of_memory")
    }
}
