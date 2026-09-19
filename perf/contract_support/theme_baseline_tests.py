"""Issue #101 theme / style work-counter extractor and gate tests."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from .extractors import extract_nana
from .invariants import evaluate_invariants
from .reports import key_error_reason
from .schema import load_catalog, load_scenario


EXPECTED_THEME_IDS = {
    "theme-static-idle",
    "theme-controls-1",
    "theme-controls-100",
    "theme-controls-1k",
    "theme-controls-10k",
    "theme-hover-one",
    "theme-focus-one",
    "theme-palette-switch",
    "theme-accent-only",
    "theme-density",
    "theme-head-style-mutation",
}

PATHS = {"theme": Path("synthetic-theme-baseline")}


def _work(
    considered: int,
    resolved: int,
    skipped: int,
    *,
    theme_reads: int = 0,
    layout: int = 0,
    text: int = 0,
    paint: int = 0,
    allocations: int = 0,
    layout_copies: int = 0,
) -> dict[str, Any]:
    return {
        "style_nodes_considered": considered,
        "style_nodes_resolved": resolved,
        "style_nodes_skipped": skipped,
        "theme_reads": theme_reads,
        "style_allocations": allocations,
        "style_allocated_bytes": allocations * 208,
        "layout_copies": layout_copies,
        "layout_copied_bytes": layout_copies * 4808,
        "layout_nodes_from_style": layout,
        "text_nodes_from_style": text,
        "paint_nodes_from_style": paint,
    }


def _frame_work(
    *, style: int = 0, text: int = 0, layout: int = 0, render: int = 0, total: int = 1001
) -> dict[str, Any]:
    return {
        "entities_total": total,
        "style_processed": style,
        "text_shaped": text,
        "layout_nodes": layout,
        "render_nodes_changed": render,
    }


def _quiet_case(scenario: dict[str, Any]) -> dict[str, Any]:
    """A row that satisfies its own scenario's invariants.

    Built per workload rather than from one template: the point of these gates
    is that they say different things, so a single shape that passes them all
    would prove nothing.
    """
    params = scenario["params"]
    workload = params["workload"]
    controls = params.get("controls", 0)
    case: dict[str, Any] = {
        "id": scenario["id"],
        "workload": workload,
        "status": "ok",
        "controls": controls,
        "nodes": controls + 1,
        "iterations": 12,
        "elapsed_ms": {"p50": 0.1, "p95": 0.2, "max": 0.3},
    }
    if workload == "idle":
        case["work"] = _work(0, 0, 0)
        case["frame_work"] = _frame_work()
    elif workload == "controls":
        nodes = controls + 1
        case["work"] = _work(nodes, nodes, 0, theme_reads=nodes, text=controls, allocations=nodes)
        case["frame_work"] = _frame_work(
            style=nodes, text=controls, layout=nodes, render=nodes, total=nodes
        )
    elif workload in {"hover", "focus"}:
        case["work"] = _work(2, 0, 2, theme_reads=6, paint=1)
        case["frame_work"] = _frame_work(style=1, render=1)
    elif workload in {"palette-switch", "accent-only"}:
        case["work"] = _work(0, 0, 0, theme_reads=1001, paint=1001, allocations=1)
        case["frame_work"] = _frame_work(render=1001)
    elif workload == "density":
        case["work"] = _work(0, 0, 0, theme_reads=1001, layout=1001, paint=1001, allocations=1)
        case["frame_work"] = _frame_work(layout=1001, render=1001)
    else:  # head-scope
        case["work"] = _work(10_000, 10_000, 0, theme_reads=1, allocations=1)
        case["frame_work"] = _frame_work(style=10_000, render=10_000, total=10_000)
        case["nodes"] = 10_000
    return case


def _payload(cases: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "profile": "release",
        "catalog_theme": {"cases": cases},
    }


def _self_test_theme_baseline(root: Path) -> list[str]:
    errors: list[str] = []
    catalog = load_catalog(root)
    theme_ids = catalog.get("nana_theme_ids") or []
    if set(theme_ids) != EXPECTED_THEME_IDS:
        errors.append(
            f"catalog nana_theme_ids must be {sorted(EXPECTED_THEME_IDS)}, got {theme_ids}"
        )
    harness = set(catalog.get("harness_ids") or [])
    overlap = EXPECTED_THEME_IDS & harness
    if overlap:
        errors.append(f"nana_theme_ids must stay out of harness_ids: {sorted(overlap)}")

    for scenario_id in sorted(EXPECTED_THEME_IDS):
        scenario = load_scenario(scenario_id, root)
        case = _quiet_case(scenario)
        report = extract_nana(scenario, {"theme": _payload([case])}, source_paths=PATHS)
        failed = [row for row in evaluate_invariants(scenario, report) if row.get("status") != "ok"]
        if failed:
            errors.append(f"{scenario_id} invariants must pass on its own baseline: {failed}")

        # The theme counters and the #8 frame counters share one object, and
        # both must reach the report: a scenario cross-checking
        # style_nodes_considered against style_processed needs both present.
        counters = report.get("work_counters") or {}
        for key in ("style_nodes_considered", "theme_reads", "style_processed", "layout_nodes"):
            if key not in counters:
                errors.append(f"{scenario_id} work_counters must carry {key}")

    # A palette change that copies a box is the regression this row exists to
    # catch: resolving sizing on the read path leaves every invalidation
    # counter at 0 while rebuilding LayoutStyle for the whole document. The
    # quiet baseline copies nothing, so the gate has to be shown failing here
    # or it would pass for the wrong reason.
    for scenario_id in ("theme-palette-switch", "theme-accent-only"):
        scenario = load_scenario(scenario_id, root)
        copied = _quiet_case(scenario)
        copied["work"]["layout_copies"] = 1
        copied["work"]["layout_copied_bytes"] = 4808
        report = extract_nana(scenario, {"theme": _payload([copied])}, source_paths=PATHS)
        fired = [
            row
            for row in evaluate_invariants(scenario, report)
            if row.get("name", "").endswith("does_not_copy_layout")
            and row.get("status") != "ok"
        ]
        if not fired:
            errors.append(
                f"{scenario_id} must fail its layout-copy invariant when a box is copied"
            )

    # A report for another workload is not this row's report.
    idle = load_scenario("theme-static-idle", root)
    mismatched = _quiet_case(idle)
    mismatched["workload"] = "hover"
    try:
        extract_nana(idle, {"theme": _payload([mismatched])}, source_paths=PATHS)
        errors.append("theme extract must reject a case whose workload does not match")
    except KeyError as exc:
        if "workload" not in key_error_reason(exc):
            errors.append(f"workload KeyError should name workload: {exc}")

    # A scale row that does not echo its scale cannot be compared with itself.
    scale = load_scenario("theme-controls-1k", root)
    wrong_scale = _quiet_case(scale)
    wrong_scale["controls"] = 100
    try:
        extract_nana(scale, {"theme": _payload([wrong_scale])}, source_paths=PATHS)
        errors.append("theme extract must reject a controls row with the wrong scale")
    except KeyError as exc:
        if "controls" not in key_error_reason(exc):
            errors.append(f"controls KeyError should name controls: {exc}")

    # considered == resolved + skipped is the invariant that makes the three
    # numbers mean anything. A report that breaks it is not a slow row, it is
    # a miscounted one.
    broken = _quiet_case(scale)
    broken["work"]["style_nodes_skipped"] += 1
    try:
        extract_nana(scale, {"theme": _payload([broken])}, source_paths=PATHS)
        errors.append("theme extract must reject considered != resolved + skipped")
    except KeyError as exc:
        if "style_nodes_considered" not in key_error_reason(exc):
            errors.append(f"split KeyError should name style_nodes_considered: {exc}")

    # A missing counter is not a zero counter.
    for section, key in (("work", "theme_reads"), ("frame_work", "layout_nodes")):
        blind = _quiet_case(scale)
        del blind[section][key]
        try:
            extract_nana(scale, {"theme": _payload([blind])}, source_paths=PATHS)
            errors.append(f"theme extract must reject a missing {section}.{key}")
        except KeyError as exc:
            if key not in key_error_reason(exc):
                errors.append(f"missing-{key} KeyError should name it: {exc}")

    # Each gate has to be load-bearing on its own.
    for scenario_id, counter, blown, what in (
        ("theme-static-idle", "style_nodes_considered", 1, "a settled frame re-resolved"),
        ("theme-static-idle", "theme_reads", 1, "a settled frame read the theme"),
        ("theme-hover-one", "style_nodes_considered", 512, "hover became document-wide"),
        ("theme-palette-switch", "text_nodes_from_style", 1, "a palette switch reshaped"),
        ("theme-palette-switch", "layout_nodes_from_style", 1, "a palette switch laid out"),
        ("theme-density", "layout_nodes_from_style", 0, "a metrics change missed layout"),
        ("theme-density", "text_nodes_from_style", 1, "a metrics change reshaped"),
    ):
        scenario = load_scenario(scenario_id, root)
        noisy = _quiet_case(scenario)
        noisy["work"][counter] = blown
        # Keep the split invariant true so the extractor judges the gate, not
        # the arithmetic.
        noisy["work"]["style_nodes_considered"] = (
            noisy["work"]["style_nodes_resolved"] + noisy["work"]["style_nodes_skipped"]
            if counter != "style_nodes_considered"
            else blown
        )
        if counter == "style_nodes_considered":
            noisy["work"]["style_nodes_resolved"] = blown
            noisy["work"]["style_nodes_skipped"] = 0
        loud = extract_nana(scenario, {"theme": _payload([noisy])}, source_paths=PATHS)
        if all(row.get("status") == "ok" for row in evaluate_invariants(scenario, loud)):
            errors.append(f"{scenario_id} must fail on {what} ({counter}={blown})")

    return errors
