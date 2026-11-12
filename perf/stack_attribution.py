#!/usr/bin/env python3
"""Attribute folded stacks from `stack-sampler` to NanaUI / wgpu layers.

    python perf/stack_attribution.py stacks.folded [--scope SUBSTRING] [--top N]

Self time: each sample is charged to the leaf-most frame that belongs to a
known owner. Generic frames (alloc, core, std, hashbrown, the CRT, ntdll heap
and memcpy) are skipped so their cost lands on the crate that called them.

Owners:
  nana        nana_* crates (the bench itself counts here too)
  wgpu-api    the `wgpu` front-end crate (argument conversion, dispatch)
  wgpu-core   wgpu_core, split into `validation+tracking` and `other` by
              function path (see CORE_VALIDATION)
  wgpu-hal    wgpu_hal and the system graphics stack it calls
              (d3d12/dxgi/driver DLLs, vulkan-1 and ICDs)
  os          kernel32/ntdll/win32u frames reached with no owner above them
  other       everything else (naga, parking_lot, ...)
  blocked     the thread was suspended inside a kernel wait (GPU fence,
              frame slot, present); wall time rather than CPU work

`--scope` keeps only samples whose stack contains the substring, e.g.
`SceneWgpuPainter::paint` or `FrameContext::submit`.

`--path-stages` charges every non-blocked sample to exactly one stage of the
vector-path pipeline (see PATH_STAGES; first match wins), for the
`gpu-scene-paths-*` rows in docs/reference/gpu.md §10. `--cpu-us-per-frame`
turns the shares into microseconds per frame.
"""

import argparse
import collections
import re
import sys

GENERIC = re.compile(
    r"!(alloc::|core::|std::|hashbrown::|smallvec::|arrayvec::|parking_lot|lock_api::|"
    r"memcpy|memmove|memset|memcmp|_?_?rust_|__rdl_|__rg_|"
    r"Rtl[A-Za-z]*Heap|RtlAllocateHeap|RtlFreeHeap|RtlReAllocateHeap|RtlpLowFragHeap|"
    r"Rtlp[A-Za-z]*Heap|HeapAlloc|HeapFree|malloc|free|realloc|"
    r"RtlEnterCriticalSection|RtlLeaveCriticalSection|RtlAcquireSRW|RtlReleaseSRW|"
    r"bytemuck::|thin_vec::|indexmap::|foldhash::|ahash::|rustc_hash::|"
    r"<alloc::|<core::|<std::|<hashbrown::|<smallvec::|<arrayvec::)"
)
GRAPHICS_DLL = re.compile(
    r"^(d3d12|D3D12Core|d3d12SDKLayers|dxgi|d3d11|dxcore|nvwgf2umx|nvldumdx|nvoglv64|"
    r"nvlddmkm|vulkan-1|nvgpucomp64|igd|amdxc|atidxx|amdvlk|nvapi64|win32u|gdi32)",
    re.IGNORECASE,
)
OS_DLL = re.compile(r"^(ntdll|KERNELBASE|kernel32|ucrtbase|VCRUNTIME)", re.IGNORECASE)
BLOCKED = re.compile(r"!(NtWait|ZwWait|NtDelayExecution|ZwDelayExecution|NtRemoveIoCompletion|NtYield|ZwYield|NtSignalAndWait)")
# wgpu-core functions whose job is validating commands or tracking resource
# state/lifetime. The rest of wgpu-core (command recording, locks, Arc
# bookkeeping, hal dispatch glue) is `other`.
CORE_VALIDATION = re.compile(
    r"track|validat|check_|::check|usage_scope|UsageScope|merge_|set_and_merge|"
    r"state_set|BindGroupStates|binding_model::|late_|BufferTracker|TextureTracker|"
    r"memory_init|init_tracker|InitTracker|clear_texture|resource_used|"
    r"is_compatible|Compat|BoundBindGroupLayouts|same_device|check_usage|"
    r"Binder::|binder::|pipeline_layout|vertex_state|VertexState|draw_kind|"
    r"limits|ensure",
    re.IGNORECASE,
)


