//! Best-effort lowering of the worker thread's scheduling priority. Failure
//! is ignored: the worker is correct at any priority.

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn lower_current_thread() {
    // <pthread/qos.h>: QOS_CLASS_UTILITY.
    const QOS_CLASS_UTILITY: u32 = 0x11;
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    }
    // SAFETY: plain libSystem call on the current thread.
    unsafe {
        pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0);
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn lower_current_thread() {
    const PRIO_PROCESS: i32 = 0;
    unsafe extern "C" {
        fn setpriority(which: i32, who: u32, prio: i32) -> i32;
    }
    // SAFETY: plain libc call. On Linux, `who = 0` with PRIO_PROCESS applies
    // to the calling thread only (threads are scheduling entities).
    unsafe {
        setpriority(PRIO_PROCESS, 0, 10);
    }
}

#[cfg(windows)]
pub(crate) fn lower_current_thread() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };
    // SAFETY: the pseudo-handle for the current thread is always valid.
    unsafe {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android",
    windows
)))]
pub(crate) fn lower_current_thread() {}
