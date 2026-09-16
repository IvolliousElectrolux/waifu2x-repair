//! 全异步队列: 按可用内存决定并发, 失败 (含 OOM) 重新入队并收缩线程/tile.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use image::RgbImage;
use tokio::sync::{mpsc, Mutex, Notify};

use crate::download;
use crate::error::Error;
use crate::mem;
use crate::models::{self, Backend, Scale, TileSize};
use crate::pdf::{self, PageKind};
use crate::waifu2x::{self, OrtEngine, UpscaleSettings};

const MAX_RETRY: u32 = 8;

#[derive(Clone, Debug)]
pub struct PageJob {
    pub id: u64,
    pub label: String,
    pub source: JobSource,
    pub out_path: PathBuf,
}

#[derive(Clone, Debug)]
pub enum JobSource {
    Image(PathBuf),
    Pdf {
        path: PathBuf,
        page: u32,
        kind: PageKind,
        scale_x: f32,
        scale_y: f32,
    },
}

#[derive(Clone, Debug)]
pub struct RunSettings {
    pub backend: Backend,
    pub model: models::ModelId,
    pub noise: i32,
    pub scale: Scale,
    pub tile: TileSize,
    pub shuffle: bool,
    pub tta: u32,
    pub binarize: bool,
}

#[derive(Clone, Debug)]
pub enum Event {
    Status(String),
    ModelReady(String),
    Workers { n: usize, tile: u32 },
    Mem {
        process: u64,
        available: u64,
        budget: u64,
    },
    PageStart { id: u64 },
    Tile {
        id: u64,
        done: usize,
        total: usize,
    },
    PageDone {
        id: u64,
        path: PathBuf,
        w: u32,
        h: u32,
    },
    PageSkip { id: u64, reason: String },
    PageFail {
        id: u64,
        err: String,
        requeued: bool,
    },
    Finished,
}

pub struct Handle {
    stop: Arc<AtomicBool>,
}

impl Handle {
    pub fn start(
        jobs: Vec<PageJob>,
        settings: RunSettings,
    ) -> (Self, mpsc::UnboundedReceiver<Event>) {
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        std::thread::Builder::new()
            .name("waifu2x-engine".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("w2x-rt")
                    .build()
                    .expect("tokio");
                rt.block_on(run_pipeline(ev_tx, stop2, jobs, settings));
            })
            .expect("engine thread");
        (Self { stop }, ev_rx)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

struct Item {
    job: PageJob,
    retries: u32,
    tile: TileSize,
}

async fn run_pipeline(
    ev: mpsc::UnboundedSender<Event>,
    stop: Arc<AtomicBool>,
    jobs: Vec<PageJob>,
    settings: RunSettings,
) {
    let send = |e: Event| {
        let _ = ev.send(e);
    };
    send(Event::Status("准备模型…".into()));
    let (method, tile_px) = match models::resolve(
        settings.model,
        settings.scale,
        settings.noise,
        settings.tile,
    ) {
        Ok(v) => v,
        Err(e) => {
            send(Event::Status(e.to_string()));
            send(Event::Finished);
            return;
        }
    };
    let method2 = method.clone();
    let model_path = match tokio::task::spawn_blocking(move || download::ensure_model(&method2)).await
    {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            send(Event::Status(e.to_string()));
            send(Event::Finished);
            return;
        }
        Err(e) => {
            send(Event::Status(e.to_string()));
            send(Event::Finished);
            return;
        }
    };
    send(Event::ModelReady(method.method.clone()));

    let snap = mem::snap();
    let budget = ((snap.available as f64) * 0.85) as u64;
    send(Event::Mem {
        process: snap.process,
        available: snap.available,
        budget,
    });

    // Session 有 Mutex, 多 worker 不能并行推理, 反而会同时把两张大图塞进内存,
    // M 系列上还容易触发收缩 tile, 变得更慢. GPU/CoreML 只跑 1 路.
    let workers = 1;

    send(Event::Status(
        if cfg!(target_os = "macos") && settings.backend != Backend::Cpu {
            "加载推理后端 (首次会编译 CoreML, 之后走缓存)…".into()
        } else {
            "加载推理后端…".into()
        },
    ));
    let engine = match tokio::task::spawn_blocking({
        let path = model_path.clone();
        let backend = settings.backend;
        move || OrtEngine::load(&path, backend, tile_px)
    })
    .await
    {
        Ok(Ok(e)) => Arc::new(e),
        Ok(Err(e)) => {
            send(Event::Status(e.to_string()));
            send(Event::Finished);
            return;
        }
        Err(e) => {
            send(Event::Status(e.to_string()));
            send(Event::Finished);
            return;
        }
    };
    send(Event::Status(format!(
        "推理后端 {} / {}",
        engine.backend_label, method.method
    )));

