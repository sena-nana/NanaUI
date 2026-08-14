#!/usr/bin/env python3
"""Validate NanaUI Runtime/Scene performance reports with stable semantic gates."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys


def load(path: Path) -> dict[str, object]:
    with path.open(encoding="utf-8") as source:
        return json.load(source)


def require(condition: bool, message: str, failures: list[str]) -> None:
    if not condition:
        failures.append(message)


def p95(distribution: dict[str, float]) -> float:
    return float(distribution["p95"])


def validate_cpu(args: argparse.Namespace, failures: list[str]) -> None:
    runtime = load(args.runtime)
    framework = load(args.framework)
    vue = load(args.vue)
    scene = load(args.scene)

    runtime_cases = runtime["cases"]
    require(len(runtime_cases) >= 4, "runtime report must cover four node scales", failures)
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
        require(
            p95(case["local_paint_systems_ms"]) <= 0.25,
            f"runtime {nodes}: local paint systems P95 exceeds 0.25 ms",
            failures,
        )
    largest_runtime = max(runtime_cases, key=lambda case: case["nodes"])
    require(
        p95(largest_runtime["initial_systems_ms"]) <= 8.0,
        "runtime 5000-node initial systems P95 exceeds 8 ms",
        failures,
    )

    require(
        p95(framework["virtual_list_10k_materialize_ms"]) <= 1.0,
        "10k virtual-list materialization P95 exceeds 1 ms",
        failures,
    )
    require(
        p95(framework["virtual_scroll_40_visible_nodes_ms"]) <= 0.25,
        "40-node virtual scroll P95 exceeds 0.25 ms",
        failures,
    )
    require(
        p95(framework["canonical_layout_5000_nodes_ms"]) <= 8.0,
        "canonical Runtime 5000-node layout P95 exceeds 8 ms",
        failures,
    )

    vue_cases = vue["cases"]
    largest_vue = max(vue_cases, key=lambda case: case["nodes"])
    require(
        p95(largest_vue["idle_semantic_ms"]) <= 0.01,
        "Vue 5000-node idle semantic P95 exceeds 0.01 ms",
        failures,
    )
    require(
        p95(largest_vue["construction_ms"]) <= 40.0,
        "Vue 5000-node construction P95 exceeds 40 ms",
        failures,
    )

    scene_rows = scene["rows"]
    largest_scene = max(scene_rows, key=lambda row: row["nodes"])
    require(
        largest_scene["local_update_p95_ms"] <= 0.25,
        "Scene 5000-node local update P95 exceeds 0.25 ms",
        failures,
    )
    require(
        largest_scene["idle_update_p95_ms"] <= 0.05,
        "Scene 5000-node idle update P95 exceeds 0.05 ms",
        failures,
    )
    require(
        largest_scene["frame_graph_p95_ms"] <= 3.0,
        "Scene 5000-node frame-graph P95 exceeds 3 ms",
        failures,
    )


def validate_gpu(path: Path, failures: list[str]) -> None:
    report = load(path)
    require(report["platform"] == "macos", "GPU report must be macOS", failures)
    require(
        report["screenshot_distinct_colors"] > 32,
        "composed screenshot lacks rendered detail",
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--framework", type=Path)
    parser.add_argument("--vue", type=Path)
    parser.add_argument("--scene", type=Path)
    parser.add_argument("--gpu", type=Path)
    args = parser.parse_args()
    failures: list[str] = []
    cpu_paths = [args.runtime, args.framework, args.vue, args.scene]
    if any(cpu_paths):
        if not all(cpu_paths):
            parser.error("CPU validation requires runtime, framework, vue, and scene reports")
        validate_cpu(args, failures)
    if args.gpu:
        validate_gpu(args.gpu, failures)
    if not any(cpu_paths) and not args.gpu:
        parser.error("provide CPU reports or --gpu")
    if failures:
        print("Runtime performance gate failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("Runtime performance gate: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
