//! Readable names for the Rust symbols a stack report shows, without a
//! demangling dependency. Covers the legacy scheme (`_ZN…E`) and the v0
//! scheme (`_R…`) for paths, impls, closures and generic arguments made of
//! ordinary types; anything else (function pointers, `dyn`, const
//! generics) keeps the mangled name, which `rustfilt` reads offline.

use std::fmt::Write as _;

/// `symbol` demangled, or `None` when it is not a Rust symbol this reads.
pub(super) fn demangle(symbol: &str) -> Option<String> {
    // Apple prefixes C symbols with an underscore.
    let symbol = symbol
        .strip_prefix('_')
        .filter(|rest| rest.starts_with(['_', 'R']))
        .unwrap_or(symbol);
    if let Some(rest) = symbol
        .strip_prefix("_R")
        .or_else(|| symbol.strip_prefix('R'))
    {
        let mut parser = V0 {
            text: rest.as_bytes(),
            at: 0,
            out: String::new(),
            depth: 0,
        };
        // An optional encoding version.
        while parser.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            parser.at += 1;
        }
        parser.path(false).ok()?;
        return Some(parser.out);
    }
    legacy(symbol)
}

fn legacy(symbol: &str) -> Option<String> {
    let mut rest = symbol
        .strip_prefix("_ZN")
        .or_else(|| symbol.strip_prefix("ZN"))?;
    let mut out = String::new();
    loop {
        if rest.starts_with('E') {
            return Some(out);
        }
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let len: usize = rest[..digits].parse().ok()?;
        let ident = rest.get(digits..digits + len)?;
        rest = &rest[digits + len..];
        // The trailing `h` + 16 hex digits is a hash, not a name.
        if rest.starts_with('E')
            && ident.len() == 17
            && ident.starts_with('h')
            && ident[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            continue;
        }
        if !out.is_empty() {
            out.push_str("::");
        }
        unescape_legacy(ident, &mut out);
    }
}

fn unescape_legacy(mut ident: &str, out: &mut String) {
    // A leading `_` only guards an escape at the start.
    if ident.starts_with("_$") {
        ident = &ident[1..];
    }
    while !ident.is_empty() {
        if let Some(rest) = ident.strip_prefix("..") {
            out.push_str("::");
            ident = rest;
            continue;
        }
        if ident.starts_with('$')
            && let Some(end) = ident[1..].find('$')
        {
            let code = &ident[1..=end];
            let text = match code {
                "SP" => "@",
                "BP" => "*",
                "RF" => "&",
                "LT" => "<",
                "GT" => ">",
                "LP" => "(",
                "RP" => ")",
                "C" => ",",
                _ => match code
                    .strip_prefix('u')
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .and_then(char::from_u32)
                {
                    Some(ch) => {
                        out.push(ch);
                        ident = &ident[end + 2..];
                        continue;
                    }
                    None => "$",
                },
            };
            out.push_str(text);
            ident = &ident[end + 2..];
            continue;
        }
        let ch = ident.chars().next().unwrap_or('?');
        out.push(ch);
        ident = &ident[ch.len_utf8()..];
    }
}

struct V0<'a> {
    text: &'a [u8],
    at: usize,
    out: String,
    depth: u32,
}

type Parsed = Result<(), ()>;