    let queue = Arc::new(Mutex::new(VecDeque::from_iter(jobs.into_iter().map(|job| {
        Item {
            job,
            retries: 0,
            tile: settings.tile,
        }
    }))));
    let remaining = Arc::new(AtomicUsize::new(queue.lock().await.len()));
    let workers_n = Arc::new(AtomicUsize::new(workers));
    let tile_cap = Arc::new(Mutex::new(settings.tile));
    let notify = Arc::new(Notify::new());
    let reserved = Arc::new(AtomicUsize::new(0));
    send(Event::Workers {
        n: workers,
        tile: settings.tile.px(),
    });

    let mut handles = Vec::new();
    for wid in 0..workers {
        let ev = ev.clone();
        let stop = stop.clone();
        let queue = queue.clone();
        let remaining = remaining.clone();
        let workers_n = workers_n.clone();
        let tile_cap = tile_cap.clone();
        let notify = notify.clone();
        let reserved = reserved.clone();
        let engine = engine.clone();
        let settings = settings.clone();
        let method = method.clone();
        handles.push(tokio::spawn(async move {
            worker(
                wid,
                ev,
                stop,
                queue,
                remaining,
                workers_n,
                tile_cap,
                notify,
                reserved,
                engine,
                settings,
                method,
                budget,
            )
            .await;
        }));
    }

    for h in handles {
        let _ = h.await;
    }
    send(Event::Finished);
}

