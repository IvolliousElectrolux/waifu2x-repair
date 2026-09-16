#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod binarize;
mod config;
mod download;
mod engine;
mod error;
mod gui;
mod keys;
mod mem;
mod models;
mod pdf;
mod seam;
mod tensor;
mod text_input;
mod waifu2x;

pub(crate) use keys::bind_primary;

use std::process::ExitCode;

fn main() -> ExitCode {
    let _cleanup = PdfCleanup;
    gui::run_gui();
    ExitCode::SUCCESS
}

struct PdfCleanup;
impl Drop for PdfCleanup {
    fn drop(&mut self) {
        pdf::cleanup_pdf_tmps();
    }
}

pub fn ui_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "PingFang SC"
    } else if cfg!(windows) {
        "Microsoft YaHei UI"
    } else {
        "sans-serif"
    }
}
