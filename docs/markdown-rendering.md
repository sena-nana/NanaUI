# NativeMarkdown drawing and image contract

`NativeMarkdown` parses and lays out content in Runtime. `UiWorld::markdown_layout` uses the current text shaper and content box; selection, pointer activation and Scene drawing consume that same geometry. Scene emits ordinary text, quad and SVG primitives into the existing tree. There is no separate Markdown window, renderer, or product GPU device.

Inline strong, emphasis, strike, link underline and code surfaces survive into drawing commands. Inline math reserves its rendered width; display math and Mermaid reserve their rendered height. TeX uses RaTeX SVG glyph outlines. Mermaid SVG text uses the existing bundled UI font through the normal SVG raster path. Source remains available for selection and copy. Invalid math falls back to source text and invalid diagrams remain code text. Math sources above 16,384 bytes and Mermaid sources above 128,000 bytes are rejected before cache-key construction; rejected oversized inputs do not enter the bounded 200-entry cache.

Applications own asynchronous image loading and resource identity. Call `resolve_image(original_source, resolved_url, width, height)` with nonzero intrinsic dimensions after loading; every occurrence, including table cells and empty alt text, is updated. The resolved URL is used for drawing while the original source and alt text remain in `RichTextEvent::ImageActivated`. Ordinary links sharing the same URL remain `LinkActivated`. Repeating an identical resolution is a no-op. Image aspect ratio, painting, hit areas and selection share the same layout.

`ImageViewer::intrinsic_size(width, height)` preserves natural size for small images and contains large images in the available stage. Zoom and pan operate on those fitted dimensions; host textures paint above the backdrop and below controls, clipped to the stage. Pointer coordinates use the shared Runtime layout conversion and capture lifecycle. Escape uses the common overlay dismissal route.

`TextInput::max_length` counts UTF-16 code units. User replacement, paste, IME and advanced replace share the same admission path; a rejected edit leaves text, selection and history unchanged. A preexisting over-limit value is not silently truncated and remains editable without increasing its current size. `TextArea::resize_vertical(true)` retains user height across normal view projection, respects explicit pixel constraints and emits no text change. Pointer cancel, capture loss and parking clean up the resize. Content-box resizing is supported only with an explicit pixel height and no min/max height constraints; unsupported cases do not expose a grip.

## Recovery and verification

These capabilities were recovered from the original task `01a066c5-4b6a-7b81-958c-1825eb3330e7` and its recorded subtask changes, then adapted to the current Runtime/Scene interfaces. Original records and extraction provenance are preserved under `../nanaui-restoration-recovery-20260910/markdown/`. Historical validation does not replace current-tree validation.

Focused validation commands:

```sh
CARGO_BUILD_JOBS=2 cargo test -p nana-ui-runtime --all-features --lib markdown
CARGO_BUILD_JOBS=2 cargo test -p nana-ui --features components --test text_input_max_length --test text_area_resize --test scroll_workspace_surface
CARGO_BUILD_JOBS=2 cargo test -p nana-ui-scene --features components --test image_viewer_content_order
CARGO_BUILD_JOBS=2 cargo run -p nana-ui-devtools --features runtime-agent,nana-ui/components --example restored-content-probe -- target/restored-content-probe
```

The probe paints real SVG/math/image content and an uploaded host texture through the shared Scene path in light and dark themes. It routes a normal pointer drag through `RuntimeInputAdapter`, checks the resulting height and captures before/after frames. Pixel output must be inspected; process success alone is insufficient acceptance.

## Current recovery evidence (2026-09-10)

The final probe completed with 16 PNGs: light/dark × logical 760×680 at 1× and 380×720 at 2× × Markdown/resize before and after, ImageViewer natural size and 3× zoom. All 16 were opened and inspected. Formula/diagram/image content is present; image aspect ratio is maintained; narrow viewer captions ellipsize and zoom content clips to the stage. Pointer resizing increased height by 55 logical pixels in every case. Link underline and strikethrough use the existing common Scene decoration helper and passed foreground-pixel assertions.

Persistent screenshots, command log and SHA-256 manifest: `../nanaui-restoration-recovery-20260910/markdown/evidence/`. Temporary working output: `/tmp/nanaui-consumer-upgrade-20260909/restoration-content-captures-after-shaping/`. The 2× captures also expose the existing URL-SVG raster path's intrinsic-resolution softness; geometry and presence checks do not claim scale-aware vector rasterization.

Code's local all-targets check and all 557 desktop library tests passed. Its formal native agent/performance entrypoints remain blocked by the locked desktop session. The framework's final full Runtime/Scene/input gates are coordinated separately; earlier recovered test fixtures that bypassed the modern hit-index were corrected to real layout and pointer coordinates instead of weakening input checks.

The focused input integrations each ran independently and passed: `scroll_workspace_surface` (1), `text_area_resize` (3), and `text_input_max_length` (5). The Workspace fixture now uses `assemble_workspace` before layout. A negative control with the same assembled tree and a structural `new` slot reproduced loss of ScrollView pointer semantics; the explicit `borrowed` contract preserves live input/accessibility and passed hover, scrollbar drag, label refresh and surface-preservation checks. The default structural slot remains unchanged. See `application-api.md` for the borrowing contract.

After the shared Painter Auto-to-Advanced shaping fix, the identical 16-image matrix was rerun successfully and every PNG was inspected again. The persistent evidence manifest now describes this final run; earlier images remain in `markdown/before-text-shaping-fix/` for comparison.
