"""Issue #98 retained-text work-counter extractor / gate tests."""
from __future__ import annotations

from pathlib import Path
from typing import Any
from .extractors import extract_nana
from .invariants import evaluate_invariants
from .reports import key_error_reason
from .schema import load_catalog, load_scenario


EXPECTED_TEXT_IDS = {"gpu-scene-text-retained"}


def _quiet_text() -> dict[str, Any]:
    """What one frame beside a ticking label owes: that label and nothing else."""
    return {
        "glyph_resolve_requests": 6.0,
        "glyph_rasterized": 1.0,
        "glyph_upload_bytes": 112.0,
        "text_instance_rebuilds": 1.0,
        "text_instance_upload_bytes": 240.0,
        "text_prepare_nodes_considered": 1000.0,
        "text_prepare_nodes_skipped": 999.0,
        "text_gpu_entries_active": 1000.0,
    }


def _payload(scenario: dict[str, Any], text: dict[str, Any] | None) -> dict[str, Any]:
    params = scenario["params"]
    report: dict[str, Any] = {
        "schema_version": 1,
        "status": "ok",
        "scenario_id": scenario["id"],
        "composition": "UiOnly",
        "materialization": {
            "viewport": params["viewport"],
            "host_texture": params["host_texture"],
            "ui_nodes": params["ui_nodes"],
            "node_repeat": params.get("node_repeat") or {},
            "shared_gpu_view_slot": bool(params.get("shared_gpu_view_slot")),
            "text_ticker": bool(params.get("text_ticker")),
            "ui_entity_count": 1002,
            "host_texture_resources": 1,
            "scene_primitive_kinds": ["host-texture", "quad", "text"],
        },
        "adapter": "synthetic",
        "frames": 20,
        "gpu_work": {
            "batch_rebuilds": 0,
            "draw_batches": 3,
            "draw_calls": 3,
            "gpu_upload_bytes": 872,
            "gpu_buffer_reallocations": 0,
        },
        "frame_stages": {
            name: {"status": "ran"} for name in ("Batch", "GpuUpload", "Encode", "Submit")
        },
        "stages": {
            name: {"p50": 0.5, "p95": 0.6, "p99": 0.7, "max": 0.7}
            for name in ("batch_ms", "gpu_upload_ms", "encode_ms", "submit_ms")
        },
    }
    if text is not None:
        report["text_counters"] = text
    return report


def _self_test_retained_text(root: Path) -> list[str]:
    errors: list[str] = []
    catalog = load_catalog(root)
    text_ids = catalog.get("nana_text_ids") or []
    if set(text_ids) != EXPECTED_TEXT_IDS:
        errors.append(
            f"catalog nana_text_ids must be {sorted(EXPECTED_TEXT_IDS)}, got {text_ids}"
        )
    harness = set(catalog.get("harness_ids") or [])
    overlap = EXPECTED_TEXT_IDS & harness
    if overlap:
        errors.append(f"nana_text_ids must stay out of harness_ids: {sorted(overlap)}")

    paths = {"gpu": Path("synthetic-retained-text")}
    for scenario_id in sorted(EXPECTED_TEXT_IDS):
        scenario = load_scenario(scenario_id, root)
        if not scenario["params"].get("text_ticker"):
            errors.append(
                f"{scenario_id} must set params.text_ticker: a frame the painter "
                "answers from its prepared batch cannot fail a text gate"
            )
        quiet = extract_nana(
            scenario,
            {"gpu": _payload(scenario, _quiet_text())},
            source_paths=paths,
        )
        rows = evaluate_invariants(scenario, quiet)
        failed = [row for row in rows if row.get("status") != "ok"]
        if failed:
            errors.append(
                f"{scenario_id} invariants must pass on a retained frame: {failed}"
            )

        # Every budget has to be load-bearing on its own: a gate that only
        # fails when several counters blow at once is a gate with spare
        # invariants in it.
        for counter, blown, what in (
            ("text_instance_rebuilds", 1000.0, "every paragraph resolved again"),
            ("text_instance_upload_bytes", 120_000.0, "the whole arena rewritten"),
            ("glyph_rasterized", 500.0, "every glyph rasterized again"),
            ("glyph_upload_bytes", 500_000.0, "the atlas reuploaded"),
            ("text_prepare_nodes_skipped", 0.0, "no node answered by its entry"),
        ):
            noisy = _quiet_text()
            noisy[counter] = blown
            loud = extract_nana(
                scenario,
                {"gpu": _payload(scenario, noisy)},
                source_paths=paths,
            )
            if all(
                row.get("status") == "ok" for row in evaluate_invariants(scenario, loud)
            ):
                errors.append(f"{scenario_id} must fail on {what} ({counter}={blown})")

        # A missing block is not a passing block.
        blind = extract_nana(
            scenario,
            {"gpu": _payload(scenario, None)},
            source_paths=paths,
        )
        if "text_counters" in blind:
            errors.append(f"{scenario_id} must not invent text_counters")
        if all(
            row.get("status") == "ok" for row in evaluate_invariants(scenario, blind)
        ):
            errors.append(
                f"{scenario_id} must not read missing text_counters as satisfied"
            )

        # The runner has to prove it ran the ticking scene.
        silent = _payload(scenario, _quiet_text())
        silent["materialization"]["text_ticker"] = False
        try:
            extract_nana(scenario, {"gpu": silent}, source_paths=paths)
            errors.append(
                f"{scenario_id} extract must reject a report that did not tick a label"
            )
        except KeyError as exc:
            if "text_ticker" not in key_error_reason(exc):
                errors.append(f"ticker KeyError should name text_ticker: {exc}")

    return errors
