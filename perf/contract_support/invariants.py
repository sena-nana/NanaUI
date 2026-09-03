"""Issue #8 measured work-counter judgments."""
from __future__ import annotations

import json
import platform
from pathlib import Path
from typing import Any, Mapping, Sequence
from .schema import (
    EXIT_ERROR,
    EXIT_OK,
    EXIT_UNSUPPORTED,
    GPU_WORK_COUNTER_KEYS,
    PR_INVARIANTS_DIR_NAME,
    SCHEMA_VERSION,
    SECTION_8_1_CATALOG_WORKLOAD_IDS,
    SECTION_8_1_HONEST_OK_IDS,
    SECTION_8_1_STATIC_UI_IDS,
    SECTION_8_1_UNSUPPORTED_IDS,
    WORK_COUNTER_KEYS,
    load_json,
    load_scenario,
)




def machine_note() -> dict[str, Any]:
    return {
        "system": platform.system(),
        "release": platform.release(),
        "machine": platform.machine(),
        "python": platform.python_version(),
        "hostname": platform.node(),
        "fixed_benchmark_machine": False,
        "note": (
            "This is the host that invoked the runner. GitHub "
            "`ubuntu-latest` / `macos-latest` weekly cron is not a named "
            "fixed benchmark machine (Issue #8 §8.2 / #12)."
        ),
    }



def counters_from_block(block: Any) -> dict[str, Any]:
    """Copy measured CPU WorkCounters. Nulls and GPU keys stay out.

    GPU keys on a CPU drain — including explicit 0 — are not observations.
    Only :func:`gpu_counters_from_observed` copies them after encode/submit.
    """
    if not isinstance(block, dict):
        return {}
    return {
        key: block[key]
        for key in WORK_COUNTER_KEYS
        if key in block
        and block[key] is not None
        and key not in GPU_WORK_COUNTER_KEYS
    }



def gpu_counters_from_observed(block: Any) -> dict[str, Any]:
    """Copy GPU keys only after a host encode/submit. Missing stays KeyError."""
    if not isinstance(block, dict):
        raise KeyError(
            "nana-gpu-scene-benchmark ok report must include gpu_work from a real encode/submit"
        )
    observed: dict[str, Any] = {}
    for key in GPU_WORK_COUNTER_KEYS:
        if key not in block or block[key] is None:
            raise KeyError(
                f"nana-gpu-scene-benchmark ok report omitted {key}; do not invent 0"
            )
        observed[key] = block[key]
    return observed



def lookup_path(payload: Mapping[str, Any] | None, path: str) -> Any:
    current: Any = payload
    for part in path.split("."):
        if not isinstance(current, Mapping) or part not in current:
            return None
        current = current[part]
    return current



def read_frames_after_idle(
    payload: Mapping[str, Any] | None,
    *,
    required: bool,
    source: str,
) -> int | None:
    """Copy a real idle-frame count. Never map idle_schedule_ms onto this field."""
    if not isinstance(payload, Mapping) or "frames_after_idle" not in payload:
        if required:
            raise KeyError(
                f"{source} must export integer frames_after_idle after settle; "
                "idle_schedule_ms is a timing, not that counter"
            )
        return None
    value = payload["frames_after_idle"]
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise KeyError(
            f"{source} frames_after_idle must be a non-negative integer, not {value!r}"
        )
    return value



def compare_invariant(measured: Any, op: str, expected: Any) -> bool | None:
    if op == "eq":
        return measured == expected
    if op == "lte":
        try:
            return measured <= expected
        except TypeError:
            return None
    if op == "gte":
        try:
            return measured >= expected
        except TypeError:
            return None
    return None



def evaluate_invariants(
    scenario: Mapping[str, Any] | None,
    payload: Mapping[str, Any],
) -> list[dict[str, Any]]:
    """Fail-closed: missing/null measured values stay not-evaluable, never ok."""
    if not isinstance(scenario, Mapping):
        return []
    results: list[dict[str, Any]] = []
    for spec in scenario.get("invariants") or []:
        if not isinstance(spec, Mapping):
            continue
        item: dict[str, Any] = {}
        name = spec.get("name")
        path = spec.get("path")
        op = spec.get("op")
        if name:
            item["name"] = name
        if path:
            item["path"] = path
        if op:
            item["op"] = op
        if "value" in spec:
            item["value"] = spec["value"]
        if not path or not op or "value" not in spec:
            item["status"] = "not-evaluable"
            if spec.get("note"):
                item["note"] = spec["note"]
            results.append(item)
            continue
        measured = lookup_path(payload, path)
        if measured is None:
            item["status"] = "not-evaluable"
            item["note"] = spec.get("note") or (
                f"{path} is missing; never treat a missing key as the expected value"
            )
            results.append(item)
            continue
        passed = compare_invariant(measured, op, spec.get("value"))
        if passed is None:
            item["status"] = "not-evaluable"
            item["note"] = spec.get("note") or f"cannot evaluate {op} on {path}"
        else:
            item["measured"] = measured
            item["status"] = "ok" if passed else "failed"
            if not passed:
                item["note"] = (
                    f"measured {measured} does not satisfy {op} {spec.get('value')}"
                )
        results.append(item)
    return results



