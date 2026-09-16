//! GPUI 主界面.

mod import;
mod panel;

use gpui::actions;
use gpui::{
    App, Application, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyBinding, Render,
    Window, WindowBounds, WindowOptions,
};

pub(crate) use std::path::PathBuf;
pub(crate) use std::sync::Arc;
pub(crate) use gpui::prelude::*;
pub(crate) use gpui::{
    canvas, div, point, px, rgb, size, Bounds, Context, Entity, ExternalPaths, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, RenderImage, SharedString,
};
pub(crate) use crate::text_input::TextInput;

use crate::config;
use crate::engine::{self, Handle as EngineHandle, PageJob, RunSettings};
use crate::keys::{bind_primary, with_mod};
use crate::mem::fmt_bytes;
use crate::models::{
    noise_label, AlphaMode, Backend, ModelId, Scale, TileSize, TtaLevel, NOISE_LEVELS,
};
use crate::pdf::PageKind;
use crate::text_input;
use crate::ui_font;

actions!(
    waifu2x_repair,
    [OpenFile, Start, Stop, PickOutDir, ConfirmImport, CancelImport]
);

#[derive(Clone, Debug)]
pub(crate) enum PageStatus {
    Queued,
    Running,
    Done(PathBuf),
    Skipped(String),
    Failed(String),
}

#[derive(Clone, Debug)]
pub(crate) struct QueuePage {
    pub id: u64,
    pub label: String,
    pub source: engine::JobSource,
    pub kind: PageKind,
    pub status: PageStatus,
    pub tiles_done: usize,
    pub tiles_total: usize,
}

struct SettingsUi {
    backend: Backend,
    model: ModelId,
    noise: i32,
    scale: Scale,
    tile: TileSize,
    shuffle: bool,
    tta: TtaLevel,
    alpha: AlphaMode,
    binarize: bool,
}

pub(crate) struct RepairApp {
    focus_handle: FocusHandle,
    pdf_import: Option<import::PdfImportState>,
    pdf_w_input: Entity<TextInput>,
    pdf_h_input: Entity<TextInput>,
    pdf_scale_input: Entity<TextInput>,
    pdf_preview_page_input: Entity<TextInput>,
    pages: Vec<QueuePage>,
    next_page_id: u64,
    settings: SettingsUi,
    out_dir: PathBuf,
    status: SharedString,
    running: bool,
    engine: Option<EngineHandle>,
    workers: usize,
    live_tile: u32,
    mem_process: u64,
    mem_avail: u64,
    mem_budget: u64,
    btn_press: Option<SharedString>,
}

