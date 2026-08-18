#!/usr/bin/env python3
"""Shared Issue #8 Scenario schema helpers and runner report envelope.

Runners must load scenarios from ``perf/scenarios/*.json``. They must not invent
a private tree size, font, DPI, or interaction script. Relative Iced/GPUI
timing gates are not enforceable until those runners produce real numbers.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import subprocess
import sys
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
        notes.append(
            "Existing overscan is reported in rows from the binary, not the contract overscan=8 items."
        )
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
        equivalence="closest-legacy-reference",
        source_binary=source_binary,
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
        + " stay omitted — Runtime glyph cache is None; do not invent numbers.",
    ]
    metrics: dict[str, Any] = {}
    work_counters: dict[str, Any] = {}
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
        notes.append(
            "Existing overscan is reported in rows from the binary, not contract overscan_rows=8. "
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
        equivalence="closest-legacy-reference",
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
    if composition != "UiOnly":
        raise KeyError(
            f"GpuScene composition {composition} has no Nana encode/submit path. "
            "Live2D is not a Scene pass; HostTexture evidence is not this composition. "
            "Required by #8 / not implemented."
        )
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
        "Hover",
        "VirtualList",
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
    if scenario["kind"] != "StaticTree":
        raise KeyError(
            "iced-scenario-bench currently implements StaticTree only; "
            f"no same-scenario mapping for {scenario['id']}"
        )
    nodes = scenario["params"]["nodes"]
    reported = report.get("nodes")
    if reported != nodes:
        raise KeyError(
            f"iced-scenario-bench nodes={reported} does not match StaticTree nodes={nodes}"
        )
    reported_id = report.get("scenario_id")
    if reported_id not in (None, scenario["id"]):
        raise KeyError(
            f"iced-scenario-bench scenario_id={reported_id!r} does not match {scenario['id']!r}"
        )
    cpu = percentile_fields(report.get("cpu_frame_ms"))
    if cpu is None:
        raise KeyError(
            "iced-scenario-bench ok report missing cpu_frame_ms percentiles; "
            "fake or empty timings are forbidden"
        )
    tree = report.get("tree")
    if not is_shared_static_tree(tree if isinstance(tree, Mapping) else None, nodes):
        raise KeyError(
            "iced-scenario-bench ok report is not the shared StaticTree heap "
            "(generation=complete-binary-heap, parent(i)=i//2, element-div, no text). "
            "A column of N text leaves is not same-scenario."
        )
    notes = [str(note) for note in (report.get("notes") or []) if note is not None]
    notes.append(
        "Mapped onto engine/iced static_tree / static_tree_parent, the same "
        "complete-binary-heap rule as nana-runtime-benchmark::tree_mutations."
    )
    notes.append(
        "Relative Iced/GPUI gates stay off until GPUI also emits same-scenario ok "
        "with real metrics on this id."
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
        metrics={
            "cpu_frame_ms": cpu,
            "view_construction_ms": percentile_fields(report.get("view_construction_ms")),
            "layout_ms": percentile_fields(report.get("layout_ms")),
            "draw_ms": percentile_fields(report.get("draw_ms")),
            "present_ms": percentile_fields(report.get("present_ms")),
        },
    )


def _scale_token(nodes: int) -> str:
    if nodes >= 1000 and nodes % 1000 == 0:
        return f"{nodes // 1000}k"
    return str(nodes)


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
        extract_nana(
            static_tree,
            {
                "runtime": load_json(runtime_path),
                "scene": load_json(scene_path),
            },
            source_paths={"runtime": runtime_path, "scene": scene_path},
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

    paint = load_scenario("mutation-paint-only", root)
    hover = load_scenario("hover", root)

    def _named_invariant(report: Mapping[str, Any], name: str) -> dict[str, Any] | None:
        for item in report.get("invariants") or []:
            if item.get("name") == name:
                return item
        return None

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
                        "overscan_rows": 10,
                        "live_ui_entities": 50,
                        "live_ui_entities_bound": 1260,
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

    try:
        extract_iced(text_table, load_json(iced_path), source_path=iced_path)
        errors.append("iced text-table extract must be unsupported")
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
    if (iced_same.get("metrics") or {}).get("cpu_frame_ms", {}).get("p50") != 1.4:
        errors.append("iced same-scenario extract must copy fixture cpu_frame_ms.p50")
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
        errors.append("iced-scenario-bench hover extract must be unsupported")
    except KeyError:
        pass
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
    return errors


def _self_test_catalog_workloads(
    root: Path,
    runtime_path: Path,
    iced_path: Path,
    reserved: set[Any],
) -> list[str]:
    errors: list[str] = []
    harness = set(load_catalog(root).get("harness_ids", []))
    wirable = ("animation", "ime", "dock-workspace", "overlay", "text-editor")
    for scenario_id in wirable:
        if scenario_id not in harness:
            errors.append(f"catalog must list wirable {scenario_id} in harness_ids")
        if scenario_id in reserved:
            errors.append(
                f"catalog must not leave wirable {scenario_id} in required_by_issue_not_in_harness"
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

    code, report, err = from_report(
        nana_script,
        "virtual-list-100k",
        root / "perf" / "fixtures" / "virtual-scales-only.json",
    )
    if code != EXIT_OK or report is None:
        errors.append(f"virtual_scales-only --from-report exit {code}: {err}")
    elif report.get("status") != "ok":
        errors.append(f"virtual_scales-only --from-report status={report.get('status')}")
    elif (report.get("work_counters") or {}).get("live_ui_entities") != 50:
        errors.append("virtual_scales-only --from-report must copy live_ui_entities=50")

    code, table_report, err = from_report(
        nana_script,
        "text-table",
        root / "perf" / "fixtures" / "virtual-table-scales.json",
    )
    if code != EXIT_OK or table_report is None:
        errors.append(f"virtual-table-scales --from-report exit {code}: {err}")
    else:
        table_from_report = table_report.get("work_counters") or {}
        if table_report.get("status") != "ok":
            errors.append(f"virtual-table-scales --from-report status={table_report.get('status')}")
        if table_from_report.get("live_ui_entities") != 50:
            errors.append("virtual-table-scales --from-report must copy live_ui_entities=50")
        if table_from_report.get("text_shaped") != 12:
            errors.append("virtual-table-scales --from-report must copy text_shaped=12")
        if table_from_report.get("text_wrap_layouts") != 4:
            errors.append("virtual-table-scales --from-report must copy text_wrap_layouts=4")
        if table_from_report.get("text_shaped_runs") != 8:
            errors.append("virtual-table-scales --from-report must copy text_shaped_runs=8")
        if table_from_report.get("text_layout_cache_hits") != 3:
            errors.append("virtual-table-scales --from-report must copy text_layout_cache_hits=3")
        if table_from_report.get("text_layout_cache_misses") != 5:
            errors.append("virtual-table-scales --from-report must copy text_layout_cache_misses=5")
        if table_from_report.get("cache_eviction") != 0:
            errors.append("virtual-table-scales --from-report must copy cache_eviction=0")
        for key in TEXT_TABLE_EXPORTED_SHAPE_KEYS:
            if key not in table_from_report:
                errors.append(f"virtual-table-scales --from-report must contain {key}")
        if table_from_report.get("wrapped_cells") != 4:
            errors.append("virtual-table-scales --from-report must copy catalog wrapped_cells=4")

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

    hover_iced = subprocess.run(
        [
            sys.executable,
            str(iced_script),
            "--repo-root",
            str(root),
            "--scenario",
            "hover",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if hover_iced.returncode != EXIT_UNSUPPORTED:
        errors.append(f"iced hover must exit 2, got {hover_iced.returncode}: {hover_iced.stderr}")

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
    parser.add_argument("--repo-root", type=Path, default=REPO_ROOT)
    args = parser.parse_args(argv)
    root = args.repo_root.resolve()
    if args.self_test:
        errors = self_test(root)
        if errors:
            for error in errors:
                print(error, file=sys.stderr)
            return EXIT_ERROR
        print("perf contract self-test: OK")
        return EXIT_OK
    if args.check_schema:
        errors = validate_all_scenarios(root)
        if errors:
            for error in errors:
                print(error, file=sys.stderr)
            return EXIT_ERROR
        print("scenario schema: OK")
        return EXIT_OK
    parser.error("provide --check-schema or --self-test")
    return EXIT_ERROR


if __name__ == "__main__":
    raise SystemExit(main())