def is_runner_envelope(payload: Mapping[str, Any] | None) -> bool:
    if not isinstance(payload, Mapping):
        return False
    return (
        payload.get("schema_version") == SCHEMA_VERSION
        and payload.get("runner") in {"nana", "iced", "gpui"}
        and payload.get("status") in {"ok", "unsupported", "error"}
        and isinstance(payload.get("scenario_id"), str)
        and bool(payload.get("scenario_id"))
    )



def _skip_section_8_1(scenario_id: str, runner: str, status: str) -> str | None:
    if runner == "gpui":
        return "GPUI is #12 observation; skipped, not a Nana §8.1 gate"
    if runner == "iced":
        return "Iced is #12 observation; skipped, not a Nana §8.1 gate"
    if scenario_id in SECTION_8_1_UNSUPPORTED_IDS:
        return (
            f"{scenario_id} is not a §8.1 honest-ok catalog id; skipped, not invariant-ok"
        )
    if scenario_id not in SECTION_8_1_HONEST_OK_IDS:
        return (
            f"{scenario_id} is not a §8.1 honest-ok catalog id; skipped, not invariant-ok"
        )
    if scenario_id == "gpu-scene-ui" and runner != "nana":
        return (
            "Iced/GPUI GpuScene stay unsupported; Nana UiOnly encode is the §8.1 path"
        )
    if status == "unsupported":
        return "envelope status=unsupported"
    return None



def judge_runner_invariants(
    report: Mapping[str, Any],
    *,
    root: Path | None = None,
) -> dict[str, Any]:
    """Judge §8.1 invariants from one runner envelope using ``evaluate_invariants``.

    This is the PR/CI entry: same rule engine runners already attach, not a
    second copy of the comparisons. Unsupported ids/runners stay skipped.
    Missing ``work_counters.layout_nodes`` stays not-evaluable / skip, never
    envelope-ok.
    """
    if not is_runner_envelope(report):
        return {
            "decision": "error",
            "note": (
                "not a runner envelope (need schema_version, runner, status, scenario_id)"
            ),
        }
    scenario_id = str(report["scenario_id"])
    runner = str(report["runner"])
    status = str(report["status"])
    judged: dict[str, Any] = {
        "scenario_id": scenario_id,
        "runner": runner,
        "envelope_status": status,
    }
    if report.get("equivalence") is not None:
        judged["equivalence"] = report.get("equivalence")
    skip = _skip_section_8_1(scenario_id, runner, status)
    if skip:
        judged["decision"] = "skipped"
        if status == "unsupported":
            judged["note"] = report.get("unsupported_reason") or skip
        else:
            judged["note"] = skip
        return judged
    if status == "error":
        judged["decision"] = "failed"
        judged["note"] = report.get("error") or "envelope status=error"
        return judged
    try:
        scenario = load_scenario(scenario_id, root)
    except (FileNotFoundError, ValueError) as exc:
        judged["decision"] = "error"
        judged["note"] = str(exc)
        return judged
    evaluated = evaluate_invariants(scenario, report)
    if not evaluated:
        judged["decision"] = "skipped"
        judged["note"] = (
            "catalog has no invariants; vacuous ok is forbidden until a real row exists"
        )
        return judged
    judged["invariants"] = evaluated
    failed = [
        str(item.get("name") or item.get("path"))
        for item in evaluated
        if item.get("status") == "failed"
    ]
    if failed:
        judged["decision"] = "failed"
        judged["note"] = "work-counter invariant failed: " + ", ".join(failed)
        return judged
    unevaluable = [
        str(item.get("path") or item.get("name") or "unknown")
        for item in evaluated
        if item.get("status") == "not-evaluable"
    ]
    if scenario_id in SECTION_8_1_STATIC_UI_IDS and unevaluable:
        judged["decision"] = "skipped"
        judged["note"] = (
            f"{', '.join(unevaluable)} missing; vacuous ok is forbidden until runners "
            "export the measured value"
        )
        return judged
    if scenario_id == "gpu-scene-ui" and any(
        item.get("status") == "not-evaluable" for item in evaluated
    ):
        judged["decision"] = "skipped"
        judged["note"] = (
            "gpu-scene-ui GPU keys missing; vacuous ok is forbidden until encode/submit "
            "observes gpu_upload_bytes and draw_calls. Do not invent 0."
        )
        return judged
    if scenario_id in SECTION_8_1_CATALOG_WORKLOAD_IDS and any(
        item.get("status") == "not-evaluable" for item in evaluated
    ):
        judged["decision"] = "skipped"
        judged["note"] = (
            f"{scenario_id} catalog counter missing; vacuous ok is forbidden until the "
            "live dump exports the catalog invariant field"
        )
        return judged
    if scenario_id == "text-table" and any(
        item.get("status") == "not-evaluable"
        and item.get("path")
        in {
            "work_counters.glyph_cache_hits",
            "work_counters.glyph_cache_misses",
        }
        for item in evaluated
    ):
        judged["decision"] = "skipped"
        judged["note"] = (
            "text-table glyph_cache_hits/misses missing; vacuous ok is forbidden "
            "until GlyphCache lookup/insert is exported. Do not invent 0."
        )
        return judged
    layout_nodes_unevaluable = any(
        item.get("path") == "work_counters.layout_nodes"
        and item.get("status") == "not-evaluable"
        for item in evaluated
    )
    if layout_nodes_unevaluable:
        judged["decision"] = "skipped"
        judged["note"] = (
            "work_counters.layout_nodes missing; not-evaluable stays skip, never envelope-ok"
        )
        return judged
    judged["decision"] = "ok"
    return judged