impl RepairApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let cfg = config::load();
        let pdf_w_input = cx.new(|cx| TextInput::new(cx, "", "宽").with_compact(true));
        let pdf_h_input = cx.new(|cx| TextInput::new(cx, "", "高").with_compact(true));
        let pdf_scale_input = cx.new(|cx| TextInput::new(cx, "", "倍率").with_compact(true));
        let pdf_preview_page_input = cx.new(|cx| TextInput::new(cx, "1", "页").with_compact(true));
        let out_dir = if cfg.out_dir.is_empty() {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        } else {
            PathBuf::from(&cfg.out_dir)
        };
        Self {
            focus_handle: cx.focus_handle(),
            pdf_import: None,
            pdf_w_input,
            pdf_h_input,
            pdf_scale_input,
            pdf_preview_page_input,
            pages: Vec::new(),
            next_page_id: 1,
            settings: SettingsUi {
                backend: cfg.backend,
                model: cfg.model,
                noise: cfg.noise_level,
                scale: cfg.scale,
                tile: cfg.tile_size,
                shuffle: cfg.tile_shuffle,
                tta: cfg.tta,
                alpha: cfg.alpha,
                binarize: cfg.binarize,
            },
            out_dir,
            status: "拖入或打开 PDF / 图片.".into(),
            running: false,
            engine: None,
            workers: 0,
            live_tile: cfg.tile_size.px(),
            mem_process: 0,
            mem_avail: 0,
            mem_budget: 0,
            btn_press: None,
        }
    }

    fn persist(&mut self, _cx: &mut Context<Self>) {
        let mut cfg = config::load();
        cfg.backend = self.settings.backend;
        cfg.model = self.settings.model;
        cfg.noise_level = self.settings.noise;
        cfg.scale = self.settings.scale;
        cfg.tile_size = self.settings.tile;
        cfg.tile_shuffle = self.settings.shuffle;
        cfg.tta = self.settings.tta;
        cfg.alpha = self.settings.alpha;
        cfg.binarize = self.settings.binarize;
        cfg.out_dir = self.out_dir.display().to_string();
        config::save(&cfg);
    }

    fn spawn_native_dialog<T, F, A>(cx: &mut Context<Self>, work: F, apply: A)
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
        A: FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    {
        let (tx, rx) = async_channel::bounded::<T>(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(work());
        });
        cx.spawn(async move |this, cx| {
            if let Ok(val) = rx.recv().await {
                this.update(cx, |view, cx| apply(view, val, cx)).ok();
            }
        })
        .detach();
    }

    fn reorder_to_index(from: usize, over: usize, after: bool) -> usize {
        if after {
            if from < over {
                over
            } else {
                over + 1
            }
        } else if from < over {
            over - 1
        } else {
            over
        }
    }

    fn btn(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        active: bool,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let bg = if active { rgb(0x2563eb) } else { rgb(0xe2e8f0) };
        let fg = if active { rgb(0xffffff) } else { rgb(0x0f172a) };
        let hover = if active { rgb(0x1d4ed8) } else { rgb(0xcbd5e1) };
        let id: SharedString = id.into();
        let id_down = id.clone();
        let id_up = id.clone();
        let id_out = id.clone();
        div()
            .id(id)
            .px_2()
            .py_1()
            .rounded_md()
            .bg(bg)
            .border_1()
            .border_color(rgb(0x94a3b8))
            .text_color(fg)
            .text_sm()
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .child(label.into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.btn_press = Some(id_down.clone());
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    let press = this.btn_press.take();
                    if press.as_ref() != Some(&id_up) {
                        return;
                    }
                    on_click(this, window, cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |this, _, _, _| {
                    if this.btn_press.as_ref() == Some(&id_out) {
                        this.btn_press = None;
                    }
                }),
            )
    }

    fn pick_out_dir(&mut self, cx: &mut Context<Self>) {
        let start = self.out_dir.clone();
        Self::spawn_native_dialog(
            cx,
            move || {
                rfd::FileDialog::new()
                    .set_title("输出目录")
                    .set_directory(&start)
                    .pick_folder()
            },
            |this, dir, cx| {
                if let Some(p) = dir {
                    this.out_dir = p;
                    this.persist(cx);
                    cx.notify();
                }
            },
        );
    }

    fn start_repair(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let jobs: Vec<PageJob> = self
            .pages
            .iter()
            .filter(|p| matches!(p.status, PageStatus::Queued | PageStatus::Failed(_)))
            .map(|p| {
                let stem = sanitize(&p.label);
                PageJob {
                    id: p.id,
                    label: p.label.clone(),
                    source: p.source.clone(),
                    out_path: self.out_dir.join(format!("{stem}_waifu2x.png")),
                }
            })
            .collect();
        if jobs.is_empty() {
            self.status = "没有待修复的页 (矢量页会跳过).".into();
            cx.notify();
            return;
        }
        let settings = RunSettings {
            backend: self.settings.backend,
            model: self.settings.model,
            noise: self.settings.noise,
            scale: self.settings.scale,
            tile: self.settings.tile,
            shuffle: self.settings.shuffle,
            tta: self.settings.tta.n(),
            binarize: self.settings.binarize,
        };
        let (handle, mut rx) = EngineHandle::start(jobs, settings);
        self.engine = Some(handle);
        self.running = true;
        self.status = "开始修复…".into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.recv().await {
                let stop = this
                    .update(cx, |view, cx| {
                        view.on_engine_event(ev, cx);
                        !view.running
                    })
                    .unwrap_or(true);
                if stop {
                    break;
                }
            }
        })
        .detach();
    }

    fn stop_repair(&mut self, cx: &mut Context<Self>) {
        if let Some(h) = &self.engine {
            h.stop();
        }
        self.status = "正在停止…".into();
        cx.notify();
    }

    fn on_engine_event(&mut self, ev: engine::Event, cx: &mut Context<Self>) {
        match ev {
            engine::Event::Status(s) => self.status = s.into(),
            engine::Event::ModelReady(m) => self.status = format!("模型就绪 {m}").into(),
            engine::Event::Workers { n, tile } => {
                self.workers = n;
                self.live_tile = tile;
            }
            engine::Event::Mem {
                process,
                available,
                budget,
            } => {
                self.mem_process = process;
                self.mem_avail = available;
                self.mem_budget = budget;
            }
            engine::Event::PageStart { id } => {
                if let Some(p) = self.pages.iter_mut().find(|p| p.id == id) {
                    p.status = PageStatus::Running;
                }
            }
            engine::Event::Tile { id, done, total } => {
                if let Some(p) = self.pages.iter_mut().find(|p| p.id == id) {
                    p.tiles_done = done;
                    p.tiles_total = total;
                }
            }
            engine::Event::PageDone { id, path, .. } => {
                if let Some(p) = self.pages.iter_mut().find(|p| p.id == id) {
                    p.status = PageStatus::Done(path);
                }
            }
            engine::Event::PageSkip { id, reason } => {
                if let Some(p) = self.pages.iter_mut().find(|p| p.id == id) {
                    p.status = PageStatus::Skipped(reason);
                }
            }
            engine::Event::PageFail { id, err, requeued } => {
                if let Some(p) = self.pages.iter_mut().find(|p| p.id == id) {
                    if requeued {
                        p.status = PageStatus::Queued;
                        self.status = format!("失败重排: {err}").into();
                    } else {
                        p.status = PageStatus::Failed(err);
                    }
                }
            }
            engine::Event::Finished => {
                self.running = false;
                self.engine = None;
                self.status = "完成.".into();
            }
        }
        cx.notify();
    }

    fn queue_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .id("queue")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .gap_1()
            .overflow_scroll();
        if self.pages.is_empty() {
            list = list.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(rgb(0x64748b))
                    .child("还没有页面. 打开或拖入 PDF / 图片."),
            );
        }
        for p in &self.pages {
            let st = match &p.status {
                PageStatus::Queued => "排队".to_string(),
                PageStatus::Running => {
                    if p.tiles_total > 0 {
                        format!("修复 {}/{}", p.tiles_done, p.tiles_total)
                    } else {
                        "修复中".into()
                    }
                }
                PageStatus::Done(_) => "完成".into(),
                PageStatus::Skipped(r) => format!("跳过 ({r})"),
                PageStatus::Failed(e) => format!("失败 {e}"),
            };
            let kind = match p.kind {
                PageKind::Image => "抽图",
                PageKind::Raster => "光栅",
                PageKind::Vector => "矢量",
            };
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(rgb(0xffffff))
                    .border_1()
                    .border_color(rgb(0xe2e8f0))
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .child(format!("{} · {kind}", p.label)),
                    )
                    .child(div().text_xs().text_color(rgb(0x64748b)).child(st)),
            );
        }
        let _ = cx;
        list
    }
}

fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c })
        .collect();
    if t.is_empty() {
        "page".into()
    } else {
        t
    }
}

pub fn is_pdf_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

pub fn is_image_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "tif" | "tiff" | "bmp" | "webp"
            )
        })
        .unwrap_or(false)
}

impl Focusable for RepairApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RepairApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let overlay = self.pdf_import_overlay(cx);
        let can_start = !self.running && self.pages.iter().any(|p| {
            matches!(p.status, PageStatus::Queued | PageStatus::Failed(_))
        });
        div()
            .key_context("Repair")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0xf8fafc))
            .text_color(rgb(0x0f172a))
            .font_family(ui_font())
            .on_action(cx.listener(|this, _: &OpenFile, _, cx| this.open_import_dialog(cx)))
            .on_action(cx.listener(|this, _: &Start, _, cx| this.start_repair(cx)))
            .on_action(cx.listener(|this, _: &Stop, _, cx| this.stop_repair(cx)))
            .on_action(cx.listener(|this, _: &PickOutDir, _, cx| this.pick_out_dir(cx)))
            .on_action(cx.listener(|this, _: &ConfirmImport, _, cx| {
                if this.pdf_import.is_some() {
                    this.confirm_pdf_import(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CancelImport, _, cx| {
                this.close_import_dialog(cx);
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                let list: Vec<PathBuf> = paths.paths().iter().cloned().collect();
                this.import_dialog_add_paths(list, cx);
            }))
            .child(
                div()
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(0xcbd5e1))
                    .bg(rgb(0xf1f5f9))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(self.btn(
                        "open",
                        format!("打开 ({})", with_mod("", "O").trim()),
                        false,
                        |this, _, cx| this.open_import_dialog(cx),
                        cx,
                    ))
                    .child(self.btn(
                        "outdir",
                        "输出目录",
                        false,
                        |this, _, cx| this.pick_out_dir(cx),
                        cx,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(rgb(0x64748b))
                            .child(self.out_dir.display().to_string()),
                    )
                    .child(self.btn(
                        "start",
                        "开始",
                        can_start,
                        |this, _, cx| this.start_repair(cx),
                        cx,
                    ))
                    .child(self.btn(
                        "stop",
                        "停止",
                        self.running,
                        |this, _, cx| this.stop_repair(cx),
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .p_3()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("页面队列"),
                            )
                            .child(self.queue_list(cx)),
                    )
                    .child(self.param_panel(cx)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(rgb(0xcbd5e1))
                    .text_xs()
                    .text_color(rgb(0x475569))
                    .child(self.status.clone()),
            )
            .child(overlay)
    }
}

pub fn run_gui() {
    Application::new().run(move |cx: &mut App| {
        text_input::bind_keys(cx);
        let mut keys = vec![
            KeyBinding::new("enter", ConfirmImport, Some("Repair")),
            KeyBinding::new("escape", CancelImport, Some("Repair")),
        ];
        keys.extend(bind_primary("o", OpenFile, Some("Repair")));
        keys.extend(bind_primary("enter", Start, Some("Repair")));
        cx.bind_keys(keys);
        let bounds = {
            const PREF_W: f32 = 1100.;
            const PREF_H: f32 = 720.;
            Bounds::centered(None, size(px(PREF_W), px(PREF_H)), cx)
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("waifu2x repair".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                cx.new(|cx| {
                    let app = RepairApp::new(cx);
                    app.focus_handle.focus(window);
                    app
                })
            },
        )
        .ok();
    });
}
