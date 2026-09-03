"""Executable contract regression cases."""
from __future__ import annotations

import copy
from pathlib import Path
from typing import Any, Mapping
from .catalog_workloads_tests import (
    _self_test_catalog_workloads,
)
from .comparison import (
    relative_gate_can_enforce,
)
from .extractors import (
    _extract_nana_framework_fixture,
    _named_invariant,
    _named_invariant_ok_measured,
    _require_same_scenario,
    _synthetic_gpu_ui_only,
    extract_iced,
    extract_nana,
    gpui_unsupported,
)
from .from_report_cli_tests import (
    _self_test_from_report_cli,
)
from .invariants import (
    judge_runner_invariants,
)
from .reports import (
    key_error_reason,
)
from .schema import (
    GPU_WORK_COUNTER_KEYS,
    REPO_ROOT,
    STATIC_TREE_GENERATION,
    STATIC_TREE_NODE_KIND,
    STATIC_TREE_PARENT_RULE,
    TEXT_TABLE_CACHE_GAPS,
    TEXT_TABLE_EXPORTED_SHAPE_KEYS,
    is_shared_static_tree,
    load_catalog,
    load_json,
    load_scenario,
    static_tree_children,
    static_tree_parent,
    static_tree_sample_parents,
    validate_all_scenarios,
)
from .section_8_1_runner_invariants_tests import (
    _self_test_section_8_1_runner_invariants,
)




