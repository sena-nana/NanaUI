#!/usr/bin/env python3
"""Validate NanaUI Runtime/Scene performance reports with stable semantic gates.

Ordinary PR CI uses work-counter cargo tests, not this timing script.
This validator is the weekly/dispatch timing and scale gate.
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import sys
import tempfile
from typing import Any


DISTRIBUTION_FIELDS = (
    "p50",
    "p95",
    "p99",
    "max",
    "frame_budget_ms",
    "frame_budget_misses",
)

# Geometric live-entity cap used by nana-framework-benchmark. Independent of
# materialized.range: two-sided overscan plus one partial item on each edge.
# 800+2×200 / 20 → 62 list rows (~60). Table adds the column window the same way.
LIST_VIEWPORT_PX = 800.0
LIST_OVERSCAN_PX = 200.0
LIST_ITEM_EXTENT_PX = 20.0
TABLE_VIEWPORT_PX = (1_280.0, 800.0)
TABLE_OVERSCAN_PX = (160.0, 200.0)
TABLE_COLUMN_EXTENT_PX = 80.0


def geometric_window_cap(viewport: float, overscan: float, item_extent: float) -> int:
    return math.ceil((viewport + 2.0 * overscan) / item_extent) + 2


def list_live_entity_bound() -> int:
    return geometric_window_cap(LIST_VIEWPORT_PX, LIST_OVERSCAN_PX, LIST_ITEM_EXTENT_PX)


def table_live_entity_bound() -> int:
    rows = geometric_window_cap(TABLE_VIEWPORT_PX[1], TABLE_OVERSCAN_PX[1], LIST_ITEM_EXTENT_PX)
    columns = geometric_window_cap(TABLE_VIEWPORT_PX[0], TABLE_OVERSCAN_PX[0], TABLE_COLUMN_EXTENT_PX)
    return rows + rows * columns


def load(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as source:
        return json.load(source)


def require(condition: bool, message: str, failures: list[str]) -> None:
    if not condition:
        failures.append(message)


def require_distribution(value: Any, label: str, failures: list[str]) -> dict[str, float] | None:
    if not isinstance(value, dict):
        failures.append(f"{label} must be a distribution object")
        return None
    for field in DISTRIBUTION_FIELDS:
        if field not in value:
            failures.append(f"{label} missing {field}")
            return None
    require(
        float(value["frame_budget_ms"]) > 0.0,
        f"{label} frame_budget_ms must be a positive documented budget",
        failures,
    )
    require(
        int(value["frame_budget_misses"]) >= 0,
        f"{label} frame_budget_misses must be a count",
        failures,
    )
    return value


def p95(distribution: dict[str, float]) -> float:
    return float(distribution["p95"])


def case_kind(case: dict[str, Any]) -> str:
    return str(case.get("kind", "full"))


def find_case(cases: list[dict[str, Any]], nodes: int, kind: str) -> dict[str, Any] | None:
    for case in cases:
        if int(case["nodes"]) == nodes and case_kind(case) == kind:
            return case
    return None


def validate_cpu(args: argparse.Namespace, failures: list[str]) -> None:
    runtime = load(args.runtime)
    framework = load(args.framework)
    vue = load(args.vue)
    scene = load(args.scene)

    runtime_cases = runtime["cases"]
    require(len(runtime_cases) >= 6, "runtime report must cover 100..10k full plus 50k construction", failures)
    require(
        find_case(runtime_cases, 5_000, "full") is not None,
        "runtime report must include a 5k full case",
        failures,
    )
    require(
        find_case(runtime_cases, 10_000, "full") is not None,
        "runtime report must include a 10k full case",
        failures,
    )
    require(
        find_case(runtime_cases, 50_000, "construction") is not None,
        "runtime report must include a 50k construction case",
        failures,
    )
    for case in runtime_cases:
        nodes = case["nodes"]
        require(
            case["local_paint_work_nodes"] == 1,
            f"runtime {nodes}: local paint must dirty exactly one node",
            failures,
        )
        require(
            case["pointer_hover_work_nodes"] <= 2,
            f"runtime {nodes}: pointer hover must dirty at most two nodes",
            failures,
        )
        require_distribution(case["enqueue_ms"], f"runtime {nodes} enqueue_ms", failures)
        require_distribution(case["local_paint_commit_ms"], f"runtime {nodes} local_paint_commit_ms", failures)
        if case_kind(case) != "full":
            continue
        local_paint = require_distribution(
            case.get("local_paint_systems_ms"),
            f"runtime {nodes} local_paint_systems_ms",
            failures,
        )
        if local_paint is not None:
            require(
                p95(local_paint) <= 0.25,
                f"runtime {nodes}: local paint systems P95 exceeds 0.25 ms",
                failures,
            )

    runtime_5k = find_case(runtime_cases, 5_000, "full")
    if runtime_5k is not None:
        initial = require_distribution(
            runtime_5k.get("initial_systems_ms"),
            "runtime 5000 initial_systems_ms",
            failures,
        )
        if initial is not None:
            # In-force gate: P95 <= 8 ms. Phase 3 baseline 2.42 ms; re-measured
            # after idle ancestor-walk skip (release, Apple M4): p95=5.343.
            require(
                p95(initial) <= 8.0,
                "runtime 5000-node initial systems P95 exceeds 8 ms",
                failures,
            )

    runtime_10k = find_case(runtime_cases, 10_000, "full")
    if runtime_10k is not None:
        initial_10k = require_distribution(
            runtime_10k.get("initial_systems_ms"),
            "runtime 10000 initial_systems_ms",
            failures,
        )
        if initial_10k is not None:
            # Coverage only: 10k full systems is a new scale. The in-force
            # timing gate remains 5k initial_systems P95 <= 8 ms (phase3
            # baseline 2.42 ms). Do not invent a 10k P95 ceiling here.
            _ = p95(initial_10k)

    list_10k = require_distribution(
        framework["virtual_list_10k_materialize_ms"],
        "framework virtual_list_10k_materialize_ms",
        failures,
    )
    if list_10k is not None:
        require(
            p95(list_10k) <= 1.0,
            "10k virtual-list materialization P95 exceeds 1 ms",
            failures,
        )
    scroll = require_distribution(
        framework["virtual_scroll_40_visible_nodes_ms"],
        "framework virtual_scroll_40_visible_nodes_ms",
        failures,
    )
    if scroll is not None:
        require(
            p95(scroll) <= 0.25,
            "40-node virtual scroll P95 exceeds 0.25 ms",
            failures,
        )
    layout = require_distribution(
        framework["canonical_layout_5000_nodes_ms"],
        "framework canonical_layout_5000_nodes_ms",
        failures,
    )
    if layout is not None:
        require(
            p95(layout) <= 8.0,
            "canonical Runtime 5000-node layout P95 exceeds 8 ms",
            failures,
        )
    validate_virtual_scales(framework, failures)

    vue_cases = vue["cases"]
    require(
        find_case(vue_cases, 5_000, "full") is not None,
        "vue report must include a 5k full case",
        failures,
    )
    require(
        find_case(vue_cases, 10_000, "full") is not None,
        "vue report must include a 10k full case",
        failures,
    )
    require(
        find_case(vue_cases, 50_000, "construction") is not None,
        "vue report must include a 50k construction case",
        failures,
    )
    vue_5k = find_case(vue_cases, 5_000, "full")
    if vue_5k is not None:
        idle = require_distribution(vue_5k.get("idle_semantic_ms"), "vue 5000 idle_semantic_ms", failures)
        construction = require_distribution(
            vue_5k.get("construction_ms"),
            "vue 5000 construction_ms",
            failures,
        )
        if idle is not None:
            require(
                p95(idle) <= 0.01,
                "Vue 5000-node idle semantic P95 exceeds 0.01 ms",
                failures,
            )
        if construction is not None:
            require(
                p95(construction) <= 40.0,
                "Vue 5000-node construction P95 exceeds 40 ms",
                failures,
            )

    scene_rows = scene["rows"]
    require(
        find_case(scene_rows, 5_000, "full") is not None,
        "scene report must include a 5k full case",
        failures,
    )
    require(
        find_case(scene_rows, 10_000, "full") is not None,
        "scene report must include a 10k full case",
        failures,
    )
    require(
        find_case(scene_rows, 50_000, "construction") is not None,
        "scene report must include a 50k construction case",
        failures,
    )
    scene_5k = find_case(scene_rows, 5_000, "full")
    if scene_5k is not None:
        local = require_distribution(scene_5k.get("local_update_ms"), "scene 5000 local_update_ms", failures)
        idle_scene = require_distribution(scene_5k.get("idle_update_ms"), "scene 5000 idle_update_ms", failures)
        graph = require_distribution(scene_5k.get("frame_graph_ms"), "scene 5000 frame_graph_ms", failures)
        if local is not None:
            require(
                p95(local) <= 0.25,
                "Scene 5000-node local update P95 exceeds 0.25 ms",
                failures,
            )
        if idle_scene is not None:
            require(
                p95(idle_scene) <= 0.05,
                "Scene 5000-node idle update P95 exceeds 0.05 ms",
                failures,
            )
        if graph is not None:
            require(
                p95(graph) <= 3.0,
                "Scene 5000-node frame-graph P95 exceeds 3 ms",
                failures,
            )


def validate_virtual_scales(framework: dict[str, Any], failures: list[str]) -> None:
    tree = framework.get("virtual_tree")
    require(isinstance(tree, dict), "framework report must document the virtual-tree gap", failures)
    if isinstance(tree, dict):
        require(
            tree.get("status") == "skipped",
            "virtual tree has no scale bench; report it as skipped rather than a fake timing",
            failures,
        )

    scales = framework.get("virtual_scales")
    require(isinstance(scales, list) and scales, "framework report must include virtual_scales", failures)
    if not isinstance(scales, list):
        return

    def has_scale(kind: str, rows: int, status: str | None = None) -> bool:
        for scale in scales:
            if scale.get("kind") != kind or int(scale.get("logical_rows", 0)) != rows:
                continue
            if status is None or scale.get("status") == status:
                return True
        return False

    require(has_scale("list", 10_000, "ok"), "virtual_scales must include a 10k list", failures)
    require(has_scale("table", 10_000, "ok"), "virtual_scales must include a 10k table", failures)
    require(has_scale("list", 100_000, "ok"), "virtual_scales must include a measured 100k list", failures)
    require(has_scale("table", 100_000, "ok"), "virtual_scales must include a measured 100k table", failures)
    require(
        has_scale("list", 1_000_000),
        "virtual_scales must include a 1M list case (ok or skipped via NANA_PERF_SCALE=large)",
        failures,
    )
    require(
        has_scale("table", 1_000_000),
        "virtual_scales must include a 1M table case (ok or skipped via NANA_PERF_SCALE=large)",
        failures,
    )

    list_bound = list_live_entity_bound()
    table_bound = table_live_entity_bound()
    for scale in scales:
        label = f"{scale.get('kind')} {scale.get('logical_rows')}"
        if scale.get("status") == "skipped":
            # 1M may skip on public runners or without NANA_PERF_SCALE=large.
            # Do not invent a timing threshold for a skipped case.
            continue
        live = scale.get("live_ui_entities")
        reported_bound = scale.get("live_ui_entities_bound")
        require(live is not None and reported_bound is not None, f"{label}: live entity counts missing", failures)
        if live is None or reported_bound is None:
            continue
        geometric = list_bound if scale.get("kind") == "list" else table_bound
        require(
            int(reported_bound) == geometric,
            f"{label}: live_ui_entities_bound {reported_bound} must be the geometric cap {geometric}, not materialized.range",
            failures,
        )
        require(
            int(live) <= geometric,
            f"{label}: live_ui_entities {live} exceeds geometric bound {geometric}",
            failures,
        )
        materialize = scale.get("materialize_ms")
        if materialize is None:
            continue
        distribution = require_distribution(materialize, f"{label} materialize_ms", failures)
        if distribution is None:
            continue
        rows = int(scale.get("logical_rows", 0))
        kind = scale.get("kind")
        if kind == "list" and rows == 10_000:
            require(
                p95(distribution) <= 1.0,
                f"{label}: materialize P95 exceeds 1 ms",
                failures,
            )
        elif kind == "list" and rows == 100_000:
            # Same 1 ms 10k list gate: virtualization must stay size-independent.
            # Phase 4 10k list materialize p95 was 0.043 ms.
            require(
                p95(distribution) <= 1.0,
                f"{label}: materialize P95 exceeds 1 ms",
                failures,
            )
        elif kind == "table" and rows == 100_000:
            # Evidence from this workstream's 100k table bench (not a 1M guess).
            require(
                p95(distribution) <= 40.0,
                f"{label}: materialize P95 exceeds 40 ms",
                failures,
            )
        elif rows == 1_000_000:
            # Coverage only: 1M stays skip-or-measure with null timings allowed.
            # Do not invent a 1M P95 ceiling.
            _ = p95(distribution)


def validate_gpu(path: Path, failures: list[str]) -> None:
    report = load(path)
    require(report["platform"] == "macos", "GPU report must be macOS", failures)
    require(
        report["screenshot_distinct_colors"] > 32,
        "composed screenshot lacks rendered detail",
        failures,
    )
    for name in ("ui_only", "live2d_only", "ui_live2d_composed"):
        distribution = report.get(name)
        require(isinstance(distribution, dict), f"GPU {name} missing", failures)
        if not isinstance(distribution, dict):
            continue
        for field in ("p50", "p95", "p99", "max"):
            require(field in distribution, f"GPU {name} missing {field}", failures)
        require("frame_budget_ms" in distribution, f"GPU {name} missing frame_budget_ms", failures)
        require(
            "frame_budget_misses" in distribution,
            f"GPU {name} missing frame_budget_misses",
            failures,
        )
    require(
        report["ui_only"]["p95"]["total_ms"] <= 12.0,
        "UI-only total P95 exceeds 12 ms",
        failures,
    )
    require(
        report["live2d_only"]["p95"]["total_ms"] <= 12.0,
        "Live2D-only total P95 exceeds 12 ms",
        failures,
    )
    require(
        report["ui_live2d_composed"]["p95"]["total_ms"] <= 16.67,
        "UI + Live2D composed total P95 exceeds one 60 Hz frame",
        failures,
    )


def _distribution(p95_ms: float = 0.01, budget_ms: float = 16.67) -> dict[str, float]:
    return {
        "p50": p95_ms / 2,
        "p95": p95_ms,
        "p99": p95_ms,
        "max": p95_ms,
        "frame_budget_ms": budget_ms,
        "frame_budget_misses": 0,
    }


def _ok_scale(kind: str, rows: int, columns: int | None = None) -> dict[str, Any]:
    bound = list_live_entity_bound() if kind == "list" else table_live_entity_bound()
    live = 57 if kind == "list" else 1_140
    payload = {
        "kind": kind,
        "logical_rows": rows,
        "status": "ok",
        "visible_rows": 40,
        "overscan_rows": 17,
        "cache_rows": 0,
        "live_ui_entities": live,
        "live_ui_entities_bound": bound,
        "materialize_ms": _distribution(0.05 if kind == "list" else 8.0),
    }
    if columns is not None:
        payload["logical_columns"] = columns
    return payload


def _cpu_args(runtime: dict[str, Any], framework: dict[str, Any], vue: dict[str, Any], scene: dict[str, Any]) -> argparse.Namespace:
    directory = Path(tempfile.mkdtemp())
    paths = {
        "runtime": directory / "runtime.json",
        "framework": directory / "framework.json",
        "vue": directory / "vue.json",
        "scene": directory / "scene.json",
    }
    for name, payload in (
        ("runtime", runtime),
        ("framework", framework),
        ("vue", vue),
        ("scene", scene),
    ):
        paths[name].write_text(json.dumps(payload), encoding="utf-8")
    return argparse.Namespace(
        runtime=paths["runtime"],
        framework=paths["framework"],
        vue=paths["vue"],
        scene=paths["scene"],
        gpu=None,
    )


def _sample_reports() -> tuple[dict[str, Any], dict[str, Any], dict[str, Any], dict[str, Any]]:
    runtime = {
        "cases": [
            {
                "nodes": nodes,
                "kind": "full",
                "local_paint_work_nodes": 1,
                "pointer_hover_work_nodes": 2,
                "enqueue_ms": _distribution(),
                "local_paint_commit_ms": _distribution(),
                "local_paint_systems_ms": _distribution(0.1),
                "initial_systems_ms": _distribution(4.0 if nodes == 5_000 else 0.5),
            }
            for nodes in (100, 500, 1_000, 5_000, 10_000)
        ]
        + [
            {
                "nodes": 50_000,
                "kind": "construction",
                "local_paint_work_nodes": 1,
                "pointer_hover_work_nodes": 2,
                "enqueue_ms": _distribution(2.0),
                "local_paint_commit_ms": _distribution(),
            }
        ]
    }
    framework = {
        "virtual_list_10k_materialize_ms": _distribution(0.05),
        "virtual_scroll_40_visible_nodes_ms": _distribution(0.01),
        "canonical_layout_5000_nodes_ms": _distribution(2.0),
        "virtual_tree": {"status": "skipped", "reason": "no scale bench this round"},
        "virtual_scales": [
            _ok_scale("list", 10_000),
            _ok_scale("table", 10_000, 100),
            _ok_scale("list", 100_000),
            _ok_scale("table", 100_000, 100),
            {
                "kind": "list",
                "logical_rows": 1_000_000,
                "status": "skipped",
                "skip_reason": "NANA_PERF_SCALE!=large",
            },
            {
                "kind": "table",
                "logical_rows": 1_000_000,
                "status": "skipped",
                "skip_reason": "NANA_PERF_SCALE!=large",
            },
        ],
    }
    vue = {
        "cases": [
            {
                "nodes": nodes,
                "kind": "full",
                "construction_ms": _distribution(10.0 if nodes == 5_000 else 1.0),
                "idle_semantic_ms": _distribution(0.0),
            }
            for nodes in (100, 500, 1_000, 5_000, 10_000)
        ]
        + [{"nodes": 50_000, "kind": "construction", "construction_ms": _distribution(20.0)}]
    }
    scene = {
        "rows": [
            {
                "nodes": nodes,
                "kind": "full",
                "local_update_ms": _distribution(0.01),
                "idle_update_ms": _distribution(0.001),
                "frame_graph_ms": _distribution(0.5),
            }
            for nodes in (100, 500, 1_000, 5_000, 10_000)
        ]
        + [{"nodes": 50_000, "kind": "construction"}]
    }
    return runtime, framework, vue, scene


def self_test() -> int:
    failures: list[str] = []
    runtime, framework, vue, scene = _sample_reports()
    validate_cpu(_cpu_args(runtime, framework, vue, scene), failures)

    missing_5k = dict(runtime)
    missing_5k["cases"] = [case for case in runtime["cases"] if int(case["nodes"]) != 5_000]
    missing_5k_failures: list[str] = []
    validate_cpu(_cpu_args(missing_5k, framework, vue, scene), missing_5k_failures)
    require(
        any("5k full case" in failure for failure in missing_5k_failures),
        "self-test must reject a runtime report that omits the 5k full case",
        failures,
    )

    slow_5k = dict(runtime)
    slow_5k["cases"] = [dict(case) for case in runtime["cases"]]
    for case in slow_5k["cases"]:
        if int(case["nodes"]) == 5_000:
            case["initial_systems_ms"] = _distribution(9.0)
    slow_failures: list[str] = []
    validate_cpu(_cpu_args(slow_5k, framework, vue, scene), slow_failures)
    require(
        any("exceeds 8 ms" in failure for failure in slow_failures),
        "self-test must reject 5k initial_systems p95 above the in-force 8 ms gate",
        failures,
    )

    no_tree = dict(framework)
    no_tree.pop("virtual_tree", None)
    tree_failures: list[str] = []
    validate_cpu(_cpu_args(runtime, no_tree, vue, scene), tree_failures)
    require(
        any("virtual-tree" in failure for failure in tree_failures),
        "self-test must reject a report that omits virtual_tree",
        failures,
    )

    over_bound = dict(framework)
    over_bound["virtual_scales"] = [dict(scale) for scale in framework["virtual_scales"]]
    for scale in over_bound["virtual_scales"]:
        if scale.get("kind") == "list" and int(scale.get("logical_rows", 0)) == 100_000:
            scale["live_ui_entities"] = list_live_entity_bound() + 1
    bound_failures: list[str] = []
    validate_cpu(_cpu_args(runtime, over_bound, vue, scene), bound_failures)
    require(
        any("geometric bound" in failure for failure in bound_failures),
        "self-test must reject live_ui_entities above the geometric bound",
        failures,
    )

    if failures:
        print("Runtime performance validator self-test failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("Runtime performance validator self-test: OK")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--framework", type=Path)
    parser.add_argument("--vue", type=Path)
    parser.add_argument("--scene", type=Path)
    parser.add_argument("--gpu", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    failures: list[str] = []
    cpu_paths = [args.runtime, args.framework, args.vue, args.scene]
    if any(cpu_paths):
        if not all(cpu_paths):
            parser.error("CPU validation requires runtime, framework, vue, and scene reports")
        validate_cpu(args, failures)
    if args.gpu:
        validate_gpu(args.gpu, failures)
    if not any(cpu_paths) and not args.gpu:
        parser.error("provide CPU reports, --gpu, or --self-test")
    if failures:
        print("Runtime performance gate failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("Runtime performance gate: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
