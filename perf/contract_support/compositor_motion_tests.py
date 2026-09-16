"""Issue #87 compositor work-counter extractor / gate tests."""
from __future__ import annotations

from pathlib import Path
from typing import Any
from .extractors import extract_nana
from .invariants import evaluate_invariants
from .reports import key_error_reason
from .schema import load_catalog, load_scenario




def _quiet_work(tracks: int) -> dict[str, Any]:
    return {
        "motion_tracks_active": tracks,
        "motion_tracks_cpu": 0,
        "motion_tracks_compositor": tracks,
        "presentation_values_cpu_sampled": 0,
        "compositor_layers_active": tracks,
        "compositor_layers_promoted": 0,
        "compositor_layers_demoted": 0,
        "compositor_cache_bytes": 0,
        "uiworld_mutations_from_animation": 0,
        "layout_nodes_from_animation": 0,
        "style_processed_from_animation": 0,
        "render_nodes_reextracted_from_animation": 0,
        "animations_considered": 0,
        "animation_deadlines_scanned": 0,
    }


def _catalog(tracks: int = 1) -> dict[str, Any]:
    work = _quiet_work(tracks)
    scale_rows = []
    for count in (1, 100, 1000, 10000):
        for properties in ("transform", "opacity", "mixed"):
            row_tracks = count if properties != "mixed" else count
            scale_rows.append(
                {
                    "id": "compositor-tracks",
                    "kind": "Animation",
                    "status": "ok",
                    "tracks": row_tracks,
                    "properties": properties,
                    "workload": "scale",
                    "work": _quiet_work(row_tracks),
                    "steady_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
                }
            )
    return {
        "schema_version": 1,
        "phase": "issue-87-compositor",
        "catalog_compositor": {
            "steady": {
                "id": "compositor-steady",
                "kind": "Animation",
                "status": "ok",
                "tracks": 1,
                "properties": "opacity",
                "workload": "steady",
                "work": work,
                "steady_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
            },
            "scales": scale_rows,
            "retarget": {
                "id": "compositor-retarget",
                "kind": "Animation",
                "status": "ok",
                "tracks": 100,
                "properties": "opacity",
                "workload": "retarget",
                "work": _quiet_work(100),
                "retargets": 100,
                "steady_ms": {"p50": 0.2, "p95": 0.3, "p99": 0.4},
            },
            "churn": {
                "id": "compositor-churn",
                "kind": "Animation",
                "status": "ok",
                "tracks": 100,
                "properties": "opacity",
                "workload": "churn",
                "work": _quiet_work(100),
                "start_stop": 100,
                "steady_ms": {"p50": 0.2, "p95": 0.3, "p99": 0.4},
            },
        },
    }


def _self_test_compositor_motion(root: Path) -> list[str]:
    errors: list[str] = []
    catalog = load_catalog(root)
    motion_ids = catalog.get("nana_motion_ids") or []
    expected = {
        "compositor-steady",
        "compositor-tracks-1",
        "compositor-tracks-100",
        "compositor-tracks-1k",
        "compositor-tracks-10k",
        "compositor-retarget",
        "compositor-churn",
    }
    if set(motion_ids) != expected:
        errors.append(f"catalog nana_motion_ids must be {sorted(expected)}, got {motion_ids}")
    harness = set(catalog.get("harness_ids") or [])
    overlap = expected & harness
    if overlap:
        errors.append(f"nana_motion_ids must stay out of harness_ids: {sorted(overlap)}")

    dump = _catalog()
    reports = {"scene_compositor": dump}
    paths = {"scene_compositor": Path("synthetic-compositor")}
    for scenario_id in sorted(expected):
        scenario = load_scenario(scenario_id, root)
        try:
            envelope = extract_nana(scenario, reports, source_paths=paths)
        except KeyError as exc:
            errors.append(f"{scenario_id} extract failed: {exc}")
            continue
        if envelope.get("status") != "ok":
            errors.append(f"{scenario_id} extract status={envelope.get('status')}")
            continue
        rows = evaluate_invariants(scenario, envelope)
        failed = [row for row in rows if row.get("status") != "ok"]
        if failed:
            errors.append(f"{scenario_id} structural invariants must pass on quiet work: {failed}")

    dirty = _catalog()
    dirty["catalog_compositor"]["steady"]["work"]["uiworld_mutations_from_animation"] = 4
    dirty_env = extract_nana(
        load_scenario("compositor-steady", root),
        {"scene_compositor": dirty},
        source_paths=paths,
    )
    dirty_rows = evaluate_invariants(load_scenario("compositor-steady", root), dirty_env)
    if all(row.get("status") == "ok" for row in dirty_rows):
        errors.append("compositor-steady must fail when uiworld_mutations_from_animation=4")

    gpu_fake = _catalog()
    gpu_fake["catalog_compositor"]["steady"]["work"]["motion_descriptors_uploaded"] = 0
    try:
        extract_nana(
            load_scenario("compositor-steady", root),
            {"scene_compositor": gpu_fake},
            source_paths=paths,
        )
        errors.append("compositor extract must KeyError invented motion_descriptors_uploaded")
    except KeyError as exc:
        if "motion_descriptors_uploaded" not in key_error_reason(exc):
            errors.append(f"GPU-upload KeyError should name motion_descriptors_uploaded: {exc}")

    animation = load_scenario("animation", root)
    try:
        extract_nana(animation, reports, source_paths=paths)
        errors.append("animation extract must not treat catalog_compositor as catalog_animation")
    except KeyError as exc:
        reason = key_error_reason(exc)
        if "catalog_animation" not in reason and "nana-runtime-benchmark" not in reason:
            errors.append(f"animation KeyError should name catalog_animation: {exc}")

    return errors
