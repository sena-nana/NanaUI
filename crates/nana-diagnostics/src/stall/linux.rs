//! Stack samples of the watched thread on Linux and Android.
//!
//! Linux cannot suspend one thread from another, so the watchdog asks the
//! watched thread to copy itself: it sends one real-time signal to that
//! thread (`tgkill`), and the handler — async-signal-safe, no allocation,
//! no lock, no libc call besides `gettid` — copies the interrupted
//! registers and the stack above the interrupted stack pointer into a
//! buffer the watchdog allocated beforehand, then returns. The frame-record
//! walk runs on the copy, on the watchdog.
//!
//! The handler is installed on the first sample and only while that signal
//! has no other handler; with `SA_RESTART`, most interrupted system calls
//! restart (those that never restart, such as `poll` or `nanosleep`, return
//! `EINTR`, which std and the event loop already retry). A thread that
//! blocks the signal is simply not sampled.

use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicPtr, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::frames::{self, MAX_FRAMES, StackCopy};
pub(super) use super::unix_symbols::describe;

/// How long the watchdog waits for the handler to run.
const HANDLER_WAIT: Duration = Duration::from_millis(200);

// Copy protocol: the watchdog moves IDLE -> REQUESTED, the handler
// REQUESTED -> COPYING -> DONE, the watchdog DONE -> IDLE. A watchdog that
// gives up moves REQUESTED -> IDLE; once COPYING it waits for DONE, so the
// buffer is never freed under the handler.
const IDLE: u8 = 0;
const REQUESTED: u8 = 1;
const COPYING: u8 = 2;
const DONE: u8 = 3;

static STATE: AtomicU8 = AtomicU8::new(IDLE);
static TARGET_TID: AtomicI32 = AtomicI32::new(0);
static STACK_HIGH: AtomicUsize = AtomicUsize::new(0);
static BUFFER: AtomicPtr<u64> = AtomicPtr::new(std::ptr::null_mut());
static BUFFER_WORDS: AtomicUsize = AtomicUsize::new(0);
static COPIED_WORDS: AtomicUsize = AtomicUsize::new(0);
static PC: AtomicU64 = AtomicU64::new(0);
static SP: AtomicU64 = AtomicU64::new(0);
static FP: AtomicU64 = AtomicU64::new(0);
/// One sample at a time per process: the slots above are shared.
static SAMPLING: Mutex<()> = Mutex::new(());
/// The signal the handler is installed on, or 0 before the first sample,
/// or -1 when it could not be installed.
static SIGNAL: AtomicI32 = AtomicI32::new(0);

/// The handler as `sigaction` stores it.
fn handler_address() -> libc::sighandler_t {
    let handler: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) = on_signal;
    handler as *const () as libc::sighandler_t
}

fn gettid() -> libc::pid_t {
    // SAFETY: gettid has no arguments and cannot fail.
    unsafe { libc::syscall(libc::SYS_gettid) as libc::pid_t }
}

/// The interrupted thread's program counter, stack and frame pointers.
///
/// # Safety
/// `context` is the `ucontext_t` the kernel passed to an `SA_SIGINFO` handler.
unsafe fn registers(context: *const libc::c_void) -> (u64, u64, u64) {
    // SAFETY: per the contract above.
    let context = unsafe { &*context.cast::<libc::ucontext_t>() };
    #[cfg(target_arch = "x86_64")]
    {
        let gregs = &context.uc_mcontext.gregs;
        (
            gregs[libc::REG_RIP as usize] as u64,
            gregs[libc::REG_RSP as usize] as u64,
            gregs[libc::REG_RBP as usize] as u64,
        )
    }
    #[cfg(target_arch = "aarch64")]
    {
        let mcontext = &context.uc_mcontext;
        (mcontext.pc, mcontext.sp, mcontext.regs[29])
    }
}

extern "C" fn on_signal(
    _signal: libc::c_int,
    _info: *mut libc::siginfo_t,
    context: *mut libc::c_void,
) {
    if STATE
        .compare_exchange(REQUESTED, COPYING, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    // A copy of the signal sent to the process can land on another thread.
    if gettid() != TARGET_TID.load(Ordering::Relaxed) {
        STATE.store(REQUESTED, Ordering::Release);
        return;
    }
    // SAFETY: the kernel passes the interrupted context to SA_SIGINFO handlers.
    let (pc, sp, fp) = unsafe { registers(context) };
    let high = STACK_HIGH.load(Ordering::Relaxed);
    let sp_at = sp as usize;
    let mut words = 0;
    // The handler's own frame is below `sp`: [sp, high) holds still.
    if sp_at < high
        && sp_at.is_multiple_of(8)
        && high - sp_at <= BUFFER_WORDS.load(Ordering::Relaxed) * 8
    {
        words = (high - sp_at) / 8;
        // SAFETY: the stack range is this thread's own and mapped; the
        // buffer holds BUFFER_WORDS words and outlives COPYING.
        unsafe {
            std::ptr::copy_nonoverlapping(
                sp_at as *const u64,
                BUFFER.load(Ordering::Relaxed),
                words,
            );
        }
    }
    PC.store(pc, Ordering::Relaxed);
    SP.store(sp, Ordering::Relaxed);
    FP.store(fp, Ordering::Relaxed);
    COPIED_WORDS.store(words, Ordering::Relaxed);
    STATE.store(DONE, Ordering::Release);
}

/// The sampling signal, installing the handler on first use.
fn signal() -> Option<libc::c_int> {
    match SIGNAL.load(Ordering::Acquire) {
        -1 => return None,
        0 => {}
        signal => return Some(signal),
    }
    // Clear of the low real-time signals libraries and runtimes favour.
    let signal = libc::SIGRTMAX() - 3;
    // SAFETY: sigaction with valid out/in structs; the handler is
    // async-signal-safe.
    let installed = unsafe {
        let mut current: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(signal, std::ptr::null(), &mut current) != 0 {
            false
        } else if current.sa_sigaction == handler_address() {
            true
        } else if current.sa_sigaction != libc::SIG_DFL {
            // Someone else's: leave it alone.
            false
        } else {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = handler_address();
            action.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut()) == 0
        }
    };
    SIGNAL.store(if installed { signal } else { -1 }, Ordering::Release);
    installed.then_some(signal)
}

