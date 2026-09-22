"""Issue #98 / #99 retained-text work-counter extractor / gate tests: the
ticker (static steady), paint-only, compositor-only and constraint-only rows."""
from __future__ import annotations

from pathlib import Path
from typing import Any
from .extractors import extract_nana
from .invariants import evaluate_invariants, judge_runner_invariants
from .reports import key_error_reason
from .schema import load_catalog, load_scenario


TICKER_ID = "gpu-scene-text-retained"
ANIMATED_IDS = {
    "gpu-scene-text-paint-color": "color",
    "gpu-scene-text-compositor-opacity": "opacity",
    "gpu-scene-text-compositor-transform": "transform",
    "gpu-scene-text-constraint-resize": "resize",
}
RESIZE_ID = "gpu-scene-text-constraint-resize"
EXPECTED_TEXT_IDS = {TICKER_ID, *ANIMATED_IDS}


def _quiet_text(scenario_id: str = TICKER_ID) -> dict[str, Any]:
    """What one retained frame owes.

    Beside a ticking label: that label and nothing else. Under a recolor, a
    fade or a turn: nothing at all — the text is what it was. Under a resize:
    a new layout for every label, and none of it shaped.
    """
    if scenario_id == RESIZE_ID:
        return {
            "glyph_resolve_requests": 6000.0,
            "glyph_rasterized": 0.0,
            "glyph_upload_bytes": 0.0,
            "text_instance_rebuilds": 400.0,
            "text_instance_upload_bytes": 190000.0,
            "text_prepare_nodes_considered": 1000.0,
            "text_prepare_nodes_skipped": 0.0,
            "text_gpu_entries_active": 400.0,
            "text_nodes_shaped": 0.0,
            "text_layouts_created": 0.0,
            "text_layout_lookups": 1000.0,
            "text_constraint_only_relayouts": 0.0,
            "text_layouts_reshaped": 0.0,
            "paint_shape_cache_misses": 0.0,
        }
    if scenario_id == TICKER_ID:
        return {
            "glyph_resolve_requests": 6.0,
            "glyph_rasterized": 1.0,
            "glyph_upload_bytes": 112.0,
            "text_instance_rebuilds": 1.0,
            "text_instance_upload_bytes": 240.0,
            "text_prepare_nodes_considered": 1000.0,
            "text_prepare_nodes_skipped": 999.0,
            "text_gpu_entries_active": 1000.0,
            "text_nodes_shaped": 1.0,
            "text_layouts_created": 1.0,
            "paint_shape_cache_misses": 0.0,
        }
    return {
        "glyph_resolve_requests": 0.0,
        "glyph_rasterized": 0.0,
        "glyph_upload_bytes": 0.0,
        "text_instance_rebuilds": 0.0,
        "text_instance_upload_bytes": 0.0,
        "text_prepare_nodes_considered": 1000.0,
        "text_prepare_nodes_skipped": 1000.0,
        "text_gpu_entries_active": 1000.0,
        "text_nodes_shaped": 0.0,
        "text_layouts_created": 0.0,
        "paint_shape_cache_misses": 0.0,
    }


def _blown(invariant: dict[str, Any]) -> float:
    """A value that breaks `invariant` by a clear margin."""
    value = float(invariant["value"])
    if invariant["op"] == "lte":
        return value + max(1.0, value) * 10.0
    if invariant["op"] == "gte":
        return 0.0 if value > 0 else -1.0
    raise ValueError(f"unexpected op {invariant['op']!r} in {invariant['name']}")


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
    if params.get("text_animation") is not None:
        report["materialization"]["text_animation"] = params["text_animation"]
    if text is not None:
        report["text_counters"] = text
    return report