def self_test(root: Path | None = None) -> list[str]:
    """Validate schema and extractors against checked-in fixtures."""
    root = root or REPO_ROOT
    errors = validate_all_scenarios(root)
    runtime_path = root / "perf" / "fixtures" / "nana-runtime-static-tree.json"
    framework_scales_path = root / "perf" / "fixtures" / "virtual-scales-only.json"
    iced_path = root / "perf" / "fixtures" / "iced-scenario-static-tree-100.json"
    gallery_list_100 = {
        "cases": [
            {
                "scenario": "list-100",
                "cpu_total_ms": {"p50": 1.0, "p95": 1.1, "p99": 1.2},
                "total_ms": {"p50": 1.0, "p95": 1.1, "p99": 1.2},
                "view_construction_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
                "layout_diff_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
            }
        ]
    }

    static_tree = load_scenario("static-tree-100", root)
    try:
        nana_static_fixture = extract_nana(
            static_tree,
            {"runtime": load_json(runtime_path)},
            source_paths={"runtime": runtime_path},
        )
        if nana_static_fixture.get("status") != "ok":
            errors.append("nana static-tree-100 fixture extract did not return ok")
        if (nana_static_fixture.get("metrics") or {}).get("frames_after_idle") != 0:
            errors.append("nana-runtime-static-tree fixture must export frames_after_idle=0")
        runtime_missing_idle = copy.deepcopy(load_json(runtime_path))
        for case in runtime_missing_idle.get("cases") or []:
            if isinstance(case, dict):
                case.pop("frames_after_idle", None)
        nana_missing_idle = extract_nana(
            static_tree,
            {"runtime": runtime_missing_idle},
            source_paths={"runtime": Path("synthetic-runtime-without-idle")},
        )
        if "frames_after_idle" in (nana_missing_idle.get("metrics") or {}):
            errors.append(
                "extractor must not invent frames_after_idle when the dump omits it"
            )
        missing_idle_judge = judge_runner_invariants(nana_missing_idle, root=root)
        if missing_idle_judge.get("decision") != "skipped":
            errors.append(
                "StaticTree without frames_after_idle must stay skipped, not vacuous ok"
            )
    except Exception as exc:  # noqa: BLE001 — self-test must surface mapper failures
        errors.append(f"nana static-tree-100 extract failed: {exc}")

    iced_static = extract_iced(
        static_tree, gallery_list_100, source_path=Path("synthetic-gallery-list-100")
    )
    if iced_static.get("status") != "ok":
        errors.append("iced static-tree-100 extract did not return ok")
    if iced_static.get("equivalence") != "closest-legacy-reference":
        errors.append("Gallery ui-benchmark extract must stay closest-legacy-reference")

    virtual = load_scenario("virtual-list-10k", root)
    try:
        nana_virtual = extract_nana(
            virtual,
            {"framework": load_json(framework_scales_path)},
            source_paths={"framework": framework_scales_path},
        )
        if nana_virtual.get("status") != "ok":
            errors.append("nana virtual-list-10k extract did not return ok")
        if nana_virtual.get("equivalence") != "same-scenario":
            errors.append(
                "nana virtual-list-10k with catalog window must be same-scenario"
            )
    except Exception as exc:  # noqa: BLE001
        errors.append(f"nana virtual-list-10k extract failed: {exc}")

    virtual_100k = load_scenario("virtual-list-100k", root)
    scaled = extract_nana(
        virtual_100k,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "list",
                        "logical_rows": 100000,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 10,
                        "live_ui_entities": 50,
                        "live_ui_entities_bound": 50,
                        "materialize_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic")},
    )
    if scaled.get("status") != "ok" or scaled.get("work_counters", {}).get("live_ui_entities") != 50:
        errors.append("nana virtual-list-100k virtual_scales extract failed")
    if scaled.get("equivalence") != "closest-legacy-reference":
        errors.append(
            "nana virtual-list-100k without list_overscan_px must stay closest-legacy-reference"
        )
    if not any(
        "list_overscan_px" in str(note) for note in (scaled.get("mapping_notes") or [])
    ):
        errors.append("nana virtual-list-100k missing-window notes must name list_overscan_px")
    try:
        extract_nana(
            virtual_100k,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "list",
                            "logical_rows": 100000,
                            "status": "ok",
                            "visible_rows": 40,
                            "overscan_rows": 10,
                            "live_ui_entities": 50,
                            "list_viewport_px": 800.0,
                            "list_overscan_px": 200.0,
                            "list_item_extent_px": 20.0,
                            "materialize_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-list-200px")},
        )
        errors.append("nana virtual-list-100k with list_overscan_px=200 must KeyError")
    except KeyError as exc:
        reason = key_error_reason(exc)
        if "200" not in reason and "overscan" not in reason:
            errors.append(f"mismatched list overscan KeyError should name overscan/200: {exc}")
    list_same = _extract_nana_framework_fixture(
        root, virtual_100k, "virtual-scales-only.json"
    )
    _require_same_scenario(
        errors,
        list_same,
        "nana virtual-list-100k",
        live_ui_entities=57,
        shared_note="Shared catalog list window",
    )
    list_10k_live = _extract_nana_framework_fixture(
        root, virtual, "virtual-scales-only.json"
    )
    _require_same_scenario(errors, list_10k_live, "nana virtual-list-10k")

    paint = load_scenario("mutation-paint-only", root)
    hover = load_scenario("hover", root)

    try:
        painted = extract_nana(
            paint,
            {"runtime": load_json(runtime_path)},
            source_paths={"runtime": runtime_path},
        )
        if painted.get("status") != "ok":
            errors.append("nana mutation-paint-only extract did not return ok")
        counters = painted.get("work_counters") or {}
        if counters.get("layout_nodes") != 0:
            errors.append("nana-runtime-static-tree paint drain must measure layout_nodes=0")
        paint_ok_fixture = _named_invariant(painted, "paint_only_does_not_layout_full_tree")
        if paint_ok_fixture is None or paint_ok_fixture.get("status") != "ok":
            errors.append("fixture paint extract must evaluate layout_nodes=0")
        runtime_unmeasured_paint = copy.deepcopy(load_json(runtime_path))
        for case in runtime_unmeasured_paint.get("cases") or []:
            work = case.get("local_paint_work") if isinstance(case, dict) else None
            if isinstance(work, dict):
                work.pop("layout_nodes", None)
        painted_missing = extract_nana(
            paint,
            {"runtime": runtime_unmeasured_paint},
            source_paths={"runtime": Path("synthetic-runtime-paint-unmeasured")},
        )
        counters_missing = painted_missing.get("work_counters") or {}
        if "layout_nodes" in counters_missing:
            errors.append("ok envelope must not serialize unmeasured layout_nodes")
        paint_inv = _named_invariant(painted_missing, "paint_only_does_not_layout_full_tree")
        if paint_inv is None or paint_inv.get("status") != "not-evaluable":
            errors.append(
                "paint extract without layout_nodes must leave the invariant not-evaluable"
            )
        if paint_inv and "measured" in paint_inv:
            errors.append("not-evaluable paint invariant must omit measured")
    except Exception as exc:  # noqa: BLE001
        errors.append(f"nana paint extract failed: {exc}")

    paint_measured = extract_nana(
        paint,
        {
            "runtime": {
                "cases": [
                    {
                        "nodes": 5000,
                        "local_paint_systems_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "local_paint_commit_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "local_paint_schedule_ms": {"p50": 0.0, "p95": 0.0, "p99": 0.0},
                        "local_paint_work_nodes": 1,
                        "local_paint_work": {
                            "layout_nodes": 0,
                            "entities_total": 5000,
                            "entities_changed": 1,
                            "entities_spawned": 0,
                            "render_nodes_extracted": 1,
                            "extracted_text_spans": 0,
                            "gpu_upload_bytes": None,
                        },
                    }
                ]
            }
        },
        source_paths={"runtime": Path("synthetic-paint-counters")},
    )
    if paint_measured.get("status") != "ok":
        errors.append("nana paint extract with layout_nodes=0 must be ok")
    measured_counters_block = paint_measured.get("work_counters") or {}
    if measured_counters_block.get("layout_nodes") != 0:
        errors.append("ok envelope must contain numeric layout_nodes when fixture has it")
    if "gpu_upload_bytes" in measured_counters_block:
        errors.append("ok envelope must omit null work-counter fields")
    paint_cpu_zero = extract_nana(
        paint,
        {
            "runtime": {
                "cases": [
                    {
                        "nodes": 5000,
                        "local_paint_systems_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "local_paint_commit_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "local_paint_schedule_ms": {"p50": 0.0, "p95": 0.0, "p99": 0.0},
                        "local_paint_work_nodes": 1,
                        "local_paint_work": {
                            "layout_nodes": 0,
                            "entities_total": 5000,
                            "gpu_upload_bytes": 0,
                            "draw_calls": 0,
                            "draw_batches": 0,
                            "batch_rebuilds": 0,
                            "gpu_buffer_reallocations": 0,
                        },
                    }
                ]
            }
        },
        source_paths={"runtime": Path("synthetic-paint-cpu-gpu-zero")},
    )
    cpu_zero_counters = paint_cpu_zero.get("work_counters") or {}
    for key in GPU_WORK_COUNTER_KEYS:
        if key in cpu_zero_counters:
            errors.append(f"CPU drain must not serialize {key}=0 as an observed GPU count")
    if (paint_cpu_zero.get("metrics") or {}).get("gpu_upload_ms"):
        errors.append("Runtime-only paint envelope must not export gpu_upload_ms")
    if (paint_cpu_zero.get("frame_stages") or {}).get("GpuUpload", {}).get("status") == "ran":
        errors.append("Runtime-only paint envelope must not mark GPU stages ran")
    paint_ok_inv = _named_invariant(paint_measured, "paint_only_does_not_layout_full_tree")
    if (
        paint_ok_inv is None
        or paint_ok_inv.get("status") != "ok"
        or paint_ok_inv.get("measured") != 0
    ):
        errors.append("paint invariant must be ok with measured layout_nodes=0")

    gpu_scene = load_scenario("gpu-scene-ui", root)
    gpu_ok = extract_nana(
        gpu_scene,
        {"gpu": _synthetic_gpu_ui_only(gpu_scene)},
        source_paths={"gpu": Path("synthetic-gpu-scene")},
    )
    if gpu_ok.get("status") != "ok":
        errors.append("nana gpu-scene-ui extract with observed GPU work must be ok")
    gpu_counters = gpu_ok.get("work_counters") or {}
    if gpu_counters.get("gpu_upload_bytes") != 128:
        errors.append("gpu-scene-ui envelope must copy observed gpu_upload_bytes")
    if gpu_counters.get("draw_calls") != 3:
        errors.append("gpu-scene-ui envelope must copy observed draw_calls")
    if gpu_counters.get("gpu_buffer_reallocations") != 0:
        errors.append("gpu-scene-ui Some(0) reallocs after encode must stay numeric 0")
    gpu_upload_inv = _named_invariant(gpu_ok, "gpu_upload_bytes_observed_on_encode")
    if gpu_upload_inv is None or gpu_upload_inv.get("status") != "ok":
        errors.append("gpu-scene-ui upload invariant must be ok when encode was observed")
    if (gpu_ok.get("frame_stages") or {}).get("GpuUpload", {}).get("status") != "ran":
        errors.append("gpu-scene-ui envelope must mark FrameStage.GpuUpload as ran")
    if not (gpu_ok.get("metrics") or {}).get("gpu_upload_ms"):
        errors.append("gpu-scene-ui envelope must export gpu_upload_ms after FrameProfiler ran")
    try:
        extract_nana(
            gpu_scene,
            {"gpu": {"status": "ok", "scenario_id": "gpu-scene-ui", "composition": "UiOnly"}},
            source_paths={"gpu": Path("synthetic-gpu-missing-work")},
        )
        errors.append("gpu-scene-ui without gpu_work must KeyError, not invent zeros")
    except KeyError as exc:
        reason = key_error_reason(exc)
        if "gpu_work" not in reason and "materialization" not in reason:
            errors.append(f"gpu-scene-ui missing work KeyError should name gpu_work: {exc}")
    try:
        extract_nana(
            gpu_scene,
            {
                "gpu": _synthetic_gpu_ui_only(
                    gpu_scene,
                    materialization={
                        "viewport": [32, 32],
                        "host_texture": gpu_scene["params"]["host_texture"],
                        "ui_nodes": gpu_scene["params"]["ui_nodes"],
                        "ui_entity_count": 4,
                        "scene_primitive_kinds": ["host-texture", "quad"],
                    },
                )
            },
            source_paths={"gpu": Path("synthetic-gpu-wrong-viewport")},
        )
        errors.append("gpu-scene-ui with a private viewport must KeyError")
    except KeyError as exc:
        if "viewport" not in key_error_reason(exc):
            errors.append(f"gpu-scene-ui viewport mismatch KeyError should name viewport: {exc}")
    try:
        extract_nana(
            gpu_scene,
            {"gpu": _synthetic_gpu_ui_only(gpu_scene, include_frame=False)},
            source_paths={"gpu": Path("synthetic-gpu-missing-stages")},
        )
        errors.append("gpu-scene-ui without frame_stages must KeyError")
    except KeyError as exc:
        if "frame_stages" not in key_error_reason(exc) and "FrameProfiler" not in key_error_reason(
            exc
        ):
            errors.append(f"gpu-scene-ui missing frame_stages KeyError should name FrameProfiler: {exc}")
    try:
        extract_nana(
            gpu_scene,
            {"gpu": _synthetic_gpu_ui_only(gpu_scene, frame_status="unsupported")},
            source_paths={"gpu": Path("synthetic-gpu-unsupported-stages")},
        )
        errors.append(
            "gpu-scene-ui must not export GPU timings when FrameProfiler marks stages unsupported"
        )
    except KeyError as exc:
        if "unsupported" not in key_error_reason(exc) and "ran" not in key_error_reason(exc):
            errors.append(f"gpu-scene-ui unsupported-stage KeyError should name status: {exc}")
    live2d = load_scenario("gpu-scene-ui-live2d", root)
    try:
        extract_nana(
            live2d,
            {"gpu": {"status": "ok"}},
            source_paths={"gpu": Path("synthetic-live2d")},
        )
        errors.append("gpu-scene-ui-live2d extract must KeyError, not invent GPU numbers")
    except KeyError as exc:
        if "Live2D" not in key_error_reason(exc) and "UiLive2d" not in key_error_reason(exc):
            errors.append(f"gpu-scene-ui-live2d KeyError should name Live2D: {exc}")
    try:
        extract_nana(
            gpu_scene,
            {
                "gpu": {
                    "status": "unsupported",
                    "scenario_id": "gpu-scene-ui",
                    "composition": "UiOnly",
                    "unsupported_reason": (
                        "No WGPU adapter for the hosted GPU scene path. "
                        "Do not invent upload/batch zeros."
                    ),
                }
            },
            source_paths={"gpu": Path("synthetic-no-adapter")},
        )
        errors.append("gpu-scene-ui missing adapter must KeyError, not invent GPU keys")
    except KeyError as exc:
        reason = key_error_reason(exc)
        if "adapter" not in reason.lower() and "WGPU" not in reason:
            errors.append(f"gpu-scene-ui no-adapter KeyError should name adapter: {exc}")
    runtime_only_path = root / "perf" / "fixtures" / "nana-runtime-static-tree.json"
    try:
        extracted_runtime = extract_nana(
            gpu_scene,
            {"runtime": load_json(runtime_only_path)},
            source_paths={"runtime": runtime_only_path},
        )
        errors.append("gpu-scene-ui from a runtime-only dump must KeyError, not invent GPU keys")
        if extracted_runtime.get("work_counters"):
            errors.append("gpu-scene-ui runtime-only extract must not grow GPU work_counters")
    except KeyError as exc:
        if "nana-gpu-scene-benchmark" not in key_error_reason(exc):
            errors.append(
                f"gpu-scene-ui runtime-only KeyError should name nana-gpu-scene-benchmark: {exc}"
            )

    paint_failed = extract_nana(
        paint,
        {
            "runtime": {
                "cases": [
                    {
                        "nodes": 5000,
                        "local_paint_systems_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "local_paint_work_nodes": 1,
                        "local_paint_work": {"layout_nodes": 5},
                    }
                ]
            }
        },
        source_paths={"runtime": Path("synthetic-paint-layout")},
    )
    if paint_failed.get("status") != "error":
        errors.append("paint extract must fail-closed when layout_nodes != 0")

    text_mutation = load_scenario("mutation-text", root)
    text_ok = extract_nana(
        text_mutation,
        {
            "runtime": {
                "cases": [
                    {
                        "nodes": 5000,
                        "single_node_mutations": {
                            "Text": {
                                "systems_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                                "commit_ms": {"p50": 0.0, "p95": 0.0, "p99": 0.0},
                                "schedule_ms": {"p50": 0.0, "p95": 0.0, "p99": 0.0},
                                "work": {
                                    "text_shaped": 1,
                                    "layout_nodes": 0,
                                    "entities_changed": 1,
                                },
                            }
                        },
                    }
                ]
            }
        },
        source_paths={"runtime": Path("synthetic-text-mutation")},
    )
    if text_ok.get("status") != "ok":
        errors.append("nana mutation-text extract with text_shaped=1 must be ok")
    if (text_ok.get("work_counters") or {}).get("text_shaped") != 1:
        errors.append("mutation-text envelope must carry measured text_shaped")
    text_inv = _named_invariant(text_ok, "single_text_patch_shapes_bounded_nodes")
    if text_inv is None or text_inv.get("status") != "ok":
        errors.append("mutation-text invariant must be ok when text_shaped=1")

    try:
        extract_nana(
            text_mutation,
            {"runtime": {"cases": [{"nodes": 5000}]}},
            source_paths={"runtime": Path("synthetic-text-missing")},
        )
        errors.append("nana mutation-text without single_node_mutations.Text must KeyError")
    except KeyError as exc:
        if "single_node_mutations.Text" not in key_error_reason(exc):
            errors.append(f"mutation-text KeyError should name the drain: {exc}")

    def _single_node_runtime(mutation_kind: str, work: Mapping[str, Any]) -> dict[str, Any]:
        return {
            "runtime": {
                "cases": [
                    {
                        "nodes": 5000,
                        "single_node_mutations": {
                            mutation_kind: {
                                "systems_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                                "commit_ms": {"p50": 0.0, "p95": 0.0, "p99": 0.0},
                                "schedule_ms": {"p50": 0.0, "p95": 0.0, "p99": 0.0},
                                "work": dict(work),
                            }
                        },
                    }
                ]
            }
        }

    for scenario_id, kind, invariant_name, ok_work, fail_work in (
        (
            "mutation-transform",
            "Transform",
            "transform_does_not_layout_full_tree",
            {"layout_nodes": 0, "render_nodes_changed": 1},
            {"layout_nodes": 1, "render_nodes_changed": 1},
        ),
        (
            "mutation-a11y",
            "Accessibility",
            "a11y_does_not_layout",
            {
                "layout_nodes": 0,
                "accessibility_nodes_updated": 1,
                "render_nodes_changed": 0,
            },
            {"layout_nodes": 3},
        ),
    ):
        scenario = load_scenario(scenario_id, root)
        ok_report = extract_nana(
            scenario,
            _single_node_runtime(kind, ok_work),
            source_paths={"runtime": Path(f"synthetic-{scenario_id}")},
        )
        if ok_report.get("status") != "ok":
            errors.append(f"nana {scenario_id} extract with layout_nodes=0 must be ok")
        if not _named_invariant_ok_measured(ok_report, invariant_name, 0):
            errors.append(f"{scenario_id} invariant must be ok when layout_nodes=0")
        failed = extract_nana(
            scenario,
            _single_node_runtime(kind, fail_work),
            source_paths={"runtime": Path(f"synthetic-{scenario_id}-layout")},
        )
        if failed.get("status") != "error":
            errors.append(
                f"{kind.lower()} extract must fail-closed when layout_nodes != 0"
            )

    visibility_mutation = load_scenario("mutation-visibility", root)
    for spec in visibility_mutation.get("invariants") or []:
        if (
            spec.get("path") == "work_counters.layout_nodes"
            and spec.get("op") == "eq"
            and spec.get("value") == 0
        ):
            errors.append(
                "mutation-visibility must not claim layout_nodes==0; the live dump layouts"
            )
    visibility_ok = extract_nana(
        visibility_mutation,
        _single_node_runtime(
            "Visibility",
            {"layout_nodes": 12, "render_nodes_changed": 12, "style_processed": 1},
        ),
        source_paths={"runtime": Path("synthetic-visibility-mutation")},
    )
    if visibility_ok.get("status") != "ok":
        errors.append("nana mutation-visibility extract with ancestor-chain work must be ok")
    if not _named_invariant_ok_measured(
        visibility_ok, "visibility_does_not_extract_full_tree", 12
    ):
        errors.append("mutation-visibility extract invariant must measure render_nodes_changed=12")
    if not _named_invariant_ok_measured(
        visibility_ok, "visibility_does_not_layout_full_tree", 12
    ):
        errors.append("mutation-visibility layout invariant must measure layout_nodes=12, not 0")
    visibility_failed = extract_nana(
        visibility_mutation,
        _single_node_runtime(
            "Visibility",
            {"layout_nodes": 5000, "render_nodes_changed": 5000},
        ),
        source_paths={"runtime": Path("synthetic-visibility-full-tree")},
    )
    if visibility_failed.get("status") != "error":
        errors.append("visibility extract must fail-closed on a full-tree dirty set")

    for missing_id, missing_kind in (
        ("mutation-transform", "Transform"),
        ("mutation-visibility", "Visibility"),
        ("mutation-a11y", "Accessibility"),
    ):
        try:
            extract_nana(
                load_scenario(missing_id, root),
                {"runtime": {"cases": [{"nodes": 5000}]}},
                source_paths={"runtime": Path(f"synthetic-{missing_id}-missing")},
            )
            errors.append(
                f"nana {missing_id} without single_node_mutations.{missing_kind} must KeyError"
            )
        except KeyError as exc:
            if f"single_node_mutations.{missing_kind}" not in key_error_reason(exc):
                errors.append(f"{missing_id} KeyError should name the drain: {exc}")

    virtual_1m = load_scenario("virtual-list-1m", root)
    try:
        extract_nana(
            virtual_1m,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "list",
                            "logical_rows": 1000000,
                            "status": "skipped",
                            "skip_reason": "NANA_PERF_SCALE!=large",
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-1m-skipped")},
        )
        errors.append("nana virtual-list-1m skipped scale must KeyError, not ok")
    except KeyError as exc:
        if "1000000" not in key_error_reason(exc) and "1M" not in key_error_reason(exc).upper():
            errors.append(f"virtual-list-1m skip KeyError should name the 1M row: {exc}")

    virtual_1m_ok = extract_nana(
        virtual_1m,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "list",
                        "logical_rows": 1000000,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 10,
                        "live_ui_entities": 50,
                        "live_ui_entities_bound": 50,
                        "materialize_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic-1m-ok")},
    )
    if virtual_1m_ok.get("status") != "ok":
        errors.append("nana virtual-list-1m extract must be ok when virtual_scales status=ok")
    if virtual_1m_ok.get("scenario_id") != "virtual-list-1m":
        errors.append("virtual-list-1m must keep its catalog id, not virtual-list-1000k")
    if (virtual_1m_ok.get("work_counters") or {}).get("live_ui_entities") != 50:
        errors.append("virtual-list-1m envelope must carry live_ui_entities")

    catalog = load_catalog(root)
    harness = set(catalog.get("harness_ids", []))
    reserved_ids = {
        item.get("id") for item in catalog.get("required_by_issue_not_in_harness", [])
    }
    for tree_id in ("virtual-tree-10k", "virtual-tree-100k"):
        if tree_id not in harness:
            errors.append(f"catalog must list wirable {tree_id} in harness_ids")
        if tree_id in reserved_ids:
            errors.append(f"catalog must not leave wirable {tree_id} in required_by_issue_not_in_harness")
    if "virtual-tree-1m" in harness:
        errors.append("catalog must keep virtual-tree-1m out of harness_ids")
    if "virtual-tree-1m" not in reserved_ids:
        errors.append("catalog must list virtual-tree-1m in required_by_issue_not_in_harness")

    virtual_tree = load_scenario("virtual-tree-100k", root)
    try:
        extract_nana(
            virtual_tree,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "tree",
                            "logical_rows": 100000,
                            "status": "skipped",
                            "skip_reason": "NANA_PERF_SCALE!=large",
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-tree-skipped")},
        )
        errors.append("nana virtual-tree-100k skipped scale must KeyError, not ok")
    except KeyError as exc:
        if "tree/100000" not in key_error_reason(exc):
            errors.append(f"virtual-tree-100k skip KeyError should name the tree row: {exc}")

    try:
        extract_nana(
            virtual_tree,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "tree",
                            "logical_rows": 100000,
                            "status": "ok",
                            "live_ui_entities": None,
                            "materialize_ms": None,
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-tree-empty-ok")},
        )
        errors.append("nana virtual-tree-100k empty ok must KeyError, not ok")
    except KeyError as exc:
        if "fake empty ok" not in key_error_reason(exc):
            errors.append(f"virtual-tree empty ok KeyError should name fake empty ok: {exc}")

    try:
        extract_nana(
            virtual_tree,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "list",
                            "logical_rows": 100000,
                            "status": "ok",
                            "live_ui_entities": 50,
                            "materialize_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-tree-as-list")},
        )
        errors.append("nana virtual-tree-100k must not extract a list scale row as ok")
    except KeyError as exc:
        if "tree/100000" not in key_error_reason(exc):
            errors.append(f"virtual-tree list-row KeyError should name tree/100000: {exc}")

    tree_ok = _extract_nana_framework_fixture(
        root, virtual_tree, "virtual-tree-scales.json"
    )
    if tree_ok.get("scenario_id") != "virtual-tree-100k":
        errors.append("virtual-tree-100k must keep its catalog id")
    _require_same_scenario(
        errors, tree_ok, "nana virtual-tree-100k", live_ui_entities=57
    )
    tree_10k = _extract_nana_framework_fixture(
        root, "virtual-tree-10k", "virtual-tree-scales.json"
    )
    _require_same_scenario(errors, tree_10k, "nana virtual-tree-10k")

    virtual_tree_1m = load_scenario("virtual-tree-1m", root)
    try:
        extract_nana(
            virtual_tree_1m,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "tree",
                            "logical_rows": 1000000,
                            "status": "skipped",
                            "skip_reason": "NANA_PERF_SCALE!=large",
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-tree-1m-skipped")},
        )
        errors.append("nana virtual-tree-1m skipped scale must KeyError, not ok")
    except KeyError as exc:
        if "tree/1000000" not in key_error_reason(exc):
            errors.append(f"virtual-tree-1m skip KeyError should name the 1M tree row: {exc}")

    text_table = load_scenario("text-table", root)
    if "text-table" not in load_catalog(root).get("harness_ids", []):
        errors.append("catalog must list wirable text-table in harness_ids")
    reserved = {
        item.get("id") for item in load_catalog(root).get("required_by_issue_not_in_harness", [])
    }
    if "text-table" in reserved:
        errors.append("catalog must not leave wirable text-table in required_by_issue_not_in_harness")
    errors.extend(_self_test_catalog_workloads(root, runtime_path, iced_path, reserved))

    try:
        nana_table_legacy = extract_nana(
            text_table,
            {
                "framework": {
                    "virtual_table_10k_x_100_materialize_ms": {
                        "p50": 1.0,
                        "p95": 2.0,
                        "p99": 3.0,
                    },
                    "virtual_table_10k_x_100_window_ms": {
                        "p50": 0.1,
                        "p95": 0.2,
                        "p99": 0.3,
                    },
                }
            },
            source_paths={"framework": Path("synthetic-legacy-table-fields")},
        )
        if nana_table_legacy.get("status") != "ok":
            errors.append("nana text-table extract from legacy framework fields must be ok")
        if (nana_table_legacy.get("work_counters") or {}).get("text_shaped") is not None:
            errors.append("legacy text-table extract must omit unmeasured text_shaped")
        for gap in TEXT_TABLE_CACHE_GAPS:
            if gap in (nana_table_legacy.get("work_counters") or {}):
                errors.append(f"legacy text-table extract must omit {gap}")
    except Exception as exc:  # noqa: BLE001
        errors.append(f"nana text-table legacy extract failed: {exc}")

    try:
        extract_nana(
            text_table,
            {
                "framework": {
                    "virtual_scales": [
                        {
                            "kind": "table",
                            "logical_rows": 10000,
                            "logical_columns": 100,
                            "status": "skipped",
                            "skip_reason": "NANA_PERF_SCALE!=large",
                        }
                    ]
                }
            },
            source_paths={"framework": Path("synthetic-table-skipped")},
        )
        errors.append("nana text-table skipped scale must KeyError, not ok")
    except KeyError as exc:
        reason = key_error_reason(exc)
        if "10000" not in reason and "table" not in reason:
            errors.append(f"text-table skip KeyError should name the table scale: {exc}")

    table_ok = extract_nana(
        text_table,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "table",
                        "logical_rows": 10000,
                        "logical_columns": 100,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 8,
                        "table_overscan_y_px": 160.0,
                        "table_overscan_x_px": 160.0,
                        "live_ui_entities": 50,
                        "live_ui_entities_bound": 1334,
                        "materialize_ms": {"p50": 0.8, "p95": 0.9, "p99": 1.0},
                        "work": {
                            "text_shaped": 12,
                            "extracted_text_spans": 12,
                            "entities_total": 50,
                            "layout_nodes": 12,
                            "text_shaped_runs": 8,
                            "text_layout_cache_hits": 3,
                            "text_layout_cache_misses": 5,
                            "text_wrap_layouts": 4,
                            "cache_eviction": 0,
                            "glyph_cache_hits": None,
                        },
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic-table-ok")},
    )
    if table_ok.get("status") != "ok":
        errors.append("nana text-table extract must be ok when virtual_scales table status=ok")
    if table_ok.get("scenario_id") != "text-table":
        errors.append("text-table must keep its catalog id")
    table_counters = table_ok.get("work_counters") or {}
    if table_counters.get("live_ui_entities") != 50:
        errors.append("text-table envelope must carry live_ui_entities")
    if table_counters.get("text_shaped") != 12:
        errors.append("text-table envelope must export WorkCounters.text_shaped")
    if table_counters.get("extracted_text_spans") != 12:
        errors.append("text-table envelope must export WorkCounters.extracted_text_spans")
    if table_counters.get("text_wrap_layouts") != 4:
        errors.append("text-table envelope must export WorkCounters.text_wrap_layouts")
    if table_counters.get("text_shaped_runs") != 8:
        errors.append("text-table envelope must export WorkCounters.text_shaped_runs")
    if table_counters.get("text_layout_cache_hits") != 3:
        errors.append("text-table envelope must export WorkCounters.text_layout_cache_hits")
    if table_counters.get("text_layout_cache_misses") != 5:
        errors.append("text-table envelope must export WorkCounters.text_layout_cache_misses")
    if table_counters.get("cache_eviction") != 0:
        errors.append("text-table envelope must export WorkCounters.cache_eviction")
    for key in TEXT_TABLE_EXPORTED_SHAPE_KEYS:
        if key not in table_counters:
            errors.append(f"text-table envelope must contain {key}")
    if table_counters.get("wrapped_cells") != 4:
        errors.append("text-table envelope must copy catalog wrapped_cells=4")
    if table_counters.get("wrapped_cell_len") != 256:
        errors.append("text-table envelope must copy catalog wrapped_cell_len=256")
    if "glyph_cache_hits" in table_counters:
        errors.append("text-table envelope must omit null glyph_cache fields")
    table_glyphs = extract_nana(
        text_table,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "table",
                        "logical_rows": 10000,
                        "logical_columns": 100,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 8,
                        "table_overscan_y_px": 160.0,
                        "table_overscan_x_px": 160.0,
                        "live_ui_entities": 50,
                        "live_ui_entities_bound": 1334,
                        "materialize_ms": {"p50": 0.8, "p95": 0.9, "p99": 1.0},
                        "work": {
                            "text_shaped": 12,
                            "glyph_cache_hits": 7,
                            "glyph_cache_misses": 4,
                        },
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic-table-glyphs")},
    )
    glyph_counters = table_glyphs.get("work_counters") or {}
    if glyph_counters.get("glyph_cache_hits") != 7:
        errors.append("text-table envelope must copy real glyph_cache_hits")
    if glyph_counters.get("glyph_cache_misses") != 4:
        errors.append("text-table envelope must copy real glyph_cache_misses")
    null_glyph_inv = _named_invariant(table_ok, "text_table_glyph_cache_hits_observed")
    if null_glyph_inv is None or null_glyph_inv.get("status") != "not-evaluable":
        errors.append("text-table glyph_cache_hits must stay not-evaluable when omitted")
    if not any(
        "wrapping" in str(note).lower() or "wrapped_cells" in str(note)
        for note in table_ok.get("mapping_notes") or []
    ):
        errors.append("text-table ok mapping must mention wrapping / wrapped_cells")
    table_inv = _named_invariant(table_ok, "text_table_live_entities_bounded")
    if table_inv is None or table_inv.get("status") != "ok" or table_inv.get("measured") != 50:
        errors.append("text-table live_ui_entities invariant must be ok when measured")
    leftover_ten = {
        "framework": {
            "virtual_scales": [
                {
                    "kind": "table",
                    "logical_rows": 10000,
                    "logical_columns": 100,
                    "status": "ok",
                    "visible_rows": 40,
                    "overscan_rows": 10,
                    "table_overscan_y_px": 200.0,
                    "live_ui_entities": 50,
                    "materialize_ms": {"p50": 0.8, "p95": 0.9, "p99": 1.0},
                }
            ]
        }
    }
    try:
        extract_nana(
            text_table,
            leftover_ten,
            source_paths={"framework": Path("synthetic-table-10-row")},
        )
        errors.append("nana text-table with table_overscan_y_px=200 must KeyError")
    except KeyError as exc:
        if "overscan" not in key_error_reason(exc) and "200" not in key_error_reason(exc):
            errors.append(f"mismatched table overscan KeyError should name overscan/200: {exc}")
    catalog_window_ok = extract_nana(
        text_table,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "table",
                        "logical_rows": 10000,
                        "logical_columns": 100,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 8,
                        "table_overscan_y_px": 160.0,
                        "table_overscan_x_px": 160.0,
                        "live_ui_entities": 50,
                        "materialize_ms": {"p50": 0.8, "p95": 0.9, "p99": 1.0},
                        "work": {"text_shaped": 12},
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic-table-catalog-window")},
    )
    if catalog_window_ok.get("status") != "ok":
        errors.append("nana text-table extract with catalog table_overscan_y_px=160 must be ok")
    if catalog_window_ok.get("equivalence") != "closest-legacy-reference":
        errors.append(
            "nana text-table with only table_overscan_*_px must stay closest-legacy-reference"
        )
    if not any(
        "table_viewport_width_px" in str(note) or "table_row_extent_px" in str(note)
        for note in (catalog_window_ok.get("mapping_notes") or [])
    ):
        errors.append(
            "nana text-table missing viewport/extent notes must name the undeclared fields"
        )
    table_same = _extract_nana_framework_fixture(
        root, text_table, "virtual-table-scales.json"
    )
    _require_same_scenario(
        errors,
        table_same,
        "nana text-table",
        shared_note="Shared catalog table window",
    )
    max_window_ok = extract_nana(
        text_table,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "table",
                        "logical_rows": 10000,
                        "logical_columns": 100,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 8,
                        "table_overscan_y_px": 160.0,
                        "table_overscan_x_px": 160.0,
                        "live_ui_entities": 1334,
                        "live_ui_entities_bound": 1334,
                        "materialize_ms": {"p50": 0.8, "p95": 0.9, "p99": 1.0},
                        "work": {"text_shaped": 12},
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic-table-max-window")},
    )
    if max_window_ok.get("status") != "ok":
        errors.append("nana text-table catalog-8 max window live=1334 must stay ok")
    max_inv = _named_invariant(max_window_ok, "text_table_live_entities_bounded")
    if max_inv is None or max_inv.get("status") != "ok" or max_inv.get("measured") != 1334:
        errors.append("nana text-table invariant must accept catalog-8 cap live=1334")
    full_grid = extract_nana(
        text_table,
        {
            "framework": {
                "virtual_scales": [
                    {
                        "kind": "table",
                        "logical_rows": 10000,
                        "logical_columns": 100,
                        "status": "ok",
                        "visible_rows": 40,
                        "overscan_rows": 8,
                        "table_overscan_y_px": 160.0,
                        "table_overscan_x_px": 160.0,
                        "live_ui_entities": 1_000_000,
                        "live_ui_entities_bound": 1334,
                        "materialize_ms": {"p50": 0.8, "p95": 0.9, "p99": 1.0},
                    }
                ]
            }
        },
        source_paths={"framework": Path("synthetic-table-full-grid")},
    )
    if full_grid.get("status") != "error":
        errors.append("nana text-table live=1000000 must be rejected as envelope error")

    try:
        extract_iced(text_table, load_json(iced_path), source_path=iced_path)
        errors.append("iced text-table gallery extract must be unsupported")
    except KeyError:
        pass

    try:
        runtime_without_10k = {
            "cases": [
                case
                for case in load_json(runtime_path).get("cases") or []
                if isinstance(case, dict) and case.get("nodes") != 10000
            ]
        }
        extract_nana(
            hover,
            {"runtime": runtime_without_10k},
            source_paths={"runtime": Path("synthetic-runtime-without-10k")},
        )
        errors.append("nana hover without a 10k case must KeyError")
    except KeyError as exc:
        if "nodes=10000" not in key_error_reason(exc):
            errors.append(f"nana hover KeyError should name nodes=10000: {exc}")

    hover_ok = extract_nana(
        hover,
        {
            "runtime": {
                "cases": [
                    {
                        "nodes": 10000,
                        "pointer_hover_transition_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "pointer_hover_work_nodes": 2,
                    }
                ]
            }
        },
        source_paths={"runtime": Path("synthetic-hover-10k")},
    )
    if hover_ok.get("status") != "ok":
        errors.append("nana hover extract must be ok when a 10k hover case exists")
    if (hover_ok.get("work_counters") or {}).get("layout_nodes") is not None:
        errors.append("ok hover envelope must omit unmeasured layout_nodes")
    hover_inv = _named_invariant(hover_ok, "hover_without_size_change")
    if hover_inv is None or hover_inv.get("status") != "not-evaluable":
        errors.append("unmeasured hover extract must leave layout_nodes invariant not-evaluable")

    hover_measured = extract_nana(
        hover,
        {
            "runtime": {
                "cases": [
                    {
                        "nodes": 10000,
                        "pointer_hover_transition_ms": {"p50": 0.01, "p95": 0.02, "p99": 0.03},
                        "pointer_hover_work_nodes": 2,
                        "pointer_hover_work": {
                            "layout_nodes": 0,
                            "entities_changed": 2,
                            "render_nodes_extracted": 2,
                            "extracted_text_spans": None,
                        },
                    }
                ]
            }
        },
        source_paths={"runtime": Path("synthetic-hover-counters")},
    )
    if hover_measured.get("status") != "ok":
        errors.append("nana hover extract with layout_nodes=0 must be ok")
    hover_counters = hover_measured.get("work_counters") or {}
    if hover_counters.get("layout_nodes") != 0:
        errors.append("ok hover envelope must contain numeric layout_nodes when fixture has it")
    if "extracted_text_spans" in hover_counters:
        errors.append("ok hover envelope must omit null work-counter fields")
    hover_ok_inv = _named_invariant(hover_measured, "hover_without_size_change")
    if (
        hover_ok_inv is None
        or hover_ok_inv.get("status") != "ok"
        or hover_ok_inv.get("measured") != 0
    ):
        errors.append("hover invariant must be ok with measured layout_nodes=0")

    try:
        extract_iced(hover, load_json(iced_path), source_path=iced_path)
        errors.append("iced hover extract must be unsupported")
    except KeyError:
        pass

    errors.extend(_self_test_from_report_cli(root))

    gpui = gpui_unsupported(virtual)
    if gpui.get("status") != "unsupported":
        errors.append("gpui_unsupported missing-adapter helper must return unsupported")
    if gpui.get("relative_gate_enforceable") is not False:
        errors.append("gpui_unsupported must keep relative_gate_enforceable False")
    if gpui.get("metrics"):
        errors.append("gpui_unsupported must not invent metrics")

    if static_tree_parent(1) is not None or static_tree_parent(100) != 50:
        errors.append("StaticTree heap parent rule must be parent(1)=None, parent(100)=50")
    if static_tree_children(1, 100) != [2, 3] or static_tree_children(50, 100) != [100]:
        errors.append("StaticTree heap children must be left=2i, right=2i+1")
    if not is_shared_static_tree(
        {
            "generation": STATIC_TREE_GENERATION,
            "parent_rule": STATIC_TREE_PARENT_RULE,
            "node_kind": STATIC_TREE_NODE_KIND,
            "text": None,
            "sample_parents": static_tree_sample_parents(100),
        },
        100,
    ):
        errors.append("shared StaticTree heap descriptor must validate")

    iced_bench_path = root / "perf" / "fixtures" / "iced-scenario-static-tree-100.json"
    iced_same = extract_iced(
        static_tree, load_json(iced_bench_path), source_path=iced_bench_path
    )
    if iced_same.get("status") != "ok":
        errors.append("iced same-scenario static-tree-100 extract did not return ok")
    if iced_same.get("equivalence") != "same-scenario":
        errors.append("iced-scenario-bench extract must be same-scenario, not Gallery mapping")
    if iced_same.get("relative_gate_enforceable") is not False:
        errors.append("same-scenario iced extract must keep relative_gate_enforceable False")
    if iced_same.get("timing_gate_enforceable") is not False:
        errors.append("same-scenario iced extract must keep timing_gate_enforceable False")
    iced_cpu = (iced_same.get("metrics") or {}).get("cpu_frame_ms") or {}
    if not isinstance(iced_cpu.get("p50"), (int, float)):
        errors.append("iced same-scenario extract must copy live cpu_frame_ms.p50")
    if (iced_same.get("metrics") or {}).get("frames_after_idle") != 0:
        errors.append("iced same-scenario extract must copy frames_after_idle=0")
    if not isinstance((iced_same.get("metrics") or {}).get("busy_probe_frames"), int) or (
        iced_same.get("metrics") or {}
    ).get("busy_probe_frames", 0) < 1:
        errors.append("iced same-scenario extract must copy live busy_probe_frames > 0")
    iced_raw = load_json(iced_bench_path)
    if iced_raw.get("adapter", {}).get("name") == "extractor-fixture":
        errors.append("iced static-tree-100 fixture must be a live dump, not extractor-fixture")
    if iced_raw.get("frames_after_idle") != 0:
        errors.append("live iced static-tree-100 dump must have frames_after_idle=0")
    if not isinstance(iced_raw.get("busy_probe_frames"), int) or iced_raw.get(
        "busy_probe_frames", 0
    ) < 1:
        errors.append("live iced static-tree-100 dump must have busy_probe_frames > 0")
    gpui_static = gpui_unsupported(static_tree)
    if relative_gate_can_enforce(iced_same, gpui_static):
        errors.append("relative_gate_can_enforce must stay false for the missing-adapter helper")
    if relative_gate_can_enforce(iced_same, iced_same):
        errors.append("relative_gate_can_enforce must require runner=gpui, not two iced reports")
    if iced_same.get("relative_gate_enforceable") is not False:
        errors.append(
            "envelope relative_gate_enforceable must stay False"
        )
    try:
        extract_iced(
            load_scenario("static-tree-5k", root),
            load_json(iced_bench_path),
            source_path=iced_bench_path,
        )
        errors.append("iced-scenario-bench extract must not remap a 100-node fixture onto 5k")
    except KeyError as exc:
        if "nodes=" not in key_error_reason(exc):
            errors.append(f"mismatched StaticTree size KeyError should name nodes: {exc}")
    try:
        extract_iced(hover, load_json(iced_bench_path), source_path=iced_bench_path)
        errors.append("iced-scenario-bench hover extract from StaticTree fixture must KeyError")
    except KeyError:
        pass

    mutation = load_scenario("mutation-paint-only", root)
    mutation_path = root / "perf" / "fixtures" / "iced-scenario-mutation-paint-only.json"
    mutation_ok = extract_iced(mutation, load_json(mutation_path), source_path=mutation_path)
    if mutation_ok.get("status") != "ok" or mutation_ok.get("equivalence") != "same-scenario":
        errors.append("iced mutation-paint-only fixture extract must be same-scenario ok")
    if (mutation_ok.get("metrics") or {}).get("cpu_frame_ms", {}).get("p50") != 1.7:
        errors.append("iced mutation extract must copy fixture cpu_frame_ms.p50")
    if mutation_ok.get("relative_gate_enforceable") is not False:
        errors.append("iced mutation extract must keep relative gates off")
    if (mutation_ok.get("work_counters") or {}).get("layout_nodes") is not None:
        errors.append("iced paint-only extract must omit unmeasured layout_nodes")
    paint_extract_inv = _named_invariant(mutation_ok, "paint_only_does_not_layout_full_tree")
    if paint_extract_inv is None or paint_extract_inv.get("status") != "not-evaluable":
        errors.append("iced paint-only without layout_nodes must stay not-evaluable, not 0")

    fake_mutation_ok = {
        "source": "iced-scenario-bench",
        "status": "ok",
        "nodes": 5000,
        "cpu_frame_ms": {"p50": 1.0, "p95": 1.2, "p99": 1.3},
        "tree": {
            "generation": STATIC_TREE_GENERATION,
            "parent_rule": STATIC_TREE_PARENT_RULE,
            "node_kind": STATIC_TREE_NODE_KIND,
            "text": None,
            "sample_parents": static_tree_sample_parents(5000),
        },
        "mutation": {"kind": "Transform", "target_index": 2503, "single_node": True},
    }
    for unsupported_id in (
        "mutation-transform",
        "mutation-visibility",
        "mutation-a11y",
    ):
        unsupported_scenario = load_scenario(unsupported_id, root)
        fake_mutation_ok["scenario_id"] = unsupported_id
        fake_mutation_ok["mutation"] = {
            "kind": unsupported_scenario["params"]["kind"],
            "target_index": 2500,
            "single_node": True,
        }
        try:
            extracted = extract_iced(
                unsupported_scenario,
                fake_mutation_ok,
                source_path=Path(f"fake-{unsupported_id}"),
            )
            if extracted.get("status") == "ok":
                errors.append(
                    f"iced {unsupported_id} extract must not be ok / same-scenario"
                )
        except KeyError as exc:
            reason = key_error_reason(exc)
            if unsupported_scenario["params"]["kind"] not in reason and "same-scenario" not in reason:
                errors.append(
                    f"iced {unsupported_id} KeyError should name the missing dirty work: {exc}"
                )

    hover_path = root / "perf" / "fixtures" / "iced-scenario-hover.json"
    hover_iced_ok = extract_iced(hover, load_json(hover_path), source_path=hover_path)
    if hover_iced_ok.get("status") != "ok":
        errors.append("iced hover fixture extract must be ok at 10k, not a smaller tree")
    if (hover_iced_ok.get("work_counters") or {}).get("nodes") != 10000:
        errors.append("iced hover fixture extract must record work_counters.nodes=10000")
    if (hover_iced_ok.get("work_counters") or {}).get("layout_nodes") is not None:
        errors.append("iced hover extract must omit unmeasured layout_nodes")
    hover_extract_inv = _named_invariant(hover_iced_ok, "hover_without_size_change")
    if hover_extract_inv is None or hover_extract_inv.get("status") != "not-evaluable":
        errors.append("iced hover without layout_nodes must stay not-evaluable, not 0")
    if hover_iced_ok.get("equivalence") != "same-scenario":
        errors.append("iced hover fixture extract must be same-scenario")
    if (hover_iced_ok.get("metrics") or {}).get("cpu_frame_ms", {}).get("p50") != 2.1:
        errors.append("iced hover extract must copy fixture cpu_frame_ms.p50")

    virtual_path = root / "perf" / "fixtures" / "iced-scenario-virtual-list-10k.json"
    virtual_iced = extract_iced(virtual, load_json(virtual_path), source_path=virtual_path)
    if virtual_iced.get("status") != "ok":
        errors.append("iced virtual-list-10k fixture extract must be ok")
    if (virtual_iced.get("work_counters") or {}).get("live_ui_entities") != 56:
        errors.append("iced virtual-list extract must copy live_ui_entities=56")
    if any(
        "200px" in note or "may differ" in note
        for note in (virtual_iced.get("mapping_notes") or [])
    ):
        errors.append("iced virtual-list same-scenario notes must not claim overscan may differ")
    wrong_overscan = load_json(virtual_path)
    wrong_overscan["virtualization"] = dict(wrong_overscan["virtualization"])
    wrong_overscan["virtualization"]["overscan"] = 10
    try:
        extract_iced(virtual, wrong_overscan, source_path=Path("wrong-overscan"))
        errors.append("iced VirtualList with overscan!=catalog must KeyError")
    except KeyError as exc:
        if "overscan" not in key_error_reason(exc):
            errors.append(f"mismatched VirtualList overscan KeyError should name overscan: {exc}")
    fake_full_list = load_json(virtual_path)
    fake_full_list["virtualization"] = dict(fake_full_list["virtualization"])
    fake_full_list["virtualization"]["live_ui_entities"] = 10000
    try:
        extract_iced(virtual, fake_full_list, source_path=Path("fake-full-list"))
        errors.append("iced VirtualList with live_ui_entities==items must KeyError")
    except KeyError as exc:
        if "10000" not in key_error_reason(exc):
            errors.append(f"full-list VirtualList KeyError should name 10000: {exc}")

    table_iced_path = root / "perf" / "fixtures" / "iced-scenario-text-table.json"
    table_iced_ok = extract_iced(text_table, load_json(table_iced_path), source_path=table_iced_path)
    if table_iced_ok.get("status") != "ok" or table_iced_ok.get("equivalence") != "same-scenario":
        errors.append("iced text-table fixture extract must be same-scenario ok")
    if (table_iced_ok.get("work_counters") or {}).get("overscan_rows") != 8:
        errors.append("iced text-table extract must record overscan_rows=8")
    if (table_iced_ok.get("metrics") or {}).get("cpu_frame_ms", {}).get("p50") != 5.5:
        errors.append("iced text-table extract must copy fixture cpu_frame_ms.p50")
    max_iced = load_json(table_iced_path)
    max_iced["virtualization"] = dict(max_iced["virtualization"])
    max_iced["virtualization"]["live_ui_entities"] = 1334
    max_iced["virtualization"]["live_ui_entities_bound"] = 1334
    max_iced_ok = extract_iced(text_table, max_iced, source_path=Path("iced-table-max-window"))
    if max_iced_ok.get("status") != "ok" or max_iced_ok.get("equivalence") != "same-scenario":
        errors.append("iced text-table catalog-8 max window live=1334 must stay same-scenario ok")
    max_iced_inv = _named_invariant(max_iced_ok, "text_table_live_entities_bounded")
    if (
        max_iced_inv is None
        or max_iced_inv.get("status") != "ok"
        or max_iced_inv.get("measured") != 1334
    ):
        errors.append("iced text-table invariant must accept catalog-8 cap live=1334")
    wrong_table = load_json(table_iced_path)
    wrong_table["virtualization"] = dict(wrong_table["virtualization"])
    wrong_table["virtualization"]["overscan_rows"] = 10
    try:
        extract_iced(text_table, wrong_table, source_path=Path("fake-table-10-row"))
        errors.append("iced Table with overscan_rows=10 must KeyError")
    except KeyError as exc:
        if "overscan" not in key_error_reason(exc):
            errors.append(f"mismatched Table overscan KeyError should name overscan: {exc}")
    full_table = load_json(table_iced_path)
    full_table["virtualization"] = dict(full_table["virtualization"])
    full_table["virtualization"]["live_ui_entities"] = 1_000_000
    try:
        extract_iced(text_table, full_table, source_path=Path("fake-full-table"))
        errors.append("iced Table with live_ui_entities==rows×columns must KeyError")
    except KeyError as exc:
        if "10000" not in key_error_reason(exc) and "cells" not in key_error_reason(exc):
            errors.append(f"full Table KeyError should name 10000/cells: {exc}")

    for unsupported_id, token in (
        ("animation", "advance_animations"),
        ("ime", "set_ime_preedit"),
        ("overlay", "OverlayHost"),
        ("gpu-scene-ui", "Live2D"),
        ("dock-workspace", "assemble_dock"),
        ("text-editor", "drain_text"),
    ):
        unsupported_scenario = load_scenario(unsupported_id, root)
        fake_ok = {
            "source": "iced-scenario-bench",
            "status": "ok",
            "scenario_id": unsupported_id,
            "cpu_frame_ms": {"p50": 1.0, "p95": 1.2, "p99": 1.3},
        }
        try:
            extracted = extract_iced(
                unsupported_scenario,
                fake_ok,
                source_path=Path(f"fake-{unsupported_id}"),
            )
            if extracted.get("status") == "ok":
                errors.append(f"iced {unsupported_id} extract must not be ok / same-scenario")
        except KeyError as exc:
            reason = key_error_reason(exc)
            if token not in reason:
                errors.append(
                    f"iced {unsupported_id} KeyError should name the missing work ({token}): {exc}"
                )

    try:
        extract_iced(
            load_scenario("static-tree-50k", root),
            {
                "source": "iced-scenario-bench",
                "status": "ok",
                "scenario_id": "static-tree-50k",
                "nodes": 50000,
                "cpu_frame_ms": {"p50": 1.0, "p95": 2.0, "p99": 3.0},
                "tree": {
                    "generation": STATIC_TREE_GENERATION,
                    "parent_rule": STATIC_TREE_PARENT_RULE,
                    "node_kind": STATIC_TREE_NODE_KIND,
                    "text": None,
                    "sample_parents": static_tree_sample_parents(50000),
                },
            },
            source_path=Path("fake-50k"),
        )
        errors.append("iced static-tree-50k extract must be unsupported/incomparable")
    except KeyError as exc:
        if "50k" not in key_error_reason(exc).lower() and "50000" not in key_error_reason(exc):
            errors.append(f"iced 50k KeyError should name incomparable 50k: {exc}")

    try:
        extract_nana(
            load_scenario("static-tree-50k", root),
            {
                "runtime": {
                    "cases": [
                        {
                            "nodes": 50000,
                            "kind": "construction",
                            "enqueue_ms": {"p50": 1.0, "p95": 2.0, "p99": 3.0},
                            "initial_commit_ms": {"p50": 1.0, "p95": 2.0, "p99": 3.0},
                        }
                    ]
                }
            },
            source_paths={"runtime": Path("synthetic-50k")},
        )
        errors.append("nana static-tree-50k construction extract must KeyError as incomparable")
    except KeyError as exc:
        if "50k" not in key_error_reason(exc).lower() and "construction" not in key_error_reason(exc).lower():
            errors.append(f"nana 50k KeyError should name incomparable construction: {exc}")

    empty_ok = {
        "source": "iced-scenario-bench",
        "status": "ok",
        "scenario_id": "static-tree-100",
        "nodes": 100,
        "cpu_frame_ms": {},
    }
    try:
        extract_iced(static_tree, empty_ok, source_path=Path("empty-cpu"))
        errors.append("iced-scenario-bench ok without cpu_frame_ms percentiles must KeyError")
    except KeyError:
        pass
    leaf_column = {
        "source": "iced-scenario-bench",
        "status": "ok",
        "scenario_id": "static-tree-100",
        "nodes": 100,
        "cpu_frame_ms": {"p50": 1.4, "p95": 1.7, "p99": 2.0},
    }
    try:
        extract_iced(static_tree, leaf_column, source_path=Path("text-leaves"))
        errors.append("N text leaves without heap provenance must not be same-scenario")
    except KeyError as exc:
        if "complete-binary-heap" not in key_error_reason(exc):
            errors.append(f"text-leaf KeyError should name the heap rule: {exc}")
    heap_without_idle = {
        "source": "iced-scenario-bench",
        "status": "ok",
        "scenario_id": "static-tree-100",
        "nodes": 100,
        "cpu_frame_ms": {"p50": 1.4, "p95": 1.7, "p99": 2.0},
        "tree": {
            "generation": STATIC_TREE_GENERATION,
            "parent_rule": STATIC_TREE_PARENT_RULE,
            "node_kind": STATIC_TREE_NODE_KIND,
            "text": None,
            "sample_parents": static_tree_sample_parents(100),
        },
    }
    try:
        extract_iced(static_tree, heap_without_idle, source_path=Path("heap-no-idle"))
        errors.append("iced StaticTree without frames_after_idle must KeyError")
    except KeyError as exc:
        if "frames_after_idle" not in key_error_reason(exc):
            errors.append(f"missing idle-frame KeyError should name frames_after_idle: {exc}")
    stuffed_zero = dict(heap_without_idle)
    stuffed_zero["frames_after_idle"] = 0
    try:
        extract_iced(static_tree, stuffed_zero, source_path=Path("stuffed-idle-zero"))
        errors.append("iced StaticTree stuffed frames_after_idle=0 without busy_probe must KeyError")
    except KeyError as exc:
        if "busy_probe_frames" not in key_error_reason(exc):
            errors.append(f"stuffed idle-0 KeyError should name busy_probe_frames: {exc}")
    errors.extend(_self_test_section_8_1_runner_invariants(root))
    return errors
