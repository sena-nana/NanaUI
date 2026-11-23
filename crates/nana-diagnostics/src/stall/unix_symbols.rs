//! Frame lines for Apple and Linux stack samples: `module+0xoffset` (what
//! `atos`, `addr2line` or `llvm-symbolizer` resolve offline against the same
//! build), then the symbol when this machine has it. `dladdr` names local
//! symbols on Apple; on Linux it sees only the dynamic table, so the module's
//! own `.symtab` is read from disk when the file still has one (a stripped
//! release binary does not, and keeps the offsets).
//!
//! Runs on the watchdog thread after the watched thread has been let go:
//! it may allocate and read files.

use std::ffi::{CStr, c_void};
use std::fmt::Write as _;

use super::demangle::demangle;

/// Where `pc` lives: the module's path and load address, and the nearest
/// symbol `dladdr` knows with its address.
struct Place {
    path: String,
    base: u64,
    symbol: Option<(String, u64)>,
}

fn place(pc: u64) -> Option<Place> {
    // SAFETY: zeroed is a valid empty Dl_info.
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: dladdr only reads the loader's tables for the address.
    if unsafe { libc::dladdr(pc as *const c_void, &mut info) } == 0 {
        return None;
    }
    let text = |pointer: *const libc::c_char| {
        // SAFETY: dladdr returns NUL-terminated strings owned by the loader.
        (!pointer.is_null()).then(|| {
            unsafe { CStr::from_ptr(pointer) }
                .to_string_lossy()
                .into_owned()
        })
    };
    let path = text(info.dli_fname).unwrap_or_default();
    let symbol = text(info.dli_sname)
        .filter(|name| !name.is_empty() && !info.dli_saddr.is_null())
        .map(|name| (name, info.dli_saddr as u64));
    Some(Place {
        path,
        base: info.dli_fbase as u64,
        symbol,
    })
}