def expand_invariant_report_paths(paths: Sequence[Path | str]) -> list[Path]:
    expanded: list[Path] = []
    for raw in paths:
        path = Path(raw)
        if path.is_dir():
            files = sorted(path.glob("*.json"))
            if not files:
                raise FileNotFoundError(f"no *.json envelopes in directory {path}")
            expanded.extend(files)
            continue
        expanded.append(path)
    return expanded



def _requires_honest_ok_set(paths: Sequence[Path | str]) -> bool:
    """Full HONEST_OK completeness applies only to a PR ``invariants/`` directory."""
    return any(
        Path(raw).is_dir() and Path(raw).name == PR_INVARIANTS_DIR_NAME for raw in paths
    )



def evaluate_runner_invariant_paths(
    paths: Sequence[Path | str],
    *,
    root: Path | None = None,
) -> tuple[dict[str, Any], int]:
    """Judge one or more runner envelopes. Returns (summary, exit code)."""
    reports: list[dict[str, Any]] = []
    for path in expand_invariant_report_paths(paths):
        try:
            payload = load_json(path)
        except FileNotFoundError:
            reports.append(
                {
                    "source": str(path),
                    "decision": "error",
                    "note": f"report not found: {path}",
                }
            )
            continue
        except json.JSONDecodeError as exc:
            reports.append(
                {
                    "source": str(path),
                    "decision": "error",
                    "note": f"invalid JSON: {exc}",
                }
            )
            continue
        judged = judge_runner_invariants(payload, root=root)
        judged["source"] = str(path)
        reports.append(judged)
    failed = [item for item in reports if item.get("decision") in {"failed", "error"}]
    ok = [item for item in reports if item.get("decision") == "ok"]
    skipped = [item for item in reports if item.get("decision") == "skipped"]
    summary: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "ok": len(ok),
        "failed": len(failed),
        "skipped": len(skipped),
        "reports": reports,
    }
    if failed:
        summary["status"] = "failed"
        return summary, EXIT_ERROR
    present_ok = {item.get("scenario_id") for item in ok}
    gated_skipped = sorted(
        {
            str(item.get("scenario_id"))
            for item in skipped
            if item.get("scenario_id") in SECTION_8_1_HONEST_OK_IDS
            and item.get("runner") == "nana"
            and item.get("scenario_id") not in present_ok
        }
    )
    if ok and gated_skipped:
        summary["status"] = "failed"
        summary["note"] = (
            "§8.1 honest-ok catalog id skipped; mixed skip is fail-closed: "
            + ", ".join(gated_skipped)
        )
        return summary, EXIT_ERROR
    if ok and _requires_honest_ok_set(paths):
        missing = sorted(SECTION_8_1_HONEST_OK_IDS - present_ok)
        if missing:
            summary["status"] = "failed"
            summary["note"] = (
                "§8.1 directory missing gated Nana ids: " + ", ".join(missing)
            )
            return summary, EXIT_ERROR
    if ok:
        summary["status"] = "ok"
        return summary, EXIT_OK
    summary["status"] = "unsupported"
    return summary, EXIT_UNSUPPORTED
