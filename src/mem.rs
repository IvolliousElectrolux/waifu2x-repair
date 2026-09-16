//! 进程工作集 + 系统可用内存, 给引擎决定并发.

use sysinfo::{MemoryRefreshKind, RefreshKind, System};

#[derive(Clone, Copy, Debug, Default)]
pub struct MemSnap {
    pub total: u64,
    pub available: u64,
    pub process: u64,
}

pub fn snap() -> MemSnap {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
    );
    sys.refresh_memory();
    let process = process_bytes();
    MemSnap {
        total: sys.total_memory(),
        available: sys.available_memory(),
        process,
    }
}

pub fn fmt_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let x = n as f64;
    if x >= GB {
        format!("{:.2}GB", x / GB)
    } else if x >= MB {
        format!("{:.1}MB", x / MB)
    } else if x >= KB {
        format!("{:.1}KB", x / KB)
    } else {
        format!("{n}B")
    }
}

fn process_bytes() -> u64 {
    #[cfg(windows)]
    {
        windows_process_memory().unwrap_or(0)
    }
    #[cfg(target_os = "macos")]
    {
        macos_process_memory().unwrap_or(0)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        0
    }
}

#[cfg(windows)]
fn windows_process_memory() -> Option<u64> {
    use std::mem::MaybeUninit;
    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    #[link(name = "psapi")]
    extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut core::ffi::c_void,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }
    extern "system" {
        fn GetCurrentProcess() -> *mut core::ffi::c_void;
    }
    unsafe {
        let mut c = MaybeUninit::<ProcessMemoryCounters>::zeroed();
        (*c.as_mut_ptr()).cb = std::mem::size_of::<ProcessMemoryCounters>() as u32;
        if GetProcessMemoryInfo(
            GetCurrentProcess(),
            c.as_mut_ptr(),
            std::mem::size_of::<ProcessMemoryCounters>() as u32,
        ) == 0
        {
            return None;
        }
        Some(c.assume_init().working_set_size as u64)
    }
}

#[cfg(target_os = "macos")]
fn macos_process_memory() -> Option<u64> {
    use std::mem::MaybeUninit;
    #[repr(C)]
    struct MachTaskBasicInfo {
        virtual_size: u64,
        resident_size: u64,
        resident_size_max: u64,
        user_time: [u32; 2],
        system_time: [u32; 2],
        policy: i32,
        suspend_count: i32,
    }
    extern "C" {
        fn mach_task_self() -> u32;
        fn task_info(
            target_task: u32,
            flavor: i32,
            task_info_out: *mut u64,
            task_info_outCnt: *mut u32,
        ) -> i32;
    }
    const MACH_TASK_BASIC_INFO: i32 = 20;
    unsafe {
        let mut info = MaybeUninit::<MachTaskBasicInfo>::zeroed();
        let mut count =
            (std::mem::size_of::<MachTaskBasicInfo>() / std::mem::size_of::<u64>()) as u32;
        let kr = task_info(
            mach_task_self(),
            MACH_TASK_BASIC_INFO,
            info.as_mut_ptr() as *mut u64,
            &mut count,
        );
        if kr != 0 {
            return None;
        }
        Some(info.assume_init().resident_size)
    }
}
