#!/usr/bin/env python3
"""Shared Issue #8 Scenario schema helpers and runner report envelope.

Runners must load scenarios from ``perf/scenarios/*.json``. They must not invent
a private tree size, font, DPI, or interaction script. Relative Iced/GPUI
timing gates are not enforceable until those runners produce real numbers.
"""

from __future__ import annotations

import argparse
import copy
import json
import math
import os
import platform
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping, Sequence


SCHEMA_VERSION = 1
EXIT_OK = 0
EXIT_ERROR = 1
EXIT_UNSUPPORTED = 2

# StaticTree JSON only carries params.nodes. Hierarchy/kind/text are this rule,
# shared by Nana `tree_mutations` (nana-runtime-benchmark.rs) and Iced
# `static_tree` / `static_tree_parent` (engine/iced/examples/scenario-bench).
STATIC_TREE_GENERATION = "complete-binary-heap"
STATIC_TREE_PARENT_RULE = "parent(i)=i//2, root=1"
STATIC_TREE_NODE_KIND = "element-div"

KIND_PARAM_KEYS: dict[str, tuple[str, ...]] = {
    "StaticTree": ("nodes",),
    "DeepTree": ("nodes", "depth"),
    "Mutation": ("tree_nodes", "kind"),
    "Hover": ("nodes",),
    "VirtualList": ("items", "visible", "overscan"),
    "VirtualTree": ("items", "visible", "overscan"),
    "Table": ("rows", "columns"),
    "TextEditor": ("document_chars",),
    "Ime": ("scripts",),
    "DockWorkspace": ("panes",),
    "Overlay": ("kinds",),
    "Animation": ("active",),
    "GpuScene": ("composition",),
}

MUTATION_KINDS = {
    "Text",
    "PaintOnly",
    "LayoutStyle",
    "Visibility",
    "Transform",
    "Accessibility",
}

GPU_COMPOSITIONS = {"UiOnly", "UiLive2d", "UiLive2dEffect"}


class UsageError(Exception):
    """CLI usage error; runners must exit 1, not argparse's default 2."""

PERF_ROOT = Path(__file__).resolve().parent
REPO_ROOT = PERF_ROOT.parent
SCENARIO_DIR = PERF_ROOT / "scenarios"
CATALOG_PATH = SCENARIO_DIR / "catalog.json"


def repo_root_from(start: Path | None = None) -> Path:
    if start is not None:
        return start.resolve()
    return REPO_ROOT


def load_json(path: Path) -> Any:
    with path.open(encoding="utf-8") as source:
        return json.load(source)


def dump_json(path: Path | None, payload: Mapping[str, Any]) -> str:
    text = json.dumps(payload, indent=2, ensure_ascii=False) + "\n"
    if path is None:
        sys.stdout.write(text)
        return text
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return text


def load_catalog(root: Path | None = None) -> dict[str, Any]:
    path = (root or REPO_ROOT) / "perf" / "scenarios" / "catalog.json"
    catalog = load_json(path)
    if catalog.get("schema_version") != SCHEMA_VERSION:
        raise ValueError(f"{path}: unsupported catalog schema_version")
    return catalog


def scenario_path(scenario_id: str, root: Path | None = None) -> Path:
    return (root or REPO_ROOT) / "perf" / "scenarios" / f"{scenario_id}.json"


def load_scenario(scenario_id: str, root: Path | None = None) -> dict[str, Any]:
    path = scenario_path(scenario_id, root)
    if not path.is_file():
        raise FileNotFoundError(f"scenario file not found: {path}")
    scenario = load_json(path)
    errors = validate_scenario(scenario)
    if errors:
        raise ValueError(f"{path}: " + "; ".join(errors))
    if scenario["id"] != scenario_id:
        raise ValueError(f"{path}: id {scenario['id']!r} does not match file stem {scenario_id!r}")
    return scenario


def list_harness_ids(root: Path | None = None) -> list[str]:
    catalog = load_catalog(root)
    return list(catalog["harness_ids"])


def validate_scenario(scenario: Mapping[str, Any]) -> list[str]:
    errors: list[str] = []
    if scenario.get("schema_version") != SCHEMA_VERSION:
        errors.append("schema_version must be 1")
    scenario_id = scenario.get("id")
    if not isinstance(scenario_id, str) or not scenario_id:
        errors.append("id must be a non-empty string")
    kind = scenario.get("kind")
    if kind not in KIND_PARAM_KEYS:
        errors.append(f"unknown kind {kind!r}")
        return errors
    params = scenario.get("params")
    if not isinstance(params, dict):
        errors.append("params must be an object")
        return errors
    for key in KIND_PARAM_KEYS[kind]:
        if key not in params:
            errors.append(f"{kind} params missing {key}")
    if kind == "Mutation" and params.get("kind") not in MUTATION_KINDS:
        errors.append(f"Mutation.kind must be one of {sorted(MUTATION_KINDS)}")
    if kind == "GpuScene" and params.get("composition") not in GPU_COMPOSITIONS:
        errors.append(f"GpuScene.composition must be one of {sorted(GPU_COMPOSITIONS)}")
    if kind == "GpuScene" and params.get("composition") == "UiOnly":
        errors.extend(_validate_gpu_scene_ui_only(params))
    if kind == "StaticTree" and not _positive_int(params.get("nodes")):
        errors.append("StaticTree.nodes must be a positive integer")
    if kind == "VirtualList":
        for key in ("items", "visible", "overscan"):
            if not _positive_int(params.get(key)) and not (
                key == "overscan" and params.get(key) == 0
            ):
                errors.append(f"VirtualList.{key} must be a non-negative integer")
    if kind == "VirtualTree":
        for key in ("items", "visible", "overscan"):
            if not _positive_int(params.get(key)) and not (
                key == "overscan" and params.get(key) == 0
            ):
                errors.append(f"VirtualTree.{key} must be a non-negative integer")
    if kind == "Table":
        for key in ("rows", "columns"):
            if not _positive_int(params.get(key)):
                errors.append(f"Table.{key} must be a positive integer")
    return errors


def static_tree_parent(index: int) -> int | None:
    """Complete binary heap parent. Matches Nana tree_mutations and Iced static_tree_parent."""
    if not isinstance(index, int) or isinstance(index, bool) or index <= 1:
        return None
    return index // 2


def static_tree_children(index: int, nodes: int) -> list[int]:
    """Children of a StaticTree heap node. Left=2i, right=2i+1 when in 1..=nodes."""
    if not isinstance(index, int) or isinstance(index, bool) or index < 1:
        return []
    children: list[int] = []
    left = index * 2
    right = left + 1
    if left <= nodes:
        children.append(left)
    if right <= nodes:
        children.append(right)
    return children


def static_tree_sample_parents(nodes: int) -> list[dict[str, Any]]:
    indexes = [1, 2, 3]
    if nodes >= 50:
        indexes.append(50)
    indexes.append(nodes)
    unique: list[int] = []
    for index in indexes:
        if 1 <= index <= nodes and index not in unique:
            unique.append(index)
    return [{"index": index, "parent": static_tree_parent(index)} for index in unique]


INCOMPARABLE_STATIC_TREE_50K = 50_000
INCOMPARABLE_STATIC_TREE_50K_REASON = (
    "StaticTree 50k is not comparable: Nana nana-runtime-benchmark maps this id as "
    "kind=construction (enqueue/commit/paint/hover only), not a full systems pass. "
    "Iced scenario-bench refuses a full 50k layout+draw for the same reason. "
    "Unsupported until both sides share the same work definition. "
    "Do not silently compare construction-only Nana vs full Iced 50k."
)
REQUIRED_HOVER_NODES = 10_000
REQUIRED_MUTATION_NODES = 5_000
ICED_SAME_SCENARIO_MUTATION_KINDS = frozenset({"PaintOnly", "Text", "LayoutStyle"})
ICED_UNSUPPORTED_MUTATION_KINDS = frozenset({"Visibility", "Transform", "Accessibility"})
ICED_UNSUPPORTED_MUTATION_REASON = (
    "Iced scenario-bench has no same-scenario Visibility, Transform, or Accessibility. "
    "Nana measure_single_node_mutations uses LayoutStyle.hidden, PaintTransform.e "
    "{4|8}, and set_accessibility labels alpha/beta. Height-0+clip, Shadow offset, "
    "or widget Id is not that dirty work. Unsupported until Iced applies the same kind."
)
NANA_TABLE_ROW_EXTENT_PX = 20.0
NANA_TABLE_COLUMN_EXTENT_PX = 80.0
REQUIRED_TABLE_ROWS = 10_000
REQUIRED_TABLE_COLUMNS = 100
REQUIRED_TABLE_VISIBLE_ROWS = 40
REQUIRED_TABLE_VISIBLE_COLUMNS = 16
REQUIRED_TABLE_OVERSCAN_ROWS = 8
REQUIRED_TABLE_OVERSCAN_COLUMNS = 2
ICED_UNSUPPORTED_ANIMATION_REASON = (
    "Iced has no Runtime animation scheduler. Nana catalog Animation measures "
    "next_animation_deadline idle/scheduled plus advance_animations with "
    "due_animation_samples=1 on an isolated UiWorld. A tweening widget is not that work."
)
ICED_UNSUPPORTED_IME_REASON = (
    "Iced has no set_ime_preedit / commit_ime. Nana Ime measures those Runtime calls "
    "on a focused TextInput for latin/zh/ja/ko. OS IME UI and text_input typing are "
    "not that dirty work."
)
ICED_UNSUPPORTED_OVERLAY_REASON = (
    "Iced has no OverlayHost activate_overlay/dismiss_overlay or toggle_popover. "
    "Nana Overlay measures those APIs for Tooltip, Menu, Dialog, and Popover. "
    "Always-on tooltips or centered containers are not that dirty work."
)
ICED_UNSUPPORTED_GPU_SCENE_REASON = (
    "Iced has no nana-gpu-scene-benchmark UiOnly path. GpuScene / Live2D stay "
    "unsupported; do not invent GPU upload or Live2D zeros."
)


def nana_gpu_scene_skip_reason(scenario: Mapping[str, Any]) -> str | None:
    """Why Nana must not emit ok for this GpuScene, or None to run UiOnly."""
    if scenario.get("kind") != "GpuScene":
        return None
    params = scenario.get("params") if isinstance(scenario.get("params"), Mapping) else {}
    composition = params.get("composition")
    if composition == "UiOnly":
        return None
    return (
        f"GpuScene composition {composition} has no Nana encode/submit path. "
        "Live2D is not a Scene pass; HostTexture evidence is not this composition. "
        "Do not invent upload/batch zeros."
    )
ICED_UNSUPPORTED_DOCK_REASON = (
    "Iced pane_grid topology/axis/0.50-0.55/1280x800/panes=8 is not Nana catalog Dock. "
    "Nana adjust_focused_dock_split calls assemble_dock and rebuilds chrome "
    "(titles/strips/handles). Chrome-less incremental splits are not that dirty work. "
    "Unsupported until Iced resize rebuilds equivalent chrome."
)
ICED_UNSUPPORTED_TEXT_EDITOR_REASON = (
    "Iced cannot observe Nana replace_text_area_selection then drain_text on a 100k "
    "buffer. Timing only view + reused UserInterface::build + draw after an untimed "
    "edit is a cached no-op, not that dirty work. Unsupported until the analog of "
    "edit+drain dirties the timed frame. Do not invent WorkCounters.text_shaped."
)
ICED_UNSUPPORTED_VIRTUAL_TREE_REASON = (
    "Iced scenario-bench has no VirtualTree Fenwick / disclosure-row materializer. "
    "A VirtualList window is not an expanded-walk tree. Fake Iced numbers are forbidden."
)


def is_incomparable_static_tree_50k(scenario: Mapping[str, Any]) -> bool:
    params = scenario.get("params") if isinstance(scenario.get("params"), Mapping) else {}
    return scenario.get("kind") == "StaticTree" and params.get("nodes") == INCOMPARABLE_STATIC_TREE_50K


def iced_scenario_bench_skip_reason(scenario: Mapping[str, Any]) -> str | None:
    """Why Iced scenario-bench must not emit ok for this scenario, or None to run."""
    kind = scenario.get("kind")
    params = scenario.get("params") if isinstance(scenario.get("params"), Mapping) else {}
    if is_incomparable_static_tree_50k(scenario):
        return INCOMPARABLE_STATIC_TREE_50K_REASON
    if kind == "Hover" and params.get("nodes") != REQUIRED_HOVER_NODES:
        return (
            f"Hover must use nodes={REQUIRED_HOVER_NODES}; catalog has "
            f"nodes={params.get('nodes')}. Refusing to substitute a smaller tree."
        )
    if kind == "Mutation":
        if params.get("tree_nodes") != REQUIRED_MUTATION_NODES:
            return (
                f"Mutation must use tree_nodes={REQUIRED_MUTATION_NODES}; catalog has "
                f"tree_nodes={params.get('tree_nodes')}. Refusing to substitute another tree size."
            )
        mutation_kind = params.get("kind")
        if mutation_kind in ICED_UNSUPPORTED_MUTATION_KINDS:
            return ICED_UNSUPPORTED_MUTATION_REASON
        if mutation_kind not in ICED_SAME_SCENARIO_MUTATION_KINDS:
            return (
                f"Iced scenario-bench has no same-scenario Mutation.params.kind="
                f"{mutation_kind!r}. Only PaintOnly, Text, and LayoutStyle are wired."
            )
    if kind == "Table":
        if (
            params.get("rows") != REQUIRED_TABLE_ROWS
            or params.get("columns") != REQUIRED_TABLE_COLUMNS
            or params.get("visible_rows") != REQUIRED_TABLE_VISIBLE_ROWS
            or params.get("visible_columns") != REQUIRED_TABLE_VISIBLE_COLUMNS
            or params.get("overscan_rows") != REQUIRED_TABLE_OVERSCAN_ROWS
            or params.get("overscan_columns") != REQUIRED_TABLE_OVERSCAN_COLUMNS
        ):
            return (
                "Iced Table must use the catalog text-table window "
                f"(rows={REQUIRED_TABLE_ROWS}, columns={REQUIRED_TABLE_COLUMNS}, "
                f"visible={REQUIRED_TABLE_VISIBLE_ROWS}x{REQUIRED_TABLE_VISIBLE_COLUMNS}, "
                f"overscan={REQUIRED_TABLE_OVERSCAN_ROWS}x{REQUIRED_TABLE_OVERSCAN_COLUMNS}). "
                f"catalog has rows={params.get('rows')} columns={params.get('columns')} "
                f"visible={params.get('visible_rows')}x{params.get('visible_columns')} "
                f"overscan={params.get('overscan_rows')}x{params.get('overscan_columns')}."
            )
    if kind == "Animation":
        return ICED_UNSUPPORTED_ANIMATION_REASON
    if kind == "Ime":
        return ICED_UNSUPPORTED_IME_REASON
    if kind == "Overlay":
        return ICED_UNSUPPORTED_OVERLAY_REASON
    if kind == "GpuScene":
        return ICED_UNSUPPORTED_GPU_SCENE_REASON
    if kind == "DockWorkspace":
        return ICED_UNSUPPORTED_DOCK_REASON
    if kind == "TextEditor":
        return ICED_UNSUPPORTED_TEXT_EDITOR_REASON
    if kind == "VirtualTree":
        return ICED_UNSUPPORTED_VIRTUAL_TREE_REASON
    return None


def catalog_virtual_list_window(params: Mapping[str, Any]) -> dict[str, Any]:
    """Catalog VirtualList window: visible/overscan are item counts."""
    visible = params["visible"]
    overscan = params["overscan"]
    extent = params["item_extent_px"]
    return {
        "visible": visible,
        "overscan": overscan,
        "item_extent_px": extent,
        "viewport_px": float(visible) * float(extent),
        "overscan_px": float(overscan) * float(extent),
    }


def nana_framework_list_window_args(scenario: Mapping[str, Any]) -> list[str]:
    """Pass catalog list window into nana-framework-benchmark (px)."""
    window = catalog_virtual_list_window(scenario["params"])
    return [
        "--list-viewport-px",
        str(window["viewport_px"]),
        "--list-overscan-px",
        str(window["overscan_px"]),
        "--list-item-extent-px",
        str(window["item_extent_px"]),
    ]


def catalog_table_window(params: Mapping[str, Any]) -> dict[str, Any]:
    """Catalog Table window in px using Nana row 20 / column 80 extents."""
    visible_rows = params["visible_rows"]
    visible_columns = params["visible_columns"]
    overscan_rows = params["overscan_rows"]
    overscan_columns = params["overscan_columns"]
    return {
        "visible_rows": visible_rows,
        "visible_columns": visible_columns,
        "overscan_rows": overscan_rows,
        "overscan_columns": overscan_columns,
        "row_extent_px": NANA_TABLE_ROW_EXTENT_PX,
        "column_extent_px": NANA_TABLE_COLUMN_EXTENT_PX,
        "viewport_width_px": float(visible_columns) * NANA_TABLE_COLUMN_EXTENT_PX,
        "viewport_height_px": float(visible_rows) * NANA_TABLE_ROW_EXTENT_PX,
        "overscan_x_px": float(overscan_columns) * NANA_TABLE_COLUMN_EXTENT_PX,
        "overscan_y_px": float(overscan_rows) * NANA_TABLE_ROW_EXTENT_PX,
    }


def nana_framework_table_window_args(scenario: Mapping[str, Any]) -> list[str]:
    """Pass catalog table window into nana-framework-benchmark (px)."""
    window = catalog_table_window(scenario["params"])
    return [
        "--table-viewport-width-px",
        str(window["viewport_width_px"]),
        "--table-viewport-height-px",
        str(window["viewport_height_px"]),
        "--table-overscan-x-px",
        str(window["overscan_x_px"]),
        "--table-overscan-y-px",
        str(window["overscan_y_px"]),
        "--table-column-extent-px",
        str(window["column_extent_px"]),
        "--table-row-extent-px",
        str(window["row_extent_px"]),
    ]


# Nana extract is same-scenario only when the dump declares the full catalog
# window; leftover 200px overscan KeyErrors. Missing fields stay closest-legacy.
NANA_LIST_CATALOG_WINDOW_FIELDS: tuple[tuple[str, str], ...] = (
    ("list_viewport_px", "viewport_px"),
    ("list_overscan_px", "overscan_px"),
    ("list_item_extent_px", "item_extent_px"),
)
NANA_TABLE_CATALOG_WINDOW_FIELDS: tuple[tuple[str, str], ...] = (
    ("table_viewport_width_px", "viewport_width_px"),
    ("table_viewport_height_px", "viewport_height_px"),
    ("table_overscan_x_px", "overscan_x_px"),
    ("table_overscan_y_px", "overscan_y_px"),
    ("table_column_extent_px", "column_extent_px"),
    ("table_row_extent_px", "row_extent_px"),
)


def nana_catalog_window_equivalence(
    scale: Mapping[str, Any],
    window: Mapping[str, Any],
    fields: Sequence[tuple[str, str]],
    *,
    mismatch_context: str,
    missing_note: str,
    shared_note: str,
) -> tuple[str, list[str]]:
    missing: list[str] = []
    for reported_key, window_key in fields:
        reported = scale.get(reported_key)
        if reported is None:
            missing.append(reported_key)
            continue
        if not _same_number(reported, window[window_key]):
            raise KeyError(
                f"nana {reported_key}={reported} does not match catalog "
                f"{mismatch_context} ({window[window_key]}). "
                "Do not claim same-scenario while the window differs."
            )
    if missing:
        return "closest-legacy-reference", [
            f"This report does not declare {', '.join(missing)}. {missing_note}"
        ]
    return "same-scenario", [shared_note]


def nana_list_catalog_window_equivalence(
    scale: Mapping[str, Any], window: Mapping[str, Any], *, kind: str
) -> tuple[str, list[str]]:
    mismatch = (
        f"{kind} window viewport={window['viewport_px']}px "
        f"overscan={window['overscan']} items ({window['overscan_px']}px) "
        f"item_extent={window['item_extent_px']}px"
    )
    missing = (
        "Standalone nana-framework-benchmark used 200px overscan; the Nana runner "
        "now passes the catalog window (visible items × item_extent_px, overscan "
        f"items × item_extent_px = {window['overscan_px']}px). Extract cannot "
        "label same-scenario without the dump declaring that catalog window."
    )
    shared = (
        f"Shared catalog {kind} window: viewport={window['viewport_px']}px, "
        f"overscan={window['overscan']} items ({window['overscan_px']}px), "
        f"item_extent={window['item_extent_px']}px."
    )
    return nana_catalog_window_equivalence(
        scale,
        window,
        NANA_LIST_CATALOG_WINDOW_FIELDS,
        mismatch_context=mismatch,
        missing_note=missing,
        shared_note=shared,
    )


def nana_table_catalog_window_equivalence(
    scale: Mapping[str, Any], window: Mapping[str, Any]
) -> tuple[str, list[str]]:
    mismatch = (
        f"table window viewport={window['viewport_width_px']}x"
        f"{window['viewport_height_px']}px overscan="
        f"{window['overscan_columns']}x{window['overscan_rows']} items "
        f"({window['overscan_x_px']}x{window['overscan_y_px']}px) "
        f"extents={window['column_extent_px']}x{window['row_extent_px']}px"
    )
    missing = (
        "Standalone nana-framework-benchmark used 200px row overscan (10 rows); "
        "the Nana runner now passes the catalog window "
        f"(overscan_rows={window['overscan_rows']} × {window['row_extent_px']}px "
        f"= {window['overscan_y_px']}px). Extract cannot label same-scenario "
        "without the dump declaring that catalog window."
    )
    shared = (
        f"Shared catalog table window: viewport={window['viewport_width_px']}x"
        f"{window['viewport_height_px']}px, overscan="
        f"{window['overscan_columns']}x{window['overscan_rows']} items "
        f"({window['overscan_x_px']}x{window['overscan_y_px']}px), "
        f"extents={window['column_extent_px']}x{window['row_extent_px']}px."
    )
    return nana_catalog_window_equivalence(
        scale,
        window,
        NANA_TABLE_CATALOG_WINDOW_FIELDS,
        mismatch_context=mismatch,
        missing_note=missing,
        shared_note=shared,
    )


def catalog_uniform_window_item_cap(
    viewport_px: Any, overscan_px: Any, item_extent_px: Any
) -> int:
    """Same geometric cap as Nana ``VirtualListLayout::uniform_window_item_cap``."""
    extent = float(item_extent_px)
    if extent <= 0:
        return 0
    return math.ceil((float(viewport_px) + 2.0 * float(overscan_px)) / extent) + 2


def catalog_virtual_list_live_bound(params: Mapping[str, Any]) -> int:
    window = catalog_virtual_list_window(params)
    return catalog_uniform_window_item_cap(
        window["viewport_px"], window["overscan_px"], window["item_extent_px"]
    )


def catalog_table_live_bound(params: Mapping[str, Any]) -> int:
    window = catalog_table_window(params)
    rows = catalog_uniform_window_item_cap(
        window["viewport_height_px"], window["overscan_y_px"], window["row_extent_px"]
    )
    columns = catalog_uniform_window_item_cap(
        window["viewport_width_px"], window["overscan_x_px"], window["column_extent_px"]
    )
    return rows + rows * columns


# Catalog ids whose Nana/Iced runners already emit honest ``ok`` envelopes
# *and* a non-empty catalog ``invariants`` row. ``--evaluate-invariants``
# judges these. Everything else is skipped, not invariant-ok — including
# Dock / TextEditor / Animation / IME / Overlay / GpuScene Live2D /
# StaticTree 50k / GPUI even if a report is present. Nana ``gpu-scene-ui``
# is judged only for Nana encode envelopes; missing GPU keys / adapter /
# Iced stay skipped (exit 2), never vacuous 0.
# Nana Visibility / Transform / Accessibility reuse the live 5k
# ``single_node_mutations`` dump. Iced stays unsupported (exit 2) for those
# kinds. StaticTree 100/1k/5k/10k export ``metrics.frames_after_idle``
# (docs/performance-contract.md §8.1). ``idle_schedule_ms`` is never that
# counter. Missing the field stays skipped, not vacuous ok.
SECTION_8_1_STATIC_UI_IDS = frozenset(
    {
        "static-tree-100",
        "static-tree-1k",
        "static-tree-5k",
        "static-tree-10k",
    }
)
SECTION_8_1_HONEST_OK_IDS = frozenset(
    {
        "mutation-paint-only",
        "mutation-text",
        "mutation-layout-style",
        "mutation-visibility",
        "mutation-transform",
        "mutation-a11y",
        "hover",
        "virtual-list-10k",
        "virtual-list-100k",
        "virtual-tree-10k",
        "virtual-tree-100k",
        "text-table",
        "gpu-scene-ui",
        *SECTION_8_1_STATIC_UI_IDS,
    }
)
SECTION_8_1_UNSUPPORTED_IDS = frozenset(
    {
        "static-tree-50k",
        "animation",
        "ime",
        "dock-workspace",
        "overlay",
        "text-editor",
        "gpu-scene-ui-live2d",
        "gpu-scene-ui-live2d-effect",
        "virtual-list-1m",
        "virtual-tree-1m",
    }
)


def _same_number(left: Any, right: Any) -> bool:
    try:
        return float(left) == float(right)
    except (TypeError, ValueError):
        return False