/// One line per frame, innermost first.
pub(super) fn describe(frames: &[u64], out: &mut String) {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let mut tables = elf::Tables::default();
    for (index, &pc) in frames.iter().enumerate() {
        let _ = write!(out, "  {index:2} ");
        // A return address may sit just past its call's function.
        let lookup = if index == 0 { pc } else { pc.saturating_sub(1) };
        let Some(place) = place(lookup) else {
            let _ = writeln!(out, "0x{pc:x}");
            continue;
        };
        let name = place.path.rsplit('/').next().unwrap_or(&place.path);
        let name = if name.is_empty() { "?" } else { name };
        let _ = write!(out, "{name}+0x{:x}", pc.wrapping_sub(place.base));
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let symbol = tables
            .lookup(&place.path, place.base, lookup)
            .or(place.symbol);
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let symbol = place.symbol;
        if let Some((symbol, address)) = symbol {
            let symbol = demangle(&symbol).unwrap_or(symbol);
            let _ = write!(out, " {symbol}+0x{:x}", pc.wrapping_sub(address));
        }
        out.push('\n');
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod elf {
    //! The function symbols of a little-endian ELF64 module's `.symtab`.

    use std::fs::File;
    use std::os::unix::fs::FileExt as _;

    /// Above this many symbols a table is not read: a report stays cheap.
    const MAX_SYMBOLS: u64 = 4 << 20;

    struct Table {
        /// Load bias: runtime address minus file address.
        bias: u64,
        /// (address, size, name offset), sorted by address.
        functions: Vec<(u64, u64, u32)>,
        names: Vec<u8>,
    }

    /// Tables read during one report, by module path.
    #[derive(Default)]
    pub(super) struct Tables {
        read: Vec<(String, Option<Table>)>,
    }

    impl Tables {
        pub(super) fn lookup(&mut self, path: &str, base: u64, pc: u64) -> Option<(String, u64)> {
            let index = match self.read.iter().position(|(known, _)| known == path) {
                Some(index) => index,
                None => {
                    self.read.push((path.to_owned(), Table::read(path, base)));
                    self.read.len() - 1
                }
            };
            let table = self.read[index].1.as_ref()?;
            let address = pc.checked_sub(table.bias)?;
            let at = table
                .functions
                .partition_point(|&(start, _, _)| start <= address);
            let &(start, size, name) = table.functions.get(at.checked_sub(1)?)?;
            if size != 0 && address >= start.saturating_add(size) {
                return None;
            }
            let names = table.names.get(name as usize..)?;
            let end = names.iter().position(|&byte| byte == 0)?;
            let text = std::str::from_utf8(&names[..end]).ok()?;
            Some((text.to_owned(), start + table.bias))
        }
    }

    fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
        Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
    }
    fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    }
    fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
        Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
    }

    fn read_at(file: &File, offset: u64, len: u64) -> Option<Vec<u8>> {
        let mut bytes = vec![0; usize::try_from(len).ok()?];
        file.read_exact_at(&mut bytes, offset).ok()?;
        Some(bytes)
    }

    impl Table {
        fn read(path: &str, base: u64) -> Option<Self> {
            // The main program's entry may name it by argv[0] or not at all.
            let file = File::open(path)
                .ok()
                .filter(|_| !path.is_empty())
                .or_else(|| File::open(std::env::current_exe().ok()?).ok())?;
            let header = read_at(&file, 0, 64)?;
            // ELF64, little-endian.
            if header.get(..4)? != b"\x7fELF" || header[4] != 2 || header[5] != 1 {
                return None;
            }
            let phoff = u64_at(&header, 0x20)?;
            let shoff = u64_at(&header, 0x28)?;
            let phentsize = u64::from(u16_at(&header, 0x36)?);
            let phnum = u64::from(u16_at(&header, 0x38)?);
            let shentsize = u64::from(u16_at(&header, 0x3a)?);
            let shnum = u64::from(u16_at(&header, 0x3c)?);
            if phentsize < 56 || shentsize < 64 {
                return None;
            }

            // The lowest loaded segment sits at the module's load address.
            let programs = read_at(&file, phoff, phentsize * phnum)?;
            let mut lowest = u64::MAX;
            for index in 0..phnum as usize {
                let entry = &programs[index * phentsize as usize..];
                if u32_at(entry, 0)? == 1 {
                    let vaddr = u64_at(entry, 16)?;
                    let align = u64_at(entry, 48)?.max(1);
                    lowest = lowest.min(vaddr & !(align - 1));
                }
            }
            if lowest == u64::MAX {
                return None;
            }
            let bias = base.checked_sub(lowest)?;

            let sections = read_at(&file, shoff, shentsize * shnum)?;
            let section = |index: u64| sections.get((index * shentsize) as usize..);
            // SHT_SYMTAB; its sh_link names the string table.
            let symtab = (0..shnum).find_map(|index| {
                let entry = section(index)?;
                (u32_at(entry, 4)? == 2).then_some(entry)
            })?;
            let (offset, size, link, entsize) = (
                u64_at(symtab, 24)?,
                u64_at(symtab, 32)?,
                u64::from(u32_at(symtab, 40)?),
                u64_at(symtab, 56)?,
            );
            if entsize < 24 || size / entsize > MAX_SYMBOLS {
                return None;
            }
            let strtab = section(link)?;
            let names = read_at(&file, u64_at(strtab, 24)?, u64_at(strtab, 32)?)?;
            let symbols = read_at(&file, offset, size)?;
            let mut functions = Vec::new();
            for symbol in symbols.chunks_exact(entsize as usize) {
                // STT_FUNC, defined in some section.
                if symbol[4] & 0xf == 2 && u16_at(symbol, 6)? != 0 {
                    let address = u64_at(symbol, 8)?;
                    if address != 0 {
                        functions.push((address, u64_at(symbol, 16)?, u32_at(symbol, 0)?));
                    }
                }
            }
            functions
                .sort_unstable_by_key(|&(address, size, _)| (address, std::cmp::Reverse(size)));
            functions.dedup_by_key(|&mut (address, _, _)| address);
            Some(Self {
                bias,
                functions,
                names,
            })
        }
    }
}
