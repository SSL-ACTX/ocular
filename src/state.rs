// state.rs
use crate::model::TraceEvent;
use crossbeam_queue::ArrayQueue;
#[cfg(not(feature = "perfetto"))]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;
#[cfg(feature = "perfetto")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Instant;

pub static START_TIME: OnceLock<Instant> = OnceLock::new();
pub static EVENT_QUEUE: OnceLock<ArrayQueue<Vec<TraceEvent>>> = OnceLock::new();
pub static FREE_QUEUE: OnceLock<ArrayQueue<Vec<TraceEvent>>> = OnceLock::new();
pub static IS_RUNNING: AtomicBool = AtomicBool::new(false);
pub static IS_PRECISE: AtomicBool = AtomicBool::new(true);
pub static WORKER_THREAD: Mutex<Option<thread::JoinHandle<()>>> = Mutex::new(None);

pub static TSC_FREQ: OnceLock<f64> = OnceLock::new();
pub static START_TSC: OnceLock<u64> = OnceLock::new();

pub static DEINSTRUMENT_THRESHOLD: AtomicU32 = AtomicU32::new(500);

#[cfg(feature = "perfetto")]
pub static IS_PERFETTO_ENABLED: AtomicBool = AtomicBool::new(true);

#[allow(dead_code)]
pub fn is_perfetto_enabled() -> bool {
    #[cfg(feature = "perfetto")]
    {
        IS_PERFETTO_ENABLED.load(Ordering::Relaxed)
    }

    #[cfg(not(feature = "perfetto"))]
    {
        false
    }
}

pub fn set_perfetto_enabled(enabled: bool) {
    #[cfg(feature = "perfetto")]
    {
        IS_PERFETTO_ENABLED.store(enabled, Ordering::Relaxed);
    }
    #[cfg(not(feature = "perfetto"))]
    {
        let _ = enabled;
    }
}

/// Reads the hardware Time Stamp Counter (TSC) for ultra-low-overhead cycle timing.
#[inline(always)]
pub fn read_tsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let mut val: u64;
        unsafe { std::arch::asm!("mrs {}, cntvct_el0", out(reg) val) };
        val
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        0
    }
}

pub fn init_tsc_calibration() {
    START_TSC.get_or_init(|| {
        let tsc1 = read_tsc();
        let t1 = Instant::now();
        thread::sleep(std::time::Duration::from_millis(5));
        let tsc2 = read_tsc();
        let t2 = Instant::now();
        let elapsed_us = t2.duration_since(t1).as_micros() as f64;
        if elapsed_us > 0.0 {
            let _ = TSC_FREQ.set((tsc2.saturating_sub(tsc1)) as f64 / elapsed_us);
        }
        let _ = START_TIME.set(t2);
        tsc2
    });
}

pub fn get_ts() -> u64 {
    if let (Some(&start_tsc), Some(&freq)) = (START_TSC.get(), TSC_FREQ.get()) {
        let current_tsc = read_tsc();
        if current_tsc > start_tsc {
            ((current_tsc - start_tsc) as f64 / freq) as u64
        } else {
            0
        }
    } else {
        0
    }
}
