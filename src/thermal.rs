//! 读芯片温度; 过高则停一停, 凉一点再继续. 读不到传感器就跳过, 不挡修复.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use sysinfo::{Component, Components};

/// Apple Silicon 大约 100–105°C 才硬限频. 96 是有风险但不会动不动就停.
const PAUSE_C: f32 = 96.0;
/// 滞回: 必须降到这个以下才继续, 避免 95/96 来回抖.
const RESUME_C: f32 = 88.0;
const MIN_C: f32 = 30.0;
const MAX_C: f32 = 115.0;

static COMPONENTS: Mutex<Option<Components>> = Mutex::new(None);

#[derive(Clone, Copy, Debug)]
pub struct ChipTemp {
    pub celsius: f32,
    pub paused: bool,
}

pub fn snap() -> Option<(f32, String)> {
    let mut guard = COMPONENTS.lock().ok()?;
    if guard.is_none() {
        *guard = Some(Components::new_with_refreshed_list());
    } else if let Some(c) = guard.as_mut() {
        c.refresh(false);
    }
    let comps = guard.as_ref()?;
    pick_hottest(comps)
}

/// 若超过暂停阈值则阻塞到降回恢复阈值 (或 stop).
pub fn pause_if_hot(
    stop: &AtomicBool,
    mut report: impl FnMut(String, Option<ChipTemp>),
) {
    let Some((t, label)) = snap() else {
        return;
    };
    if t < PAUSE_C {
        report(
            String::new(),
            Some(ChipTemp {
                celsius: t,
                paused: false,
            }),
        );
        return;
    }
    report(
        format!("芯片 {t:.0}°C ({label}), 暂停散热, 降到 {RESUME_C:.0}°C 再继续"),
        Some(ChipTemp {
            celsius: t,
            paused: true,
        }),
    );
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_secs(5));
        let Some((t, label)) = snap() else {
            return;
        };
        if t <= RESUME_C {
            report(
                format!("芯片 {t:.0}°C, 继续修复"),
                Some(ChipTemp {
                    celsius: t,
                    paused: false,
                }),
            );
            return;
        }
        report(
            format!("芯片 {t:.0}°C ({label}), 暂停散热, 降到 {RESUME_C:.0}°C 再继续"),
            Some(ChipTemp {
                celsius: t,
                paused: true,
            }),
        );
    }
}

fn pick_hottest(comps: &[Component]) -> Option<(f32, String)> {
    let mut best_chip: Option<(f32, String)> = None;
    let mut best_any: Option<(f32, String)> = None;
    for c in comps {
        let Some(t) = c.temperature() else {
            continue;
        };
        if !t.is_finite() || t < MIN_C || t > MAX_C {
            continue;
        }
        let label = c.label().to_string();
        if best_any.as_ref().map(|(u, _)| t > *u).unwrap_or(true) {
            best_any = Some((t, label.clone()));
        }
        if is_chip_label(&label) && best_chip.as_ref().map(|(u, _)| t > *u).unwrap_or(true) {
            best_chip = Some((t, label));
        }
    }
    best_chip.or(best_any)
}

fn is_chip_label(label: &str) -> bool {
    let l = label.to_ascii_lowercase();
    if ["nand", "airport", "wifi", "battery", "ambient", "palm", "ssd"]
        .iter()
        .any(|s| l.contains(s))
    {
        return false;
    }
    l.contains("cpu")
        || l.contains("gpu")
        || l.contains("acc")
        || l.contains("soc")
        || l.contains("die")
        || l.contains("mtr")
        || l.contains("ane")
        || l == "computer"
}
