#!/usr/bin/env python3
"""Issue #12 GPUI adapter. Live compile was removed; --from-report fixtures remain."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

PERF = Path(__file__).resolve().parents[2]
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
            "# gpui-scenario-bench implements StaticTree, Mutation, Hover, VirtualList, Table",
        ]
    return [
        f"# gpui unsupported for {scenario['id']}; exit {contract.EXIT_UNSUPPORTED}",
        f"# {contract.GPUI_SNAPSHOT_REMOVED_REASON}",
    ]


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
                f"gpui-scenario-bench has no {scenario['kind']} adapter; fake numbers are forbidden"
            ),
        )

    if not args.from_report:
        return contract.envelope(
            runner="gpui",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=contract.GPUI_SNAPSHOT_REMOVED_REASON,
        )

    command: list[str] = []
    source = args.from_report
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
