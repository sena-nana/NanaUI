"""Executable contract regression cases."""
from __future__ import annotations

import copy
from pathlib import Path
from typing import Any
from .extractors import (
    _named_invariant,
    _named_invariant_status,
    extract_iced,
    extract_nana,
)
from .invariants import (
    judge_runner_invariants,
)
from .reports import (
    key_error_reason,
)
from .schema import (
    load_catalog,
    load_json,
    load_scenario,
)




def _self_test_catalog_workloads(
    root: Path,
    runtime_path: Path,
    iced_path: Path,
    reserved: set[Any],
) -> list[str]:
    errors: list[str] = []
    harness = set(load_catalog(root).get("harness_ids", []))
    wirable = ("animation", "ime", "dock-workspace", "overlay", "text-editor", "gpu-scene-ui")
    for scenario_id in wirable:
        if scenario_id not in harness:
            errors.append(f"catalog must list wirable {scenario_id} in harness_ids")
        if scenario_id in reserved:
            errors.append(
                f"catalog must not leave wirable {scenario_id} in required_by_issue_not_in_harness"
            )
    for live2d_id in ("gpu-scene-ui-live2d", "gpu-scene-ui-live2d-effect"):
        if live2d_id in harness:
            errors.append(f"catalog must not list {live2d_id} in harness_ids")
        if live2d_id not in reserved:
            errors.append(
                f"catalog must keep {live2d_id} in required_by_issue_not_in_harness"
            )

    animation = load_scenario("animation", root)
    try:
        runtime_without_catalog = copy.deepcopy(load_json(runtime_path))
        if isinstance(runtime_without_catalog, dict):
            runtime_without_catalog.pop("catalog_animation", None)
        extract_nana(
            animation,
            {"runtime": runtime_without_catalog},
            source_paths={"runtime": Path("synthetic-runtime-without-catalog-animation")},
        )
        errors.append(
            "nana animation extract without catalog_animation must KeyError"
        )
    except KeyError as exc:
        if "catalog_animation" not in key_error_reason(exc):
            errors.append(f"animation KeyError should name catalog_animation: {exc}")

    try:
        extract_nana(
            animation,
            {
                "runtime": {
                    "cases": [
                        {
                            "nodes": 5000,
                            "sparse_animation_sample_ms": {"p50": 0.1},
                            "due_animation_samples": 1,
                            "idle_animation_deadline_ms": {"p50": 0.01},
                            "scheduled_animation_deadline_ms": {"p50": 0.02},
                            "scheduled_animations": 5000,
                        }
                    ]
                }
            },
            source_paths={"runtime": Path("synthetic-legacy-5k-animation")},
        )
        errors.append(
            "nana animation must not treat 5k-tree sparse sample as catalog_animation"
        )
    except KeyError as exc:
        if "catalog_animation" not in key_error_reason(exc):
            errors.append(f"animation KeyError should name catalog_animation: {exc}")

    try:
        extract_nana(
            animation,
            {
                "runtime": {
                    "catalog_animation": {
                        "id": "animation",
                        "status": "ok",
                        "active": 1,
                        "scheduled_idle": True,
                        "scheduled_animations": 64,
                        "due_animation_samples": 1,
                        "idle_animation_deadline_ms": {"p50": 0.01},
                        "sparse_animation_sample_ms": {"p50": 0.1},
                    }
                }
            },
            source_paths={"runtime": Path("catalog-animation-missing-work")},
        )
        errors.append(
            "nana animation extract must KeyError without work.animations_considered"
        )
    except KeyError as exc:
        if "animations_considered" not in key_error_reason(exc):
            errors.append(
                f"animation missing-work KeyError should name animations_considered: {exc}"
            )

    fixture = load_json(root / "perf" / "fixtures" / "catalog-workloads.json")
    reports = {"framework": fixture}
    paths = {"framework": Path("catalog-workloads.json")}
    ime = extract_nana(load_scenario("ime", root), reports, source_paths=paths)
    if ime.get("status") != "ok":
        errors.append("nana ime extract must be ok when catalog_workloads ime status=ok")
    if (ime.get("work_counters") or {}).get("ime_script_count") != 4:
        errors.append("ime envelope must copy ime_script_count=4")
    dock = extract_nana(load_scenario("dock-workspace", root), reports, source_paths=paths)
    if dock.get("status") != "ok":
        errors.append("nana dock-workspace extract must be ok when catalog_workloads status=ok")
    if (dock.get("work_counters") or {}).get("panes") != 8:
        errors.append("dock-workspace envelope must copy panes=8")
    overlay = extract_nana(load_scenario("overlay", root), reports, source_paths=paths)
    if overlay.get("status") != "ok":
        errors.append("nana overlay extract must be ok when catalog_workloads status=ok")
    if (overlay.get("work_counters") or {}).get("overlay_kind_count") != 4:
        errors.append("overlay envelope must copy overlay_kind_count=4")
    editor = extract_nana(load_scenario("text-editor", root), reports, source_paths=paths)
    if editor.get("status") != "ok":
        errors.append("nana text-editor extract must be ok when catalog_workloads status=ok")
    if (editor.get("work_counters") or {}).get("text_shaped") != 1:
        errors.append("text-editor envelope must export text_shaped=1")
    if (editor.get("work_counters") or {}).get("layout_nodes") != 0:
        errors.append("text-editor extractor fixture must copy layout_nodes=0")
    if _named_invariant_status(editor, "text_editor_does_not_layout_full_tree") != "ok":
        errors.append("text-editor local-edit layout_nodes==0 invariant must be ok")
    if _named_invariant(editor, "text_editor_local_edit_shapes_bounded_nodes") is not None:
        errors.append("text-editor must not use text_shaped<=1 as the §8.1 pass")

    try:
        extract_nana(
            load_scenario("ime", root),
            {
                "framework": {
                    "catalog_workloads": [
                        {"id": "ime", "kind": "Ime", "status": "skipped", "skip_reason": "missing"}
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-ime-skipped")},
        )
        errors.append("nana ime skipped catalog_workloads must KeyError, not ok")
    except KeyError as exc:
        if "ime" not in key_error_reason(exc):
            errors.append(f"ime skip KeyError should name ime: {exc}")

    for scenario_id, field in (
        ("ime", "ime_script_count"),
        ("overlay", "overlay_kind_count"),
    ):
        missing_payload = copy.deepcopy(fixture)
        for row in missing_payload.get("catalog_workloads") or []:
            if row.get("id") == scenario_id:
                row.pop(field, None)
        try:
            extract_nana(
                load_scenario(scenario_id, root),
                {"framework": missing_payload},
                source_paths={"framework": Path(f"synthetic-{scenario_id}-missing-{field}")},
            )
            errors.append(
                f"nana {scenario_id} missing {field} must KeyError, "
                "not invent a count from len()"
            )
        except KeyError as exc:
            if field not in key_error_reason(exc):
                errors.append(f"{scenario_id} missing-field KeyError should name {field}: {exc}")

    ime_wrong = copy.deepcopy(fixture)
    for row in ime_wrong.get("catalog_workloads") or []:
        if row.get("id") == "ime":
            row["ime_script_count"] = 3
    wrong_ime = extract_nana(
        load_scenario("ime", root),
        {"framework": ime_wrong},
        source_paths={"framework": Path("synthetic-ime-wrong-count")},
    )
    if (wrong_ime.get("work_counters") or {}).get("ime_script_count") != 3:
        errors.append("ime envelope must copy the explicit ime_script_count=3")
    if judge_runner_invariants(wrong_ime, root=root).get("decision") != "ok":
        errors.append("ime_script_count=3 must not be the §8.1 pass when layout_nodes==0")
    overlay_wrong = copy.deepcopy(fixture)
    for row in overlay_wrong.get("catalog_workloads") or []:
        if row.get("id") == "overlay":
            row["overlay_kind_count"] = 3
    wrong_overlay = extract_nana(
        load_scenario("overlay", root),
        {"framework": overlay_wrong},
        source_paths={"framework": Path("synthetic-overlay-wrong-count")},
    )
    if (wrong_overlay.get("work_counters") or {}).get("overlay_kind_count") != 3:
        errors.append("overlay envelope must copy the explicit overlay_kind_count=3")
    if judge_runner_invariants(wrong_overlay, root=root).get("decision") != "ok":
        errors.append("overlay_kind_count=3 must not be the §8.1 pass when dirty caps hold")

    for scenario_id in wirable:
        try:
            extract_iced(
                load_scenario(scenario_id, root),
                load_json(iced_path),
                source_path=iced_path,
            )
            errors.append(f"iced {scenario_id} extract must be unsupported")
        except KeyError:
            pass
    return errors
