# nana-text corpus font fixtures

Regenerate with `python3 scripts/build-text-corpus-fonts.py`; verify with
`--check`. These files exist so the parity corpus never depends on a font a
developer happens to have installed.

| File | Upstream | Contents |
| --- | --- | --- |
| `noto-sans-arabic.ttf` | [https://raw.githubusercontent.com/google/fonts/1ac2012c34919f5fa2675aacf723fa98edb30b5f/ofl/notosansarabic/NotoSansArabic%5Bwdth,wght%5D.ttf](https://raw.githubusercontent.com/google/fonts/1ac2012c34919f5fa2675aacf723fa98edb30b5f/ofl/notosansarabic/NotoSansArabic%5Bwdth,wght%5D.ttf) | Arabic joining and RTL shaping. U+0020 U+0627 U+0628 U+0629 U+0631 U+0639 U+0644 U+064A |
| `noto-sans-kr.ttf` | [https://raw.githubusercontent.com/google/fonts/1ac2012c34919f5fa2675aacf723fa98edb30b5f/ofl/notosanskr/NotoSansKR%5Bwght%5D.ttf](https://raw.githubusercontent.com/google/fonts/1ac2012c34919f5fa2675aacf723fa98edb30b5f/ofl/notosanskr/NotoSansKR%5Bwght%5D.ttf) | Hangul syllables. U+0020 U+D55C U+AE00 U+C548 U+B155 U+D558 U+C138 U+C694 |
| `noto-emoji.ttf` | [https://raw.githubusercontent.com/google/fonts/1ac2012c34919f5fa2675aacf723fa98edb30b5f/ofl/notoemoji/NotoEmoji%5Bwght%5D.ttf](https://raw.githubusercontent.com/google/fonts/1ac2012c34919f5fa2675aacf723fa98edb30b5f/ofl/notoemoji/NotoEmoji%5Bwght%5D.ttf) | Emoji and ZWJ sequences, monochrome outlines. U+0020 U+200D U+FE0F U+2764 U+1F468 U+1F469 U+1F4BB U+1F525 |
| `nana-test-vf.ttf` | `crates/nana-ui/src/nana_text/fixtures/nana-wdth-bevl.ttf` | Two glyphs, `wdth` plus a custom `BEVL` axis, cmap covers only U+0041. |

`noto-sans-sc` is **not** here: the corpus reads the product UI face from
`nana_ui_core::fonts::UI_FONT_REGULAR` rather than committing a second copy.

## SHA-256

```text
9759f33f418ea0ca3e48e4fe5bf69285c132159b6f13eb102012982da0035d0f  nana-test-vf.ttf
eb33391fddba3d5111f4c98ec710679abfcb31e24993b7e7cfc3c9a04993dae0  noto-emoji.ttf
6812d867a6e3fd23eae85ac7465aa34fae4406e46358517838b68f652c8f249e  noto-sans-arabic.ttf
97534e2086b7e4655a2b24b2899ef49e4ad4f33cc364f1fc2f8486173124583c  noto-sans-kr.ttf
```

## Upstream sources

Pinned to google/fonts `1ac2012c34919f5fa2675aacf723fa98edb30b5f`. SHA-256 of each downloaded
source, so the committed fixtures can always be reproduced:

```text
63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e  noto-sans-arabic
194018e6b2b293a7964f037b25c0249ce1418bc9ab3c971060a03aa57861e252  noto-sans-kr
de6c18832938afc99caf132b39d6a30a19bac7f2e812e28db2535b4608d27551  noto-emoji
```

## Licence

All Noto faces are licensed under the SIL Open Font
License 1.1; see `OFL.txt`.