# Inclusive groups: a non-blocked sample counts toward a group when any frame
# contains one of its substrings.
INCLUSIVE = [
    ("runtime: RuntimeDocument::flush", ["RuntimeDocument::flush"]),
    ("painter: SceneWgpuPainter::paint", ["SceneWgpuPainter::paint"]),
    ("  painter text", ["scene_paint::text::", "nana_ui::nana_text::", "nana_text::"]),
    ("  painter path/mesh", ["scene_paint::mesh::"]),
    ("  painter icon", ["scene_paint::icon"]),
    ("  painter quad", ["scene_paint::quad::"]),
    ("frame: GpuContext::begin_frame", ["GpuContext::begin_frame"]),
    ("frame: FrameContext::submit", ["FrameContext::submit"]),
    ("any wgpu-core", ["!wgpu_core::", "<wgpu_core::"]),
    ("any wgpu-hal/driver", ["!wgpu_hal::", "<wgpu_hal::", "nvwgf2umx!", "D3D12Core!", "d3d12!", "nvoglv64!", "vulkan-1!", "win32u!NtGdiDd"]),
]


# Exclusive stages of the vector-path pipeline, checked in order. A sample is
# charged to the first stage any of whose needles occurs in any of its frames,
# so the order matters: `MeshBuilder::fill` calls `tessellate` (lyon), and
# `build_ops` contains all four build stages.
BUILD = "custom_paint::build_ops"
PATH_STAGES = [
    ("build: i_overlay (fill rule, stroke outline, clip)",
     [BUILD], ["i_overlay::", "i_float::", "i_shape::", "i_tree::", "i_key_sort::"]),
    ("build: lyon fill tessellation",
     [BUILD], ["custom_paint::tessellate", "lyon_tessellation::fill", "custom_paint::triangulate"]),
    ("build: flatten curves",
     [BUILD], ["custom_paint::flatten", "custom_paint::lyon_path", "lyon_path::iterator", "lyon_geom::"]),
    ("build: AA fringe / shadow band",
     [BUILD], ["custom_paint::MeshBuilder", "custom_paint::miters", "custom_paint::ring_reach",
               "custom_paint::clip_polygon"]),
    ("build: mesh reuse (key, lookup, recolour)",
     [BUILD], ["MeshReuse::begin", "MeshReuse::path", "MeshReuse::word", "MeshReuse::affine",
               "MeshReuse::finish", "custom_paint::recolored", "custom_paint::KeyHasher", "hashbrown::"]),
    ("build: other (dashes, op walk, alloc)", [BUILD], [""]),
    ("diagram: parse + layout", [], ["nana_ui_diagram::"]),
    ("diagram: record", [], ["markdown_diagram::record"]),
    ("record: Painter::paint", [], ["as nana_ui_runtime::custom_paint::Painter>::paint",
                                     "custom_paint::PaintContext::", "vector::VectorCanvas"]),
    ("painter: content match (probe + compare)", [], ["mesh::mesh_probe", "mesh::same_content"]),
    ("painter: push_path (vertex transform)", [], ["MeshPipeline::push_path"]),
    ("painter: path upload (diff + write)", [], ["MeshPipeline::upload"]),
    ("submit: upload-ring copy (all uploads)", ["FrameContext::submit"], ["nana_gpu::upload::"]),
    ("scene: emit built ops", [], ["custom_paint::emit_", "UiScene::emit_built_ops"]),
    ("runtime flush: other", [], ["RuntimeDocument::flush"]),
    ("painter: other", [], ["SceneWgpuPainter::paint"]),
    ("submit + driver", [], ["FrameContext::submit", "GpuContext::begin_frame"]),
    ("other", [], [""]),
]


# Native command-list work (first match wins). wgpu-core opens one hal list
# for each pass and one more for the barriers before it.
LIST_STAGES = [
    ("native list: open (begin_encoding)", [], ["::begin_encoding"]),
    ("native list: close (end_encoding)", [], ["::end_encoding"]),
    ("native list: recycle (reset_all)", [], ["::reset_all"]),
    ("queue submit (ExecuteCommandLists / vkQueueSubmit)", [],
     ["wgpu_hal::dynamic::queue::impl$0::submit"]),
    ("barriers (insert_barriers / transition_*)", [],
     ["insert_barriers_from", "::transition_textures", "::transition_buffers"]),
    ("pass recording (encode_render_pass, other)", [], ["encode_render_pass"]),
    ("other", [], [""]),
]


def path_stage(frames, table=PATH_STAGES):
    for name, scope, needles in table:
        if scope and not any(n in f for f in frames for n in scope):
            continue
        if any(n in f for f in frames for n in needles):
            return name
    return "other"


