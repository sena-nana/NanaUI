"""Scenario data contracts and catalog validation."""
from __future__ import annotations

import json
import math
import sys
from pathlib import Path
from typing import Any, Mapping, Sequence




SCHEMA_VERSION = 1

EXIT_OK = 0

EXIT_ERROR = 1

EXIT_UNSUPPORTED = 2


# StaticTree JSON only carries params.nodes. Hierarchy/kind/text are this rule,
# shared by Nana `tree_mutations` (nana-runtime-benchmark.rs) and Iced
# `static_tree` / `static_tree_parent` (historical iced scenario-bench fixtures).
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


PERF_ROOT = Path(__file__).resolve().parent.parent

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

ICED_SNAPSHOT_REMOVED_REASON = (
    "engine/iced snapshot was removed; live scenario-bench is gone. "
    "Issue #12 observation uses --from-report fixtures only."
)

GPUI_SNAPSHOT_REMOVED_REASON = (
    "engine/gpui-scenario-bench snapshot was removed; live gpui-scenario-bench is gone. "
    "Issue #12 observation uses --from-report fixtures only."
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



def gpui_scenario_bench_skip_reason(scenario: Mapping[str, Any]) -> str | None:
    """Same skip kinds as Iced; wording swapped to GPUI."""
    reason = iced_scenario_bench_skip_reason(scenario)
    if reason is None:
        return None
    return reason.replace("Iced", "GPUI").replace("iced", "GPUI")



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



# Nana catalog ids with a dirty/sparse hotspot that can fail. Identity
# attendance is not a pass. Completeness applies only to a PR
# ``invariants/`` directory; weekly ``weekly/`` does not require macos-only
# ``gpu-scene-ui``. Present gated skip among oks is exit 1. See §8.1.
SECTION_8_1_STATIC_UI_IDS = frozenset(
    {
        "static-tree-100",
        "static-tree-1k",
        "static-tree-5k",
        "static-tree-10k",
    }
)

SECTION_8_1_CATALOG_WORKLOAD_IDS = frozenset(
    {
        "animation",
        "ime",
        "dock-workspace",
        "overlay",
        "text-editor",
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
        *SECTION_8_1_CATALOG_WORKLOAD_IDS,
        *SECTION_8_1_STATIC_UI_IDS,
    }
)

SECTION_8_1_UNSUPPORTED_IDS = frozenset(
    {
        "static-tree-50k",
        "gpu-scene-ui-live2d",
        "gpu-scene-ui-live2d-effect",
        "virtual-list-1m",
        "virtual-tree-1m",
    }
)

# Ubuntu weekly ``cpu-runtime-scene`` maps these ids into
# ``target/performance/issue8/weekly``. ``gpu-scene-ui`` stays on the
# macos encode job and must not be required here. ``animation`` is mapped
# and §8.1 gated on the sparse-advance counter.
SECTION_8_1_WEEKLY_UBUNTU_IDS = frozenset(
    {
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
        "animation",
        "virtual-list-10k",
        "virtual-list-100k",
        "virtual-tree-10k",
        "virtual-tree-100k",
        "text-table",
        "ime",
        "dock-workspace",
        "overlay",
        "text-editor",
    }
)

PR_INVARIANTS_DIR_NAME = "invariants"



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
    issue12 = catalog.get("issue12") if isinstance(catalog.get("issue12"), Mapping) else {}
    same = list(issue12.get("same_scenario_ids") or [])
    unsupported = list(issue12.get("unsupported_ids") or [])
    harness = set(catalog["harness_ids"])
    if not same or not unsupported:
        errors.append("catalog issue12 must list same_scenario_ids and unsupported_ids")
    overlap = set(same) & set(unsupported)
    if overlap:
        errors.append(
            "catalog issue12 same_scenario_ids and unsupported_ids overlap: "
            + ", ".join(sorted(overlap))
        )
    listed = set(same) | set(unsupported)
    if listed != harness:
        missing = sorted(harness - listed)
        extra = sorted(listed - harness)
        if missing:
            errors.append(
                "catalog issue12 ids missing harness entries: " + ", ".join(missing)
            )
        if extra:
            errors.append(
                "catalog issue12 ids not in harness_ids: " + ", ".join(extra)
            )
    return errors



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
    "validation_nodes_scanned",
    "hit_test_nodes_rebuilt",
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



def is_iced_scenario_bench_report(report: Mapping[str, Any] | None) -> bool:
    return isinstance(report, Mapping) and report.get("source") == "iced-scenario-bench"



def is_gpui_scenario_bench_report(report: Mapping[str, Any] | None) -> bool:
    return isinstance(report, Mapping) and report.get("source") == "gpui-scenario-bench"



RELATIVE_CPU_LIMITS = {"p50": 1.15, "p95": 1.20, "p99": 1.25}

RELATIVE_MEMORY_LIMIT = 1.20

ISSUE12_FIXTURE_IDS = (
    "static-tree-100",
    "mutation-paint-only",
    "hover",
    "virtual-list-10k",
    "text-table",
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
