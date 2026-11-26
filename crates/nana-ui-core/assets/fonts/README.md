# Bundled UI font

`NotoSansSC-Variable.ttf` is the one face `nana_ui_core::fonts` embeds. Regenerate
it with `python3 scripts/build-ui-font.py`; verify with `--check`.

- Upstream: Noto Sans SC v2.004, google/fonts `1ac2012c34919f5fa2675aacf723fa98edb30b5f`,
  `https://raw.githubusercontent.com/google/fonts/1ac2012c34919f5fa2675aacf723fa98edb30b5f/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf`
- Upstream SHA-256: `a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da`
- Axis: `wght` 400..700, default 400; named instances Regular (400), Medium (500), SemiBold (600), Bold (700)
- Glyphs: 8621; codepoints: 8329
- Layout features kept: `ccmp`, `liga`, `locl`, `vert`, `vrt2`, `halt`, `kern`, `mark`, `palt`, `vhal`, `vkrn`, `vpal`
- SHA-256: `57e2f36009a60573d078d1ff8916ff49ef98010964b815a29974ce3ca07852d8`

Character set: `scripts/ui-font/base-codepoints.txt` (the coverage of the
former four static weights), JIS X 0208 level 1, and
`scripts/ui-font/ui-text-codepoints.txt` (interface text of NanaUI and its
consumers).

Added over the former static coverage: 378 ideographs and 5 other characters:

```text
←→⌘⚠〜
```

Requested but absent from the upstream face, so not covered (a fallback
face draws them):

```text
U+2325
```

## Licence

SIL Open Font License 1.1; `OFL.txt` is the upstream licence file.
