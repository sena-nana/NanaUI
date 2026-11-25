//! `AChoreographer` hardware vsync phase-aligned pacing for Android.
//!
//! `android-activity`'s `GameActivity::poll_events` polls the main native thread's
//! `ALooper` directly and treats `ALOOPER_POLL_CALLBACK` results as errors. To align
//! frame production cleanly to display vsync without touching the main thread's
//! looper registrations, [`ChoreographerDriver`] runs a lightweight dedicated
//! `ALooper` thread hosting `AChoreographer_getInstance()`, arming a single
//! `AChoreographer_postFrameCallback` when `FrameDemand::Now` is active and waking
//! the main loop via `AndroidAppWaker::wake()` when vsync fires.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

/// Host-testable vsync arming / coalescing state machine shared with the Android
/// choreographer callback thread.
#[derive(Debug, Default)]
pub struct VsyncGate {
    pending_vsync: AtomicBool,
    post_requested: AtomicBool,
    frame_ready: AtomicBool,
    last_frame_time_ns: AtomicI64,
}

impl VsyncGate {
    pub const fn new() -> Self {
        Self {
            pending_vsync: AtomicBool::new(false),
            post_requested: AtomicBool::new(false),
            frame_ready: AtomicBool::new(false),
            last_frame_time_ns: AtomicI64::new(0),
        }
    }

    /// Requests the next hardware vsync callback if not already pending or ready.
    /// Returns `true` when the caller should wake the choreographer looper.
    pub fn request_vsync(&self) -> bool {
        if self.frame_ready.load(Ordering::Acquire) {
            return false;
        }
        if self
            .pending_vsync
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.post_requested.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }

    /// Consumes a queued post request inside the choreographer looper thread.
    #[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
    pub fn take_post_request(&self) -> bool {
        self.post_requested.swap(false, Ordering::AcqRel)
    }

    /// Records a hardware vsync tick delivered by `AChoreographer`.
    #[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
    pub fn on_vsync(&self, frame_time_ns: i64) {
        self.last_frame_time_ns
            .store(frame_time_ns, Ordering::Release);
        self.pending_vsync.store(false, Ordering::Release);
        self.frame_ready.store(true, Ordering::Release);
    }

    /// Marks the next frame immediately eligible (used on surface creation /
    /// explicit redraw lifecycle events).
    pub fn mark_immediate_ready(&self) {
        self.pending_vsync.store(false, Ordering::Release);
        self.post_requested.store(false, Ordering::Release);
        self.frame_ready.store(true, Ordering::Release);
    }

    pub fn is_frame_ready(&self) -> bool {
        self.frame_ready.load(Ordering::Acquire)
    }

    /// Consumes the armed frame token when rendering on the main loop.
    pub fn take_frame_ready(&self) -> bool {
        self.frame_ready.swap(false, Ordering::AcqRel)
    }

    /// Clears pending vsync requests when the document goes `Idle` or detaches.
    pub fn disarm(&self) {
        self.pending_vsync.store(false, Ordering::Release);
        self.post_requested.store(false, Ordering::Release);
        self.frame_ready.store(false, Ordering::Release);
    }

    #[cfg(test)]
    pub fn last_frame_time_ns(&self) -> i64 {
        self.last_frame_time_ns.load(Ordering::Acquire)
    }
}

#[cfg(target_os = "android")]
mod android {
    use std::ffi::{c_int, c_long, c_void};
    use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use android_activity::AndroidAppWaker;

    use super::VsyncGate;

    #[repr(C)]
    struct AChoreographer {
        _unused: [u8; 0],
    }

    #[repr(C)]
    struct ALooper {
        _unused: [u8; 0],
    }

    const ALOOPER_PREPARE_ALLOW_NON_CALLBACKS: c_int = 1 << 0;

    #[link(name = "android")]
    unsafe extern "C" {
        fn ALooper_prepare(opts: c_int) -> *mut ALooper;
        fn ALooper_pollOnce(
            timeout_millis: c_int,
            out_fd: *mut c_int,
            out_events: *mut c_int,
            out_data: *mut *mut c_void,
        ) -> c_int;
        fn ALooper_wake(looper: *mut ALooper);
        fn AChoreographer_getInstance() -> *mut AChoreographer;
        fn AChoreographer_postFrameCallback(
            choreographer: *mut AChoreographer,
            callback: unsafe extern "C" fn(c_long, *mut c_void),
            data: *mut c_void,
        );
    }

    struct CallbackContext {
        gate: Arc<VsyncGate>,
        waker: AndroidAppWaker,
    }