def owner(frame):
    module, _, symbol = frame.partition("!")
    if GRAPHICS_DLL.match(module):
        return "wgpu-hal"
    if symbol.startswith("wgpu_hal::") or "<wgpu_hal::" in symbol:
        return "wgpu-hal"
    if symbol.startswith("wgpu_core::") or "<wgpu_core::" in symbol:
        if CORE_VALIDATION.search(symbol):
            return "wgpu-core/validation+tracking"
        return "wgpu-core/other"
    if symbol.startswith("wgpu::") or "<wgpu::" in symbol:
        return "wgpu-api"
    if symbol.startswith("nana_") or "<nana_" in symbol or symbol.startswith("<nana"):
        return "nana"
    if GENERIC.search(frame):
        return None
    if OS_DLL.match(module):
        return None
    if symbol.startswith("naga::") or "<naga::" in symbol:
        return "other/naga"
    if module.lower().startswith("nana") and symbol.startswith("0x"):
        return None
    return "other"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("folded")
    parser.add_argument("--scope", action="append", default=[])
    parser.add_argument("--top", type=int, default=25)
    parser.add_argument(
        "--json", action="store_true", help="print one JSON summary instead of tables"
    )
    parser.add_argument("--path-stages", action="store_true")
    parser.add_argument("--list-stages", action="store_true")
    parser.add_argument("--cpu-us-per-frame", type=float)
    args = parser.parse_args()
    table = LIST_STAGES if args.list_stages else PATH_STAGES
    inclusive = collections.Counter()
    stages = collections.Counter()

    totals = collections.Counter()
    leaves = collections.defaultdict(collections.Counter)
    scoped = 0
    total = 0
    with open(args.folded, encoding="utf-8") as handle:
        for line in handle:
            stack, _, count = line.rstrip("\n").rpartition(" ")
            count = int(count)
            frames = stack.split(";")
            total += count
            if args.scope and not any(any(s in f for f in frames) for s in args.scope):
                continue
            scoped += count
            charged = "os"
            charged_frame = frames[-1]
            if BLOCKED.search(frames[-1]):
                # Suspended inside a wait: wall time, not CPU work.
                charged = "blocked"
            for frame in ([] if charged == "blocked" else reversed(frames)):
                who = owner(frame)
                if who is not None:
                    charged = who
                    charged_frame = frame
                    break
            totals[charged] += count
            if charged != "blocked":
                stages[path_stage(frames, table)] += count
                for name, needles in INCLUSIVE:
                    if any(n in f for f in frames for n in needles):
                        inclusive[name] += count
            leaves[charged][charged_frame.partition("!")[2][:160]] += count

    busy = scoped - totals["blocked"]
    if args.path_stages or args.list_stages:
        import json

        shares = {name: stages[name] / max(busy, 1) for name, _, _ in table}
        if args.json:
            print(json.dumps({
                "cpu_samples": busy,
                "blocked": totals["blocked"],
                "share": {k: round(v, 4) for k, v in shares.items()},
                "us_per_frame": None if args.cpu_us_per_frame is None else
                {k: round(v * args.cpu_us_per_frame, 1) for k, v in shares.items()},
            }))
            return 0
        print(f"{busy} non-blocked samples ({totals['blocked']} blocked)")
        for name, _, _ in table:
            us = "" if args.cpu_us_per_frame is None else f"  {shares[name] * args.cpu_us_per_frame:9.1f} us/frame"
            print(f"  {name:52s} {stages[name]:7d}  {100 * shares[name]:5.1f}%{us}")
        return 0
    if args.json:
        import json

        print(json.dumps({
            "samples": scoped,
            "blocked": totals["blocked"],
            "cpu_samples": busy,
            "self_share_of_cpu": {k: round(v / max(busy, 1), 4) for k, v in totals.items() if k != "blocked"},
            "inclusive_share_of_cpu": {k: round(v / max(busy, 1), 4) for k, v in inclusive.items()},
        }))
        return 0
    print(f"samples: {total}  in scope: {scoped} ({100 * scoped / max(total, 1):.1f}%)")
    for who, count in totals.most_common():
        print(f"  {who:34s} {count:7d}  {100 * count / max(scoped, 1):5.1f}%")
    print(f"\n== inclusive, share of the {busy} non-blocked samples")
    for name, _ in INCLUSIVE:
        print(f"  {name:34s} {inclusive[name]:7d}  {100 * inclusive[name] / max(busy, 1):5.1f}%")
    for who, _ in totals.most_common():
        if who == "nana" and args.top < 50:
            limit = min(args.top, 12)
        else:
            limit = args.top
        print(f"\n== {who}: top self frames")
        for name, count in leaves[who].most_common(limit):
            print(f"  {count:6d} {100 * count / max(scoped, 1):5.1f}%  {name}")


if __name__ == "__main__":
    sys.exit(main())
