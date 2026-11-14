//! Stack samples of the watched thread on Windows x64, and the stack walk
//! and symbolization the native crash writer shares.
//!
//! The thread is suspended only long enough to read its registers and copy
//! its stack: while it is suspended it may hold the heap or loader lock, so
//! nothing in between allocates, locks or looks up unwind data. Unwinding
//! then runs on the copy, with pointers into the original stack rewritten
//! to point into the copy, and stops at the first frame it cannot follow
//! inside the copy rather than read memory it cannot vouch for.

use std::fmt::Write as _;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, FILETIME, HANDLE, HMODULE,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    CONTEXT, CONTEXT_CONTROL_AMD64, CONTEXT_INTEGER_AMD64, GetThreadContext,
    IMAGE_RUNTIME_FUNCTION_ENTRY, RtlLookupFunctionEntry, RtlVirtualUnwind, SYMBOL_INFOW,
    SYMOPT_DEFERRED_LOADS, SYMOPT_FAIL_CRITICAL_ERRORS, SYMOPT_NO_PROMPTS, SYMOPT_UNDNAME,
    SymCleanup, SymFromAddrW, SymInitializeW, SymSetOptions, UNW_FLAG_NHANDLER,
};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, GetCurrentThreadStackLimits, GetThreadTimes, ResumeThread,
    SuspendThread,
};

pub(crate) const MAX_FRAMES: usize = 64;
/// Zeroed bytes past the copied stack, so an unwind step that reads a few
/// words beyond the last frame stays inside the buffer.
const SLACK_WORDS: usize = 512;
const NAME_CHARS: usize = 512;

/// `CONTEXT` must be 16-byte aligned for `GetThreadContext`.
#[repr(C, align(16))]
struct Context(CONTEXT);

pub(super) struct Thread {
    handle: HANDLE,
    stack_low: usize,
    stack_high: usize,
}

// SAFETY: a real thread handle may be used from any thread.
unsafe impl Send for Thread {}
// SAFETY: as above; the watchdog is the only user.
unsafe impl Sync for Thread {}

impl Thread {
    /// The calling thread, by a real handle (not the pseudo-handle) and its
    /// stack reservation.
    pub(super) fn current() -> Self {
        let mut handle: HANDLE = std::ptr::null_mut();
        let mut stack_low = 0usize;
        let mut stack_high = 0usize;
        // SAFETY: plain Win32 calls on this process and thread.
        unsafe {
            let process = GetCurrentProcess();
            if DuplicateHandle(
                process,
                GetCurrentThread(),
                process,
                &mut handle,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            ) == 0
            {
                handle = std::ptr::null_mut();
            }
            GetCurrentThreadStackLimits(&mut stack_low, &mut stack_high);
        }
        Self {
            handle,
            stack_low,
            stack_high,
        }
    }

    pub(super) fn cpu_time(&self) -> Option<Duration> {
        if self.handle.is_null() {
            return None;
        }
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: valid handle and out-pointers.
        let ok = unsafe {
            GetThreadTimes(
                self.handle,
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        };
        let ticks =
            |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
        (ok != 0).then(|| Duration::from_nanos((ticks(kernel) + ticks(user)).saturating_mul(100)))
    }

    /// Fill `frames` with the thread's current call stack, innermost first.
    pub(super) fn sample(&self, frames: &mut Vec<u64>) -> bool {
        if self.handle.is_null() || self.stack_high <= self.stack_low {
            return false;
        }
        // Everything the suspended window needs is allocated before it.
        let reserved = (self.stack_high - self.stack_low) / 8;
        let mut copy = vec![0u64; reserved + SLACK_WORDS];
        // SAFETY: zeroed CONTEXT is a valid all-default value.
        let mut context = Context(unsafe { std::mem::zeroed() });
        context.0.ContextFlags = CONTEXT_CONTROL_AMD64 | CONTEXT_INTEGER_AMD64;

        // SAFETY: suspend, read registers, copy the committed part of the
        // stack ([rsp, high), which a suspended thread cannot release), and
        // resume — no allocation, lock or loader call in between.
        let rsp = unsafe {
            if SuspendThread(self.handle) == u32::MAX {
                return false;
            }
            let read = GetThreadContext(self.handle, &mut context.0) != 0;
            let rsp = context.0.Rsp as usize;
            let usable =
                read && rsp >= self.stack_low && rsp < self.stack_high && rsp.is_multiple_of(8);
            if usable {
                std::ptr::copy_nonoverlapping(
                    rsp as *const u64,
                    copy.as_mut_ptr(),
                    (self.stack_high - rsp) / 8,
                );
            }
            ResumeThread(self.handle);
            if !usable {
                return false;
            }
            rsp
        };

        let words = (self.stack_high - rsp) / 8;
        let base = copy.as_ptr() as u64;
        let (original_low, original_high) = (rsp as u64, self.stack_high as u64);
        let translate = |value: u64| {
            if (original_low..original_high).contains(&value) {
                value - original_low + base
            } else {
                value
            }
        };
        for word in &mut copy[..words] {
            *word = translate(*word);
        }
        let context = &mut context.0;
        context.Rsp = base;
        let end = base + (words * 8) as u64;
        for register in [
            &mut context.Rbp,
            &mut context.Rbx,
            &mut context.Rsi,
            &mut context.Rdi,
            &mut context.R12,
            &mut context.R13,
            &mut context.R14,
            &mut context.R15,
        ] {
            *register = translate(*register);
        }

        let mut found = [0u64; MAX_FRAMES];
        let count = unwind(context, base, end, &mut found);
        frames.extend_from_slice(&found[..count]);
        !frames.is_empty()
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: we own the duplicated handle.
            unsafe { CloseHandle(self.handle) };
        }
    }
}

