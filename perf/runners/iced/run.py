#!/usr/bin/env python3
"""Issue #8 / #12 Iced reference runner.

StaticTree invokes ``engine/iced`` ``scenario-bench``, which reads the shared
Scenario JSON and materializes the complete-binary-heap used by Nana
``tree_mutations`` (parent(i)=i//2, element-div, no text). That path is
``same-scenario`` only when the report declares that generation.

``--from-report`` still accepts historical Gallery ``ui-benchmark`` JSON as
``closest-legacy-reference``. Mutation, Hover, VirtualList, and Table stay
unsupported (exit 2) until those kinds exist on the engine/iced adapter.

Relative Iced/GPUI gates stay off: GPUI is still a stub.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

PERF = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PERF))

import contract  # noqa: E402


def plan(scenario_id: str, args: Any) -> list[str]:
    if args.from_report:
        return [f"# --from-report {args.from_report} ({scenario_id})"]
    try:
        scenario = contract.load_scenario(scenario_id, args.repo_root)
    except FileNotFoundError:
        return [f"# missing scenario file for {scenario_id}"]
    if scenario["kind"] != "StaticTree":
        return [
            f"# iced unsupported for {scenario_id}; exit {contract.EXIT_UNSUPPORTED}",
            "# engine/iced scenario-bench currently implements StaticTree only",
        ]
    output = args.repo_root / "target" / "performance" / "issue8" / f"iced-{scenario_id}.json"
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
    if scenario["kind"] in {
        "Mutation",
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
        return contract.envelope(
            runner="iced",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=(
                f"engine/iced scenario-bench currently implements StaticTree only. "
                f"{scenario['kind']} is required by #8 / not implemented. "
                "Gallery ui-benchmark is not a substitute. Fake Iced numbers are forbidden."
            ),
        )
    if scenario["kind"] != "StaticTree":
        return contract.envelope(
            runner="iced",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=(
                f"engine/iced scenario-bench has no mapping for kind={scenario['kind']}. "
                "Required by #8 / not implemented."
            ),
        )

    if args.from_report:
        source = args.from_report
        payload = contract.load_json(source)
        command: list[str] = []
    else:
        source = (
            args.repo_root / "target" / "performance" / "issue8" / f"iced-{scenario_id}.json"
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
