//! Remaining thread-stack queries via native platform APIs.
//!
//! All tree traversals that can scale with `MAX_TREE_DEPTH` run on explicit heap
//! work-stacks or bounded `recursion::SPAN` queues; only layout stack-budget
//! instrumentation reads how much stack remains on the calling thread.

/// Stack left on this thread, in bytes; `None` where the platform cannot
/// tell.
pub(crate) fn remaining() -> Option<usize> {
    native::remaining()
}

#[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
mod native {
    use std::cell::Cell;

    thread_local! {
        /// The lowest address of this thread's stack, read once per thread.
        static LIMIT: Cell<Option<Option<usize>>> = const { Cell::new(None) };
    }

    pub(super) fn remaining() -> Option<usize> {
        let marker = 0u8;
        let here = std::ptr::addr_of!(marker) as usize;
        let limit = LIMIT.with(|cell| {
            let limit = cell.get().unwrap_or_else(lowest_address);
            cell.set(Some(limit));
            limit
        })?;
        Some(here.saturating_sub(limit))
    }

    #[cfg(target_vendor = "apple")]
    fn lowest_address() -> Option<usize> {
        // SAFETY: both read the calling thread's own attributes.
        let (top, size) = unsafe {
            let thread = libc::pthread_self();
            (
                libc::pthread_get_stackaddr_np(thread) as usize,
                libc::pthread_get_stacksize_np(thread),
            )
        };
        top.checked_sub(size)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn lowest_address() -> Option<usize> {
        // SAFETY: `attr` is initialised by `pthread_getattr_np` before it is
        // read, and destroyed once.
        unsafe {
            let mut attr = std::mem::MaybeUninit::<libc::pthread_attr_t>::uninit();
            if libc::pthread_getattr_np(libc::pthread_self(), attr.as_mut_ptr()) != 0 {
                return None;
            }
            let mut base = std::ptr::null_mut();
            let mut size = 0;
            let read = libc::pthread_attr_getstack(attr.as_ptr(), &mut base, &mut size);
            libc::pthread_attr_destroy(attr.as_mut_ptr());
            (read == 0).then_some(base as usize)
        }
    }
}

#[cfg(target_os = "windows")]
mod native {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadStackLimits(low_limit: *mut usize, high_limit: *mut usize);
    }

    pub(super) fn remaining() -> Option<usize> {
        let marker = 0u8;
        let here = std::ptr::addr_of!(marker) as usize;
        let mut low = 0usize;
        let mut high = 0usize;
        // SAFETY: `GetCurrentThreadStackLimits` writes the valid stack bounds
        // of the calling thread into the two provided pointers.
        unsafe { GetCurrentThreadStackLimits(&mut low, &mut high) };
        (low != 0 && here >= low && here <= high).then(|| here - low)
    }
}

#[cfg(not(any(
    target_vendor = "apple",
    target_os = "linux",
    target_os = "android",
    target_os = "windows"
)))]
mod native {
    pub(super) fn remaining() -> Option<usize> {
        None
    }
}
