"""Executable contract regression cases."""
from __future__ import annotations

import copy
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from .extractors import (
    _extract_nana_framework_fixture,
    _named_invariant,
    _named_invariant_ok_measured,
    _named_invariant_status,
    extract_iced,
    extract_nana,
)
from .invariants import (
    evaluate_runner_invariant_paths,
    judge_runner_invariants,
)
from .reports import (
    envelope,
)
from .schema import (
    EXIT_ERROR,
    EXIT_OK,
    EXIT_UNSUPPORTED,
    INCOMPARABLE_STATIC_TREE_50K_REASON,
    PR_INVARIANTS_DIR_NAME,
    SECTION_8_1_CATALOG_WORKLOAD_IDS,
    SECTION_8_1_HONEST_OK_IDS,
    SECTION_8_1_UNSUPPORTED_IDS,
    SECTION_8_1_WEEKLY_UBUNTU_IDS,
    catalog_table_live_bound,
    catalog_virtual_list_live_bound,
    dump_json,
    load_json,
    load_scenario,
)




def _self_test_section_8_1_runner_invariants(root: Path) -> list[str]:
    """Prove §8.1 judging of runner envelopes (pass, fail, skip). No log-string match."""
    errors: list[str] = []
    virtual = load_scenario("virtual-list-10k", root)
    text_table = load_scenario("text-table", root)
    list_bound = catalog_virtual_list_live_bound(virtual["params"])
    table_bound = catalog_table_live_bound(text_table["params"])
    list_spec = (virtual.get("invariants") or [{}])[0]
    table_spec = (text_table.get("invariants") or [{}])[0]
    if list_spec.get("value") != list_bound:
        errors.append(
            f"virtual-list-10k invariant value {list_spec.get('value')} "
            f"must be catalog cap {list_bound}"
        )
    hundredk = load_scenario("virtual-list-100k", root)
    if (hundredk.get("invariants") or [{}])[0].get("value") != list_bound:
        errors.append("virtual-list-100k must share the catalog-8 live bound")
    tree_10k = load_scenario("virtual-tree-10k", root)
    tree_100k = load_scenario("virtual-tree-100k", root)
    if (tree_10k.get("invariants") or [{}])[0].get("value") != list_bound:
        errors.append("virtual-tree-10k must share the catalog-8 live bound")
    if (tree_100k.get("invariants") or [{}])[0].get("value") != list_bound:
        errors.append("virtual-tree-100k must share the catalog-8 live bound")
    if table_spec.get("value") != table_bound:
        errors.append(
            f"text-table invariant value {table_spec.get('value')} "
            f"must be catalog cap {table_bound}"
        )

    static_tree_ids = (
        "static-tree-100",
        "static-tree-1k",
        "static-tree-5k",
        "static-tree-10k",
    )
    for scenario_id in static_tree_ids:
        if scenario_id not in SECTION_8_1_HONEST_OK_IDS:
            errors.append(f"{scenario_id} must be §8.1 honest-ok once frames_after_idle exists")
        if scenario_id in SECTION_8_1_UNSUPPORTED_IDS:
            errors.append(f"{scenario_id} must leave §8.1 unsupported once idle frames exist")
        static_scenario = load_scenario(scenario_id, root)
        specs = static_scenario.get("invariants") or []
        if not specs:
            errors.append(f"{scenario_id} must declare static_ui frames_after_idle == 0")
        else:
            spec = specs[0]
            if (
                spec.get("name") != "static_ui"
                or spec.get("path") != "metrics.frames_after_idle"
                or spec.get("op") != "eq"
                or spec.get("value") != 0
            ):
                errors.append(
                    f"{scenario_id} idle invariant must be static_ui metrics.frames_after_idle == 0"
                )
    if "static-tree-50k" not in SECTION_8_1_UNSUPPORTED_IDS:
        errors.append("static-tree-50k must stay §8.1 skipped")
    if "static-tree-50k" in SECTION_8_1_HONEST_OK_IDS:
        errors.append("static-tree-50k must not be §8.1 honest-ok")
    if "gpu-scene-ui" not in SECTION_8_1_HONEST_OK_IDS or "gpu-scene-ui" in SECTION_8_1_UNSUPPORTED_IDS:
        errors.append("gpu-scene-ui must be §8.1 honest-ok for Nana encode envelopes")
    if "animation" not in SECTION_8_1_HONEST_OK_IDS:
        errors.append(
            "animation must be §8.1 honest-ok once animations_considered exists; "
            "due==1 is not the pass"
        )
    if "gpu-scene-ui" in SECTION_8_1_WEEKLY_UBUNTU_IDS:
        errors.append("weekly ubuntu set must not include macos-only gpu-scene-ui")
    if "animation" not in SECTION_8_1_WEEKLY_UBUNTU_IDS:
        errors.append("weekly ubuntu still maps animation and must evaluate the sparse hotspot")
    extra_weekly = SECTION_8_1_WEEKLY_UBUNTU_IDS - SECTION_8_1_HONEST_OK_IDS
    if extra_weekly:
        errors.append(
            "weekly ubuntu gated ids must be a subset of honest-ok: "
            + ", ".join(sorted(extra_weekly))
        )
    weekly_wf = (root / ".github" / "workflows" / "runtime-performance.yml").read_text()
    weekly_cpu = weekly_wf.split("macos-composition:")[0]
    if "issue8/weekly" not in weekly_cpu:
        errors.append("weekly ubuntu must --evaluate-invariants target/performance/issue8/weekly")
    if "gpu-scene-ui" in weekly_cpu:
        errors.append("weekly ubuntu job must not map macos-only gpu-scene-ui")
    pr_ci = (root / ".github" / "workflows" / "ci.yml").read_text()
    if "issue8/invariants" not in pr_ci or "nana-gpu-scene-ui.json" not in pr_ci:
        errors.append("PR invariants directory must still map gpu-scene-ui")
    for scenario_id in sorted(SECTION_8_1_CATALOG_WORKLOAD_IDS):
        if scenario_id not in SECTION_8_1_HONEST_OK_IDS:
            errors.append(f"{scenario_id} must be §8.1 honest-ok once a dirty hotspot exists")
        if scenario_id in SECTION_8_1_UNSUPPORTED_IDS:
            errors.append(f"{scenario_id} must leave §8.1 unsupported once a dirty hotspot exists")
        specs = load_scenario(scenario_id, root).get("invariants") or []
        identity_paths = {
            "work_counters.ime_script_count",
            "work_counters.panes",
            "work_counters.overlay_kind_count",
            "work_counters.due_animation_samples",
        }
        if scenario_id == "text-editor":
            identity_paths.add("work_counters.text_shaped")
        for spec in specs:
            if spec.get("path") in identity_paths:
                errors.append(
                    f"{scenario_id} must not use identity {spec.get('path')} as the §8.1 pass"
                )
    animation_specs = load_scenario("animation", root).get("invariants") or []
    animation_gates = {
        spec.get("path"): spec
        for spec in animation_specs
        if spec.get("op") == "lte" and spec.get("value") == 8
    }
    for path in (
        "work_counters.animations_considered",
        "work_counters.animation_deadlines_scanned",
    ):
        if path not in animation_gates:
            errors.append(
                f"animation must judge {path} with lte 8; a due-only increment must not stay green"
            )
    for live2d_id in ("gpu-scene-ui-live2d", "gpu-scene-ui-live2d-effect"):
        if live2d_id in SECTION_8_1_HONEST_OK_IDS or live2d_id not in SECTION_8_1_UNSUPPORTED_IDS:
            errors.append(f"{live2d_id} must stay §8.1 skipped")
    for scenario_id in sorted(SECTION_8_1_HONEST_OK_IDS):
        specs = load_scenario(scenario_id, root).get("invariants") or []
        if not specs:
            errors.append(
                f"{scenario_id} is §8.1 honest-ok but has no catalog invariants; "
                "vacuous ok is forbidden"
            )

    virtual_path = root / "perf" / "fixtures" / "iced-scenario-virtual-list-10k.json"
    table_path = root / "perf" / "fixtures" / "iced-scenario-text-table.json"
    hover_path = root / "perf" / "fixtures" / "iced-scenario-hover.json"
    paint_path = root / "perf" / "fixtures" / "iced-scenario-mutation-paint-only.json"
    static_path = root / "perf" / "fixtures" / "iced-scenario-static-tree-100.json"
    try:
        virtual_iced = extract_iced(virtual, load_json(virtual_path), source_path=virtual_path)
        static_iced = extract_iced(
            load_scenario("static-tree-100", root),
            load_json(static_path),
            source_path=static_path,
        )
        table_iced = extract_iced(text_table, load_json(table_path), source_path=table_path)
        hover_iced = extract_iced(
            load_scenario("hover", root), load_json(hover_path), source_path=hover_path
        )
        paint_iced = extract_iced(
            load_scenario("mutation-paint-only", root),
            load_json(paint_path),
            source_path=paint_path,
        )
        nana_table_missing_glyphs = _extract_nana_framework_fixture(
            root, text_table, "virtual-table-scales.json"
        )
        nana_table = _extract_nana_framework_fixture(
            root, text_table, "nana-framework-text-table.json"
        )
        nana_list = _extract_nana_framework_fixture(
            root, "virtual-list-100k", "virtual-scales-only.json"
        )
        nana_list_10k = _extract_nana_framework_fixture(
            root, "virtual-list-10k", "virtual-scales-only.json"
        )
        nana_tree_10k = _extract_nana_framework_fixture(
            root, "virtual-tree-10k", "virtual-tree-scales.json"
        )
        nana_tree_100k = _extract_nana_framework_fixture(
            root, "virtual-tree-100k", "virtual-tree-scales.json"
        )
        nana_runtime_live_path = root / "perf" / "fixtures" / "nana-runtime-static-tree.json"
        nana_runtime_live = load_json(nana_runtime_live_path)
        nana_live = {
            scenario_id: extract_nana(
                load_scenario(scenario_id, root),
                {"runtime": nana_runtime_live},
                source_paths={"runtime": nana_runtime_live_path},
            )
            for scenario_id in (
                "static-tree-100",
                "static-tree-1k",
                "static-tree-5k",
                "static-tree-10k",
                "mutation-transform",
                "mutation-a11y",
                "mutation-visibility",
                "mutation-text",
                "mutation-layout-style",
                "hover",
                "mutation-paint-only",
                "animation",
            )
        }
        nana_static = nana_live["static-tree-100"]
        nana_transform = nana_live["mutation-transform"]
        nana_a11y = nana_live["mutation-a11y"]
        nana_visibility = nana_live["mutation-visibility"]
        nana_hover = nana_live["hover"]
        nana_paint = nana_live["mutation-paint-only"]
        nana_animation = nana_live["animation"]
        nana_catalog_path = root / "perf" / "fixtures" / "nana-framework-catalog-workloads.json"
        nana_catalog_live = load_json(nana_catalog_path)
        nana_catalog = {
            scenario_id: extract_nana(
                load_scenario(scenario_id, root),
                {"framework": nana_catalog_live},
                source_paths={"framework": nana_catalog_path},
            )
            for scenario_id in ("ime", "dock-workspace", "overlay", "text-editor")
        }
    except Exception as exc:  # noqa: BLE001
        errors.append(f"§8.1 envelope fixtures failed to extract: {exc}")
        return errors

    for payload, label in (
        (virtual_iced, "iced VirtualList"),
        (table_iced, "iced text-table"),
        (hover_iced, "iced hover"),
        (paint_iced, "iced paint-only"),
        (static_iced, "iced StaticTree"),
    ):
        judged = judge_runner_invariants(payload, root=root)
        if judged.get("decision") != "skipped":
            errors.append(f"{label} envelope must be skipped, not a §8.1 pass")

    hover_inv = _named_invariant(hover_iced, "hover_without_size_change")
    if hover_inv is None or hover_inv.get("status") != "not-evaluable":
        errors.append("iced hover without layout_nodes must stay not-evaluable, not 0")
    if (hover_iced.get("work_counters") or {}).get("layout_nodes") is not None:
        errors.append("iced hover envelope must omit unmeasured layout_nodes")
    paint_inv = _named_invariant(paint_iced, "paint_only_does_not_layout_full_tree")
    if paint_inv is None or paint_inv.get("status") != "not-evaluable":
        errors.append("iced paint-only without layout_nodes must stay not-evaluable")
    if (paint_iced.get("work_counters") or {}).get("layout_nodes") is not None:
        errors.append("iced paint-only envelope must omit unmeasured layout_nodes")

    list_fail_payload = copy.deepcopy(nana_list)
    hundredk_items = hundredk["params"]["items"]
    counters = dict(list_fail_payload.get("work_counters") or {})
    counters["live_ui_entities"] = hundredk_items
    list_fail_payload["work_counters"] = counters
    list_fail = judge_runner_invariants(list_fail_payload, root=root)
    if list_fail.get("decision") != "failed":
        errors.append("VirtualList live_ui_entities==items must fail §8.1")

    cap_ok_payload = copy.deepcopy(nana_table)
    cap_ok_payload["work_counters"] = dict(cap_ok_payload.get("work_counters") or {})
    cap_ok_payload["work_counters"]["live_ui_entities"] = table_bound
    cap_ok = judge_runner_invariants(cap_ok_payload, root=root)
    if cap_ok.get("decision") != "ok":
        errors.append("table live_ui_entities at catalog-8 cap must pass §8.1")
    if _named_invariant_status(cap_ok, "text_table_live_entities_bounded") != "ok":
        errors.append("table live=catalog cap must measure as ok")

    table_fail_payload = copy.deepcopy(nana_table)
    table_fail_payload["work_counters"] = dict(table_fail_payload.get("work_counters") or {})
    table_fail_payload["work_counters"]["live_ui_entities"] = 1_000_000
    table_fail = judge_runner_invariants(table_fail_payload, root=root)
    if table_fail.get("decision") != "failed":
        errors.append("table live_ui_entities=1e6 must fail §8.1")

    if judge_runner_invariants(nana_table_missing_glyphs, root=root).get("decision") != "skipped":
        errors.append(
            "virtual-table-scales text-table without glyph_cache_* must skip, not envelope-ok"
        )
    nana_table_judge = judge_runner_invariants(nana_table, root=root)
    if nana_table_judge.get("decision") != "ok":
        errors.append("nana text-table live table slice must pass §8.1")
    table_hits_only = copy.deepcopy(nana_table)
    table_hits_only["work_counters"] = {
        **dict(table_hits_only.get("work_counters") or {}),
        "glyph_cache_hits": 8,
        "glyph_cache_misses": 0,
    }
    if judge_runner_invariants(table_hits_only, root=root).get("decision") != "failed":
        errors.append("text-table glyph_cache misses=0 must fail §8.1")
    table_misses_only = copy.deepcopy(nana_table)
    table_misses_only["work_counters"] = {
        **dict(table_misses_only.get("work_counters") or {}),
        "glyph_cache_hits": 0,
        "glyph_cache_misses": 8,
    }
    if judge_runner_invariants(table_misses_only, root=root).get("decision") != "failed":
        errors.append("text-table glyph_cache hits=0 must fail §8.1")
    nana_list_judge = judge_runner_invariants(nana_list, root=root)
    if nana_list_judge.get("decision") != "ok":
        errors.append("nana virtual-list-100k fixture envelope must pass §8.1")
    nana_static_judge = judge_runner_invariants(nana_static, root=root)
    if nana_static_judge.get("decision") != "ok":
        errors.append("nana static-tree-100 fixture envelope must pass §8.1")
    if _named_invariant_status(nana_static_judge, "static_ui") != "ok":
        errors.append("nana StaticTree 100 must evaluate static_ui frames_after_idle==0")
    nana_hover_judge = judge_runner_invariants(nana_hover, root=root)
    if nana_hover_judge.get("decision") != "ok":
        errors.append("nana hover live dump must pass §8.1 when layout_nodes is measured")
    nana_paint_judge = judge_runner_invariants(nana_paint, root=root)
    if nana_paint_judge.get("decision") != "ok":
        errors.append("nana paint-only live dump must pass §8.1 when layout_nodes is measured")

    for payload, scenario_id, rows in (
        (
            nana_transform,
            "mutation-transform",
            [("transform_does_not_layout_full_tree", 0,
              "nana Transform live dump must measure layout_nodes=0")],
        ),
        (
            nana_a11y,
            "mutation-a11y",
            [("a11y_does_not_layout", 0,
              "nana Accessibility live dump must measure layout_nodes=0")],
        ),
        (
            nana_visibility,
            "mutation-visibility",
            [
                ("visibility_does_not_extract_full_tree", 12,
                 "nana Visibility live dump must measure render_nodes_changed=12"),
                ("visibility_does_not_layout_full_tree", 12,
                 "nana Visibility live dump must measure layout_nodes=12 as lte 64, not eq 0"),
            ],
        ),
    ):
        judged = judge_runner_invariants(payload, root=root)
        if judged.get("decision") != "ok":
            errors.append(f"nana {scenario_id} live dump must pass §8.1")
        for name, measured, message in rows:
            if not _named_invariant_ok_measured(judged, name, measured):
                errors.append(message)
    if (nana_visibility.get("work_counters") or {}).get("layout_nodes") != 12:
        errors.append(
            "nana Visibility live dump must keep measured layout_nodes=12; "
            "do not claim or stuff layout_nodes==0"
        )
    vis_eq_zero = copy.deepcopy(nana_visibility)
    vis_eq_zero["scenario_id"] = "mutation-transform"
    vis_as_transform = judge_runner_invariants(vis_eq_zero, root=root)
    if vis_as_transform.get("decision") != "failed":
        errors.append("Visibility layout_nodes=12 must fail Transform's layout_nodes==0 row")

    hover_judge = judge_runner_invariants(hover_iced, root=root)
    if hover_judge.get("decision") != "skipped":
        errors.append("iced hover envelope must be skipped, never a §8.1 pass")
    if _named_invariant(hover_iced, "hover_without_size_change") is None:
        errors.append("iced hover extract must still attach hover_without_size_change")

    if (static_iced.get("metrics") or {}).get("frames_after_idle") != 0:
        errors.append("iced StaticTree fixture extract must copy frames_after_idle=0")
    fake_busy = copy.deepcopy(nana_static)
    fake_busy["frames_after_idle"] = 1
    metrics = dict(fake_busy.get("metrics") or {})
    metrics["frames_after_idle"] = 1
    fake_busy["metrics"] = metrics
    busy_judge = judge_runner_invariants(fake_busy, root=root)
    if busy_judge.get("decision") != "failed":
        errors.append("StaticTree frames_after_idle=1 must fail §8.1 static_ui")
    missing_idle = copy.deepcopy(nana_static)
    missing_metrics = dict(missing_idle.get("metrics") or {})
    missing_metrics.pop("frames_after_idle", None)
    missing_idle["metrics"] = missing_metrics
    missing_idle.pop("frames_after_idle", None)
    missing_idle_judge = judge_runner_invariants(missing_idle, root=root)
    if missing_idle_judge.get("decision") != "skipped":
        errors.append(
            "StaticTree missing frames_after_idle must stay skipped, not vacuous ok"
        )
    if "frames_after_idle" not in str(missing_idle_judge.get("note") or ""):
        errors.append(
            "STATIC_UI skip note must name frames_after_idle when that field is not-evaluable"
        )
    missing_validation = copy.deepcopy(nana_live["static-tree-10k"])
    missing_validation_counters = dict(missing_validation.get("work_counters") or {})
    missing_validation_counters.pop("validation_nodes_scanned", None)
    missing_validation["work_counters"] = missing_validation_counters
    skipped_validation = judge_runner_invariants(missing_validation, root=root)
    if skipped_validation.get("decision") != "skipped":
        errors.append(
            "static-tree-10k without validation_nodes_scanned must stay skipped, not vacuous ok"
        )
    skipped_validation_note = str(skipped_validation.get("note") or "")
    if "validation_nodes_scanned" not in skipped_validation_note:
        errors.append(
            "STATIC_UI skip note must name validation_nodes_scanned when that field is not-evaluable"
        )
    if skipped_validation_note.startswith("frames_after_idle missing"):
        errors.append(
            "STATIC_UI skip note must not always say frames_after_idle missing"
        )

    paint_judge = judge_runner_invariants(paint_iced, root=root)
    if paint_judge.get("decision") != "skipped":
        errors.append("iced paint-only envelope must be skipped, never a §8.1 pass")

    dock_ok_fake = envelope(
        runner="iced",
        status="ok",
        scenario_id="dock-workspace",
        scenario=load_scenario("dock-workspace", root),
        work_counters={"panes": 8},
        equivalence="same-scenario",
    )
    dock_judge = judge_runner_invariants(dock_ok_fake, root=root)
    if dock_judge.get("decision") != "skipped":
        errors.append("Iced Dock ok envelope must be skipped, not invariant-ok")

    gpui_ok_fake = envelope(
        runner="gpui",
        status="ok",
        scenario_id="virtual-list-10k",
        scenario=virtual,
        work_counters={"live_ui_entities": 56},
        equivalence="same-scenario",
    )
    gpui_judge = judge_runner_invariants(gpui_ok_fake, root=root)
    if gpui_judge.get("decision") != "skipped":
        errors.append("GPUI envelope must be skipped, not invariant-ok")

    gpu_scene = load_scenario("gpu-scene-ui", root)
    gpu_live_path = root / "perf" / "fixtures" / "nana-gpu-scene-ui.json"
    try:
        gpu_live = extract_nana(
            gpu_scene,
            {"gpu": load_json(gpu_live_path)},
            source_paths={"gpu": gpu_live_path},
        )
    except Exception as exc:  # noqa: BLE001
        errors.append(f"nana gpu-scene-ui live encode fixture failed to extract: {exc}")
        gpu_live = None
    missing_gpu = envelope(
        runner="nana",
        status="ok",
        scenario_id="gpu-scene-ui",
        scenario=gpu_scene,
        equivalence="same-scenario",
    )
    no_adapter = envelope(
        runner="nana",
        status="unsupported",
        scenario_id="gpu-scene-ui",
        scenario=gpu_scene,
        unsupported_reason=(
            "No WGPU adapter for the hosted GPU scene path. Do not invent upload/batch zeros."
        ),
    )
    if gpu_live is not None:
        if gpu_live.get("status") != "ok" or gpu_live.get("equivalence") != "same-scenario":
            errors.append("nana gpu-scene-ui live encode fixture must extract same-scenario ok")
        if gpu_live.get("relative_gate_enforceable") is not False:
            errors.append("gpu-scene-ui must keep relative_gate_enforceable False")
        live_counters = gpu_live.get("work_counters") or {}
        if (live_counters.get("draw_calls") or 0) < 1:
            errors.append("gpu-scene-ui live dump must not handwrite draw_calls; encode observed < 1")
        if "gpu_upload_bytes" not in live_counters:
            errors.append("gpu-scene-ui live encode fixture must copy observed gpu_upload_bytes")
        if judge_runner_invariants(gpu_live, root=root).get("decision") != "ok":
            errors.append("nana gpu-scene-ui live encode envelope must be judged, not skipped")
        zero_draws = copy.deepcopy(gpu_live)
        zero_draws["work_counters"] = {
            **dict(live_counters),
            "draw_calls": 0,
            "gpu_upload_bytes": 0,
        }
        if judge_runner_invariants(zero_draws, root=root).get("decision") != "failed":
            errors.append(
                "gpu-scene-ui draw_calls=0 must fail §8.1; gpu_upload_bytes>=0 must not greenwash it"
            )
    if judge_runner_invariants(missing_gpu, root=root).get("decision") != "skipped":
        errors.append(
            "gpu-scene-ui ok envelope without GPU keys must skip, not vacuous invariant-ok"
        )
    if judge_runner_invariants(no_adapter, root=root).get("decision") != "skipped":
        errors.append(
            "gpu-scene-ui missing adapter must stay skipped/unsupported, not failed or invariant-ok"
        )
    iced_gpu_ok = envelope(
        runner="iced",
        status="ok",
        scenario_id="gpu-scene-ui",
        scenario=gpu_scene,
        work_counters={"gpu_upload_bytes": 0, "draw_calls": 1},
        equivalence="same-scenario",
    )
    if judge_runner_invariants(iced_gpu_ok, root=root).get("decision") != "skipped":
        errors.append("iced gpu-scene-ui must stay skipped, not invariant-ok")
    for live2d_id in ("gpu-scene-ui-live2d", "gpu-scene-ui-live2d-effect"):
        fake_live2d = envelope(
            runner="nana",
            status="ok",
            scenario_id=live2d_id,
            scenario=load_scenario(live2d_id, root),
            work_counters={"gpu_upload_bytes": 0, "draw_calls": 1},
            equivalence="same-scenario",
        )
        if judge_runner_invariants(fake_live2d, root=root).get("decision") != "skipped":
            errors.append(f"{live2d_id} fake-ok must stay skipped, not invariant-ok")

    fifty = envelope(
        runner="nana",
        status="unsupported",
        scenario_id="static-tree-50k",
        scenario=load_scenario("static-tree-50k", root),
        unsupported_reason=INCOMPARABLE_STATIC_TREE_50K_REASON,
    )
    if judge_runner_invariants(fifty, root=root).get("decision") != "skipped":
        errors.append("static-tree-50k must stay skipped")

    for unsupported_id in (
        "text-editor",
        "animation",
        "ime",
        "overlay",
        "dock-workspace",
    ):
        unsupported = envelope(
            runner="iced",
            status="unsupported",
            scenario_id=unsupported_id,
            scenario=load_scenario(unsupported_id, root),
            unsupported_reason="unsupported fixture",
        )
        if judge_runner_invariants(unsupported, root=root).get("decision") != "skipped":
            errors.append(f"{unsupported_id} unsupported envelope must be skipped")

    if judge_runner_invariants(nana_animation, root=root).get("decision") != "ok":
        errors.append("nana animation live dump must pass the sparse-advance hotspot")
    if (nana_animation.get("work_counters") or {}).get("due_animation_samples") != 1:
        errors.append("animation envelope must still copy due_animation_samples")
    if (nana_animation.get("work_counters") or {}).get("animations_considered") != 1:
        errors.append("animation envelope must copy animations_considered from the sparse advance")
    if (nana_animation.get("work_counters") or {}).get("animation_deadlines_scanned") != 1:
        errors.append("animation envelope must copy animation_deadlines_scanned from the sparse advance")
    scanned_gate = _named_invariant(
        nana_animation, "animation_advance_does_not_walk_full_deadline_index"
    )
    if scanned_gate is None or scanned_gate.get("status") != "ok":
        errors.append("animation extract must judge animation_deadlines_scanned as a §8.1 row")
    anim_identity = _named_invariant(nana_animation, "idle_scheduled_animations_sparse_sample")
    if anim_identity is not None:
        errors.append("animation must not attach due==1 as a §8.1 invariant")
    anim_full = copy.deepcopy(nana_animation)
    anim_full["work_counters"] = {
        **dict(anim_full.get("work_counters") or {}),
        "due_animation_samples": 1,
        "animations_considered": 64,
        "animation_deadlines_scanned": 64,
    }
    if judge_runner_invariants(anim_full, root=root).get("decision") != "failed":
        errors.append(
            "animation full-world animations_considered must fail even when due_animation_samples==1"
        )
    anim_full_index = copy.deepcopy(nana_animation)
    anim_full_index["work_counters"] = {
        **dict(anim_full_index.get("work_counters") or {}),
        "due_animation_samples": 1,
        "animations_considered": 1,
        "animation_deadlines_scanned": 64,
    }
    if judge_runner_invariants(anim_full_index, root=root).get("decision") != "failed":
        errors.append(
            "animation full-table animation_deadlines_scanned=64 must fail even when "
            "animations_considered==1"
        )
    anim_missing = copy.deepcopy(nana_animation)
    anim_missing["work_counters"] = {
        "due_animation_samples": 1,
        "scheduled_animations": 64,
    }
    if judge_runner_invariants(anim_missing, root=root).get("decision") != "skipped":
        errors.append("animation missing animations_considered must fail-closed (skipped)")
    for scenario_id, payload in nana_catalog.items():
        judged = judge_runner_invariants(payload, root=root)
        if judged.get("decision") != "ok":
            errors.append(f"nana {scenario_id} live dump must be judged, not skipped")
    ime_full = copy.deepcopy(nana_catalog["ime"])
    ime_full["work_counters"] = {
        **dict(ime_full.get("work_counters") or {}),
        "ime_script_count": 4,
        "layout_nodes": 12,
    }
    if judge_runner_invariants(ime_full, root=root).get("decision") != "failed":
        errors.append("IME full-tree layout_nodes>>0 must fail even when ime_script_count==4")
    editor_full = copy.deepcopy(nana_catalog["text-editor"])
    editor_full["work_counters"] = {
        **dict(editor_full.get("work_counters") or {}),
        "text_shaped": 1,
        "layout_nodes": 12,
    }
    if judge_runner_invariants(editor_full, root=root).get("decision") != "failed":
        errors.append("text-editor full-tree layout_nodes>>0 must fail even when text_shaped==1")
    overlay_full = copy.deepcopy(nana_catalog["overlay"])
    overlay_full["work_counters"] = {
        **dict(overlay_full.get("work_counters") or {}),
        "overlay_kind_count": 4,
        "layout_nodes": 5,
        "entities_changed": 5,
    }
    if judge_runner_invariants(overlay_full, root=root).get("decision") != "failed":
        errors.append("overlay full-tree layout/dirty must fail even when overlay_kind_count==4")
    overlay_chrome = copy.deepcopy(nana_catalog["overlay"])
    overlay_chrome["work_counters"] = {
        **dict(overlay_chrome.get("work_counters") or {}),
        "layout_nodes": 2,
        "entities_changed": 3,
    }
    if judge_runner_invariants(overlay_chrome, root=root).get("decision") != "ok":
        errors.append("overlay modest chrome growth must not false-fail the loose cap")
    dock_full = copy.deepcopy(nana_catalog["dock-workspace"])
    dock_full["work_counters"] = {
        **dict(dock_full.get("work_counters") or {}),
        "panes": 8,
        "layout_nodes": 45,
        "render_nodes_changed": 45,
    }
    if judge_runner_invariants(dock_full, root=root).get("decision") != "failed":
        errors.append("dock full-tree layout/extract must fail even when panes==8")
    dock_chrome = copy.deepcopy(nana_catalog["dock-workspace"])
    dock_chrome["work_counters"] = {
        **dict(dock_chrome.get("work_counters") or {}),
        "layout_nodes": 26,
        "render_nodes_changed": 34,
    }
    if judge_runner_invariants(dock_chrome, root=root).get("decision") != "ok":
        errors.append("dock modest chrome growth must not false-fail the loose cap")
    missing_editor = envelope(
        runner="nana",
        status="ok",
        scenario_id="text-editor",
        scenario=load_scenario("text-editor", root),
        equivalence="closest-legacy-reference",
    )
    if judge_runner_invariants(missing_editor, root=root).get("decision") != "skipped":
        errors.append(
            "nana text-editor ok envelope without layout_nodes must stay skipped, "
            "not vacuous invariant-ok"
        )

    raw_fixture = judge_runner_invariants(load_json(virtual_path), root=root)
    if raw_fixture.get("decision") != "error":
        errors.append("raw scenario-bench fixture must not be judged as a runner envelope")

    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        pass_path = tmp_path / "pass.json"
        fail_path = tmp_path / "fail.json"
        skip_path = tmp_path / "skip.json"
        static_dir = tmp_path / "static-only"
        static_dir.mkdir()
        static_only = static_dir / "static.json"
        dump_json(pass_path, nana_list)
        dump_json(fail_path, list_fail_payload)
        dump_json(skip_path, fifty)
        dump_json(static_only, nana_static)
        summary_ok, code_ok = evaluate_runner_invariant_paths([pass_path], root=root)
        if code_ok != EXIT_OK or summary_ok.get("status") != "ok":
            errors.append("§8.1 path judge must pass a live VirtualList envelope")
        summary_fail, code_fail = evaluate_runner_invariant_paths([fail_path], root=root)
        if code_fail != EXIT_ERROR or summary_fail.get("status") != "failed":
            errors.append("§8.1 path judge must fail live==items")
        summary_skip, code_skip = evaluate_runner_invariant_paths([skip_path], root=root)
        if code_skip != EXIT_UNSUPPORTED or summary_skip.get("status") != "unsupported":
            errors.append("§8.1 path judge on only-skipped reports must exit 2")
        summary_static, code_static = evaluate_runner_invariant_paths(
            [static_only], root=root
        )
        if (
            code_static != EXIT_OK
            or summary_static.get("status") != "ok"
            or summary_static.get("ok") != 1
            or summary_static.get("skipped") != 0
        ):
            errors.append("§8.1 StaticTree-only dump with frames_after_idle=0 must pass")
        iced_hover_dir = tmp_path / "iced-hover-only"
        iced_hover_dir.mkdir()
        dump_json(iced_hover_dir / "hover.json", hover_iced)
        summary_iced_hover, code_iced_hover = evaluate_runner_invariant_paths(
            [iced_hover_dir], root=root
        )
        if (
            code_iced_hover != EXIT_UNSUPPORTED
            or summary_iced_hover.get("status") != "unsupported"
            or summary_iced_hover.get("ok") != 0
            or summary_iced_hover.get("skipped") != 1
        ):
            errors.append("§8.1 iced hover-only dump must skip, never envelope-ok")
        extra_dir = tmp_path / "idle-cases"
        extra_dir.mkdir()
        missing_path = extra_dir / "static-missing.json"
        dump_json(missing_path, missing_idle)
        summary_missing, code_missing = evaluate_runner_invariant_paths(
            [missing_path], root=root
        )
        if (
            code_missing != EXIT_UNSUPPORTED
            or summary_missing.get("status") != "unsupported"
            or summary_missing.get("skipped") != 1
        ):
            errors.append("§8.1 StaticTree missing frames_after_idle must skip")
        busy_path = extra_dir / "static-busy.json"
        dump_json(busy_path, fake_busy)
        summary_busy, code_busy = evaluate_runner_invariant_paths([busy_path], root=root)
        if code_busy != EXIT_ERROR or summary_busy.get("status") != "failed":
            errors.append("§8.1 StaticTree frames_after_idle=1 must fail")
        mixed, mixed_code = evaluate_runner_invariant_paths(
            [pass_path, skip_path], root=root
        )
        if mixed_code != EXIT_OK or mixed.get("skipped") != 1 or mixed.get("ok") != 1:
            errors.append("§8.1 mixed ok+unsupported-skip envelopes must exit 0")
        missing_glyph_path = tmp_path / "text-table-missing-glyphs.json"
        dump_json(missing_glyph_path, nana_table_missing_glyphs)
        mixed_gated, mixed_gated_code = evaluate_runner_invariant_paths(
            [pass_path, missing_glyph_path], root=root
        )
        if mixed_gated_code != EXIT_ERROR or mixed_gated.get("status") != "failed":
            errors.append(
                "§8.1 mixed ok+gated-skip (text-table without glyph_cache_*) must exit 1"
            )
        dir_summary, dir_code = evaluate_runner_invariant_paths([tmp_path], root=root)
        if dir_code != EXIT_ERROR or dir_summary.get("failed") != 1:
            errors.append("§8.1 directory judge must fail-closed when a file fails")
        if gpu_live is not None:
            gpu_env = tmp_path / "gpu-scene-ui.json"
            dump_json(gpu_env, gpu_live)
            gpu_summary, gpu_code = evaluate_runner_invariant_paths([gpu_env], root=root)
            if gpu_code != EXIT_OK or gpu_summary.get("status") != "ok":
                errors.append("§8.1 path judge must pass a live gpu-scene-ui encode envelope")
        missing_gpu_path = tmp_path / "gpu-scene-ui-missing.json"
        dump_json(missing_gpu_path, missing_gpu)
        missing_summary, missing_code = evaluate_runner_invariant_paths(
            [missing_gpu_path], root=root
        )
        if missing_code != EXIT_UNSUPPORTED or missing_summary.get("skipped") != 1:
            errors.append(
                "§8.1 gpu-scene-ui without GPU keys must exit 2, not vacuous ok"
            )
        missing_glyph_summary, missing_glyph_code = evaluate_runner_invariant_paths(
            [missing_glyph_path], root=root
        )
        if (
            missing_glyph_code != EXIT_UNSUPPORTED
            or missing_glyph_summary.get("skipped") != 1
            or missing_glyph_summary.get("ok") != 0
        ):
            errors.append(
                "§8.1 text-table without glyph_cache_* must exit 2, not vacuous ok"
            )
        if gpu_live is None:
            errors.append("§8.1 complete gated directory needs gpu-scene-ui live encode")
        else:
            honest_ok = {
                "static-tree-100": nana_live["static-tree-100"],
                "static-tree-1k": nana_live["static-tree-1k"],
                "static-tree-5k": nana_live["static-tree-5k"],
                "static-tree-10k": nana_live["static-tree-10k"],
                "hover": nana_hover,
                "mutation-paint-only": nana_paint,
                "mutation-text": nana_live["mutation-text"],
                "mutation-layout-style": nana_live["mutation-layout-style"],
                "mutation-visibility": nana_visibility,
                "mutation-transform": nana_transform,
                "mutation-a11y": nana_a11y,
                "virtual-list-10k": nana_list_10k,
                "virtual-list-100k": nana_list,
                "virtual-tree-10k": nana_tree_10k,
                "virtual-tree-100k": nana_tree_100k,
                "text-table": nana_table,
                "gpu-scene-ui": gpu_live,
                "animation": nana_animation,
                **nana_catalog,
            }
            missing_envelopes = SECTION_8_1_HONEST_OK_IDS - set(honest_ok)
            if missing_envelopes:
                errors.append(
                    "self-test missing honest-ok envelopes: "
                    + ", ".join(sorted(str(item) for item in missing_envelopes))
                )
            else:
                gated_dir = tmp_path / PR_INVARIANTS_DIR_NAME
                gated_dir.mkdir()
                for scenario_id, payload in honest_ok.items():
                    dump_json(gated_dir / f"nana-{scenario_id}.json", payload)
                complete, complete_code = evaluate_runner_invariant_paths(
                    [gated_dir], root=root
                )
                if complete_code != EXIT_OK or complete.get("ok") != len(
                    SECTION_8_1_HONEST_OK_IDS
                ):
                    errors.append("§8.1 PR invariants/ directory must exit 0 when complete")
                dump_json(gated_dir / "nana-static-tree-50k.json", fifty)
                dump_json(gated_dir / "iced-hover.json", hover_iced)
                with_unsupported_code = evaluate_runner_invariant_paths(
                    [gated_dir], root=root
                )[1]
                if with_unsupported_code != EXIT_OK:
                    errors.append(
                        "§8.1 PR invariants/ + Iced/50k skip must exit 0"
                    )
                (gated_dir / "nana-static-tree-50k.json").unlink()
                (gated_dir / "iced-hover.json").unlink()
                skipped_anim = copy.deepcopy(nana_animation)
                skipped_anim["work_counters"] = {
                    "due_animation_samples": 1,
                    "scheduled_animations": 64,
                }
                dump_json(gated_dir / "nana-animation.json", skipped_anim)
                if evaluate_runner_invariant_paths([gated_dir], root=root)[1] != EXIT_ERROR:
                    errors.append(
                        "§8.1 PR invariants/ + animation missing animations_considered must exit 1"
                    )
                dump_json(gated_dir / "nana-animation.json", nana_animation)
                dump_json(
                    gated_dir / "virtual-table-scales.json",
                    load_json(root / "perf" / "fixtures" / "virtual-table-scales.json"),
                )
                if evaluate_runner_invariant_paths([gated_dir], root=root)[1] != EXIT_ERROR:
                    errors.append(
                        "§8.1 PR invariants/ containing raw virtual-table-scales.json must exit 1"
                    )
                (gated_dir / "virtual-table-scales.json").unlink()
                (gated_dir / "nana-overlay.json").unlink()
                missing_pr, missing_pr_code = evaluate_runner_invariant_paths(
                    [gated_dir], root=root
                )
                if missing_pr_code != EXIT_ERROR:
                    errors.append("§8.1 PR invariants/ missing a gated id must exit 1")
                elif "overlay" not in str(missing_pr.get("note")):
                    errors.append(
                        "§8.1 PR invariants/ missing-id note must name the omitted gated id"
                    )
                dump_json(gated_dir / "nana-overlay.json", nana_catalog["overlay"])
                weekly_dir = tmp_path / "weekly"
                weekly_dir.mkdir()
                for scenario_id in sorted(SECTION_8_1_WEEKLY_UBUNTU_IDS):
                    dump_json(weekly_dir / f"nana-{scenario_id}.json", honest_ok[scenario_id])
                weekly_summary, weekly_code = evaluate_runner_invariant_paths(
                    [weekly_dir], root=root
                )
                if (
                    weekly_code != EXIT_OK
                    or weekly_summary.get("ok") != len(SECTION_8_1_WEEKLY_UBUNTU_IDS)
                    or weekly_summary.get("skipped") != 0
                ):
                    errors.append(
                        "weekly-shaped directory (all weekly ids ok, including animation hotspot, "
                        "no gpu-scene-ui) "
                        f"must exit 0, got {weekly_code} ok={weekly_summary.get('ok')} "
                        f"skipped={weekly_summary.get('skipped')} note={weekly_summary.get('note')}"
                    )
                dump_json(weekly_dir / "nana-text-table.json", nana_table_missing_glyphs)
                weekly_glyph, weekly_glyph_code = evaluate_runner_invariant_paths(
                    [weekly_dir], root=root
                )
                if weekly_glyph_code != EXIT_ERROR:
                    errors.append(
                        "weekly-shaped directory must fail-closed when a present gated id skips"
                    )
                elif "text-table" not in str(weekly_glyph.get("note")):
                    errors.append(
                        "weekly mixed-skip note must name the skipped gated id in that directory"
                    )
                dump_json(weekly_dir / "nana-text-table.json", nana_table)
                (weekly_dir / "nana-overlay.json").unlink()
                weekly_missing, weekly_missing_code = evaluate_runner_invariant_paths(
                    [weekly_dir], root=root
                )
                if weekly_missing_code != EXIT_OK:
                    errors.append(
                        "weekly-shaped directory must not apply PR completeness "
                        f"(missing overlay/gpu-scene-ui): exit={weekly_missing_code} "
                        f"note={weekly_missing.get('note')}"
                    )
                skip_only = tmp_path / "skip-only"
                skip_only.mkdir()
                dump_json(skip_only / "fifty.json", fifty)
                dump_json(skip_only / "iced.json", hover_iced)
                skip_only_summary, skip_only_code = evaluate_runner_invariant_paths(
                    [skip_only], root=root
                )
                if skip_only_code != EXIT_UNSUPPORTED or skip_only_summary.get("ok") != 0:
                    errors.append("§8.1 all-skip-only directory must exit 2, never 0")
        cli = subprocess.run(
            [
                sys.executable,
                str(root / "perf" / "contract.py"),
                "--repo-root",
                str(root),
                "--evaluate-invariants",
                str(pass_path),
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if cli.returncode != EXIT_OK:
            errors.append(f"--evaluate-invariants on a passing envelope must exit 0, got {cli.returncode}")
        else:
            try:
                payload = json.loads(cli.stdout)
                if payload.get("status") != "ok" or payload.get("ok") != 1:
                    errors.append("--evaluate-invariants stdout must be a status=ok summary")
            except json.JSONDecodeError:
                errors.append("--evaluate-invariants stdout must be JSON")
        fail_cli = subprocess.run(
            [
                sys.executable,
                str(root / "perf" / "contract.py"),
                "--repo-root",
                str(root),
                "--evaluate-invariants",
                str(fail_path),
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if fail_cli.returncode != EXIT_ERROR:
            errors.append(
                f"--evaluate-invariants on live==items must exit 1, got {fail_cli.returncode}"
            )
    return errors
