//! Stack samples of the watched thread on macOS and iOS, through Mach.
//!
//! The watchdog suspends the thread (`thread_suspend`), reads its registers
//! (`thread_get_state`), copies its stack from the stack pointer up into a
//! buffer allocated beforehand, and resumes it: nothing in between
//! allocates or locks, since the suspended thread may hold the allocator.
//! The frame-record walk then runs on the copy. Apple's ABIs keep frame
//! pointers on both architectures, so the walk sees the whole chain.

use std::time::Duration;

use super::frames::{self, MAX_FRAMES, StackCopy};
pub(super) use super::unix_symbols::describe;

type KernReturn = libc::c_int;
type MachPort = libc::mach_port_t;

unsafe extern "C" {
    fn thread_suspend(thread: MachPort) -> KernReturn;
    fn thread_resume(thread: MachPort) -> KernReturn;
    fn thread_get_state(
        thread: MachPort,
        flavor: libc::c_int,
        state: *mut u32,
        count: *mut libc::mach_msg_type_number_t,
    ) -> KernReturn;
    fn mach_port_deallocate(task: MachPort, name: MachPort) -> KernReturn;
    fn mach_thread_self() -> MachPort;
    static mach_task_self_: MachPort;
}

const KERN_SUCCESS: KernReturn = 0;
const THREAD_BASIC_INFO: libc::c_int = 3;

/// `<mach/arm/_structs.h>`: `__darwin_arm_thread_state64`.
#[cfg(target_arch = "aarch64")]
#[repr(C)]
#[derive(Default)]
struct State {
    x: [u64; 29],
    fp: u64,
    lr: u64,
    sp: u64,
    pc: u64,
    cpsr: u32,
    flags: u32,
}
#[cfg(target_arch = "aarch64")]
const STATE_FLAVOR: libc::c_int = 6; // ARM_THREAD_STATE64

#[cfg(target_arch = "aarch64")]
impl State {
    fn registers(&self) -> (u64, u64, u64) {
        (self.pc, self.sp, self.fp)
    }
}

/// `<mach/i386/_structs.h>`: `__darwin_x86_thread_state64`.
#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Default)]
struct State {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rdi: u64,
    rsi: u64,
    rbp: u64,
    rsp: u64,
    r: [u64; 8],
    rip: u64,
    rflags: u64,
    cs: u64,
    fs: u64,
    gs: u64,
}
#[cfg(target_arch = "x86_64")]
const STATE_FLAVOR: libc::c_int = 4; // x86_THREAD_STATE64

#[cfg(target_arch = "x86_64")]
impl State {
    fn registers(&self) -> (u64, u64, u64) {
        (self.rip, self.rsp, self.rbp)
    }
}

pub(super) struct Thread {
    port: MachPort,
    stack_low: usize,
    stack_high: usize,
}

impl Thread {
    /// The calling thread: its Mach port and stack range.
    pub(super) fn current() -> Self {
        // SAFETY: plain calls on the calling thread; the port right is ours
        // and released on drop.
        unsafe {
            let this = libc::pthread_self();
            let high = libc::pthread_get_stackaddr_np(this) as usize;
            let size = libc::pthread_get_stacksize_np(this);
            Self {
                port: mach_thread_self(),
                stack_low: high.saturating_sub(size),
                stack_high: high,
            }
        }
    }

    pub(super) fn cpu_time(&self) -> Option<Duration> {
        // SAFETY: zeroed is a valid empty thread_basic_info.
        let mut info: libc::thread_basic_info = unsafe { std::mem::zeroed() };
        let mut count = libc::THREAD_BASIC_INFO_COUNT;
        // SAFETY: the buffer is a thread_basic_info and `count` its size.
        let status = unsafe {
            libc::thread_info(
                self.port,
                THREAD_BASIC_INFO as libc::thread_flavor_t,
                (&raw mut info).cast(),
                &mut count,
            )
        };
        if status != KERN_SUCCESS {
            return None;
        }
        let time = |value: libc::time_value_t| {
            Duration::from_secs(value.seconds.max(0) as u64)
                + Duration::from_micros(value.microseconds.max(0) as u64)
        };
        Some(time(info.user_time) + time(info.system_time))
    }

    /// Fill `frames` with the thread's current call stack, innermost first.
    pub(super) fn sample(&self, frames: &mut Vec<u64>) -> bool {
        if self.stack_high <= self.stack_low {
            return false;
        }
        // Everything the suspended window needs is allocated before it.
        let mut copy = vec![0u64; (self.stack_high - self.stack_low) / 8];
        let mut state = State::default();
        let mut count = (size_of::<State>() / size_of::<u32>()) as libc::mach_msg_type_number_t;

        // SAFETY: suspend, read registers, copy the stack the thread has in
        // use ([sp, high), which it cannot release while suspended), resume.
        let (pc, sp, fp, copied) = unsafe {
            if thread_suspend(self.port) != KERN_SUCCESS {
                return false;
            }
            let read =
                thread_get_state(self.port, STATE_FLAVOR, (&raw mut state).cast(), &mut count)
                    == KERN_SUCCESS;
            let (pc, sp, fp) = state.registers();
            let sp_at = sp as usize;
            let usable = read
                && sp_at >= self.stack_low
                && sp_at < self.stack_high
                && sp_at.is_multiple_of(8);
            let words = if usable {
                (self.stack_high - sp_at) / 8
            } else {
                0
            };
            if usable {
                std::ptr::copy_nonoverlapping(sp_at as *const u64, copy.as_mut_ptr(), words);
            }
            thread_resume(self.port);
            if !usable {
                return false;
            }
            (pc, sp, fp, words)
        };

        let stack = StackCopy {
            words: &copy[..copied],
            low: sp,
        };
        let mut found = [0u64; MAX_FRAMES];
        let count = frames::walk(pc, fp, &stack, &mut found);
        frames.extend_from_slice(&found[..count]);
        !frames.is_empty()
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        // SAFETY: releases the send right `mach_thread_self` gave us.
        unsafe {
            mach_port_deallocate(mach_task_self_, self.port);
        }
    }
}

#[cfg(test)]
mod tests {
    super::super::sampling_tests!();
}
