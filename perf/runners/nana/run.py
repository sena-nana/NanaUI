#!/usr/bin/env python3
"""Map Issue #8 Scenarios onto existing NanaUI benchmark binaries.

Does not reimplement Runtime. Invokes:

- nana-runtime-benchmark (StaticTree, PaintOnly, Hover)
- nana-framework-benchmark (VirtualList)
- nana-scene-benchmark (optional StaticTree scene rows)

Relative Iced/GPUI gates are not applied here.
"""

from __future__ import annotations

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


def _needed_bins(scenario: dict[str, Any]) -> list[dict[str, str]]:
    kind = scenario["kind"]
    if kind == "StaticTree":
        return [RUNTIME_BIN, SCENE_BIN]
    if kind in {"Mutation", "Hover"}:
        return [RUNTIME_BIN]
    if kind == "VirtualList":
        return [FRAMEWORK_BIN]
    return []


def plan(scenario_id: str, args: Any) -> list[str]:
    try:
        scenario = contract.load_scenario(scenario_id, args.repo_root)
    except FileNotFoundError:
        return [f"# missing scenario file for {scenario_id}"]
    bins = _needed_bins(scenario)
    if not bins:
        return [f"# no Nana binary mapping for {scenario_id}"]
    if args.from_report:
        return [f"# --from-report {args.from_report} ({scenario_id})"]
    lines = []
    work = args.repo_root / "target" / "performance" / "issue8"
    for spec in bins:
        output = work / f"{spec['key']}.json"
        command = contract.cargo_run(
            args.repo_root,
            package=spec["package"],
            binary=spec["binary"],
            features=spec["features"],
            output=output,
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
            command = contract.cargo_run(
                args.repo_root,
                package=spec["package"],
                binary=spec["binary"],
                features=spec["features"],
                output=output,
            )
            commands.append(" ".join(command))
            if spec["key"] not in cache:
                contract.run_command(command, args.repo_root)
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


def _guess_report_key(payload: dict[str, Any]) -> str:
    if "virtual_list_10k_materialize_ms" in payload or "virtual_scales" in payload:
        return "framework"
    if "rows" in payload and "phase" in payload:
        return "scene"
    return "runtime"


def main(argv: list[str] | None = None) -> int:
    return contract.run_cli(runner="nana", argv=argv, plan=plan, execute=execute)


if __name__ == "__main__":
    raise SystemExit(main())
