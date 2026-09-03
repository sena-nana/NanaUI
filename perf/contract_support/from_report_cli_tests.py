"""Executable contract regression cases."""
from __future__ import annotations

import copy
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Mapping
from .comparison import (
    compare_issue12_pair,
    evaluate_relative_paths,
    relative_gate_can_enforce,
)
from .extractors import (
    extract_iced,
)
from .invariants import (
    judge_runner_invariants,
)
from .reports import (
    envelope,
    key_error_reason,
)
from .schema import (
    EXIT_OK,
    EXIT_UNSUPPORTED,
    ISSUE12_FIXTURE_IDS,
    TEXT_TABLE_EXPORTED_SHAPE_KEYS,
    dump_json,
    load_json,
    load_scenario,
)




def _self_test_from_report_cli(root: Path) -> list[str]:
    """Exercise --from-report through runners, not only extract_*."""
    errors: list[str] = []
    nana_script = root / "perf" / "runners" / "nana" / "run.py"
    iced_script = root / "perf" / "runners" / "iced" / "run.py"

    def from_report(
        script: Path, scenario_id: str, fixture: Path
    ) -> tuple[int, dict[str, Any] | None, str]:
        result = subprocess.run(
            [
                sys.executable,
                str(script),
                "--repo-root",
                str(root),
                "--scenario",
                scenario_id,
                "--from-report",
                str(fixture),
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != EXIT_OK:
            return result.returncode, None, result.stderr
        try:
            return result.returncode, json.loads(result.stdout), result.stderr
        except json.JSONDecodeError as exc:
            return result.returncode, None, str(exc)

    def _from_report_same_scenario(code, report, err, label, live_ui_entities):
        if code != EXIT_OK or report is None:
            errors.append(f"{label} --from-report exit {code}: {err}")
        elif report.get("status") != "ok":
            errors.append(f"{label} --from-report status={report.get('status')}")
        elif report.get("equivalence") != "same-scenario":
            errors.append(f"{label} --from-report must be same-scenario")
        elif report.get("relative_gate_enforceable") is not False:
            errors.append(
                f"{label} --from-report must keep relative_gate_enforceable False"
            )
        elif (report.get("work_counters") or {}).get("live_ui_entities") != live_ui_entities:
            errors.append(
                f"{label} --from-report must copy live_ui_entities={live_ui_entities}"
            )

    code, report, err = from_report(
        nana_script,
        "virtual-list-100k",
        root / "perf" / "fixtures" / "virtual-scales-only.json",
    )
    _from_report_same_scenario(code, report, err, "virtual_scales-only", 57)

    code, tree_report, err = from_report(
        nana_script,
        "virtual-tree-100k",
        root / "perf" / "fixtures" / "virtual-tree-scales.json",
    )
    _from_report_same_scenario(code, tree_report, err, "virtual-tree-scales", 57)

    code, table_report, err = from_report(
        nana_script,
        "text-table",
        root / "perf" / "fixtures" / "virtual-table-scales.json",
    )
    if code != EXIT_OK or table_report is None:
        errors.append(f"virtual-table-scales --from-report exit {code}: {err}")
    else:
        if table_report.get("status") != "ok":
            errors.append(f"virtual-table-scales --from-report status={table_report.get('status')}")
        if table_report.get("equivalence") != "same-scenario":
            errors.append("virtual-table-scales --from-report must be same-scenario")
        if table_report.get("relative_gate_enforceable") is not False:
            errors.append(
                "virtual-table-scales --from-report must keep relative_gate_enforceable False"
            )
        table_from_report = table_report.get("work_counters") or {}
        expected_table = {
            "live_ui_entities": 1100,
            "text_shaped": 864,
            "text_wrap_layouts": 1728,
            "text_shaped_runs": 1728,
            "text_layout_cache_hits": 0,
            "text_layout_cache_misses": 1728,
            "cache_eviction": 0,
            "wrapped_cells": 4,
        }
        for key, value in expected_table.items():
            if table_from_report.get(key) != value:
                errors.append(
                    f"virtual-table-scales --from-report must copy {key}={value}"
                )
        for key in TEXT_TABLE_EXPORTED_SHAPE_KEYS:
            if key not in table_from_report:
                errors.append(f"virtual-table-scales --from-report must contain {key}")
        if "glyph_cache_hits" in table_from_report or "glyph_cache_misses" in table_from_report:
            errors.append("virtual-table-scales --from-report must omit glyph_cache_*")

    code, live_table_report, err = from_report(
        nana_script,
        "text-table",
        root / "perf" / "fixtures" / "nana-framework-text-table.json",
    )
    if code != EXIT_OK or live_table_report is None:
        errors.append(f"nana-framework-text-table --from-report exit {code}: {err}")
    else:
        live_table_counters = live_table_report.get("work_counters") or {}
        if live_table_counters.get("glyph_cache_hits", 0) < 1:
            errors.append("nana-framework-text-table --from-report must copy real glyph_cache_hits")
        if live_table_counters.get("glyph_cache_misses", 0) < 1:
            errors.append("nana-framework-text-table --from-report must copy real glyph_cache_misses")
        if judge_runner_invariants(live_table_report, root=root).get("decision") != "ok":
            errors.append("nana-framework-text-table --from-report must pass §8.1")

    with tempfile.TemporaryDirectory() as tmp:
        catalog_fixture = load_json(root / "perf" / "fixtures" / "catalog-workloads.json")
        for scenario_id, field in (
            ("ime", "ime_script_count"),
            ("overlay", "overlay_kind_count"),
        ):
            missing_payload = copy.deepcopy(catalog_fixture)
            for row in missing_payload.get("catalog_workloads") or []:
                if row.get("id") == scenario_id:
                    row.pop(field, None)
            missing_path = Path(tmp) / f"{scenario_id}-missing-count.json"
            dump_json(missing_path, missing_payload)
            code, _, err = from_report(nana_script, scenario_id, missing_path)
            if code != EXIT_UNSUPPORTED:
                errors.append(
                    f"{scenario_id} missing {field} --from-report must exit 2, got {code}: {err}"
                )

    live_catalog = root / "perf" / "fixtures" / "nana-framework-catalog-workloads.json"
    for scenario_id, counter, value in (
        ("ime", "layout_nodes", 0),
        ("dock-workspace", "layout_nodes", 25),
        ("overlay", "layout_nodes", 1),
        ("text-editor", "layout_nodes", 0),
    ):
        code, live_report, err = from_report(nana_script, scenario_id, live_catalog)
        if code != EXIT_OK or live_report is None:
            errors.append(f"{scenario_id} live catalog --from-report exit {code}: {err}")
        elif live_report.get("status") != "ok":
            errors.append(f"{scenario_id} live catalog --from-report status={live_report.get('status')}")
        elif (live_report.get("work_counters") or {}).get(counter) != value:
            errors.append(
                f"{scenario_id} live catalog --from-report must copy {counter}={value}"
            )
        elif judge_runner_invariants(live_report, root=root).get("decision") != "ok":
            errors.append(f"{scenario_id} live catalog --from-report must pass the dirty hotspot")

    code, anim_report, err = from_report(
        nana_script,
        "animation",
        root / "perf" / "fixtures" / "nana-runtime-static-tree.json",
    )
    if code != EXIT_OK or anim_report is None:
        errors.append(f"animation live runtime --from-report exit {code}: {err}")
    elif anim_report.get("status") != "ok":
        errors.append(f"animation live runtime --from-report status={anim_report.get('status')}")
    elif (anim_report.get("work_counters") or {}).get("due_animation_samples") != 1:
        errors.append("animation live runtime --from-report must copy due_animation_samples=1")
    elif (anim_report.get("work_counters") or {}).get("animations_considered") != 1:
        errors.append("animation live runtime --from-report must copy animations_considered=1")
    elif judge_runner_invariants(anim_report, root=root).get("decision") != "ok":
        errors.append("animation live runtime --from-report must pass the sparse-advance hotspot")

    code, iced_report, err = from_report(
        iced_script,
        "static-tree-100",
        root / "perf" / "fixtures" / "iced-scenario-static-tree-100.json",
    )
    if code != EXIT_OK or iced_report is None:
        errors.append(f"iced-scenario-bench --from-report exit {code}: {err}")
    else:
        if iced_report.get("equivalence") != "same-scenario":
            errors.append("iced-scenario-bench --from-report must be same-scenario")
        if iced_report.get("relative_gate_enforceable") is not False:
            errors.append("iced-scenario-bench --from-report must keep relative gates off")
        if iced_report.get("status") != "ok":
            errors.append(f"iced-scenario-bench --from-report status={iced_report.get('status')}")
        elif (iced_report.get("metrics") or {}).get("frames_after_idle") != 0:
            errors.append("iced static-tree-100 --from-report must copy frames_after_idle=0")
        if not isinstance((iced_report.get("metrics") or {}).get("busy_probe_frames"), int) or (
            iced_report.get("metrics") or {}
        ).get("busy_probe_frames", 0) < 1:
            errors.append("iced static-tree-100 --from-report must copy busy_probe_frames > 0")

    nana_static_live = root / "perf" / "fixtures" / "nana-runtime-static-tree.json"
    nana_live = load_json(nana_static_live)
    if nana_live.get("profile") != "release" or nana_live.get("storage") == "extractor-fixture":
        errors.append("nana-runtime-static-tree.json must be a live release dump")
    live_cases = {
        case.get("nodes"): case
        for case in nana_live.get("cases") or []
        if isinstance(case, Mapping)
    }
    for nodes in (100, 1000, 5000, 10000):
        case = live_cases.get(nodes)
        if not isinstance(case, Mapping) or case.get("kind") != "full":
            errors.append(f"live nana dump must include kind=full nodes={nodes}")
        elif case.get("frames_after_idle") != 0:
            errors.append(f"live nana nodes={nodes} must have frames_after_idle=0")
    fifty_case = live_cases.get(50000)
    if isinstance(fifty_case, Mapping) and "frames_after_idle" in fifty_case:
        errors.append("live nana 50k construction must omit frames_after_idle")
    hover_case = live_cases.get(10000)
    if not isinstance(hover_case, Mapping) or "pointer_hover_transition_ms" not in hover_case:
        errors.append("live nana dump must include 10k pointer_hover_transition_ms")
    tenk_initial = hover_case.get("initial_work") if isinstance(hover_case, Mapping) else None
    if not isinstance(tenk_initial, Mapping) or "validation_nodes_scanned" not in tenk_initial:
        errors.append("live nana 10k initial_work must export validation_nodes_scanned")
    mut_case = live_cases.get(5000)
    if not isinstance(mut_case, Mapping) or "local_paint_systems_ms" not in mut_case:
        errors.append("live nana dump must include 5k local_paint_systems_ms")
    else:
        block = mut_case.get("single_node_mutations") or {}
        layout_honesty = {
            "Visibility": (
                lambda n: n != 0,
                "live nana Visibility must not report layout_nodes=0; last dump layouts 12",
            ),
            "Transform": (lambda n: n == 0, "live nana Transform must measure layout_nodes=0"),
            "Accessibility": (
                lambda n: n == 0,
                "live nana Accessibility must measure layout_nodes=0",
            ),
        }
        for kind in ("Text", "LayoutStyle", "Visibility", "Transform", "Accessibility"):
            drain = block.get(kind)
            work = drain.get("work") if isinstance(drain, Mapping) else None
            if not isinstance(drain, Mapping) or not isinstance(work, Mapping):
                errors.append(
                    f"live nana dump must include 5k single_node_mutations.{kind} WorkCounters"
                )
                continue
            if "layout_nodes" not in work:
                errors.append(
                    f"live nana dump single_node_mutations.{kind} must measure layout_nodes"
                )
                continue
            if kind == "LayoutStyle" and "hit_test_nodes_rebuilt" not in work:
                errors.append(
                    "live nana LayoutStyle must measure hit_test_nodes_rebuilt"
                )
            check = layout_honesty.get(kind)
            if check is not None and not check[0](work.get("layout_nodes")):
                errors.append(check[1])

    expected_mutation_layout = {
        "mutation-visibility": 12,
        "mutation-transform": 0,
        "mutation-a11y": 0,
    }
    for scenario_id in (
        "static-tree-100",
        "static-tree-1k",
        "static-tree-5k",
        "static-tree-10k",
        "hover",
        "mutation-paint-only",
        "mutation-text",
        "mutation-layout-style",
        "mutation-visibility",
        "mutation-transform",
        "mutation-a11y",
    ):
        code, nana_static_report, err = from_report(
            nana_script,
            scenario_id,
            nana_static_live,
        )
        if code != EXIT_OK or nana_static_report is None:
            errors.append(f"nana {scenario_id} --from-report exit {code}: {err}")
        elif nana_static_report.get("status") != "ok":
            errors.append(
                f"nana {scenario_id} --from-report status={nana_static_report.get('status')}"
            )
        elif scenario_id.startswith("static-tree-") and (
            nana_static_report.get("metrics") or {}
        ).get("frames_after_idle") != 0:
            errors.append(f"nana {scenario_id} --from-report must copy frames_after_idle=0")
        elif scenario_id in expected_mutation_layout and (
            nana_static_report.get("work_counters") or {}
        ).get("layout_nodes") != expected_mutation_layout[scenario_id]:
            errors.append(
                f"nana {scenario_id} --from-report must copy layout_nodes="
                f"{expected_mutation_layout[scenario_id]}"
            )

    hover_iced = subprocess.run(
        [
            sys.executable,
            str(iced_script),
            "--repo-root",
            str(root),
            "--scenario",
            "hover",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if hover_iced.returncode != EXIT_OK or "snapshot was removed" not in hover_iced.stdout:
        errors.append(
            f"iced hover --print-plan must report removed snapshot: {hover_iced.stdout!r} {hover_iced.stderr}"
        )

    code, hover_report, err = from_report(
        iced_script,
        "hover",
        root / "perf" / "fixtures" / "iced-scenario-hover.json",
    )
    if code != EXIT_OK or hover_report is None:
        errors.append(f"iced hover --from-report exit {code}: {err}")
    elif hover_report.get("status") != "ok" or hover_report.get("equivalence") != "same-scenario":
        errors.append("iced hover --from-report must be same-scenario ok")
    elif (hover_report.get("metrics") or {}).get("cpu_frame_ms", {}).get("p50") != 2.1:
        errors.append("iced hover --from-report must copy fixture cpu_frame_ms.p50")

    for fake_ok_id, token in (
        ("dock-workspace", "assemble_dock"),
        ("text-editor", "drain_text"),
    ):
        fake_plan = subprocess.run(
            [
                sys.executable,
                str(iced_script),
                "--repo-root",
                str(root),
                "--scenario",
                fake_ok_id,
                "--print-plan",
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if fake_plan.returncode != EXIT_OK or "unsupported" not in fake_plan.stdout:
            errors.append(
                f"iced {fake_ok_id} --print-plan must be unsupported: "
                f"{fake_plan.stdout!r} {fake_plan.stderr}"
            )
        fake_ok = {
            "source": "iced-scenario-bench",
            "status": "ok",
            "scenario_id": fake_ok_id,
            "cpu_frame_ms": {"p50": 1.0, "p95": 1.2, "p99": 1.3},
        }
        with tempfile.TemporaryDirectory() as tmp:
            fake_path = Path(tmp) / f"fake-ok-{fake_ok_id}.json"
            fake_path.write_text(json.dumps(fake_ok), encoding="utf-8")
            code, fake_report, err = from_report(iced_script, fake_ok_id, fake_path)
            if code != EXIT_UNSUPPORTED:
                errors.append(
                    f"iced {fake_ok_id} fake-ok --from-report must exit 2, got {code}: {err}"
                )
            elif fake_report is not None and (
                fake_report.get("status") == "ok"
                or fake_report.get("equivalence") == "same-scenario"
            ):
                errors.append(f"iced {fake_ok_id} fake-ok --from-report must not be same-scenario")
            try:
                extracted = extract_iced(
                    load_scenario(fake_ok_id, root),
                    fake_ok,
                    source_path=fake_path,
                )
                if extracted.get("status") == "ok":
                    errors.append(f"iced {fake_ok_id} fake-ok extract must KeyError")
            except KeyError as exc:
                if token not in key_error_reason(exc):
                    errors.append(
                        f"iced {fake_ok_id} fake-ok KeyError should name {token}: {exc}"
                    )

    code, fifty_report, err = from_report(
        iced_script,
        "static-tree-50k",
        root / "perf" / "fixtures" / "iced-scenario-static-tree-100.json",
    )
    if code != EXIT_UNSUPPORTED:
        errors.append(f"iced static-tree-50k must exit 2, got {code}: {err}")
    elif fifty_report is None:
        # runner prints JSON on stdout even for exit 2
        pass

    fifty_iced = subprocess.run(
        [
            sys.executable,
            str(iced_script),
            "--repo-root",
            str(root),
            "--scenario",
            "static-tree-50k",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if fifty_iced.returncode != EXIT_OK or "unsupported" not in fifty_iced.stdout:
        errors.append(
            f"iced static-tree-50k --print-plan must be unsupported: {fifty_iced.stdout!r}"
        )

    nana_gpu_plan = subprocess.run(
        [
            sys.executable,
            str(nana_script),
            "--repo-root",
            str(root),
            "--scenario",
            "gpu-scene-ui",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if nana_gpu_plan.returncode != EXIT_OK or "nana-gpu-scene-benchmark" not in nana_gpu_plan.stdout:
        errors.append(
            f"nana gpu-scene-ui --print-plan must name nana-gpu-scene-benchmark: "
            f"{nana_gpu_plan.stdout!r} {nana_gpu_plan.stderr}"
        )
    elif "gpu-scene-ui.json" not in nana_gpu_plan.stdout:
        errors.append(
            f"nana gpu-scene-ui --print-plan must pass --scenario gpu-scene-ui.json: "
            f"{nana_gpu_plan.stdout!r}"
        )

    gpu_live_code, gpu_live_report, gpu_live_err = from_report(
        nana_script, "gpu-scene-ui", root / "perf" / "fixtures" / "nana-gpu-scene-ui.json"
    )
    if gpu_live_code != EXIT_OK or gpu_live_report is None:
        errors.append(f"nana gpu-scene-ui live encode --from-report must exit 0: {gpu_live_err}")
    elif gpu_live_report.get("status") != "ok":
        errors.append(
            f"nana gpu-scene-ui live encode status={gpu_live_report.get('status')}"
        )
    elif gpu_live_report.get("relative_gate_enforceable") is not False:
        errors.append("nana gpu-scene-ui --from-report must keep relative_gate_enforceable False")
    elif (gpu_live_report.get("work_counters") or {}).get("draw_calls", 0) < 1:
        errors.append("nana gpu-scene-ui live encode must copy draw_calls >= 1")

    runtime_gpu_code, _, runtime_gpu_err = from_report(
        nana_script,
        "gpu-scene-ui",
        root / "perf" / "fixtures" / "nana-runtime-static-tree.json",
    )
    if runtime_gpu_code != EXIT_UNSUPPORTED:
        errors.append(
            f"nana gpu-scene-ui from a runtime-only dump must exit 2, got {runtime_gpu_code}: "
            f"{runtime_gpu_err}"
        )

    for live2d_id in ("gpu-scene-ui-live2d", "gpu-scene-ui-live2d-effect"):
        live2d_plan = subprocess.run(
            [
                sys.executable,
                str(nana_script),
                "--repo-root",
                str(root),
                "--scenario",
                live2d_id,
                "--print-plan",
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if live2d_plan.returncode != EXIT_OK or "Live2D" not in live2d_plan.stdout:
            errors.append(
                f"nana {live2d_id} --print-plan must stay Live2D unsupported: "
                f"{live2d_plan.stdout!r} {live2d_plan.stderr}"
            )
        live2d_run = subprocess.run(
            [
                sys.executable,
                str(nana_script),
                "--repo-root",
                str(root),
                "--scenario",
                live2d_id,
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if live2d_run.returncode != EXIT_UNSUPPORTED:
            errors.append(
                f"nana {live2d_id} must exit 2, got {live2d_run.returncode}: "
                f"{live2d_run.stderr}"
            )
        else:
            try:
                live2d_payload = json.loads(live2d_run.stdout)
                if live2d_payload.get("status") != "unsupported":
                    errors.append(
                        f"nana {live2d_id} status={live2d_payload.get('status')}; "
                        "do not invent a Live2D encode"
                    )
                if live2d_payload.get("metrics") or live2d_payload.get("work_counters"):
                    errors.append(f"nana {live2d_id} must not invent GPU metrics or counters")
                reason = live2d_payload.get("unsupported_reason") or ""
                if "Live2D" not in reason:
                    errors.append(
                        f"nana {live2d_id} unsupported_reason should name Live2D: {reason!r}"
                    )
            except json.JSONDecodeError as exc:
                errors.append(f"nana {live2d_id} stdout must be JSON: {exc}")

        iced_live2d = subprocess.run(
            [
                sys.executable,
                str(iced_script),
                "--repo-root",
                str(root),
                "--scenario",
                live2d_id,
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if iced_live2d.returncode != EXIT_UNSUPPORTED:
            errors.append(
                f"iced {live2d_id} must exit 2, got {iced_live2d.returncode}: "
                f"{iced_live2d.stderr}"
            )

    nana_fifty = subprocess.run(
        [
            sys.executable,
            str(nana_script),
            "--repo-root",
            str(root),
            "--scenario",
            "static-tree-50k",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if nana_fifty.returncode != EXIT_OK or "unsupported" not in nana_fifty.stdout:
        errors.append(
            f"nana static-tree-50k --print-plan must be unsupported: {nana_fifty.stdout!r}"
        )

    for unsupported_id in (
        "mutation-transform",
        "mutation-visibility",
        "mutation-a11y",
        "animation",
        "ime",
        "overlay",
        "gpu-scene-ui",
        "dock-workspace",
        "text-editor",
        "virtual-tree-100k",
    ):
        iced_unsupported = subprocess.run(
            [
                sys.executable,
                str(iced_script),
                "--repo-root",
                str(root),
                "--scenario",
                unsupported_id,
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if iced_unsupported.returncode != EXIT_UNSUPPORTED:
            errors.append(
                f"iced {unsupported_id} must exit 2, got {iced_unsupported.returncode}: "
                f"{iced_unsupported.stderr}"
            )
        else:
            try:
                payload = json.loads(iced_unsupported.stdout)
                if payload.get("status") == "ok" or payload.get("equivalence") == "same-scenario":
                    errors.append(f"iced {unsupported_id} must not emit same-scenario ok")
            except json.JSONDecodeError as exc:
                errors.append(f"iced {unsupported_id} stdout must be JSON: {exc}")

    for list_id in ("virtual-list-10k", "virtual-list-100k", "virtual-tree-10k", "virtual-tree-100k"):
        nana_list = subprocess.run(
            [
                sys.executable,
                str(nana_script),
                "--repo-root",
                str(root),
                "--scenario",
                list_id,
                "--print-plan",
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if nana_list.returncode != EXIT_OK or "--list-overscan-px" not in nana_list.stdout:
            errors.append(
                f"nana {list_id} --print-plan must pass catalog --list-overscan-px: "
                f"{nana_list.stdout!r}"
            )
        elif "160" not in nana_list.stdout:
            errors.append(
                f"nana {list_id} --print-plan must pass overscan 160px (8×20): "
                f"{nana_list.stdout!r}"
            )

    nana_table = subprocess.run(
        [
            sys.executable,
            str(nana_script),
            "--repo-root",
            str(root),
            "--scenario",
            "text-table",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if nana_table.returncode != EXIT_OK or "--table-overscan-y-px" not in nana_table.stdout:
        errors.append(
            f"nana text-table --print-plan must pass --table-overscan-y-px: "
            f"{nana_table.stdout!r}"
        )
    elif "160" not in nana_table.stdout:
        errors.append(
            f"nana text-table --print-plan must pass overscan y 160px (8×20): "
            f"{nana_table.stdout!r}"
        )
    if "--table-overscan-y-px" in (
        subprocess.run(
            [
                sys.executable,
                str(nana_script),
                "--repo-root",
                str(root),
                "--scenario",
                "virtual-list-10k",
                "--print-plan",
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        ).stdout
    ):
        errors.append("nana virtual-list-10k --print-plan must not pass table flags")

    table_iced_plan = subprocess.run(
        [
            sys.executable,
            str(iced_script),
            "--repo-root",
            str(root),
            "--scenario",
            "text-table",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if (
        table_iced_plan.returncode != EXIT_OK
        or "snapshot was removed" not in table_iced_plan.stdout
    ):
        errors.append(
            f"iced text-table --print-plan must report removed snapshot: "
            f"{table_iced_plan.stdout!r} {table_iced_plan.stderr}"
        )
    code, table_from_report, err = from_report(
        iced_script,
        "text-table",
        root / "perf" / "fixtures" / "iced-scenario-text-table.json",
    )
    if code != EXIT_OK or table_from_report is None:
        errors.append(f"iced text-table --from-report exit {code}: {err}")
    elif table_from_report.get("status") != "ok" or table_from_report.get("equivalence") != "same-scenario":
        errors.append("iced text-table --from-report must be same-scenario ok")

    gpui_script = root / "perf" / "runners" / "gpui" / "run.py"
    gpui_plan = subprocess.run(
        [
            sys.executable,
            str(gpui_script),
            "--repo-root",
            str(root),
            "--scenario",
            "static-tree-100",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if gpui_plan.returncode != EXIT_OK or "snapshot was removed" not in gpui_plan.stdout:
        errors.append(
            f"gpui static-tree-100 --print-plan must report removed snapshot: "
            f"{gpui_plan.stdout!r} {gpui_plan.stderr}"
        )
    elif "issue8" in gpui_plan.stdout:
        errors.append("gpui print-plan must not target target/performance/issue8")
    gpui_skip = subprocess.run(
        [
            sys.executable,
            str(gpui_script),
            "--repo-root",
            str(root),
            "--scenario",
            "gpu-scene-ui",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if gpui_skip.returncode != EXIT_UNSUPPORTED:
        errors.append(
            f"gpui gpu-scene-ui must exit 2, got {gpui_skip.returncode}: {gpui_skip.stderr}"
        )
    else:
        try:
            gpui_skip_report = json.loads(gpui_skip.stdout)
            if gpui_skip_report.get("status") != "unsupported":
                errors.append(f"gpui gpu-scene-ui status={gpui_skip_report.get('status')}")
            if gpui_skip_report.get("metrics"):
                errors.append("gpui unsupported path must not invent metrics")
        except json.JSONDecodeError as exc:
            errors.append(f"gpui gpu-scene-ui stdout is not JSON: {exc}")
    gpui_fixture = root / "perf" / "fixtures" / "gpui-scenario-static-tree-100.json"
    if not gpui_fixture.is_file():
        errors.append("gpui-scenario-static-tree-100.json live dump is missing")
    else:
        code, gpui_from_report, err = from_report(
            gpui_script, "static-tree-100", gpui_fixture
        )
        if code != EXIT_OK or gpui_from_report is None:
            errors.append(f"gpui static-tree-100 --from-report exit {code}: {err}")
        elif (
            gpui_from_report.get("status") != "ok"
            or gpui_from_report.get("equivalence") != "same-scenario"
        ):
            errors.append("gpui static-tree-100 --from-report must be same-scenario ok")
        elif gpui_from_report.get("relative_gate_enforceable") is not False:
            errors.append("gpui same-scenario extract must keep relative_gate_enforceable False")
        elif not isinstance(
            (gpui_from_report.get("metrics") or {}).get("cpu_frame_ms", {}).get("p50"),
            (int, float),
        ):
            errors.append("gpui same-scenario extract must copy live cpu_frame_ms.p50")
        else:
            gpui_raw = load_json(gpui_fixture)
            if gpui_raw.get("adapter", {}).get("name") == "extractor-fixture":
                errors.append("gpui static-tree-100 fixture must be a live dump, not extractor-fixture")
            if gpui_raw.get("gpu_present") is not False:
                errors.append("gpui live dump must declare gpu_present=false (TestWindow has no GPU present)")
            if "frames_after_idle" in gpui_raw or (gpui_from_report.get("metrics") or {}).get(
                "frames_after_idle"
            ) is not None:
                errors.append(
                    "gpui TestPlatform cannot observe idle redraw; frames_after_idle must stay omitted"
                )
            iced_same = extract_iced(
                load_scenario("static-tree-100", root),
                load_json(root / "perf" / "fixtures" / "iced-scenario-static-tree-100.json"),
                source_path=root / "perf" / "fixtures" / "iced-scenario-static-tree-100.json",
            )
            if not relative_gate_can_enforce(iced_same, gpui_from_report):
                errors.append(
                    "relative_gate_can_enforce should be true for live Iced+GPUI same-scenario "
                    "static-tree-100, but envelope relative_gate_enforceable stays False"
                )
            compared = compare_issue12_pair(iced_same, gpui_from_report)
            if compared.get("status") != "observation" or compared.get("can_enforce") is not True:
                errors.append(
                    "live static-tree-100 Iced+GPUI must be observation with can_enforce, "
                    f"got status={compared.get('status')!r} can_enforce={compared.get('can_enforce')!r}"
                )
            elif compared.get("named_fixed_machine") is not False:
                errors.append("laptop fixtures must not claim a named fixed machine")
            elif compared.get("relative_gate_enforceable") is not False:
                errors.append("compare_issue12_pair must keep relative_gate_enforceable False")
            elif (compared.get("metrics") or {}).get("present_ms", {}).get("status") != "not-evaluable":
                errors.append("GPUI-omitted present_ms must stay not-evaluable, not 0")
            elif (compared.get("metrics") or {}).get("memory", {}).get("status") != "not-evaluable":
                errors.append("missing Iced/GPUI process memory must stay not-evaluable, not 0")
            else:
                stuffed = copy.deepcopy(gpui_from_report)
                stuffed.setdefault("metrics", {})["present_ms"] = {
                    "p50": 0,
                    "p95": 0,
                    "p99": 0,
                }
                stuffed_cmp = compare_issue12_pair(iced_same, stuffed)
                if stuffed_cmp.get("status") != "error":
                    errors.append("GPUI stuffed present_ms=0 must honesty-error, not compare")
                empty_ok = envelope(
                    runner="iced",
                    status="ok",
                    scenario_id="static-tree-100",
                    equivalence="same-scenario",
                )
                empty_cmp = compare_issue12_pair(empty_ok, gpui_from_report)
                if empty_cmp.get("status") != "error":
                    errors.append("ok-without-metrics must honesty-error, not a vacuous pass")
                iced_named = copy.deepcopy(iced_same)
                iced_named.setdefault("machine", {})["fixed_benchmark_machine"] = True
                gpui_named = copy.deepcopy(gpui_from_report)
                gpui_named.setdefault("machine", {})["fixed_benchmark_machine"] = True
                nana_named = envelope(
                    runner="nana",
                    status="ok",
                    scenario_id="static-tree-100",
                    scenario=load_scenario("static-tree-100", root),
                    equivalence="same-scenario",
                    metrics={
                        "cpu_frame_ms": {"p50": 100.0, "p95": 120.0, "p99": 140.0},
                    },
                )
                nana_named.setdefault("machine", {})["fixed_benchmark_machine"] = True
                named_fail = compare_issue12_pair(iced_named, gpui_named, nana_named)
                if named_fail.get("status") != "failed":
                    errors.append(
                        "named-box Nana 100ms vs live Iced/GPUI must fail historical 1.15×, "
                        f"got {named_fail.get('status')!r}"
                    )

    for fixture_id in ISSUE12_FIXTURE_IDS:
        iced_path = root / "perf" / "fixtures" / f"iced-scenario-{fixture_id}.json"
        gpui_path = root / "perf" / "fixtures" / f"gpui-scenario-{fixture_id}.json"
        if not iced_path.is_file():
            errors.append(f"missing Iced live dump {iced_path.name}")
            continue
        if not gpui_path.is_file():
            errors.append(f"missing GPUI live dump {gpui_path.name}")
            continue
        code, gpui_fix, err = from_report(gpui_script, fixture_id, gpui_path)
        if code != EXIT_OK or gpui_fix is None:
            errors.append(f"gpui {fixture_id} --from-report exit {code}: {err}")
            continue
        if gpui_fix.get("equivalence") != "same-scenario" or gpui_fix.get("status") != "ok":
            errors.append(f"gpui {fixture_id} --from-report must be same-scenario ok")
        elif (gpui_fix.get("metrics") or {}).get("present_ms") is not None:
            errors.append(f"gpui {fixture_id} must omit present_ms")
        elif (gpui_fix.get("machine") or {}).get("fixed_benchmark_machine") is not False:
            errors.append(f"gpui {fixture_id} dump must not claim a named fixed machine")
        rel_summary, rel_code = evaluate_relative_paths([iced_path, gpui_path], root=root)
        if rel_code != EXIT_OK:
            errors.append(
                f"--evaluate-relative {fixture_id} expected observation exit 0, "
                f"got {rel_code} status={rel_summary.get('status')!r}"
            )
        elif rel_summary.get("status") != "observation":
            errors.append(
                f"--evaluate-relative {fixture_id} must stay observation without a named box, "
                f"got {rel_summary.get('status')!r}"
            )
        elif rel_summary.get("relative_gate_enforceable") is not False:
            errors.append("--evaluate-relative must keep relative_gate_enforceable False")

    gpu_scene = load_scenario("gpu-scene-ui", root)
    unsupported_cmp = compare_issue12_pair(
        envelope(
            runner="iced",
            status="unsupported",
            scenario_id="gpu-scene-ui",
            scenario=gpu_scene,
        ),
        envelope(
            runner="gpui",
            status="unsupported",
            scenario_id="gpu-scene-ui",
            scenario=gpu_scene,
        ),
    )
    if unsupported_cmp.get("status") != "unsupported" or unsupported_cmp.get("can_enforce"):
        errors.append("unsupported Iced+GPUI pair must stay unsupported, not a vacuous pass")
    skip_summary, skip_code = evaluate_relative_paths(
        [
            root / "perf" / "fixtures" / "iced-scenario-static-tree-100.json",
        ],
        root=root,
    )
    if skip_code != EXIT_UNSUPPORTED:
        errors.append(
            f"--evaluate-relative with only Iced must exit 2, got {skip_code} "
            f"status={skip_summary.get('status')!r}"
        )

    plan = subprocess.run(
        [
            sys.executable,
            str(iced_script),
            "--repo-root",
            str(root),
            "--scenario",
            "static-tree-100",
            "--print-plan",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if plan.returncode != EXIT_OK or "snapshot was removed" not in plan.stdout:
        errors.append(
            f"iced static-tree-100 --print-plan must report removed snapshot: {plan.stdout!r} {plan.stderr}"
        )
    elif "issue8" in plan.stdout:
        errors.append("iced print-plan must not target target/performance/issue8")
    return errors