impl V0<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn next(&mut self) -> Result<u8, ()> {
        let byte = self.peek().ok_or(())?;
        self.at += 1;
        Ok(byte)
    }

    fn eat(&mut self, byte: u8) -> bool {
        let found = self.peek() == Some(byte);
        if found {
            self.at += 1;
        }
        found
    }

    /// A base-62 number. Crate disambiguators are 64-bit hashes whose
    /// encoding can exceed `u64`; their value is never shown, so it saturates.
    fn base62(&mut self) -> Result<u64, ()> {
        if self.eat(b'_') {
            return Ok(0);
        }
        let mut value = 0u128;
        loop {
            let byte = self.next()?;
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'z' => byte - b'a' + 10,
                b'A'..=b'Z' => byte - b'A' + 36,
                b'_' => return Ok(u64::try_from(value + 1).unwrap_or(u64::MAX)),
                _ => return Err(()),
            };
            value = value.saturating_mul(62).saturating_add(u128::from(digit));
        }
    }

    fn disambiguator(&mut self) -> Result<u64, ()> {
        if self.eat(b's') {
            self.base62().map(|value| value.saturating_add(1))
        } else {
            Ok(0)
        }
    }

    /// An identifier's text.
    fn ident(&mut self) -> Result<&str, ()> {
        let _ = self.disambiguator()?;
        // Punycode identifiers are left encoded.
        let _punycode = self.eat(b'u');
        let start = self.at;
        // A decimal number has no leading zeros: `0` stands alone.
        if self.eat(b'0') {
        } else {
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.at += 1;
            }
        }
        let len: usize = std::str::from_utf8(&self.text[start..self.at])
            .ok()
            .and_then(|digits| digits.parse().ok())
            .ok_or(())?;
        self.eat(b'_');
        let end = self
            .at
            .checked_add(len)
            .filter(|&end| end <= self.text.len())
            .ok_or(())?;
        let bytes = &self.text[self.at..end];
        self.at = end;
        std::str::from_utf8(bytes).map_err(|_| ())
    }

    fn enter(&mut self) -> Parsed {
        self.depth += 1;
        if self.depth > 64 { Err(()) } else { Ok(()) }
    }

    /// Follow a back-reference, then come back.
    fn backref(&mut self, in_value: bool, what: fn(&mut Self, bool) -> Parsed) -> Parsed {
        let target = usize::try_from(self.base62()?).map_err(|_| ())?;
        if target >= self.at {
            return Err(());
        }
        let resume = std::mem::replace(&mut self.at, target);
        what(self, in_value)?;
        self.at = resume;
        Ok(())
    }

    /// `in_value`: inside a type, where generic args print without `::`.
    fn path(&mut self, in_value: bool) -> Parsed {
        self.enter()?;
        match self.next()? {
            b'C' => {
                let name = self.ident()?.to_owned();
                self.out.push_str(&name);
            }
            b'N' => {
                let namespace = self.next()?;
                self.path(in_value)?;
                let disambiguator = {
                    let save = self.at;
                    let value = self.disambiguator()?;
                    self.at = save;
                    value
                };
                let name = self.ident()?.to_owned();
                match namespace {
                    b'C' => {
                        let _ = write!(self.out, "::{{closure");
                        if !name.is_empty() {
                            let _ = write!(self.out, ":{name}");
                        }
                        let _ = write!(self.out, "#{disambiguator}}}");
                    }
                    b'S' => {
                        let _ = write!(self.out, "::{{shim:{name}#{disambiguator}}}");
                    }
                    _ if name.is_empty() => {}
                    _ => {
                        self.out.push_str("::");
                        self.out.push_str(&name);
                    }
                }
            }
            b'M' => {
                self.disambiguator()?;
                self.skip_path()?;
                self.out.push('<');
                self.ty()?;
                self.out.push('>');
            }
            b'X' => {
                self.disambiguator()?;
                self.skip_path()?;
                self.out.push('<');
                self.ty()?;
                self.out.push_str(" as ");
                self.path(true)?;
                self.out.push('>');
            }
            b'Y' => {
                self.out.push('<');
                self.ty()?;
                self.out.push_str(" as ");
                self.path(true)?;
                self.out.push('>');
            }
            b'I' => {
                self.path(in_value)?;
                self.out.push_str(if in_value { "<" } else { "::<" });
                let mut first = true;
                while !self.eat(b'E') {
                    if self.peek() == Some(b'L') {
                        self.at += 1;
                        self.base62()?;
                        continue;
                    }
                    if !first {
                        self.out.push_str(", ");
                    }
                    first = false;
                    self.ty()?;
                }
                self.out.push('>');
            }
            b'B' => {
                self.backref(in_value, Self::path)?;
            }
            _ => return Err(()),
        }
        self.depth -= 1;
        Ok(())
    }

    /// An impl's own path only places it; the type and trait name it.
    fn skip_path(&mut self) -> Parsed {
        let mark = self.out.len();
        self.path(false)?;
        self.out.truncate(mark);
        Ok(())
    }

    fn ty(&mut self) -> Parsed {
        self.enter()?;
        let byte = self.peek().ok_or(())?;
        let basic = match byte {
            b'a' => Some("i8"),
            b'b' => Some("bool"),
            b'c' => Some("char"),
            b'd' => Some("f64"),
            b'e' => Some("str"),
            b'f' => Some("f32"),
            b'h' => Some("u8"),
            b'i' => Some("isize"),
            b'j' => Some("usize"),
            b'l' => Some("i32"),
            b'm' => Some("u32"),
            b'n' => Some("i128"),
            b'o' => Some("u128"),
            b's' => Some("i16"),
            b't' => Some("u16"),
            b'u' => Some("()"),
            b'v' => Some("..."),
            b'x' => Some("i64"),
            b'y' => Some("u64"),
            b'z' => Some("!"),
            b'p' => Some("_"),
            _ => None,
        };
        if let Some(name) = basic {
            self.at += 1;
            self.out.push_str(name);
            self.depth -= 1;
            return Ok(());
        }
        match byte {
            b'C' | b'M' | b'X' | b'Y' | b'N' | b'I' => self.path(true)?,
            b'B' => {
                self.at += 1;
                self.backref(true, |parser, _| parser.ty())?;
            }
            b'R' | b'Q' => {
                self.at += 1;
                if self.eat(b'L') {
                    self.base62()?;
                }
                self.out.push_str(if byte == b'R' { "&" } else { "&mut " });
                self.ty()?;
            }
            b'P' | b'O' => {
                self.at += 1;
                self.out
                    .push_str(if byte == b'P' { "*const " } else { "*mut " });
                self.ty()?;
            }
            b'S' => {
                self.at += 1;
                self.out.push('[');
                self.ty()?;
                self.out.push(']');
            }
            b'T' => {
                self.at += 1;
                self.out.push('(');
                let mut count = 0;
                while !self.eat(b'E') {
                    if count > 0 {
                        self.out.push_str(", ");
                    }
                    self.ty()?;
                    count += 1;
                }
                if count == 1 {
                    self.out.push(',');
                }
                self.out.push(')');
            }
            // Arrays, fn pointers, dyn and consts: not read here.
            _ => return Err(()),
        }
        self.depth -= 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::demangle;

    #[test]
    fn reads_v0_and_legacy_paths() {
        assert_eq!(
            demangle("_RNvCsi1O3nTjtKDH_1m15stall_test_spin").as_deref(),
            Some("m::stall_test_spin")
        );
        // Apple's extra underscore.
        assert_eq!(
            demangle("__RNvCsi1O3nTjtKDH_1m15stall_test_spin").as_deref(),
            Some("m::stall_test_spin")
        );
        assert_eq!(
            demangle("_RNvNtCs1234_7mycrate3foo3bar").as_deref(),
            Some("mycrate::foo::bar")
        );
        // A closure inside a function.
        assert_eq!(
            demangle("_RNCNvCs1234_7mycrate4main0B3_").as_deref(),
            Some("mycrate::main::{closure#0}")
        );
        // `<mycrate::Foo as mycrate::Bar>::run`.
        assert_eq!(
            demangle("_RNvXCs1234_7mycrateNtB2_3FooNtB2_3Bar3run").as_deref(),
            Some("<mycrate::Foo as mycrate::Bar>::run")
        );
        // Generic arguments.
        assert_eq!(
            demangle("_RINvCs1234_7mycrate4swapRhEB2_").as_deref(),
            Some("mycrate::swap::<&u8>")
        );
        assert_eq!(
            demangle("_ZN4core3ptr13drop_in_place17h0123456789abcdefE").as_deref(),
            Some("core::ptr::drop_in_place")
        );
        assert_eq!(
            demangle("_ZN58_$LT$alloc..string..String$u20$as$u20$core..fmt..Debug$GT$3fmt17h0123456789abcdefE")
                .as_deref(),
            Some("<alloc::string::String as core::fmt::Debug>::fmt")
        );
        // Real crate hashes overflow u64 in base 62; nested closures in a
        // generic instantiation.
        assert_eq!(
            demangle(
                "_RNCNCINvNtNtCs1SSuMAdGM58_3std6thread9lifecycle15spawn_uncheckedNCNvNtNtNtCseZsg8QLyqo2_16nana_diagnostics5stall5apple5testss_43samples_name_the_function_a_thread_spins_in0uEs_00B1a_"
            )
            .as_deref(),
            Some(
                "std::thread::lifecycle::spawn_unchecked::<nana_diagnostics::stall::apple::tests::samples_name_the_function_a_thread_spins_in::{closure#0}, ()>::{closure#1}::{closure#0}"
            )
        );
        assert_eq!(demangle("pthread_cond_wait"), None);
        assert_eq!(demangle("_RNvC"), None);
    }
}