/// Walk `context` outwards, innermost frame first, into `frames`; returns
/// how many were found. Reads only the words in `[low, high)`, which must be
/// readable and not change meanwhile (a copy, or the stack of a thread that
/// is blocked), and stops at the first frame it cannot follow inside it. No
/// allocation, no lock: the native crash writer runs it while the crashed
/// thread may hold the heap lock.
pub(crate) fn unwind(context: &mut CONTEXT, low: u64, high: u64, frames: &mut [u64]) -> usize {
    let inside = |address: u64| address >= low && address.saturating_add(8) <= high;
    let mut count = 0;
    for depth in 0..frames.len() {
        let pc = context.Rip;
        if pc == 0 || !inside(context.Rsp) {
            break;
        }
        frames[count] = pc;
        count += 1;
        // A return address may sit just past its call's function.
        let lookup = if depth == 0 { pc } else { pc - 1 };
        let mut image_base = 0u64;
        // SAFETY: lookups read only the loaded image's unwind tables.
        let entry =
            unsafe { RtlLookupFunctionEntry(lookup, &mut image_base, std::ptr::null_mut()) };
        if entry.is_null() {
            if depth > 0 {
                break;
            }
            // A leaf function: the return address is on top.
            // SAFETY: `inside` checked the word.
            context.Rip = unsafe { *(context.Rsp as *const u64) };
            context.Rsp += 8;
            continue;
        }
        // A frame that addresses its locals through a frame register is
        // unwound from that register: it has to point into the copy.
        // SAFETY: the entry and its unwind info belong to a loaded image.
        let frame_register = unsafe {
            let entry: &IMAGE_RUNTIME_FUNCTION_ENTRY = &*entry;
            let info = (image_base + u64::from(entry.Anonymous.UnwindData)) as *const u8;
            *info.add(3) & 0x0f
        };
        if frame_register != 0 && !inside(register(context, frame_register)) {
            break;
        }
        let mut handler_data = std::ptr::null_mut();
        let mut establisher_frame = 0u64;
        // SAFETY: the context's stack pointers all point into [low, high),
        // which the caller vouches is readable.
        unsafe {
            RtlVirtualUnwind(
                UNW_FLAG_NHANDLER,
                image_base,
                pc,
                entry,
                context,
                &mut handler_data,
                &mut establisher_frame,
                std::ptr::null_mut(),
            );
        }
    }
    count
}

/// An x64 unwind register number's value in `context`.
fn register(context: &CONTEXT, number: u8) -> u64 {
    match number {
        0 => context.Rax,
        1 => context.Rcx,
        2 => context.Rdx,
        3 => context.Rbx,
        4 => context.Rsp,
        5 => context.Rbp,
        6 => context.Rsi,
        7 => context.Rdi,
        8 => context.R8,
        9 => context.R9,
        10 => context.R10,
        11 => context.R11,
        12 => context.R12,
        13 => context.R13,
        14 => context.R14,
        _ => context.R15,
    }
}

#[repr(C)]
struct Symbol {
    info: SYMBOL_INFOW,
    name: [u16; NAME_CHARS],
}

