//! A tiny system-wide CPU sampler for the M4.5 phase measurements (BENCH-M4
//! §5): "загрузка процессора" per phase, without a new dependency.
//!
//! Windows reads `GetSystemTimes`, Linux `/proc/stat`; any other platform
//! reports `None` and the report says «не измерено». The value is the share
//! of busy CPU time across all cores between two samples, so it can approach
//! 100 % when every core is busy and reflects the machine, not this process.

/// One system-wide CPU reading: busy and idle time in platform units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuSample {
    busy: u64,
    idle: u64,
}

/// Reads the current system CPU counters, if the platform has them.
pub fn sample() -> Option<CpuSample> {
    platform::times().map(|(busy, idle)| CpuSample { busy, idle })
}

/// Busy share (0..=100) of the CPU time between two samples; `None` when the
/// platform has no counters or the interval saw no CPU time at all.
pub fn busy_percent(start: CpuSample, end: CpuSample) -> Option<f64> {
    let busy = end.busy.checked_sub(start.busy)?;
    let idle = end.idle.checked_sub(start.idle)?;
    let total = busy + idle;
    if total == 0 {
        return None;
    }
    Some(busy as f64 * 100.0 / total as f64)
}

#[cfg(windows)]
mod platform {
    //! `GetSystemTimes`: kernel + user + idle, in 100 ns FILETIME ticks.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    impl FileTime {
        fn as_u64(self) -> u64 {
            (u64::from(self.high) << 32) | u64::from(self.low)
        }
    }

    // SOURCE: Microsoft Win32 `GetSystemTimes`.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemTimes(idle: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
    }

    pub(super) fn times() -> Option<(u64, u64)> {
        let (mut idle, mut kernel, mut user) = (
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
        );
        // SAFETY: `GetSystemTimes` writes three FILETIME values into the
        // pointers; all three out-params are valid, aligned, live locals.
        let ok = unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
        if ok == 0 {
            return None;
        }
        // `kernel` already includes idle time; busy is user + non-idle kernel.
        let idle = idle.as_u64();
        let busy = kernel
            .as_u64()
            .saturating_add(user.as_u64())
            .saturating_sub(idle);
        Some((busy, idle))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    //! The first `/proc/stat` line: user nice system idle iowait irq softirq
    //! steal guest guest_nice (all in USER_HZ).
    pub(super) fn times() -> Option<(u64, u64)> {
        let content = std::fs::read_to_string("/proc/stat").ok()?;
        let mut fields = content.lines().next()?.split_whitespace();
        if fields.next()? != "cpu" {
            return None;
        }
        let values: Vec<u64> = fields.filter_map(|v| v.parse().ok()).take(8).collect();
        if values.len() < 5 {
            return None;
        }
        let idle = values[3] + values[4]; // idle + iowait
        let total: u64 = values.iter().sum();
        Some((total.saturating_sub(idle), idle))
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub(super) fn times() -> Option<(u64, u64)> {
        None
    }
}
