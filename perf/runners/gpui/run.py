#!/usr/bin/env python3
"""Issue #8 GPUI runner stub.

NanaUI does not depend on GPUI. This process implements the same CLI and
Scenario schema as the Nana and Iced runners, then exits 2 (unsupported).

CI must treat exit 2 as "reference missing", not as a failed benchmark.
Invented GPUI timings are forbidden.

Plug-in: add ``perf/runners/gpui/adapter.py`` with
``run_scenario(scenario, args) -> dict`` that consumes the same Scenario JSON
and returns the shared run-report envelope with ``status=ok`` only after a real
GPUI build ran.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from typing import Any

PERF = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PERF))

import contract  # noqa: E402

ADAPTER_PATH = Path(__file__).resolve().parent / "adapter.py"


def _load_adapter() -> Any | None:
    if not ADAPTER_PATH.is_file():
        return None
    spec = importlib.util.spec_from_file_location("nana_perf_gpui_adapter", ADAPTER_PATH)
    if spec is None or spec.loader is None:
        return None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def plan(scenario_id: str, args: Any) -> list[str]:
    if ADAPTER_PATH.is_file():
        return [f"# gpui adapter present: {ADAPTER_PATH} ({scenario_id})"]
    return [
        f"# gpui unsupported for {scenario_id}; exit {contract.EXIT_UNSUPPORTED}",
        "# add perf/runners/gpui/adapter.py to plug in a real GPUI Scenario runner",
    ]


def execute(scenario_id: str, args: Any) -> dict[str, Any]:
    try:
        scenario = contract.load_scenario(scenario_id, args.repo_root)
    except FileNotFoundError:
        scenario = {"schema_version": 1, "id": scenario_id, "kind": "StaticTree", "params": {}}
        return contract.gpui_unsupported(scenario) | {
            "unsupported_reason": f"No scenario JSON at perf/scenarios/{scenario_id}.json"
        }

    adapter = _load_adapter()
    if adapter is not None and hasattr(adapter, "run_scenario"):
        report = adapter.run_scenario(scenario, args)
        if not isinstance(report, dict) or report.get("runner") != "gpui":
            return contract.envelope(
                runner="gpui",
                status="error",
                scenario_id=scenario_id,
                scenario=scenario,
                error="adapter.run_scenario must return a gpui run-report envelope",
            )
        if report.get("status") == "ok" and not report.get("metrics"):
            return contract.envelope(
                runner="gpui",
                status="error",
                scenario_id=scenario_id,
                scenario=scenario,
                error="GPUI adapter returned ok without metrics; fake numbers are forbidden",
            )
        return report
    return contract.gpui_unsupported(scenario)


def main(argv: list[str] | None = None) -> int:
    return contract.run_cli(runner="gpui", argv=argv, plan=plan, execute=execute)


if __name__ == "__main__":
    raise SystemExit(main())
