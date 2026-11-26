# Visual and Theme Guidelines

NanaUI's default visuals are designed for desktop productivity software: both dark and light appearances, compact spacing, and restrained, subtle separators. The accent color (Accent) is used only for primary actions or for controls whose value itself carries emphasis (such as primary buttons, switches that are on, and sliders with a valid value). Switching between dark and light replaces only palette colors; it changes neither control sizes nor how interaction states are layered.

The host returns the currently active `Arc<CompiledTheme>` from `RuntimeProgram::theme`. The built-in `Light` (`nana.light`) and `Dark` (`nana.dark`) are just two prebuilt themes; applications can register any custom `ThemeId` in `ThemeRegistry` through `ThemeDefinition`. All built-in controls read only semantic tokens and do not depend on specific theme names.

## Size and type scale

### Control heights and spacing grid

Single-line interactive controls come in three standard heights, applied through `ControlSize`:

| Tier | Height | Typical use |
| --- | ---: | --- |
| Small (`Small`) | 28 px | Navigation rows, menu items, dense lists, compact icon buttons |
| Medium (`Medium`) | 32 px | Form inputs, regular buttons, dropdown selects, default actions |
| Large (`Large`) | 36 px | Primary actions that need extra emphasis, set explicitly by the application |

Fixed desktop shell sizes: the title bar is `36 px` tall (`TITLE_BAR_HEIGHT`), window control buttons are `28 px`, and sidebar navigation rows are `28 px`. Multi-line containers grow with their content, but the single-line action controls inside them still follow the three heights above.

The spacing scale lives in `nana_ui::theme::space` and is defined on a **2 px grid** (1, 3, 5, and 7 have no tiers of their own and fold into `XXS`, `XS`, and `SM`). The `1 px` hairline stroke uses the `HAIRLINE` constant and is not a spacing tier.

### Type scale and weights

The product type scale and weights are defined in `nana_ui::theme::type_scale`. The standard body size is `13 px` (that is, `UI_BASE_TEXT_SIZE` / `type_scale::BODY`):

| Scale constant | Size | Weight constant | Value |
| --- | ---: | --- | ---: |
| `HINT` | 11 px | `REGULAR` | 400 |
| `META` | 12 px | `MEDIUM` | 500 |
| `BODY` | 13 px | `SEMIBOLD` | 600 |
| `SECTION` | 14 px | `BOLD` | 700 |
| `HEADING` | 16 px | | |
| `TITLE` | 18 px | | |
| `DISPLAY` | 20 px | | |

## Color, high contrast, and shadows

UI surfaces step up by level: a panel is one base surface, input boxes and inactive items step back one level, and hover, pressed, and selected step up in turn; text is correspondingly split into primary, muted, and faint. A focused input adds a neutral outline ring without changing its fill; validation errors prefer the danger color. Cards draw no outline by default and use shadows when levels need distinguishing; selected cards use a soft selected fill rather than an accent-colored border.

CSS single-layer `box-shadow` (outset and inset) and `text-shadow` (outset only) are wired into the rendering pipeline: inset shadows are painted with an inner-shadow SDF; when the container has children, the shadow is automatically layered over the content as an overlay through a destination composition group (dest group), rather than faking it by painting an outer shadow inside the box.

### Theme definitions and high-contrast overrides

Colors all come from the shared `ThemeTokens` and `SemanticPalette` (in `nana_ui::theme`), not from a separate stylesheet per control. Do not put arbitrary hard-coded business colors into framework tokens:

- **A whole custom theme**: build a `ThemeDefinition` (it can derive from `ThemeDefinition::NANA_DARK` or `NANA_LIGHT`, changing only the tiers you need, but every component recipe slot must be filled), then call `nana_ui::theme::install_theme_definition(&mut context, &definition)` or register it with the host's `ThemeRegistry`. This replaces colors, sizes, typography, motion durations, shadows, and component recipes in one step.
- **System high contrast**: when the operating system turns on high-contrast mode, the native host calls `AppContext::set_high_contrast(true)`. The installed dark theme then switches automatically to `SemanticPalette::high_contrast()` (black background, white text, yellow accent, cyan highlight), and a light theme switches to `SemanticPalette::high_contrast_light()` (white background, black text, dark blue accent); `muted` and `faint` keep grays distinguishable from body text, so disabled text does not look enabled. High contrast is a render-time policy overlay and does not rewrite the installed theme itself; turning high contrast off restores it unchanged, and themes installed in the meantime also get the high-contrast overlay automatically. When a high-contrast theme needs a thicker keyboard focus ring, use `ThemeDefinition::with_metrics(ThemeMetrics { focus_ring: FocusRingMetrics { width: 3.0, offset: 2.0 }, ..UI_METRICS })`: the ring is painted but takes no layout space, so switching themes does not relayout.
- **Public exports**: `nana_ui::theme` exports every token and recipe type that makes up a theme definition, including `ThemeMetrics`, `SwitchMetrics`, `TreeMetrics`, `KeycapMetrics`, `TypographyTokens`, `MotionTokens`, `EffectTokens`, `SpacingTokens`, `BorderTokens`, `OpacityTokens`, `SurfaceTokens`, `AccentRamp`, `ComponentThemeRegistry`, the recipe drafts (such as `ButtonVariantDraft`), component recipes (such as `DialogRecipe`, `ToastRecipe` with its `ToastIndicator` / `ToastGlyphs` / `ToastProgressRecipe` / `ToastMotion`, `ShapeRecipe` with its `ShapeRole`, `ChromeRadii`), and `ThemeId`, `ThemeSchemaVersion`, `ThemeGeneration`. `RadiusTier`, `ControlHeight`, `ControlPadding`, `SurfacePadding`, `SquareSize`, and `SemanticColorMix`, used for node-level named sizes, are imported from here too, so applications need no direct dependency on `nana-ui-core`.

> If you are changing the framework's own theme architecture or auditing hard-coded values in components, first read the maintainer document [Theme Audit Record](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/theme.md).

## Fonts, language, and typography features

Interface text, `<text>` in SVG images, and text an application draws or exports itself are all given their fonts by one font set: `nana-text`'s `FontSystem`, one per process. It has three kinds of source: the platform's system fonts, the face `bundled-fonts` registers, and the fonts the application registers. What the generic families (`sans-serif`, `serif`, `monospace`) point at, and what a missing character falls back to by its script, is its policy. No drawing path carries a font of its own.

To learn which fonts draw a piece of text, call `nana_ui::resolve_font_faces(family, weight, text)`: `family` is a CSS family list, and the result is, in order, the face it resolves to first and the fallback face each character of `text` needs. Each `ResolvedFontFace` has the family name, the bytes of the font file (shared with the font set, not copied) and the face's index in a collection. SVG text is drawn with exactly this result; an application that rasterizes or exports text itself should ask here rather than embed another face.

### Bundled font or system fonts

With the `bundled-fonts` feature enabled, the host automatically registers Noto Sans SC and makes it the default `sans-serif` UI font. The bundled font is a variable font: `wght` axis 400–700, default instance Regular, named instances Regular / Medium / SemiBold / Bold; all four UI weights use the real weight axis, with no synthesized bold. The character set is common Simplified Chinese characters plus JIS X 0208 level-1 kanji and the characters used in each consumer's UI copy. The file is generated by `scripts/build-ui-font.py`; for its source, checksums, and character set see `crates/nana-ui-core/assets/fonts/README.md`. Visual snapshots and design baseline checks must use the bundled font (`use_hermetic_fonts`); system fonts are not a substitute.

With the feature off, `sans-serif` resolves to system fonts through the platform's fallback table, and the binary is about 4.2 MiB smaller:

| Platform | Latin | Simplified Chinese |
| --- | --- | --- |
| Windows | Segoe UI | Microsoft YaHei UI |
| macOS / iOS | Helvetica Neue | PingFang SC |
| HarmonyOS | HarmonyOS Sans | HarmonyOS Sans SC |
| Linux / Android | Noto Sans, Roboto, DejaVu Sans | Noto Sans CJK SC, Noto Sans SC |