pub(super) struct Thread {
    pid: libc::pid_t,
    tid: libc::pid_t,
    clock: Option<libc::clockid_t>,
    stack_low: usize,
    stack_high: usize,
}

impl Thread {
    /// The calling thread: its id, CPU clock and stack range.
    pub(super) fn current() -> Self {
        // SAFETY: plain pthread calls on the calling thread with valid
        // out-pointers; the attribute object is destroyed after use.
        unsafe {
            let this = libc::pthread_self();
            let mut clock: libc::clockid_t = 0;
            let clock = (libc::pthread_getcpuclockid(this, &mut clock) == 0).then_some(clock);
            let (mut stack_low, mut stack_high) = (0usize, 0usize);
            let mut attr: libc::pthread_attr_t = std::mem::zeroed();
            if libc::pthread_getattr_np(this, &mut attr) == 0 {
                let mut address: *mut libc::c_void = std::ptr::null_mut();
                let mut size: libc::size_t = 0;
                if libc::pthread_attr_getstack(&attr, &mut address, &mut size) == 0 {
                    stack_low = address as usize;
                    stack_high = stack_low.saturating_add(size);
                }
                libc::pthread_attr_destroy(&mut attr);
            }
            Self {
                pid: libc::getpid(),
                tid: gettid(),
                clock,
                stack_low,
                stack_high,
            }
        }
    }

    pub(super) fn cpu_time(&self) -> Option<Duration> {
        let clock = self.clock?;
        // SAFETY: zeroed is a valid timespec; the clock is this process's.
        let mut time: libc::timespec = unsafe { std::mem::zeroed() };
        // SAFETY: valid clock id and out-pointer.
        if unsafe { libc::clock_gettime(clock, &mut time) } != 0 {
            return None;
        }
        Some(Duration::new(
            time.tv_sec.max(0) as u64,
            time.tv_nsec.clamp(0, 999_999_999) as u32,
        ))
    }

    /// Fill `frames` with the thread's current call stack, innermost first.
    pub(super) fn sample(&self, frames: &mut Vec<u64>) -> bool {
        if self.stack_high <= self.stack_low {
            return false;
        }
        let Some(signal) = signal() else {
            return false;
        };
        let _one = SAMPLING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut copy = vec![0u64; (self.stack_high - self.stack_low) / 8];
        TARGET_TID.store(self.tid, Ordering::Relaxed);
        STACK_HIGH.store(self.stack_high, Ordering::Relaxed);
        BUFFER.store(copy.as_mut_ptr(), Ordering::Relaxed);
        BUFFER_WORDS.store(copy.len(), Ordering::Relaxed);
        STATE.store(REQUESTED, Ordering::Release);

        // SAFETY: tgkill to a thread of this process, with a handled signal.
        let sent = unsafe { libc::syscall(libc::SYS_tgkill, self.pid, self.tid, signal) } == 0;
        let deadline = Instant::now() + HANDLER_WAIT;
        let done = sent
            && loop {
                if STATE.load(Ordering::Acquire) == DONE {
                    break true;
                }
                if Instant::now() >= deadline {
                    break false;
                }
                std::thread::sleep(Duration::from_micros(200));
            };
        if !done {
            // Withdraw the request; a handler already copying finishes first.
            if STATE
                .compare_exchange(REQUESTED, IDLE, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                while STATE.load(Ordering::Acquire) == COPYING {
                    std::hint::spin_loop();
                }
            }
            STATE.store(IDLE, Ordering::Release);
            BUFFER.store(std::ptr::null_mut(), Ordering::Relaxed);
            BUFFER_WORDS.store(0, Ordering::Relaxed);
            return false;
        }
        let (pc, sp, fp, words) = (
            PC.load(Ordering::Relaxed),
            SP.load(Ordering::Relaxed),
            FP.load(Ordering::Relaxed),
            COPIED_WORDS.load(Ordering::Relaxed),
        );
        BUFFER.store(std::ptr::null_mut(), Ordering::Relaxed);
        BUFFER_WORDS.store(0, Ordering::Relaxed);
        STATE.store(IDLE, Ordering::Release);

        let stack = StackCopy {
            words: &copy[..words],
            low: sp,
        };
        let mut found = [0u64; MAX_FRAMES];
        let count = frames::walk(pc, fp, &stack, &mut found);
        frames.extend_from_slice(&found[..count]);
        !frames.is_empty()
    }
}

#[cfg(test)]
mod tests {
    super::super::sampling_tests!();
}