    unsafe extern "C" fn vsync_callback(frame_time_nanos: c_long, data: *mut c_void) {
        if data.is_null() {
            return;
        }
        // SAFETY: `data` points to `CallbackContext` owned by `ChoreographerDriver`
        // for the entire lifetime of the choreographer thread.
        let ctx = unsafe { &*(data as *const CallbackContext) };
        ctx.gate.on_vsync(frame_time_nanos as i64);
        ctx.waker.wake();
    }

    pub(crate) struct ChoreographerDriver {
        gate: Arc<VsyncGate>,
        looper: Arc<AtomicPtr<ALooper>>,
        stop: Arc<AtomicBool>,
        available: bool,
        thread: Option<JoinHandle<()>>,
    }

    // SAFETY: `ALooper_wake` is explicitly thread-safe in the Android NDK.
    unsafe impl Send for ChoreographerDriver {}

    impl ChoreographerDriver {
        pub(crate) fn new(waker: AndroidAppWaker) -> Self {
            let gate = Arc::new(VsyncGate::new());
            let looper_ptr = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
            let stop = Arc::new(AtomicBool::new(false));
            let (ready_tx, ready_rx) = mpsc::channel::<bool>();

            let thread_gate = Arc::clone(&gate);
            let thread_looper = Arc::clone(&looper_ptr);
            let thread_stop = Arc::clone(&stop);

            let thread = std::thread::Builder::new()
                .name("nana-android-choreographer".into())
                .spawn(move || {
                    let looper = unsafe { ALooper_prepare(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS) };
                    let choreographer = if looper.is_null() {
                        std::ptr::null_mut()
                    } else {
                        unsafe { AChoreographer_getInstance() }
                    };
                    if looper.is_null() || choreographer.is_null() {
                        let _ = ready_tx.send(false);
                        return;
                    }
                    thread_looper.store(looper, Ordering::Release);
                    let ctx = Arc::new(CallbackContext {
                        gate: Arc::clone(&thread_gate),
                        waker,
                    });
                    let ctx_ptr = Arc::into_raw(ctx) as *mut c_void;
                    let _ = ready_tx.send(true);

                    while !thread_stop.load(Ordering::Acquire) {
                        if thread_gate.take_post_request() {
                            unsafe {
                                AChoreographer_postFrameCallback(
                                    choreographer,
                                    vsync_callback,
                                    ctx_ptr,
                                );
                            }
                        }
                        let mut fd = 0;
                        let mut events = 0;
                        let mut data = std::ptr::null_mut();
                        unsafe {
                            ALooper_pollOnce(-1, &mut fd, &mut events, &mut data);
                        }
                    }

                    thread_looper.store(std::ptr::null_mut(), Ordering::Release);
                    // Reclaim context once the thread stops polling `choreographer`.
                    unsafe {
                        drop(Arc::from_raw(ctx_ptr as *const CallbackContext));
                    }
                })
                .ok();

            let available = ready_rx
                .recv_timeout(Duration::from_millis(500))
                .unwrap_or(false);
            Self {
                gate,
                looper: looper_ptr,
                stop,
                available,
                thread,
            }
        }

        pub(crate) fn request_vsync(&self) {
            if !self.available {
                self.gate.mark_immediate_ready();
                return;
            }
            if self.gate.request_vsync() {
                let looper = self.looper.load(Ordering::Acquire);
                if !looper.is_null() {
                    unsafe { ALooper_wake(looper) };
                }
            }
        }

        pub(crate) fn mark_immediate_ready(&self) {
            self.gate.mark_immediate_ready();
        }

        pub(crate) fn is_frame_ready(&self) -> bool {
            self.gate.is_frame_ready()
        }

        pub(crate) fn take_frame_ready(&self) -> bool {
            self.gate.take_frame_ready()
        }

        pub(crate) fn disarm(&self) {
            self.gate.disarm();
        }
    }

    impl Drop for ChoreographerDriver {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            let looper = self.looper.load(Ordering::Acquire);
            if !looper.is_null() {
                unsafe { ALooper_wake(looper) };
            }
            if let Some(handle) = self.thread.take() {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(target_os = "android")]
pub(crate) use android::ChoreographerDriver;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vsync_gate_coalesces_requests_and_clears_on_idle() {
        let gate = VsyncGate::new();
        assert!(!gate.is_frame_ready());
        assert!(gate.request_vsync());
        // Duplicate request while pending does not re-post.
        assert!(!gate.request_vsync());
        assert!(gate.take_post_request());
        assert!(!gate.take_post_request());

        gate.on_vsync(16_666_666);
        assert_eq!(gate.last_frame_time_ns(), 16_666_666);
        assert!(gate.is_frame_ready());
        assert!(!gate.request_vsync());
        assert!(gate.take_frame_ready());
        assert!(!gate.is_frame_ready());

        gate.mark_immediate_ready();
        assert!(gate.is_frame_ready());
        gate.disarm();
        assert!(!gate.is_frame_ready());
    }
}
