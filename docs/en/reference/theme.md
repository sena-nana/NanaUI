# Theme audit record

This page covers the first two phases of the Theme rework ([#100](https://github.com/sena-nana/NanaUI/issues/100)).

- **§0–§6 are Phase 0 ([#101](https://github.com/sena-nana/NanaUI/issues/101)).** Before touching the architecture. It pins down the theme and style contracts, the visual baseline, and the performance baseline as they were at the time.
- **§7 is Phase 1 ([#102](https://github.com/sena-nana/NanaUI/issues/102)).** `ThemeDefinition` and the typed token system land. NanaLight and NanaDark are expressed with it.

The Phase 0 sections are kept as the audit record **of that time**. They are not rewritten after the fact. The value of that record is that it describes how things were before the work started. Conclusions that Phase 1 changed are marked "→ resolved in §7" in place. Everything else stays as it was.

No new Theme package was created. There is no ThemeScope. No string token map was introduced either.

**No existing pixel changed.** The 29 new fixtures capture states that were never captured before. The semantic baseline was confirmed file by file. It is purely additive. Token convergence is byte-for-byte unchanged under the default theme. The semantic baseline is 146/146 MATCH. What changed is "where the values come from".

If you are writing an application, read [Look](look.md) and [Controls](components.md). This page is for people who need to change the Theme architecture.

## Theme registration and persistence

`ThemeId` is the stable identity of a theme. It is an owned string, so applications can
register their own themes at runtime. `ThemeDefinition` describes the semantic palette, metrics, recipes, motion,
and effects. The host passes it to `ThemeRegistry::register`; the registry validates and compiles it,
and it is then installed through `AppContext::set_theme_definition` or `set_theme_tokens`.

`ThemeDefinition::NANA_LIGHT` and `ThemeDefinition::NANA_DARK` are the two built-in themes.
`ThemeAppearance` is only an appearance hint (`Light`, `Dark`, or `Custom`), not a theme identity.
A custom theme can use `Custom`; the runtime does not fall back to light or dark tokens for it.
Settings pages should use the `ThemeChoice` list provided by the registry, and persistence stores only `theme_id`.
At startup, if the saved ID is not registered yet, `ThemeRegistry::resolve` returns the built-in Light theme
and keeps a diagnosable fallback result through `fell_back_to_light`.

## 0. One-minute summary

| Question | Current state |
| --- | --- |
| Is there a Style Model | Yes, and only one: `nana_ui_core::style_model`, shared by the Rust API and CSS |
| Color authority | `SemanticPalette` (24 fields + 15 derived roles); components express intent through roles, never RGB |
| Size authority | **Converging**: radius, scrollbar, control height, control/field/list inset, **panel inset, icon-button square, multiline field vertical inset, Large padding** already follow the installed `ThemeMetrics`; the renderer reads no constants anywhere. What remains are local geometry `const`s and a few menu height estimates |
| Motion authority | ~~**Split**: `ThemeMetrics::motion_*_ms` are dead fields (zero consumers); the real durations are `const`s in `nana_ui_core::motion`~~ → **resolved in §7**: each of the eight durations is its own `MotionRole`, and the `motion` `const`s now read the default theme instead |
| Where the Theme is resolved | Two places, by value size: **colors at extract** (16 bytes, resolved downstream), **sizes at style write time** (`LayoutStyle` is 4808 bytes; on the read path it would be amplified every frame — see §1.5). Neither is in the retained style phase |
| Cost of a Theme switch | Paint invalidation for the whole document; palette-only does not touch Layout / Text (verified) |
| Does the renderer know about the Theme | Essentially no: `nana-ui-scene` has only 3 uses of `SemanticColorRole`, all in tests and benchmarks |
| Hard-coded RGB in components | 0 |
| Duplicate token types | Two removed: `Colors` (= `SemanticPalette`) and the standalone `SCROLLBAR_METRICS`; §7 removed two more: `space`/`type_scale`/`HAIRLINE`/motion `const`s all read token defaults now |
| State matrix | Components declared paint for states, 32 of which were never captured; this round added 29, and the remaining 3 are explained as to why they cannot be added |

## 1. Existing Style Model inventory

### 1.1 Single-authority diagram

```text
                        AppearanceSettings              WindowMaterialMode
                     (radius ×4 / backdrop /            (Solid / Vibrancy /
                      workspace corners)                 Mica / Acrylic)
                              │                                │
                              └───────────┬────────────────────┘
                                          ▼
                              nana_ui::theme::ThemeTokens
                              (SemanticPalette + ThemeMetrics + titlebar
                                + with_backdrop alpha override)
                                          │
                              install_theme_tokens
                              → AppContext::set_theme_tokens
                                          ▼
   CSS subset  ─┐                 StyleModelRef  ← ThemeAppearance
                ├───► Semantics ──►  · theme_id / appearance
   L3 Rust API ─┘   (ButtonKind/     · metrics : ThemeMetrics
                     ControlSize/    · palette : SemanticPalette
                     StatusTone…)    · titlebar: SemanticColor
                            │                │
                            ▼                ▼
                     NodeStyle           UiWorld.style_model
                  (layout: LayoutStyle,         │
                    foreground/background/      │ resolve_styles
                    border: SemanticColorRole,  │ (inheritance + palette_paint_colors)
                    interaction: InteractionStyle)
                            │                   ▼
                            └────────────► ComputedStyle  (= existing ResolvedStyle)
                                                │
                                         extract_nodes
                                   (colors recomputed here when palette_epoch is stale)
                                                ▼
                                          ExtractedNode
                                    · style: Arc<ComputedStyle>
                                    · standard_visual: StandardVisual
                                    · component_geometry
                                                ▼
                                      UiScene (ScenePrimitive)
                                                ▼
                                         SceneWgpuPainter
```

Two **bypasses** must be read together with it. Otherwise the diagram lies:

```text
UI_METRICS (compile-time const) ────────────► view_components.rs and other components
   (panel / icon / vertical field and module-level consts remain;
     world/geometry.rs and nana-ui-scene/primitives* are no longer on this list)

nana_ui_core::motion::{HOVER_COLOR, …} ─────► component animations
   (const, not themeable; nobody reads ThemeMetrics.motion_*_ms)
```

### 1.2 Type inventory

| Type | Location | What it is | Destination |
| --- | --- | --- | --- |
| `ThemeId` / `ThemeAppearance` | [theme/mod.rs](../../crates/nana-ui-core/src/theme/mod.rs) | Persistable theme identity and the Light / Dark / Custom appearance hint | Runtime theme identity; Light/Dark are built-in themes in the registry |
| `SemanticPalette` | [style_model.rs](../../crates/nana-ui-core/src/style_model.rs) | 24 semantic color fields, two constants for dark/light | **Keep**, used directly as the semantic color token layer; copying a second one is forbidden |
| `SemanticColorRole` | Same | 39 roles: 24 map to fields, 15 are derived in `get()` (`WarningSoft*`, `DangerSoft*`, `Titlebar`, 9 code token roles) | **Keep**; the derivation rules are the seed of recipes, and Phase 1 must make them explicit |
| `SemanticColorMix` | Same | Premultiplied mix of two roles / single-role alpha, weights in basis points | **Keep**; this is the existing expression of the "state layer" |
| `SemanticColor` | Same | Backend-neutral RGBA 0..=1 | **Keep** |
| `ThemeMetrics` | [theme/mod.rs](../../crates/nana-ui-core/src/theme/mod.rs) | Non-color tokens: radius ×4, control height ×3, padding, icon size, panel padding, field/list padding, plus the composed scrollbar. ~~**motion ×2**~~ removed (§7) | **Evolved**: `DesignTokens.metrics` in §7 holds it by value |
| `UI_METRICS` | Same | The `const` default of `ThemeMetrics` | **Retired as the sole authority**: it is now the initial value of `DesignTokens::for_appearance`; there are still 62 product read sites, see §1.5 |
| `ScrollbarMetrics` / `SCROLLBAR_METRICS` | [scrollbar.rs](../../crates/nana-ui-core/src/scrollbar.rs) | 5 scrollbar-specific geometry tokens (thickness 12 / thumb 6 / min length 24 / inset 2 / page 0.9) | **Keep**: this round it is held by composition in `ThemeMetrics.scrollbar` (§1.5) and is no longer a third set of metrics |
| `space` / `type_scale` | Same | 10 spacing steps, 8 font sizes (including `HINT` 11 / `TITLE` 18, and `CARD_TITLE` 15 for card or dialog names, matching `TypeRole::CardTitle`) + 4 font weights, bare `const`s; every `TypeRole` step has a bare constant of the same name | **Evolved**: `SpacingTokens` / `TypographyTokens` in §7; the `const`s now read their `DEFAULT` instead of holding numbers themselves |
| `StyleModelRef` | [style_model.rs](../../crates/nana-ui-core/src/style_model.rs) | Read-only view of appearance + metrics + palette + titlebar; `color(role)` is the token read entry point | Provided by the installed `CompiledTheme` |
| `ControlSemantics` | Same | Combination of size + button/card kind + status | **Keep**; it is the recipe selector |
| `ControlSize` / `ButtonKind` / `CardKind` / `StatusTone` / `ToastTone` / `ValidationIntent` | [semantics.rs](../../crates/nana-ui-core/src/semantics.rs) | Component intent enums | **Keep**; the variant axes of Component Recipes |
| `ThemeTokens` / `ThemeAppearanceExt` | [nana-ui/theme.rs](../../crates/nana-ui/src/theme.rs) | L3 host-side adapter: installs `SemanticPalette` + `ThemeMetrics` + titlebar directly | **Keep** (the `Colors` copy was removed this round, see §1.5); from §7 on, `ThemeTokens` is the color + size projection of `ThemeDefinition`, and `with_backdrop` decides which role gets alpha according to `SurfaceTokens` |
| `RadiusTier` | [theme.rs](../../crates/nana-ui-core/src/theme.rs) | Corner radius tier intent (Xs/Sm/Md/Lg), carried by `NodeStyle.radius` and resolved against the installed `ThemeMetrics` at style write time | **Keep**: this is the first landing point of "components state intent, the theme supplies values" on the size side; the other metrics follow the same shape |
| `ControlHeight` | [theme.rs](../../crates/nana-ui-core/src/theme.rs) | Control height intent: `Min(ControlSize)` / `Exact(ControlSize)`, carried by `NodeStyle.control_height` and resolved at style write time | **Keep**: the `Min` / `Exact` distinction used to be hidden in whether a call site wrote `min_height` or `height`; now it is stated intent |
| `ControlPadding` | Same | Horizontal inset intent: `Compact` / `Standard` / `Roomy` / `Field` / `ListItem`, carried by `NodeStyle.control_padding_x`; multiline fields also use `control_padding_y` | **Keep**: not a wrapper around `ControlSize` — text fields and list rows have their own metrics fields |
| `SurfacePadding` | Same | Panel inset: `Panel` writes all four sides, `PanelX` writes only left and right | **Keep**: used by card/settings group/form surfaces, not controls |
| `SquareSize` | Same | Square box: `IconButton`, `Control(ControlSize)`, or `Keycap`, writes `min_width`/`min_height`; an axis on which the node writes its own `width` / `height` is not written | **Keep**: an icon button uses `icon_button_size` by default and switches to ControlSize only through `size()` |
| `ChromeRadii` | Same | The four resolved radius tiers, carried to the renderer on `ExtractedNode` | **Keep**: lets the renderer only consume, never resolve; as a result `nana-ui-scene` no longer reads `UI_METRICS` anywhere |
| `CornerShape` | [theme/mod.rs](../../crates/nana-ui-core/src/theme/mod.rs) | Corner shape: `Round` (circular arc, default) or CSS `superellipse(k)`, held by `ThemeMetrics.corner_shape` | **A scene-level value**, not carried on `ExtractedNode` like `ChromeRadii`: the painter writes it as a uniform once per draw, not per primitive — quad vertex attributes and clip data are already full, and carrying it per primitive would widen every pipeline's instance struct. `RuntimeDocument` syncs it into `UiScene::corner_shape` every frame and swaps the scene instance when it changes; changing only the shape does not mark LAYOUT, theme installation still marks every node RENDER, and the shape lands with that frame's delta |
| `AppearanceSettings` | [settings.rs](../../crates/nana-ui-core/src/settings.rs) | User/system policy: radius ×4, window material, backdrop target/opacity, titlebar following, workspace corners | **Evolves** into the policy overlay of #100 §11, not a second theme |
| `NodeStyle` / `InteractionStyle` / `SemanticPaint` | [nana-ui-runtime](../../crates/nana-ui-runtime/src) | Node-local style intent (semantic color roles + radius tier) + paint overlays for 7 interaction states | **Keep**; it is the explicit local style intent and state input; the intent fields are where size/color convergence lands |
| `ComputedStyle` + `ResolvedStyle(Arc<ComputedStyle>, palette_epoch)` | [nana-ui-runtime](../../crates/nana-ui-runtime/src) | Inheritance resolution result + theme generation | **Evolves** into `ResolvedStyle` of #100 §6 (it already has that name, but covers only inheritance and color, not metrics/recipes) |
| `StandardVisual` / `ComponentGeometry` | Same | Retained "what control this is / its geometry" | **Keep**; recipe selector and layout output |

### 1.3 Sources of default styles

| Layer | Where the default style comes from | Conflicts |
| --- | --- | --- |
| **L3 Rust** | `impl ComponentView` in `view_components.rs`: picks a `SemanticColorRole` from semantics such as `ButtonKind` and writes it into `NodeStyle.interaction`; geometry and the remaining colors are decided by `world/geometry.rs` + `world/extraction.rs` | The look of one control is spread across three places: the component body (roles + states), extraction (`StandardVisual` colors), scene primitives (radius/stroke) |
| **CSS** | [`css_map.rs`](../../crates/nana-ui-css/src/css_map.rs) maps the CSS subset onto Layout; [`style.rs`](../../crates/nana-ui-css/src/style.rs) only does paint resolution; known token names enter Tokens through `SemanticColorRole::from_css_token_name`, unknown `#hex` can only be a restricted paint hint | The rules are fixed and tested; currently the cleanest layer |

The names `from_css_token_name` recognizes are the official CSS token surface: `background`/`surface`/`subtle`/`raised` (alias `elevated`)/`header`/`hover`/`active`/`selected*`/`border*`/`text`/`muted`/`faint`/`accent*`/`success`/`warning*`/`danger*`/`titlebar`/`on-media`. Plus the `--nana-` prefix and `var()` wrapping. The 9 code token roles (`Keyword`/`Function`/…) are **not** on the CSS surface. They are used only in L3 semantic highlighting.

### 1.4 Audit findings

Ordered by impact. Each has reproducible evidence.

**F1 — Metrics have two authorities. The compile-time constant wins.**
`AppearanceSettings::metrics()` builds a `ThemeMetrics` with custom radii. It is installed into `StyleModelRef` through `set_style_tokens`. `apply_style_model` also correctly escalates a metrics change into LAYOUT invalidation. But product code (excluding tests and bins) reads the **`UI_METRICS` constant on 116 lines** (runtime 92 / scene 21 / host 3). Only **16 lines** read the installed value. All of them are in runtime. A typical case: `control_layout()` in `view_components.rs` writes `border_radius: Some(6.0)` directly. Elsewhere in the same file it writes `layout.border_radius = Some(world.theme_metrics().radius_md)`.
→ The first job of #100 Phase 1 should be converging the metrics read sites. Otherwise installing a `ThemeDefinition` has no effect. **This round did four of them: radius, scrollbar, control height, and control/field/list horizontal inset** (scene 0; what remains in runtime is panel / icon / vertical field and module-level consts). The rest and what blocks them are in §1.5.

**F2 — Motion tokens are dead fields.** (**resolved in §7**)
`ThemeMetrics::motion_fast_ms` (120) / `motion_standard_ms` (240) have zero consumers in the whole repository. The real durations are 8 `const`s in `nana_ui_core::motion`: `HOVER_COLOR` 120, `OVERLAY_FADE` 140, `MENU_OPACITY` 160, `MENU_POP` 180, `SIDEBAR_COLLAPSE` 260, `SKELETON_PULSE` 1400, `SPINNER_ROTATION` 900, `LOADING_SPIN` 800. They are `const`. A theme cannot change them.
→ When connecting #100 §8 / #87, these two fields must be either wired up or removed. They cannot keep pretending "it looks like this already exists". **Phase 1 chose removal**: both fields are gone. Each of the eight durations became its own `MotionRole`. The 260 of `SIDEBAR_COLLAPSE` no longer needs to line up with anything. It is simply its own step.

**F3 — The Theme is resolved in the extract phase. Not in the retained style phase.**
`SetTheme` marks only RENDER. It does not mark STYLE (`set_theme_marks_render_not_style_when_only_palette_roles_change` in [world/tests.rs](../../crates/nana-ui-runtime/src/world/tests.rs) already pins this behavior). The actual color change happens in `extract_node`: when `resolved_epoch != palette_epoch` it reruns `palette_paint_colors`. The new colors are written into `ExtractedNode.style`. The retained `ComputedStyle` is not refreshed.
This is **cheap** for a palette-only switch (it does not touch Layout and does not reshape; see §4). But it puts token reads on the per-frame extract path. That is exactly the shape #100 §6 wants to remove.

This round tried extending the same shape to the size side. **It does not hold**: a color is 16 bytes. Recomputing it is cheap. Sizes live in the 4808-byte `LayoutStyle`. Put on the same path, they are amplified every frame (§1.5). So when #100 §6 moves resolution into the retained phase, it does not copy what extract does. **The resolution point must be chosen per value, according to the size of the resolved value.**

**F4 — Installing a Theme = whole-document invalidation.**
`apply_style_model` walks the entire subtree of every live document root and `mark`s each node. A palette-only switch on 1k controls = paint invalidation on 1001 nodes. The dependency classes of #100 §7 exist to bring this number down. Phase 0's job is to measure it (§4).

**F5 — Components do not write RGB. But they carry the role decisions themselves.**
Across 122 components (`impl ComponentView for X` plus `impl X`): raw RGBA **0**. Bare design numbers **29**. `SemanticColorRole` decisions 232 (`Button` 22, `SidebarRow` 10, `ListItem` 9…). Another 216 are outside components (`StandardVisual` colors in `world/extraction.rs`, `menus.rs`, `popover.rs`, `select.rs`, `settings.rs`…).
This is **not a defect**. "The hover of primary is AccentStrong" is exactly the semantics #100 wants components to express. It is the migration workload for Component Recipes: 448 role decisions need to move.

> This was corrected once during this round: the first scan covered only `impl ComponentView` function bodies. As a result `ListItem` was reported as "declares no states". Its whole `InteractionStyle` is written in `ListItem::new`. The constructor is where a component's default look lives. The scanner now counts both kinds of impl.

**F6 — Interaction states are declared by 19 components themselves.**
19 of the 122 components use `InteractionStyle`: `Button`, `Checkbox`, `Chip`, `CommandPalette`, `Dropdown`, `HoverCard`, `IconButton`, `InteractiveCard`, `ListItem`, `NumberInput`, `RangeField`, `SearchDropdown`, `SettingsCollapsibleCard`, `SidebarRow`, `SidebarSection`, `Switch`, `TextArea`, `TextInput`, `XYPad`. The hover/pressed/selected visuals of the other controls come from `StandardVisual` branches in extraction / scene.
→ The unified state bits of #100 §4 must cover both paths. Changing only `InteractionStyle` would miss about a hundred controls.

**F7 — 217 module-level design constants.**
The runtime crate has 217 `const NAME: f32/u16/u64 = <number>` items (`ROW_HEIGHT`, `SWATCH_SIZE`, `SHORTCUT_TEXT_SIZE`, `DOCK_TITLE_BAR_HEIGHT`…). Moving a number from inline into a private `const` is not the same as moving it into the Theme.

**F8 — The renderer already knows essentially nothing about the Theme.** (This round closed the last gap; see §1.5.)
The entire `nana-ui-scene` crate has only 3 uses of `SemanticColorRole`. All in tests / benchmark bins. `scene_paint` in `nana-ui` has only 1. Also in tests. Colors arrive with `ExtractedNode` as resolved `[f32; 4]`.
→ The #100 boundary "Renderer != Theme resolver" **already holds**. The only gap was scene reading `UI_METRICS` (F1). This round closed it. Scene now reads it nowhere.

**F9 — `Colors` is an isomorphic copy of `SemanticPalette`.** (Resolved this round; see §1.5.)
`nana_ui::theme::Colors` has exactly the same 24 fields as `SemanticPalette`. Plus conversions both ways. It is a historical adapter layer. Not a second design authority. But it really is "the same design semantics defined in two places".

**F10 — The scrollbar has its own metrics type.** (Resolved this round; see §1.5.)
`ScrollbarMetrics` / `SCROLLBAR_METRICS` (thickness 12, thumb 6, min length 24, inset 2, page 0.9) live in `nana-ui-core/scrollbar.rs`. On the same level as `ThemeMetrics`. CSS `::-webkit-scrollbar` overrides attach to it too. Density adjustment cannot change the scrollbar today.
→ Phase 1 merges it into the control metrics. Do not leave a third copy outside `ThemeDefinition`.

**F11 — The accessibility role declared by `SettingsCollapsibleCard` does not reach the node.**
`SettingsCollapsibleCard::project` writes `role: AccessibilityRole::Button` and `disabled: self.disabled`. But the card node always reads `role=Generic disabled=false`. This includes the `expanded` / `collapsed` fixtures that existed before this round. The semantic baseline surfaced this: pixel snapshots cannot see roles. Only recording a11y alongside exposes this kind of "declared but never landed".
→ Not changed within the scope of #101: changing accessibility roles is a product decision. It affects a11y verification. Recorded for now. This is also the direct reason the disabled / focused fixtures of `settings-collapsible-card` cannot be added (see §3.3).

**F12 — The largest heap cost on the style path. Counters cannot see it.** (Fixed this round; see §1.5.)
`style_allocated_bytes` reports "208 bytes per node" accurately. But it misses the 4808-byte per-node `LayoutStyle` copy on the same path. The 1k-control row is under-reported by 23×. The reason is not that someone forgot to record it. It **cannot be recorded**: the copy happens inside `Arc::make_mut`. When unique it reuses in place; when shared it clones. Neither goes through any allocator counter.
→ This is something Phase 0 must deliver: a baseline recorded with a blind counter invalidates every later "looks unchanged" judgment. `layout_copies` / `layout_copied_bytes` and two required invariants have been added. **The more general lesson**: every time #100 introduces a new resolution point, ask "which counter sees this point's cost". When the answer is "none", add a counter. Do not wait for it to show up somewhere else as time.

## 1.5 Convergence done this round

After the audit reached its conclusions, six things were done in place. **Rendering under the default theme is byte-for-byte unchanged.** What changed is the mechanism. Not the pixels.

The evidence has two layers: the semantic baseline is 146/146 MATCH. And on the same machine, with the same temporary adapter key, the full set of 615 pixel snapshots was recorded once before and once after the convergence. `diff -rq` reports **0 files different**. A change that "makes the theme actually take effect" should change nothing under the default theme.

### One palette, not two (F9 resolved)

`nana_ui::theme::Colors` is deleted. It had exactly the same 24 fields as `SemanticPalette` and a pair of conversions back and forth. `ThemeTokens` now holds `SemanticPalette` directly. `ThemeAppearanceExt::colors()` is removed (`palette()` already exists). One design language does not need two spellings. The second spelling is exactly where the two start drifting apart.

Consumer changes: `Colors` → `SemanticPalette`. `theme.colors()` → `theme.palette()`. `tokens.colors` → `tokens.palette`.

### The scrollbar joins the theme (F10 resolved)

`ThemeMetrics` now **composes** (not flattens) a `scrollbar: ScrollbarMetrics`. The five scrollbar shape numbers should not sit next to `control_height`. But they do belong to the **installed** theme. The one base line in `world/geometry.rs` switched from the constant to `self.style_model.metrics.scrollbar`.

`#[serde(default)]`: settings blobs written before this field still read.

The test that pins it: `a_thicker_scrollbar_in_the_installed_theme_reaches_the_bar`. Switch that line back to the constant and it reports `12 -> 12` and fails.

### Tree geometry joins the theme (#341)

`ThemeMetrics` also composes a `tree: TreeMetrics`, with these fields:

- `row_gap`: spacing between rows.
- `indent`: indentation per level.
- `inset`: row padding.
- `disclosure`: the box of the disclosure triangle.
- `icon`: the box of the row icon.

The default `TREE_METRICS` is exactly the constants that used to be hard-coded in `TreeView` (2 / 12 / 4 / 16 / 12), so default-theme pixels do not change. The row height was already the small control height (`compact_control_height`), which already followed the theme. `#[serde(default)]` keeps older metrics blobs readable; at theme compile time these five values, like every other length, must pass the finite and non-negative checks.

`COMPACT_METRICS` is an example compact density, matching the 20 px rows common in professional tool panels:

- Small and medium controls are 20 / 24 tall.
- Padding is tighter.
- Tree rows have no gap between them, and each level indents 10.
- Keycaps are 20 tall.
- Colors, font sizes, and radii are unchanged.

Install it with `ThemeDefinition::NANA_DARK.with_metrics(COMPACT_METRICS)`. Menu item height is also the small control height, so it follows this density as well. The `tree-view/compact` fixture in the gallery shows this density.

The test that pins it: `rows_follow_the_installed_tree_metrics`. It measures the tree height, row pitch, and indentation once under the default theme and once under the compact example.

`TOUCH_METRICS` is the touch density for phones and tablets:

- Small and medium controls and icon buttons are all 44 tall, large ones 48, never below the minimum touch target of the platform guidelines.
- The switch track grows to 44 × 26.
- Padding is wider, tree indentation per level is larger, and keycaps are 32 tall.
- Colors, font sizes, and radii are unchanged.

The mobile host (`platform/mobile-core`) installs this density by default; see [mobile-host.md](mobile-host.md). The test that pins it: `touch_metrics_keep_every_target_at_the_touch_minimum`.

### Keycap geometry joins the theme (#345)

`ThemeMetrics` also composes a `keycap: KeycapMetrics`:

- `height`: keycap height, which is also the minimum width of a single-letter keycap.
- `edge`: thickness of the bottom edge drawn inside the keycap.

The default `KEYCAP_METRICS` (24 / 2) is exactly the two constants that used to be written in `view/marks.rs`, so default-theme pixels do not change. `keycap_face()` names its minimum width and height with `SquareSize::Keycap`, and `KeycapPainter` reads the bottom edge from `PaintContext::metrics()`, so both are re-resolved when the theme changes. `#[serde(default)]` and the theme compile checks are the same as for `tree`.

The test that pins it: `a_keycap_follows_the_installed_keycap_height`.

Menu surface padding and item spacing (`MENU_SURFACE_PADDING` / `MENU_ITEM_GAP`) are still constants. They are written into the style when the menu style is built, so an installed theme cannot reach them. Turning them into tokens requires every menu surface (ContextMenu, Select, ActionMenu, AppMenuBar) to switch to write-time resolution together; changing only one would misalign panel height and content.

### Radii name a tier, not pixels (F1 partly resolved)

This is the structural change of this round.

`NodeStyle` gains `radius: Option<RadiusTier>`. It has **exactly the same shape** as `background: Option<SemanticColorRole>` two lines above it: the component names a design tier. The theme decides the value.

```text
Before: Button::new()  ──► layout.border_radius = Some(6.0)   // token spent at construction time
        set_style_tokens(radius_sm = 2.0)                      // too late, it is already a number

Now:    Button::new()  ──► style.radius = Some(RadiusTier::Sm) // names only the tier
        write_node_style ──► resolved_layout.border_radius = tier.resolve(installed.metrics)
```

**The resolution point is at write time. Not at extract. This was changed after performance data overturned the first choice.** The first version followed the color precedent and put it at extract (resolved downstream, same invalidation). After writing it, it turned out that `LayoutStyle` is **4808 bytes**, and extract calls `Arc::make_mut` on it once per control per frame. Measured: palette-switch **+196%**, density **+240%**, controls-1k **+101%**. Colors can be resolved at extract because a color is 16 bytes. A radius sits inside a 4.8 KB box. The same path no longer works.

It moved to `NodeRecord.resolved_layout`: resolved once when the node style is written. Readers take it directly. The cost moves from **every frame** to **every change**, and theme changes are rare events. The price is that every node with an intent holds one more `Arc<LayoutStyle>`. See "The real cost of this convergence" below.

The single write entry point `write_node_style` guarantees that the authored style and the resolved layout never go separate ways. This is not excess caution. It plugs a hole: after the change, 3 EmptyState tests failed, because two write sites (`world/text.rs`, `world/motion.rs`) bypassed `SetStyle`.

`NodeStyle::radius_px()` is kept as a one-off explicit override. #100 states explicitly that the ability to write explicit local style is not removed. The tests pin both sides: a tier follows the theme. px does not.

25 sites have been migrated (Button / TextInput / TextArea / IconButton / ListItem / Select / Switch / Chip / menu rows / Popover / XYPad / ColorField / PathField / HoverCard / GraphMinimap / SidebarRow / SidebarSection / SidebarFooterButton, and others). `color_field.rs`, `hover_card.rs`, `path_field.rs`, `thumbnail.rs`, `menus.rs`, `select.rs`, `xy_pad.rs`, and `media_transport.rs` no longer import `UI_METRICS` at all. That is the signal that the tokens have really left the components.

The test that pins it: `an_installed_radius_reaches_a_control_that_named_the_tier`.

### The renderer no longer reads constants (scene side of F1, done)

The remaining 20 scene reads were all `corner_radii(UI_METRICS.radius_*)`. They draw framework chrome on nodes that did **not** declare a radius: menu surfaces, modals, command palette rows, focus plates, calendar cells, the image viewer.

`ExtractedNode` gains `chrome_radii: ChromeRadii`. The four resolved radius tiers. It follows the same precedent as `standard_visual_foreground: Option<[f32; 4]>` on the line above it: a value decided by the theme. Carried to the renderer. So the renderer does not have to look it up itself.

```text
Before: scene/primitives ──► UI_METRICS.radius_md     // renderer looks up a constant
Now:    extract_node     ──► chrome_radii = metrics.into()
        scene/primitives ──► node.chrome_radii.md     // renderer only consumes
```

**Why ride on `ExtractedNode` instead of giving scene a frame-level theme handle**: a metrics change marks every node RENDER → everything is re-extracted → primitives are rebuilt. Riding on the node, the invalidation contract is already right. A separate handle for scene would have to guarantee ordering and invalidation on its own. And that is closer to "giving the renderer a copy of the theme".

Result: **`nana-ui-scene` now reads `UI_METRICS` nowhere** (previously 21 sites). Even the import is gone. The #100 boundary "Renderer != Theme resolver" went from "essentially holds" to "fully holds".

Which tier to pick while drawing is still a design decision that stays in the renderer. That is the part #100 §3 Component Recipes must absorb. This round first gets to "the renderer only consumes, never resolves".

The tests that pin it: `a_menu_surface_paints_the_radius_it_was_handed_not_the_constant` (scene side) and the `chrome_radii` assertion of `an_installed_radius_reaches_a_control_that_named_the_tier` (end to end).

### Control heights name a tier, not pixels (option A, implemented this round)

The first wave done with A after the comparison in §1.6. `NodeStyle` gains `control_height: Option<ControlHeight>`. `ControlHeight::Min(ControlSize)` / `Exact(ControlSize)` distinguishes "minimum height" from "fixed height". This distinction used to be hidden in whether the call site wrote `min_height` or `height`.

`control_layout()` no longer writes `min_height`. The `size()` of `Button` / `TextInput` / `ListItem` now sets an intent instead of computing pixels.

**The semantic baseline caught a real bug here**: the `ControlHeight::Min(Medium)` hard-coded in the constructor overrode `Button::size()`. Small and Large buttons were both rendered at Medium height. The pixel baseline could not catch it (the gallery did not capture buttons in non-Medium sizes). The semantic baseline caught it. Because it records "what the theme resolves to", not "what it looks like".

`.layout(custom)` clears `control_height`, `control_padding_x`, and `radius`: replacing the whole layout has always meant "I take over". An intent that survived it would fight the explicit layout (an overlay test in `nana-ui-scene` hit exactly this).

### Horizontal insets name a tier, not pixels (option A, second wave)

`ControlSize` does not cover text fields and list rows: each has its own metrics field. It is not "a medium control with a different padding". So this wave did not wrap insets into `ControlSize`. It made `ControlPadding`, shaped like `RadiusTier`: `Compact` / `Standard` / `Roomy` / `Field` / `ListItem`.

`control_layout()` no longer writes `padding_left` / `padding_right`. `Button` / `TextInput` / `ListItem` / `IconButton` / `TableCell` / `Chip` / `Select` / `Switch` / `RangeField` / menu rows name a tier on the node. `write_node_style` resolves it. The fallback in `used_layout_padding` also reads `resolved_layout` now. Otherwise extract, before layout has run, would report the inset of an intent node as 0. The scene would get the spent number rather than the installed value.

Geometry calculations (menu text start, gutter `max`, the negative margin of pill bleed) keep using the installed metrics. No new intent fields. The **vertical** inset of a multiline `TextArea` still reads `UI_METRICS.field_padding_x`: `ControlPadding` writes only left and right.

The test that pins it: `an_installed_control_padding_reaches_a_control_that_named_its_inset`. The three tiers of Button / TextInput / ListItem each follow the installed value. Merging `Field` into `Standard` makes this test fail.

### Panels, squares, vertical insets, Large padding (option A, third wave)

The same path continues through the size interfaces that were still open:

- `SurfacePadding::{Panel, PanelX}`: cards / settings groups / form surfaces / pane tabs. `Panel` writes all four sides. `PanelX` writes only left and right. Tabs must not take the vertical panel padding. Otherwise they grow taller.
- `SquareSize::{IconButton, Control(size), Keycap}`: an icon button uses `icon_button_size` by default and a keycap uses `keycap.height`. Only `size()` switches to ControlSize. An explicit side length wins: on an axis where the node wrote `width` / `height`, `square` does not write a minimum (the 20px sidebar tools, an application's 22px close button), so the intent never stretches a spent box back into a square.
- `control_padding_y`: a multiline TextArea uses `Field` as its block-axis inset. Single-line fields do not write it. That avoids stretching the line box.
- `ThemeMetrics.large_control_padding_x`: `serde(default)` is still `space::XXL`. Old settings blobs still read. Large / Roomy can finally be moved by the installed theme.
- The workspace main area radius is now `RadiusTier::Lg`. It no longer spends `UI_METRICS.radius_md`.
- Extract / menu / Select / TreeView / ReorderList geometry now reads `height_in` / `padding_x_in` against the installed values.

The tests that pin them: `an_installed_panel_padding_reaches_a_card_that_named_the_surface`, `an_installed_icon_button_size_reaches_the_square`, `an_installed_field_padding_reaches_a_textarea_block_inset`, `an_installed_roomy_padding_is_no_longer_a_spacing_constant`.

### The real cost of this convergence

"One copy of the authored intent, one copy of the resolved value" is not free: a node with an intent holds two `Arc<LayoutStyle>`. Each is 4808 bytes.

One density change (1000 controls) re-resolves **1000 boxes / 4.59 MB**. This is a **deliberate** one-time cost. It replaces a per-frame cost. The causality was measured directly. Not inferred: with the copy turned off and remeasured, density went from 2.306 ms back to 1.239 ms (baseline 1.239). palette-switch and accent-only copy **0 boxes**. Measured, they do return to or below baseline. The original 3× regression is gone.

**A methodology lesson to record here**: `style_allocated_bytes` **did not see this cost at all**. It under-reported by 23×. Because the cost happens inside `Arc::make_mut`. When unique it reuses in place; when shared it clones. Neither goes through any allocator counter. A counter that cannot see the largest heap event on its path makes the baseline read as healthy. `ThemeWorkCounters.layout_copies` / `layout_copied_bytes` were added as separate columns (not merged into `style_allocations`: that field has precise semantics and invariants built on it. Mixing this in would turn existing assertions into lies). A required `does_not_copy_layout` invariant was added for palette-switch / accent-only. **This is exactly the one that the original regression would have tripped while no other counter would have**: when sizes are resolved on the read path, every invalidation counter is still 0.

### Not converged yet (rest of F1)

`UI_METRICS` read lines 116 → 62 (product code, excluding tests and bins). The rest fall into three classes. Each has its own blocker:

| Remaining | Why it is not done yet |
| --- | --- |
| Motion tokens (F2) | ~~`motion_*_ms` are still dead fields~~ → **done in §7**: each of the eight durations belongs to a role, and the `motion` `const`s now read the theme instead |

The remaining literals have been connected to `space` / `type_scale` / `ControlSize` (command palette row height = Large + `space::XS` = 40. Swatch/title loading width = `PAGE_TIGHT + XXS` = 22. Sidebar tool side = `PAGE_TIGHT`). Steps that did not line up were evaluated by role. Not shifted by the nearest 1px:

| Original value | Decision | Reason |
| --- | --- | --- |
| 11px font size | **New** `type_scale::HINT` | Compact chrome captions, paired with `META` 12 (compact badge / section title / toast description) |
| 18px font size | **New** `type_scale::TITLE` | Settings page titles; 2px from both `HEADING` 16 and `DISPLAY` 20, not something one step can absorb |
| 10px font size | **Mapped to** `HINT` | Pane actions, chart/calendar labels; not a separate design from 11, so no MICRO step |
| 7px | **Mapped to** `space::SM` | Off the 2px grid (tooltip horizontal padding, toast indicator dot, compact badge horizontal) |
| 5px | **Mapped to** `space::XS` | Same (form Medium spacing, compact badge/validation gap) |
| 3px | **Mapped to** `space::XXS` | Compact badge vertical padding, kept tighter than the rest's `XS` |
| 1px row gap | **Mapped to** `space::XXS` | Hairline separator; a 1px **stroke** stays in `border_width` and is not forced onto spacing |
| 34px pane chrome | **Composed** Medium + `XXS` | Exactly 32+2, not a new height step |
| 10px shortcuts | **Mapped to** `HINT` | The command palette used to compose 10 as `META - XXS` |
| 8px unsaved dot | **Mapped to** `space::MD` | `FileTab::DOT_SIZE`; a marker, not a caption |
| 9px markdown block gap | **Component parameter** `BLOCK_GAP` | Single site, 1px off the grid |
| 54px viewer edge | **Composed** `PAGE*2 + SM` | ImageViewer surface padding |
| Settings row height 13 | **Mapped to** `BODY` | Single-line ellipsis follows the font size, not `LINE` 16 |
| 1px stroke | **`HAIRLINE`** | Not spacing |
| Calendar cells 11/3/42 | **Component parameter** `CalendarHeatmap::CELL_*` | Heatmap geometry |
| Graph port 5 | **Composed** `PORT_RADIUS + HAIRLINE` | +1 relative to idle 4 |

Tests call `height_in(UI_METRICS)`. They no longer go through the removed `ControlSize::height()` / `padding_x()`. Overloads without metrics that the product did not use (`ImageViewer::geometry()` with no arguments, `row_bounds()` without metrics) have been removed. Call sites always pass the installed metrics.

**The judgment for the next step**: the two options for the layout-input class have been compared with evidence. See §1.6. The conclusion is to push intent down (option A). The steps that can be named on the size side have been named. The only thing not yet connected is #100's Motion.

## 1.6 How layout inputs converge: comparing two options

Once radius worked end to end, the remaining bulk is **layout inputs** such as **control height / padding**. They cannot take the extract path. There are two paths to choose from. This section is a comparison made with evidence. Not a preference.

### Shared premise

Both paths require components to **stop spending tokens as pixels at construction time**. Neither is free. The real difference is "who resolves afterwards".

### Option A: intent stays on the node, resolved downstream

The same shape as radius: `NodeStyle` carries intent fields. Readers resolve them where they have access to the installed theme.

- **The hook already exists**: `UiWorld::effective_layout_style(&self, id)` is the single funnel through which layout reads a node's `LayoutStyle`. And it **already derives values** (hidden, overlay position/width/z-index). `self.style_model` is right at hand.
- **Most of the intent vocabulary already exists.** Runtime has 58 `.height()` calls. **All** derive from `ControlSize` (36 `size.height()` + 11 `Small` + 10 `Medium` + 1 `Large`; not a single one is another type). 22 `.padding_x()`. **80 call sites already speak in tiers.** The only problem is that `ControlSize::height()` resolves against `UI_METRICS` internally. And `height_in(metrics)` already exists.
- Only ~38 bare `UI_METRICS.<field>` sites need new names (panel_padding ×18, list_item_padding ×6, icon_button_size ×5, field_padding ×4…).
- **Invalidation is already right**: `apply_style_model` marks LAYOUT when metrics change. The existing LAYOUT channel reruns `effective_layout_style`.
- **Extra cost per theme change: 0.**
- **Already proven**: radius took this path. 25 sites. 0 pixel changes. 2 tests.

Cost and open question: Scene reads `ExtractedNode.source_style.layout`. It does **not** go through `effective_layout_style`. So resolution must live in a helper shared by both sides (extraction already has a precedent of patching padding).

### Option B: theme installation triggers reprojection

- **The mechanism already exists.** What I said last round, "`project` is not object-safe to call", was **wrong**: `ChildReprojectFn = fn(&mut AppContext, StableNodeId) -> Result<(), FrameworkError>` is captured at stamp time. It reprojects type-erased views through the typed `update_component` pipeline. Today components opt in through `wants_child_reproject` (6 components use it).
- **Measured cost** (1000 Buttons, release): reprojecting 1000 views takes **0.86 ms**. One metrics installation by itself (invalidating 1001 nodes + layout inputs + extract) takes **4.2–4.7 ms**. So B is **+19%** on top of the existing cost.
- **The decisive point**: those 0.86 ms produce **0** mutations (`style=0 render=0`). Because `effective_style(&self)` still reads `UI_METRICS`. **Reprojection by itself changes nothing.** So B **cannot replace** A's per-component work. It adds a mechanism and a per-change cost on top of A's work.
- **What B buys that A does not**: **conditional** decisions on metrics at projection time (switching structure or visuals by density). **Evidence: there are none today.** 0 `if`s on metric values. 0 comparisons. The only reverse mapping, `ControlSize::nearest_text(px)`, maps a font size back to a tier. It does not depend on the installed metrics.
- **Risk**: `update_component` clones the component and reruns `project`. The latter issues `set_standard_visual` / `set_accessibility` / text (all diffed, but they really run). Components that probe their retained subtree during project and memoize the result (the reason `wants_child_reproject` exists) would rerun that probe on every theme change.
- **Boundary**: it turns a theme change into a **structural event**. That conflicts with #100 §7's "precise invalidation by dependency class".

### Conclusion: choose A

In order of importance:

1. **B does not replace A. It only adds on top of A.** Reprojection produces 0 mutations. Unless components also switch to reading the installed metrics. That is exactly A's per-component work.
2. **80/118 of the intent vocabulary already exists.** The bulk of A is not inventing vocabulary. It is making the existing `ControlSize` resolve downstream.
3. **A's invalidation is ready-made with zero extra cost.** B costs +19% per change for 0 mutations.
4. **The only extra capability of B has no users today.**
5. **A has already worked once in this repository** (radius, 0 pixel changes).

### After implementing A: the answer to the first of the three questions, and the one nobody expected

**"Where does the shared resolution point go" now has an answer. And it is neither of the two options originally listed.** The original assumption was a helper shared between `effective_layout_style` and extraction. The premise was "resolve on read". Measurement overturned that premise (§1.5 "The real cost of this convergence"): `LayoutStyle` is 4808 bytes. On any read path it is amplified every frame. The final answer is **resolve once on write**. Store it in `NodeRecord.resolved_layout`. Both readers take it directly. No shared helper is needed.

This also corrects the original text of §1.6: that section treated "resolved downstream" as the definition of A. **Resolving downstream is not the same as resolving on the read path.** What A really means is "components do not spend tokens as pixels, and the intent lives until it reaches a place that has the installed theme". Whether that place is a read or a write depends on how large the resolved value is. A color is 16 bytes. Resolve on read. A box is 4.8 KB. Resolve on write.

The other two are still open:

- How to name the ~38 non-`ControlSize` metrics: merge them into `ControlSize`, or give each its own tier like `RadiusTier`.
- Whether to deprecate the convenience method `ControlSize::height()` that "resolves against the constant". As long as it stays, someone will accidentally walk back to the constant.

### When B is the right answer

When a component really needs to **change what it projects based on a metric** (a different structure at a different density). At that point, let **that one component** opt into reprojection. The mechanism is already there. Per-component opt-in is far better than making it global behavior.

## 2. Hard-coded design value inventory and migration matrix

### 2.1 How the inventory was produced

[`scripts/audit-theme-hardcoding.py`](https://github.com/sena-nana/NanaUI/blob/main/scripts/audit-theme-hardcoding.py) scans **per component**: the function bodies of each `impl ComponentView for X` count as X's own design authority. Everything else counts as "shared authority outside component bodies". It is a textual count, not a parser. It would rather over-count than under-count.

```bash
python3 scripts/audit-theme-hardcoding.py --format markdown
python3 scripts/audit-theme-hardcoding.py --check scripts/baselines/theme-hardcoding.json
```

`--check` exits with 1 when any "local design value" category grows. This is the seed of #100 Phase 9, "prevent reintroducing hard-coded default style authority": for now it guards against getting worse.

The scanning unit is **every impl a component owns**: `impl ComponentView for X` plus `impl X`. Scanning only the former misses most of it. The whole `InteractionStyle` of `ListItem` is written in `ListItem::new`. Looking only at the trait body reports it as "declares no states". The first draft of the audit really made this mistake. A test now pins it.

The classification rules themselves are tested (`scripts/tests/test_theme_hardcoding.py`, run by CI's `python3 -m unittest discover -s scripts/tests`): comments/strings/test modules are not design values. `border_radius: Some(6.0)` counts, `let width = 0.0` does not. Lines that read a token authority go into the denominator, not the numerator. Without these, "the number went up" would only teach people to re-record the baseline.

Baseline (2026-09-19, 122 components):

| Location | color_role | color_literal | design_number | motion | elevation |
| --- | ---: | ---: | ---: | ---: | ---: |
| Component's own impls | 232 | 0 | 29 | 2 | 0 |
| Shared authority outside components | 216 | 0 | 139 | 18 | 1 |
| Module-level design constants | — | — | 217 | — | — |

`color_role` is **intent**. It moves into recipes. `design_number` / `motion` are the part that **must disappear**.

When reading `design_number`, know that it mixes two kinds of things: values that are really tokenizable (font size, font weight, spacing, radius). And structural switches (`border_width: Some(0.0)`, `opacity: 1.0`, `border_radius: Some(999.0)` — "off / pill" sentinels). The latter do not disappear just because `ThemeDefinition` exists. So the target for this number is not 0.

This round also cleaned up 13 of them that "copied a token's value as a literal" (`Some(500)`→`type_scale::MEDIUM`, `6.0`→`space::SM`, `13.0`→`type_scale::BODY`, and so on): the values are identical. Zero pixel change. But from now on they are greppable token reads instead of invisible numbers. In-component `design_number` dropped from 42 to 29 as a result.

### 2.2 Migration matrix

The "Current authority" column uses abbreviations: **V**=`view_components.rs` component body. **X**=`world/extraction.rs`. **G**=`world/geometry.rs`. **S**=`nana-ui-scene` primitives. **C**=module-level `const`. **M**=`nana_ui_core::motion` const.

| Component | Current authority | Tokenizable | Recipe needed | Motion needed | Layout impact |
| --- | --- | --- | --- | --- | --- |
| Button | V (20 roles, hover/pressed/focused) + X + S(radius) | color ✓ / radius ✗(F1) / padding ✓ | **Yes**: full variant×size×state matrix | hover color (M) | size→height/padding |
| IconButton | V (14 roles, 5 states) + S | Same + icon size | **Yes** | hover color (M) | size→square side length |
| Input / TextArea | V (focus/hover border) + X + G(text area) + S | color ✓ / field padding ✓ / radius ✗ | **Yes**: including invalid / read-only | focus ring | size→height, multiline follows content |
| Checkbox / Radio / Switch | V (6 states on Checkbox) + S(indicator geometry) | indicator size / gap are in `ControlSize`, read from constants | **Yes**: checked×hover×disabled | switch toggle | indicator affects row height |
| Slider / Range | V (disabled) + G(track/thumb) + S | track/thumb sizes are all bare numbers in S | **Yes** | None | Fixed height |
| Card / Surface | V (4 roles) + X + S(elevation) | color ✓ / radius ✗ / elevation only 1 enum site | **Yes**: `CardKind` is already a variant | None | padding |
| ListItem | X (no `InteractionStyle`) + S | selection/hover color ✓ | **Yes**: generic selection row recipe | None | `list_item_padding_x` |
| Tabs / Segmented | X + G + S | indicator geometry in S | **Yes** | indicator slide | segment width |
| Menu / ContextMenu | `menus.rs` (14 roles) + S(overlays.rs radius) | color ✓ / radius ✗ / padding ✗ | **Yes** | `MENU_OPACITY`/`MENU_POP` (M) | menu item height |
| Tooltip / Dialog / Popover | `TooltipConfig` constants + `popover.rs` (10 roles) + S | tooltip padding/radius/font are `const`s on `TooltipConfig` | **Yes** | `OVERLAY_FADE` (M) | overlay size |
| Scrollbar | `ThemeMetrics::scrollbar` + `ScrollbarRecipe` + G + S | Sizes in metrics; color, motion, and visibility policy in the recipe (resolved, see §7.9.1) | **Yes** | Fade in/out, thickening (`ScrollbarMotion`) | None: overlay, takes no layout space |
| Window / Titlebar / Sidebar | `AppTitleBar` + `sidebar.rs` + `AppearanceSettings` (backdrop) | titlebar is already its own role; backdrop alpha is already policy | **Yes** | `SIDEBAR_COLLAPSE` (M) | titlebar height, sidebar width |
| Tree / Table / List selection row | `tree.rs` / `virtual_table.rs` + X + S | selection three-state colors ✓ | **Yes**: shared with ListItem | None | row height |

How to read it: cells with **Tokenizable ✗** are where F1 concretely lands. Those values read constants today. Installing a `ThemeDefinition` would not affect them. **Recipe needed being "Yes" everywhere** is not a shortcut: the first batch of recipes named in #100 §3 is exactly this table. Phase 0's conclusion is that no row can be skipped.

## 3. Component Gallery baseline

Two baselines. Both in [`examples/component-gallery/snapshots/`](https://github.com/sena-nana/NanaUI/tree/main/examples/component-gallery/snapshots).

### 3.1 Pixel baseline (existing)

`snapshots/<adapter-key>/component-migration/<component>/<dark|light>/<state>.png`: 598 images for 78 components. Plus 84 full-window snapshots of shell / dock / titlebar and others. 682 committed images in total. One complete run renders all of them. Zero tolerance. Split into directories by GPU adapter. The rules are in that directory's [README](https://github.com/sena-nana/NanaUI/blob/main/examples/component-gallery/snapshots/README.md). That page now also records **how to read a case where one baseline disagrees with another baseline**. That is more common and harder to notice than a single image disagreeing with what is committed.

```bash
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked
```

Its limitation is exactly what #101 §3 needs to fill in: only the machine that recorded it can verify it.

### 3.2 Semantic baseline (new this round)

`snapshots/semantic/component-migration/<component>/<dark|light>.txt`. 156 files. **Not split by adapter.** Not one pixel in them is rasterized.

Each fixture records: the target node's layout box, accessibility state (role / disabled / checked / selected / mixed), and the resolved result of every `ScenePrimitive` in that frame in scene order. Background, stroke, border width, the four corner radii, shadow, text color/font size/font weight/line height/letter spacing/italic/underline, opacity.

```bash
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic --bless
```

It runs in CI: the step `cargo test -p component-gallery --bin ui-snapshots --features snapshots` was always "the adapter-independent half". The semantic baseline is exactly that half. The pixel gate is skipped on every hosted runner. Without this, nobody would look at the state matrix in CI.

**It depends on reproducible shaping. Not on rasterization.** The geometry in it (including text box widths) comes from `nana-text` layout. Not from the GPU. The same bundled fonts + a locked shaper give the same numbers on all three platforms. `crates/nana-text/corpus/golden/` has long committed layout results precise to `advance_px: 11.664` as cross-platform golden data and verified them on ubuntu/macos/windows. So if one platform fails on its own, it means **shaping changed** (a different font, a different shaper version, a different feature combination). Not "floating-point noise". Do not paper over it with bless.

A record looks like this:

```text
state: primary
  target: 20.00,20.00 81.67x32.00
  a11y: role=Button disabled=false checked=None selected=None mixed=false
  quad #2/0 20.00,20.00 81.67x32.00 opacity=1.00 bg=#7bb9f024 border=none border_width=0.00 radius=6.00/6.00/6.00/6.00 shadow=none
  text #2/2 31.00,21.00 59.67x30.00 opacity=1.00 color=#7bb9f0ff size=13.00 weight=500 ...
```

This is the readable form of "what the Theme resolved to": change one token step and the diff says directly which role moved. Instead of "15 images, max_channel_delta=1".

Both are kept: semantic snapshots cannot see rasterization bugs. Pixel snapshots cannot say **why** a color changed.

### 3.3 State matrix coverage

The criterion is the component's own code. Not impressions: a state "should have a fixture" if and only if the component declares paint for it in `InteractionStyle`. The first draft of the audit eyeballed it per family. It found 32 states that were declared but never captured. This round filled in 29 of them.

After filling them in (the `--check` criterion is "every declared state has a fixture"):

| Component | States added this round |
| --- | --- |
| `checkbox` | selected-hover, selected-pressed |
| `chip` | hover, pressed |
| `dropdown` | hover, focused, disabled |
| `icon-button` | selected-hover, selected-pressed |
| `interactive-card` | hover, pressed, disabled, selected-hover, selected-pressed |
| `search-dropdown` | hover, focused, disabled |
| `settings-collapsible-card` | hover, pressed |
| `sidebar-row` | hover, pressed, focused, disabled, selected-hover, selected-pressed |
| `sidebar-section` | focused |
| `textarea` | hover |
| `xy-pad` | hover, focused |

29 fixtures × light/dark = 58 new pixel keys + 58 new semantic baseline sections. The semantic baseline was recorded and verified this round. The pixels are new keys (not "pixels changed"). They were recorded on 2026-09-20.

**Every section was verified to really enter its state.** The method is a byte-for-byte comparison against the semantic baseline: different from every other state of that component. Or, when identical, with a clear explanation of why.

At the time this comparison was only spot-checked per family. It missed something it could have settled in one go. Later, all 615 baselines were **grouped by md5** and examined again. The conclusion is much bigger: **of the 14 components with a `focused` fixture, 11 rendered focus byte-for-byte identical to some other state**. `button` / `icon-button` / `sidebar-section` matched the rest state (nothing was drawn). `dropdown` / `search-dropdown` / `xy-pad` matched hover. `tabs` matched selected. `sidebar-row` wrote `border: Accent` but paired it with `border_width: 0`. This is not a Phase 4 to-do. It has already been fixed on this branch: focus values moved into `AccentRamp.focus` and three semantic roles. They appear only on keyboard focus (editable text fields are the exception: they also show it when clicked into; see `UiWorld::focus_visible` and `SemanticPaint::FOCUS_FIELD`). It is guarded by
`a_focused_fixture_never_looks_like_a_state_the_keyboard_did_not_cause` (run against the pre-fix baseline, it reports 18 collisions).

**The method is more worth keeping than this conclusion**: group the baselines by content and see which "different states" are actually the same image. It needs no adapter and no build. One pass over the files. And every real defect in this tree eventually surfaced this way.

What was **not** filled in, and why:

| Component / state | Why it was not added |
| --- | --- |
| disabled / focused of `settings-collapsible-card` | Both are on the card's summary row, and the a11y of the card root still reports `disabled=false`. With the fixture mounted as it is now, these two states cannot be captured; the result would be a fake baseline of "the rest state posing as a state" |
| pressed of `sidebar-section` | The section declares pressed itself, but both its header and body rows consume the press first; no hit point in this fixture resolves to the section itself |
| selected of `CommandPalette` | **Already covered**: the first row of the `open` fixture is `bg=#353535` (`palette.selected`); the fixture just is not named that |
| `HoverCard` / `NumberInput` | Not in `component_catalog()`; adding fixtures first requires making them first-class catalog components — that is a product API change, not a baseline |
| `Scrollbar` | Same: `ScrollView` is not a catalog component, and the whole family has no fixture directory |

The overlay and chrome families (`tooltip` / `dialog` / `popover` / `context-menu` / `tree-view`) still have only single-state fixtures. But this is not a gap: in code they **do not declare** interaction states. The states belong to their inner rows/buttons.

## 4. Theme / Style performance baseline

### 4.1 counters

New [`ThemeWorkCounters`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui-core/src/work.rs). Published by `UiWorld::last_theme_work_counters()`. It is not `WorkCounters`: the latter's `style_processed` only says "how many nodes this drain queued". The Theme baseline needs what happened **inside** that pass.

| counter | Meaning |
| --- | --- |
| `style_nodes_considered` | Nodes the pass actually evaluated (including skipped ones). Nodes on the parent chain are counted once per pass |
| `style_nodes_resolved` | Of those, the ones that published a new resolved style |
| `style_nodes_skipped` | Of those, the ones whose value and theme generation were both unchanged and returned on the fast path |
| `theme_reads` | Number of reads against the token authority (`StyleModelRef`). One mix counts as one |
| `style_allocations` / `style_allocated_bytes` | Heap events caused by the style path itself: one per published resolved style, one for the invalidation list of each theme installation |
| `layout_copies` / `layout_copied_bytes` | `LayoutStyle` copies made so that "the resolved value" and "the layout the component wrote" can coexist. **A separate column, not merged into `style_allocations`**: it is a different event with a different cause, an order of magnitude larger, and invisible to any allocator counter (it happens inside `Arc::make_mut`). Must be 0 for a color-only theme change |
| `layout_nodes_from_style` | Nodes the theme/style authority decided need relayout |
| `text_nodes_from_style` | Nodes whose shaping or line constraints were invalidated by a resolved style change (**pure color changes do not count**, otherwise a palette switch would read as a reshape) |
| `paint_nodes_from_style` | Nodes the theme/style authority decided need repaint |

Invariant: `considered == resolved + skipped` (the extractor checks it and rejects the report outright if it does not hold).

A theme change is more than one event (install → drain → style pass). So measure between `begin_frame_counters` / `end_frame_counters`. Outside that frame, each event overwrites the previous one. Same behavior as `last_text_work_counters`.

### 4.2 Workloads and baseline numbers

```bash
cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-theme-benchmark -- --output target/performance/issue101/theme.json
python3 perf/runners/nana/run.py --scenario theme-palette-switch --from-report target/performance/issue101/theme.json
```

`perf/scenarios/theme-*.json` has 11 rows. They are in `nana_theme_ids` of `catalog.json`. **Not** in `harness_ids`. The criterion is work counts. Not timing in public CI.

The extractor and these gates themselves are covered by `theme_baseline_tests` in `perf/contract.py --self-test` (CI already runs it): it uses synthetic reports to prove, one by one, that each gate can fail on its own. And it rejects reports with mismatched workloads, scales not echoed back, `considered != resolved + skipped`, or missing counters. Real reports cannot test these negative cases.

Baseline (release; 1k controls = 1000 product `Button`s + one column container):

| id | nodes | considered | resolved | skipped | theme_reads | style_alloc | layout_copies | layout_from | text_from | paint_from | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `theme-controls-1` | 2 | 2 | 2 | 0 | 2 | 2 | 0 | 0 | 1 | 0 | 0.008 |
| `theme-controls-100` | 101 | 101 | 101 | 0 | 101 | 101 | 0 | 0 | 100 | 0 | 0.377 |
| `theme-controls-1k` | 1001 | 1001 | 1001 | 0 | 1001 | 1001 | 0 | 0 | 1000 | 0 | 4.311 |
| `theme-controls-10k` | 10001 | 10001 | 10001 | 0 | 10001 | 10001 | 0 | 0 | 10000 | 0 | 66.345 |
| `theme-static-idle` | 1001 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0.000 |
| `theme-hover-one` | 1001 | 2 | 0 | 2 | 6 | 0 | 0 | 0 | 0 | 1 | 0.004 |
| `theme-focus-one` | 1001 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0.004 |
| `theme-palette-switch` | 1001 | 0 | 0 | 0 | 1001 | 1 | 0 | 0 | 0 | 1001 | 1.316 |
| `theme-accent-only` | 1001 | 0 | 0 | 0 | 1001 | 1 | 0 | 0 | 0 | 1001 | 0.817 |
| `theme-density` | 1001 | 0 | 0 | 0 | 1001 | 1 | 1000 | 1001 | 0 | 1001 | 2.681 |
| `theme-head-style-mutation` | 10000 | 10000 | 10000 | 0 | 1 | 1 | 0 | 0 | 0 | 0 | 14.154 |

**How to read `p50 ms`: only as an A/B within one session. Never compared across recordings.** The counter columns are deterministic. The 4 runs folded into this archive are identical field by field. On another machine too. `elapsed_ms` is not: the same binary on this machine swings `theme-head-style-mutation` between 7.7 ms and 14.2 ms depending on whether something is compiling alongside. Close to 2×. The values in the table are the per-row minimum of the 4 runs. For order-of-magnitude reference only. Any "it got faster/slower" conclusion must come from an A/B within one sitting. Every performance conclusion on this page was reached that way.

The raw reports are archived in [`archive/docs-notes/performance-data/theme-audit-2026-09-19/`](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/performance-data/theme-audit-2026-09-19).

### 4.3 What the numbers say

- **static idle is all zeros.** A stable frame does not walk, does not read tokens, and does not allocate. The zero-work fast path of #100 §15 **already holds**.
- **palette-only / accent-only: `layout_from = 0`, `text_from = 0`. `paint_from = 1001`.** A color switch triggers no layout and no reshape. That is correct. Whole-document paint invalidation is F4. It is the number #100 §7 wants to narrow.
- **density: `layout_from = 1001`, `layout_copies = 1000`.** A metrics change correctly escalates to LAYOUT. And **this is no longer half true**: the 1000 box re-resolutions mean the heights of those 1000 controls really follow the installed metrics (before this round their sizes were spent as pixels at construction time, and after the invalidation ran the sizes stayed the same). The cost is in the same number. 4.59 MB. One-time. It replaces a per-frame cost.
- **palette-only / accent-only: `layout_copies = 0`.** Touching only colors must not touch boxes. This is now a required invariant. And it is the **only** one that catches "resolving sizes on the read path": that approach rebuilds every control's `LayoutStyle` every frame, while every invalidation counter above stays at 0.
- **`considered = 0` while `theme_reads = 1001`.** This pair of numbers is F3's fingerprint: no style pass, yet tokens were read 1001 times. It happens in extract.
- **hover one: `considered = 2`, `resolved = 0`.** Only the target node and its parent were walked. Both took the fast path. `resolved = 0` because the hover transition starts at progress 0. The first frame's resolved value equals the original. The color is advanced by subsequent animation frames.
- **focus one: all zeros.** Focus rings do not go through the theme/style path at all today. Extraction/scene draws them directly. This explains F6.
- **control scale is linear.** For 1/100/1k/10k, considered, theme_reads, and style_allocations all equal the node count: resolving a document for the first time = one pass. `text_from` equals the control count because the first resolution changes the font from the default to the control's value.
- **head scope: 10000 considered, `theme_reads = 1`.** One inherited color change at the head re-resolves the whole subtree. But the theme was asked only once. The inheritance cache works. This is the real cost of one scope patch before ThemeScope exists.

## 5. Boundaries

The constraints of #100, checked one by one against the current state.

```text
Tokens + Semantics + Layout = existing Style Model    ← holds
ThemeDefinition = design-system authority for Tokens/recipes  ← not established (Phase 1)    → established in §7
Theme Resolver  = retained StyleSystem work            ← does not hold: resolution is in extract (F3)
ResolvedStyle   = downstream contract for Layout/Text/Paint  ← partial: ComputedStyle covers only inheritance and color
Renderer       != Theme resolver                       ← holds (F8), except scene reading UI_METRICS (F1)
Motion         != Theme runtime                        ← holds: Motion IR is the only executor; but the Theme has no transition policy either (F2) → §7 added the policy, there is still only one executor
CSSOM          != Nana core                            ← holds: CSS parsing lives only in nana-ui-css (used at build time by the `.vue` compiler, `view!`, and `css!`)
```

Layer responsibilities, as confirmed this round:

| Layer | Responsible for | Not responsible for |
| --- | --- | --- |
| **Theme** | Design targets: token values, component recipes, transition policy | Per-frame evaluation, animation clock, layout algorithm |
| **Semantics** | Component intent: kind / variant / size / status / interaction state | Concrete RGB, concrete pixels |
| **Layout** | Box model and geometry: flex/gap/padding/sizes | Colors, state layering |
| **Motion** | Transitioning from the current presented state to the target state (#87 Motion IR) | What the target state is |
| **Text** | Shaping and line layout (#88 nana-text) | Text color (that is Paint) |
| **Renderer** | Consuming resolved data and drawing it | Understanding token names, recipes, scopes |

Two easily violated details, made explicit this round:

1. **A color change is not text work.** `text_nodes_from_style` counts only when `TextDirty::work()` includes SHAPE or LAYOUT. Color goes through `TextWork::SCENE_PAINT`. This classification is answered in one place, `TextDirty::work()`. The style path does not keep a second copy of the mapping.
2. **`SemanticPalette` / `ThemeMetrics` are the basis to evolve, not something to copy.** Phase 1 must not produce `ThemeColors2` / `ThemeMetrics2`. Conversely, `nana_ui::theme::Colors` (F9) is a second copy that **already exists**. Phase 1 must retire it, not add another. (§7's `DesignTokens` holds these two types themselves by value. There is no third spelling.)

## 6. Reproduction

```bash
# Types and data flow (the F1 / F8 numbers on this page)
rg -c "\bUI_METRICS\b" -g '!*tests.rs' -g '!*/bin/*' crates/nana-ui-runtime/src crates/nana-ui-scene/src crates/nana-ui/src
rg -c "style_model\.metrics|theme_metrics\(\)" -g '!*tests.rs' -g '!*/bin/*' crates/nana-ui-runtime/src crates/nana-ui-scene/src crates/nana-ui/src
rg -c "SemanticColorRole::" crates/nana-ui-scene/src crates/nana-ui/src/scene_paint*

# Hard-coded inventory
python3 scripts/audit-theme-hardcoding.py --format markdown
python3 scripts/audit-theme-hardcoding.py --check scripts/baselines/theme-hardcoding.json

# Gallery baseline
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked   # needs an adapter with a recorded baseline

# Performance baseline
cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-theme-benchmark -- --output target/performance/issue101/theme.json
python3 perf/contract.py --self-test
for id in theme-static-idle theme-controls-1k theme-palette-switch theme-density theme-head-style-mutation; do
  python3 perf/runners/nana/run.py --scenario "$id" --from-report target/performance/issue101/theme.json
done

# Counter behavior
cargo test -p nana-ui-core --lib work::
cargo test -p nana-ui-runtime --lib world::tests::

# §1.5 convergence: the installed theme really reaches controls and chrome
cargo test -p nana-ui-runtime --lib an_installed_radius_reaches_a_control_that_named_the_tier
cargo test -p nana-ui-runtime --lib a_thicker_scrollbar_in_the_installed_theme_reaches_the_bar
cargo test -p nana-ui-scene --lib a_menu_surface_paints_the_radius_it_was_handed_not_the_constant

# The cost of intent resolution is visible to counters (F12)
cargo test -p nana-ui-runtime --lib resolving_layout_intent_reports_the_layout_it_had_to_copy
# The size premise the resolution-point decision depends on
cargo test -p nana-ui-runtime --test layout_style_size

# The renderer no longer reads theme constants (should be 0)
rg -c "UI_METRICS" -g '!*tests.rs' crates/nana-ui-scene/src

# The gates and classification rules themselves
python3 -m unittest discover -s scripts/tests
cargo test -p component-gallery --bin ui-snapshots --features snapshots --locked
```

This round was verified on Windows + Vulkan (RTX 5060). Counters and scenario gates all green. Semantic baseline 146/146 MATCH. The pixel suite recorded and re-verified 557/557 under a temporary adapter key. The `metal-apple-m4` baseline in the committed tree **cannot** be verified on that machine. That is exactly why the semantic baseline exists.

## 7. Phase 1: ThemeDefinition and the typed token system

This section tells you what [#102](https://github.com/sena-nana/NanaUI/issues/102) delivered. Phase 0 concluded that "the token authority is scattered across constants, and installing a theme has no effect". Phase 1 builds the **thing that gets installed**. A versioned, strongly typed, validatable `ThemeDefinition`.

**Nor did it change a single existing pixel.** The evidence is not "we ran the snapshots and nothing looked wrong". It is three independent checks:

1. **Byte-for-byte semantic baseline comparison.** On the same machine, through the same binary path. All 146 fixtures recorded once against clean HEAD and once against this round (666 output files, baseline copies included). `diff -rq` reported **0 files different**.
2. **An equality assertion on the compiled result.** `the_built_in_definitions_compile_to_exactly_the_tokens_already_rendered` asserts that the `SemanticPalette` and `ThemeMetrics` produced by `ThemeDefinition::NANA_DARK.compile()` are **exactly equal** to the ones the tree renders with today. When equality holds. No fixture has room to move. A migration that has to re-record 615 images to prove it is safe has proved nothing.
3. **Field-by-field work counter comparison.** Across 11 theme scenarios, **all 11 counters are field-for-field identical** to the Phase 0 archive (see §7.7).

> ⚠️ **The semantic baseline already disagreed with HEAD before this round.** Running `--semantic` on clean HEAD reported **46 fixtures CHANGED**. `cargo test -p component-gallery --bin ui-snapshots` was therefore already red on main. Unrelated to #102. Check 1 above was done that way precisely to keep the two things apart. Those 46 were re-recorded in a later round after each cause was traced (now 146/146 MATCH). Three of the classes turned out to be regressions, not constant convergence. The pixel suite was brought up to 615/615 in the same round.

### 7.1 The type landscape

```text
                     ThemeDefinition                            ← authored, diffable
                     ├─ id / schema / generation
                     ├─ mode
                     ├─ tokens: DesignTokens
                     │  ├─ foundation: FoundationTokens         ← author-private, components cannot reach it
                     │  │  └─ accent: AccentRamp
                     │  ├─ palette:  SemanticPalette            ← the same type as before
                     │  ├─ metrics:  ThemeMetrics               ← the same type as before
                     │  ├─ spacing:  SpacingTokens
                     │  ├─ border:   BorderTokens
                     │  ├─ opacity:  OpacityTokens
                     │  └─ titlebar: Option<SemanticColor>
                     ├─ typography: TypographyTokens
                     ├─ motion:     MotionTokens
                     ├─ effects:    EffectTokens
                     ├─ surfaces:   SurfaceTokens
                     └─ components: ComponentThemeRegistry      ← slots are Option
                                │
                                │  compile()                    ← validate + degrade, all-or-nothing install
                                ▼
                     CompiledTheme                              ← read at runtime, slots are no longer Option
                     ├─ identity: ThemeIdentity
                     ├─ style_model: StyleModelRef              ← the hot-path slice
                     ├─ spacing / border / typography / motion / effects / surfaces
                     └─ recipes: CompiledRecipes                ← fixed-length arrays, enum indices
                                │
                                ▼
                     UiWorld.theme: Arc<CompiledTheme>
                     UiWorld.style_model: StyleModelRef         ← the cached projection of the one above
```

`DesignTokens` is not called `ThemeTokens`: that one is `nana_ui::theme::ThemeTokens`, the host-side installation bundle. **One name, one type.** That is the direct consequence of the F9 lesson.

### 7.2 Why authoring and compiled are two types

Two reasons. Neither is fastidiousness:

**fail-closed.** A `ComponentThemeRegistry` slot is `Option`. A `CompiledRecipes` slot is not. The author misses one family. `compile()` returns `MissingRecipe { component, slot }`. The whole theme fails to install. Rather than quietly falling back to `Text` in that one place, to be found three screens later when some corner renders the wrong color. Validation covers: incompatible schema, generation 0, non-finite values, negative lengths, alpha out of range, font size out of range, font weight out of range, zero duration, missing recipe slot. Each one has a test that starts from a theme that compiles and touches exactly one field.

**The hot path knows no strings.** Names live only on the authoring side (`ThemeId`, the token names in errors). What crosses into the runtime is enum-indexed fixed-length arrays. `theme.get("button.primary.background")` does not exist as a type.

### 7.3 Why `StyleModelRef` did not become the theme handle

`CompiledTheme` is about 1 KB. It is **not `Copy`**. It hangs off an `Arc`. `StyleModelRef` is still the small, by-value read handle. This round only added the 24 bytes of `OpacityTokens` to it (needed in §7.5 below).

This copies the §1.5 lesson: 4808 bytes of `LayoutStyle` on the read path get multiplied by every frame. palette-switch measured +196%. "Make the whole theme `Copy` and hand it to every reader" is the same shape. Only this time it is 1 KB. So `CompiledTheme` also does **not** precompute a color table for the 39 roles: resolving a role is one `match` plus at most one alpha substitution anyway. A table would carry ~600 extra bytes. It buys nothing. A cache has to be worth the cost it removes.

### 7.4 Foundation → Semantic: only where something is really derived

#100 §2 asks for a foundation layer. This round provides **only** the accent family. Because that is the one place in the palette where something is really derived: `accent_soft` / `accent_soft_hover` / `accent_soft_pressed` are three alphas of the base accent. They used to be three literal RGBA constants.

The consequence is that a real bug is gone: setting only `palette.accent` (the `theme-accent-only` scenario, `--nana-custom-accent`) used to leave the three soft fills from the previous hue. The derivation rule existed only in whoever picked those constants back then. Now `ThemeDefinition::with_accent(ramp)` moves the whole family at once; the focus (`focus_*`) and highlight (`highlight*`) roles move to the new hue too if they were following the old accent, while colors the theme sets separately stay put. When `RuntimeProgram::theme()` returns the new `Arc`, the host installs it into every open window's document. No per-window `set_theme_tokens` call.

Conversely. `with_palette(p)` still writes `p` verbatim. It just **re-reads the foundation ramp from `p` on the way** (`AccentRamp::from_palette`). Records. Does not correct. A palette whose `accent_soft` and `accent` hues disagree. The ramp that comes out says so.

The four colors besides `base` are not computed: a foreground that reads clearly on the accent is a design decision, not a formula. Light mode is the evidence (`on_soft` is darker than `base`, `text` is white).

Components cannot reach the foundation: there is no path from a component to `FoundationTokens`. That is how "must not depend on `blue500`" is enforced.

### 7.5 The tokens wired up this round, and their consumers

**A token with no consumer is the next F2.** Every family has a real reader:

| Family | Type | Who reads it | What it was before being wired |
| --- | --- | --- | --- |
| Color | `SemanticPalette` | The whole tree | Same (held directly, no second copy) |
| Control metrics | `ThemeMetrics` | The whole tree | Same |
| Corner shape | `CornerShape` (`ThemeMetrics.corner_shape`) | The painter reads it once per frame from `UiScene::corner_shape` and writes it into the uniform of each quad, mesh, icon, text, backdrop, group and HostTexture; corner distances, strokes, shadows and rounded clipping are all computed from it | None: corner radii were arcs only |
| Spacing | `SpacingTokens` | `space::*` reads its `DEFAULT` | 10 bare `const`s |
| Typography | `TypographyTokens` | `type_scale::*` / `UI_BASE_TEXT_SIZE` read its `DEFAULT` | 13 bare `const`s |
| Border | `BorderTokens` | `HAIRLINE` reads its `DEFAULT` | 1 bare `const` |
| Opacity | `OpacityTokens` | `StyleModelRef::color` resolves five soft roles; `secondary_text` is the α of a list row's note/detail relative to the main text | Literals inside `SemanticPalette::get` + a `background.r > 0.5` luminance sniff |
| Motion | `MotionTokens` | The durations and hold fraction of the hover cross-fade, the switch throw and the settings row flash (`attention_ms` / `attention_hold`, the latter must be in `0.0..=1.0` at compile time) read the **installed value**; the eight `motion::*` `const`s read `DEFAULT` | Eight bare `const`s + two dead fields |
| Effect / Elevation | `EffectTokens` | The menu/overlay shadow, the modal shadow, the image viewer's media scrim (`media_scrim`, dark in both modes; the painter composites in linear light, so the built-in value is black 0.99, about `#1a1a1a` on white), the foreground stacked on media (`media_foreground`, white in both modes, read through `SemanticColorRole::OnMedia`), and the dialog and drawer scrims (`modal_scrim` black 0.45, `modal_scrim_blur` 0; writing a CSS-style `rgba(0,0,0,c)` through `linear_scrim_alpha(c)` converts it to a linear-light α) read the **installed value** | The `match mode` in `surface_shadow` + the modal's luminance sniff; the scrim was a hard-coded `[0, 0, 0, 0.45]` in the scene |
| Surface / material | `SurfaceTokens` | `ThemeTokens::with_backdrop` decides which role the backdrop puts alpha on | A `match target` hard-coded in the host adapter |
| Focus | `AccentRamp.focus` + `FocusSurface` / `FocusBorder` / `FocusText` | The `InteractionStyle::focused` of 11 components, plus the radio focus ring color | Three components wrote `border: Accent` with a zero-width border (invisible), three used the hover value, one used the selected value |
| Component recipe | `ComponentThemeRegistry` | The extraction family foreground table, `Button`'s variant×state table, the status tone table | A 25-arm `match StandardVisual` + five inline tables in `Button::project` |

Direction is the whole point: **`const`s read the token. Not the token reading a `const`.** F2 could not be fixed. Precisely because the direction was reversed. The duration was the authority. The theme just carried two fields nobody read.

Two luminance sniffs are gone (the warning soft alpha, the modal shadow). They were never "convenient". They were wrong: a theme whose background is mid-gray picks the opposite branch. And no theme could change that number.

### 7.6 Component recipe: two tables, two resolution points

Only two tables moved. Both were chosen to resolve where **the installed theme is already at hand**:

**The family foreground table** (the 25 arms in `world/extraction.rs`) → `ComponentRecipe`. It resolves on every extract. So a theme that changes the recipe reaches live nodes **without reprojection**. Test: `an_installed_family_recipe_reaches_a_live_node_without_reprojecting_it`.

**Button's variant×state table** (five inline `match self.kind` in `Button::project`) → `ButtonRecipe`. This one resolves **at projection time**. Because what it decides is which `SemanticColorRole` the component writes onto its own node. Only the component can write that field. Nothing downstream can. So `Button` opts into a new hook, `wants_recipe_reproject()`.

This looks like it contradicts "choose A, not B" in §1.6. It is actually the opening §1.6 left itself:

> When B is the right answer: when some component really needs **to change what it projects based on a metric**. Then let **that one component** opt into reprojection.

§1.6 rejected B because "reprojection produces 0 mutations, since `effective_style` still reads `UI_METRICS`". The recipe makes that no longer true: reprojection really does write different roles. And the hook is **separate** from `wants_metrics_reproject`. Not merged. Merging would make a palette-only switch needlessly reproject a batch of components. And that is exactly the common scenario where §4 measured 0 reprojections. Two tests pin it down: removing `Button`'s opt-in. `an_installed_button_recipe_reaches_a_button_that_is_already_on_screen` goes red (verified).

`invalid` is the first **stacked** state this round rather than a mutually exclusive one: an invalid primary button keeps the primary fill. Plus a danger border on top. #100 §4 requires the recipe to say which states stack. This is the first one.

`ComponentRecipeId` has 14 families. Several of them resolve to `Text` today. They still each occupy a slot: the point of the registry is that a theme can move one family without disturbing the others.

### 7.7 The cost

The work counters are **field-for-field identical** to the Phase 0 archive. 11 scenarios × 11 items:

```bash
cargo run --release --locked -p nana-ui-runtime --features benchmark \
  --bin nana-theme-benchmark -- --output target/performance/issue102/theme.json
```

The archive is at [`archive/docs-notes/performance-data/theme-phase1-2026-09-20/`](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/performance-data/theme-phase1-2026-09-20). Highlights:

- `layout_copies` for `theme-palette-switch` / `theme-accent-only` is still **0**. `layout_nodes_from_style` is still **0**. Touching only colors must not touch boxes. The new tokens did not break this.
- Neither scenario **triggers** any reprojection either: the recipe did not change. `reproject_recipe_views` does not run.
- `theme-static-idle` is all zeros: a stable frame does not walk, does not read tokens, does not allocate.
- `theme-density` still has `layout_copies = 1000`.

One theme installation now does two more things: `compile()` (validation of about fifty floats + building a fixed-length recipe array) and a whole-value comparison of `*self.theme == *next` (about 1 KB). Against the 1001 node invalidations the same installation already performs. It does not amount to a line at that scale.

### 7.8 What changed in the hard-coded inventory

The same ruler (`scripts/audit-theme-hardcoding.py`). Run once against clean HEAD and once against this round:

| | HEAD | Phase 1 | Note |
| --- | ---: | ---: | --- |
| A component's own `color_role` | 232 | **208** | Button's 24 role decisions moved into the theme |
| `color_role` outside components | 216 | **211** | The extraction family table moved into the theme |
| `design_number` outside components | 87 | **81** | Two sets of shadow literals + two luminance sniffs |
| `motion` outside components | 18 | **16** | hover / switch read the installed value |
| `elevation` outside components | 1 | **0** | |
| `token_read` (denominator) | 681 | **683** | |

**The scanner changed in two places this round too. Otherwise this table would lie.** `TOKEN_READ` gained the new naming forms `MotionRole::` / `ElevationRole::` / `ComponentRecipeId::` / `recipes()`. The reason is word for word the one for adding `RadiusTier::` back then: a component going from `motion::HOVER_COLOR` to `MotionRole::HoverColor` is **reading** a token. Without it the denominator would shrink and the migration would read as a regression. `ELEVATION` excludes `ComponentElevation::from_shadow`. That consumes a shadow the theme already resolved. The opposite of a component picking a shadow itself. Without the exclusion the number would rise as the migration succeeds. Both have tests (`scripts/tests/test_theme_hardcoding.py`).

The `--check` baseline now points at the Phase 1 archive: gating against a number the tree passed long ago. Which means the gate has slack.

### 7.9 Not done, and why

| Item | Status |
| --- | --- |
| ThemeScope / subtree override | Explicitly out of scope for #102 |
| Migrating all components to recipes | Explicitly out of scope for #102. Button + the family foreground table + the status tone table moved; the other 11 families' recipes have only the foreground slot |
| Theme package / file-format loading | Out of scope. At this stage the application API registers themes; `ThemeId` already supports an owned string, and a file theme-package format is designed separately |
| F3 (the resolution point is in extract, not retained) | Untouched. That is #100 §6, and what changes is the shape of the resolver, not the shape of the tokens |
| F4 (installation = whole-document invalidation) | Untouched. That is the dependency class of #100 §7 |
| Convergence of the typography / spacing call sites | Not done. The contract exists, but things like `ControlSize::text_size()` still resolve against `type_scale` constants — the same shape of work Phase 0 did for metrics, just a different family, left to #107 |
| The focus ring's 2px stroke and 4px outset | **Now part of the theme**: `ThemeMetrics::focus_ring` (`FocusRingMetrics { width, offset }`, built in `FOCUS_RING_METRICS` = 2px stroke, 1px off the control). The scene reads it from the installed theme before applying a delta (`UiScene::set_focus_ring`), and all five focus rings (button, text field, list row, option, slider thumb) draw from it; the option's ring sits 1px further out than the rest (its indicator is in a larger box), and the slider thumb's ring scales by the thumb's stroke unit. Paint only, no layout: `same_layout` ignores it, so changing it does not relayout. Its length is validated like the other metrics, and a negative value is rejected at install |
| The `sidebar-section` focus background covers the whole 200x86 block | The focusable node is the whole section, so this renders it faithfully. Highlighting only the header would mean moving the focusable node from the section root to the header — a behavior change (focus order, hit testing), not an indicator change |

### 7.9.1 Follow-ups after §7 (consumer-driven)

Things an L3 consumer (NanaLive) ran into when installing a complete `ThemeDefinition`. All holes §7 left behind rather than new requirements. So they were filled in place. Without waiting for #105 / #110:

| Follow-up | What it was before |
| --- | --- |
| **The L3 re-export surface** | `ThemeDefinition` / `CompiledTheme` were exported, and **not one of the token structs they are made of**. A consumer could take a definition and call `.compile()` on it, but could not construct or modify one: every type a `with_*` builder takes had no name to say, and the three identity fields `ThemeId` / `ThemeSchemaVersion` / `ThemeGeneration` were all out of reach. `nana_ui::theme` now mirrors the list in `nana_ui_core::lib`, plus `SemanticColorMix` / `PaintStyle` / `BoxShadowSpec`, and `MaskImage` / `CssGradient` / `LinearGradient` / `RadialGradient` / `GradientStop` for `PaintStyle::mask`. |
| **`ThemeMetrics::switch`** | The switch track's 30×16 and the 8 label gap were four literals in `world/geometry.rs`, out of the theme's reach; the scene painter kept its own hand-copied `38.0` (= 30+8) for the label inset, so changing the track silently mismatched. `SwitchMetrics` is composed into `ThemeMetrics` following the `ScrollbarMetrics` precedent. At the same time `Switch::project` wrote `control_padding_x` unconditionally, overriding the caller's padding — the one intent field that could not be opted out of, so a switch with no label was left with a 2pt content box and its track drawn as a slit. Clearing `control_padding_x` is not an opt-out either: it is `None` to begin with, and that line would immediately write it back. So the criterion became **whether the caller already spent a number on that edge** — if it did, it wants that value, and a component should not override it by default, which is the same rule `Button::layout` states for the caller handing over the whole box. |
| **`SemanticPaint::foreground_secondary`** | Inline secondary text (detail, hint, placeholder, unit) was read straight from `palette.muted` by `world/geometry.rs`, ignoring node state. A focused row lays `focus_surface` and moves its label to `focus_text`, while the gray text on that row was still resolved against the **previous** surface: 1.09:1 in light mode. No single `muted` value solves it — it also has to stay quiet on a white card. So the state speaks for itself: `FOCUS_SURFACE` hands over a secondary too, the default `None` still resolves to `Muted`, and rest-state rendering is byte-for-byte unchanged. Later `ListItem`'s note/detail changed to default to the row's main text color multiplied by `OpacityTokens::secondary_text` (dark 0.55, light 0.90: the painter blends in linear light, and a single α only reaches 2.9:1 in light mode; at rest these values are 7.05:1 / 6.36:1 on the surface, against 6.46 / 5.66 for `muted`, and hover and selected stay above 4.5:1). On disabled, selected or app-recolored rows, secondary text follows the main text; a state that supplies `foreground_secondary` (focus) still wins. hint, placeholder and unit keep going through `Muted`. |
| **`NodeStyle::corner_radii`** | Per-corner radii existed only as `paint.border_radii`, which takes lengths. Two pieces joined into one shape (rounded outside, square at the seam) could only spend `radius_*` as numbers at construction time, so changing the corner radius setting moved every single-value radius in the same window while those pieces stayed put. `corner_radii: Option<[Option<RadiusTier>; 4]>` is the four-corner form of `radius` (`None` is a square corner: at a seam even the smallest tier leaves a notch where the two arcs separate), on the same intent resolution path and re-resolved on the same theme installation; when set it wins over `radius`, matching how `border_radii` wins over `border_radius` at paint time. |
| **A label-less `Switch` is its track** | A trailing switch has no text to give the node width: the default `Fill` spreads an invisible hit area across the row (NanaUI's own settings page does exactly this — the title and description squeezed to half width and truncated), and the size tier's padding then eats into the content box. Every caller had to hand-write the track width and zero the padding, and that number did not follow a theme installed later. Now, when both label and hint are empty, the default width takes the installed `switch.track_width` and no padding; `Switch` also turns on `wants_metrics_reproject` — the hint row's height was already spent from installed values in `project`, but it had never reprojected. This one changed 8 pixel and 6 semantic baselines (all that trailing switch in the settings page) and is an intended visual change. |
| **`ScrollbarRecipe`: what the scrollbar looks like and when it appears** | Colors were hard-wired to three shared roles (rest `border_strong`, drag `muted`, persistent track `subtle`), so changing them dragged a pile of other controls along; the foreground of `ComponentRecipeId::Scrollbar` had no visible effect; show/hide was an instant cut, and it did not appear while scrolling. Now it is `ComponentThemeRegistry::scrollbar`, a whole `Option`, missing means `MissingRecipe { Scrollbar, "scrollbar" }`: the thumb in rest, lane-active and drag, and both tracks (active and persistent) are `SemanticColorMix` slots (with a new `const` constructor and `SemanticColorMix::role`); `ScrollbarMotion` supplies the three fade-in, fade-out and thicken transitions plus `hide_delay_ms` (a transition must not be 0, the delay may be); `visibility` is an application-level policy that scroll views without `ScrollView::scrollbars` follow (`ThemeDefinition::with_scrollbar` / `with_scrollbar_visibility`). The built-in values for rest, drag and the persistent track are unchanged. `ScrollView` turns on `wants_recipe_reproject`, so a policy change reaches scroll views already on screen. |
| **`ScrollbarMetrics::thumb_active_thickness` and `track_inset`** | `track_inset` and `page_fraction` were never read, and the scrollbar sat on the outer edge of the border box, on top of the content. Now the lane sits `track_inset` inside the padding box edge, and the thumb grows symmetrically in the lane from `thumb_thickness` to the new `thumb_active_thickness` (default 8, `serde(default)` on the struct, old blobs still load); `page_fraction` is documented as a reserved field. `ThemeMetrics::same_layout` no longer compares `scrollbar`: the scrollbar is an overlay, so a theme change that only touches it repaints without relayout. |
| **`RadiusTier::Xl`** | Four tiers up to `Lg` (page-level surfaces). A rail floating above the page (Dock, toolbar) has to be one tier rounder than the card beside it before it reads as a separate object, and a consumer could only write `lg + 8`, a number that does not follow the theme. `ThemeMetrics::radius_xl` defaults to `space::PAGE_TIGHT` (20), with `serde(default)` so old metrics still load; `AppearanceSettings` does not store it separately, it follows `lg` at the default spacing, so raising `lg` does not leave it behind. |
| **`RadiusTier::Xxl` (the 2xl tier)** | The tiers stopped at `Xl`. Large surfaces written against the Vue theme's `--radius-2xl` (twice the base) — a player bar, a detail card — could only take a radius in pixels, and a theme installed later could not move it. `ThemeMetrics::radius_xxl` defaults to `space::PAGE` (24): the five existing tiers each read one spacing step (`XXS`, `SM`, `LG`, `XXL`, `PAGE_TIGHT`), and 2xl takes the step after `PAGE_TIGHT`. `serde(default)` so old metrics still load; theme compilation rejects negative and non-finite values like any other length; `AppearanceSettings` does not store it separately, and like `Xl` it follows `lg` at the default spacing. The paint script's tier name is `"xxl"`. |
| **`DialogRecipe`: where the dialog card stands** | The card's 90px from the top and its 76% of the scrim height as a maximum were constants in `overlay_surfaces.rs`, and its width had only the five `DialogSize` tiers. A design written as `min(520px, 92vw)`, `margin-top: 12vh`, `max-height: 72vh` did not line up and had to snap to the nearest tier. Now the width belongs to the dialog (`DialogSize::Width(length)` / `capped(px, vw)`, CSS length semantics), and the distance from the top, the maximum height, the title row's icon / close slot / gap, the three sections' padding, the divider and the card radius all belong to the theme: `ComponentThemeRegistry::dialog`, a whole `Option` like the status table, missing means `MissingRecipe { Overlay, "dialog" }`, and a length that is not finite and non-negative is rejected at install. The command palette borrows the dialog card's placement and does not move with the theme. The recipe is not part of any layout input, so on theme installation, if only it changed, the dependency marks just the modals for relayout and text re-measurement and does not touch other nodes; skipping that step would leave incremental layout disagreeing with full layout (`layout-verify` catches it). |
| **`DialogRecipe::align`: centering the card vertically** | The card could only stand at `top` from the scrim's top edge and grow downward with its content; a design that centers the card in the window the way `justify-content: center` does could not be expressed, so a `top` had to be guessed and drifted whenever the content changed. Now `align: DialogAlign { Top, Center }`: with `Center` the card is equally far from the scrim's top and bottom, placed after its height is measured, the slots follow, and it still does not cross the scrim's margins; the built-in is `Top`, with zero baseline change. The command palette borrows the built-in recipe and does not follow the centering. |
| **`DialogMotion`: how a dialog arrives and leaves** | Like other overlays, the dialog faded the whole node (scrim plus card) with the `OVERLAY_FADE` constant, the card did not move, the theme could not reach it, and reduced motion was ignored. A design written against Vue's `.modal-fade` (scrim .16s, card `translateY(-8px) scale(.98)` moving back over .18s) could not be expressed. Now it is `DialogRecipe::motion`: one `DialogTransition { millis, easing }` each for the scrim, the card fade and the card offset, plus the starting `enter_offset_y` / `enter_scale`; the built-in value is the old 140ms ease-out fade. The card's layer turns about the card center, and the scrim fades by its own `scrim_opacity` (the blur fades with it). The reduced-motion preference on `AppContext` is mirrored to `UiWorld`, so dialogs, drawers and menus all open and close immediately. |
| **The dialog title and divider in `DialogRecipe`** | The dialog title was 14 / 600 written separately in `world/geometry.rs` and `world/text.rs`, out of the theme's reach; a danger dialog's title turned danger-colored unconditionally; the title block ran all the way to the close slot; the divider went edge to edge. A design that paints the card face as a tab standing above the body, with the title inside the tab (NanaLive's card slot), could not be expressed: the title ran over the tab onto the slope, and the hairline did not start at the title text. Now `title: RecipeText` (measurement and painting read the same one, drawers keep their own), `danger_title`, `title_max_width` (`%` against the card width, `vw` / `vh` against the scrim) and `divider_inset: DialogDividerInset { Edge, Content }` all belong to the recipe, the built-in values are what they were, with zero baseline change. The card outline itself is not something a recipe can finish saying (tab, slope and base plate are the application's own outline), so `Dialog::surface_painter` / `ConfirmDialog::surface_painter` open a separate path: the application's `Painter` records once in card-face coordinates, `PaintContext::modal_sections()` gives the position of each section, and the scene draws it at the card's original layer (above the scrim, below the title, fading and moving with the card); the theme's card is no longer drawn unless the painter calls `draw_default()`. The recording is cached by painter key, card size, sections and theme, independently of the node's own painter. |
| **`SettingsRow::divided` is a divider, not a frame** | `divided` set the uniform `border_width`, stroking all four edges: stacked row after row, the top and bottom edges overlapped and looked like a divider, but with a single row in a card it was a square frame inside a rounded card. Now only the bottom edge is stroked, and the last row of a group is not stroked at all. Fixing it turned up another problem: the painter read per-edge widths **without** the resolved color: when the color came from a `SemanticColorRole` there was no color on the layout, all four edges read 0, and the quad fell back to stroking four edges at the uniform width — so any single-edge border using a semantic color was painted as a frame. Per-edge widths now take their color from the same place as the uniform width. 13 pixel and 8 semantic baselines were intentionally updated, all on pages that contain settings rows; the painter fix changed no other snapshot. |
| **`ToastRecipe`: what a toast looks like** | A toast's look was constants copied in both `toast.rs`'s `effective_style` and `world/geometry.rs`: card radius, Surface background, Border hairline, 10 / 12 padding, 8 gap, 6px dot, 12 / 600 title and 11 description, out of the theme's reach, with the two copies kept from drifting by a comment. All of it now belongs to `ComponentThemeRegistry::toast`: radius tier, background, border (`None` for none, otherwise the theme's hairline), elevation (`ElevationRole`, painted as the root quad's shadow), padding, gap, tone indicator (`ToastIndicator::None` / `Dot` / `Glyph`; the core icons are the shell's and there is no status icon, so `Glyph` takes its `ToastGlyphs` from the theme), the title and description fonts and the gap between them, and the close slot's control size, with layout and geometry reading the same copy. Like the dialog it is a whole `Option`, missing means `MissingRecipe { Overlay, "toast" }`, and a length that is not finite and non-negative, a font size or weight out of range, a zero duration, or a non-positive enter scale is rejected at install. `Toast` opts into recipe reprojection, so a toast already on screen follows; the border width is spent at projection time, so a theme that changes only the hairline still counts as a recipe change. The recipe also carries several items for notifications: the progress bar (`ToastProgressRecipe`), the action button's kind and size, enter/exit and stack movement (`ToastMotion`, none of it played under reduced motion), the stack gap, and how many lines the title and description may take. A bare `Toast` lays the title and description one line each and does not move by itself, and does not read these items; an assembled `Toast` (`AppContext::assemble_toast`) reads the title and description line limits (0 means no truncation), the progress bar's `height` / `track` / `fill` / `top_gap` (the bar's top margin inside the `copy_gap` copy column), the percentage's `show_percent` / `percent` / `percent_role`, the new `progress.min_width` (a toast showing progress is at least this wide, default 240, validated like any other length), and the action button's `action_kind` and `control_size` (the same tier as the close button); `ToastStack` reads `motion`'s `enter` / `exit` / `enter_offset_y` / `enter_scale` / `origin` (the scale origin, center by default) / `change` / `change_scale` (the rebound when the text changes in place, 1 means no playback) / `stack_move` and `stack_gap`. The built-in values are what they were, and the toast semantic baselines are byte-for-byte unchanged. |
| **`ShapeRecipe`: which radius tier buttons, icon buttons and tags take** | Components named the tier themselves (`Button` and `IconButton` write `radius: Sm`, `tag_frame` writes `.radius(Sm)`), so the theme could change how round `Sm` is but not which tier a button stands on; a design system wanting every button and tag in the app to be a capsule had to rewrite them one by one, and it was lost again on the next theme swap and view rebuild. A node now carries an intent field shaped like `radius`, `NodeStyle::shape: Option<ShapeRole>` (`Button` / `IconButton` / `Tag`), where the component only says what it is: `ComponentThemeRegistry::shape` (`ShapeRecipe { button, icon_button, tag }`, one `RadiusTier` per slot) decides the tier and `ThemeMetrics` decides the pixels, both at style resolution time, so it goes through intent resolution like `radius` (`IntentTokens` carries the recipe), and on theme installation a changed recipe re-resolves the nodes that carry an intent — including nodes like `Stack` that do not reproject. A tier the caller named (`radius`, `corner_radii`) wins over it; a radius number the caller's style writes over the component's style (`overlaid`) removes it; `Button::layout` clears it too when it hands over the whole box. Clearing `radius` does not clear `shape`; for a square corner both have to be `None`. Like toast it is a whole `Option`, missing means `MissingRecipe { Button, "shape" }`. All three built-in slots are `Sm`, the original tier, with zero baseline change. List rows, text fields, dropdowns and segmented tracks are in the next row. The action button inside a notification still names its radius explicitly through `ToastRecipe::control_radius` and does not follow `ShapeRecipe`. |
| **`ShapeRecipe` extended to controls, rows and dropdowns** | The first version covered only buttons, icon buttons and tags; text fields, list rows, dropdown triggers, segmented tracks, popover triggers and menu rows still named `Sm` themselves, and the segmented pill was even computed as a number at projection time from `radius_sm − inset`, immovable by the theme. Five more roles and slots now exist: `Field` (`TextInput`, `NumberInput`, `TextArea` and the frames of the color, size, drag-number and path fields), `Select` (`Select` plus the trigger frames of `Dropdown` and `SearchDropdown`, which share its style; a tier the caller names is no longer overridden back to `Sm` by projection), `ListItem`, `Segmented` (the track; the pill now takes the track's **resolved** radius minus the inset, using the control that is placing it before it enters the track, and `SegmentedOption` opts into recipe reprojection, so changing only that slot keeps it concentric), and `MenuRow` (menu row nodes such as `ActionMenuItem`); `Popover` triggers use `Button`. The dropdown option rows the scene draws itself (the `Select` list, menu panel option rows) and the menu search field have no node, so the new `ChromeRadii` `menu_row` / `field` carry them instead, computed at extraction through `ChromeRadii::resolve(metrics, shape)`, where `From<ThemeMetrics>` equals the built-in recipe. Places where a component cleared a radius internally (`Button::layout`, `TextInput::layout`, the bare input fields inside the color and size fields) now call the new `NodeStyle::clear_corners()`, which clears `radius`, `corner_radii` and `shape` in one go. `ShapeRecipe::with(role, tier)` changes one item; `uniform` still means every item, now including the new slots. All built-in values are `Sm`, with zero baseline change. Tabs and radio group options are in the next row. |
| **`ShapeRecipe` extended to tabs, radio rows, tree rows and the command palette** | Still naming `Sm` after the previous batch: the `Tabs` labels and radio group rows wrote `radius_sm` numbers directly at projection time (overriding the caller's tier and `clear_corners` alike); `TreeView` rows, the `ReorderList` row base, and the command palette's input and result rows were drawn by the scene at `chrome.sm`. Now: `ShapeRole::Tab` / `RadioRow` are written on the option style, and projection spends a number only for the segmented pill, everything else going through ordinary intent resolution; `ShapeRole::TreeRow` and `ChromeRadii::tree_row` cover tree rows and the drag-and-drop frame; the `ReorderList` row base reads `ChromeRadii::list_item` (the `ListItem` slot); command palette result rows are like menu items and belong to `MenuRow`, and its input to `Field`. Tab labels and radio rows have no track and need no concentricity. All built-in values are `Sm`, with zero baseline change. |

While there, the 15 design numbers in `world/geometry.rs` came down to 10. And **every** direct read of `palette.<field>` in that file now goes through `style_model.color(role)`. Semantic intent has one resolution path from here on. Landing on existing tiers, inventing no new ones: key badge 28/8/12/64 → `ControlSize::Small` / `space::MD` / `type_scale::META` / `space::XXXL * 4`. chart strip 2 → `XXS`. ListItem gap 8 and the collapse floor 16 → `MD` / `XXXL`. Range's private 6/8/10 tiers → `SM`/`MD`/`LG`. Progress 6 → `space::SM` with the radius changed to `girth / 2.0` (it used to be `3.0`, which equals it but was free to drift on its own). LevelMeter 4 → `XS`. EmptyState 22 → `PAGE_TIGHT + XXS`. XYPad axis line 1 → `HAIRLINE`. ReorderList 8 → `MD`. The `indicator_slot * 10.0 / 24.0` that StatusBadge and ValidationMessage each had merged into one named `STATUS_DOT_FRACTION`. 24 was a phantom denominator. Two phantom denominators are how two badges start to drift.

The 10 left in `world/geometry.rs` all need **new** token families. They are not in-place follow-ups: interaction state alphas (0.55 / 0.78 / 0.70 / 0.42 / 0.12) belong to #105's state matrix. Font-scale fallbacks like `(size - 1.0).max(10.0)` and the `* 1.2` line-height factor belong to #107. The TextInput editor chrome cluster belongs to #110.

The three rows above, together with the geometry convergence round. 614 pixel baselines and 146 semantic baselines, **zero change**. Those were moves that kept every value. Not visual changes.

### 7.10 Reproduction

```bash
# Definition, validation, fail-closed, accent ramp, state-layer alpha
cargo test -p nana-ui-core --all-features --lib theme::

# The installed recipe / motion / elevation really reach the runtime
cargo test -p nana-ui-runtime --all-features --lib an_installed_button_recipe
cargo test -p nana-ui-runtime --all-features --lib an_installed_family_recipe
cargo test -p nana-ui-runtime --all-features --lib an_installed_hover_duration
cargo test -p nana-ui-runtime --all-features --lib a_theme_that_fails_validation

# Hard-coded inventory and scan rules
python3 scripts/audit-theme-hardcoding.py --check \
  scripts/baselines/theme-hardcoding.json
python3 -m unittest discover -s scripts/tests

# work counters (should be field-for-field identical to the Phase 0 archive)
cargo run --release --locked -p nana-ui-runtime --features benchmark \
  --bin nana-theme-benchmark -- --output target/performance/issue102/theme.json
for id in theme-static-idle theme-controls-1k theme-palette-switch theme-accent-only \
          theme-density theme-head-style-mutation; do
  python3 perf/runners/nana/run.py --scenario "$id" --from-report target/performance/issue102/theme.json
done

# Semantic baseline: byte-for-byte identical to clean HEAD this round (both sides had 46 pre-existing CHANGED at the time, since re-recorded)
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic
```

## 8. Subtree colour overrides: the first cut of #104

A node can replace some colour roles for itself and its whole subtree. This is the first cut of #104 ThemeScope. Colours only.

```rust
use nana_ui::theme::{ColorOverrides, SemanticColor};
use nana_ui_core::SemanticColorRole;

let modeler = ColorOverrides::new()
    .with(SemanticColorRole::Accent, SemanticColor::rgb8(124, 77, 210))
    .with(SemanticColorRole::AccentStrong, SemanticColor::rgb8(104, 58, 186));

// In a view: bind a closure to follow light and dark; an empty set gives the theme back.
view.theme_colors(modeler.clone());
// Or on a style:
style.color_overrides = Some(Arc::new(modeler));
```

- `nana_ui_core::ColorOverrides` (re-exported from `nana_ui::theme`): a small `SemanticColorRole → SemanticColor` table. Typed. Not a string map, and not a copy of the palette. `new()`, `with(role, color)`, `set`, `get(role)`, `is_empty`, `iter`, `over(&outer)`.
- `NodeStyle::color_overrides: Option<Arc<ColorOverrides>>`. `El::theme_colors(impl IntoProp<ColorOverrides>)` binds it.
- Flattened once at resolution. `ComputedStyle::color_scope` is inherited: the node's own overrides written over its parent's scope, inner over outer role by role. Descendants read that one `Arc` and never walk the parent chain. A node without overrides still shares its parent's result.
- Where it reaches: role-based foreground, background, border, ring and every interaction state (`SemanticPaint`) of node styles; component geometry (all of `derive_component_geometry` reads the scoped `StyleModelRef`: switch track and thumb, status badge tone, button, checkbox, radio, slider, segmented control, list rows and their focus rings); standard visual foregrounds; text spans, selection fill, drop highlight; a text input's counter and suffix.
- Invalidation: a changed `color_overrides` is treated like an inherited foreground — the subtree is marked STYLE | RENDER, and an unchanged flattened result stops propagation on re-resolve.
- Derived roles (`WarningSoft*`, `DangerSoft*`, the code-token roles) follow their base: overriding `Warning` moves `WarningSoft` too. Naming a derived role alone has no effect. `Titlebar` and `OnMedia` can be overridden.
- Under the default theme no node carries overrides, and the resolution path is byte-for-byte what it was.

Left for #104: typography, metrics, recipe and motion patches; per-window root binding and multi-window checks; scope identity with generation caching (resolution flattens now, so a nested scope allocates its merged result once per resolve); charts (`chart_geometry` caches per size and theme and still reads the installed theme).
