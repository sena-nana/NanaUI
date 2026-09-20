"""Retained-compositor gate tests for the Windows presentation path.

The measurement itself needs real Windows hardware with a DX12 adapter — there
is no synthetic substitute for `IDCompositionDevice::Commit`. What is checkable
here is the gate: that the scenario's invariants pass on a settled window and
fail on every way the host could start re-synchronising a tree that did not
move. A rule nobody has seen reject anything is not a gate.
"""
from __future__ import annotations

from pathlib import Path
from typing import Any

from .invariants import evaluate_invariants
from .schema import load_scenario


SCENARIO_ID = "windows-composition-steady"


def _settled_work() -> dict[str, Any]:
    """Counter deltas a settled window must report, however many frames it
    presented. Deltas, not totals: the window did real work to reach this
    state, and what the contract gates is that it stops."""
    return {
        "commits": 0,
        "tree_mutations": 0,
        "native_content_region_rebuilds": 0,
        "native_content_regions_considered": 0,
        "native_content_regions_changed": 0,
        "native_chrome_writes": 0,
    }


def _envelope(work: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "runner": "nana",
        "status": "ok",
        "scenario_id": SCENARIO_ID,
        "composition_work": work,
    }


def _self_test_windows_composition(root: Path | None = None) -> list[str]:
    errors: list[str] = []
    try:
        scenario = load_scenario(SCENARIO_ID, root)
    except (FileNotFoundError, ValueError) as exc:
        return [f"{SCENARIO_ID} scenario missing: {exc}"]

    rows = evaluate_invariants(scenario, _envelope(_settled_work()))
    if not rows:
        errors.append(f"{SCENARIO_ID} declares no invariants")
    failed = [row for row in rows if row.get("status") != "ok"]
    if failed:
        errors.append(f"{SCENARIO_ID} must pass on a settled window: {failed}")

    # Every counter is its own way for the steady state to be lost, so each one
    # has to be able to fail on its own.
    for key in _settled_work():
        work = _settled_work()
        work[key] = 1
        rows = evaluate_invariants(scenario, _envelope(work))
        if all(row.get("status") == "ok" for row in rows):
            errors.append(
                f"{SCENARIO_ID} must fail when {key}=1; a retained tree is not "
                "re-synchronised by a frame that changed nothing"
            )

    # Fail-closed: a run that could not measure must not read as a pass. This is
    # the likely shape of a mistake here, because the only machine that can
    # produce these numbers is a Windows one with a composition target.
    missing = _envelope(_settled_work())
    del missing["composition_work"]
    rows = evaluate_invariants(scenario, missing)
    if any(row.get("status") == "ok" for row in rows):
        errors.append(
            f"{SCENARIO_ID} must stay not-evaluable without composition_work, "
            "never treat a missing counter as zero"
        )
    return errors
