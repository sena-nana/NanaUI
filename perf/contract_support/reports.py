"""Stable runner report envelopes."""
from __future__ import annotations

from typing import Any, Iterable, Mapping, Sequence
from .invariants import (
    evaluate_invariants,
    machine_note,
)
from .schema import (
    SCHEMA_VERSION,
)




def envelope(
    *,
    runner: str,
    status: str,
    scenario_id: str,
    scenario: Mapping[str, Any] | None = None,
    unsupported_reason: str | None = None,
    error: str | None = None,
    equivalence: str | None = None,
    command: Sequence[str] | None = None,
    source_binary: str | None = None,
    source_report: str | None = None,
    mapping_notes: Sequence[str] | None = None,
    metrics: Mapping[str, Any] | None = None,
    work_counters: Mapping[str, Any] | None = None,
    plug_in: str | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "runner": runner,
        "status": status,
        "scenario_id": scenario_id,
        "timing_gate_enforceable": False,
        "relative_gate_enforceable": False,  # stays False; #12 observation only
        "machine": machine_note(),
    }
    if scenario is not None:
        payload["scenario"] = dict(scenario)
    if unsupported_reason:
        payload["unsupported_reason"] = unsupported_reason
    if error:
        payload["error"] = error
    payload["equivalence"] = equivalence or (
        "unsupported" if status == "unsupported" else "closest-legacy-reference"
    )
    if command:
        payload["command"] = list(command)
    if source_binary:
        payload["source_binary"] = source_binary
    if source_report:
        payload["source_report"] = source_report
    if mapping_notes:
        payload["mapping_notes"] = list(mapping_notes)
    if metrics:
        payload["metrics"] = {
            key: value for key, value in metrics.items() if value is not None
        }
        if not payload["metrics"]:
            del payload["metrics"]
    measured = measured_counters(work_counters)
    if measured:
        payload["work_counters"] = measured
    if plug_in:
        payload["plug_in"] = plug_in
    evaluated = evaluate_invariants(scenario, payload)
    if evaluated:
        payload["invariants"] = evaluated
        if payload.get("status") == "ok" and any(
            item.get("status") == "failed" for item in evaluated
        ):
            payload["status"] = "error"
            failed = [
                str(item.get("name") or item.get("path"))
                for item in evaluated
                if item.get("status") == "failed"
            ]
            payload["error"] = "work-counter invariant failed: " + ", ".join(failed)
    return payload



def key_error_reason(exc: KeyError) -> str:
    if exc.args and isinstance(exc.args[0], str):
        return exc.args[0]
    return str(exc)



def measured_counters(values: Mapping[str, Any] | None) -> dict[str, Any] | None:
    if not values:
        return None
    measured = {key: value for key, value in values.items() if value is not None}
    return measured or None



def percentile_fields(sample: Mapping[str, Any] | None) -> dict[str, Any] | None:
    if not isinstance(sample, dict):
        return None
    p50 = sample.get("p50", sample.get("median"))
    p95 = sample.get("p95")
    p99 = sample.get("p99")
    if p50 is None and p95 is None and p99 is None:
        return None
    out: dict[str, Any] = {}
    if p50 is not None:
        out["p50"] = p50
    if p95 is not None:
        out["p95"] = p95
    if p99 is not None:
        out["p99"] = p99
    if "min" in sample:
        out["min"] = sample["min"]
    if "max" in sample:
        out["max"] = sample["max"]
    if "frame_budget_misses" in sample:
        out["frame_budget_misses"] = sample["frame_budget_misses"]
    if "frame_budget_ms" in sample:
        out["frame_budget_ms"] = sample["frame_budget_ms"]
    return out



def find_case_by_nodes(cases: Iterable[Mapping[str, Any]], nodes: int) -> dict[str, Any] | None:
    for case in cases:
        if case.get("nodes") == nodes:
            return dict(case)
    return None
