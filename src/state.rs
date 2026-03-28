// state.rs
use crate::model::TraceEvent;
use crossbeam_queue::ArrayQueue;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};
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

pub static IS_PERFETTO_ENABLED: AtomicBool = AtomicBool::new(false);

pub struct PatternSet {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl Default for PatternSet {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
}

pub static FILTER_PATTERNS: OnceLock<RwLock<PatternSet>> = OnceLock::new();

#[allow(dead_code)]
pub fn is_perfetto_enabled() -> bool {
    IS_PERFETTO_ENABLED.load(Ordering::Relaxed)
}

pub fn set_perfetto_enabled(enabled: bool) {
    IS_PERFETTO_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn set_exclude_patterns(patterns: Vec<String>) {
    let rw = FILTER_PATTERNS.get_or_init(|| RwLock::new(PatternSet::default()));
    if let Ok(mut guard) = rw.write() {
        guard.exclude = patterns;
    }
}

pub fn get_exclude_patterns() -> Vec<String> {
    if let Some(rw) = FILTER_PATTERNS.get() {
        if let Ok(guard) = rw.read() {
            return guard.exclude.clone();
        }
    }
    Vec::new()
}

pub fn clear_exclude_patterns() {
    if let Some(rw) = FILTER_PATTERNS.get() {
        if let Ok(mut guard) = rw.write() {
            guard.exclude.clear();
        }
    }
}

pub fn set_include_patterns(patterns: Vec<String>) {
    let rw = FILTER_PATTERNS.get_or_init(|| RwLock::new(PatternSet::default()));
    if let Ok(mut guard) = rw.write() {
        guard.include = patterns;
    }
}

pub fn get_include_patterns() -> Vec<String> {
    if let Some(rw) = FILTER_PATTERNS.get() {
        if let Ok(guard) = rw.read() {
            return guard.include.clone();
        }
    }
    Vec::new()
}

pub fn clear_include_patterns() {
    if let Some(rw) = FILTER_PATTERNS.get() {
        if let Ok(mut guard) = rw.write() {
            guard.include.clear();
        }
    }
}

pub fn with_pattern_set<R>(f: impl FnOnce(&PatternSet) -> R) -> Option<R> {
    if let Some(rw) = FILTER_PATTERNS.get() {
        if let Ok(guard) = rw.read() {
            return Some(f(&*guard));
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfetto_is_disabled_by_default() {
        set_perfetto_enabled(false);
        assert_eq!(is_perfetto_enabled(), false);
    }

    #[test]
    fn perfetto_toggle_on_off() {
        set_perfetto_enabled(true);
        assert_eq!(is_perfetto_enabled(), true);
        set_perfetto_enabled(false);
        assert_eq!(is_perfetto_enabled(), false);
    }

    #[test]
    fn exclude_patterns_can_be_set_and_cleared() {
        set_exclude_patterns(vec!["/usr".to_string(), "_unpack_opargs".to_string()]);
        let patterns = get_exclude_patterns();
        assert!(patterns.contains(&"/usr".to_string()));
        assert!(patterns.contains(&"_unpack_opargs".to_string()));

        clear_exclude_patterns();
        assert!(get_exclude_patterns().is_empty());
    }

    #[test]
    fn include_patterns_can_be_set_and_cleared() {
        set_include_patterns(vec!["tests/test3.py".to_string()]);
        let patterns = get_include_patterns();
        assert!(patterns.contains(&"tests/test3.py".to_string()));

        clear_include_patterns();
        assert!(get_include_patterns().is_empty());
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