Three things are traded:

- **Linux may lack the glyphs**: the fallback table expects Noto's CJK fonts to be installed, and without them Chinese is not drawn. An application distributed for Linux should keep `bundled-fonts`; its own `Cargo.toml` can enable the feature per target.
- **Weights**: the built-in styles use 450, 600 and 650. The bundled face has all four UI weights on a real axis; a system family has only the weights it ships (Microsoft YaHei has Light / Regular / Bold), and an in-between weight is drawn with a neighbouring one, emboldened synthetically where needed.
- **Widths differ across platforms**: the same text measures differently on each, so lines break and truncate at different places.

Text in SVG images follows the same choice: the bundled face when it is there, system fonts when it is not.

### Host font registration and hot unloading

Applications can load their own fonts into the same `FontSystem`, alongside the bundled Noto font, at two granularities (glyphs not found still fall back to the bundled font or system fonts):

1. **Process-lifetime fonts**: call `register_host_font_bytes` or `register_host_font_file`; registered fonts stay until the process exits.
2. **Unloadable scoped fonts (skins / theme packs)**: use `HostFontScope`, whose `add_bytes`, `add_file`, and `add_face` methods each return a `HostFontRegistration` carrying the font family name; calling `clear()` or dropping the scope withdraws all fonts it registered at once (a single registration can also be managed by hand with `register_host_font_source*` and `unregister_host_font`). Withdrawing fonts advances to a new font generation (`FontGeneration`): any text that used those fonts is remeasured automatically at the next `flush`, and glyph bitmaps from the old generation are no longer sampled.

### OpenType shaping, CJK punctuation compression, and vertical text

