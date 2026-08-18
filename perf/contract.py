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
    if kind == "StaticTree" and not _positive_int(params.get("nodes")):
        errors.append("StaticTree.nodes must be a positive integer")
    if kind == "VirtualList":
        for key in ("items", "visible", "overscan"):
            if not _positive_int(params.get(key)) and not (
                key == "overscan" and params.get(key) == 0
            ):
                errors.append(f"VirtualList.{key} must be a non-negative integer")
    return errors


def _positive_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


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
    "accessibility_nodes_updated",
    "render_nodes_extracted",
    "extracted_text_spans",
)


def counters_from_block(block: Any) -> dict[str, Any]:
    """Copy measured WorkCounters fields. Nulls and unknown GPU-byte keys stay out."""
    if not isinstance(block, dict):
        return {}
    return {key: block[key] for key in WORK_COUNTER_KEYS if key in block}


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
    if kind == "Hover":
        return _extract_nana_hover(scenario, reports, source_paths)
    if kind == "VirtualList":
        return _extract_nana_virtual_list(scenario, reports, source_paths)
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
    scenario_id = "virtual-list-10k" if items == 10_000 else f"virtual-list-{_scale_token(items)}"
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


def extract_iced(
    scenario: Mapping[str, Any],
    report: Mapping[str, Any],
    *,
    source_path: Path,
) -> dict[str, Any]:
    kind = scenario["kind"]
    cases = {case.get("scenario"): case for case in report.get("cases", [])}
    if kind == "StaticTree":
        nodes = scenario["params"]["nodes"]
        name = {100: "list-100", 1000: "list-1000"}.get(nodes)
        if name is None or name not in cases:
            raise KeyError(
                f"ui-benchmark has no list case for StaticTree nodes={nodes}; "
                "5k/10k/50k are unsupported on this legacy path"
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
                "docs/performance-baseline.md were taken on the Iced Gallery path. A dedicated "
                "engine/iced Scenario adapter is required by #8 / not implemented.",
            ],
            metrics={
                "cpu_frame_ms": percentile_fields(case.get("cpu_total_ms")),
                "total_ms": percentile_fields(case.get("total_ms")),
                "view_construction_ms": percentile_fields(case.get("view_construction_ms")),
                "layout_diff_ms": percentile_fields(case.get("layout_diff_ms")),
            },
        )
    if kind == "Hover":
        raise KeyError(
            "ui-benchmark has no dedicated same-Scenario hover case. "
            "Gallery list-100 event_update_ms is pointer/press/wheel on a full-layout list "
            "and is not hover.json (nodes=10000, layout_nodes==0)."
        )
    if kind == "VirtualList":
        raise KeyError(
            "ui-benchmark lists are full-layout Gallery cases (list-100/list-1000), not "
            "VirtualList {items, visible, overscan}. Mapping list-1000 onto virtual-list-10k "
            "is forbidden; a dedicated engine/iced Scenario adapter is required by #8 / not implemented."
        )
    raise KeyError(f"no Iced mapping for {scenario['id']}")


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
    paint_ok_inv = _named_invariant(paint_measured, "paint_only_does_not_layout_full_tree")
    if (
        paint_ok_inv is None
        or paint_ok_inv.get("status") != "ok"
        or paint_ok_inv.get("measured") != 0
    ):
        errors.append("paint invariant must be ok with measured layout_nodes=0")

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
    return errors


def _self_test_from_report_cli(root: Path) -> list[str]:
    """Exercise --from-report through the Nana runner, not only extract_nana."""
    errors: list[str] = []
    script = root / "perf" / "runners" / "nana" / "run.py"
    fixture = root / "perf" / "fixtures" / "virtual-scales-only.json"
    result = subprocess.run(
        [
            sys.executable,
            str(script),
            "--repo-root",
            str(root),
            "--scenario",
            "virtual-list-100k",
            "--from-report",
            str(fixture),
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != EXIT_OK:
        errors.append(
            f"virtual_scales-only --from-report exit {result.returncode}: {result.stderr}"
        )
        return errors
    try:
        report = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        errors.append(f"virtual_scales-only --from-report stdout is not JSON: {exc}")
        return errors
    if report.get("status") != "ok":
        errors.append(f"virtual_scales-only --from-report status={report.get('status')}")
    if (report.get("work_counters") or {}).get("live_ui_entities") != 50:
        errors.append("virtual_scales-only --from-report must copy live_ui_entities=50")
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