def is_shared_static_tree(tree: Mapping[str, Any] | None, nodes: int) -> bool:
    if not isinstance(tree, Mapping):
        return False
    if tree.get("generation") != STATIC_TREE_GENERATION:
        return False
    if tree.get("parent_rule") not in {None, STATIC_TREE_PARENT_RULE}:
        return False
    if tree.get("node_kind") not in {None, STATIC_TREE_NODE_KIND}:
        return False
    if tree.get("text") not in {None, False}:
        return False
    samples = tree.get("sample_parents")
    if samples is None:
        return True
    if not isinstance(samples, list):
        return False
    expected = {(item["index"], item["parent"]) for item in static_tree_sample_parents(nodes)}
    measured = set()
    for item in samples:
        if not isinstance(item, Mapping) or "index" not in item:
            return False
        measured.add((item["index"], item.get("parent")))
    return expected <= measured


def _positive_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def _size2(value: Any) -> bool:
    return (
        isinstance(value, list)
        and len(value) == 2
        and all(_positive_int(item) for item in value)
    )


_UI_ONLY_NODE_KINDS = {"list", "text", "gpu-texture-view", "button"}


def _validate_gpu_scene_ui_only(params: Mapping[str, Any]) -> list[str]:
    errors: list[str] = []
    if not _size2(params.get("viewport")):
        errors.append("GpuScene UiOnly params.viewport must be [width, height] pixels")
    host = params.get("host_texture")
    if not isinstance(host, dict):
        errors.append("GpuScene UiOnly params.host_texture must be an object")
    else:
        if not isinstance(host.get("slot"), str) or not host["slot"].strip():
            errors.append("GpuScene UiOnly host_texture.slot must be a non-empty string")
        if not _positive_int(host.get("width")) or not _positive_int(host.get("height")):
            errors.append("GpuScene UiOnly host_texture width/height must be positive integers")
    nodes = params.get("ui_nodes")
    if not isinstance(nodes, list) or not nodes:
        errors.append("GpuScene UiOnly params.ui_nodes must be a non-empty list")
        return errors
    unknown = [node for node in nodes if node not in _UI_ONLY_NODE_KINDS]
    if unknown:
        errors.append(f"GpuScene UiOnly ui_nodes unknown: {unknown}")
    if "list" not in nodes:
        errors.append("GpuScene UiOnly ui_nodes must include list")
    if "gpu-texture-view" not in nodes:
        errors.append("GpuScene UiOnly ui_nodes must include gpu-texture-view")
    if not any(node in {"list", "text", "button"} for node in nodes if node != "gpu-texture-view"):
        errors.append("GpuScene UiOnly ui_nodes must include UI chrome (list/text/button)")
    return errors


def validate_all_scenarios(root: Path | None = None) -> list[str]:
    base = (root or REPO_ROOT) / "perf" / "scenarios"
    errors: list[str] = []
    catalog = load_catalog(root)
    for scenario_id in catalog["harness_ids"]:
        path = base / f"{scenario_id}.json"
        if not path.is_file():
            errors.append(f"catalog harness_ids entry missing file: {path}")
            continue
        try:
            load_scenario(scenario_id, root)
        except ValueError as exc:
            errors.append(str(exc))
    for path in sorted(base.glob("*.json")):
        if path.name == "catalog.json":
            continue
        payload = load_json(path)
        for message in validate_scenario(payload):
            errors.append(f"{path.name}: {message}")
        if payload.get("id") != path.stem:
            errors.append(f"{path.name}: id must equal file stem")
    return errors


def machine_note() -> dict[str, Any]:
    return {
        "system": platform.system(),
        "release": platform.release(),
        "machine": platform.machine(),
        "python": platform.python_version(),
        "hostname": platform.node(),
        "note": (
            "This is the host that invoked the runner. GitHub "
            "`ubuntu-latest` / `macos-latest` weekly cron is not a fixed "
            "benchmark machine (Issue #8 §12.2)."
        ),
    }


WORK_COUNTER_KEYS = (
    "entities_total",
    "entities_changed",
    "entities_spawned",
    "entities_despawned",
    "style_processed",
    "text_shaped",
    "layout_nodes",
    "hit_test_candidates",
    "input_targets",
    "accessibility_nodes_updated",
    "render_nodes_changed",
    "render_nodes_extracted",
    "extracted_text_spans",
    "allocations",
    "allocated_bytes",
    "text_shaped_runs",
    "text_layout_cache_hits",
    "text_layout_cache_misses",
    "text_wrap_layouts",
    "glyph_cache_hits",
    "glyph_cache_misses",
    "cache_eviction",
    "batch_rebuilds",
    "draw_batches",
    "draw_calls",
    "gpu_upload_bytes",
    "gpu_buffer_reallocations",
)

GPU_WORK_COUNTER_KEYS = (
    "batch_rebuilds",
    "draw_batches",
    "draw_calls",
    "gpu_upload_bytes",
    "gpu_buffer_reallocations",
)

GPU_HOST_STAGES = ("Batch", "GpuUpload", "Encode", "Submit")


def counters_from_block(block: Any) -> dict[str, Any]:
    """Copy measured CPU WorkCounters. Nulls and GPU keys stay out.

    GPU keys on a CPU drain — including explicit 0 — are not observations.
    Only :func:`gpu_counters_from_observed` copies them after encode/submit.
    """
    if not isinstance(block, dict):
        return {}
    return {
        key: block[key]
        for key in WORK_COUNTER_KEYS
        if key in block
        and block[key] is not None
        and key not in GPU_WORK_COUNTER_KEYS
    }


def gpu_counters_from_observed(block: Any) -> dict[str, Any]:
    """Copy GPU keys only after a host encode/submit. Missing stays KeyError."""
    if not isinstance(block, dict):
        raise KeyError(
            "nana-gpu-scene-benchmark ok report must include gpu_work from a real encode/submit"
        )
    observed: dict[str, Any] = {}
    for key in GPU_WORK_COUNTER_KEYS:
        if key not in block or block[key] is None:
            raise KeyError(
                f"nana-gpu-scene-benchmark ok report omitted {key}; do not invent 0"
            )
        observed[key] = block[key]
    return observed


def lookup_path(payload: Mapping[str, Any] | None, path: str) -> Any:
    current: Any = payload
    for part in path.split("."):
        if not isinstance(current, Mapping) or part not in current:
            return None
        current = current[part]
    return current


def read_frames_after_idle(
    payload: Mapping[str, Any] | None,
    *,
    required: bool,
    source: str,
) -> int | None:
    """Copy a real idle-frame count. Never map idle_schedule_ms onto this field."""
    if not isinstance(payload, Mapping) or "frames_after_idle" not in payload:
        if required:
            raise KeyError(
                f"{source} must export integer frames_after_idle after settle; "
                "idle_schedule_ms is a timing, not that counter"
            )
        return None
    value = payload["frames_after_idle"]
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise KeyError(
            f"{source} frames_after_idle must be a non-negative integer, not {value!r}"
        )
    return value


def compare_invariant(measured: Any, op: str, expected: Any) -> bool | None:
    if op == "eq":
        return measured == expected
    if op == "lte":
        try:
            return measured <= expected
        except TypeError:
            return None
    if op == "gte":
        try:
            return measured >= expected
        except TypeError:
            return None
    return None


def evaluate_invariants(
    scenario: Mapping[str, Any] | None,
    payload: Mapping[str, Any],
) -> list[dict[str, Any]]:
    """Fail-closed: missing/null measured values stay not-evaluable, never ok."""
    if not isinstance(scenario, Mapping):
        return []
    results: list[dict[str, Any]] = []
    for spec in scenario.get("invariants") or []:
        if not isinstance(spec, Mapping):
            continue
        item: dict[str, Any] = {}
        name = spec.get("name")
        path = spec.get("path")
        op = spec.get("op")
        if name:
            item["name"] = name
        if path:
            item["path"] = path
        if op:
            item["op"] = op
        if "value" in spec:
            item["value"] = spec["value"]
        if not path or not op or "value" not in spec:
            item["status"] = "not-evaluable"
            if spec.get("note"):
                item["note"] = spec["note"]
            results.append(item)
            continue
        measured = lookup_path(payload, path)
        if measured is None:
            item["status"] = "not-evaluable"
            item["note"] = spec.get("note") or (
                f"{path} is missing; never treat a missing key as the expected value"
            )
            results.append(item)
            continue
        passed = compare_invariant(measured, op, spec.get("value"))
        if passed is None:
            item["status"] = "not-evaluable"
            item["note"] = spec.get("note") or f"cannot evaluate {op} on {path}"
        else:
            item["measured"] = measured
            item["status"] = "ok" if passed else "failed"
            if not passed:
                item["note"] = (
                    f"measured {measured} does not satisfy {op} {spec.get('value')}"
                )
        results.append(item)
    return results


def is_runner_envelope(payload: Mapping[str, Any] | None) -> bool:
    if not isinstance(payload, Mapping):
        return False
    return (
        payload.get("schema_version") == SCHEMA_VERSION
        and payload.get("runner") in {"nana", "iced", "gpui"}
        and payload.get("status") in {"ok", "unsupported", "error"}
        and isinstance(payload.get("scenario_id"), str)
        and bool(payload.get("scenario_id"))
    )


def _skip_section_8_1(scenario_id: str, runner: str, status: str) -> str | None:
    if runner == "gpui":
        return "GPUI stays unsupported; do not treat as invariant-ok"
    if scenario_id in SECTION_8_1_UNSUPPORTED_IDS:
        return (
            f"{scenario_id} is not a §8.1 honest-ok catalog id; skipped, not invariant-ok"
        )
    if scenario_id not in SECTION_8_1_HONEST_OK_IDS:
        return (
            f"{scenario_id} is not a §8.1 honest-ok catalog id; skipped, not invariant-ok"
        )
    if scenario_id == "gpu-scene-ui" and runner != "nana":
        return (
            "Iced/GPUI GpuScene stay unsupported; Nana UiOnly encode is the §8.1 path"
        )
    if status == "unsupported":
        return "envelope status=unsupported"
    return None


def judge_runner_invariants(
    report: Mapping[str, Any],
    *,
    root: Path | None = None,
) -> dict[str, Any]:
    """Judge §8.1 invariants from one runner envelope using ``evaluate_invariants``.

    This is the PR/CI entry: same rule engine runners already attach, not a
    second copy of the comparisons. Unsupported ids/runners stay skipped.
    """
    if not is_runner_envelope(report):
        return {
            "decision": "error",
            "note": (
                "not a runner envelope (need schema_version, runner, status, scenario_id)"
            ),
        }
    scenario_id = str(report["scenario_id"])
    runner = str(report["runner"])
    status = str(report["status"])
    judged: dict[str, Any] = {
        "scenario_id": scenario_id,
        "runner": runner,
        "envelope_status": status,
    }
    if report.get("equivalence") is not None:
        judged["equivalence"] = report.get("equivalence")
    skip = _skip_section_8_1(scenario_id, runner, status)
    if skip:
        judged["decision"] = "skipped"
        if status == "unsupported":
            judged["note"] = report.get("unsupported_reason") or skip
        else:
            judged["note"] = skip
        return judged
    if status == "error":
        judged["decision"] = "failed"
        judged["note"] = report.get("error") or "envelope status=error"
        return judged
    try:
        scenario = load_scenario(scenario_id, root)
    except (FileNotFoundError, ValueError) as exc:
        judged["decision"] = "error"
        judged["note"] = str(exc)
        return judged
    evaluated = evaluate_invariants(scenario, report)
    if not evaluated:
        judged["decision"] = "skipped"
        judged["note"] = (
            "catalog has no invariants; vacuous ok is forbidden until a real row exists"
        )
        return judged
    judged["invariants"] = evaluated
    failed = [
        str(item.get("name") or item.get("path"))
        for item in evaluated
        if item.get("status") == "failed"
    ]
    if failed:
        judged["decision"] = "failed"
        judged["note"] = "work-counter invariant failed: " + ", ".join(failed)
        return judged
    if scenario_id in SECTION_8_1_STATIC_UI_IDS and any(
        item.get("status") == "not-evaluable" for item in evaluated
    ):
        judged["decision"] = "skipped"
        judged["note"] = (
            "frames_after_idle missing; vacuous ok is forbidden until runners "
            "export the idle-frame count"
        )
        return judged
    if scenario_id == "gpu-scene-ui" and any(
        item.get("status") == "not-evaluable" for item in evaluated
    ):
        judged["decision"] = "skipped"
        judged["note"] = (
            "gpu-scene-ui GPU keys missing; vacuous ok is forbidden until encode/submit "
            "observes gpu_upload_bytes and draw_calls. Do not invent 0."
        )
        return judged
    judged["decision"] = "ok"
    return judged


def expand_invariant_report_paths(paths: Sequence[Path | str]) -> list[Path]:
    expanded: list[Path] = []
    for raw in paths:
        path = Path(raw)
        if path.is_dir():
            files = sorted(path.glob("*.json"))
            if not files:
                raise FileNotFoundError(f"no *.json envelopes in directory {path}")
            expanded.extend(files)
            continue
        expanded.append(path)
    return expanded


def evaluate_runner_invariant_paths(
    paths: Sequence[Path | str],
    *,
    root: Path | None = None,
) -> tuple[dict[str, Any], int]:
    """Judge one or more runner envelopes. Returns (summary, exit code)."""
    reports: list[dict[str, Any]] = []
    for path in expand_invariant_report_paths(paths):
        try:
            payload = load_json(path)
        except FileNotFoundError:
            reports.append(
                {
                    "source": str(path),
                    "decision": "error",
                    "note": f"report not found: {path}",
                }
            )
            continue
        except json.JSONDecodeError as exc:
            reports.append(
                {
                    "source": str(path),
                    "decision": "error",
                    "note": f"invalid JSON: {exc}",
                }
            )
            continue
        judged = judge_runner_invariants(payload, root=root)
        judged["source"] = str(path)
        reports.append(judged)
    failed = [item for item in reports if item.get("decision") in {"failed", "error"}]
    ok = [item for item in reports if item.get("decision") == "ok"]
    skipped = [item for item in reports if item.get("decision") == "skipped"]
    summary: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "ok": len(ok),
        "failed": len(failed),
        "skipped": len(skipped),
        "reports": reports,
    }
    if failed:
        summary["status"] = "failed"
        return summary, EXIT_ERROR
    if ok:
        summary["status"] = "ok"
        return summary, EXIT_OK
    summary["status"] = "unsupported"
    return summary, EXIT_UNSUPPORTED


def envelope(
    *,
    runner: str,
    status: str,
    scenario_id: str,
    scenario: Mapping[str, Any] | None = None,
    unsupported_reason: str | None = None,
    error: str | None = None,
    equivalence: str | None = None,
    command: Sequence[str] | None = None,
    source_binary: str | None = None,
    source_report: str | None = None,
    mapping_notes: Sequence[str] | None = None,
    metrics: Mapping[str, Any] | None = None,
    work_counters: Mapping[str, Any] | None = None,
    plug_in: str | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "runner": runner,
        "status": status,
        "scenario_id": scenario_id,
        "timing_gate_enforceable": False,
        "relative_gate_enforceable": False,
        "machine": machine_note(),
    }
    if scenario is not None:
        payload["scenario"] = dict(scenario)
    if unsupported_reason:
        payload["unsupported_reason"] = unsupported_reason
    if error:
        payload["error"] = error
    payload["equivalence"] = equivalence or (
        "unsupported" if status == "unsupported" else "closest-legacy-reference"
    )
    if command:
        payload["command"] = list(command)
    if source_binary:
        payload["source_binary"] = source_binary
    if source_report:
        payload["source_report"] = source_report
    if mapping_notes:
        payload["mapping_notes"] = list(mapping_notes)
    if metrics:
        payload["metrics"] = {
            key: value for key, value in metrics.items() if value is not None
        }
        if not payload["metrics"]:
            del payload["metrics"]
    measured = measured_counters(work_counters)
    if measured:
        payload["work_counters"] = measured
    if plug_in:
        payload["plug_in"] = plug_in
    evaluated = evaluate_invariants(scenario, payload)
    if evaluated:
        payload["invariants"] = evaluated
        if payload.get("status") == "ok" and any(
            item.get("status") == "failed" for item in evaluated
        ):
            payload["status"] = "error"
            failed = [
                str(item.get("name") or item.get("path"))
                for item in evaluated
                if item.get("status") == "failed"
            ]
            payload["error"] = "work-counter invariant failed: " + ", ".join(failed)
    return payload


def key_error_reason(exc: KeyError) -> str:
    if exc.args and isinstance(exc.args[0], str):
        return exc.args[0]
    return str(exc)


def measured_counters(values: Mapping[str, Any] | None) -> dict[str, Any] | None:
    if not values:
        return None
    measured = {key: value for key, value in values.items() if value is not None}
    return measured or None


def percentile_fields(sample: Mapping[str, Any] | None) -> dict[str, Any] | None:
    if not isinstance(sample, dict):
        return None
    p50 = sample.get("p50", sample.get("median"))
    p95 = sample.get("p95")
    p99 = sample.get("p99")
    if p50 is None and p95 is None and p99 is None:
        return None
    out: dict[str, Any] = {}
    if p50 is not None:
        out["p50"] = p50
    if p95 is not None:
        out["p95"] = p95
    if p99 is not None:
        out["p99"] = p99
    if "min" in sample:
        out["min"] = sample["min"]
    if "max" in sample:
        out["max"] = sample["max"]
    if "frame_budget_misses" in sample:
        out["frame_budget_misses"] = sample["frame_budget_misses"]
    if "frame_budget_ms" in sample:
        out["frame_budget_ms"] = sample["frame_budget_ms"]
    return out


def find_case_by_nodes(cases: Iterable[Mapping[str, Any]], nodes: int) -> dict[str, Any] | None:
    for case in cases:
        if case.get("nodes") == nodes:
            return dict(case)
    return None


def cargo_run(
    root: Path,
    *,
    package: str,
    binary: str,
    features: str,
    output: Path,
    extra_args: Sequence[str] | None = None,
) -> list[str]:
    command = [
        "cargo",
        "run",
        "--release",
        "--locked",
        "-p",
        package,
        "--features",
        features,
        "--bin",
        binary,
        "--",
        "--output",
        str(output),
    ]
    if extra_args:
        command.extend(extra_args)
    return command


def run_command(command: Sequence[str], cwd: Path) -> None:
    env = os.environ.copy()
    subprocess.run(command, cwd=cwd, check=True, env=env)


def cargo_run_iced_scenario_bench(
    root: Path,
    *,
    scenario_path: Path,
    output: Path,
) -> list[str]:
    """Invoke engine/iced scenario-bench against a shared Scenario JSON file."""
    return [
        "cargo",
        "run",
        "--release",
        "--locked",
        "--manifest-path",
        str(root / "engine" / "iced" / "Cargo.toml"),
        "-p",
        "scenario-bench",
        "--",
        "--scenario",
        str(scenario_path),
        "--output",
        str(output),
    ]


def is_iced_scenario_bench_report(report: Mapping[str, Any] | None) -> bool:
    return isinstance(report, Mapping) and report.get("source") == "iced-scenario-bench"


def _real_same_scenario(report: Mapping[str, Any] | None) -> bool:
    if not isinstance(report, Mapping):
        return False
    if report.get("status") != "ok" or report.get("equivalence") != "same-scenario":
        return False
    metrics = report.get("metrics")
    if not isinstance(metrics, Mapping) or not metrics:
        return False
    cpu = metrics.get("cpu_frame_ms")
    if not isinstance(cpu, Mapping):
        return False
    return cpu.get("p50") is not None and cpu.get("p95") is not None


def relative_gate_can_enforce(
    iced_report: Mapping[str, Any] | None,
    gpui_report: Mapping[str, Any] | None,
) -> bool:
    """True only when Iced and GPUI both emitted same-scenario ok with real metrics.

    Envelope ``relative_gate_enforceable`` stays False until this is True.
    A GPUI stub or Gallery closest-legacy-reference must not open the 1.15× gate.
    """
    if not _real_same_scenario(iced_report) or not _real_same_scenario(gpui_report):
        return False
    if iced_report.get("runner") != "iced" or gpui_report.get("runner") != "gpui":
        return False
    return iced_report.get("scenario_id") == gpui_report.get("scenario_id")


def add_common_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--scenario", help="Scenario id from perf/scenarios/")
    parser.add_argument("--all", action="store_true", help="Run every harness scenario id")
    parser.add_argument("--list", action="store_true", help="Print harness scenario ids")
    parser.add_argument("--check-schema", action="store_true", help="Validate scenario JSON")
    parser.add_argument("--output", type=Path, help="Write one JSON report")
    parser.add_argument("--output-dir", type=Path, help="Write one JSON file per scenario")
    parser.add_argument("--from-report", type=Path, help="Map an existing bench JSON; do not invoke cargo")
    parser.add_argument("--print-plan", action="store_true", help="Print cargo/commands only")
    parser.add_argument("--repo-root", type=Path, default=REPO_ROOT)


def selected_ids(args: argparse.Namespace) -> list[str]:
    if args.all:
        return list_harness_ids(args.repo_root)
    if args.scenario:
        return [args.scenario]
    raise UsageError("provide --scenario <id>, --all, --list, or --check-schema")


def write_one(args: argparse.Namespace, report: Mapping[str, Any]) -> Path | None:
    if args.output_dir is not None:
        path = args.output_dir / f"{report['runner']}-{report['scenario_id']}.json"
        dump_json(path, report)
        return path
    if args.output is not None:
        dump_json(args.output, report)
        return args.output
    dump_json(None, report)
    return None


def run_cli(
    *,
    runner: str,
    argv: Sequence[str] | None,
    plan: Callable[[str, argparse.Namespace], list[str]],
    execute: Callable[[str, argparse.Namespace], dict[str, Any]],
) -> int:
    parser = argparse.ArgumentParser(description=f"Issue #8 {runner} scenario runner")
    add_common_args(parser)
    args = parser.parse_args(argv)
    args.repo_root = args.repo_root.resolve()

    if args.check_schema:
        errors = validate_all_scenarios(args.repo_root)
        if errors:
            for error in errors:
                print(error, file=sys.stderr)
            return EXIT_ERROR
        print("scenario schema: OK")
        return EXIT_OK

    if args.list:
        for scenario_id in list_harness_ids(args.repo_root):
            print(scenario_id)
        return EXIT_OK

    try:
        ids = selected_ids(args)
    except UsageError as exc:
        print(exc, file=sys.stderr)
        return EXIT_ERROR
    if args.print_plan:
        for scenario_id in ids:
            for line in plan(scenario_id, args):
                print(line)
        return EXIT_OK

    statuses: list[str] = []
    for scenario_id in ids:
        try:
            report = execute(scenario_id, args)
        except FileNotFoundError as exc:
            report = envelope(
                runner=runner,
                status="error",
                scenario_id=scenario_id,
                error=str(exc),
                equivalence="unsupported",
            )
        except subprocess.CalledProcessError as exc:
            report = envelope(
                runner=runner,
                status="error",
                scenario_id=scenario_id,
                error=f"command failed with exit {exc.returncode}: {exc.cmd}",
                command=[str(part) for part in exc.cmd] if isinstance(exc.cmd, list) else None,
            )
        statuses.append(report["status"])
        if args.all and args.output is None and args.output_dir is None:
            args.output_dir = args.repo_root / "perf" / "reports"
        if args.all:
            path = (args.output_dir or (args.repo_root / "perf" / "reports")) / (
                f"{runner}-{scenario_id}.json"
            )
            dump_json(path, report)
            print(path)
        else:
            write_one(args, report)

    if "error" in statuses:
        return EXIT_ERROR
    if "unsupported" in statuses:
        return EXIT_UNSUPPORTED
    return EXIT_OK


