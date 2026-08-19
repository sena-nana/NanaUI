#!/usr/bin/env python3
"""Issue #12 GPUI adapter. Dumps under target/performance/issue12/; not a #8 gate."""

from __future__ import annotations

import subprocess
from pathlib import Path
from typing import Any

PERF = Path(__file__).resolve().parents[2]
import sys

sys.path.insert(0, str(PERF))

import contract  # noqa: E402

SCENARIO_BENCH_KINDS = {"StaticTree", "Mutation", "Hover", "VirtualList", "Table"}


def plan_scenario(scenario: dict[str, Any], args: Any) -> list[str]:
    if args.from_report:
        return [f"# --from-report {args.from_report} ({scenario['id']})"]
    skip = contract.gpui_scenario_bench_skip_reason(scenario)
    if skip:
        return [
            f"# gpui unsupported for {scenario['id']}; exit {contract.EXIT_UNSUPPORTED}",
            f"# {skip}",
        ]
    if scenario["kind"] not in SCENARIO_BENCH_KINDS:
        return [
            f"# gpui unsupported for {scenario['id']}; exit {contract.EXIT_UNSUPPORTED}",
            "# engine/gpui-scenario-bench implements StaticTree, Mutation, Hover, "
            "VirtualList, and Table only",
        ]
    output = (
        args.repo_root / "target" / "performance" / "issue12" / f"gpui-{scenario['id']}.json"
    )
    command = contract.cargo_run_gpui_scenario_bench(
        args.repo_root,
        scenario_path=contract.scenario_path(scenario["id"], args.repo_root),
        output=output,
    )
    return [" ".join(command)]


def run_scenario(scenario: dict[str, Any], args: Any) -> dict[str, Any]:
    scenario_id = scenario["id"]
    skip = contract.gpui_scenario_bench_skip_reason(scenario)
    if skip:
        return contract.envelope(
            runner="gpui",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=skip,
        )
    if scenario["kind"] not in SCENARIO_BENCH_KINDS:
        return contract.envelope(
            runner="gpui",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=(
                "engine/gpui-scenario-bench implements StaticTree, Mutation, Hover, "
                f"VirtualList, and Table. {scenario['kind']} is #12 / not implemented. "
                "Fake GPUI numbers are forbidden."
            ),
        )

    command: list[str] = []
    if args.from_report:
        source = args.from_report
        payload = contract.load_json(source)
    else:
        source = (
            args.repo_root / "target" / "performance" / "issue12" / f"gpui-{scenario_id}.json"
        )
        command = contract.cargo_run_gpui_scenario_bench(
            args.repo_root,
            scenario_path=contract.scenario_path(scenario_id, args.repo_root),
            output=source,
        )
        source.parent.mkdir(parents=True, exist_ok=True)
        try:
            contract.run_command(command, args.repo_root)
        except subprocess.CalledProcessError as exc:
            if exc.returncode == contract.EXIT_UNSUPPORTED and source.is_file():
                payload = contract.load_json(source)
                if payload.get("status") == "unsupported":
                    try:
                        return contract.extract_gpui(scenario, payload, source_path=source)
                    except KeyError as key_exc:
                        return contract.envelope(
                            runner="gpui",
                            status="unsupported",
                            scenario_id=scenario_id,
                            scenario=scenario,
                            command=command,
                            unsupported_reason=contract.key_error_reason(key_exc),
                        )
            raise
        payload = contract.load_json(source)

    try:
        report = contract.extract_gpui(scenario, payload, source_path=source)
    except KeyError as exc:
        return contract.envelope(
            runner="gpui",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            command=command,
            unsupported_reason=contract.key_error_reason(exc),
        )
    if command:
        report["command"] = [" ".join(command)]
    return report