/// One line per frame: `module+0xoffset` (what a symbol server resolves
/// offline), then the symbol when this machine has it (a PDB beside the
/// module, or a DLL's exports).
pub(crate) fn describe(frames: &[u64], out: &mut String) {
    let modules: Vec<Option<(String, u64)>> = frames.iter().map(|&pc| module(pc)).collect();
    // Rust links PDBs by file name alone (`/PDBALTPATH:%_PDB%`), so dbghelp
    // looks for them beside each module.
    let mut search = String::new();
    for (path, _) in modules.iter().flatten() {
        if let Some((directory, _)) = path.rsplit_once(['\\', '/'])
            && !search.split(';').any(|known| known == directory)
        {
            if !search.is_empty() {
                search.push(';');
            }
            search.push_str(directory);
        }
    }
    let search: Vec<u16> = search.encode_utf16().chain([0]).collect();
    // SAFETY: dbghelp calls on this process. If another component (the
    // standard library's backtraces) already initialised dbghelp, ours fails
    // and theirs is used and left alone.
    let process = unsafe { GetCurrentProcess() };
    let initialised = unsafe {
        SymSetOptions(
            SYMOPT_UNDNAME
                | SYMOPT_DEFERRED_LOADS
                | SYMOPT_FAIL_CRITICAL_ERRORS
                | SYMOPT_NO_PROMPTS,
        );
        SymInitializeW(process, search.as_ptr(), 1) != 0
    };
    for (index, (&pc, module)) in frames.iter().zip(&modules).enumerate() {
        let _ = write!(out, "  {index:2} ");
        match module {
            Some((path, base)) => {
                let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
                let _ = write!(out, "{name}+0x{:x}", pc - base);
            }
            None => {
                let _ = write!(out, "0x{pc:x}");
            }
        }
        if let Some((name, displacement)) = symbol(process, pc) {
            let _ = write!(out, " {name}+0x{displacement:x}");
        }
        out.push('\n');
    }
    if initialised {
        // SAFETY: we initialised this session.
        unsafe { SymCleanup(process) };
    }
}

/// The loaded module holding `pc`: its path and base address.
fn module(pc: u64) -> Option<(String, u64)> {
    let mut module: HMODULE = std::ptr::null_mut();
    // SAFETY: FROM_ADDRESS takes the address in place of a name; the
    // refcount is left alone so nothing needs releasing.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            pc as *const u16,
            &mut module,
        )
    };
    if found == 0 {
        return None;
    }
    let mut path = [0u16; 520];
    // SAFETY: the buffer length is passed.
    let len = unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) } as usize;
    let path = String::from_utf16_lossy(&path[..len.min(path.len())]);
    Some((path, module as u64))
}

fn symbol(process: HANDLE, pc: u64) -> Option<(String, u64)> {
    // SAFETY: zeroed is a valid empty SYMBOL_INFOW plus name buffer.
    let mut symbol: Box<Symbol> = Box::new(unsafe { std::mem::zeroed() });
    symbol.info.SizeOfStruct = std::mem::size_of::<SYMBOL_INFOW>() as u32;
    symbol.info.MaxNameLen = NAME_CHARS as u32 + 1;
    let mut displacement = 0u64;
    // SAFETY: the struct carries its own size and name capacity.
    let found = unsafe { SymFromAddrW(process, pc, &mut displacement, &mut symbol.info) } != 0;
    if !found {
        return None;
    }
    let len = (symbol.info.NameLen as usize).min(NAME_CHARS + 1);
    // SAFETY: `Name` is followed in memory by `name`, which together hold
    // `MaxNameLen` characters.
    let name = unsafe { std::slice::from_raw_parts(symbol.info.Name.as_ptr(), len) };
    Some((String::from_utf16_lossy(name), displacement))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[inline(never)]
    fn stall_test_spin(stop: &AtomicBool) {
        while !stop.load(Ordering::Relaxed) {
            std::hint::spin_loop();
        }
    }

    struct StopOnDrop(Arc<AtomicBool>);
    impl Drop for StopOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn samples_name_the_function_a_thread_spins_in() {
        let stop = Arc::new(AtomicBool::new(false));
        let _stop_guard = StopOnDrop(Arc::clone(&stop));
        let (sender, receiver) = std::sync::mpsc::channel();
        let spinning = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                sender.send(Thread::current()).unwrap();
                stall_test_spin(&stop);
            })
        };
        let thread = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("spinner thread started within 5s");
        std::thread::sleep(Duration::from_millis(50));
        let before = thread.cpu_time().unwrap();
        let mut frames = Vec::new();
        let sampled = thread.sample(&mut frames);
        std::thread::sleep(Duration::from_millis(100));
        let used = thread.cpu_time().unwrap() - before;
        stop.store(true, Ordering::Relaxed);
        spinning.join().unwrap();

        assert!(sampled);
        let mut text = String::new();
        describe(&frames, &mut text);
        assert!(text.contains("stall_test_spin"), "{text}");
        assert!(used >= Duration::from_millis(50), "spinning used {used:?}");
    }
}