def extract_nana(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    *,
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    kind = scenario["kind"]
    params = scenario["params"]
    if kind == "StaticTree":
        return _extract_nana_static_tree(scenario, reports, source_paths)
    if kind == "Mutation" and params.get("kind") == "PaintOnly":
        return _extract_nana_paint_only(scenario, reports, source_paths)
    if kind == "Mutation":
        return _extract_nana_single_node_mutation(scenario, reports, source_paths)
    if kind == "Hover":
        return _extract_nana_hover(scenario, reports, source_paths)
    if kind == "VirtualList":
        return _extract_nana_virtual_list(scenario, reports, source_paths)
    if kind == "VirtualTree":
        return _extract_nana_virtual_tree(scenario, reports, source_paths)
    if kind == "Table":
        return _extract_nana_text_table(scenario, reports, source_paths)
    if kind == "Animation":
        return _extract_nana_animation(scenario, reports, source_paths)
    if kind in {"Ime", "DockWorkspace", "Overlay", "TextEditor"}:
        return _extract_nana_catalog_workload(scenario, reports, source_paths)
    if kind == "GpuScene":
        return _extract_nana_gpu_scene(scenario, reports, source_paths)
    raise KeyError(f"no Nana mapping for {scenario['id']}")


def _extract_nana_static_tree(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    nodes = scenario["params"]["nodes"]
    runtime = reports.get("runtime")
    scene = reports.get("scene")
    if runtime is None:
        raise KeyError("nana-runtime-benchmark report required")
    case = find_case_by_nodes(runtime.get("cases", []), nodes)
    if case is None:
        raise KeyError(f"nana-runtime-benchmark has no nodes={nodes} case")
    notes = [
        f"Mapped onto nana-runtime-benchmark case nodes={nodes}.",
        "construction uses enqueue_ms + initial_commit_ms; systems use initial_systems_ms when present.",
        "This is the existing Runtime tree binary, not a dedicated Scenario executable.",
        "StaticTree JSON only has params.nodes. Generation is complete-binary-heap "
        "parent(i)=i//2, NodeKind::Element div, no text (tree_mutations).",
    ]
    if nodes == INCOMPARABLE_STATIC_TREE_50K:
        raise KeyError(INCOMPARABLE_STATIC_TREE_50K_REASON)
    kind = case.get("kind", "full")
    if kind == "construction":
        notes.append(
            "This case is kind=construction: no full initial_systems pass. "
            "Required by #8 §3.1 record set is only partially covered."
        )
    metrics: dict[str, Any] = {
        "enqueue_ms": percentile_fields(case.get("enqueue_ms")),
        "initial_commit_ms": percentile_fields(case.get("initial_commit_ms")),
        "initial_systems_ms": percentile_fields(case.get("initial_systems_ms")),
        "idle_schedule_ms": percentile_fields(case.get("idle_schedule_ms")),
        "kind": kind,
    }
    frames_after_idle = read_frames_after_idle(
        case, required=False, source="nana-runtime-benchmark StaticTree"
    )
    if frames_after_idle is not None:
        metrics["frames_after_idle"] = frames_after_idle
        notes.append(
            "frames_after_idle is the §8.1 idle-frame count (non-empty take_system_work drains)."
        )
    if scene is not None:
        row = find_case_by_nodes(scene.get("rows", []), nodes)
        if row is not None:
            metrics["scene_initial_extraction_ms"] = row.get("initial_extraction_ms")
            metrics["scene_local_update_p95_ms"] = row.get("local_update_p95_ms")
            metrics["scene_idle_update_p95_ms"] = row.get("idle_update_p95_ms")
            metrics["scene_frame_graph_p95_ms"] = row.get("frame_graph_p95_ms")
            notes.append("Scene extraction/idle/frame-graph taken from nana-scene-benchmark.")
            notes.append(
                "nana-scene-benchmark applies UiScene deltas and does not flush "
                "RuntimeDocument, so FrameProfiler stage timings are not in this envelope."
            )
    primary = percentile_fields(case.get("initial_systems_ms")) or percentile_fields(
        case.get("initial_commit_ms")
    )
    if primary:
        metrics["cpu_frame_ms"] = primary
    work_counters = counters_from_block(case.get("initial_work"))
    return envelope(
        runner="nana",
        status="ok",
        scenario_id=f"static-tree-{_scale_token(nodes)}",
        scenario=scenario,
        equivalence="closest-legacy-reference",
        source_binary="nana-runtime-benchmark",
        source_report=str(source_paths.get("runtime", "")),
        mapping_notes=notes,
        metrics={key: value for key, value in metrics.items() if value is not None},
        work_counters=work_counters,
    )


def _extract_nana_paint_only(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    tree_nodes = scenario["params"]["tree_nodes"]
    runtime = reports.get("runtime")
    if runtime is None:
        raise KeyError("nana-runtime-benchmark report required")
    case = find_case_by_nodes(runtime.get("cases", []), tree_nodes)
    if case is None:
        raise KeyError(f"nana-runtime-benchmark has no nodes={tree_nodes} case")
    if "local_paint_systems_ms" not in case:
        raise KeyError(
            "local_paint_systems_ms missing; current nana-runtime-benchmark schema is required"
        )
    paint_work = counters_from_block(case.get("local_paint_work"))
    notes = [
        f"Mapped onto nana-runtime-benchmark local_paint_* at nodes={tree_nodes}.",
        "local_paint_work_nodes is a stand-in, not a full Issue #8 work-counter set.",
    ]
    if "layout_nodes" in paint_work:
        notes.append(
            "layout_nodes is measured from SystemWork/WorkCounters. "
            "The paint-only invariant is evaluable."
        )
    else:
        notes.append(
            "layout_nodes is not measured in this envelope. "
            "The paint-only invariant is not-evaluable until a report field exists."
        )
    work_counters = {
        "entities_changed_reported": case.get("local_paint_work_nodes"),
        **paint_work,
    }
    return envelope(
        runner="nana",
        status="ok",
        scenario_id="mutation-paint-only",
        scenario=scenario,
        equivalence="closest-legacy-reference",
        source_binary="nana-runtime-benchmark",
        source_report=str(source_paths.get("runtime", "")),
        mapping_notes=notes,
        metrics={
            "cpu_frame_ms": percentile_fields(case.get("local_paint_systems_ms")),
            "local_paint_commit_ms": percentile_fields(case.get("local_paint_commit_ms")),
            "local_paint_schedule_ms": percentile_fields(case.get("local_paint_schedule_ms")),
        },
        work_counters=work_counters,
    )


def _extract_nana_single_node_mutation(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    mutation_kind = scenario["params"]["kind"]
    tree_nodes = scenario["params"]["tree_nodes"]
    runtime = reports.get("runtime")
    if runtime is None:
        raise KeyError("nana-runtime-benchmark report required")
    case = find_case_by_nodes(runtime.get("cases", []), tree_nodes)
    if case is None:
        raise KeyError(f"nana-runtime-benchmark has no nodes={tree_nodes} case")
    block = (case.get("single_node_mutations") or {}).get(mutation_kind)
    if not isinstance(block, Mapping):
        raise KeyError(
            f"nana-runtime-benchmark nodes={tree_nodes} has no "
            f"single_node_mutations.{mutation_kind}"
        )
    if "systems_ms" not in block:
        raise KeyError(
            f"single_node_mutations.{mutation_kind}.systems_ms missing; "
            "current nana-runtime-benchmark schema is required"
        )
    work = counters_from_block(block.get("work"))
    notes = [
        f"Mapped onto nana-runtime-benchmark single_node_mutations.{mutation_kind} "
        f"at nodes={tree_nodes}.",
        "These drains share the 5k full case; they are not a dedicated Scenario process.",
    ]
    return envelope(
        runner="nana",
        status="ok",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence="closest-legacy-reference",
        source_binary="nana-runtime-benchmark",
        source_report=str(source_paths.get("runtime", "")),
        mapping_notes=notes,
        metrics={
            "cpu_frame_ms": percentile_fields(block.get("systems_ms")),
            "commit_ms": percentile_fields(block.get("commit_ms")),
            "schedule_ms": percentile_fields(block.get("schedule_ms")),
        },
        work_counters=work,
    )


def _extract_nana_hover(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    requested_nodes = scenario["params"]["nodes"]
    runtime = reports.get("runtime")
    if runtime is None:
        raise KeyError("nana-runtime-benchmark report required")
    cases = list(runtime.get("cases", []))
    available = [
        int(case["nodes"])
        for case in cases
        if "pointer_hover_transition_ms" in case and isinstance(case.get("nodes"), int)
    ]
    if not available:
        raise KeyError("pointer_hover_transition_ms missing from nana-runtime-benchmark report")
    if requested_nodes not in available:
        raise KeyError(
            f"nana-runtime-benchmark has no hover case nodes={requested_nodes}; "
            f"available={sorted(available)}. Refusing to substitute another tree size."
        )
    case = find_case_by_nodes(cases, requested_nodes)
    assert case is not None
    hover_work = counters_from_block(case.get("pointer_hover_work"))
    notes = [
        f"Mapped onto nana-runtime-benchmark pointer_hover_* at nodes={requested_nodes}.",
    ]
    if "layout_nodes" in hover_work:
        notes.append(
            "layout_nodes is measured from SystemWork/WorkCounters. "
            "The hover-without-size-change invariant is evaluable."
        )
    else:
        notes.append(
            "pointer_hover_work_nodes is not layout_nodes. "
            "layout_nodes is not measured in this envelope."
        )
    work_counters = {
        "pointer_hover_work_nodes": case.get("pointer_hover_work_nodes"),
        **hover_work,
    }
    return envelope(
        runner="nana",
        status="ok",
        scenario_id="hover",
        scenario=scenario,
        equivalence="closest-legacy-reference",
        source_binary="nana-runtime-benchmark",
        source_report=str(source_paths.get("runtime", "")),
        mapping_notes=notes,
        metrics={"cpu_frame_ms": percentile_fields(case.get("pointer_hover_transition_ms"))},
        work_counters=work_counters,
    )


def _extract_nana_virtual_list(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    params = scenario["params"]
    framework = reports.get("framework")
    if framework is None:
        raise KeyError("nana-framework-benchmark report required")
    items = params["items"]
    scale = None
    for row in framework.get("virtual_scales") or []:
        if row.get("kind") == "list" and row.get("logical_rows") == items:
            scale = row
            break
    notes = [
        f"Contract VirtualList items={items}, visible={params.get('visible')}, overscan={params.get('overscan')}.",
        "Gallery ui-benchmark lists remain full-layout and must not be cited as virtualization.",
    ]
    metrics: dict[str, Any] = {}
    work_counters: dict[str, Any] = {}
    source_binary = "nana-framework-benchmark"
    equivalence = "closest-legacy-reference"
    if scale is not None:
        notes.append(
            "Mapped onto nana-framework-benchmark virtual_scales[] "
            f"kind=list logical_rows={items} status={scale.get('status')}."
        )
        if scale.get("status") != "ok":
            raise KeyError(
                f"virtual_scales list/{items} status={scale.get('status')}: {scale.get('skip_reason')}"
            )
        metrics["cpu_frame_ms"] = percentile_fields(scale.get("materialize_ms"))
        metrics["window_ms"] = percentile_fields(scale.get("window_ms"))
        work_counters = {
            "live_ui_entities": scale.get("live_ui_entities"),
            "live_ui_entities_bound": scale.get("live_ui_entities_bound"),
            "visible_rows": scale.get("visible_rows"),
            "overscan_rows": scale.get("overscan_rows"),
        }
        window = catalog_virtual_list_window(params)
        equivalence, window_notes = nana_list_catalog_window_equivalence(
            scale, window, kind="list"
        )
        notes.extend(window_notes)
    elif items == 10_000 and "virtual_list_10k_materialize_ms" in framework:
        notes.append(
            "Mapped onto legacy virtual_list_10k_* fields (no virtual_scales in this report)."
        )
        notes.append(
            "Existing binary uses 10_000 rows, viewport 800px, overscan 200px, item extent 20px "
            "(about 40 visible + 10 overscan items). Contract params ask overscan=8 items."
        )
        metrics = {
            "cpu_frame_ms": percentile_fields(framework.get("virtual_list_10k_materialize_ms")),
            "virtual_list_10k_window_ms": percentile_fields(
                framework.get("virtual_list_10k_window_ms")
            ),
            "virtual_list_10k_update_ms": percentile_fields(
                framework.get("virtual_list_10k_update_ms")
            ),
            "virtual_scroll_40_visible_nodes_ms": percentile_fields(
                framework.get("virtual_scroll_40_visible_nodes_ms")
            ),
        }
    else:
        raise KeyError(
            f"nana-framework-benchmark has no virtual_scales list/{items} and no legacy 10k fields"
        )
    scenario_id = scenario["id"]
    return envelope(
        runner="nana",
        status="ok",
        scenario_id=scenario_id,
        scenario=scenario,
        equivalence=equivalence,
        source_binary=source_binary,
        source_report=str(source_paths.get("framework", "")),
        mapping_notes=notes,
        metrics={key: value for key, value in metrics.items() if value is not None},
        work_counters=work_counters,
    )


def _extract_nana_virtual_tree(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    params = scenario["params"]
    framework = reports.get("framework")
    if framework is None:
        raise KeyError("nana-framework-benchmark report required")
    items = params["items"]
    scale = None
    for row in framework.get("virtual_scales") or []:
        if row.get("kind") == "tree" and row.get("logical_rows") == items:
            scale = row
            break
    notes = [
        f"Contract VirtualTree items={items}, visible={params.get('visible')}, overscan={params.get('overscan')}.",
        "Mapped onto nana-framework-benchmark virtual_scales[] kind=tree "
        "(Fenwick VirtualTreeLayout + AppContext::materialize_virtual_tree). "
        "A VirtualList scale row is not this path.",
    ]
    if scale is None:
        raise KeyError(
            f"nana-framework-benchmark has no virtual_scales tree/{items}"
        )
    notes.append(
        "Mapped onto nana-framework-benchmark virtual_scales[] "
        f"kind=tree logical_rows={items} status={scale.get('status')}."
    )
    if scale.get("status") != "ok":
        raise KeyError(
            f"virtual_scales tree/{items} status={scale.get('status')}: {scale.get('skip_reason')}"
        )
    if scale.get("materialize_ms") is None or scale.get("live_ui_entities") is None:
        raise KeyError(
            f"virtual_scales tree/{items} status=ok without materialize_ms/live_ui_entities; "
            "fake empty ok is forbidden"
        )
    metrics = {
        "cpu_frame_ms": percentile_fields(scale.get("materialize_ms")),
        "window_ms": percentile_fields(scale.get("window_ms")),
    }
    work_counters = {
        "live_ui_entities": scale.get("live_ui_entities"),
        "live_ui_entities_bound": scale.get("live_ui_entities_bound"),
        "visible_rows": scale.get("visible_rows"),
        "overscan_rows": scale.get("overscan_rows"),
    }
    window = catalog_virtual_list_window(params)
    equivalence, window_notes = nana_list_catalog_window_equivalence(
        scale, window, kind="tree"
    )
    notes.extend(window_notes)
    return envelope(
        runner="nana",
        status="ok",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence=equivalence,
        source_binary="nana-framework-benchmark",
        source_report=str(source_paths.get("framework", "")),
        mapping_notes=notes,
        metrics={key: value for key, value in metrics.items() if value is not None},
        work_counters=work_counters,
    )


TEXT_TABLE_EXPORTED_SHAPE_KEYS = (
    "text_shaped",
    "text_shaped_runs",
    "text_layout_cache_hits",
    "text_layout_cache_misses",
    "text_wrap_layouts",
    "cache_eviction",
)

TEXT_TABLE_CACHE_GAPS = (
    "glyph_cache_hits",
    "glyph_cache_misses",
)


def _extract_nana_text_table(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    params = scenario["params"]
    framework = reports.get("framework")
    if framework is None:
        raise KeyError("nana-framework-benchmark report required")
    rows = params["rows"]
    columns = params["columns"]
    scale = None
    for row in framework.get("virtual_scales") or []:
        if row.get("kind") != "table" or row.get("logical_rows") != rows:
            continue
        logical_columns = row.get("logical_columns")
        if logical_columns is None or logical_columns == columns:
            scale = row
            break
    notes = [
        f"Contract Table rows={rows}, columns={columns}, "
        f"visible_rows={params.get('visible_rows')}, overscan_rows={params.get('overscan_rows')}.",
        "Mapped onto the existing nana-framework-benchmark virtual table scale path, "
        "not a dedicated Scenario process.",
        "shaping/cache: "
        + ", ".join(TEXT_TABLE_EXPORTED_SHAPE_KEYS)
        + " and extracted_text_spans copy WorkCounters when the binary exports "
        "virtual_scales[].work after shaping the "
        f"catalog wrapped_cells={params.get('wrapped_cells')} / "
        f"wrapped_cell_len={params.get('wrapped_cell_len')} cells. "
        + ", ".join(TEXT_TABLE_CACHE_GAPS)
        + " stay omitted on MeasureTextShaper (no glyph backend); "
        "do not invent zeros. Runtime GlyphCache is Some on NanaTextShaper.",
    ]
    metrics: dict[str, Any] = {}
    work_counters: dict[str, Any] = {}
    equivalence = "closest-legacy-reference"
    if scale is not None:
        notes.append(
            "Mapped onto nana-framework-benchmark virtual_scales[] "
            f"kind=table logical_rows={rows} logical_columns={columns} "
            f"status={scale.get('status')}."
        )
        if scale.get("status") != "ok":
            raise KeyError(
                f"virtual_scales table/{rows}x{columns} status={scale.get('status')}: "
                f"{scale.get('skip_reason')}"
            )
        metrics["cpu_frame_ms"] = percentile_fields(scale.get("materialize_ms"))
        metrics["window_ms"] = percentile_fields(scale.get("window_ms"))
        work_counters = {
            "live_ui_entities": scale.get("live_ui_entities"),
            "live_ui_entities_bound": scale.get("live_ui_entities_bound"),
            "visible_rows": scale.get("visible_rows"),
            "overscan_rows": scale.get("overscan_rows"),
            **counters_from_block(scale.get("work")),
        }
        if "text_shaped" in work_counters:
            notes.append(
                "text_shaped is WorkCounters.text_shaped (dirty TEXT set size this drain), "
                "the existing shaping-calls stand-in."
            )
        else:
            notes.append(
                "text_shaped is missing from this report. shaping calls/frame stay not-evaluable."
            )
        window = catalog_table_window(params)
        equivalence, window_notes = nana_table_catalog_window_equivalence(scale, window)
        notes.extend(window_notes)
        notes.append(
            "Most cells are short_cell_len labels; each 40-row band keeps wrapped_cells "
            "long wrapping cells (wrapped_cell_len) in column 0, shaped against the 80px column box."
        )
        if params.get("wrapped_cells") is not None:
            work_counters["wrapped_cells"] = params.get("wrapped_cells")
        if params.get("wrapped_cell_len") is not None:
            work_counters["wrapped_cell_len"] = params.get("wrapped_cell_len")
        if params.get("short_cell_len") is not None:
            work_counters["short_cell_len"] = params.get("short_cell_len")
    elif (
        rows == 10_000
        and columns == 100
        and "virtual_table_10k_x_100_materialize_ms" in framework
    ):
        notes.append(
            "Mapped onto legacy virtual_table_10k_x_100_* fields (no virtual_scales table row)."
        )
        notes.append(
            "Existing binary uses 10_000×100 logical cells, viewport 1280×800, "
            "overscan 160×200 px, row extent 20 px, column extent 80 px. "
            "Contract params ask visible_rows=40, overscan_rows=8, wrapped long cells."
        )
        notes.append(
            "Legacy 10k table fields have no WorkCounters; text_shaped and cache keys stay omitted."
        )
        metrics = {
            "cpu_frame_ms": percentile_fields(
                framework.get("virtual_table_10k_x_100_materialize_ms")
            ),
            "virtual_table_10k_x_100_window_ms": percentile_fields(
                framework.get("virtual_table_10k_x_100_window_ms")
            ),
            "virtual_table_column_resize_ms": percentile_fields(
                framework.get("virtual_table_column_resize_ms")
            ),
        }
    else:
        raise KeyError(
            f"nana-framework-benchmark has no virtual_scales table/{rows}x{columns} "
            "and no legacy virtual_table_10k_x_100_* fields"
        )
    return envelope(
        runner="nana",
        status="ok",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence=equivalence,
        source_binary="nana-framework-benchmark",
        source_report=str(source_paths.get("framework", "")),
        mapping_notes=notes,
        metrics={key: value for key, value in metrics.items() if value is not None},
        work_counters=work_counters,
    )


def _extract_nana_animation(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    active = scenario["params"]["active"]
    runtime = reports.get("runtime")
    if runtime is None:
        raise KeyError("nana-runtime-benchmark report required")
    case = runtime.get("catalog_animation")
    if not isinstance(case, Mapping):
        raise KeyError(
            "nana-runtime-benchmark has no catalog_animation; "
            "5k-tree sparse_animation_sample_ms is not the animation Scenario"
        )
    if case.get("id") not in (None, "animation"):
        raise KeyError(f"catalog_animation id={case.get('id')!r} is not animation")
    if case.get("status") not in (None, "ok"):
        raise KeyError(f"catalog_animation status={case.get('status')}")
    due = case.get("due_animation_samples")
    if due != active:
        raise KeyError(
            f"catalog_animation due_animation_samples={due} "
            f"does not match catalog active={active}"
        )
    if case.get("active") not in (None, active):
        raise KeyError(
            f"catalog_animation active={case.get('active')} "
            f"does not match catalog active={active}"
        )
    if scenario["params"].get("scheduled_idle"):
        if case.get("scheduled_idle") not in (None, True):
            raise KeyError("catalog_animation scheduled_idle must be true")
        if "idle_animation_deadline_ms" not in case:
            raise KeyError(
                "idle_animation_deadline_ms missing; catalog animation scheduled_idle=true"
            )
    notes = [
        "Mapped onto nana-runtime-benchmark catalog_animation on an isolated UiWorld.",
        "One due animation (active=1) plus idle-scheduled animations; "
        "advance_animations samples only the due set.",
        "Does not reuse the 5k-tree incidental sparse_animation_sample_ms.",
    ]
    work_counters = {
        "due_animation_samples": due,
        "scheduled_animations": case.get("scheduled_animations"),
    }
    return envelope(
        runner="nana",
        status="ok",
        scenario_id="animation",
        scenario=scenario,
        equivalence="closest-legacy-reference",
        source_binary="nana-runtime-benchmark",
        source_report=str(source_paths.get("runtime", "")),
        mapping_notes=notes,
        metrics={
            "cpu_frame_ms": percentile_fields(case.get("sparse_animation_sample_ms")),
            "idle_animation_deadline_ms": percentile_fields(
                case.get("idle_animation_deadline_ms")
            ),
            "scheduled_animation_deadline_ms": percentile_fields(
                case.get("scheduled_animation_deadline_ms")
            ),
        },
        work_counters=work_counters,
    )


def _find_catalog_workload(
    framework: Mapping[str, Any], scenario_id: str
) -> Mapping[str, Any]:
    for row in framework.get("catalog_workloads") or []:
        if isinstance(row, Mapping) and row.get("id") == scenario_id:
            return row
    raise KeyError(
        f"nana-framework-benchmark has no catalog_workloads id={scenario_id}"
    )


def _extract_nana_catalog_workload(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    framework = reports.get("framework")
    if framework is None:
        raise KeyError("nana-framework-benchmark report required")
    row = _find_catalog_workload(framework, scenario["id"])
    if row.get("status") != "ok":
        raise KeyError(
            f"catalog_workloads {scenario['id']} status={row.get('status')}: "
            f"{row.get('skip_reason')}"
        )
    if row.get("kind") not in (None, scenario["kind"]):
        raise KeyError(
            f"catalog_workloads {scenario['id']} kind={row.get('kind')!r} "
            f"does not match {scenario['kind']!r}"
        )
    params = scenario["params"]
    notes = [
        f"Mapped onto nana-framework-benchmark catalog_workloads[] id={scenario['id']}.",
        "These drains reuse existing Runtime APIs; they are not a dedicated Scenario process.",
    ]
    metrics: dict[str, Any] = {}
    work_counters: dict[str, Any] = {
        **counters_from_block(row.get("work")),
    }
    if row.get("live_ui_entities") is not None:
        work_counters["live_ui_entities"] = row.get("live_ui_entities")

    if scenario["kind"] == "Ime":
        scripts = list(row.get("scripts") or [])
        expected = list(params.get("scripts") or [])
        if scripts != expected:
            raise KeyError(
                f"catalog_workloads ime scripts={scripts} do not match catalog {expected}"
            )
        if params.get("include_preedit") and "preedit_ms" not in row:
            raise KeyError("catalog_workloads ime missing preedit_ms")
        if params.get("include_candidate_commit") and "commit_ms" not in row:
            raise KeyError("catalog_workloads ime missing commit_ms")
        metrics["cpu_frame_ms"] = percentile_fields(row.get("commit_ms"))
        metrics["preedit_ms"] = percentile_fields(row.get("preedit_ms"))
        work_counters["ime_script_count"] = row.get("ime_script_count", len(scripts))
        notes.append(
            "set_ime_preedit + commit_ime on a focused TextInput. "
            "Does not measure the OS IME candidate window; "
            "commit_ime is the Runtime candidate-commit path only."
        )
    elif scenario["kind"] == "DockWorkspace":
        panes = row.get("panes")
        if panes != params.get("panes"):
            raise KeyError(
                f"catalog_workloads dock-workspace panes={panes} "
                f"does not match catalog panes={params.get('panes')}"
            )
        if params.get("include_splitter_resize") and "resize_ms" not in row:
            raise KeyError("catalog_workloads dock-workspace missing resize_ms")
        metrics["cpu_frame_ms"] = percentile_fields(row.get("resize_ms"))
        work_counters["panes"] = panes
        notes.append(
            "assemble_dock of an 8-leaf DockNode tree plus adjust_focused_dock_split. "
            "This is keyboard/API splitter resize, not a pointer-drag splitter."
        )
    elif scenario["kind"] == "Overlay":
        kinds = list(row.get("overlay_kinds") or [])
        expected = list(params.get("kinds") or [])
        if kinds != expected:
            raise KeyError(
                f"catalog_workloads overlay kinds={kinds} do not match catalog {expected}"
            )
        metrics["cpu_frame_ms"] = percentile_fields(row.get("activate_ms"))
        work_counters["overlay_kind_count"] = row.get("overlay_kind_count", len(kinds))
        notes.append(
            "OverlayHost activate_overlay/dismiss_overlay for Tooltip / Menu / Dialog; "
            "toggle_popover for popup. Measures activate/dismiss only; "
            "does not measure popup pointer-follow reposition."
        )
    else:
        chars = row.get("document_chars")
        if chars != params.get("document_chars"):
            raise KeyError(
                f"catalog_workloads text-editor document_chars={chars} "
                f"does not match catalog {params.get('document_chars')}"
            )
        metrics["cpu_frame_ms"] = percentile_fields(row.get("local_edit_ms"))
        if row.get("visible_lines") is not None:
            work_counters["visible_lines"] = row.get("visible_lines")
        work_counters["document_chars"] = chars
        notes.append(
            "TextArea holding catalog document_chars with a 40-line viewport height. "
            "Measures caret-local replace_text_area_selection, not a full-document reshape."
        )
        if "text_shaped" in work_counters:
            notes.append("text_shaped is WorkCounters.text_shaped after the local edit drain.")
        else:
            notes.append(
                "text_shaped is missing from this report. local-edit shaping stays not-evaluable."
            )

    return envelope(
        runner="nana",
        status="ok",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence="closest-legacy-reference",
        source_binary="nana-framework-benchmark",
        source_report=str(source_paths.get("framework", "")),
        mapping_notes=notes,
        metrics={key: value for key, value in metrics.items() if value is not None},
        work_counters=work_counters,
    )


def _synthetic_gpu_ui_only(
    scenario: Mapping[str, Any],
    *,
    frame_status: str = "ran",
    include_frame: bool = True,
    materialization: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    params = scenario["params"]
    payload: dict[str, Any] = {
        "status": "ok",
        "scenario_id": scenario["id"],
        "composition": "UiOnly",
        "adapter": "test",
        "materialization": dict(materialization)
        if materialization is not None
        else {
            "viewport": params["viewport"],
            "host_texture": params["host_texture"],
            "ui_nodes": params["ui_nodes"],
            "ui_entity_count": len(params["ui_nodes"]),
            "scene_primitive_kinds": ["host-texture", "quad", "text"],
        },
        "gpu_work": {
            "gpu_upload_bytes": 128,
            "draw_calls": 3,
            "draw_batches": 2,
            "batch_rebuilds": 1,
            "gpu_buffer_reallocations": 0,
        },
        "stages": {
            "batch_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
            "gpu_upload_ms": {"p50": 0.05, "p95": 0.08, "p99": 0.1},
            "encode_ms": {"p50": 0.4, "p95": 0.5, "p99": 0.6},
            "submit_ms": {"p50": 0.02, "p95": 0.03, "p99": 0.04},
        },
    }
    if include_frame:
        payload["frame_stages"] = {
            name: {"status": frame_status} for name in GPU_HOST_STAGES
        }
    return payload


def _extract_nana_gpu_scene(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    composition = scenario["params"]["composition"]
    skip = nana_gpu_scene_skip_reason(scenario)
    if skip:
        raise KeyError(skip)
    payload = reports.get("gpu")
    if payload is None:
        raise KeyError("nana-gpu-scene-benchmark report required")
    if payload.get("status") == "unsupported":
        raise KeyError(
            payload.get("unsupported_reason")
            or "nana-gpu-scene-benchmark reported unsupported without a reason"
        )
    if payload.get("status") != "ok":
        raise KeyError(
            f"nana-gpu-scene-benchmark status={payload.get('status')} is not an encoded GPU frame"
        )
    if payload.get("scenario_id") != scenario["id"]:
        raise KeyError(
            f"nana-gpu-scene-benchmark scenario_id {payload.get('scenario_id')!r} "
            f"does not match catalog {scenario['id']}"
        )
    if payload.get("composition") != "UiOnly":
        raise KeyError(
            f"nana-gpu-scene-benchmark composition {payload.get('composition')!r} "
            "does not match gpu-scene-ui UiOnly"
        )
    _require_ui_only_materialization(scenario, payload)
    _require_ran_gpu_frame_stages(payload)
    work = gpu_counters_from_observed(payload.get("gpu_work"))
    stages = payload.get("stages") if isinstance(payload.get("stages"), dict) else {}
    metrics = {
        "batch_ms": percentile_fields(stages.get("batch_ms")),
        "gpu_upload_ms": percentile_fields(stages.get("gpu_upload_ms")),
        "encode_ms": percentile_fields(stages.get("encode_ms")),
        "submit_ms": percentile_fields(stages.get("submit_ms")),
    }
    missing = [key for key, value in metrics.items() if value is None]
    if missing:
        raise KeyError(
            f"nana-gpu-scene-benchmark FrameProfiler ran GPU stages but omitted {missing}"
        )
    notes = [
        "Loaded perf/scenarios/gpu-scene-ui.json and materialized its UiOnly params "
        "(viewport, host_texture slot, ui_nodes).",
        "RuntimeDocument flush + HostTexture content slot + SceneWgpuPainter encode/submit. "
        "Not a private hosted-gpu-demo tree. No CPU readback.",
        "gpu_upload_bytes counts observed queue.write_buffer on that path. cryoglyph atlas "
        "uploads are not estimated.",
        "Batch / GpuUpload / Encode / Submit are FrameProfiler status=ran on the same "
        "encode/submit. Runtime-only drains keep those stages unsupported and omit GPU keys.",
    ]
    if payload.get("adapter"):
        notes.append(f"Adapter: {payload['adapter']}.")
    report = envelope(
        runner="nana",
        status="ok",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence="same-scenario",
        source_binary="nana-gpu-scene-benchmark",
        source_report=str(source_paths.get("gpu", "")),
        mapping_notes=notes,
        metrics=metrics,
        work_counters=work,
    )
    report["frame_stages"] = {
        name: {"status": "ran"} for name in GPU_HOST_STAGES
    }
    return report


def _require_ui_only_materialization(
    scenario: Mapping[str, Any],
    payload: Mapping[str, Any],
) -> None:
    params = scenario["params"]
    echoed = payload.get("materialization")
    if not isinstance(echoed, dict):
        raise KeyError(
            "nana-gpu-scene-benchmark must echo scenario materialization "
            "(viewport, host_texture, ui_nodes); do not run a private demo tree"
        )
    for key in ("viewport", "host_texture", "ui_nodes"):
        if echoed.get(key) != params.get(key):
            raise KeyError(
                f"nana-gpu-scene-benchmark materialization.{key}={echoed.get(key)!r} "
                f"does not match scenario JSON {params.get(key)!r}"
            )
    kinds = echoed.get("scene_primitive_kinds") or []
    if "host-texture" not in kinds:
        raise KeyError(
            "UiOnly scene must contain a host-texture primitive for the GPU content slot"
        )
    if not any(kind in kinds for kind in ("text", "quad")):
        raise KeyError(
            "UiOnly scene must contain UI primitives (text/quad), not a slot-only triangle"
        )
    entities = echoed.get("ui_entity_count")
    expected = len(params.get("ui_nodes") or [])
    if not isinstance(entities, int) or isinstance(entities, bool) or entities < expected:
        raise KeyError(
            f"UiOnly ui_entity_count={entities!r} must cover catalog ui_nodes ({expected})"
        )


def _require_ran_gpu_frame_stages(payload: Mapping[str, Any]) -> None:
    stages = payload.get("frame_stages")
    if not isinstance(stages, dict):
        raise KeyError(
            "nana-gpu-scene-benchmark ok report must include frame_stages from FrameProfiler "
            "on the same encode/submit"
        )
    for name in GPU_HOST_STAGES:
        status = (stages.get(name) or {}).get("status")
        if status != "ran":
            raise KeyError(
                f"gpu-scene-ui FrameStage.{name} status={status!r}; "
                "export timings only when a host encoded/submitted (status=ran). "
                "Runtime-only profilers keep these unsupported."
            )


def extract_iced(
    scenario: Mapping[str, Any],
    report: Mapping[str, Any],
    *,
    source_path: Path,
) -> dict[str, Any]:
    if is_iced_scenario_bench_report(report):
        return _extract_iced_scenario_bench(scenario, report, source_path=source_path)
    kind = scenario["kind"]
    cases = {case.get("scenario"): case for case in report.get("cases", [])}
    if kind == "StaticTree":
        nodes = scenario["params"]["nodes"]
        name = {100: "list-100", 1000: "list-1000"}.get(nodes)
        if name is None or name not in cases:
            raise KeyError(
                f"ui-benchmark has no list case for StaticTree nodes={nodes}; "
                "5k/10k/50k are unsupported on this legacy path. "
                "Use engine/iced scenario-bench for same-scenario StaticTree."
            )
        case = cases[name]
        return envelope(
            runner="iced",
            status="ok",
            scenario_id=scenario["id"],
            scenario=scenario,
            equivalence="closest-legacy-reference",
            source_binary="ui-benchmark",
            source_report=str(source_path),
            mapping_notes=[
                f"Mapped onto Gallery ui-benchmark `{name}`.",
                "Gallery lists still layout every item (legacy full-layout). This is a reference, not virtualization.",
                "Current ui-benchmark paints through SceneWgpuPainter; historical numbers in "
                "docs/performance-baseline.md were taken on the Iced Gallery path. "
                "same-scenario StaticTree uses engine/iced scenario-bench, not this Gallery wrap.",
            ],
            metrics={
                "cpu_frame_ms": percentile_fields(case.get("cpu_total_ms")),
                "total_ms": percentile_fields(case.get("total_ms")),
                "view_construction_ms": percentile_fields(case.get("view_construction_ms")),
                "layout_diff_ms": percentile_fields(case.get("layout_diff_ms")),
            },
        )
    if kind in {
        "Mutation",
        "Hover",
        "VirtualList",
        "VirtualTree",
        "Table",
        "Animation",
        "Ime",
        "DockWorkspace",
        "Overlay",
        "TextEditor",
        "GpuScene",
    }:
        raise KeyError(
            f"ui-benchmark / engine/iced scenario-bench has no same-Scenario {kind} adapter. "
            "Gallery lists are not a substitute. Required by #8 / not implemented. "
            "Fake Iced numbers are forbidden."
        )
    raise KeyError(f"no Iced mapping for {scenario['id']}")


def _extract_iced_scenario_bench(
    scenario: Mapping[str, Any],
    report: Mapping[str, Any],
    *,
    source_path: Path,
) -> dict[str, Any]:
    if report.get("status") == "unsupported":
        raise KeyError(
            report.get("unsupported_reason")
            or f"iced-scenario-bench unsupported for {scenario['id']}"
        )
    skip = iced_scenario_bench_skip_reason(scenario)
    if skip:
        raise KeyError(skip)
    kind = scenario["kind"]
    reported_id = report.get("scenario_id")
    if reported_id not in (None, scenario["id"]):
        reported_nodes = report.get("nodes")
        expected_nodes = (scenario.get("params") or {}).get("nodes") or (
            scenario.get("params") or {}
        ).get("tree_nodes")
        detail = ""
        if reported_nodes is not None or expected_nodes is not None:
            detail = f" (report nodes={reported_nodes}, scenario nodes={expected_nodes})"
        raise KeyError(
            f"iced-scenario-bench scenario_id={reported_id!r} does not match "
            f"{scenario['id']!r}{detail}"
        )
    cpu = percentile_fields(report.get("cpu_frame_ms"))
    if cpu is None:
        raise KeyError(
            "iced-scenario-bench ok report missing cpu_frame_ms percentiles; "
            "fake or empty timings are forbidden"
        )
    notes = [str(note) for note in (report.get("notes") or []) if note is not None]
    notes.append(
        "Relative Iced/GPUI gates stay off until GPUI also emits same-scenario ok "
        "with real metrics on this id."
    )
    metrics: dict[str, Any] = {
        "cpu_frame_ms": cpu,
        "view_construction_ms": percentile_fields(report.get("view_construction_ms")),
        "layout_ms": percentile_fields(report.get("layout_ms")),
        "draw_ms": percentile_fields(report.get("draw_ms")),
        "present_ms": percentile_fields(report.get("present_ms")),
        "window_ms": percentile_fields(report.get("window_ms")),
    }
    work_counters: dict[str, Any] | None = None
    if kind == "StaticTree":
        nodes = scenario["params"]["nodes"]
        reported = report.get("nodes")
        if reported != nodes:
            raise KeyError(
                f"iced-scenario-bench nodes={reported} does not match StaticTree nodes={nodes}"
            )
        tree = report.get("tree")
        if not is_shared_static_tree(tree if isinstance(tree, Mapping) else None, nodes):
            raise KeyError(
                "iced-scenario-bench ok report is not the shared StaticTree heap "
                "(generation=complete-binary-heap, parent(i)=i//2, element-div, no text). "
                "A column of N text leaves is not same-scenario."
            )
        notes.append(
            "Mapped onto engine/iced static_tree / static_tree_parent, the same "
            "complete-binary-heap rule as nana-runtime-benchmark::tree_mutations."
        )
        metrics["frames_after_idle"] = read_frames_after_idle(
            report, required=True, source="iced-scenario-bench StaticTree"
        )
        busy_probe = report.get("busy_probe_frames")
        if isinstance(busy_probe, bool) or not isinstance(busy_probe, int) or busy_probe < 1:
            raise KeyError(
                "iced-scenario-bench StaticTree must export busy_probe_frames > 0 from the "
                "live BusyPulse probe; refusing a stuffed frames_after_idle=0"
            )
        metrics["busy_probe_frames"] = busy_probe
    elif kind == "Mutation":
        nodes = scenario["params"]["tree_nodes"]
        mutation_kind = scenario["params"]["kind"]
        if report.get("nodes") != nodes:
            raise KeyError(
                f"iced-scenario-bench nodes={report.get('nodes')} does not match "
                f"Mutation tree_nodes={nodes}"
            )
        mutation = report.get("mutation") if isinstance(report.get("mutation"), Mapping) else {}
        if mutation.get("kind") != mutation_kind:
            raise KeyError(
                f"iced-scenario-bench mutation.kind={mutation.get('kind')!r} does not match "
                f"{mutation_kind!r}"
            )
        if mutation.get("single_node") is not True:
            raise KeyError("iced-scenario-bench Mutation report must set mutation.single_node=true")
        tree = report.get("tree")
        if not is_shared_static_tree(tree if isinstance(tree, Mapping) else None, nodes):
            raise KeyError(
                "iced-scenario-bench Mutation tree is not the shared complete-binary-heap"
            )
        notes.append(
            f"Mapped onto engine/iced scenario-bench Mutation {mutation_kind} at "
            f"tree_nodes={nodes}, same heap as Nana tree_mutations. Single-node change; "
            "Iced has no WorkCounters.layout_nodes so paint/a11y layout invariants stay "
            "not-evaluable."
        )
    elif kind == "Hover":
        nodes = scenario["params"]["nodes"]
        if report.get("nodes") != nodes:
            raise KeyError(
                f"iced-scenario-bench nodes={report.get('nodes')} does not match Hover nodes={nodes}"
            )
        tree = report.get("tree")
        if not is_shared_static_tree(tree if isinstance(tree, Mapping) else None, nodes):
            raise KeyError(
                "iced-scenario-bench Hover tree is not the shared complete-binary-heap"
            )
        notes.append(
            f"Mapped onto engine/iced scenario-bench Hover at nodes={nodes}. "
            "Same heap as Nana tree_mutations; last two nodes toggle hover style. "
            "Iced has no WorkCounters.layout_nodes; hover_without_size_change stays "
            "not-evaluable."
        )
        work_counters = {"nodes": nodes}
    elif kind == "VirtualList":
        params = scenario["params"]
        items = params["items"]
        virtual = report.get("virtualization")
        if not isinstance(virtual, Mapping):
            raise KeyError(
                "iced-scenario-bench VirtualList ok report missing virtualization block"
            )
        if virtual.get("logical_items") != items:
            raise KeyError(
                f"iced-scenario-bench logical_items={virtual.get('logical_items')} "
                f"does not match VirtualList items={items}"
            )
        live = virtual.get("live_ui_entities")
        bound = virtual.get("live_ui_entities_bound")
        if not isinstance(live, int) or live <= 0:
            raise KeyError(
                "iced-scenario-bench VirtualList must report a positive live_ui_entities count"
            )
        if live == items:
            raise KeyError(
                f"iced-scenario-bench VirtualList live_ui_entities={live} equals logical "
                f"items={items}; that is a full widget list, not Nana virtualization"
            )
        if isinstance(bound, int) and live > bound:
            raise KeyError(
                f"iced-scenario-bench live_ui_entities={live} exceeds bound={bound}"
            )
        window = catalog_virtual_list_window(params)
        if virtual.get("visible") != window["visible"]:
            raise KeyError(
                f"iced-scenario-bench VirtualList visible={virtual.get('visible')} "
                f"does not match catalog visible={window['visible']}"
            )
        if virtual.get("overscan") != window["overscan"]:
            raise KeyError(
                f"iced-scenario-bench VirtualList overscan={virtual.get('overscan')} "
                f"does not match catalog overscan={window['overscan']} items"
            )
        if not _same_number(virtual.get("item_extent_px"), window["item_extent_px"]):
            raise KeyError(
                f"iced-scenario-bench VirtualList item_extent_px={virtual.get('item_extent_px')} "
                f"does not match catalog item_extent_px={window['item_extent_px']}"
            )
        notes.append(
            f"Mapped onto engine/iced scenario-bench VirtualList items={items} "
            f"visible={window['visible']} overscan={window['overscan']} "
            f"({window['overscan_px']}px) item_extent={window['item_extent_px']} "
            f"(viewport {window['viewport_px']}px). Only the catalog window is materialized."
        )
        notes.append(
            "Nana runner passes the same catalog window into nana-framework-benchmark "
            "via --list-viewport-px / --list-overscan-px / --list-item-extent-px."
        )
        work_counters = {
            "live_ui_entities": live,
            "live_ui_entities_bound": bound,
            "visible_rows": virtual.get("visible"),
            "overscan_rows": virtual.get("overscan"),
        }
    elif kind == "Table":
        params = scenario["params"]
        rows = params["rows"]
        columns = params["columns"]
        virtual = report.get("virtualization")
        if not isinstance(virtual, Mapping):
            raise KeyError(
                "iced-scenario-bench Table ok report missing virtualization block"
            )
        if virtual.get("logical_rows") != rows or virtual.get("logical_columns") != columns:
            raise KeyError(
                f"iced-scenario-bench Table logical={virtual.get('logical_rows')}x"
                f"{virtual.get('logical_columns')} does not match catalog "
                f"{rows}x{columns}"
            )
        live = virtual.get("live_ui_entities")
        bound = virtual.get("live_ui_entities_bound")
        if not isinstance(live, int) or live <= 0:
            raise KeyError(
                "iced-scenario-bench Table must report a positive live_ui_entities count"
            )
        if live == rows * columns:
            raise KeyError(
                f"iced-scenario-bench Table live_ui_entities={live} equals logical "
                f"cells={rows}x{columns}; that is a full table, not Nana virtualization"
            )
        if isinstance(bound, int) and live > bound:
            raise KeyError(
                f"iced-scenario-bench Table live_ui_entities={live} exceeds bound={bound}"
            )
        window = catalog_table_window(params)
        if virtual.get("visible_rows") != window["visible_rows"]:
            raise KeyError(
                f"iced-scenario-bench Table visible_rows={virtual.get('visible_rows')} "
                f"does not match catalog visible_rows={window['visible_rows']}"
            )
        if virtual.get("overscan_rows") != window["overscan_rows"]:
            raise KeyError(
                f"iced-scenario-bench Table overscan_rows={virtual.get('overscan_rows')} "
                f"does not match catalog overscan_rows={window['overscan_rows']}"
            )
        if virtual.get("visible_columns") != window["visible_columns"]:
            raise KeyError(
                f"iced-scenario-bench Table visible_columns={virtual.get('visible_columns')} "
                f"does not match catalog visible_columns={window['visible_columns']}"
            )
        if virtual.get("overscan_columns") != window["overscan_columns"]:
            raise KeyError(
                f"iced-scenario-bench Table overscan_columns={virtual.get('overscan_columns')} "
                f"does not match catalog overscan_columns={window['overscan_columns']}"
            )
        invented_shape = None
        if isinstance(report.get("work_counters"), Mapping):
            invented_shape = report["work_counters"].get("text_shaped")
        if invented_shape is not None:
            raise KeyError(
                "iced-scenario-bench must not invent WorkCounters.text_shaped; "
                "leave the catalog invariant not-evaluable"
            )
        notes.append(
            f"Mapped onto engine/iced scenario-bench Table {rows}x{columns} "
            f"visible={window['visible_rows']}x{window['visible_columns']} "
            f"overscan={window['overscan_rows']}x{window['overscan_columns']} "
            f"(viewport {window['viewport_width_px']}x{window['viewport_height_px']}px, "
            f"overscan {window['overscan_x_px']}x{window['overscan_y_px']}px). "
            "Only the catalog window is materialized."
        )
        notes.append(
            "Nana runner passes the same catalog window into nana-framework-benchmark "
            "via --table-viewport-*-px / --table-overscan-*-px / --table-*-extent-px."
        )
        work_counters = {
            "live_ui_entities": live,
            "live_ui_entities_bound": bound,
            "visible_rows": virtual.get("visible_rows"),
            "overscan_rows": virtual.get("overscan_rows"),
            "visible_columns": virtual.get("visible_columns"),
            "overscan_columns": virtual.get("overscan_columns"),
        }
    else:
        raise KeyError(
            f"iced-scenario-bench has no same-scenario mapping for {scenario['id']} "
            f"(kind={kind})"
        )
    return envelope(
        runner="iced",
        status="ok",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence="same-scenario",
        source_binary="scenario-bench",
        source_report=str(source_path),
        mapping_notes=notes,
        metrics={key: value for key, value in metrics.items() if value is not None},
        work_counters=work_counters,
    )


def _scale_token(nodes: int) -> str:
    if nodes >= 1000 and nodes % 1000 == 0:
        return f"{nodes // 1000}k"
    return str(nodes)


def _extract_nana_framework_fixture(
    root: Path, scenario: Mapping[str, Any] | str, fixture_name: str
) -> dict[str, Any]:
    path = root / "perf" / "fixtures" / fixture_name
    loaded = load_scenario(scenario, root) if isinstance(scenario, str) else scenario
    return extract_nana(
        loaded,
        {"framework": load_json(path)},
        source_paths={"framework": path},
    )


def _require_same_scenario(
    errors: list[str],
    report: Mapping[str, Any],
    label: str,
    *,
    live_ui_entities: int | None = None,
    shared_note: str | None = None,
) -> None:
    if report.get("status") != "ok" or report.get("equivalence") != "same-scenario":
        errors.append(f"{label} live dump with catalog window must be same-scenario")
    if report.get("relative_gate_enforceable") is not False:
        errors.append(f"{label} same-scenario must keep relative_gate_enforceable False")
    if live_ui_entities is not None and (report.get("work_counters") or {}).get(
        "live_ui_entities"
    ) != live_ui_entities:
        errors.append(f"{label} must copy live_ui_entities={live_ui_entities}")
    if shared_note is not None and not any(
        shared_note in str(note) for note in (report.get("mapping_notes") or [])
    ):
        errors.append(f"{label} notes must mention {shared_note}")


def _named_invariant(report: Mapping[str, Any], name: str) -> dict[str, Any] | None:
    for item in report.get("invariants") or []:
        if item.get("name") == name and isinstance(item, dict):
            return item
    return None


def _named_invariant_status(report: Mapping[str, Any], name: str) -> str | None:
    item = _named_invariant(report, name)
    if item is None or item.get("status") is None:
        return None
    return str(item.get("status"))


def _named_invariant_ok_measured(report: Mapping[str, Any], name: str, measured: Any) -> bool:
    item = _named_invariant(report, name)
    return (
        item is not None
        and item.get("status") == "ok"
        and item.get("measured") == measured
    )


def self_test(root: Path | None = None) -> list[str]:
    """Validate schema and extractors against checked-in historical reports."""
    root = root or REPO_ROOT
    errors = validate_all_scenarios(root)
    runtime_path = (
        root / "docs" / "performance" / "2026-08-14-issue7-phase3-runtime.json"
    )
    if not runtime_path.is_file():
        runtime_path = root / "docs" / "performance" / "2026-08-14-issue7-phase2-runtime.json"
    framework_path = root / "docs" / "performance" / "2026-08-14-issue7-phase4-framework.json"
    scene_path = root / "docs" / "performance" / "2026-08-14-issue7-phase6-scene.json"
    iced_path = root / "docs" / "performance" / "2026-08-14-issue7-phase0-iced.json"

    static_tree = load_scenario("static-tree-100", root)
    try:
        nana_historical = extract_nana(
            static_tree,
            {
                "runtime": load_json(runtime_path),
                "scene": load_json(scene_path),
            },
            source_paths={"runtime": runtime_path, "scene": scene_path},
        )
        if "frames_after_idle" in (nana_historical.get("metrics") or {}):
            errors.append(
                "historical nana-runtime-benchmark JSON must not invent frames_after_idle"
            )
        historical_judge = judge_runner_invariants(nana_historical, root=root)
        if historical_judge.get("decision") != "skipped":
            errors.append(
                "historical StaticTree without frames_after_idle must stay skipped, not vacuous ok"
            )
    except Exception as exc:  # noqa: BLE001 — self-test must surface mapper failures
        errors.append(f"nana static-tree-100 extract failed: {exc}")

    iced_static = extract_iced(
        static_tree, load_json(iced_path), source_path=iced_path
    )
    if iced_static.get("status") != "ok":
        errors.append("iced static-tree-100 extract did not return ok")
    if iced_static.get("equivalence") != "closest-legacy-reference":
        errors.append("Gallery ui-benchmark extract must stay closest-legacy-reference")

    virtual = load_scenario("virtual-list-10k", root)
    try:
        nana_virtual = extract_nana(
            virtual,
            {"framework": load_json(framework_path)},
            source_paths={"framework": framework_path},
        )
        if nana_virtual.get("status") != "ok":
            errors.append("nana virtual-list-10k extract did not return ok")
        if nana_virtual.get("equivalence") != "closest-legacy-reference":
            errors.append(
                "historical nana virtual-list-10k without list_overscan_px must stay "
                "closest-legacy-reference"
            )
    except Exception as exc:  # noqa: BLE001
        errors.append(f"nana virtual-list-10k extract failed: {exc}")

    virtual_100k = load_scenario("virtual-list-100k", root)
    scaled = extract_nana(
        virtual_100k,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "list",
                        "logical_rows": 100000,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 10,
                        "live_ui_entities": 50,
                        "live_ui_entities_bound": 50,
                        "materialize_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic")},
    )
    if scaled.get("status") != "ok" or scaled.get("work_counters", {}).get("live_ui_entities") != 50:
        errors.append("nana virtual-list-100k virtual_scales extract failed")
    if scaled.get("equivalence") != "closest-legacy-reference":
        errors.append(
            "nana virtual-list-100k without list_overscan_px must stay closest-legacy-reference"
        )
    if not any(
        "list_overscan_px" in str(note) for note in (scaled.get("mapping_notes") or [])
    ):
        errors.append("nana virtual-list-100k missing-window notes must name list_overscan_px")
    try:
        extract_nana(
            virtual_100k,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "list",
                            "logical_rows": 100000,
                            "status": "ok",
                            "visible_rows": 40,
                            "overscan_rows": 10,
                            "live_ui_entities": 50,
                            "list_viewport_px": 800.0,
                            "list_overscan_px": 200.0,
                            "list_item_extent_px": 20.0,
                            "materialize_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-list-200px")},
        )
        errors.append("nana virtual-list-100k with list_overscan_px=200 must KeyError")
    except KeyError as exc:
        reason = key_error_reason(exc)
        if "200" not in reason and "overscan" not in reason:
            errors.append(f"mismatched list overscan KeyError should name overscan/200: {exc}")
    list_same = _extract_nana_framework_fixture(
        root, virtual_100k, "virtual-scales-only.json"
    )
    _require_same_scenario(
        errors,
        list_same,
        "nana virtual-list-100k",
        live_ui_entities=57,
        shared_note="Shared catalog list window",
    )
    list_10k_live = _extract_nana_framework_fixture(
        root, virtual, "virtual-scales-only.json"
    )
    _require_same_scenario(errors, list_10k_live, "nana virtual-list-10k")

    paint = load_scenario("mutation-paint-only", root)
    hover = load_scenario("hover", root)

    try:
        painted = extract_nana(
            paint,
            {"runtime": load_json(runtime_path)},
            source_paths={"runtime": runtime_path},
        )
        if painted.get("status") != "ok":
            errors.append("nana mutation-paint-only extract did not return ok")
        counters = painted.get("work_counters") or {}
        if "layout_nodes" in counters:
            errors.append("ok envelope must not serialize unmeasured layout_nodes")
        paint_inv = _named_invariant(painted, "paint_only_does_not_layout_full_tree")
        if paint_inv is None or paint_inv.get("status") != "not-evaluable":
            errors.append(
                "historical paint extract must leave layout_nodes invariant not-evaluable"
            )
        if paint_inv and "measured" in paint_inv:
            errors.append("not-evaluable paint invariant must omit measured")
    except Exception as exc:  # noqa: BLE001
        errors.append(f"nana paint extract failed: {exc}")

    paint_measured = extract_nana(
        paint,
        {
            "runtime": {
                "cases": [
                    {
                        "nodes": 5000,
                        "local_paint_systems_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "local_paint_commit_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "local_paint_schedule_ms": {"p50": 0.0, "p95": 0.0, "p99": 0.0},
                        "local_paint_work_nodes": 1,
                        "local_paint_work": {
                            "layout_nodes": 0,
                            "entities_total": 5000,
                            "entities_changed": 1,
                            "entities_spawned": 0,
                            "render_nodes_extracted": 1,
                            "extracted_text_spans": 0,
                            "gpu_upload_bytes": None,
                        },
                    }
                ]
            }
        },
        source_paths={"runtime": Path("synthetic-paint-counters")},
    )
    if paint_measured.get("status") != "ok":
        errors.append("nana paint extract with layout_nodes=0 must be ok")
    measured_counters_block = paint_measured.get("work_counters") or {}
    if measured_counters_block.get("layout_nodes") != 0:
        errors.append("ok envelope must contain numeric layout_nodes when fixture has it")
    if "gpu_upload_bytes" in measured_counters_block:
        errors.append("ok envelope must omit null work-counter fields")
    paint_cpu_zero = extract_nana(
        paint,
        {
            "runtime": {
                "cases": [
                    {
                        "nodes": 5000,
                        "local_paint_systems_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "local_paint_commit_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "local_paint_schedule_ms": {"p50": 0.0, "p95": 0.0, "p99": 0.0},
                        "local_paint_work_nodes": 1,
                        "local_paint_work": {
                            "layout_nodes": 0,
                            "entities_total": 5000,
                            "gpu_upload_bytes": 0,
                            "draw_calls": 0,
                            "draw_batches": 0,
                            "batch_rebuilds": 0,
                            "gpu_buffer_reallocations": 0,
                        },
                    }
                ]
            }
        },
        source_paths={"runtime": Path("synthetic-paint-cpu-gpu-zero")},
    )
    cpu_zero_counters = paint_cpu_zero.get("work_counters") or {}
    for key in GPU_WORK_COUNTER_KEYS:
        if key in cpu_zero_counters:
            errors.append(f"CPU drain must not serialize {key}=0 as an observed GPU count")
    if (paint_cpu_zero.get("metrics") or {}).get("gpu_upload_ms"):
        errors.append("Runtime-only paint envelope must not export gpu_upload_ms")
    if (paint_cpu_zero.get("frame_stages") or {}).get("GpuUpload", {}).get("status") == "ran":
        errors.append("Runtime-only paint envelope must not mark GPU stages ran")
    paint_ok_inv = _named_invariant(paint_measured, "paint_only_does_not_layout_full_tree")
    if (
        paint_ok_inv is None
        or paint_ok_inv.get("status") != "ok"
        or paint_ok_inv.get("measured") != 0
    ):
        errors.append("paint invariant must be ok with measured layout_nodes=0")

    gpu_scene = load_scenario("gpu-scene-ui", root)
    gpu_ok = extract_nana(
        gpu_scene,
        {"gpu": _synthetic_gpu_ui_only(gpu_scene)},
        source_paths={"gpu": Path("synthetic-gpu-scene")},
    )
    if gpu_ok.get("status") != "ok":
        errors.append("nana gpu-scene-ui extract with observed GPU work must be ok")
    gpu_counters = gpu_ok.get("work_counters") or {}
    if gpu_counters.get("gpu_upload_bytes") != 128:
        errors.append("gpu-scene-ui envelope must copy observed gpu_upload_bytes")
    if gpu_counters.get("draw_calls") != 3:
        errors.append("gpu-scene-ui envelope must copy observed draw_calls")
    if gpu_counters.get("gpu_buffer_reallocations") != 0:
        errors.append("gpu-scene-ui Some(0) reallocs after encode must stay numeric 0")
    gpu_upload_inv = _named_invariant(gpu_ok, "gpu_upload_bytes_observed_on_encode")
    if gpu_upload_inv is None or gpu_upload_inv.get("status") != "ok":
        errors.append("gpu-scene-ui upload invariant must be ok when encode was observed")
    if (gpu_ok.get("frame_stages") or {}).get("GpuUpload", {}).get("status") != "ran":
        errors.append("gpu-scene-ui envelope must mark FrameStage.GpuUpload as ran")
    if not (gpu_ok.get("metrics") or {}).get("gpu_upload_ms"):
        errors.append("gpu-scene-ui envelope must export gpu_upload_ms after FrameProfiler ran")
    try:
        extract_nana(
            gpu_scene,
            {"gpu": {"status": "ok", "scenario_id": "gpu-scene-ui", "composition": "UiOnly"}},
            source_paths={"gpu": Path("synthetic-gpu-missing-work")},
        )
        errors.append("gpu-scene-ui without gpu_work must KeyError, not invent zeros")
    except KeyError as exc:
        reason = key_error_reason(exc)
        if "gpu_work" not in reason and "materialization" not in reason:
            errors.append(f"gpu-scene-ui missing work KeyError should name gpu_work: {exc}")
    try:
        extract_nana(
            gpu_scene,
            {
                "gpu": _synthetic_gpu_ui_only(
                    gpu_scene,
                    materialization={
                        "viewport": [32, 32],
                        "host_texture": gpu_scene["params"]["host_texture"],
                        "ui_nodes": gpu_scene["params"]["ui_nodes"],
                        "ui_entity_count": 4,
                        "scene_primitive_kinds": ["host-texture", "quad"],
                    },
                )
            },
            source_paths={"gpu": Path("synthetic-gpu-wrong-viewport")},
        )
        errors.append("gpu-scene-ui with a private viewport must KeyError")
    except KeyError as exc:
        if "viewport" not in key_error_reason(exc):
            errors.append(f"gpu-scene-ui viewport mismatch KeyError should name viewport: {exc}")
    try:
        extract_nana(
            gpu_scene,
            {"gpu": _synthetic_gpu_ui_only(gpu_scene, include_frame=False)},
            source_paths={"gpu": Path("synthetic-gpu-missing-stages")},
        )
        errors.append("gpu-scene-ui without frame_stages must KeyError")
    except KeyError as exc:
        if "frame_stages" not in key_error_reason(exc) and "FrameProfiler" not in key_error_reason(
            exc
        ):
            errors.append(f"gpu-scene-ui missing frame_stages KeyError should name FrameProfiler: {exc}")
    try:
        extract_nana(
            gpu_scene,
            {"gpu": _synthetic_gpu_ui_only(gpu_scene, frame_status="unsupported")},
            source_paths={"gpu": Path("synthetic-gpu-unsupported-stages")},
        )
        errors.append(
            "gpu-scene-ui must not export GPU timings when FrameProfiler marks stages unsupported"
        )
    except KeyError as exc:
        if "unsupported" not in key_error_reason(exc) and "ran" not in key_error_reason(exc):
            errors.append(f"gpu-scene-ui unsupported-stage KeyError should name status: {exc}")
    live2d = load_scenario("gpu-scene-ui-live2d", root)
    try:
        extract_nana(
            live2d,
            {"gpu": {"status": "ok"}},
            source_paths={"gpu": Path("synthetic-live2d")},
        )
        errors.append("gpu-scene-ui-live2d extract must KeyError, not invent GPU numbers")
    except KeyError as exc:
        if "Live2D" not in key_error_reason(exc) and "UiLive2d" not in key_error_reason(exc):
            errors.append(f"gpu-scene-ui-live2d KeyError should name Live2D: {exc}")
    try:
        extract_nana(
            gpu_scene,
            {
                "gpu": {
                    "status": "unsupported",
                    "scenario_id": "gpu-scene-ui",
                    "composition": "UiOnly",
                    "unsupported_reason": (
                        "No WGPU adapter for the hosted GPU scene path. "
                        "Do not invent upload/batch zeros."
                    ),
                }
            },
            source_paths={"gpu": Path("synthetic-no-adapter")},
        )
        errors.append("gpu-scene-ui missing adapter must KeyError, not invent GPU keys")
    except KeyError as exc:
        reason = key_error_reason(exc)
        if "adapter" not in reason.lower() and "WGPU" not in reason:
            errors.append(f"gpu-scene-ui no-adapter KeyError should name adapter: {exc}")
    runtime_only_path = root / "perf" / "fixtures" / "nana-runtime-static-tree.json"
    try:
        extracted_runtime = extract_nana(
            gpu_scene,
            {"runtime": load_json(runtime_only_path)},
            source_paths={"runtime": runtime_only_path},
        )
        errors.append("gpu-scene-ui from a runtime-only dump must KeyError, not invent GPU keys")
        if extracted_runtime.get("work_counters"):
            errors.append("gpu-scene-ui runtime-only extract must not grow GPU work_counters")
    except KeyError as exc:
        if "nana-gpu-scene-benchmark" not in key_error_reason(exc):
            errors.append(
                f"gpu-scene-ui runtime-only KeyError should name nana-gpu-scene-benchmark: {exc}"
            )

    paint_failed = extract_nana(
        paint,
        {
            "runtime": {
                "cases": [
                    {
                        "nodes": 5000,
                        "local_paint_systems_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "local_paint_work_nodes": 1,
                        "local_paint_work": {"layout_nodes": 5},
                    }
                ]
            }
        },
        source_paths={"runtime": Path("synthetic-paint-layout")},
    )
    if paint_failed.get("status") != "error":
        errors.append("paint extract must fail-closed when layout_nodes != 0")

    text_mutation = load_scenario("mutation-text", root)
    text_ok = extract_nana(
        text_mutation,
        {
            "runtime": {
                "cases": [
                    {
                        "nodes": 5000,
                        "single_node_mutations": {
                            "Text": {
                                "systems_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                                "commit_ms": {"p50": 0.0, "p95": 0.0, "p99": 0.0},
                                "schedule_ms": {"p50": 0.0, "p95": 0.0, "p99": 0.0},
                                "work": {
                                    "text_shaped": 1,
                                    "layout_nodes": 0,
                                    "entities_changed": 1,
                                },
                            }
                        },
                    }
                ]
            }
        },
        source_paths={"runtime": Path("synthetic-text-mutation")},
    )
    if text_ok.get("status") != "ok":
        errors.append("nana mutation-text extract with text_shaped=1 must be ok")
    if (text_ok.get("work_counters") or {}).get("text_shaped") != 1:
        errors.append("mutation-text envelope must carry measured text_shaped")
    text_inv = _named_invariant(text_ok, "single_text_patch_shapes_bounded_nodes")
    if text_inv is None or text_inv.get("status") != "ok":
        errors.append("mutation-text invariant must be ok when text_shaped=1")

    try:
        extract_nana(
            text_mutation,
            {"runtime": {"cases": [{"nodes": 5000}]}},
            source_paths={"runtime": Path("synthetic-text-missing")},
        )
        errors.append("nana mutation-text without single_node_mutations.Text must KeyError")
    except KeyError as exc:
        if "single_node_mutations.Text" not in key_error_reason(exc):
            errors.append(f"mutation-text KeyError should name the drain: {exc}")

    def _single_node_runtime(mutation_kind: str, work: Mapping[str, Any]) -> dict[str, Any]:
        return {
            "runtime": {
                "cases": [
                    {
                        "nodes": 5000,
                        "single_node_mutations": {
                            mutation_kind: {
                                "systems_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                                "commit_ms": {"p50": 0.0, "p95": 0.0, "p99": 0.0},
                                "schedule_ms": {"p50": 0.0, "p95": 0.0, "p99": 0.0},
                                "work": dict(work),
                            }
                        },
                    }
                ]
            }
        }

    for scenario_id, kind, invariant_name, ok_work, fail_work in (
        (
            "mutation-transform",
            "Transform",
            "transform_does_not_layout_full_tree",
            {"layout_nodes": 0, "render_nodes_changed": 1},
            {"layout_nodes": 1, "render_nodes_changed": 1},
        ),
        (
            "mutation-a11y",
            "Accessibility",
            "a11y_does_not_layout",
            {
                "layout_nodes": 0,
                "accessibility_nodes_updated": 1,
                "render_nodes_changed": 0,
            },
            {"layout_nodes": 3},
        ),
    ):
        scenario = load_scenario(scenario_id, root)
        ok_report = extract_nana(
            scenario,
            _single_node_runtime(kind, ok_work),
            source_paths={"runtime": Path(f"synthetic-{scenario_id}")},
        )
        if ok_report.get("status") != "ok":
            errors.append(f"nana {scenario_id} extract with layout_nodes=0 must be ok")
        if not _named_invariant_ok_measured(ok_report, invariant_name, 0):
            errors.append(f"{scenario_id} invariant must be ok when layout_nodes=0")
        failed = extract_nana(
            scenario,
            _single_node_runtime(kind, fail_work),
            source_paths={"runtime": Path(f"synthetic-{scenario_id}-layout")},
        )
        if failed.get("status") != "error":
            errors.append(
                f"{kind.lower()} extract must fail-closed when layout_nodes != 0"
            )

    visibility_mutation = load_scenario("mutation-visibility", root)
    for spec in visibility_mutation.get("invariants") or []:
        if (
            spec.get("path") == "work_counters.layout_nodes"
            and spec.get("op") == "eq"
            and spec.get("value") == 0
        ):
            errors.append(
                "mutation-visibility must not claim layout_nodes==0; the live dump layouts"
            )
    visibility_ok = extract_nana(
        visibility_mutation,
        _single_node_runtime(
            "Visibility",
            {"layout_nodes": 12, "render_nodes_changed": 12, "style_processed": 1},
        ),
        source_paths={"runtime": Path("synthetic-visibility-mutation")},
    )
    if visibility_ok.get("status") != "ok":
        errors.append("nana mutation-visibility extract with ancestor-chain work must be ok")
    if not _named_invariant_ok_measured(
        visibility_ok, "visibility_does_not_extract_full_tree", 12
    ):
        errors.append("mutation-visibility extract invariant must measure render_nodes_changed=12")
    if not _named_invariant_ok_measured(
        visibility_ok, "visibility_does_not_layout_full_tree", 12
    ):
        errors.append("mutation-visibility layout invariant must measure layout_nodes=12, not 0")
    visibility_failed = extract_nana(
        visibility_mutation,
        _single_node_runtime(
            "Visibility",
            {"layout_nodes": 5000, "render_nodes_changed": 5000},
        ),
        source_paths={"runtime": Path("synthetic-visibility-full-tree")},
    )
    if visibility_failed.get("status") != "error":
        errors.append("visibility extract must fail-closed on a full-tree dirty set")

    for missing_id, missing_kind in (
        ("mutation-transform", "Transform"),
        ("mutation-visibility", "Visibility"),
        ("mutation-a11y", "Accessibility"),
    ):
        try:
            extract_nana(
                load_scenario(missing_id, root),
                {"runtime": {"cases": [{"nodes": 5000}]}},
                source_paths={"runtime": Path(f"synthetic-{missing_id}-missing")},
            )
            errors.append(
                f"nana {missing_id} without single_node_mutations.{missing_kind} must KeyError"
            )
        except KeyError as exc:
            if f"single_node_mutations.{missing_kind}" not in key_error_reason(exc):
                errors.append(f"{missing_id} KeyError should name the drain: {exc}")

    virtual_1m = load_scenario("virtual-list-1m", root)
    try:
        extract_nana(
            virtual_1m,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "list",
                            "logical_rows": 1000000,
                            "status": "skipped",
                            "skip_reason": "NANA_PERF_SCALE!=large",
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-1m-skipped")},
        )
        errors.append("nana virtual-list-1m skipped scale must KeyError, not ok")
    except KeyError as exc:
        if "1000000" not in key_error_reason(exc) and "1M" not in key_error_reason(exc).upper():
            errors.append(f"virtual-list-1m skip KeyError should name the 1M row: {exc}")

    virtual_1m_ok = extract_nana(
        virtual_1m,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "list",
                        "logical_rows": 1000000,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 10,
                        "live_ui_entities": 50,
                        "live_ui_entities_bound": 50,
                        "materialize_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic-1m-ok")},
    )
    if virtual_1m_ok.get("status") != "ok":
        errors.append("nana virtual-list-1m extract must be ok when virtual_scales status=ok")
    if virtual_1m_ok.get("scenario_id") != "virtual-list-1m":
        errors.append("virtual-list-1m must keep its catalog id, not virtual-list-1000k")
    if (virtual_1m_ok.get("work_counters") or {}).get("live_ui_entities") != 50:
        errors.append("virtual-list-1m envelope must carry live_ui_entities")

    catalog = load_catalog(root)
    harness = set(catalog.get("harness_ids", []))
    reserved_ids = {
        item.get("id") for item in catalog.get("required_by_issue_not_in_harness", [])
    }
    for tree_id in ("virtual-tree-10k", "virtual-tree-100k"):
        if tree_id not in harness:
            errors.append(f"catalog must list wirable {tree_id} in harness_ids")
        if tree_id in reserved_ids:
            errors.append(f"catalog must not leave wirable {tree_id} in required_by_issue_not_in_harness")
    if "virtual-tree-1m" in harness:
        errors.append("catalog must keep virtual-tree-1m out of harness_ids")
    if "virtual-tree-1m" not in reserved_ids:
        errors.append("catalog must list virtual-tree-1m in required_by_issue_not_in_harness")

    virtual_tree = load_scenario("virtual-tree-100k", root)
    try:
        extract_nana(
            virtual_tree,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "tree",
                            "logical_rows": 100000,
                            "status": "skipped",
                            "skip_reason": "NANA_PERF_SCALE!=large",
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-tree-skipped")},
        )
        errors.append("nana virtual-tree-100k skipped scale must KeyError, not ok")
    except KeyError as exc:
        if "tree/100000" not in key_error_reason(exc):
            errors.append(f"virtual-tree-100k skip KeyError should name the tree row: {exc}")

    try:
        extract_nana(
            virtual_tree,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "tree",
                            "logical_rows": 100000,
                            "status": "ok",
                            "live_ui_entities": None,
                            "materialize_ms": None,
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-tree-empty-ok")},
        )
        errors.append("nana virtual-tree-100k empty ok must KeyError, not ok")
    except KeyError as exc:
        if "fake empty ok" not in key_error_reason(exc):
            errors.append(f"virtual-tree empty ok KeyError should name fake empty ok: {exc}")

    try:
        extract_nana(
            virtual_tree,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "list",
                            "logical_rows": 100000,
                            "status": "ok",
                            "live_ui_entities": 50,
                            "materialize_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-tree-as-list")},
        )
        errors.append("nana virtual-tree-100k must not extract a list scale row as ok")
    except KeyError as exc:
        if "tree/100000" not in key_error_reason(exc):
            errors.append(f"virtual-tree list-row KeyError should name tree/100000: {exc}")

    tree_ok = _extract_nana_framework_fixture(
        root, virtual_tree, "virtual-tree-scales.json"
    )
    if tree_ok.get("scenario_id") != "virtual-tree-100k":
        errors.append("virtual-tree-100k must keep its catalog id")
    _require_same_scenario(
        errors, tree_ok, "nana virtual-tree-100k", live_ui_entities=57
    )
    tree_10k = _extract_nana_framework_fixture(
        root, "virtual-tree-10k", "virtual-tree-scales.json"
    )
    _require_same_scenario(errors, tree_10k, "nana virtual-tree-10k")

    virtual_tree_1m = load_scenario("virtual-tree-1m", root)
    try:
        extract_nana(
            virtual_tree_1m,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "tree",
                            "logical_rows": 1000000,
                            "status": "skipped",
                            "skip_reason": "NANA_PERF_SCALE!=large",
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-tree-1m-skipped")},
        )
        errors.append("nana virtual-tree-1m skipped scale must KeyError, not ok")
    except KeyError as exc:
        if "tree/1000000" not in key_error_reason(exc):
            errors.append(f"virtual-tree-1m skip KeyError should name the 1M tree row: {exc}")

    text_table = load_scenario("text-table", root)
    if "text-table" not in load_catalog(root).get("harness_ids", []):
        errors.append("catalog must list wirable text-table in harness_ids")
    reserved = {
        item.get("id") for item in load_catalog(root).get("required_by_issue_not_in_harness", [])
    }
    if "text-table" in reserved:
        errors.append("catalog must not leave wirable text-table in required_by_issue_not_in_harness")
    errors.extend(_self_test_catalog_workloads(root, runtime_path, iced_path, reserved))

    try:
        nana_table_legacy = extract_nana(
            text_table,
            {"framework": load_json(framework_path)},
            source_paths={"framework": framework_path},
        )
        if nana_table_legacy.get("status") != "ok":
            errors.append("nana text-table extract from historical framework report must be ok")
        if (nana_table_legacy.get("work_counters") or {}).get("text_shaped") is not None:
            errors.append("legacy text-table extract must omit unmeasured text_shaped")
        for gap in TEXT_TABLE_CACHE_GAPS:
            if gap in (nana_table_legacy.get("work_counters") or {}):
                errors.append(f"legacy text-table extract must omit {gap}")
    except Exception as exc:  # noqa: BLE001
        errors.append(f"nana text-table historical extract failed: {exc}")

    try:
        extract_nana(
            text_table,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "table",
                            "logical_rows": 10000,
                            "logical_columns": 100,
                            "status": "skipped",
                            "skip_reason": "NANA_PERF_SCALE!=large",
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-table-skipped")},
        )
        errors.append("nana text-table skipped scale must KeyError, not ok")
    except KeyError as exc:
        reason = key_error_reason(exc)
        if "10000" not in reason and "table" not in reason:
            errors.append(f"text-table skip KeyError should name the table scale: {exc}")

    table_ok = extract_nana(
        text_table,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "table",
                        "logical_rows": 10000,
                        "logical_columns": 100,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 8,
                        "table_overscan_y_px": 160.0,
                        "table_overscan_x_px": 160.0,
                        "live_ui_entities": 50,
                        "live_ui_entities_bound": 1334,
                        "materialize_ms": {"p50": 0.8, "p95": 0.9, "p99": 1.0},
                        "work": {
                            "text_shaped": 12,
                            "extracted_text_spans": 12,
                            "entities_total": 50,
                            "layout_nodes": 12,
                            "text_shaped_runs": 8,
                            "text_layout_cache_hits": 3,
                            "text_layout_cache_misses": 5,
                            "text_wrap_layouts": 4,
                            "cache_eviction": 0,
                            "glyph_cache_hits": None,
                        },
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic-table-ok")},
    )
    if table_ok.get("status") != "ok":
        errors.append("nana text-table extract must be ok when virtual_scales table status=ok")
    if table_ok.get("scenario_id") != "text-table":
        errors.append("text-table must keep its catalog id")
    table_counters = table_ok.get("work_counters") or {}
    if table_counters.get("live_ui_entities") != 50:
        errors.append("text-table envelope must carry live_ui_entities")
    if table_counters.get("text_shaped") != 12:
        errors.append("text-table envelope must export WorkCounters.text_shaped")
    if table_counters.get("extracted_text_spans") != 12:
        errors.append("text-table envelope must export WorkCounters.extracted_text_spans")
    if table_counters.get("text_wrap_layouts") != 4:
        errors.append("text-table envelope must export WorkCounters.text_wrap_layouts")
    if table_counters.get("text_shaped_runs") != 8:
        errors.append("text-table envelope must export WorkCounters.text_shaped_runs")
    if table_counters.get("text_layout_cache_hits") != 3:
        errors.append("text-table envelope must export WorkCounters.text_layout_cache_hits")
    if table_counters.get("text_layout_cache_misses") != 5:
        errors.append("text-table envelope must export WorkCounters.text_layout_cache_misses")
    if table_counters.get("cache_eviction") != 0:
        errors.append("text-table envelope must export WorkCounters.cache_eviction")
    for key in TEXT_TABLE_EXPORTED_SHAPE_KEYS:
        if key not in table_counters:
            errors.append(f"text-table envelope must contain {key}")
    if table_counters.get("wrapped_cells") != 4:
        errors.append("text-table envelope must copy catalog wrapped_cells=4")
    if table_counters.get("wrapped_cell_len") != 256:
        errors.append("text-table envelope must copy catalog wrapped_cell_len=256")
    if "glyph_cache_hits" in table_counters:
        errors.append("text-table envelope must omit null glyph_cache fields")
    if not any(
        "wrapping" in str(note).lower() or "wrapped_cells" in str(note)
        for note in table_ok.get("mapping_notes") or []
    ):
        errors.append("text-table ok mapping must mention wrapping / wrapped_cells")
    table_inv = _named_invariant(table_ok, "text_table_live_entities_bounded")
    if table_inv is None or table_inv.get("status") != "ok" or table_inv.get("measured") != 50:
        errors.append("text-table live_ui_entities invariant must be ok when measured")
    leftover_ten = {
        "framework": {
            "virtual_scales": [
                {
                    "kind": "table",
                    "logical_rows": 10000,
                    "logical_columns": 100,
                    "status": "ok",
                    "visible_rows": 40,
                    "overscan_rows": 10,
                    "table_overscan_y_px": 200.0,
                    "live_ui_entities": 50,
                    "materialize_ms": {"p50": 0.8, "p95": 0.9, "p99": 1.0},
                }
            ]
        }
    }
    try:
        extract_nana(
            text_table,
            leftover_ten,
            source_paths={"framework": Path("synthetic-table-10-row")},
        )
        errors.append("nana text-table with table_overscan_y_px=200 must KeyError")
    except KeyError as exc:
        if "overscan" not in key_error_reason(exc) and "200" not in key_error_reason(exc):
            errors.append(f"mismatched table overscan KeyError should name overscan/200: {exc}")
    catalog_window_ok = extract_nana(
        text_table,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "table",
                        "logical_rows": 10000,
                        "logical_columns": 100,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 8,
                        "table_overscan_y_px": 160.0,
                        "table_overscan_x_px": 160.0,
                        "live_ui_entities": 50,
                        "materialize_ms": {"p50": 0.8, "p95": 0.9, "p99": 1.0},
                        "work": {"text_shaped": 12},
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic-table-catalog-window")},
    )
    if catalog_window_ok.get("status") != "ok":
        errors.append("nana text-table extract with catalog table_overscan_y_px=160 must be ok")
    if catalog_window_ok.get("equivalence") != "closest-legacy-reference":
        errors.append(
            "nana text-table with only table_overscan_*_px must stay closest-legacy-reference"
        )
    if not any(
        "table_viewport_width_px" in str(note) or "table_row_extent_px" in str(note)
        for note in (catalog_window_ok.get("mapping_notes") or [])
    ):
        errors.append(
            "nana text-table missing viewport/extent notes must name the undeclared fields"
        )
    table_same = _extract_nana_framework_fixture(
        root, text_table, "virtual-table-scales.json"
    )
    _require_same_scenario(
        errors,
        table_same,
        "nana text-table",
        shared_note="Shared catalog table window",
    )
    max_window_ok = extract_nana(
        text_table,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "table",
                        "logical_rows": 10000,
                        "logical_columns": 100,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 8,
                        "table_overscan_y_px": 160.0,
                        "table_overscan_x_px": 160.0,
                        "live_ui_entities": 1334,
                        "live_ui_entities_bound": 1334,
                        "materialize_ms": {"p50": 0.8, "p95": 0.9, "p99": 1.0},
                        "work": {"text_shaped": 12},
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic-table-max-window")},
    )
    if max_window_ok.get("status") != "ok":
        errors.append("nana text-table catalog-8 max window live=1334 must stay ok")
    max_inv = _named_invariant(max_window_ok, "text_table_live_entities_bounded")
    if max_inv is None or max_inv.get("status") != "ok" or max_inv.get("measured") != 1334:
        errors.append("nana text-table invariant must accept catalog-8 cap live=1334")
    full_grid = extract_nana(
        text_table,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "table",
                        "logical_rows": 10000,
                        "logical_columns": 100,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 8,
                        "table_overscan_y_px": 160.0,
                        "table_overscan_x_px": 160.0,
                        "live_ui_entities": 1_000_000,
                        "live_ui_entities_bound": 1334,
                        "materialize_ms": {"p50": 0.8, "p95": 0.9, "p99": 1.0},
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic-table-full-grid")},
    )
    if full_grid.get("status") != "error":
        errors.append("nana text-table live=1000000 must be rejected as envelope error")

    try:
        extract_iced(text_table, load_json(iced_path), source_path=iced_path)
        errors.append("iced text-table gallery extract must be unsupported")
    except KeyError:
        pass

    try:
        extract_nana(
            hover,
            {"runtime": load_json(runtime_path)},
            source_paths={"runtime": runtime_path},
        )
        errors.append("nana hover on phase3 report must be unsupported (no 10k case)")
    except KeyError as exc:
        if "nodes=10000" not in key_error_reason(exc):
            errors.append(f"nana hover KeyError should name nodes=10000: {exc}")

    hover_ok = extract_nana(
        hover,
        {
            "runtime": {
                "cases": [
                    {
                        "nodes": 10000,
                        "pointer_hover_transition_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "pointer_hover_work_nodes": 2,
                    }
                ]
            }
        },
        source_paths={"runtime": Path("synthetic-hover-10k")},
    )
    if hover_ok.get("status") != "ok":
        errors.append("nana hover extract must be ok when a 10k hover case exists")
    if (hover_ok.get("work_counters") or {}).get("layout_nodes") is not None:
        errors.append("ok hover envelope must omit unmeasured layout_nodes")
    hover_inv = _named_invariant(hover_ok, "hover_without_size_change")
    if hover_inv is None or hover_inv.get("status") != "not-evaluable":
        errors.append("unmeasured hover extract must leave layout_nodes invariant not-evaluable")

    hover_measured = extract_nana(
        hover,
        {
            "runtime": {
                "cases": [
                    {
                        "nodes": 10000,
                        "pointer_hover_transition_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "pointer_hover_work_nodes": 2,
                        "pointer_hover_work": {
                            "layout_nodes": 0,
                            "entities_changed": 2,
                            "render_nodes_extracted": 2,
                            "extracted_text_spans": None,
                        },
                    }
                ]
            }
        },
        source_paths={"runtime": Path("synthetic-hover-counters")},
    )
    if hover_measured.get("status") != "ok":
        errors.append("nana hover extract with layout_nodes=0 must be ok")
    hover_counters = hover_measured.get("work_counters") or {}
    if hover_counters.get("layout_nodes") != 0:
        errors.append("ok hover envelope must contain numeric layout_nodes when fixture has it")
    if "extracted_text_spans" in hover_counters:
        errors.append("ok hover envelope must omit null work-counter fields")
    hover_ok_inv = _named_invariant(hover_measured, "hover_without_size_change")
    if (
        hover_ok_inv is None
        or hover_ok_inv.get("status") != "ok"
        or hover_ok_inv.get("measured") != 0
    ):
        errors.append("hover invariant must be ok with measured layout_nodes=0")

    try:
        extract_iced(hover, load_json(iced_path), source_path=iced_path)
        errors.append("iced hover extract must be unsupported")
    except KeyError:
        pass

    errors.extend(_self_test_from_report_cli(root))

    gpui = gpui_unsupported(virtual)
    if gpui.get("status") != "unsupported":
        errors.append("gpui stub must return unsupported")

    if static_tree_parent(1) is not None or static_tree_parent(100) != 50:
        errors.append("StaticTree heap parent rule must be parent(1)=None, parent(100)=50")
    if static_tree_children(1, 100) != [2, 3] or static_tree_children(50, 100) != [100]:
        errors.append("StaticTree heap children must be left=2i, right=2i+1")
    if not is_shared_static_tree(
        {
            "generation": STATIC_TREE_GENERATION,
            "parent_rule": STATIC_TREE_PARENT_RULE,
            "node_kind": STATIC_TREE_NODE_KIND,
            "text": None,
            "sample_parents": static_tree_sample_parents(100),
        },
        100,
    ):
        errors.append("shared StaticTree heap descriptor must validate")

    iced_bench_path = root / "perf" / "fixtures" / "iced-scenario-static-tree-100.json"
    iced_same = extract_iced(
        static_tree, load_json(iced_bench_path), source_path=iced_bench_path
    )
    if iced_same.get("status") != "ok":
        errors.append("iced same-scenario static-tree-100 extract did not return ok")
    if iced_same.get("equivalence") != "same-scenario":
        errors.append("iced-scenario-bench extract must be same-scenario, not Gallery mapping")
    if iced_same.get("relative_gate_enforceable") is not False:
        errors.append("same-scenario iced extract must keep relative_gate_enforceable False")
    if iced_same.get("timing_gate_enforceable") is not False:
        errors.append("same-scenario iced extract must keep timing_gate_enforceable False")
    iced_cpu = (iced_same.get("metrics") or {}).get("cpu_frame_ms") or {}
    if not isinstance(iced_cpu.get("p50"), (int, float)):
        errors.append("iced same-scenario extract must copy live cpu_frame_ms.p50")
    if (iced_same.get("metrics") or {}).get("frames_after_idle") != 0:
        errors.append("iced same-scenario extract must copy frames_after_idle=0")
    if not isinstance((iced_same.get("metrics") or {}).get("busy_probe_frames"), int) or (
        iced_same.get("metrics") or {}
    ).get("busy_probe_frames", 0) < 1:
        errors.append("iced same-scenario extract must copy live busy_probe_frames > 0")
    iced_raw = load_json(iced_bench_path)
    if iced_raw.get("adapter", {}).get("name") == "extractor-fixture":
        errors.append("iced static-tree-100 fixture must be a live dump, not extractor-fixture")
    if iced_raw.get("frames_after_idle") != 0:
        errors.append("live iced static-tree-100 dump must have frames_after_idle=0")
    if not isinstance(iced_raw.get("busy_probe_frames"), int) or iced_raw.get(
        "busy_probe_frames", 0
    ) < 1:
        errors.append("live iced static-tree-100 dump must have busy_probe_frames > 0")
    gpui_static = gpui_unsupported(static_tree)
    if relative_gate_can_enforce(iced_same, gpui_static):
        errors.append("relative gates must stay off while GPUI is unsupported")
    if relative_gate_can_enforce(iced_same, iced_same):
        errors.append("relative_gate_can_enforce must require runner=gpui, not two iced reports")
    synthetic_gpui_ok = {
        "runner": "gpui",
        "status": "ok",
        "equivalence": "same-scenario",
        "scenario_id": "static-tree-100",
        "metrics": {"cpu_frame_ms": {"p50": 1.0, "p95": 1.2}},
    }
    if not relative_gate_can_enforce(iced_same, synthetic_gpui_ok):
        errors.append(
            "relative_gate_can_enforce should be true for a real Iced+GPUI same-scenario pair"
        )
    try:
        extract_iced(
            load_scenario("static-tree-5k", root),
            load_json(iced_bench_path),
            source_path=iced_bench_path,
        )
        errors.append("iced-scenario-bench extract must not remap a 100-node fixture onto 5k")
    except KeyError as exc:
        if "nodes=" not in key_error_reason(exc):
            errors.append(f"mismatched StaticTree size KeyError should name nodes: {exc}")
    try:
        extract_iced(hover, load_json(iced_bench_path), source_path=iced_bench_path)
        errors.append("iced-scenario-bench hover extract from StaticTree fixture must KeyError")
    except KeyError:
        pass

    mutation = load_scenario("mutation-paint-only", root)
    mutation_path = root / "perf" / "fixtures" / "iced-scenario-mutation-paint-only.json"
    mutation_ok = extract_iced(mutation, load_json(mutation_path), source_path=mutation_path)
    if mutation_ok.get("status") != "ok" or mutation_ok.get("equivalence") != "same-scenario":
        errors.append("iced mutation-paint-only fixture extract must be same-scenario ok")
    if (mutation_ok.get("metrics") or {}).get("cpu_frame_ms", {}).get("p50") != 1.7:
        errors.append("iced mutation extract must copy fixture cpu_frame_ms.p50")
    if mutation_ok.get("relative_gate_enforceable") is not False:
        errors.append("iced mutation extract must keep relative gates off")

    fake_mutation_ok = {
        "source": "iced-scenario-bench",
        "status": "ok",
        "nodes": 5000,
        "cpu_frame_ms": {"p50": 1.0, "p95": 1.2, "p99": 1.3},
        "tree": {
            "generation": STATIC_TREE_GENERATION,
            "parent_rule": STATIC_TREE_PARENT_RULE,
            "node_kind": STATIC_TREE_NODE_KIND,
            "text": None,
            "sample_parents": static_tree_sample_parents(5000),
        },
        "mutation": {"kind": "Transform", "target_index": 2503, "single_node": True},
    }
    for unsupported_id in (
        "mutation-transform",
        "mutation-visibility",
        "mutation-a11y",
    ):
        unsupported_scenario = load_scenario(unsupported_id, root)
        fake_mutation_ok["scenario_id"] = unsupported_id
        fake_mutation_ok["mutation"] = {
            "kind": unsupported_scenario["params"]["kind"],
            "target_index": 2500,
            "single_node": True,
        }
        try:
            extracted = extract_iced(
                unsupported_scenario,
                fake_mutation_ok,
                source_path=Path(f"fake-{unsupported_id}"),
            )
            if extracted.get("status") == "ok":
                errors.append(
                    f"iced {unsupported_id} extract must not be ok / same-scenario"
                )
        except KeyError as exc:
            reason = key_error_reason(exc)
            if unsupported_scenario["params"]["kind"] not in reason and "same-scenario" not in reason:
                errors.append(
                    f"iced {unsupported_id} KeyError should name the missing dirty work: {exc}"
                )

    hover_path = root / "perf" / "fixtures" / "iced-scenario-hover.json"
    hover_iced_ok = extract_iced(hover, load_json(hover_path), source_path=hover_path)
    if hover_iced_ok.get("status") != "ok":
        errors.append("iced hover fixture extract must be ok at 10k, not a smaller tree")
    if (hover_iced_ok.get("work_counters") or {}).get("nodes") != 10000:
        errors.append("iced hover fixture extract must record work_counters.nodes=10000")
    if hover_iced_ok.get("equivalence") != "same-scenario":
        errors.append("iced hover fixture extract must be same-scenario")
    if (hover_iced_ok.get("metrics") or {}).get("cpu_frame_ms", {}).get("p50") != 2.1:
        errors.append("iced hover extract must copy fixture cpu_frame_ms.p50")

    virtual_path = root / "perf" / "fixtures" / "iced-scenario-virtual-list-10k.json"
    virtual_iced = extract_iced(virtual, load_json(virtual_path), source_path=virtual_path)
    if virtual_iced.get("status") != "ok":
        errors.append("iced virtual-list-10k fixture extract must be ok")
    if (virtual_iced.get("work_counters") or {}).get("live_ui_entities") != 56:
        errors.append("iced virtual-list extract must copy live_ui_entities=56")
    if any(
        "200px" in note or "may differ" in note
        for note in (virtual_iced.get("mapping_notes") or [])
    ):
        errors.append("iced virtual-list same-scenario notes must not claim overscan may differ")
    wrong_overscan = load_json(virtual_path)
    wrong_overscan["virtualization"] = dict(wrong_overscan["virtualization"])
    wrong_overscan["virtualization"]["overscan"] = 10
    try:
        extract_iced(virtual, wrong_overscan, source_path=Path("wrong-overscan"))
        errors.append("iced VirtualList with overscan!=catalog must KeyError")
    except KeyError as exc:
        if "overscan" not in key_error_reason(exc):
            errors.append(f"mismatched VirtualList overscan KeyError should name overscan: {exc}")
    fake_full_list = load_json(virtual_path)
    fake_full_list["virtualization"] = dict(fake_full_list["virtualization"])
    fake_full_list["virtualization"]["live_ui_entities"] = 10000
    try:
        extract_iced(virtual, fake_full_list, source_path=Path("fake-full-list"))
        errors.append("iced VirtualList with live_ui_entities==items must KeyError")
    except KeyError as exc:
        if "10000" not in key_error_reason(exc):
            errors.append(f"full-list VirtualList KeyError should name 10000: {exc}")

    table_iced_path = root / "perf" / "fixtures" / "iced-scenario-text-table.json"
    table_iced_ok = extract_iced(text_table, load_json(table_iced_path), source_path=table_iced_path)
    if table_iced_ok.get("status") != "ok" or table_iced_ok.get("equivalence") != "same-scenario":
        errors.append("iced text-table fixture extract must be same-scenario ok")
    if (table_iced_ok.get("work_counters") or {}).get("overscan_rows") != 8:
        errors.append("iced text-table extract must record overscan_rows=8")
    if (table_iced_ok.get("metrics") or {}).get("cpu_frame_ms", {}).get("p50") != 5.5:
        errors.append("iced text-table extract must copy fixture cpu_frame_ms.p50")
    max_iced = load_json(table_iced_path)
    max_iced["virtualization"] = dict(max_iced["virtualization"])
    max_iced["virtualization"]["live_ui_entities"] = 1334
    max_iced["virtualization"]["live_ui_entities_bound"] = 1334
    max_iced_ok = extract_iced(text_table, max_iced, source_path=Path("iced-table-max-window"))
    if max_iced_ok.get("status") != "ok" or max_iced_ok.get("equivalence") != "same-scenario":
        errors.append("iced text-table catalog-8 max window live=1334 must stay same-scenario ok")
    max_iced_inv = _named_invariant(max_iced_ok, "text_table_live_entities_bounded")
    if (
        max_iced_inv is None
        or max_iced_inv.get("status") != "ok"
        or max_iced_inv.get("measured") != 1334
    ):
        errors.append("iced text-table invariant must accept catalog-8 cap live=1334")
    wrong_table = load_json(table_iced_path)
    wrong_table["virtualization"] = dict(wrong_table["virtualization"])
    wrong_table["virtualization"]["overscan_rows"] = 10
    try:
        extract_iced(text_table, wrong_table, source_path=Path("fake-table-10-row"))
        errors.append("iced Table with overscan_rows=10 must KeyError")
    except KeyError as exc:
        if "overscan" not in key_error_reason(exc):
            errors.append(f"mismatched Table overscan KeyError should name overscan: {exc}")
    full_table = load_json(table_iced_path)
    full_table["virtualization"] = dict(full_table["virtualization"])
    full_table["virtualization"]["live_ui_entities"] = 1_000_000
    try:
        extract_iced(text_table, full_table, source_path=Path("fake-full-table"))
        errors.append("iced Table with live_ui_entities==rows×columns must KeyError")
    except KeyError as exc:
        if "10000" not in key_error_reason(exc) and "cells" not in key_error_reason(exc):
            errors.append(f"full Table KeyError should name 10000/cells: {exc}")

    for unsupported_id, token in (
        ("animation", "advance_animations"),
        ("ime", "set_ime_preedit"),
        ("overlay", "OverlayHost"),
        ("gpu-scene-ui", "Live2D"),
        ("dock-workspace", "assemble_dock"),
        ("text-editor", "drain_text"),
    ):
        unsupported_scenario = load_scenario(unsupported_id, root)
        fake_ok = {
            "source": "iced-scenario-bench",
            "status": "ok",
            "scenario_id": unsupported_id,
            "cpu_frame_ms": {"p50": 1.0, "p95": 1.2, "p99": 1.3},
        }
        try:
            extracted = extract_iced(
                unsupported_scenario,
                fake_ok,
                source_path=Path(f"fake-{unsupported_id}"),
            )
            if extracted.get("status") == "ok":
                errors.append(f"iced {unsupported_id} extract must not be ok / same-scenario")
        except KeyError as exc:
            reason = key_error_reason(exc)
            if token not in reason:
                errors.append(
                    f"iced {unsupported_id} KeyError should name the missing work ({token}): {exc}"
                )

    try:
        extract_iced(
            load_scenario("static-tree-50k", root),
            {
                "source": "iced-scenario-bench",
                "status": "ok",
                "scenario_id": "static-tree-50k",
                "nodes": 50000,
                "cpu_frame_ms": {"p50": 1.0, "p95": 2.0, "p99": 3.0},
                "tree": {
                    "generation": STATIC_TREE_GENERATION,
                    "parent_rule": STATIC_TREE_PARENT_RULE,
                    "node_kind": STATIC_TREE_NODE_KIND,
                    "text": None,
                    "sample_parents": static_tree_sample_parents(50000),
                },
            },
            source_path=Path("fake-50k"),
        )
        errors.append("iced static-tree-50k extract must be unsupported/incomparable")
    except KeyError as exc:
        if "50k" not in key_error_reason(exc).lower() and "50000" not in key_error_reason(exc):
            errors.append(f"iced 50k KeyError should name incomparable 50k: {exc}")

    try:
        extract_nana(
            load_scenario("static-tree-50k", root),
            {
                "runtime": {
                    "cases": [
                        {
                            "nodes": 50000,
                            "kind": "construction",
                            "enqueue_ms": {"p50": 1.0, "p95": 2.0, "p99": 3.0},
                            "initial_commit_ms": {"p50": 1.0, "p95": 2.0, "p99": 3.0},
                        }
                    ]
                }
            },
            source_paths={"runtime": Path("synthetic-50k")},
        )
        errors.append("nana static-tree-50k construction extract must KeyError as incomparable")
    except KeyError as exc:
        if "50k" not in key_error_reason(exc).lower() and "construction" not in key_error_reason(exc).lower():
            errors.append(f"nana 50k KeyError should name incomparable construction: {exc}")

    empty_ok = {
        "source": "iced-scenario-bench",
        "status": "ok",
        "scenario_id": "static-tree-100",
        "nodes": 100,
        "cpu_frame_ms": {},
    }
    try:
        extract_iced(static_tree, empty_ok, source_path=Path("empty-cpu"))
        errors.append("iced-scenario-bench ok without cpu_frame_ms percentiles must KeyError")
    except KeyError:
        pass
    leaf_column = {
        "source": "iced-scenario-bench",
        "status": "ok",
        "scenario_id": "static-tree-100",
        "nodes": 100,
        "cpu_frame_ms": {"p50": 1.4, "p95": 1.7, "p99": 2.0},
    }
    try:
        extract_iced(static_tree, leaf_column, source_path=Path("text-leaves"))
        errors.append("N text leaves without heap provenance must not be same-scenario")
    except KeyError as exc:
        if "complete-binary-heap" not in key_error_reason(exc):
            errors.append(f"text-leaf KeyError should name the heap rule: {exc}")
    heap_without_idle = {
        "source": "iced-scenario-bench",
        "status": "ok",
        "scenario_id": "static-tree-100",
        "nodes": 100,
        "cpu_frame_ms": {"p50": 1.4, "p95": 1.7, "p99": 2.0},
        "tree": {
            "generation": STATIC_TREE_GENERATION,
            "parent_rule": STATIC_TREE_PARENT_RULE,
            "node_kind": STATIC_TREE_NODE_KIND,
            "text": None,
            "sample_parents": static_tree_sample_parents(100),
        },
    }
    try:
        extract_iced(static_tree, heap_without_idle, source_path=Path("heap-no-idle"))
        errors.append("iced StaticTree without frames_after_idle must KeyError")
    except KeyError as exc:
        if "frames_after_idle" not in key_error_reason(exc):
            errors.append(f"missing idle-frame KeyError should name frames_after_idle: {exc}")
    stuffed_zero = dict(heap_without_idle)
    stuffed_zero["frames_after_idle"] = 0
    try:
        extract_iced(static_tree, stuffed_zero, source_path=Path("stuffed-idle-zero"))
        errors.append("iced StaticTree stuffed frames_after_idle=0 without busy_probe must KeyError")
    except KeyError as exc:
        if "busy_probe_frames" not in key_error_reason(exc):
            errors.append(f"stuffed idle-0 KeyError should name busy_probe_frames: {exc}")
    errors.extend(_self_test_section_8_1_runner_invariants(root))
    return errors


def _self_test_section_8_1_runner_invariants(root: Path) -> list[str]:
    """Prove §8.1 judging of runner envelopes (pass, fail, skip). No log-string match."""
    errors: list[str] = []
    virtual = load_scenario("virtual-list-10k", root)
    text_table = load_scenario("text-table", root)
    list_bound = catalog_virtual_list_live_bound(virtual["params"])
    table_bound = catalog_table_live_bound(text_table["params"])
    list_spec = (virtual.get("invariants") or [{}])[0]
    table_spec = (text_table.get("invariants") or [{}])[0]
    if list_spec.get("value") != list_bound:
        errors.append(
            f"virtual-list-10k invariant value {list_spec.get('value')} "
            f"must be catalog cap {list_bound}"
        )
    hundredk = load_scenario("virtual-list-100k", root)
    if (hundredk.get("invariants") or [{}])[0].get("value") != list_bound:
        errors.append("virtual-list-100k must share the catalog-8 live bound")
    tree_10k = load_scenario("virtual-tree-10k", root)
    tree_100k = load_scenario("virtual-tree-100k", root)
    if (tree_10k.get("invariants") or [{}])[0].get("value") != list_bound:
        errors.append("virtual-tree-10k must share the catalog-8 live bound")
    if (tree_100k.get("invariants") or [{}])[0].get("value") != list_bound:
        errors.append("virtual-tree-100k must share the catalog-8 live bound")
    if table_spec.get("value") != table_bound:
        errors.append(
            f"text-table invariant value {table_spec.get('value')} "
            f"must be catalog cap {table_bound}"
        )

    static_tree_ids = (
        "static-tree-100",
        "static-tree-1k",
        "static-tree-5k",
        "static-tree-10k",
    )
    for scenario_id in static_tree_ids:
        if scenario_id not in SECTION_8_1_HONEST_OK_IDS:
            errors.append(f"{scenario_id} must be §8.1 honest-ok once frames_after_idle exists")
        if scenario_id in SECTION_8_1_UNSUPPORTED_IDS:
            errors.append(f"{scenario_id} must leave §8.1 unsupported once idle frames exist")
        static_scenario = load_scenario(scenario_id, root)
        specs = static_scenario.get("invariants") or []
        if not specs:
            errors.append(f"{scenario_id} must declare static_ui frames_after_idle == 0")
        else:
            spec = specs[0]
            if (
                spec.get("name") != "static_ui"
                or spec.get("path") != "metrics.frames_after_idle"
                or spec.get("op") != "eq"
                or spec.get("value") != 0
            ):
                errors.append(
                    f"{scenario_id} idle invariant must be static_ui metrics.frames_after_idle == 0"
                )
    if "static-tree-50k" not in SECTION_8_1_UNSUPPORTED_IDS:
        errors.append("static-tree-50k must stay §8.1 skipped")
    if "static-tree-50k" in SECTION_8_1_HONEST_OK_IDS:
        errors.append("static-tree-50k must not be §8.1 honest-ok")
    if "gpu-scene-ui" not in SECTION_8_1_HONEST_OK_IDS or "gpu-scene-ui" in SECTION_8_1_UNSUPPORTED_IDS:
        errors.append("gpu-scene-ui must be §8.1 honest-ok for Nana encode envelopes")
    for live2d_id in ("gpu-scene-ui-live2d", "gpu-scene-ui-live2d-effect"):
        if live2d_id in SECTION_8_1_HONEST_OK_IDS or live2d_id not in SECTION_8_1_UNSUPPORTED_IDS:
            errors.append(f"{live2d_id} must stay §8.1 skipped")
    for scenario_id in sorted(SECTION_8_1_HONEST_OK_IDS):
        specs = load_scenario(scenario_id, root).get("invariants") or []
        if not specs:
            errors.append(
                f"{scenario_id} is §8.1 honest-ok but has no catalog invariants; "
                "vacuous ok is forbidden"
            )

    virtual_path = root / "perf" / "fixtures" / "iced-scenario-virtual-list-10k.json"
    table_path = root / "perf" / "fixtures" / "iced-scenario-text-table.json"
    hover_path = root / "perf" / "fixtures" / "iced-scenario-hover.json"
    paint_path = root / "perf" / "fixtures" / "iced-scenario-mutation-paint-only.json"
    static_path = root / "perf" / "fixtures" / "iced-scenario-static-tree-100.json"
    try:
        virtual_iced = extract_iced(virtual, load_json(virtual_path), source_path=virtual_path)
        static_iced = extract_iced(
            load_scenario("static-tree-100", root),
            load_json(static_path),
            source_path=static_path,
        )
        table_iced = extract_iced(text_table, load_json(table_path), source_path=table_path)
        hover_iced = extract_iced(
            load_scenario("hover", root), load_json(hover_path), source_path=hover_path
        )
        paint_iced = extract_iced(
            load_scenario("mutation-paint-only", root),
            load_json(paint_path),
            source_path=paint_path,
        )
        nana_table = _extract_nana_framework_fixture(
            root, text_table, "virtual-table-scales.json"
        )
        nana_list = _extract_nana_framework_fixture(
            root, "virtual-list-100k", "virtual-scales-only.json"
        )
        nana_runtime_live_path = root / "perf" / "fixtures" / "nana-runtime-static-tree.json"
        nana_runtime_live = load_json(nana_runtime_live_path)
        nana_live = {
            scenario_id: extract_nana(
                load_scenario(scenario_id, root),
                {"runtime": nana_runtime_live},
                source_paths={"runtime": nana_runtime_live_path},
            )
            for scenario_id in (
                "static-tree-100",
                "mutation-transform",
                "mutation-a11y",
                "mutation-visibility",
            )
        }
        nana_static = nana_live["static-tree-100"]
        nana_transform = nana_live["mutation-transform"]
        nana_a11y = nana_live["mutation-a11y"]
        nana_visibility = nana_live["mutation-visibility"]
    except Exception as exc:  # noqa: BLE001
        errors.append(f"§8.1 envelope fixtures failed to extract: {exc}")
        return errors

    list_pass = judge_runner_invariants(virtual_iced, root=root)
    if list_pass.get("decision") != "ok":
        errors.append("iced VirtualList envelope with live=56 must pass §8.1")
    if _named_invariant_status(list_pass, "virtual_list_live_entities_bounded") != "ok":
        errors.append("iced VirtualList live=56 must evaluate the catalog live bound")

    list_fail_payload = copy.deepcopy(virtual_iced)
    items = virtual["params"]["items"]
    counters = dict(list_fail_payload.get("work_counters") or {})
    counters["live_ui_entities"] = items
    list_fail_payload["work_counters"] = counters
    list_fail = judge_runner_invariants(list_fail_payload, root=root)
    if list_fail.get("decision") != "failed":
        errors.append("VirtualList live_ui_entities==items must fail §8.1")

    table_pass = judge_runner_invariants(table_iced, root=root)
    if table_pass.get("decision") != "ok":
        errors.append("iced text-table envelope must pass §8.1")
    cap_ok_payload = copy.deepcopy(table_iced)
    cap_ok_payload["work_counters"] = dict(cap_ok_payload.get("work_counters") or {})
    cap_ok_payload["work_counters"]["live_ui_entities"] = table_bound
    cap_ok = judge_runner_invariants(cap_ok_payload, root=root)
    if cap_ok.get("decision") != "ok":
        errors.append("table live_ui_entities at catalog-8 cap must pass §8.1")
    if _named_invariant_status(cap_ok, "text_table_live_entities_bounded") != "ok":
        errors.append("table live=catalog cap must measure as ok")

    table_fail_payload = copy.deepcopy(table_iced)
    table_fail_payload["work_counters"] = dict(table_fail_payload.get("work_counters") or {})
    table_fail_payload["work_counters"]["live_ui_entities"] = 1_000_000
    table_fail = judge_runner_invariants(table_fail_payload, root=root)
    if table_fail.get("decision") != "failed":
        errors.append("table live_ui_entities=1e6 must fail §8.1")

    nana_table_judge = judge_runner_invariants(nana_table, root=root)
    if nana_table_judge.get("decision") != "ok":
        errors.append("nana text-table fixture envelope must pass §8.1")
    nana_list_judge = judge_runner_invariants(nana_list, root=root)
    if nana_list_judge.get("decision") != "ok":
        errors.append("nana virtual-list-100k fixture envelope must pass §8.1")
    nana_static_judge = judge_runner_invariants(nana_static, root=root)
    if nana_static_judge.get("decision") != "ok":
        errors.append("nana static-tree-100 fixture envelope must pass §8.1")
    if _named_invariant_status(nana_static_judge, "static_ui") != "ok":
        errors.append("nana StaticTree 100 must evaluate static_ui frames_after_idle==0")

    for payload, scenario_id, rows in (
        (
            nana_transform,
            "mutation-transform",
            [("transform_does_not_layout_full_tree", 0,
              "nana Transform live dump must measure layout_nodes=0")],
        ),
        (
            nana_a11y,
            "mutation-a11y",
            [("a11y_does_not_layout", 0,
              "nana Accessibility live dump must measure layout_nodes=0")],
        ),
        (
            nana_visibility,
            "mutation-visibility",
            [
                ("visibility_does_not_extract_full_tree", 12,
                 "nana Visibility live dump must measure render_nodes_changed=12"),
                ("visibility_does_not_layout_full_tree", 12,
                 "nana Visibility live dump must measure layout_nodes=12 as lte 64, not eq 0"),
            ],
        ),
    ):
        judged = judge_runner_invariants(payload, root=root)
        if judged.get("decision") != "ok":
            errors.append(f"nana {scenario_id} live dump must pass §8.1")
        for name, measured, message in rows:
            if not _named_invariant_ok_measured(judged, name, measured):
                errors.append(message)
    if (nana_visibility.get("work_counters") or {}).get("layout_nodes") != 12:
        errors.append(
            "nana Visibility live dump must keep measured layout_nodes=12; "
            "do not claim or stuff layout_nodes==0"
        )
    vis_eq_zero = copy.deepcopy(nana_visibility)
    vis_eq_zero["scenario_id"] = "mutation-transform"
    vis_as_transform = judge_runner_invariants(vis_eq_zero, root=root)
    if vis_as_transform.get("decision") != "failed":
        errors.append("Visibility layout_nodes=12 must fail Transform's layout_nodes==0 row")

    hover_judge = judge_runner_invariants(hover_iced, root=root)
    if hover_judge.get("decision") != "ok":
        errors.append("iced hover ok envelope must be judged (not skipped)")
    if _named_invariant_status(hover_judge, "hover_without_size_change") != "not-evaluable":
        errors.append("iced hover without layout_nodes must stay not-evaluable, not 0")

    static_judge = judge_runner_invariants(static_iced, root=root)
    if static_judge.get("decision") != "ok":
        errors.append("StaticTree 100 idle envelope with frames_after_idle=0 must pass §8.1")
    if _named_invariant_status(static_judge, "static_ui") != "ok":
        errors.append("StaticTree 100 must evaluate static_ui frames_after_idle==0")
    if (static_iced.get("metrics") or {}).get("frames_after_idle") != 0:
        errors.append("iced StaticTree fixture extract must copy frames_after_idle=0")
    fake_busy = copy.deepcopy(static_iced)
    fake_busy["frames_after_idle"] = 1
    metrics = dict(fake_busy.get("metrics") or {})
    metrics["frames_after_idle"] = 1
    fake_busy["metrics"] = metrics
    busy_judge = judge_runner_invariants(fake_busy, root=root)
    if busy_judge.get("decision") != "failed":
        errors.append("StaticTree frames_after_idle=1 must fail §8.1 static_ui")
    missing_idle = copy.deepcopy(static_iced)
    missing_metrics = dict(missing_idle.get("metrics") or {})
    missing_metrics.pop("frames_after_idle", None)
    missing_idle["metrics"] = missing_metrics
    missing_idle.pop("frames_after_idle", None)
    if judge_runner_invariants(missing_idle, root=root).get("decision") != "skipped":
        errors.append(
            "StaticTree missing frames_after_idle must stay skipped, not vacuous ok"
        )

    paint_judge = judge_runner_invariants(paint_iced, root=root)
    if paint_judge.get("decision") != "ok":
        errors.append("iced paint-only ok envelope must be judged")
    if (
        _named_invariant_status(paint_judge, "paint_only_does_not_layout_full_tree")
        != "not-evaluable"
    ):
        errors.append("iced paint-only without layout_nodes must stay not-evaluable")

    dock_ok_fake = envelope(
        runner="iced",
        status="ok",
        scenario_id="dock-workspace",
        scenario=load_scenario("dock-workspace", root),
        work_counters={"panes": 8},
        equivalence="same-scenario",
    )
    dock_judge = judge_runner_invariants(dock_ok_fake, root=root)
    if dock_judge.get("decision") != "skipped":
        errors.append("Dock ok envelope must be skipped, not invariant-ok")

    gpui_ok_fake = envelope(
        runner="gpui",
        status="ok",
        scenario_id="virtual-list-10k",
        scenario=virtual,
        work_counters={"live_ui_entities": 56},
        equivalence="same-scenario",
    )
    gpui_judge = judge_runner_invariants(gpui_ok_fake, root=root)
    if gpui_judge.get("decision") != "skipped":
        errors.append("GPUI envelope must be skipped, not invariant-ok")

    gpu_scene = load_scenario("gpu-scene-ui", root)
    gpu_live_path = root / "perf" / "fixtures" / "nana-gpu-scene-ui.json"
    try:
        gpu_live = extract_nana(
            gpu_scene,
            {"gpu": load_json(gpu_live_path)},
            source_paths={"gpu": gpu_live_path},
        )
    except Exception as exc:  # noqa: BLE001
        errors.append(f"nana gpu-scene-ui live encode fixture failed to extract: {exc}")
        gpu_live = None
    missing_gpu = envelope(
        runner="nana",
        status="ok",
        scenario_id="gpu-scene-ui",
        scenario=gpu_scene,
        equivalence="same-scenario",
    )
    no_adapter = envelope(
        runner="nana",
        status="unsupported",
        scenario_id="gpu-scene-ui",
        scenario=gpu_scene,
        unsupported_reason=(
            "No WGPU adapter for the hosted GPU scene path. Do not invent upload/batch zeros."
        ),
    )
    if gpu_live is not None:
        if gpu_live.get("status") != "ok" or gpu_live.get("equivalence") != "same-scenario":
            errors.append("nana gpu-scene-ui live encode fixture must extract same-scenario ok")
        if gpu_live.get("relative_gate_enforceable") is not False:
            errors.append("gpu-scene-ui must keep relative_gate_enforceable False")
        live_counters = gpu_live.get("work_counters") or {}
        if (live_counters.get("draw_calls") or 0) < 1:
            errors.append("gpu-scene-ui live dump must not handwrite draw_calls; encode observed < 1")
        if "gpu_upload_bytes" not in live_counters:
            errors.append("gpu-scene-ui live encode fixture must copy observed gpu_upload_bytes")
        if judge_runner_invariants(gpu_live, root=root).get("decision") != "ok":
            errors.append("nana gpu-scene-ui live encode envelope must be judged, not skipped")
        zero_draws = copy.deepcopy(gpu_live)
        zero_draws["work_counters"] = {
            **dict(live_counters),
            "draw_calls": 0,
            "gpu_upload_bytes": 0,
        }
        if judge_runner_invariants(zero_draws, root=root).get("decision") != "failed":
            errors.append(
                "gpu-scene-ui draw_calls=0 must fail §8.1; gpu_upload_bytes>=0 must not greenwash it"
            )
    if judge_runner_invariants(missing_gpu, root=root).get("decision") != "skipped":
        errors.append(
            "gpu-scene-ui ok envelope without GPU keys must skip, not vacuous invariant-ok"
        )
    if judge_runner_invariants(no_adapter, root=root).get("decision") != "skipped":
        errors.append(
            "gpu-scene-ui missing adapter must stay skipped/unsupported, not failed or invariant-ok"
        )
    iced_gpu_ok = envelope(
        runner="iced",
        status="ok",
        scenario_id="gpu-scene-ui",
        scenario=gpu_scene,
        work_counters={"gpu_upload_bytes": 0, "draw_calls": 1},
        equivalence="same-scenario",
    )
    if judge_runner_invariants(iced_gpu_ok, root=root).get("decision") != "skipped":
        errors.append("iced gpu-scene-ui must stay skipped, not invariant-ok")
    for live2d_id in ("gpu-scene-ui-live2d", "gpu-scene-ui-live2d-effect"):
        fake_live2d = envelope(
            runner="nana",
            status="ok",
            scenario_id=live2d_id,
            scenario=load_scenario(live2d_id, root),
            work_counters={"gpu_upload_bytes": 0, "draw_calls": 1},
            equivalence="same-scenario",
        )
        if judge_runner_invariants(fake_live2d, root=root).get("decision") != "skipped":
            errors.append(f"{live2d_id} fake-ok must stay skipped, not invariant-ok")

    fifty = envelope(
        runner="nana",
        status="unsupported",
        scenario_id="static-tree-50k",
        scenario=load_scenario("static-tree-50k", root),
        unsupported_reason=INCOMPARABLE_STATIC_TREE_50K_REASON,
    )
    if judge_runner_invariants(fifty, root=root).get("decision") != "skipped":
        errors.append("static-tree-50k must stay skipped")

    for unsupported_id in (
        "text-editor",
        "animation",
        "ime",
        "overlay",
        "dock-workspace",
    ):
        unsupported = envelope(
            runner="iced",
            status="unsupported",
            scenario_id=unsupported_id,
            scenario=load_scenario(unsupported_id, root),
            unsupported_reason="unsupported fixture",
        )
        if judge_runner_invariants(unsupported, root=root).get("decision") != "skipped":
            errors.append(f"{unsupported_id} unsupported envelope must be skipped")

    raw_fixture = judge_runner_invariants(load_json(virtual_path), root=root)
    if raw_fixture.get("decision") != "error":
        errors.append("raw scenario-bench fixture must not be judged as a runner envelope")

    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        pass_path = tmp_path / "pass.json"
        fail_path = tmp_path / "fail.json"
        skip_path = tmp_path / "skip.json"
        static_dir = tmp_path / "static-only"
        static_dir.mkdir()
        static_only = static_dir / "static.json"
        dump_json(pass_path, virtual_iced)
        dump_json(fail_path, list_fail_payload)
        dump_json(skip_path, fifty)
        dump_json(static_only, static_iced)
        summary_ok, code_ok = evaluate_runner_invariant_paths([pass_path], root=root)
        if code_ok != EXIT_OK or summary_ok.get("status") != "ok":
            errors.append("§8.1 path judge must pass a live VirtualList envelope")
        summary_fail, code_fail = evaluate_runner_invariant_paths([fail_path], root=root)
        if code_fail != EXIT_ERROR or summary_fail.get("status") != "failed":
            errors.append("§8.1 path judge must fail live==items")
        summary_skip, code_skip = evaluate_runner_invariant_paths([skip_path], root=root)
        if code_skip != EXIT_UNSUPPORTED or summary_skip.get("status") != "unsupported":
            errors.append("§8.1 path judge on only-skipped reports must exit 2")
        summary_static, code_static = evaluate_runner_invariant_paths(
            [static_only], root=root
        )
        if (
            code_static != EXIT_OK
            or summary_static.get("status") != "ok"
            or summary_static.get("ok") != 1
            or summary_static.get("skipped") != 0
        ):
            errors.append("§8.1 StaticTree-only dump with frames_after_idle=0 must pass")
        extra_dir = tmp_path / "idle-cases"
        extra_dir.mkdir()
        missing_path = extra_dir / "static-missing.json"
        dump_json(missing_path, missing_idle)
        summary_missing, code_missing = evaluate_runner_invariant_paths(
            [missing_path], root=root
        )
        if (
            code_missing != EXIT_UNSUPPORTED
            or summary_missing.get("status") != "unsupported"
            or summary_missing.get("skipped") != 1
        ):
            errors.append("§8.1 StaticTree missing frames_after_idle must skip")
        busy_path = extra_dir / "static-busy.json"
        dump_json(busy_path, fake_busy)
        summary_busy, code_busy = evaluate_runner_invariant_paths([busy_path], root=root)
        if code_busy != EXIT_ERROR or summary_busy.get("status") != "failed":
            errors.append("§8.1 StaticTree frames_after_idle=1 must fail")
        mixed, mixed_code = evaluate_runner_invariant_paths(
            [pass_path, skip_path], root=root
        )
        if mixed_code != EXIT_OK or mixed.get("skipped") != 1 or mixed.get("ok") != 1:
            errors.append("§8.1 mixed ok+skipped envelopes must exit 0")
        dir_summary, dir_code = evaluate_runner_invariant_paths([tmp_path], root=root)
        if dir_code != EXIT_ERROR or dir_summary.get("failed") != 1:
            errors.append("§8.1 directory judge must fail-closed when a file fails")
        if gpu_live is not None:
            gpu_env = tmp_path / "gpu-scene-ui.json"
            dump_json(gpu_env, gpu_live)
            gpu_summary, gpu_code = evaluate_runner_invariant_paths([gpu_env], root=root)
            if gpu_code != EXIT_OK or gpu_summary.get("status") != "ok":
                errors.append("§8.1 path judge must pass a live gpu-scene-ui encode envelope")
        missing_gpu_path = tmp_path / "gpu-scene-ui-missing.json"
        dump_json(missing_gpu_path, missing_gpu)
        missing_summary, missing_code = evaluate_runner_invariant_paths(
            [missing_gpu_path], root=root
        )
        if missing_code != EXIT_UNSUPPORTED or missing_summary.get("skipped") != 1:
            errors.append(
                "§8.1 gpu-scene-ui without GPU keys must exit 2, not vacuous ok"
            )
        cli = subprocess.run(
            [
                sys.executable,
                str(root / "perf" / "contract.py"),
                "--repo-root",
                str(root),
                "--evaluate-invariants",
                str(pass_path),
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if cli.returncode != EXIT_OK:
            errors.append(f"--evaluate-invariants on a passing envelope must exit 0, got {cli.returncode}")
        else:
            try:
                payload = json.loads(cli.stdout)
                if payload.get("status") != "ok" or payload.get("ok") != 1:
                    errors.append("--evaluate-invariants stdout must be a status=ok summary")
            except json.JSONDecodeError:
                errors.append("--evaluate-invariants stdout must be JSON")
        fail_cli = subprocess.run(
            [
                sys.executable,
                str(root / "perf" / "contract.py"),
                "--repo-root",
                str(root),
                "--evaluate-invariants",
                str(fail_path),
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if fail_cli.returncode != EXIT_ERROR:
            errors.append(
                f"--evaluate-invariants on live==items must exit 1, got {fail_cli.returncode}"
            )
    return errors


def _self_test_catalog_workloads(
    root: Path,
    runtime_path: Path,
    iced_path: Path,
    reserved: set[Any],
) -> list[str]:
    errors: list[str] = []
    harness = set(load_catalog(root).get("harness_ids", []))
    wirable = ("animation", "ime", "dock-workspace", "overlay", "text-editor", "gpu-scene-ui")
    for scenario_id in wirable:
        if scenario_id not in harness:
            errors.append(f"catalog must list wirable {scenario_id} in harness_ids")
        if scenario_id in reserved:
            errors.append(
                f"catalog must not leave wirable {scenario_id} in required_by_issue_not_in_harness"
            )
    for live2d_id in ("gpu-scene-ui-live2d", "gpu-scene-ui-live2d-effect"):
        if live2d_id in harness:
            errors.append(f"catalog must not list {live2d_id} in harness_ids")
        if live2d_id not in reserved:
            errors.append(
                f"catalog must keep {live2d_id} in required_by_issue_not_in_harness"
            )

    animation = load_scenario("animation", root)
    try:
        extract_nana(
            animation,
            {"runtime": load_json(runtime_path)},
            source_paths={"runtime": runtime_path},
        )
        errors.append(
            "nana animation extract from historical runtime report must KeyError "
            "without catalog_animation"
        )
    except KeyError as exc:
        if "catalog_animation" not in key_error_reason(exc):
            errors.append(f"animation KeyError should name catalog_animation: {exc}")

    try:
        extract_nana(
            animation,
            {
                "runtime": {
                    "cases": [
                        {
                            "nodes": 5000,
                            "sparse_animation_sample_ms": {"p50": 0.1},
                            "due_animation_samples": 1,
                            "idle_animation_deadline_ms": {"p50": 0.01},
                            "scheduled_animation_deadline_ms": {"p50": 0.02},
                            "scheduled_animations": 5000,
                        }
                    ]
                }
            },
            source_paths={"runtime": Path("synthetic-legacy-5k-animation")},
        )
        errors.append(
            "nana animation must not treat 5k-tree sparse sample as catalog_animation"
        )
    except KeyError as exc:
        if "catalog_animation" not in key_error_reason(exc):
            errors.append(f"animation KeyError should name catalog_animation: {exc}")

    anim_fixture = load_json(root / "perf" / "fixtures" / "catalog-animation.json")
    try:
        animated = extract_nana(
            animation,
            {"runtime": anim_fixture},
            source_paths={"runtime": Path("catalog-animation.json")},
        )
        if animated.get("status") != "ok":
            errors.append("nana animation extract from catalog-animation fixture must be ok")
        if (animated.get("work_counters") or {}).get("due_animation_samples") != 1:
            errors.append("animation envelope must copy due_animation_samples=1")
        anim_inv = None
        for item in animated.get("invariants") or []:
            if item.get("name") == "idle_scheduled_animations_sparse_sample":
                anim_inv = item
                break
        if anim_inv is None or anim_inv.get("status") != "ok":
            errors.append(
                "animation due_animation_samples invariant must be ok on catalog-animation fixture"
            )
    except Exception as exc:  # noqa: BLE001
        errors.append(f"nana animation catalog-animation fixture extract failed: {exc}")

    fixture = load_json(root / "perf" / "fixtures" / "catalog-workloads.json")
    reports = {"framework": fixture}
    paths = {"framework": Path("catalog-workloads.json")}
    ime = extract_nana(load_scenario("ime", root), reports, source_paths=paths)
    if ime.get("status") != "ok":
        errors.append("nana ime extract must be ok when catalog_workloads ime status=ok")
    if (ime.get("work_counters") or {}).get("ime_script_count") != 4:
        errors.append("ime envelope must copy ime_script_count=4")
    dock = extract_nana(load_scenario("dock-workspace", root), reports, source_paths=paths)
    if dock.get("status") != "ok":
        errors.append("nana dock-workspace extract must be ok when catalog_workloads status=ok")
    if (dock.get("work_counters") or {}).get("panes") != 8:
        errors.append("dock-workspace envelope must copy panes=8")
    overlay = extract_nana(load_scenario("overlay", root), reports, source_paths=paths)
    if overlay.get("status") != "ok":
        errors.append("nana overlay extract must be ok when catalog_workloads status=ok")
    if (overlay.get("work_counters") or {}).get("overlay_kind_count") != 4:
        errors.append("overlay envelope must copy overlay_kind_count=4")
    editor = extract_nana(load_scenario("text-editor", root), reports, source_paths=paths)
    if editor.get("status") != "ok":
        errors.append("nana text-editor extract must be ok when catalog_workloads status=ok")
    if (editor.get("work_counters") or {}).get("text_shaped") != 1:
        errors.append("text-editor envelope must export text_shaped=1")
    editor_inv = None
    for item in editor.get("invariants") or []:
        if item.get("name") == "text_editor_local_edit_shapes_bounded_nodes":
            editor_inv = item
            break
    if editor_inv is None or editor_inv.get("status") != "ok":
        errors.append("text-editor local-edit invariant must be ok when text_shaped=1")

    try:
        extract_nana(
            load_scenario("ime", root),
            {
                "framework": {
                    "catalog_workloads": [
                        {"id": "ime", "kind": "Ime", "status": "skipped", "skip_reason": "missing"}
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-ime-skipped")},
        )
        errors.append("nana ime skipped catalog_workloads must KeyError, not ok")
    except KeyError as exc:
        if "ime" not in key_error_reason(exc):
            errors.append(f"ime skip KeyError should name ime: {exc}")

    for scenario_id in wirable:
        try:
            extract_iced(
                load_scenario(scenario_id, root),
                load_json(iced_path),
                source_path=iced_path,
            )
            errors.append(f"iced {scenario_id} extract must be unsupported")
        except KeyError:
            pass
    return errors


def _self_test_from_report_cli(root: Path) -> list[str]:
    """Exercise --from-report through runners, not only extract_*."""
    errors: list[str] = []
    nana_script = root / "perf" / "runners" / "nana" / "run.py"
    iced_script = root / "perf" / "runners" / "iced" / "run.py"

    def from_report(
        script: Path, scenario_id: str, fixture: Path
    ) -> tuple[int, dict[str, Any] | None, str]:
        result = subprocess.run(
            [
                sys.executable,
                str(script),
                "--repo-root",
                str(root),
                "--scenario",
                scenario_id,
                "--from-report",
                str(fixture),
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != EXIT_OK:
            return result.returncode, None, result.stderr
        try:
            return result.returncode, json.loads(result.stdout), result.stderr
        except json.JSONDecodeError as exc:
            return result.returncode, None, str(exc)

    def _from_report_same_scenario(code, report, err, label, live_ui_entities):
        if code != EXIT_OK or report is None:
            errors.append(f"{label} --from-report exit {code}: {err}")
        elif report.get("status") != "ok":
            errors.append(f"{label} --from-report status={report.get('status')}")
        elif report.get("equivalence") != "same-scenario":
            errors.append(f"{label} --from-report must be same-scenario")
        elif report.get("relative_gate_enforceable") is not False:
            errors.append(
                f"{label} --from-report must keep relative_gate_enforceable False"
            )
        elif (report.get("work_counters") or {}).get("live_ui_entities") != live_ui_entities:
            errors.append(
                f"{label} --from-report must copy live_ui_entities={live_ui_entities}"
            )

    code, report, err = from_report(
        nana_script,
        "virtual-list-100k",
        root / "perf" / "fixtures" / "virtual-scales-only.json",
    )
    _from_report_same_scenario(code, report, err, "virtual_scales-only", 57)

    code, tree_report, err = from_report(
        nana_script,
        "virtual-tree-100k",
        root / "perf" / "fixtures" / "virtual-tree-scales.json",
    )
    _from_report_same_scenario(code, tree_report, err, "virtual-tree-scales", 57)

    code, table_report, err = from_report(
        nana_script,
        "text-table",
        root / "perf" / "fixtures" / "virtual-table-scales.json",
    )
    if code != EXIT_OK or table_report is None:
        errors.append(f"virtual-table-scales --from-report exit {code}: {err}")
    else:
        if table_report.get("status") != "ok":
            errors.append(f"virtual-table-scales --from-report status={table_report.get('status')}")
        if table_report.get("equivalence") != "same-scenario":
            errors.append("virtual-table-scales --from-report must be same-scenario")
        if table_report.get("relative_gate_enforceable") is not False:
            errors.append(
                "virtual-table-scales --from-report must keep relative_gate_enforceable False"
            )
        table_from_report = table_report.get("work_counters") or {}
        expected_table = {
            "live_ui_entities": 1100,
            "text_shaped": 864,
            "text_wrap_layouts": 1728,
            "text_shaped_runs": 1728,
            "text_layout_cache_hits": 0,
            "text_layout_cache_misses": 1728,
            "cache_eviction": 0,
            "wrapped_cells": 4,
        }
        for key, value in expected_table.items():
            if table_from_report.get(key) != value:
                errors.append(
                    f"virtual-table-scales --from-report must copy {key}={value}"
                )
        for key in TEXT_TABLE_EXPORTED_SHAPE_KEYS:
            if key not in table_from_report:
                errors.append(f"virtual-table-scales --from-report must contain {key}")

    code, ime_report, err = from_report(
        nana_script,
        "ime",
        root / "perf" / "fixtures" / "catalog-workloads.json",
    )
    if code != EXIT_OK or ime_report is None:
        errors.append(f"ime catalog-workloads --from-report exit {code}: {err}")
    elif ime_report.get("status") != "ok":
        errors.append(f"ime catalog-workloads --from-report status={ime_report.get('status')}")
    elif (ime_report.get("work_counters") or {}).get("ime_script_count") != 4:
        errors.append("ime catalog-workloads --from-report must copy ime_script_count=4")

    code, iced_report, err = from_report(
        iced_script,
        "static-tree-100",
        root / "perf" / "fixtures" / "iced-scenario-static-tree-100.json",
    )
    if code != EXIT_OK or iced_report is None:
        errors.append(f"iced-scenario-bench --from-report exit {code}: {err}")
    else:
        if iced_report.get("equivalence") != "same-scenario":
            errors.append("iced-scenario-bench --from-report must be same-scenario")
        if iced_report.get("relative_gate_enforceable") is not False:
            errors.append("iced-scenario-bench --from-report must keep relative gates off")
        if iced_report.get("status") != "ok":
            errors.append(f"iced-scenario-bench --from-report status={iced_report.get('status')}")
        elif (iced_report.get("metrics") or {}).get("frames_after_idle") != 0:
            errors.append("iced static-tree-100 --from-report must copy frames_after_idle=0")
        if not isinstance((iced_report.get("metrics") or {}).get("busy_probe_frames"), int) or (
            iced_report.get("metrics") or {}
        ).get("busy_probe_frames", 0) < 1:
            errors.append("iced static-tree-100 --from-report must copy busy_probe_frames > 0")

    nana_static_live = root / "perf" / "fixtures" / "nana-runtime-static-tree.json"
    nana_live = load_json(nana_static_live)
    if nana_live.get("profile") != "release" or nana_live.get("storage") == "extractor-fixture":
        errors.append("nana-runtime-static-tree.json must be a live release dump")
    live_cases = {
        case.get("nodes"): case
        for case in nana_live.get("cases") or []
        if isinstance(case, Mapping)
    }
    for nodes in (100, 1000, 5000, 10000):
        case = live_cases.get(nodes)
        if not isinstance(case, Mapping) or case.get("kind") != "full":
            errors.append(f"live nana dump must include kind=full nodes={nodes}")
        elif case.get("frames_after_idle") != 0:
            errors.append(f"live nana nodes={nodes} must have frames_after_idle=0")
    fifty_case = live_cases.get(50000)
    if isinstance(fifty_case, Mapping) and "frames_after_idle" in fifty_case:
        errors.append("live nana 50k construction must omit frames_after_idle")
    hover_case = live_cases.get(10000)
    if not isinstance(hover_case, Mapping) or "pointer_hover_transition_ms" not in hover_case:
        errors.append("live nana dump must include 10k pointer_hover_transition_ms")
    mut_case = live_cases.get(5000)
    if not isinstance(mut_case, Mapping) or "local_paint_systems_ms" not in mut_case:
        errors.append("live nana dump must include 5k local_paint_systems_ms")
    else:
        block = mut_case.get("single_node_mutations") or {}
        layout_honesty = {
            "Visibility": (
                lambda n: n != 0,
                "live nana Visibility must not report layout_nodes=0; last dump layouts 12",
            ),
            "Transform": (lambda n: n == 0, "live nana Transform must measure layout_nodes=0"),
            "Accessibility": (
                lambda n: n == 0,
                "live nana Accessibility must measure layout_nodes=0",
            ),
        }
        for kind in ("Text", "LayoutStyle", "Visibility", "Transform", "Accessibility"):
            drain = block.get(kind)
            work = drain.get("work") if isinstance(drain, Mapping) else None
            if not isinstance(drain, Mapping) or not isinstance(work, Mapping):
                errors.append(
                    f"live nana dump must include 5k single_node_mutations.{kind} WorkCounters"
                )
                continue
            if "layout_nodes" not in work:
                errors.append(
                    f"live nana dump single_node_mutations.{kind} must measure layout_nodes"
                )
                continue
            check = layout_honesty.get(kind)
            if check is not None and not check[0](work.get("layout_nodes")):
                errors.append(check[1])

    expected_mutation_layout = {
        "mutation-visibility": 12,
        "mutation-transform": 0,
        "mutation-a11y": 0,
    }
    for scenario_id in (
        "static-tree-100",
        "static-tree-1k",
        "static-tree-5k",
        "static-tree-10k",
        "hover",
        "mutation-paint-only",
        "mutation-text",
        "mutation-layout-style",
        "mutation-visibility",
        "mutation-transform",
        "mutation-a11y",
    ):
        code, nana_static_report, err = from_report(
            nana_script,
            scenario_id,
            nana_static_live,
        )
        if code != EXIT_OK or nana_static_report is None:
            errors.append(f"nana {scenario_id} --from-report exit {code}: {err}")
        elif nana_static_report.get("status") != "ok":
            errors.append(
                f"nana {scenario_id} --from-report status={nana_static_report.get('status')}"
            )
        elif scenario_id.startswith("static-tree-") and (
            nana_static_report.get("metrics") or {}
        ).get("frames_after_idle") != 0:
            errors.append(f"nana {scenario_id} --from-report must copy frames_after_idle=0")
        elif scenario_id in expected_mutation_layout and (
            nana_static_report.get("work_counters") or {}
        ).get("layout_nodes") != expected_mutation_layout[scenario_id]:
            errors.append(
                f"nana {scenario_id} --from-report must copy layout_nodes="
                f"{expected_mutation_layout[scenario_id]}"
            )

    hover_iced = subprocess.run(
        [
            sys.executable,
            str(iced_script),
            "--repo-root",
            str(root),
            "--scenario",
            "hover",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if hover_iced.returncode != EXIT_OK or "scenario-bench" not in hover_iced.stdout:
        errors.append(
            f"iced hover --print-plan must name scenario-bench: {hover_iced.stdout!r} {hover_iced.stderr}"
        )

    code, hover_report, err = from_report(
        iced_script,
        "hover",
        root / "perf" / "fixtures" / "iced-scenario-hover.json",
    )
    if code != EXIT_OK or hover_report is None:
        errors.append(f"iced hover --from-report exit {code}: {err}")
    elif hover_report.get("status") != "ok" or hover_report.get("equivalence") != "same-scenario":
        errors.append("iced hover --from-report must be same-scenario ok")
    elif (hover_report.get("metrics") or {}).get("cpu_frame_ms", {}).get("p50") != 2.1:
        errors.append("iced hover --from-report must copy fixture cpu_frame_ms.p50")

    for fake_ok_id, token in (
        ("dock-workspace", "assemble_dock"),
        ("text-editor", "drain_text"),
    ):
        fake_plan = subprocess.run(
            [
                sys.executable,
                str(iced_script),
                "--repo-root",
                str(root),
                "--scenario",
                fake_ok_id,
                "--print-plan",
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if fake_plan.returncode != EXIT_OK or "unsupported" not in fake_plan.stdout:
            errors.append(
                f"iced {fake_ok_id} --print-plan must be unsupported: "
                f"{fake_plan.stdout!r} {fake_plan.stderr}"
            )
        fake_ok = {
            "source": "iced-scenario-bench",
            "status": "ok",
            "scenario_id": fake_ok_id,
            "cpu_frame_ms": {"p50": 1.0, "p95": 1.2, "p99": 1.3},
        }
        with tempfile.TemporaryDirectory() as tmp:
            fake_path = Path(tmp) / f"fake-ok-{fake_ok_id}.json"
            fake_path.write_text(json.dumps(fake_ok), encoding="utf-8")
            code, fake_report, err = from_report(iced_script, fake_ok_id, fake_path)
            if code != EXIT_UNSUPPORTED:
                errors.append(
                    f"iced {fake_ok_id} fake-ok --from-report must exit 2, got {code}: {err}"
                )
            elif fake_report is not None and (
                fake_report.get("status") == "ok"
                or fake_report.get("equivalence") == "same-scenario"
            ):
                errors.append(f"iced {fake_ok_id} fake-ok --from-report must not be same-scenario")
            try:
                extracted = extract_iced(
                    load_scenario(fake_ok_id, root),
                    fake_ok,
                    source_path=fake_path,
                )
                if extracted.get("status") == "ok":
                    errors.append(f"iced {fake_ok_id} fake-ok extract must KeyError")
            except KeyError as exc:
                if token not in key_error_reason(exc):
                    errors.append(
                        f"iced {fake_ok_id} fake-ok KeyError should name {token}: {exc}"
                    )

    code, fifty_report, err = from_report(
        iced_script,
        "static-tree-50k",
        root / "perf" / "fixtures" / "iced-scenario-static-tree-100.json",
    )
    if code != EXIT_UNSUPPORTED:
        errors.append(f"iced static-tree-50k must exit 2, got {code}: {err}")
    elif fifty_report is None:
        # runner prints JSON on stdout even for exit 2
        pass

    fifty_iced = subprocess.run(
        [
            sys.executable,
            str(iced_script),
            "--repo-root",
            str(root),
            "--scenario",
            "static-tree-50k",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if fifty_iced.returncode != EXIT_OK or "unsupported" not in fifty_iced.stdout:
        errors.append(
            f"iced static-tree-50k --print-plan must be unsupported: {fifty_iced.stdout!r}"
        )

    nana_gpu_plan = subprocess.run(
        [
            sys.executable,
            str(nana_script),
            "--repo-root",
            str(root),
            "--scenario",
            "gpu-scene-ui",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if nana_gpu_plan.returncode != EXIT_OK or "nana-gpu-scene-benchmark" not in nana_gpu_plan.stdout:
        errors.append(
            f"nana gpu-scene-ui --print-plan must name nana-gpu-scene-benchmark: "
            f"{nana_gpu_plan.stdout!r} {nana_gpu_plan.stderr}"
        )
    elif "gpu-scene-ui.json" not in nana_gpu_plan.stdout:
        errors.append(
            f"nana gpu-scene-ui --print-plan must pass --scenario gpu-scene-ui.json: "
            f"{nana_gpu_plan.stdout!r}"
        )

    gpu_live_code, gpu_live_report, gpu_live_err = from_report(
        nana_script, "gpu-scene-ui", root / "perf" / "fixtures" / "nana-gpu-scene-ui.json"
    )
    if gpu_live_code != EXIT_OK or gpu_live_report is None:
        errors.append(f"nana gpu-scene-ui live encode --from-report must exit 0: {gpu_live_err}")
    elif gpu_live_report.get("status") != "ok":
        errors.append(
            f"nana gpu-scene-ui live encode status={gpu_live_report.get('status')}"
        )
    elif gpu_live_report.get("relative_gate_enforceable") is not False:
        errors.append("nana gpu-scene-ui --from-report must keep relative_gate_enforceable False")
    elif (gpu_live_report.get("work_counters") or {}).get("draw_calls", 0) < 1:
        errors.append("nana gpu-scene-ui live encode must copy draw_calls >= 1")

    runtime_gpu_code, _, runtime_gpu_err = from_report(
        nana_script,
        "gpu-scene-ui",
        root / "perf" / "fixtures" / "nana-runtime-static-tree.json",
    )
    if runtime_gpu_code != EXIT_UNSUPPORTED:
        errors.append(
            f"nana gpu-scene-ui from a runtime-only dump must exit 2, got {runtime_gpu_code}: "
            f"{runtime_gpu_err}"
        )

    for live2d_id in ("gpu-scene-ui-live2d", "gpu-scene-ui-live2d-effect"):
        live2d_plan = subprocess.run(
            [
                sys.executable,
                str(nana_script),
                "--repo-root",
                str(root),
                "--scenario",
                live2d_id,
                "--print-plan",
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if live2d_plan.returncode != EXIT_OK or "Live2D" not in live2d_plan.stdout:
            errors.append(
                f"nana {live2d_id} --print-plan must stay Live2D unsupported: "
                f"{live2d_plan.stdout!r} {live2d_plan.stderr}"
            )
        live2d_run = subprocess.run(
            [
                sys.executable,
                str(nana_script),
                "--repo-root",
                str(root),
                "--scenario",
                live2d_id,
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if live2d_run.returncode != EXIT_UNSUPPORTED:
            errors.append(
                f"nana {live2d_id} must exit 2, got {live2d_run.returncode}: "
                f"{live2d_run.stderr}"
            )
        else:
            try:
                live2d_payload = json.loads(live2d_run.stdout)
                if live2d_payload.get("status") != "unsupported":
                    errors.append(
                        f"nana {live2d_id} status={live2d_payload.get('status')}; "
                        "do not invent a Live2D encode"
                    )
                if live2d_payload.get("metrics") or live2d_payload.get("work_counters"):
                    errors.append(f"nana {live2d_id} must not invent GPU metrics or counters")
                reason = live2d_payload.get("unsupported_reason") or ""
                if "Live2D" not in reason:
                    errors.append(
                        f"nana {live2d_id} unsupported_reason should name Live2D: {reason!r}"
                    )
            except json.JSONDecodeError as exc:
                errors.append(f"nana {live2d_id} stdout must be JSON: {exc}")

        iced_live2d = subprocess.run(
            [
                sys.executable,
                str(iced_script),
                "--repo-root",
                str(root),
                "--scenario",
                live2d_id,
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if iced_live2d.returncode != EXIT_UNSUPPORTED:
            errors.append(
                f"iced {live2d_id} must exit 2, got {iced_live2d.returncode}: "
                f"{iced_live2d.stderr}"
            )

    nana_fifty = subprocess.run(
        [
            sys.executable,
            str(nana_script),
            "--repo-root",
            str(root),
            "--scenario",
            "static-tree-50k",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if nana_fifty.returncode != EXIT_OK or "unsupported" not in nana_fifty.stdout:
        errors.append(
            f"nana static-tree-50k --print-plan must be unsupported: {nana_fifty.stdout!r}"
        )

    for unsupported_id in (
        "mutation-transform",
        "mutation-visibility",
        "mutation-a11y",
        "animation",
        "ime",
        "overlay",
        "gpu-scene-ui",
        "dock-workspace",
        "text-editor",
        "virtual-tree-100k",
    ):
        iced_unsupported = subprocess.run(
            [
                sys.executable,
                str(iced_script),
                "--repo-root",
                str(root),
                "--scenario",
                unsupported_id,
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if iced_unsupported.returncode != EXIT_UNSUPPORTED:
            errors.append(
                f"iced {unsupported_id} must exit 2, got {iced_unsupported.returncode}: "
                f"{iced_unsupported.stderr}"
            )
        else:
            try:
                payload = json.loads(iced_unsupported.stdout)
                if payload.get("status") == "ok" or payload.get("equivalence") == "same-scenario":
                    errors.append(f"iced {unsupported_id} must not emit same-scenario ok")
            except json.JSONDecodeError as exc:
                errors.append(f"iced {unsupported_id} stdout must be JSON: {exc}")

    for list_id in ("virtual-list-10k", "virtual-list-100k", "virtual-tree-10k", "virtual-tree-100k"):
        nana_list = subprocess.run(
            [
                sys.executable,
                str(nana_script),
                "--repo-root",
                str(root),
                "--scenario",
                list_id,
                "--print-plan",
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if nana_list.returncode != EXIT_OK or "--list-overscan-px" not in nana_list.stdout:
            errors.append(
                f"nana {list_id} --print-plan must pass catalog --list-overscan-px: "
                f"{nana_list.stdout!r}"
            )
        elif "160" not in nana_list.stdout:
            errors.append(
                f"nana {list_id} --print-plan must pass overscan 160px (8×20): "
                f"{nana_list.stdout!r}"
            )

    nana_table = subprocess.run(
        [
            sys.executable,
            str(nana_script),
            "--repo-root",
            str(root),
            "--scenario",
            "text-table",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if nana_table.returncode != EXIT_OK or "--table-overscan-y-px" not in nana_table.stdout:
        errors.append(
            f"nana text-table --print-plan must pass --table-overscan-y-px: "
            f"{nana_table.stdout!r}"
        )
    elif "160" not in nana_table.stdout:
        errors.append(
            f"nana text-table --print-plan must pass overscan y 160px (8×20): "
            f"{nana_table.stdout!r}"
        )
    if "--table-overscan-y-px" in (
        subprocess.run(
            [
                sys.executable,
                str(nana_script),
                "--repo-root",
                str(root),
                "--scenario",
                "virtual-list-10k",
                "--print-plan",
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        ).stdout
    ):
        errors.append("nana virtual-list-10k --print-plan must not pass table flags")

    table_iced_plan = subprocess.run(
        [
            sys.executable,
            str(iced_script),
            "--repo-root",
            str(root),
            "--scenario",
            "text-table",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if table_iced_plan.returncode != EXIT_OK or "scenario-bench" not in table_iced_plan.stdout:
        errors.append(
            f"iced text-table --print-plan must name scenario-bench: "
            f"{table_iced_plan.stdout!r} {table_iced_plan.stderr}"
        )
    code, table_from_report, err = from_report(
        iced_script,
        "text-table",
        root / "perf" / "fixtures" / "iced-scenario-text-table.json",
    )
    if code != EXIT_OK or table_from_report is None:
        errors.append(f"iced text-table --from-report exit {code}: {err}")
    elif table_from_report.get("status") != "ok" or table_from_report.get("equivalence") != "same-scenario":
        errors.append("iced text-table --from-report must be same-scenario ok")

    gpui_script = root / "perf" / "runners" / "gpui" / "run.py"
    gpui_result = subprocess.run(
        [
            sys.executable,
            str(gpui_script),
            "--repo-root",
            str(root),
            "--scenario",
            "static-tree-100",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if gpui_result.returncode != EXIT_UNSUPPORTED:
        errors.append(
            f"gpui static-tree-100 must exit 2, got {gpui_result.returncode}: {gpui_result.stderr}"
        )
    else:
        try:
            gpui_report = json.loads(gpui_result.stdout)
            if gpui_report.get("status") != "unsupported":
                errors.append(f"gpui stub status={gpui_report.get('status')}")
            if gpui_report.get("metrics"):
                errors.append("gpui stub must not invent metrics")
        except json.JSONDecodeError as exc:
            errors.append(f"gpui stub stdout is not JSON: {exc}")

    plan = subprocess.run(
        [
            sys.executable,
            str(iced_script),
            "--repo-root",
            str(root),
            "--scenario",
            "static-tree-100",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if plan.returncode != EXIT_OK or "scenario-bench" not in plan.stdout:
        errors.append(
            f"iced static-tree-100 --print-plan must name scenario-bench: {plan.stdout!r} {plan.stderr}"
        )
    return errors


def gpui_unsupported(scenario: Mapping[str, Any]) -> dict[str, Any]:
    return envelope(
        runner="gpui",
        status="unsupported",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence="unsupported",
        unsupported_reason=(
            "NanaUI has no GPUI crate, workspace member, or adapter. Fake GPUI numbers "
            "are forbidden. This stub implements the Issue #8 CLI/schema so CI can "
            "distinguish unsupported (exit 2) from a failed run (exit 1)."
        ),
        plug_in=(
            "Add perf/runners/gpui/adapter.py implementing run_scenario(scenario, args) "
            "that builds the same Scenario JSON and returns this envelope with status=ok. "
            "Do not commit invented timings."
        ),
    )


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Issue #8 scenario schema helpers")
    parser.add_argument("--check-schema", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--evaluate-invariants",
        nargs="+",
        metavar="REPORT",
        default=None,
        help=(
            "Judge §8.1 work invariants from runner envelope JSON files or directories. "
            "Same evaluate_invariants engine runners already attach."
        ),
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="Write --evaluate-invariants summary JSON (default: stdout)",
    )
    parser.add_argument("--repo-root", type=Path, default=REPO_ROOT)
    args = parser.parse_args(argv)
    root = args.repo_root.resolve()
    if args.self_test and args.evaluate_invariants:
        parser.error("use --self-test or --evaluate-invariants, not both")
    if args.self_test:
        errors = self_test(root)
        if errors:
            for error in errors:
                print(error, file=sys.stderr)
            return EXIT_ERROR
        print("perf contract self-test: OK")
        return EXIT_OK
    if args.evaluate_invariants:
        try:
            summary, code = evaluate_runner_invariant_paths(
                args.evaluate_invariants, root=root
            )
        except FileNotFoundError as exc:
            print(str(exc), file=sys.stderr)
            return EXIT_ERROR
        dump_json(args.output, summary)
        return code
    if args.check_schema:
        errors = validate_all_scenarios(root)
        if errors:
            for error in errors:
                print(error, file=sys.stderr)
            return EXIT_ERROR
        print("scenario schema: OK")
        return EXIT_OK
    parser.error("provide --check-schema, --self-test, or --evaluate-invariants")
    return EXIT_ERROR


if __name__ == "__main__":
    raise SystemExit(main())
