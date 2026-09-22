//! The platform monotonic clock behind `std::time::Instant`, read as an
//! absolute value so a session's zero can be written to the `.nlog` header.

/// Nanoseconds on the clock `Instant` uses on this platform, or 0 where it
/// is not read.
pub(crate) fn monotonic_now_ns() -> u64 {
    imp::now()
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod imp {
    // <time.h>: CLOCK_UPTIME_RAW, what `Instant` uses on Apple platforms.
    const CLOCK_UPTIME_RAW: u32 = 8;
    unsafe extern "C" {
        fn clock_gettime_nsec_np(clock_id: u32) -> u64;
    }
    pub(super) fn now() -> u64 {
        // SAFETY: plain libSystem call.
        unsafe { clock_gettime_nsec_np(CLOCK_UPTIME_RAW) }
    }
}

#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    target_pointer_width = "64"
))]
mod imp {
    const CLOCK_MONOTONIC: i32 = 1;
    #[repr(C)]
    struct Timespec {
        tv_sec: i64,
        tv_nsec: i64,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: i32, tp: *mut Timespec) -> i32;
    }
    pub(super) fn now() -> u64 {
        let mut ts = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `ts` is a valid, writable timespec for the call.
        if unsafe { clock_gettime(CLOCK_MONOTONIC, &mut ts) } != 0 {
            return 0;
        }
        (ts.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(ts.tv_nsec as u64)
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::Performance::{
        QueryPerformanceCounter, QueryPerformanceFrequency,
    };
    pub(super) fn now() -> u64 {
        let (mut counter, mut frequency) = (0i64, 0i64);
        // SAFETY: both out-pointers are valid for the calls.
        let ok = unsafe {
            QueryPerformanceCounter(&mut counter) != 0
                && QueryPerformanceFrequency(&mut frequency) != 0
        };
        if !ok || frequency <= 0 || counter < 0 {
            return 0;
        }
        u64::try_from(counter as u128 * 1_000_000_000 / frequency as u128).unwrap_or(u64::MAX)
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    all(
        any(target_os = "linux", target_os = "android"),
        target_pointer_width = "64"
    ),
    windows
)))]
mod imp {
    pub(super) fn now() -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_platform_clock_moves_forward() {
        let a = super::monotonic_now_ns();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = super::monotonic_now_ns();
        if a != 0 {
            assert!(b > a);
        }
    }
}
