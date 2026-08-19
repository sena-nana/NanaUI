#!/usr/bin/env python3
"""Map Issue #8 Scenarios onto existing NanaUI benchmark binaries.

Does not reimplement Runtime. Invokes:

- nana-runtime-benchmark (StaticTree complete-binary-heap via tree_mutations, Mutation including remaining §3.2 kinds, Hover, catalog_animation)
- nana-framework-benchmark (VirtualList, VirtualTree, Table / text-table, Ime, DockWorkspace, Overlay, TextEditor)
- nana-scene-benchmark (optional StaticTree scene rows)
- nana-gpu-scene-benchmark (gpu-scene-ui from perf/scenarios/gpu-scene-ui.json; UiOnly UI + HostTexture)

Relative Iced/GPUI gates are not applied here.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path
from typing import Any

PERF = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PERF))

import contract  # noqa: E402


RUNTIME_BIN = {
    "package": "nana-ui-runtime",
    "binary": "nana-runtime-benchmark",
    "features": "benchmark",
    "key": "runtime",
}
FRAMEWORK_BIN = {
    "package": "nana-ui-runtime",
    "binary": "nana-framework-benchmark",
    "features": "benchmark",
    "key": "framework",
}
SCENE_BIN = {
    "package": "nana-ui-scene",
    "binary": "nana-scene-benchmark",
    "features": "benchmark",
    "key": "scene",
}
GPU_SCENE_BIN = {
    "package": "nana-ui",
    "binary": "nana-gpu-scene-benchmark",
    "features": "gpu",
    "key": "gpu",
}


def _needed_bins(scenario: dict[str, Any]) -> list[dict[str, str]]:
    kind = scenario["kind"]
    if kind == "StaticTree":
        return [RUNTIME_BIN, SCENE_BIN]
    if kind in {"Mutation", "Hover"}:
        return [RUNTIME_BIN]
    if kind in {"VirtualList", "Table", "Ime", "DockWorkspace", "Overlay", "TextEditor", "VirtualTree"}:
        return [FRAMEWORK_BIN]
    if kind == "Animation":
        return [RUNTIME_BIN]
    if kind == "GpuScene":
        if scenario.get("params", {}).get("composition") == "UiOnly":
            return [GPU_SCENE_BIN]
        return []
    return []


def plan(scenario_id: str, args: Any) -> list[str]:
    try:
        scenario = contract.load_scenario(scenario_id, args.repo_root)
    except FileNotFoundError:
        return [f"# missing scenario file for {scenario_id}"]
    if contract.is_incomparable_static_tree_50k(scenario):
        return [
            f"# nana unsupported for {scenario_id}; exit {contract.EXIT_UNSUPPORTED}",
            f"# {contract.INCOMPARABLE_STATIC_TREE_50K_REASON}",
        ]
    gpu_skip = contract.nana_gpu_scene_skip_reason(scenario)
    if gpu_skip:
        return [
            f"# nana unsupported for {scenario_id}; exit {contract.EXIT_UNSUPPORTED}",
            f"# {gpu_skip}",
        ]
    bins = _needed_bins(scenario)
    if not bins:
        return [f"# no Nana binary mapping for {scenario_id}"]
    if args.from_report:
        return [f"# --from-report {args.from_report} ({scenario_id})"]
    lines = []
    work = args.repo_root / "target" / "performance" / "issue8"
    for spec in bins:
        output = work / f"{spec['key']}.json"
        extra = _extra_args(scenario, spec, args.repo_root)
        command = contract.cargo_run(
            args.repo_root,
            package=spec["package"],
            binary=spec["binary"],
            features=spec["features"],
            output=output,
            extra_args=extra,
        )
        lines.append(" ".join(command))
    return lines


def execute(scenario_id: str, args: Any) -> dict[str, Any]:
    try:
        scenario = contract.load_scenario(scenario_id, args.repo_root)
    except FileNotFoundError:
        return contract.envelope(
            runner="nana",
            status="unsupported",
            scenario_id=scenario_id,
            unsupported_reason=f"No scenario JSON at perf/scenarios/{scenario_id}.json",
        )
    if contract.is_incomparable_static_tree_50k(scenario):
        return contract.envelope(
            runner="nana",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=contract.INCOMPARABLE_STATIC_TREE_50K_REASON,
        )
    gpu_skip = contract.nana_gpu_scene_skip_reason(scenario)
    if gpu_skip:
        return contract.envelope(
            runner="nana",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=gpu_skip,
        )
    needed = _needed_bins(scenario)
    if not needed:
        return contract.envelope(
            runner="nana",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=(
                f"No existing nana-*-benchmark mapping for kind={scenario['kind']}. "
                "Required by #8 / not implemented as a dedicated Scenario binary."
            ),
        )

    reports: dict[str, dict[str, Any]] = {}
    paths: dict[str, Path] = {}
    commands: list[str] = []
    if args.from_report:
        payload = contract.load_json(args.from_report)
        key = _guess_report_key(payload)
        reports[key] = payload
        paths[key] = args.from_report
    else:
        work = args.repo_root / "target" / "performance" / "issue8"
        work.mkdir(parents=True, exist_ok=True)
        cache: dict[str, dict[str, Any]] = getattr(args, "_nana_cache", {})
        setattr(args, "_nana_cache", cache)
        for spec in needed:
            output = work / f"{spec['key']}.json"
            extra = _extra_args(scenario, spec, args.repo_root)
            command = contract.cargo_run(
                args.repo_root,
                package=spec["package"],
                binary=spec["binary"],
                features=spec["features"],
                output=output,
                extra_args=extra,
            )
            commands.append(" ".join(command))
            if spec["key"] not in cache:
                try:
                    contract.run_command(command, args.repo_root)
                except subprocess.CalledProcessError as exc:
                    if spec["key"] == "gpu" and exc.returncode == 2 and output.is_file():
                        payload = contract.load_json(output)
                        if payload.get("status") == "unsupported":
                            return contract.envelope(
                                runner="nana",
                                status="unsupported",
                                scenario_id=scenario_id,
                                scenario=scenario,
                                command=commands,
                                unsupported_reason=payload.get("unsupported_reason")
                                or "nana-gpu-scene-benchmark has no WGPU adapter",
                            )
                    raise
                cache[spec["key"]] = contract.load_json(output)
            reports[spec["key"]] = cache[spec["key"]]
            paths[spec["key"]] = output

    try:
        report = contract.extract_nana(scenario, reports, source_paths=paths)
    except KeyError as exc:
        return contract.envelope(
            runner="nana",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            command=commands,
            unsupported_reason=contract.key_error_reason(exc),
        )
    report["scenario_id"] = scenario_id
    report["scenario"] = scenario
    if commands:
        report["command"] = commands
    return report


def catalog_framework_window_args(repo_root: Path) -> list[str]:
    """List+table catalog flags for one nana-framework-benchmark invocation."""
    return (
        contract.nana_framework_list_window_args(
            contract.load_scenario("virtual-list-10k", repo_root)
        )
        + contract.nana_framework_table_window_args(
            contract.load_scenario("text-table", repo_root)
        )
    )


def _extra_args(scenario: dict[str, Any], spec: dict[str, str], repo_root: Path) -> list[str] | None:
    if spec["key"] == "gpu":
        return ["--scenario", str(contract.scenario_path(scenario["id"], repo_root))]
    if spec["key"] == "framework" and scenario.get("kind") in {"VirtualList", "VirtualTree"}:
        return contract.nana_framework_list_window_args(scenario)
    if spec["key"] == "framework" and scenario.get("kind") == "Table":
        return contract.nana_framework_table_window_args(scenario)
    return None


def _guess_report_key(payload: dict[str, Any]) -> str:
    if payload.get("gpu_work") is not None or payload.get("composition") in {
        "UiOnly",
        "UiLive2d",
        "UiLive2dEffect",
    }:
        return "gpu"
    if (
        "virtual_list_10k_materialize_ms" in payload
        or "virtual_scales" in payload
        or "catalog_workloads" in payload
    ):
        return "framework"
    if "rows" in payload and "phase" in payload:
        return "scene"
    return "runtime"


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    if "--print-framework-window-args" in argv:
        repo_root = contract.REPO_ROOT
        if "--repo-root" in argv:
            idx = argv.index("--repo-root")
            repo_root = Path(argv[idx + 1]).resolve()
        print(" ".join(catalog_framework_window_args(repo_root)))
        return 0
    return contract.run_cli(runner="nana", argv=argv, plan=plan, execute=execute)


if __name__ == "__main__":
    raise SystemExit(main())
