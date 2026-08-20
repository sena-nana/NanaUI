#!/usr/bin/env python3
"""Issue #12 Iced observation runner. Not a Nana #8 gate.

Live scenario-bench was removed with engine/iced. ``--from-report`` still maps
archived fixtures. Other kinds stay exit 2. ``--evaluate-invariants`` skips
Iced envelopes.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

PERF = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PERF))

import contract  # noqa: E402

SCENARIO_BENCH_KINDS = {"StaticTree", "Mutation", "Hover", "VirtualList", "Table"}


def _unsupported_plan(scenario_id: str, reason: str) -> list[str]:
    return [
        f"# iced unsupported for {scenario_id}; exit {contract.EXIT_UNSUPPORTED}",
        f"# {reason}",
    ]


def plan(scenario_id: str, args: Any) -> list[str]:
    if args.from_report:
        return [f"# --from-report {args.from_report} ({scenario_id})"]
    try:
        scenario = contract.load_scenario(scenario_id, args.repo_root)
    except FileNotFoundError:
        return [f"# missing scenario file for {scenario_id}"]
    reason = contract.iced_scenario_bench_skip_reason(scenario)
    if reason:
        return _unsupported_plan(scenario_id, reason)
    if scenario["kind"] not in SCENARIO_BENCH_KINDS:
        return _unsupported_plan(
            scenario_id,
            "scenario-bench implements StaticTree, Mutation, Hover, VirtualList, Table",
        )
    return _unsupported_plan(scenario_id, contract.ICED_SNAPSHOT_REMOVED_REASON)


def execute(scenario_id: str, args: Any) -> dict[str, Any]:
    try:
        scenario = contract.load_scenario(scenario_id, args.repo_root)
    except FileNotFoundError:
        return contract.envelope(
            runner="iced",
            status="unsupported",
            scenario_id=scenario_id,
            unsupported_reason=f"No scenario JSON at perf/scenarios/{scenario_id}.json",
        )
    skip = contract.iced_scenario_bench_skip_reason(scenario)
    if skip:
        return contract.envelope(
            runner="iced",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=skip,
        )
    if scenario["kind"] not in SCENARIO_BENCH_KINDS:
        return contract.envelope(
            runner="iced",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=(
                f"scenario-bench has no {scenario['kind']} adapter; fake numbers are forbidden"
            ),
        )

    if not args.from_report:
        return contract.envelope(
            runner="iced",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=contract.ICED_SNAPSHOT_REMOVED_REASON,
        )

    source = args.from_report
    payload = contract.load_json(source)
    command: list[str] = []

    try:
        report = contract.extract_iced(scenario, payload, source_path=source)
    except KeyError as exc:
        return contract.envelope(
            runner="iced",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            command=command,
            unsupported_reason=contract.key_error_reason(exc),
        )
    if command:
        report["command"] = [" ".join(command)]
    return report


def main(argv: list[str] | None = None) -> int:
    return contract.run_cli(runner="iced", argv=argv, plan=plan, execute=execute)


if __name__ == "__main__":
    raise SystemExit(main())
