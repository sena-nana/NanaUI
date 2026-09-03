"""Issue #12 observations; never promotion gates."""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Mapping, Sequence
from .extractors import (
    extract_gpui,
    extract_iced,
)
from .invariants import (
    expand_invariant_report_paths,
    is_runner_envelope,
)
from .reports import (
    envelope,
    key_error_reason,
)
from .schema import (
    EXIT_ERROR,
    EXIT_OK,
    EXIT_UNSUPPORTED,
    RELATIVE_CPU_LIMITS,
    RELATIVE_MEMORY_LIMIT,
    REPO_ROOT,
    SCHEMA_VERSION,
    is_gpui_scenario_bench_report,
    is_iced_scenario_bench_report,
    load_json,
    load_scenario,
)




def _real_same_scenario(report: Mapping[str, Any] | None) -> bool:
    if not isinstance(report, Mapping):
        return False
    if report.get("status") != "ok" or report.get("equivalence") != "same-scenario":
        return False
    metrics = report.get("metrics")
    if not isinstance(metrics, Mapping) or not metrics:
        return False
    cpu = metrics.get("cpu_frame_ms")
    if not isinstance(cpu, Mapping):
        return False
    return cpu.get("p50") is not None and cpu.get("p95") is not None



def relative_gate_can_enforce(
    iced_report: Mapping[str, Any] | None,
    gpui_report: Mapping[str, Any] | None,
) -> bool:
    """True when both sides have real same-scenario metrics. Not multiplier CI."""
    if not _real_same_scenario(iced_report) or not _real_same_scenario(gpui_report):
        return False
    if iced_report.get("runner") != "iced" or gpui_report.get("runner") != "gpui":
        return False
    return iced_report.get("scenario_id") == gpui_report.get("scenario_id")



def named_fixed_machine(report: Mapping[str, Any] | None) -> bool:
    if not isinstance(report, Mapping):
        return False
    for key in ("machine", "machine_identity"):
        block = report.get(key)
        if isinstance(block, Mapping) and block.get("fixed_benchmark_machine") is True:
            return True
    return False



def _positive_metric(value: Any) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    if value <= 0:
        return None
    return float(value)



def _cpu_percentiles(report: Mapping[str, Any] | None) -> dict[str, float] | None:
    if not isinstance(report, Mapping):
        return None
    metrics = report.get("metrics") if isinstance(report.get("metrics"), Mapping) else {}
    cpu = metrics.get("cpu_frame_ms") if isinstance(metrics.get("cpu_frame_ms"), Mapping) else {}
    out: dict[str, float] = {}
    for key in ("p50", "p95", "p99"):
        value = _positive_metric(cpu.get(key))
        if value is None:
            return None
        out[key] = value
    return out



def _memory_bytes(report: Mapping[str, Any] | None) -> float | None:
    if not isinstance(report, Mapping):
        return None
    metrics = report.get("metrics") if isinstance(report.get("metrics"), Mapping) else {}
    for key in ("steady_state_memory_bytes", "memory_bytes", "rss_bytes"):
        value = _positive_metric(metrics.get(key))
        if value is not None:
            return value
    return None



def _honesty_error(report: Mapping[str, Any], label: str) -> str | None:
    if report.get("relative_gate_enforceable") is not False:
        return f"{label} must keep relative_gate_enforceable False"
    if report.get("status") != "ok":
        return None
    if not report.get("metrics"):
        return f"{label} ok without metrics; fake numbers are forbidden"
    if report.get("equivalence") == "same-scenario":
        cpu = (report.get("metrics") or {}).get("cpu_frame_ms")
        if not isinstance(cpu, Mapping) or _positive_metric(cpu.get("p50")) is None:
            return (
                f"{label} same-scenario ok missing positive cpu_frame_ms.p50; "
                "stuffed 0 is forbidden"
            )
    if report.get("runner") == "gpui":
        metrics = report.get("metrics") if isinstance(report.get("metrics"), Mapping) else {}
        if "present_ms" in metrics:
            return f"{label} GPUI must omit present_ms"
        if "frames_after_idle" in metrics:
            return f"{label} GPUI must omit frames_after_idle"
    return None



