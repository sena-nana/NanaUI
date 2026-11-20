//! Deterministic `tar` (ustar), `ar` and gzip writers for `.deb` files.
//! Entries are written in the order given, owned by root, with one fixed
//! modification time, so the same tree always produces the same bytes.

use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;

const BLOCK: usize = 512;

pub(crate) struct Tar {
    out: Vec<u8>,
    mtime: u64,
}

impl Tar {
    pub(crate) fn new(mtime: u64) -> Self {
        Self {
            out: Vec::new(),
            mtime,
        }
    }

    pub(crate) fn dir(&mut self, path: &str, mode: u32) -> Result<(), String> {
        let path = format!("{}/", path.trim_end_matches('/'));
        self.header(&path, mode, 0, b'5', "")
    }

    pub(crate) fn file(&mut self, path: &str, mode: u32, bytes: &[u8]) -> Result<(), String> {
        self.header(path, mode, bytes.len() as u64, b'0', "")?;
        self.out.extend_from_slice(bytes);
        let pad = (BLOCK - bytes.len() % BLOCK) % BLOCK;
        self.out.extend(std::iter::repeat_n(0u8, pad));
        Ok(())
    }

    pub(crate) fn symlink(&mut self, path: &str, target: &str) -> Result<(), String> {
        if target.len() > 100 {
            return Err(format!("symlink target of {path} is longer than 100 bytes"));
        }
        self.header(path, 0o777, 0, b'2', target)
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.out.extend(std::iter::repeat_n(0u8, BLOCK * 2));
        self.out
    }

    fn header(
        &mut self,
        path: &str,
        mode: u32,
        size: u64,
        kind: u8,
        link: &str,
    ) -> Result<(), String> {
        let (prefix, name) = split_name(path)?;
        if size >= 8 << 30 {
            return Err(format!("{path} is 8 GiB or larger; ustar cannot hold it"));
        }
        let mut header = [0u8; BLOCK];
        header[..name.len()].copy_from_slice(name.as_bytes());
        octal(&mut header[100..108], u64::from(mode));
        octal(&mut header[108..116], 0);
        octal(&mut header[116..124], 0);
        octal(&mut header[124..136], size);
        octal(&mut header[136..148], self.mtime);
        header[148..156].fill(b' ');
        header[156] = kind;
        header[157..157 + link.len()].copy_from_slice(link.as_bytes());
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[265..269].copy_from_slice(b"root");
        header[297..301].copy_from_slice(b"root");
        header[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        let text = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(text.as_bytes());
        self.out.extend_from_slice(&header);
        Ok(())
    }
}

/// ustar keeps a name of up to 100 bytes plus a prefix of up to 155,
/// split at a `/`.
fn split_name(path: &str) -> Result<(&str, &str), String> {
    if path.len() <= 100 {
        return Ok(("", path));
    }
    let bytes = path.as_bytes();
    for at in (0..bytes.len()).rev() {
        if bytes[at] == b'/' && at <= 155 && bytes.len() - at - 1 <= 100 && at > 0 {
            return Ok((&path[..at], &path[at + 1..]));
        }
    }
    Err(format!("{path} is too long for a ustar entry"))
}

fn octal(field: &mut [u8], value: u64) {
    let width = field.len() - 1;
    let text = format!("{value:0width$o}");
    field[..width].copy_from_slice(&text.as_bytes()[text.len() - width..]);
    field[width] = 0;
}

/// gzip with a zero header timestamp.
pub(crate) fn gzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(bytes)
        .and_then(|()| encoder.finish())
        .map_err(|error| format!("gzip: {error}"))
}

