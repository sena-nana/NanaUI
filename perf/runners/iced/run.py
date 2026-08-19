#!/usr/bin/env python3
"""Optional Iced triangulation runner. See docs/performance-contract.md.

StaticTree, Mutation (PaintOnly / Text / LayoutStyle), Hover, VirtualList,
and Table invoke ``engine/iced`` ``scenario-bench`` against the shared Scenario JSON.
Visibility / Transform / Accessibility stay unsupported: Iced has no
``hidden`` / ``PaintTransform`` / ``set_accessibility`` equivalent.
VirtualList and Table materialize only the catalog window (Nana runner now
passes the same px windows). StaticTree 50k stays unsupported.

``--from-report`` still accepts historical Gallery ``ui-benchmark`` JSON as
``closest-legacy-reference`` for tiny StaticTree ids. Animation, Ime, Dock,
Overlay, TextEditor, VirtualTree, and GpuScene stay unsupported (exit 2). Topology-only
``pane_grid`` is not Nana ``assemble_dock`` chrome. A cached Iced editor frame
is not Nana ``replace_text_area_selection`` + ``drain_text``.

``--evaluate-invariants`` skips Iced envelopes. Relative Iced/GPUI multipliers
stay off.
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
            "engine/iced scenario-bench implements StaticTree, Mutation, Hover, "
            "VirtualList, and Table only",
        )
    output = args.repo_root / "target" / "performance" / "issue12" / f"iced-{scenario_id}.json"
    command = contract.cargo_run_iced_scenario_bench(
        args.repo_root,
        scenario_path=contract.scenario_path(scenario_id, args.repo_root),
        output=output,
    )
    return [" ".join(command)]


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
                f"engine/iced scenario-bench implements StaticTree, Mutation, Hover, "
                f"VirtualList, and Table. {scenario['kind']} is required by #8 / not implemented. "
                "Gallery ui-benchmark is not a substitute. Fake Iced numbers are forbidden."
            ),
        )

    if args.from_report:
        source = args.from_report
        payload = contract.load_json(source)
        command: list[str] = []
    else:
        source = (
            args.repo_root / "target" / "performance" / "issue12" / f"iced-{scenario_id}.json"
        )
        command = contract.cargo_run_iced_scenario_bench(
            args.repo_root,
            scenario_path=contract.scenario_path(scenario_id, args.repo_root),
            output=source,
        )
        source.parent.mkdir(parents=True, exist_ok=True)
        contract.run_command(command, args.repo_root)
        payload = contract.load_json(source)

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