def _decision(report: dict[str, Any], root: Path) -> str | None:
    """What `--evaluate-invariants` makes of `report`."""
    return judge_runner_invariants({**report, "runner": "nana"}, root=root).get("decision")


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
        params = scenario["params"]
        if scenario_id == TICKER_ID and not params.get("text_ticker"):
            errors.append(
                f"{scenario_id} must set params.text_ticker: a frame the painter "
                "answers from its prepared batch cannot fail a text gate"
            )
        if scenario_id in ANIMATED_IDS and params.get("text_animation") != ANIMATED_IDS[
            scenario_id
        ]:
            errors.append(
                f"{scenario_id} must animate {ANIMATED_IDS[scenario_id]!r}, got "
                f"{params.get('text_animation')!r}"
            )
        quiet = extract_nana(
            scenario,
            {"gpu": _payload(scenario, _quiet_text(scenario_id))},
            source_paths=paths,
        )
        rows = evaluate_invariants(scenario, quiet)
        failed = [row for row in rows if row.get("status") != "ok"]
        if failed:
            errors.append(
                f"{scenario_id} invariants must pass on a retained frame: {failed}"
            )
        # `--evaluate-invariants` is what CI runs on the report. It has to
        # judge this id, not wave it through as "not a §8.1 id".
        if _decision(quiet, root) != "ok":
            errors.append(f"{scenario_id} --evaluate-invariants must judge a retained frame ok")
        # The runner writes what `extract_nana` returns, so the counters have to
        # be in the payload the invariants were evaluated against — not stapled
        # to the report afterwards. Otherwise every text gate reads
        # `not-evaluable` on a real run, which is a gate that cannot fail.
        attached = quiet.get("invariants") or []
        text_rows = [
            row for row in attached if str(row.get("path", "")).startswith("text_counters.")
        ]
        if len(text_rows) != len(rows):
            errors.append(
                f"{scenario_id} must evaluate its text invariants in the report "
                f"it returns, got {len(text_rows)} of {len(rows)}"
            )
        blind_rows = [row for row in text_rows if row.get("status") == "not-evaluable"]
        if blind_rows:
            errors.append(
                f"{scenario_id} attached text invariants must be evaluated, got "
                f"{[row.get('name') for row in blind_rows]}"
            )

        # Every budget has to be load-bearing on its own: a gate that only
        # fails when several counters blow at once is a gate with spare
        # invariants in it.
        for invariant in scenario.get("invariants") or []:
            counter = str(invariant["path"]).removeprefix("text_counters.")
            if counter == str(invariant["path"]):
                continue
            noisy = _quiet_text(scenario_id)
            if counter not in noisy:
                errors.append(
                    f"{scenario_id} gates {counter}, which the quiet frame of this "
                    "self-test does not carry"
                )
                continue
            noisy[counter] = _blown(invariant)
            loud = extract_nana(
                scenario,
                {"gpu": _payload(scenario, noisy)},
                source_paths=paths,
            )
            if all(
                row.get("status") == "ok" for row in evaluate_invariants(scenario, loud)
            ):
                errors.append(
                    f"{scenario_id} must fail {invariant['name']} on "
                    f"{counter}={noisy[counter]}"
                )
            if _decision(loud, root) != "failed":
                errors.append(
                    f"{scenario_id} --evaluate-invariants must fail {invariant['name']}"
                )
        if scenario_id == TICKER_ID:
            gated = {
                str(row["path"]).removeprefix("text_counters.")
                for row in scenario.get("invariants") or []
            }
            for counter in (
                "text_instance_rebuilds",
                "text_instance_upload_bytes",
                "glyph_rasterized",
                "glyph_upload_bytes",
                "text_prepare_nodes_skipped",
            ):
                if counter not in gated:
                    errors.append(f"{scenario_id} must gate {counter}")
        elif scenario_id == RESIZE_ID:
            gated = {
                str(row["path"]).removeprefix("text_counters.")
                for row in scenario.get("invariants") or []
            }
            # The #99 constraint-only gate, by name, and the row that proves the
            # width change reached the text at all.
            for counter in (
                "text_nodes_shaped",
                "text_layouts_reshaped",
                "text_layout_lookups",
                "paint_shape_cache_misses",
                "glyph_rasterized",
                "glyph_upload_bytes",
            ):
                if counter not in gated:
                    errors.append(f"{scenario_id} must gate {counter}")
        else:
            gated = {
                str(row["path"]).removeprefix("text_counters.")
                for row in scenario.get("invariants") or []
            }
            # The #98 paint-only and compositor-only gates, by name.
            for counter in (
                "text_nodes_shaped",
                "text_layouts_created",
                "paint_shape_cache_misses",
                "glyph_rasterized",
                "glyph_upload_bytes",
                "text_instance_rebuilds",
            ):
                if counter not in gated:
                    errors.append(f"{scenario_id} must gate {counter}")

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
        if _decision(blind, root) == "ok":
            errors.append(f"{scenario_id} --evaluate-invariants must not pass without text_counters")

        # The runner has to prove it ran the moving scene.
        silent = _payload(scenario, _quiet_text(scenario_id))
        if scenario_id == TICKER_ID:
            silent["materialization"]["text_ticker"] = False
            echo = "text_ticker"
        else:
            silent["materialization"].pop("text_animation", None)
            echo = "text_animation"
        try:
            extract_nana(scenario, {"gpu": silent}, source_paths=paths)
            errors.append(
                f"{scenario_id} extract must reject a report that did not move its labels"
            )
        except KeyError as exc:
            if echo not in key_error_reason(exc):
                errors.append(f"{scenario_id} KeyError should name {echo}: {exc}")

    # Both workflows judge every text gate: the PR job on the reports recorded
    # on real hardware, the weekly macOS job on a live run.
    pr_ci = (root / ".github" / "workflows" / "ci.yml").read_text()
    weekly = (root / ".github" / "workflows" / "runtime-performance.yml").read_text()
    jobs = {
        "ci.yml": pr_ci.partition("issue8/text")[2],
        "weekly macOS": weekly.split("macos-composition:", 1)[-1].partition("issue8/text")[2],
    }
    for scenario_id in sorted(EXPECTED_TEXT_IDS):
        if not (root / "perf" / "fixtures" / f"nana-{scenario_id}.json").is_file():
            errors.append(f"perf/fixtures/nana-{scenario_id}.json is missing")
        for name, job in jobs.items():
            if scenario_id not in job or "--evaluate-invariants target/performance/issue8/text" not in job:
                errors.append(f"{name} must run and judge {scenario_id}")

    return errors