async fn worker(
    _wid: usize,
    ev: mpsc::UnboundedSender<Event>,
    stop: Arc<AtomicBool>,
    queue: Arc<Mutex<VecDeque<Item>>>,
    remaining: Arc<AtomicUsize>,
    workers_n: Arc<AtomicUsize>,
    tile_cap: Arc<Mutex<TileSize>>,
    notify: Arc<Notify>,
    reserved: Arc<AtomicUsize>,
    engine: Arc<OrtEngine>,
    settings: RunSettings,
    method: models::MethodConfig,
    budget: u64,
) {
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let item = {
            let mut q = queue.lock().await;
            q.pop_front()
        };
        let Some(mut item) = item else {
            if remaining.load(Ordering::SeqCst) == 0 {
                return;
            }
            notify.notified().await;
            continue;
        };

        if _wid >= workers_n.load(Ordering::SeqCst) {
            let mut q = queue.lock().await;
            q.push_front(item);
            return;
        }

        let _ = ev.send(Event::PageStart { id: item.job.id });
        let load = item.job.clone();
        let rgb = match tokio::task::spawn_blocking(move || load_rgb(&load)).await {
            Ok(Ok(img)) => img,
            Ok(Err(e)) => {
                if matches!(load_skip(&e), true) {
                    let _ = ev.send(Event::PageSkip {
                        id: item.job.id,
                        reason: e.to_string(),
                    });
                } else {
                    let _ = ev.send(Event::PageFail {
                        id: item.job.id,
                        err: e.to_string(),
                        requeued: false,
                    });
                }
                remaining.fetch_sub(1, Ordering::SeqCst);
                notify.notify_waiters();
                continue;
            }
            Err(e) => {
                let _ = ev.send(Event::PageFail {
                    id: item.job.id,
                    err: e.to_string(),
                    requeued: false,
                });
                remaining.fetch_sub(1, Ordering::SeqCst);
                notify.notify_waiters();
                continue;
            }
        };

        let cap = *tile_cap.lock().await;
        if item.tile.px() > cap.px() {
            item.tile = cap;
        }
        let tile_px = models::calc_tile_size(
            settings.model,
            item.tile.px(),
            method.scale,
            method.offset,
        );
        let up = UpscaleSettings {
            cfg: method.clone(),
            tile: tile_px,
            tta: settings.tta,
            shuffle: settings.shuffle,
        };
        let need = waifu2x::estimate_job_bytes(rgb.width(), rgb.height(), &up);
        wait_budget(&reserved, need, budget, &stop).await;
        if stop.load(Ordering::SeqCst) {
            reserved.fetch_sub(need as usize, Ordering::SeqCst);
            return;
        }

        let snap = mem::snap();
        let _ = ev.send(Event::Mem {
            process: snap.process,
            available: snap.available,
            budget,
        });

        let engine2 = engine.clone();
        let ev2 = ev.clone();
        let id = item.job.id;
        let out_path = item.job.out_path.clone();
        let binarize = settings.binarize;
        let result = tokio::task::spawn_blocking(move || {
            let img = waifu2x::upscale(&rgb, &engine2, &up, |done, total| {
                let _ = ev2.send(Event::Tile { id, done, total });
            })?;
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::msg(e.to_string()))?;
            }
            if binarize {
                crate::binarize::save_binary_png(&img, &out_path)?;
            } else {
                img.save(&out_path)
                    .map_err(|e| Error::msg(format!("写 {}: {e}", out_path.display())))?;
            }
            Ok::<_, Error>((img.width(), img.height()))
        })
        .await;

        reserved.fetch_sub(need as usize, Ordering::SeqCst);

        match result {
            Ok(Ok((w, h))) => {
                let _ = ev.send(Event::PageDone {
                    id: item.job.id,
                    path: item.job.out_path.clone(),
                    w,
                    h,
                });
                remaining.fetch_sub(1, Ordering::SeqCst);
                notify.notify_waiters();
            }
            Ok(Err(e)) => {
                let err = e;
                let oom = err.is_oom();
                item.retries += 1;
                let requeue = item.retries <= MAX_RETRY;
                if oom {
                    shrink(&workers_n, &tile_cap, &ev).await;
                }
                if requeue {
                    let _ = ev.send(Event::PageFail {
                        id: item.job.id,
                        err: err.to_string(),
                        requeued: true,
                    });
                    queue.lock().await.push_back(item);
                    notify.notify_waiters();
                } else {
                    let _ = ev.send(Event::PageFail {
                        id: item.job.id,
                        err: err.to_string(),
                        requeued: false,
                    });
                    remaining.fetch_sub(1, Ordering::SeqCst);
                    notify.notify_waiters();
                }
            }
            Err(j) => {
                let err = Error::msg(j.to_string());
                item.retries += 1;
                if item.retries <= MAX_RETRY {
                    let _ = ev.send(Event::PageFail {
                        id: item.job.id,
                        err: err.to_string(),
                        requeued: true,
                    });
                    queue.lock().await.push_back(item);
                    notify.notify_waiters();
                } else {
                    let _ = ev.send(Event::PageFail {
                        id: item.job.id,
                        err: err.to_string(),
                        requeued: false,
                    });
                    remaining.fetch_sub(1, Ordering::SeqCst);
                    notify.notify_waiters();
                }
            }
        }
    }
}

fn load_skip(e: &Error) -> bool {
    e.to_string().contains("矢量页")
}

fn load_rgb(job: &PageJob) -> Result<RgbImage, Error> {
    match &job.source {
        JobSource::Image(p) => image::open(p)
            .map(|i| i.to_rgb8())
            .map_err(|e| Error::ImageOpen {
                path: p.clone(),
                detail: e.to_string(),
            }),
        JobSource::Pdf {
            path,
            page,
            kind,
            scale_x,
            scale_y,
        } => pdf::materialize_page(path, *page, *kind, *scale_x, *scale_y),
    }
}

async fn wait_budget(
    reserved: &AtomicUsize,
    need: u64,
    budget: u64,
    stop: &AtomicBool,
) {
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let cur = reserved.load(Ordering::SeqCst) as u64;
        if cur.saturating_add(need) <= budget.max(need) {
            reserved.fetch_add(need as usize, Ordering::SeqCst);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    }
}

async fn shrink(
    workers_n: &AtomicUsize,
    tile_cap: &Mutex<TileSize>,
    ev: &mpsc::UnboundedSender<Event>,
) {
    let n = workers_n.load(Ordering::SeqCst);
    if n > 1 {
        workers_n.store(n - 1, Ordering::SeqCst);
    } else {
        let mut cap = tile_cap.lock().await;
        if let Some(s) = cap.smaller() {
            *cap = s;
        }
    }
    let _ = ev.send(Event::Workers {
        n: workers_n.load(Ordering::SeqCst).max(1),
        tile: tile_cap.lock().await.px(),
    });
    let _ = ev.send(Event::Status(
        "内存不足, 已收缩并发/tile 并重新排队".into(),
    ));
}