/// A Unix `ar` archive, as `.deb` files are.
pub(crate) fn ar(members: &[(&str, &[u8])], mtime: u64) -> Vec<u8> {
    let mut out = b"!<arch>\n".to_vec();
    for (name, bytes) in members {
        let header = format!(
            "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            name,
            mtime,
            0,
            0,
            "100644",
            bytes.len()
        );
        debug_assert_eq!(header.len(), 60);
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(bytes);
        if bytes.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    out
}

/// Members of an `ar` archive (tests and self-checks).
pub fn read_ar(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let rest = bytes
        .strip_prefix(b"!<arch>\n")
        .ok_or("not an ar archive")?;
    let mut at = 0;
    let mut members = Vec::new();
    while at < rest.len() {
        let header = rest.get(at..at + 60).ok_or("truncated ar header")?;
        let text = std::str::from_utf8(header).map_err(|_| "ar header is not ASCII")?;
        let name = text[..16].trim_end().trim_end_matches('/').to_owned();
        let size: usize = text[48..58]
            .trim()
            .parse()
            .map_err(|_| "bad ar member size")?;
        let data = rest
            .get(at + 60..at + 60 + size)
            .ok_or("truncated ar member")?;
        members.push((name, data.to_vec()));
        at += 60 + size + size % 2;
    }
    Ok(members)
}

/// One entry of a tar archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarEntry {
    pub path: String,
    /// ustar type flag: `b'0'` file, `b'2'` symlink, `b'5'` directory.
    pub kind: u8,
    pub link: String,
    pub data: Vec<u8>,
}

/// Every tar entry (tests and self-checks).
pub fn list_tar(bytes: &[u8]) -> Result<Vec<TarEntry>, String> {
    let mut entries = Vec::new();
    let mut at = 0;
    while at + BLOCK <= bytes.len() {
        let header = &bytes[at..at + BLOCK];
        if header.iter().all(|&b| b == 0) {
            break;
        }
        let field = |range: std::ops::Range<usize>| {
            let raw = &header[range];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            String::from_utf8_lossy(&raw[..end]).into_owned()
        };
        let stored: u32 =
            u32::from_str_radix(field(148..156).trim(), 8).map_err(|_| "bad tar checksum field")?;
        let mut copy = header.to_vec();
        copy[148..156].fill(b' ');
        if copy.iter().map(|&b| u32::from(b)).sum::<u32>() != stored {
            return Err("tar checksum mismatch".into());
        }
        let name = field(0..100);
        let prefix = field(345..500);
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let size =
            u64::from_str_radix(field(124..136).trim(), 8).map_err(|_| "bad tar size")? as usize;
        let data = bytes
            .get(at + BLOCK..at + BLOCK + size)
            .ok_or("truncated tar entry")?;
        entries.push(TarEntry {
            path,
            kind: header[156],
            link: field(157..257),
            data: data.to_vec(),
        });
        at += BLOCK + size.div_ceil(BLOCK) * BLOCK;
    }
    Ok(entries)
}

pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .map_err(|error| format!("gunzip: {error}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tar_round_trips_and_is_deterministic() {
        let build = || {
            let mut tar = Tar::new(1_700_000_000);
            tar.dir("./usr", 0o755).unwrap();
            tar.file("./usr/bin-file", 0o755, b"hello").unwrap();
            let long = format!("./usr/share/{}/file.bin", "d".repeat(120));
            tar.file(&long, 0o644, &[1u8; 700]).unwrap();
            tar.symlink("./usr/bin/app", "../lib/app/app").unwrap();
            tar.finish()
        };
        let bytes = build();
        assert_eq!(bytes, build());
        assert_eq!(bytes.len() % BLOCK, 0);
        let entries = list_tar(&bytes).unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].path, "./usr/");
        assert_eq!(entries[0].kind, b'5');
        assert_eq!(entries[1].data, b"hello");
        assert!(entries[2].path.ends_with("/file.bin") && entries[2].path.len() > 100);
        assert_eq!(entries[2].data.len(), 700);
        assert_eq!(entries[3].kind, b'2');
        assert_eq!(entries[3].link, "../lib/app/app");
        let gz = gzip(&bytes).unwrap();
        assert_eq!(gunzip(&gz).unwrap(), bytes);
        assert!(Tar::new(0).file(&"x".repeat(300), 0o644, b"").is_err());
    }

    #[test]
    fn ar_round_trips_with_odd_members() {
        let bytes = ar(&[("debian-binary", b"2.0\n"), ("odd", b"abc")], 0);
        assert_eq!(
            read_ar(&bytes).unwrap(),
            vec![
                ("debian-binary".to_owned(), b"2.0\n".to_vec()),
                ("odd".to_owned(), b"abc".to_vec())
            ]
        );
    }
}
