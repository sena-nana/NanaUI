#!/usr/bin/env python3
"""Break a built artifact down by section, and count the fonts inside it.

Works on both output formats this repo ships without needing platform tools:
ELF (the Android `.so`) is parsed here, and Mach-O is read via `size -m`.
`llvm-readelf` is not part of a plain macOS install, and the Android `.so` is
the artifact that most needs auditing from a Mac.

    ./scripts/report-artifact-size.py target-android/.../libnana_android_host.so
    ./scripts/report-artifact-size.py target/dist/component-gallery
"""

from __future__ import annotations

import glob
import os
import struct
import subprocess
import sys

FONT_GLOB = "crates/nana-ui/assets/fonts/*.ttf"
# Rust only emits this when debug-assertions are on.
DEBUG_MARKER = b"attempt to add with overflow"


def mib(n: int) -> str:
    return f"{n / 1024 / 1024:9.2f} MiB"


def elf_sections(path: str) -> list[tuple[str, int]]:
    with open(path, "rb") as handle:
        header = handle.read(64)
        if header[:4] != b"\x7fELF":
            raise ValueError("not an ELF file")
        if header[4] != 2:
            raise ValueError("only 64-bit ELF is supported")
        shoff, = struct.unpack_from("<Q", header, 0x28)
        shentsize, shnum, shstrndx = struct.unpack_from("<HHH", header, 0x3A)
        handle.seek(shoff)
        raw = handle.read(shentsize * shnum)
        entries = [
            struct.unpack_from("<IIQQQQIIQQ", raw, i * shentsize) for i in range(shnum)
        ]
        _, _, _, _, str_off, str_size, *_ = entries[shstrndx]
        handle.seek(str_off)
        strtab = handle.read(str_size)

    def name_at(offset: int) -> str:
        return strtab[offset : strtab.index(b"\0", offset)].decode()

    # SHT_NOBITS (8) occupies no file bytes.
    return [
        (name_at(e[0]), e[5]) for e in entries if e[1] != 8 and e[5] > 0
    ]


def report_elf(path: str) -> None:
    sections = sorted(elf_sections(path), key=lambda s: -s[1])
    debug = sum(size for name, size in sections if name.startswith(".debug"))
    symbols = sum(size for name, size in sections if name in (".symtab", ".strtab"))
    loadable = sum(
        size
        for name, size in sections
        if not name.startswith(".debug") and name not in (".symtab", ".strtab")
    )
    for name, size in sections[:12]:
        print(f"  {mib(size)}  {name}")
    print()
    print(f"  {mib(debug)}  DWARF (.debug_*)          <- strippable")
    print(f"  {mib(symbols)}  symbol tables             <- strippable")
    print(f"  {mib(loadable)}  everything else")


def report_macho(path: str) -> None:
    out = subprocess.run(["size", "-m", path], capture_output=True, text=True).stdout
    for line in out.splitlines():
        stripped = line.strip()
        # __PAGEZERO is address space, not file bytes.
        if stripped.startswith("Segment __PAGEZERO") or stripped.startswith("total "):
            continue
        if stripped.startswith(("Segment", "Section")):
            print("  " + stripped)


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    path = argv[1]
    if not os.path.isfile(path):
        print(f"report-artifact-size: no such file: {path}", file=sys.stderr)
        return 1

    print(f"{path}")
    print(f"  {mib(os.path.getsize(path))}  total on disk")
    print()

    with open(path, "rb") as handle:
        magic = handle.read(4)
    if magic == b"\x7fELF":
        report_elf(path)
    else:
        report_macho(path)

    blob = open(path, "rb").read()
    print()
    fonts = sorted(glob.glob(FONT_GLOB))
    if fonts:
        total = 0
        for ttf in fonts:
            # A TTF's first 256 bytes cover the sfnt header and table directory,
            # which is unique per face and never appears by chance.
            copies = blob.count(open(ttf, "rb").read(256))
            total += copies * os.path.getsize(ttf)
            print(f"  {os.path.basename(ttf):28s} copies: {copies}")
        print(f"  {mib(total)}  embedded font bytes")

    if DEBUG_MARKER in blob:
        print()
        print("  WARNING: debug assertions are on — this is a dev-profile build.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