- **Tracking and variable font axes**: tracking, `font-feature-settings`, and `font-kerning` go directly into the `nana-text` shaper rather than being an after-layout shift. `font-variation-settings` honors the variation axes actually present in the font: `wght` merges into `font-weight`, and `wdth` and custom axes (such as `BEVL`) go into the same `FontVariations`; axes the font lacks are skipped, never misreported or rewritten as `wght`.
- **Line breaking and CJK typography**: `word-break: break-all | break-word` and `line-break: anywhere` change the wrapping strategy (`keep-all` is not supported and is skipped when declared); `line-break: strict | loose` trims break opportunities by CJK rules. When the language (`lang`) is Chinese (`zh`) or Japanese (`ja`), punctuation at line start, line end, and between adjacent marks is compressed by default; `text-spacing-trim: space-all` turns compression off, and `auto` makes a line that does not fit compress punctuation first before wrapping. `text-autospace`, `text-align: justify` + `text-justify`, and `text-wrap-style: pretty` can be turned on as needed.
- **Language declaration (`lang`) and `locl` glyph substitution**: the application-wide default language is declared with `AppContext::set_default_language`, and a local subtree with `MutationQueue::set_language`; if none is declared and the text engine has no fallback language, text carries no language tag and no punctuation compression is triggered. As with the browser's HTML `lang`, the language tag also selects the font's built-in OpenType language system (such as `locl`) per text run's script: for example, the bundled Noto Sans SC replaces Latin digits `0–9` and `…` with shorter glyphs with no language or under `en` / `ja`, while after declaring `zh-CN` (or `zh-Hans`) digits keep standard height and straight quotes `"` `'` are raised accordingly (exactly matching how Chrome renders `lang="zh-CN"`).
- **Vertical writing (`writing-mode`)**: under `writing-mode: vertical-rl | vertical-lr`, text is laid out in top-to-bottom columns, CJK characters stand upright and Latin letters lie sideways, and the text editor also supports in-place editing in vertical columns.

## Component-specific metrics and node named sizes

The three heights and padding of single-line controls are stored in `ThemeMetrics`, while the geometry specific to particular components hangs off `ThemeMetrics` as composed substructures. This keeps a single control's internal numbers from being mixed with the global `control_height`, and guarantees they switch together with the installed theme:

- **`ThemeMetrics::switch` (`SwitchMetrics`)**: a default `30×16 px` track with an `8 px` label gap. A `Switch` without a label takes its width directly from the installed theme's track width with no extra padding, so it needs no manual width calculation when placed at the end of a settings row.
- **`ThemeMetrics::scrollbar`**: manages the scrollbar lane, idle and active thumb thickness, and inset; its colors, motion, and visibility policy live in `ScrollbarRecipe`.
- **`ThemeMetrics::tree` (`TreeMetrics`)**: manages `TreeView` row gap (`row_gap`), per-level indent (`indent`), row padding (`inset`), the disclosure triangle box (`disclosure`), and the row icon box (`icon`); the default constant `TREE_METRICS` is `2 / 12 / 4 / 16 / 12`.
- **`ThemeMetrics::keycap` (`KeycapMetrics`)**: manages the height of `kbd` / `chord` keycaps (`height`, also the minimum width of a single-letter keycap) and the bottom edge thickness (`edge`); the default constant `KEYCAP_METRICS` is `24 / 2`.
- **Compact density preset (`COMPACT_METRICS`)**: for professional tool inspectors and high-density panels (20 px small controls, 24 px medium controls, tighter padding, no gap between tree rows, 10 px per-level indent, 20 px keycap height); install it with `ThemeDefinition::with_metrics(nana_ui::theme::COMPACT_METRICS)`.

### Use named tiers instead of hard-coded numbers

When setting node sizes and corner radii, use the named tier methods on `NodeStyle`: `radius`, `corner_radii`, `control_height`, `control_padding_x`, `square`. **Do not write concrete floats such as `UI_METRICS.radius_lg` directly into `layout` when building nodes**: that expands the token into a dead number at construction time, and no theme installed later can change it.

Two rules apply when using named tiers:

1. **`control_padding_x` overrides the node's own left and right padding.** If a container needs to opt out of the standard control horizontal padding (for example `sidebar.rs`), first set `control_padding_x` to `None`, then set a custom `padding`.
2. **`square` is only the default square box when no size is specified explicitly** (written to `min_width` / `min_height`). As soon as a node explicitly sets `width` or `height` on an axis, the square's minimum on that axis yields automatically, so there is no need to clear `square` before giving an icon button `22×22`.

### Which radius tier a component takes: `ShapeRecipe`

Built-in controls do not name a radius tier themselves. Instead they write `NodeStyle::shape` (a `ShapeRole`) on the node, and the installed theme's `ShapeRecipe` decides which tier to take:

| `ShapeRole` | Recipe field | Used by |
| --- | --- | --- |

| `Button` | `button` | `Button` (every `ButtonKind`), and the trigger button of a `Popover` |
| `IconButton` | `icon_button` | `IconButton` (including `IconButton::tool`) |
| `Tag` | `tag` | `tag` / `tag_frame` |
| `Field` | `field` | `TextInput`, `NumberInput`, `TextArea`; the frames of the color, size, drag-value and path fields; the search field inside menu panels and the command palette input |
| `Select` | `select` | The trigger box of `Select` (and of `Dropdown` and `SearchDropdown`, which share its style) |
| `ListItem` | `list_item` | The hover, pressed and selected backgrounds of a `ListItem` row; the selected background, drag-origin box and drop box drawn by `ReorderList` |
| `Segmented` | `segmented` | The track of a segmented control; the selected pill takes the track's resolved radius minus the inset, so the two stay concentric |
| `MenuRow` | `menu_row` | Dropdown option rows: the `Select` list, option rows in menu panels, `ActionMenuItem` and command palette result rows |
| `Tab` | `tab` | Each tab of `Tabs` |
| `RadioRow` | `radio_row` | The hover and pressed background of each row in a radio group (`SegmentedControl::radio_group`) |
| `TreeRow` | `tree_row` | The hover and selected background of a `TreeView` row, and the "drop into this row" box while dragging |

Every built-in entry is `RadiusTier::Sm`, unchanged from before recipes existed. To change a design system, change the recipe on your own theme definition instead of rewriting each component:

```rust
// Only buttons, icon buttons and tags become pills
let halo = ThemeDefinition::NANA_DARK.with_shape(ShapeRecipe {
    button: RadiusTier::Full,
    icon_button: RadiusTier::Full,
    tag: RadiusTier::Full,
    ..ShapeRecipe::DEFAULT
});
// One entry: ShapeRecipe::DEFAULT.with(ShapeRole::Field, RadiusTier::Full)
// Everything: ShapeRecipe::uniform(RadiusTier::Full) — fields, list rows and menu rows change with it
```

Recipes are read from the installed theme when a style is resolved, so controls already on screen follow a theme switch in place. Dropdown option rows, the search fields of menus and the command palette, tree rows and the row backgrounds of `ReorderList` are drawn by the scene and read the recipe on every extraction through `ChromeRadii`'s `menu_row` / `field` / `tree_row` / `list_item`. Focus rings and hover fills follow the radius the node resolves. A tier written on the node itself (`radius` / `corner_radii`), or a numeric radius written by the caller's style, still wins. **Clearing `radius` alone does not remove `shape`**: for square corners use `NodeStyle::clear_corners()`, which clears `radius`, `corner_radii` and `shape` together; when a radius is written into `paint.border_radii`, that set of numbers still wins at paint time.

When two blocks join into one shape, use `corner_radii` to give each corner a tier in top-left, top-right, bottom-right, bottom-left order (`None` means square): keep the rounded corners on the outside and **set the seam corners to square (`None`)** — even a tiny `Xs` radius at the seam leaves a notch where the two arcs meet.

## Radius tiers and squircles (CornerShape)

`AppearanceSettings` exposes four base radii: micro (xs), control (sm), card (md) and page (lg), defaulting to `2`, `6`, `10` and `14` (`standard_radius` remains a compatibility alias for `md = 10`; changing it alone does not recompute the other three, and old JSON that only carries `standard_radius` is still derived with the ±4 / ±8 compatibility rule). On top of those, two tiers follow the page tiers for overlays and large surfaces:

- **`RadiusTier::Xl` (20 px by default)**: for rail surfaces that float above the page; always one tier rounder than a card.
- **`RadiusTier::Xxl` (24 px by default, `ThemeMetrics::radius_xxl`)**: for standalone large surfaces such as the media transport bar or a large detail card (CSS `--radius-2xl`, named `"xxl"` in Paint scripts).
- **`RadiusTier::Full` (`FULL_RADIUS`, not theme-dependent)**: pills, dots and avatars. It resolves to a value larger than any box and is clamped to half the short edge at paint time, so it never needs the height (named `"full"` in Paint scripts).

Type size, weight and spacing can also be named only by tier: `Text::role(TypeRole::Body)` / `.weight(TextWeight::Semibold)`, `Stack::gap_step(SpacingStep::Md)` / `.padding_step(x, y)` (that is, `NodeStyle::type_role` / `type_weight` / `gap_step` / `padding_step`). Like radius tiers, they resolve when the theme is installed, so a theme that changes the type or spacing scale moves the nodes with it; writing `type_scale::BODY` or `space::MD` spends the value at construction time.

An application can persist appearance settings into its own `Settings` namespace with `save_to_store` and `restore_from_store`. When the primary region sits flush against an expanded sidebar, the workspace surface squares off the two corners next to the sidebar and keeps the page radius on the other side.

### Corner shape: `CornerShape`

The geometry of every rounded corner comes from `ThemeMetrics.corner_shape` on the theme, configured with `ThemeDefinition::with_corner_shape` and installed into a document by `set_theme_definition`:

- `CornerShape::Round` (default): an ordinary circular arc.
- `CornerShape::SQUIRCLE`: CSS `corner-shape: superellipse(2)`.
- `CornerShape::Superellipse(k)`: `superellipse(k)`, with `k` in `1..=4`.

`corner_shape` applies to every GPU-drawn corner in that document: control and card surfaces, borders, shadows, `overflow` rounded clipping, frosted glass, `HostTexture`, solid rounded rectangles drawn by `Painter` without clipping, full pills, and `clip-path: inset() round`. Corners triangulated on the CPU (a `Painter` rectangle inside a clip, under a gradient or a non-uniform scale, and native window shadows) keep the ordinary arc. Each document draws with the theme installed on it, so windows and offscreen sessions do not affect each other; switching only `corner_shape` triggers a repaint, not a relayout.

## Motion and overlay interaction details

- **Shell transitions**: collapsing the Runtime sidebar and expanding a group both take `260 ms` with `EaseInOutCubic`. Frames are requested only while an animation runs, and reversing mid-way continues smoothly from the current progress.
- **Motion IR routing**: the Motion IR in `nana-ui-core::motion` routes by property. `opacity` and `transform` go through the compositor overlay; `color` and `filter` go through Paint-side CPU interpolation; `width`, `height` and per-axis `font-variation-settings` go through Layout tracks (a variable-font axis generates a real glyph variant, reshapes and rasterizes on every frame; it is never faked with a geometric scale).
- **Overlays and drag thresholds**: a menu aligns to the trigger's leading edge by default, and the framework pulls it back into the viewport near a window edge, so applications never compute coordinates by hand. A draggable tab or list row enters the drag state only after the pointer moves more than `4 px`; below the threshold the press is still an ordinary selection. The command palette (`CommandPalette`) is `680 px` wide by default and focuses its input when it opens.

### Toast action buttons

`ToastRecipe::control_radius` controls the radius of the action and close buttons inside a toast independently, defaulting to `RadiusTier::Sm`. A theme switch updates buttons that are already on screen in place. Choose a larger tier (such as `Xl`) when a pill look is wanted; the button size is still driven by `control_size`, and colors and interaction feedback keep following the button kind.

## Runtime theme expressions

Compiled Rust views retain theme references through the shared Style Model. Use
`gap: theme(space-md)`, `font-size: theme(type-body)`,
`font-weight: theme(weight-semibold)`, `line-height: theme(line-tall)`,
`border-width: theme(border-hairline)` and `border-radius: theme(radius-md)`.
Spacing names are `xxs/xs/sm/md/lg/xl/xxl/xxxl/page-tight/page`; type names are
`hint/meta/body/section/heading/title/display`. Line roles are `line/tall`.
The existing radius and control tiers also accept `theme(radius-lg)` and
`theme(control-small)`.

Padding, margin, gap and border-width shorthands accept mixed literal and theme
values. Dimensions, physical edges, individual border widths and typography also
accept linear `calc()`, for example
`min-height: calc(2 * theme(line-tall) + theme(space-xs) + 4px)`.
Multiplication and division require a constant operand; percentages and viewport
units continue to use the ordinary CSS length path. Literal declarations clear
the corresponding theme binding, including conditional and container rules.

Rust uses `nana_ui::theme::{ThemeExpr, ThemeToken, ThemeProperty}` through
`NodeStyle::theme(property, expression)` or
`Arc::make_mut(&mut style.layout).theme.padding_left = Some(expression)`.
`ThemeExpr::token(ThemeToken::Space(SpacingStep::Md))` supports `scaled`, `plus`
and `offset` for derived geometry. The expressions resolve on style writes and
theme installs, after responsive composition, without string lookup during a
frame. Palette-only changes do not invalidate layout or text shaping.

For geometry derived outside layout, `view::theme_signal()` yields the installed
`Arc<CompiledTheme>` after mounting and follows subsequent installs until its
scope is disposed. Prefer ordinary theme expressions for styles. Virtual grids
can use `.grid_theme(min_width, SpacingStep::Xxxl)` so column grouping, row spacing
and virtual row extents follow the same installed spacing step.
