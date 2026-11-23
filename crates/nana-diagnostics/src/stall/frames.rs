//! Frame-pointer walk over a copy of a thread's stack, shared by the
//! backends whose ABI keeps a frame record chain: Apple (both architectures),
//! Windows ARM64 (the ABI requires the x29 chain for its own stack walkers)
//! and Linux (aarch64 keeps it in non-leaf frames; x86_64 only when built with
//! `-C force-frame-pointers=yes`, otherwise the walk stops at the first link
//! that does not hold up and the sample keeps the frames before it).
//!
//! A frame record is two words at the frame pointer: the caller's frame
//! pointer, then the return address. The walk reads only words inside the
//! copy, needs every link to point further up the stack than the last, and
//! never dereferences the original stack.

/// Most frames kept from one sample.
pub(super) const MAX_FRAMES: usize = 64;

/// Words copied from a thread's stack, from its stack pointer at the time of
/// the copy (`low`) upwards.
pub(super) struct StackCopy<'a> {
    pub(super) words: &'a [u64],
    pub(super) low: u64,
}

impl StackCopy<'_> {
    /// The word the original stack held at `address`, if the copy has it.
    fn read(&self, address: u64) -> Option<u64> {
        if address < self.low || !address.is_multiple_of(8) {
            return None;
        }
        let index = usize::try_from((address - self.low) / 8).ok()?;
        self.words.get(index).copied()
    }
}

/// A code address without pointer-authentication bits. Apple arm64e system
/// libraries sign the return addresses they save; user space addresses fit
/// in 48 bits on every aarch64 system this runs on.
pub(super) fn strip(address: u64) -> u64 {
    if cfg!(target_arch = "aarch64") {
        address & 0x0000_ffff_ffff_ffff
    } else {
        address
    }
}

/// The program counter, then the return address of each frame record from
/// `fp` outwards, into `frames`; returns how many were found.
pub(super) fn walk(pc: u64, fp: u64, stack: &StackCopy<'_>, frames: &mut [u64]) -> usize {
    let pc = strip(pc);
    if pc == 0 || frames.is_empty() {
        return 0;
    }
    frames[0] = pc;
    let mut count = 1;
    let mut fp = fp;
    while count < frames.len() {
        let (Some(caller_fp), Some(ret)) = (stack.read(fp), stack.read(fp.wrapping_add(8))) else {
            break;
        };
        let ret = strip(ret);
        if ret == 0 {
            break;
        }
        frames[count] = ret;
        count += 1;
        // The outermost record links to 0; anything not further up the
        // stack is not a frame record.
        if caller_fp <= fp {
            break;
        }
        fp = caller_fp;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_follows_records_inside_the_copy_and_stops_at_a_bad_link() {
        let low = 0x7000_0000u64;
        // Records at low+16 -> low+48 -> low+80 (links to 0).
        let mut words = vec![0u64; 16];
        words[2] = low + 48;
        words[3] = 0x1111;
        words[6] = low + 80;
        words[7] = 0x2222;
        words[10] = 0;
        words[11] = 0x3333;
        let stack = StackCopy { words: &words, low };
        let mut frames = [0u64; MAX_FRAMES];
        let count = walk(0x1000, low + 16, &stack, &mut frames);
        assert_eq!(&frames[..count], &[0x1000, 0x1111, 0x2222, 0x3333]);

        // A link pointing down the stack ends the walk after its frame.
        words[6] = low;
        let stack = StackCopy { words: &words, low };
        let count = walk(0x1000, low + 16, &stack, &mut frames);
        assert_eq!(&frames[..count], &[0x1000, 0x1111, 0x2222]);

        // A frame pointer outside the copy yields only the program counter.
        let count = walk(0x1000, low - 64, &stack, &mut frames);
        assert_eq!(&frames[..count], &[0x1000]);
        let count = walk(0x1000, low + 8 * 16, &stack, &mut frames);
        assert_eq!(&frames[..count], &[0x1000]);
    }
}
