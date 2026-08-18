#!/usr/bin/env python3
"""Issue #8 Iced reference runner.

Wraps the existing Gallery ``ui-benchmark`` binary. Gallery lists still perform
full layout of every item; this is a legacy reference, not a virtualization
claim.

The current binary paints through SceneWgpuPainter. Historical Iced numbers live
in docs/performance-baseline.md. A dedicated engine/iced adapter that builds the
same Scenario tree is required by #8 / not implemented.
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
    output = args.repo_root / "target" / "performance" / "issue8" / "ui-benchmark.json"
    command = contract.cargo_run(
        args.repo_root,
        package="component-gallery",
        binary="ui-benchmark",
        features="benchmark",
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
    if scenario["kind"] == "Mutation":
        return contract.envelope(
            runner="iced",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=(
                "ui-benchmark has no isolated paint-only / single-node mutation case. "
                "Required by #8 / not implemented on the Gallery path."
            ),
        )
    if scenario["kind"] == "Hover":
        return contract.envelope(
            runner="iced",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=(
                "ui-benchmark has no dedicated same-Scenario hover case. "
                "Gallery list-100 event_update_ms is not hover.json."
            ),
        )
    if scenario["kind"] == "VirtualList":
        return contract.envelope(
            runner="iced",
            status="unsupported",
            scenario_id=scenario_id,
            scenario=scenario,
            unsupported_reason=(
                "Gallery lists are full-layout and are not VirtualList "
                "{items, visible, overscan}. list-1000 is not virtual-list-10k."
            ),
        )

    if args.from_report:
        source = args.from_report
        payload = contract.load_json(source)
        command: list[str] = []
    else:
        source = args.repo_root / "target" / "performance" / "issue8" / "ui-benchmark.json"
        cache = getattr(args, "_iced_cache", None)
        command = contract.cargo_run(
            args.repo_root,
            package="component-gallery",
            binary="ui-benchmark",
            features="benchmark",
            output=source,
        )
        if cache is None:
            source.parent.mkdir(parents=True, exist_ok=True)
            contract.run_command(command, args.repo_root)
            cache = contract.load_json(source)
            setattr(args, "_iced_cache", cache)
        payload = cache

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