def load_issue12_report(path: Path, root: Path) -> dict[str, Any]:
    payload = load_json(path)
    if is_runner_envelope(payload):
        error = _honesty_error(payload, str(path))
        if error:
            raise ValueError(error)
        return payload
    scenario_id = payload.get("scenario_id")
    if not isinstance(scenario_id, str) or not scenario_id:
        raise ValueError(f"{path}: missing scenario_id")
    scenario = load_scenario(scenario_id, root)
    extractor = None
    runner = None
    if is_iced_scenario_bench_report(payload):
        extractor = extract_iced
        runner = "iced"
    elif is_gpui_scenario_bench_report(payload):
        extractor = extract_gpui
        runner = "gpui"
    else:
        raise ValueError(
            f"{path}: not a runner envelope or iced/gpui scenario-bench dump"
        )
    try:
        return extractor(scenario, payload, source_path=path)
    except KeyError as exc:
        reason = key_error_reason(exc)
        if payload.get("status") == "unsupported":
            return envelope(
                runner=runner,
                status="unsupported",
                scenario_id=scenario_id,
                scenario=scenario,
                unsupported_reason=reason,
            )
        raise ValueError(f"{path}: {reason}") from exc



def compare_issue12_pair(
    iced_report: Mapping[str, Any],
    gpui_report: Mapping[str, Any],
    nana_report: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    """Honesty fail-closed. Multipliers need Nana on a named fixed machine."""
    result: dict[str, Any] = {
        "scenario_id": iced_report.get("scenario_id") or gpui_report.get("scenario_id"),
        "relative_gate_enforceable": False,
        "metrics": {},
    }
    for label, report in (("iced", iced_report), ("gpui", gpui_report)):
        error = _honesty_error(report, label)
        if error:
            result["status"] = "error"
            result["can_enforce"] = False
            result["note"] = error
            return result
    if (
        iced_report.get("status") == "unsupported"
        or gpui_report.get("status") == "unsupported"
    ):
        result["status"] = "unsupported"
        result["can_enforce"] = False
        result["note"] = "missing same-scenario Iced+GPUI pair"
        return result
    if not relative_gate_can_enforce(iced_report, gpui_report):
        result["status"] = "error"
        result["can_enforce"] = False
        result["note"] = "Iced+GPUI pair is not real same-scenario cpu_frame_ms"
        return result

    iced_cpu = _cpu_percentiles(iced_report)
    gpui_cpu = _cpu_percentiles(gpui_report)
    if iced_cpu is None or gpui_cpu is None:
        result["status"] = "error"
        result["can_enforce"] = False
        result["note"] = (
            "same-scenario cpu_frame_ms percentiles must be positive; stuffed 0 is forbidden"
        )
        return result
    result["can_enforce"] = True

    nana_cpu = None
    if nana_report is not None:
        error = _honesty_error(nana_report, "nana")
        if error:
            result["status"] = "error"
            result["note"] = error
            return result
        if (
            nana_report.get("status") == "ok"
            and nana_report.get("equivalence") == "same-scenario"
            and nana_report.get("runner") == "nana"
        ):
            nana_cpu = _cpu_percentiles(nana_report)

    named = named_fixed_machine(iced_report) and named_fixed_machine(gpui_report)
    if nana_report is not None:
        named = named and named_fixed_machine(nana_report)
    result["named_fixed_machine"] = named

    failed = False
    for key, limit in RELATIVE_CPU_LIMITS.items():
        iced_value = iced_cpu[key]
        gpui_value = gpui_cpu[key]
        faster = min(iced_value, gpui_value)
        row: dict[str, Any] = {
            "iced": iced_value,
            "gpui": gpui_value,
            "faster": faster,
            "faster_runner": "iced" if iced_value <= gpui_value else "gpui",
            "limit": limit,
        }
        if nana_cpu is not None:
            ratio = nana_cpu[key] / faster
            row["nana"] = nana_cpu[key]
            row["nana_over_faster"] = ratio
            if named:
                if ratio <= limit:
                    row["status"] = "ok"
                else:
                    row["status"] = "failed"
                    failed = True
            else:
                row["status"] = "observation"
        else:
            row["status"] = "observation"
        result["metrics"][f"cpu_frame_ms.{key}"] = row

    iced_metrics = (
        iced_report.get("metrics") if isinstance(iced_report.get("metrics"), Mapping) else {}
    )
    iced_present = iced_metrics.get("present_ms")
    present_row: dict[str, Any] = {
        "status": "not-evaluable",
        "note": "GPUI TestWindow does not GPU-present; present_ms omitted, not 0",
    }
    if isinstance(iced_present, Mapping) and iced_present.get("p50") is not None:
        present_row["iced"] = iced_present.get("p50")
    result["metrics"]["present_ms"] = present_row

    mem_iced = _memory_bytes(iced_report)
    mem_gpui = _memory_bytes(gpui_report)
    mem_nana = _memory_bytes(nana_report) if nana_report is not None else None
    memory_row: dict[str, Any] = {"limit": RELATIVE_MEMORY_LIMIT}
    if mem_nana is not None and mem_iced is not None and mem_gpui is not None:
        faster_mem = min(mem_iced, mem_gpui)
        ratio = mem_nana / faster_mem
        memory_row.update(
            {
                "iced": mem_iced,
                "gpui": mem_gpui,
                "nana": mem_nana,
                "faster": faster_mem,
                "nana_over_faster": ratio,
            }
        )
        if named:
            if ratio <= RELATIVE_MEMORY_LIMIT:
                memory_row["status"] = "ok"
            else:
                memory_row["status"] = "failed"
                failed = True
        else:
            memory_row["status"] = "observation"
    else:
        memory_row["status"] = "not-evaluable"
        memory_row["note"] = "Iced/GPUI scenario-bench does not export process memory"
    result["metrics"]["memory"] = memory_row

    if failed:
        result["status"] = "failed"
        result["note"] = "named-fixed-machine Nana vs faster(Iced,GPUI) exceeded historical multipliers"
        return result
    if named and nana_cpu is not None:
        result["status"] = "ok"
        result["note"] = "named fixed machine; Nana vs faster(Iced,GPUI) within historical multipliers"
        return result
    result["status"] = "observation"
    result["note"] = (
        "same-scenario Iced+GPUI pair; multipliers stay observation without Nana "
        "on a named fixed machine"
    )
    return result



def evaluate_relative_paths(
    paths: Sequence[Path | str],
    *,
    root: Path | None = None,
) -> tuple[dict[str, Any], int]:
    """Compare Iced/GPUI (+ optional Nana) envelopes. Not a #8 gate."""
    base = root or REPO_ROOT
    grouped: dict[str, dict[str, dict[str, Any]]] = {}
    load_errors: list[dict[str, Any]] = []
    for path in expand_invariant_report_paths(paths):
        try:
            report = load_issue12_report(path, base)
        except FileNotFoundError:
            load_errors.append({"source": str(path), "status": "error", "note": f"missing {path}"})
            continue
        except (ValueError, json.JSONDecodeError, KeyError) as exc:
            load_errors.append({"source": str(path), "status": "error", "note": str(exc)})
            continue
        scenario_id = str(report.get("scenario_id") or "")
        runner = str(report.get("runner") or "")
        if not scenario_id or runner not in {"iced", "gpui", "nana"}:
            load_errors.append(
                {
                    "source": str(path),
                    "status": "error",
                    "note": f"{path} is not an iced/gpui/nana #12 comparison envelope",
                }
            )
            continue
        by_runner = grouped.setdefault(scenario_id, {})
        if runner in by_runner:
            load_errors.append(
                {
                    "source": str(path),
                    "status": "error",
                    "note": f"duplicate {runner} envelope for {scenario_id}",
                }
            )
            continue
        by_runner[runner] = report
        by_runner.setdefault("_sources", {})[runner] = str(path)

    pairs: list[dict[str, Any]] = []
    for scenario_id, by_runner in grouped.items():
        iced_report = by_runner.get("iced")
        gpui_report = by_runner.get("gpui")
        if iced_report is None or gpui_report is None:
            continue
        compared = compare_issue12_pair(
            iced_report, gpui_report, by_runner.get("nana")
        )
        compared["sources"] = by_runner.get("_sources", {})
        pairs.append(compared)

    summary: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "relative_gate_enforceable": False,
        "note": "Issue #12 observation, not multiplier CI",
        "pairs": pairs,
        "errors": load_errors,
    }
    if load_errors or any(item.get("status") == "error" for item in pairs):
        summary["status"] = "error"
        return summary, EXIT_ERROR
    if any(item.get("status") == "failed" for item in pairs):
        summary["status"] = "failed"
        return summary, EXIT_ERROR
    comparable = [item for item in pairs if item.get("can_enforce")]
    if not comparable:
        summary["status"] = "unsupported"
        summary["note"] = (
            "no real same-scenario Iced+GPUI pair"
        )
        return summary, EXIT_UNSUPPORTED
    summary["status"] = "observation"
    if any(item.get("status") == "ok" for item in pairs) and all(
        item.get("status") in {"ok", "unsupported"} for item in pairs
    ):
        summary["status"] = "ok"
    return summary, EXIT_OK
