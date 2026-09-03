"""Harness report extraction into the common schema."""
from __future__ import annotations

from pathlib import Path
from typing import Any, Callable, Mapping
from .invariants import (
    counters_from_block,
    gpu_counters_from_observed,
    read_frames_after_idle,
)
from .reports import (
    envelope,
    find_case_by_nodes,
    percentile_fields,
)
from .schema import (
    GPU_HOST_STAGES,
    INCOMPARABLE_STATIC_TREE_50K,
    INCOMPARABLE_STATIC_TREE_50K_REASON,
    TEXT_TABLE_CACHE_GAPS,
    TEXT_TABLE_EXPORTED_SHAPE_KEYS,
    _same_number,
    catalog_table_window,
    catalog_virtual_list_window,
    gpui_scenario_bench_skip_reason,
    iced_scenario_bench_skip_reason,
    is_gpui_scenario_bench_report,
    is_iced_scenario_bench_report,
    is_shared_static_tree,
    load_json,
    load_scenario,
    nana_gpu_scene_skip_reason,
    nana_list_catalog_window_equivalence,
    nana_table_catalog_window_equivalence,
)




def extract_nana(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    *,
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    kind = scenario["kind"]
    params = scenario["params"]
    if kind == "StaticTree":
        return _extract_nana_static_tree(scenario, reports, source_paths)
    if kind == "Mutation" and params.get("kind") == "PaintOnly":
        return _extract_nana_paint_only(scenario, reports, source_paths)
    if kind == "Mutation":
        return _extract_nana_single_node_mutation(scenario, reports, source_paths)
    if kind == "Hover":
        return _extract_nana_hover(scenario, reports, source_paths)
    if kind == "VirtualList":
        return _extract_nana_virtual_list(scenario, reports, source_paths)
    if kind == "VirtualTree":
        return _extract_nana_virtual_tree(scenario, reports, source_paths)
    if kind == "Table":
        return _extract_nana_text_table(scenario, reports, source_paths)
    if kind == "Animation":
        return _extract_nana_animation(scenario, reports, source_paths)
    if kind in {"Ime", "DockWorkspace", "Overlay", "TextEditor"}:
        return _extract_nana_catalog_workload(scenario, reports, source_paths)
    if kind == "GpuScene":
        return _extract_nana_gpu_scene(scenario, reports, source_paths)
    raise KeyError(f"no Nana mapping for {scenario['id']}")



def _extract_nana_static_tree(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    nodes = scenario["params"]["nodes"]
    runtime = reports.get("runtime")
    scene = reports.get("scene")
    if runtime is None:
        raise KeyError("nana-runtime-benchmark report required")
    case = find_case_by_nodes(runtime.get("cases", []), nodes)
    if case is None:
        raise KeyError(f"nana-runtime-benchmark has no nodes={nodes} case")
    notes = [
        f"Mapped onto nana-runtime-benchmark case nodes={nodes}.",
        "construction uses enqueue_ms + initial_commit_ms; systems use initial_systems_ms when present.",
        "This is the existing Runtime tree binary, not a dedicated Scenario executable.",
        "StaticTree JSON only has params.nodes. Generation is complete-binary-heap "
        "parent(i)=i//2, NodeKind::Element div, no text (tree_mutations).",
    ]
    if nodes == INCOMPARABLE_STATIC_TREE_50K:
        raise KeyError(INCOMPARABLE_STATIC_TREE_50K_REASON)
    kind = case.get("kind", "full")
    if kind == "construction":
        notes.append(
            "This case is kind=construction: no full initial_systems pass. "
            "Required by #8 §3.1 record set is only partially covered."
        )
    metrics: dict[str, Any] = {
        "enqueue_ms": percentile_fields(case.get("enqueue_ms")),
        "initial_commit_ms": percentile_fields(case.get("initial_commit_ms")),
        "initial_systems_ms": percentile_fields(case.get("initial_systems_ms")),
        "idle_schedule_ms": percentile_fields(case.get("idle_schedule_ms")),
        "kind": kind,
    }
    frames_after_idle = read_frames_after_idle(
        case, required=False, source="nana-runtime-benchmark StaticTree"
    )
    if frames_after_idle is not None:
        metrics["frames_after_idle"] = frames_after_idle
        notes.append(
            "frames_after_idle is the §8.1 idle-frame count (non-empty take_system_work drains)."
        )
    if scene is not None:
        row = find_case_by_nodes(scene.get("rows", []), nodes)
        if row is not None:
            metrics["scene_initial_extraction_ms"] = row.get("initial_extraction_ms")
            metrics["scene_local_update_p95_ms"] = row.get("local_update_p95_ms")
            metrics["scene_idle_update_p95_ms"] = row.get("idle_update_p95_ms")
            metrics["scene_frame_graph_p95_ms"] = row.get("frame_graph_p95_ms")
            notes.append("Scene extraction/idle/frame-graph taken from nana-scene-benchmark.")
            notes.append(
                "nana-scene-benchmark applies UiScene deltas and does not flush "
                "RuntimeDocument, so FrameProfiler stage timings are not in this envelope."
            )
    primary = percentile_fields(case.get("initial_systems_ms")) or percentile_fields(
        case.get("initial_commit_ms")
    )
    if primary:
        metrics["cpu_frame_ms"] = primary
    work_counters = counters_from_block(case.get("initial_work"))
    return envelope(
        runner="nana",
        status="ok",
        scenario_id=f"static-tree-{_scale_token(nodes)}",
        scenario=scenario,
        equivalence="closest-legacy-reference",
        source_binary="nana-runtime-benchmark",
        source_report=str(source_paths.get("runtime", "")),
        mapping_notes=notes,
        metrics={key: value for key, value in metrics.items() if value is not None},
        work_counters=work_counters,
    )



def _extract_nana_paint_only(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    tree_nodes = scenario["params"]["tree_nodes"]
    runtime = reports.get("runtime")
    if runtime is None:
        raise KeyError("nana-runtime-benchmark report required")
    case = find_case_by_nodes(runtime.get("cases", []), tree_nodes)
    if case is None:
        raise KeyError(f"nana-runtime-benchmark has no nodes={tree_nodes} case")
    if "local_paint_systems_ms" not in case:
        raise KeyError(
            "local_paint_systems_ms missing; current nana-runtime-benchmark schema is required"
        )
    paint_work = counters_from_block(case.get("local_paint_work"))
    notes = [
        f"Mapped onto nana-runtime-benchmark local_paint_* at nodes={tree_nodes}.",
        "local_paint_work_nodes is render_nodes_changed from the local-paint drain.",
    ]
    if "layout_nodes" in paint_work:
        notes.append(
            "layout_nodes is measured from SystemWork/WorkCounters. "
            "The paint-only invariant is evaluable."
        )
    else:
        notes.append(
            "layout_nodes is not measured in this envelope. "
            "The paint-only invariant is not-evaluable until a report field exists."
        )
    work_counters = {
        "entities_changed_reported": case.get("local_paint_work_nodes"),
        **paint_work,
    }
    return envelope(
        runner="nana",
        status="ok",
        scenario_id="mutation-paint-only",
        scenario=scenario,
        equivalence="closest-legacy-reference",
        source_binary="nana-runtime-benchmark",
        source_report=str(source_paths.get("runtime", "")),
        mapping_notes=notes,
        metrics={
            "cpu_frame_ms": percentile_fields(case.get("local_paint_systems_ms")),
            "local_paint_commit_ms": percentile_fields(case.get("local_paint_commit_ms")),
            "local_paint_schedule_ms": percentile_fields(case.get("local_paint_schedule_ms")),
        },
        work_counters=work_counters,
    )



def _extract_nana_single_node_mutation(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    mutation_kind = scenario["params"]["kind"]
    tree_nodes = scenario["params"]["tree_nodes"]
    runtime = reports.get("runtime")
    if runtime is None:
        raise KeyError("nana-runtime-benchmark report required")
    case = find_case_by_nodes(runtime.get("cases", []), tree_nodes)
    if case is None:
        raise KeyError(f"nana-runtime-benchmark has no nodes={tree_nodes} case")
    block = (case.get("single_node_mutations") or {}).get(mutation_kind)
    if not isinstance(block, Mapping):
        raise KeyError(
            f"nana-runtime-benchmark nodes={tree_nodes} has no "
            f"single_node_mutations.{mutation_kind}"
        )
    if "systems_ms" not in block:
        raise KeyError(
            f"single_node_mutations.{mutation_kind}.systems_ms missing; "
            "current nana-runtime-benchmark schema is required"
        )
    work = counters_from_block(block.get("work"))
    notes = [
        f"Mapped onto nana-runtime-benchmark single_node_mutations.{mutation_kind} "
        f"at nodes={tree_nodes}.",
        "These drains share the 5k full case; they are not a dedicated Scenario process.",
    ]
    return envelope(
        runner="nana",
        status="ok",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence="closest-legacy-reference",
        source_binary="nana-runtime-benchmark",
        source_report=str(source_paths.get("runtime", "")),
        mapping_notes=notes,
        metrics={
            "cpu_frame_ms": percentile_fields(block.get("systems_ms")),
            "commit_ms": percentile_fields(block.get("commit_ms")),
            "schedule_ms": percentile_fields(block.get("schedule_ms")),
        },
        work_counters=work,
    )



def _extract_nana_hover(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    requested_nodes = scenario["params"]["nodes"]
    runtime = reports.get("runtime")
    if runtime is None:
        raise KeyError("nana-runtime-benchmark report required")
    cases = list(runtime.get("cases", []))
    available = [
        int(case["nodes"])
        for case in cases
        if "pointer_hover_transition_ms" in case and isinstance(case.get("nodes"), int)
    ]
    if not available:
        raise KeyError("pointer_hover_transition_ms missing from nana-runtime-benchmark report")
    if requested_nodes not in available:
        raise KeyError(
            f"nana-runtime-benchmark has no hover case nodes={requested_nodes}; "
            f"available={sorted(available)}. Refusing to substitute another tree size."
        )
    case = find_case_by_nodes(cases, requested_nodes)
    assert case is not None
    hover_work = counters_from_block(case.get("pointer_hover_work"))
    notes = [
        f"Mapped onto nana-runtime-benchmark pointer_hover_* at nodes={requested_nodes}.",
    ]
    if "layout_nodes" in hover_work:
        notes.append(
            "layout_nodes is measured from SystemWork/WorkCounters. "
            "The hover-without-size-change invariant is evaluable."
        )
    else:
        notes.append(
            "pointer_hover_work_nodes is not layout_nodes. "
            "layout_nodes is not measured in this envelope."
        )
    work_counters = {
        "pointer_hover_work_nodes": case.get("pointer_hover_work_nodes"),
        **hover_work,
    }
    return envelope(
        runner="nana",
        status="ok",
        scenario_id="hover",
        scenario=scenario,
        equivalence="closest-legacy-reference",
        source_binary="nana-runtime-benchmark",
        source_report=str(source_paths.get("runtime", "")),
        mapping_notes=notes,
        metrics={"cpu_frame_ms": percentile_fields(case.get("pointer_hover_transition_ms"))},
        work_counters=work_counters,
    )



def _extract_nana_virtual_list(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    params = scenario["params"]
    framework = reports.get("framework")
    if framework is None:
        raise KeyError("nana-framework-benchmark report required")
    items = params["items"]
    scale = None
    for row in framework.get("virtual_scales") or []:
        if row.get("kind") == "list" and row.get("logical_rows") == items:
            scale = row
            break
    notes = [
        f"Contract VirtualList items={items}, visible={params.get('visible')}, overscan={params.get('overscan')}.",
        "Gallery ui-benchmark lists remain full-layout and must not be cited as virtualization.",
    ]
    metrics: dict[str, Any] = {}
    work_counters: dict[str, Any] = {}
    source_binary = "nana-framework-benchmark"
    equivalence = "closest-legacy-reference"
    if scale is not None:
        notes.append(
            "Mapped onto nana-framework-benchmark virtual_scales[] "
            f"kind=list logical_rows={items} status={scale.get('status')}."
        )
        if scale.get("status") != "ok":
            raise KeyError(
                f"virtual_scales list/{items} status={scale.get('status')}: {scale.get('skip_reason')}"
            )
        metrics["cpu_frame_ms"] = percentile_fields(scale.get("materialize_ms"))
        metrics["window_ms"] = percentile_fields(scale.get("window_ms"))
        work_counters = {
            "live_ui_entities": scale.get("live_ui_entities"),
            "live_ui_entities_bound": scale.get("live_ui_entities_bound"),
            "visible_rows": scale.get("visible_rows"),
            "overscan_rows": scale.get("overscan_rows"),
        }
        window = catalog_virtual_list_window(params)
        equivalence, window_notes = nana_list_catalog_window_equivalence(
            scale, window, kind="list"
        )
        notes.extend(window_notes)
    elif items == 10_000 and "virtual_list_10k_materialize_ms" in framework:
        notes.append(
            "Mapped onto legacy virtual_list_10k_* fields (no virtual_scales in this report)."
        )
        notes.append(
            "Existing binary uses 10_000 rows, viewport 800px, overscan 200px, item extent 20px "
            "(about 40 visible + 10 overscan items). Contract params ask overscan=8 items."
        )
        metrics = {
            "cpu_frame_ms": percentile_fields(framework.get("virtual_list_10k_materialize_ms")),
            "virtual_list_10k_window_ms": percentile_fields(
                framework.get("virtual_list_10k_window_ms")
            ),
            "virtual_list_10k_update_ms": percentile_fields(
                framework.get("virtual_list_10k_update_ms")
            ),
            "virtual_scroll_40_visible_nodes_ms": percentile_fields(
                framework.get("virtual_scroll_40_visible_nodes_ms")
            ),
        }
    else:
        raise KeyError(
            f"nana-framework-benchmark has no virtual_scales list/{items} and no legacy 10k fields"
        )
    scenario_id = scenario["id"]
    return envelope(
        runner="nana",
        status="ok",
        scenario_id=scenario_id,
        scenario=scenario,
        equivalence=equivalence,
        source_binary=source_binary,
        source_report=str(source_paths.get("framework", "")),
        mapping_notes=notes,
        metrics={key: value for key, value in metrics.items() if value is not None},
        work_counters=work_counters,
    )



def _extract_nana_virtual_tree(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    params = scenario["params"]
    framework = reports.get("framework")
    if framework is None:
        raise KeyError("nana-framework-benchmark report required")
    items = params["items"]
    scale = None
    for row in framework.get("virtual_scales") or []:
        if row.get("kind") == "tree" and row.get("logical_rows") == items:
            scale = row
            break
    notes = [
        f"Contract VirtualTree items={items}, visible={params.get('visible')}, overscan={params.get('overscan')}.",
        "Mapped onto nana-framework-benchmark virtual_scales[] kind=tree "
        "(Fenwick VirtualTreeLayout + AppContext::materialize_virtual_tree). "
        "A VirtualList scale row is not this path.",
    ]
    if scale is None:
        raise KeyError(
            f"nana-framework-benchmark has no virtual_scales tree/{items}"
        )
    notes.append(
        "Mapped onto nana-framework-benchmark virtual_scales[] "
        f"kind=tree logical_rows={items} status={scale.get('status')}."
    )
    if scale.get("status") != "ok":
        raise KeyError(
            f"virtual_scales tree/{items} status={scale.get('status')}: {scale.get('skip_reason')}"
        )
    if scale.get("materialize_ms") is None or scale.get("live_ui_entities") is None:
        raise KeyError(
            f"virtual_scales tree/{items} status=ok without materialize_ms/live_ui_entities; "
            "fake empty ok is forbidden"
        )
    metrics = {
        "cpu_frame_ms": percentile_fields(scale.get("materialize_ms")),
        "window_ms": percentile_fields(scale.get("window_ms")),
    }
    work_counters = {
        "live_ui_entities": scale.get("live_ui_entities"),
        "live_ui_entities_bound": scale.get("live_ui_entities_bound"),
        "visible_rows": scale.get("visible_rows"),
        "overscan_rows": scale.get("overscan_rows"),
    }
    window = catalog_virtual_list_window(params)
    equivalence, window_notes = nana_list_catalog_window_equivalence(
        scale, window, kind="tree"
    )
    notes.extend(window_notes)
    return envelope(
        runner="nana",
        status="ok",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence=equivalence,
        source_binary="nana-framework-benchmark",
        source_report=str(source_paths.get("framework", "")),
        mapping_notes=notes,
        metrics={key: value for key, value in metrics.items() if value is not None},
        work_counters=work_counters,
    )



def _extract_nana_text_table(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    params = scenario["params"]
    framework = reports.get("framework")
    if framework is None:
        raise KeyError("nana-framework-benchmark report required")
    rows = params["rows"]
    columns = params["columns"]
    scale = None
    for row in framework.get("virtual_scales") or []:
        if row.get("kind") != "table" or row.get("logical_rows") != rows:
            continue
        logical_columns = row.get("logical_columns")
        if logical_columns is None or logical_columns == columns:
            scale = row
            break
    notes = [
        f"Contract Table rows={rows}, columns={columns}, "
        f"visible_rows={params.get('visible_rows')}, overscan_rows={params.get('overscan_rows')}.",
        "Mapped onto the existing nana-framework-benchmark virtual table scale path, "
        "not a dedicated Scenario process.",
        "shaping/cache: "
        + ", ".join(TEXT_TABLE_EXPORTED_SHAPE_KEYS)
        + " and extracted_text_spans copy WorkCounters when the binary exports "
        "virtual_scales[].work after shaping the "
        f"catalog wrapped_cells={params.get('wrapped_cells')} / "
        f"wrapped_cell_len={params.get('wrapped_cell_len')} cells. "
        + ", ".join(TEXT_TABLE_CACHE_GAPS)
        + " copy GlyphCache lookup/insert when WorkCounters has Some; "
        "null stays omitted.",
    ]
    metrics: dict[str, Any] = {}
    work_counters: dict[str, Any] = {}
    equivalence = "closest-legacy-reference"
    if scale is not None:
        notes.append(
            "Mapped onto nana-framework-benchmark virtual_scales[] "
            f"kind=table logical_rows={rows} logical_columns={columns} "
            f"status={scale.get('status')}."
        )
        if scale.get("status") != "ok":
            raise KeyError(
                f"virtual_scales table/{rows}x{columns} status={scale.get('status')}: "
                f"{scale.get('skip_reason')}"
            )
        metrics["cpu_frame_ms"] = percentile_fields(scale.get("materialize_ms"))
        metrics["window_ms"] = percentile_fields(scale.get("window_ms"))
        work_counters = {
            "live_ui_entities": scale.get("live_ui_entities"),
            "live_ui_entities_bound": scale.get("live_ui_entities_bound"),
            "visible_rows": scale.get("visible_rows"),
            "overscan_rows": scale.get("overscan_rows"),
            **counters_from_block(scale.get("work")),
        }
        if "text_shaped" in work_counters:
            notes.append(
                "text_shaped is WorkCounters.text_shaped (dirty TEXT set size this drain), "
                "the existing shaping-calls stand-in."
            )
        else:
            notes.append(
                "text_shaped is missing from this report. shaping calls/frame stay not-evaluable."
            )
        window = catalog_table_window(params)
        equivalence, window_notes = nana_table_catalog_window_equivalence(scale, window)
        notes.extend(window_notes)
        notes.append(
            "Most cells are short_cell_len labels; each 40-row band keeps wrapped_cells "
            "long wrapping cells (wrapped_cell_len) in column 0, shaped against the 80px column box."
        )
        if params.get("wrapped_cells") is not None:
            work_counters["wrapped_cells"] = params.get("wrapped_cells")
        if params.get("wrapped_cell_len") is not None:
            work_counters["wrapped_cell_len"] = params.get("wrapped_cell_len")
        if params.get("short_cell_len") is not None:
            work_counters["short_cell_len"] = params.get("short_cell_len")
    elif (
        rows == 10_000
        and columns == 100
        and "virtual_table_10k_x_100_materialize_ms" in framework
    ):
        notes.append(
            "Mapped onto legacy virtual_table_10k_x_100_* fields (no virtual_scales table row)."
        )
        notes.append(
            "Existing binary uses 10_000×100 logical cells, viewport 1280×800, "
            "overscan 160×200 px, row extent 20 px, column extent 80 px. "
            "Contract params ask visible_rows=40, overscan_rows=8, wrapped long cells."
        )
        notes.append(
            "Legacy 10k table fields have no WorkCounters; text_shaped and cache keys stay omitted."
        )
        metrics = {
            "cpu_frame_ms": percentile_fields(
                framework.get("virtual_table_10k_x_100_materialize_ms")
            ),
            "virtual_table_10k_x_100_window_ms": percentile_fields(
                framework.get("virtual_table_10k_x_100_window_ms")
            ),
            "virtual_table_column_resize_ms": percentile_fields(
                framework.get("virtual_table_column_resize_ms")
            ),
        }
    else:
        raise KeyError(
            f"nana-framework-benchmark has no virtual_scales table/{rows}x{columns} "
            "and no legacy virtual_table_10k_x_100_* fields"
        )
    return envelope(
        runner="nana",
        status="ok",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence=equivalence,
        source_binary="nana-framework-benchmark",
        source_report=str(source_paths.get("framework", "")),
        mapping_notes=notes,
        metrics={key: value for key, value in metrics.items() if value is not None},
        work_counters=work_counters,
    )



def _extract_nana_animation(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    active = scenario["params"]["active"]
    runtime = reports.get("runtime")
    if runtime is None:
        raise KeyError("nana-runtime-benchmark report required")
    case = runtime.get("catalog_animation")
    if not isinstance(case, Mapping):
        raise KeyError(
            "nana-runtime-benchmark has no catalog_animation; "
            "5k-tree sparse_animation_sample_ms is not the animation Scenario"
        )
    if case.get("id") not in (None, "animation"):
        raise KeyError(f"catalog_animation id={case.get('id')!r} is not animation")
    if case.get("status") not in (None, "ok"):
        raise KeyError(f"catalog_animation status={case.get('status')}")
    due = case.get("due_animation_samples")
    if due != active:
        raise KeyError(
            f"catalog_animation due_animation_samples={due} "
            f"does not match catalog active={active}"
        )
    if case.get("active") not in (None, active):
        raise KeyError(
            f"catalog_animation active={case.get('active')} "
            f"does not match catalog active={active}"
        )
    if scenario["params"].get("scheduled_idle"):
        if case.get("scheduled_idle") not in (None, True):
            raise KeyError("catalog_animation scheduled_idle must be true")
        if "idle_animation_deadline_ms" not in case:
            raise KeyError(
                "idle_animation_deadline_ms missing; catalog animation scheduled_idle=true"
            )
    work = case.get("work")
    if not isinstance(work, Mapping):
        raise KeyError("catalog_animation.work.animations_considered missing")
    considered = work.get("animations_considered")
    if considered is None:
        raise KeyError("catalog_animation.work.animations_considered missing")
    scanned = work.get("animation_deadlines_scanned")
    if scanned is None:
        raise KeyError("catalog_animation.work.animation_deadlines_scanned missing")
    notes = [
        "Mapped onto nana-runtime-benchmark catalog_animation on an isolated UiWorld.",
        "Does not reuse the 5k-tree incidental sparse_animation_sample_ms.",
    ]
    work_counters = {
        "due_animation_samples": due,
        "scheduled_animations": case.get("scheduled_animations"),
        "animation_deadlines_scanned": scanned,
        "animations_considered": considered,
    }
    return envelope(
        runner="nana",
        status="ok",
        scenario_id="animation",
        scenario=scenario,
        equivalence="closest-legacy-reference",
        source_binary="nana-runtime-benchmark",
        source_report=str(source_paths.get("runtime", "")),
        mapping_notes=notes,
        metrics={
            "cpu_frame_ms": percentile_fields(case.get("sparse_animation_sample_ms")),
            "idle_animation_deadline_ms": percentile_fields(
                case.get("idle_animation_deadline_ms")
            ),
            "scheduled_animation_deadline_ms": percentile_fields(
                case.get("scheduled_animation_deadline_ms")
            ),
        },
        work_counters=work_counters,
    )



def _find_catalog_workload(
    framework: Mapping[str, Any], scenario_id: str
) -> Mapping[str, Any]:
    for row in framework.get("catalog_workloads") or []:
        if isinstance(row, Mapping) and row.get("id") == scenario_id:
            return row
    raise KeyError(
        f"nana-framework-benchmark has no catalog_workloads id={scenario_id}"
    )



def _extract_nana_catalog_workload(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    framework = reports.get("framework")
    if framework is None:
        raise KeyError("nana-framework-benchmark report required")
    row = _find_catalog_workload(framework, scenario["id"])
    if row.get("status") != "ok":
        raise KeyError(
            f"catalog_workloads {scenario['id']} status={row.get('status')}: "
            f"{row.get('skip_reason')}"
        )
    if row.get("kind") not in (None, scenario["kind"]):
        raise KeyError(
            f"catalog_workloads {scenario['id']} kind={row.get('kind')!r} "
            f"does not match {scenario['kind']!r}"
        )
    params = scenario["params"]
    notes = [
        f"Mapped onto nana-framework-benchmark catalog_workloads[] id={scenario['id']}.",
        "These drains reuse existing Runtime APIs; they are not a dedicated Scenario process.",
    ]
    metrics: dict[str, Any] = {}
    work_counters: dict[str, Any] = {
        **counters_from_block(row.get("work")),
    }
    if row.get("live_ui_entities") is not None:
        work_counters["live_ui_entities"] = row.get("live_ui_entities")

    if scenario["kind"] == "Ime":
        scripts = list(row.get("scripts") or [])
        expected = list(params.get("scripts") or [])
        if scripts != expected:
            raise KeyError(
                f"catalog_workloads ime scripts={scripts} do not match catalog {expected}"
            )
        if params.get("include_preedit") and "preedit_ms" not in row:
            raise KeyError("catalog_workloads ime missing preedit_ms")
        if params.get("include_candidate_commit") and "commit_ms" not in row:
            raise KeyError("catalog_workloads ime missing commit_ms")
        metrics["cpu_frame_ms"] = percentile_fields(row.get("commit_ms"))
        metrics["preedit_ms"] = percentile_fields(row.get("preedit_ms"))
        if "ime_script_count" not in row:
            raise KeyError("catalog_workloads ime missing ime_script_count")
        work_counters["ime_script_count"] = row["ime_script_count"]
        notes.append(
            "set_ime_preedit + commit_ime on a focused TextInput. "
            "Does not measure the OS IME candidate window; "
            "commit_ime is the Runtime candidate-commit path only."
        )
    elif scenario["kind"] == "DockWorkspace":
        panes = row.get("panes")
        if panes != params.get("panes"):
            raise KeyError(
                f"catalog_workloads dock-workspace panes={panes} "
                f"does not match catalog panes={params.get('panes')}"
            )
        if params.get("include_splitter_resize") and "resize_ms" not in row:
            raise KeyError("catalog_workloads dock-workspace missing resize_ms")
        metrics["cpu_frame_ms"] = percentile_fields(row.get("resize_ms"))
        work_counters["panes"] = panes
        notes.append(
            "assemble_dock of an 8-leaf DockNode tree plus adjust_focused_dock_split. "
            "This is keyboard/API splitter resize, not a pointer-drag splitter."
        )
    elif scenario["kind"] == "Overlay":
        kinds = list(row.get("overlay_kinds") or [])
        expected = list(params.get("kinds") or [])
        if kinds != expected:
            raise KeyError(
                f"catalog_workloads overlay kinds={kinds} do not match catalog {expected}"
            )
        metrics["cpu_frame_ms"] = percentile_fields(row.get("activate_ms"))
        if "overlay_kind_count" not in row:
            raise KeyError("catalog_workloads overlay missing overlay_kind_count")
        work_counters["overlay_kind_count"] = row["overlay_kind_count"]
        notes.append(
            "OverlayHost activate_overlay/dismiss_overlay for Tooltip / Menu / Dialog; "
            "toggle_popover for popup. Measures activate/dismiss only; "
            "does not measure popup pointer-follow reposition."
        )
    else:
        chars = row.get("document_chars")
        if chars != params.get("document_chars"):
            raise KeyError(
                f"catalog_workloads text-editor document_chars={chars} "
                f"does not match catalog {params.get('document_chars')}"
            )
        metrics["cpu_frame_ms"] = percentile_fields(row.get("local_edit_ms"))
        if row.get("visible_lines") is not None:
            work_counters["visible_lines"] = row.get("visible_lines")
        work_counters["document_chars"] = chars
        notes.append(
            "TextArea holding catalog document_chars with a 40-line viewport height. "
            "Measures caret-local replace_text_area_selection, not a full-document reshape."
        )
        if "layout_nodes" in work_counters:
            notes.append("layout_nodes is WorkCounters.layout_nodes after the local edit drain.")
        else:
            notes.append(
                "layout_nodes is missing from this report. local-edit layout stays not-evaluable."
            )

    return envelope(
        runner="nana",
        status="ok",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence="closest-legacy-reference",
        source_binary="nana-framework-benchmark",
        source_report=str(source_paths.get("framework", "")),
        mapping_notes=notes,
        metrics={key: value for key, value in metrics.items() if value is not None},
        work_counters=work_counters,
    )



def _synthetic_gpu_ui_only(
    scenario: Mapping[str, Any],
    *,
    frame_status: str = "ran",
    include_frame: bool = True,
    materialization: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    params = scenario["params"]
    payload: dict[str, Any] = {
        "status": "ok",
        "scenario_id": scenario["id"],
        "composition": "UiOnly",
        "adapter": "test",
        "materialization": dict(materialization)
        if materialization is not None
        else {
            "viewport": params["viewport"],
            "host_texture": params["host_texture"],
            "ui_nodes": params["ui_nodes"],
            "ui_entity_count": len(params["ui_nodes"]),
            "scene_primitive_kinds": ["host-texture", "quad", "text"],
        },
        "gpu_work": {
            "gpu_upload_bytes": 128,
            "draw_calls": 3,
            "draw_batches": 2,
            "batch_rebuilds": 1,
            "gpu_buffer_reallocations": 0,
        },
        "stages": {
            "batch_ms": {"p50": 0.1, "p95": 0.2, "p99": 0.3},
            "gpu_upload_ms": {"p50": 0.05, "p95": 0.08, "p99": 0.1},
            "encode_ms": {"p50": 0.4, "p95": 0.5, "p99": 0.6},
            "submit_ms": {"p50": 0.02, "p95": 0.03, "p99": 0.04},
        },
    }
    if include_frame:
        payload["frame_stages"] = {
            name: {"status": frame_status} for name in GPU_HOST_STAGES
        }
    return payload



def _extract_nana_gpu_scene(
    scenario: Mapping[str, Any],
    reports: Mapping[str, Mapping[str, Any]],
    source_paths: Mapping[str, Path],
) -> dict[str, Any]:
    composition = scenario["params"]["composition"]
    skip = nana_gpu_scene_skip_reason(scenario)
    if skip:
        raise KeyError(skip)
    payload = reports.get("gpu")
    if payload is None:
        raise KeyError("nana-gpu-scene-benchmark report required")
    if payload.get("status") == "unsupported":
        raise KeyError(
            payload.get("unsupported_reason")
            or "nana-gpu-scene-benchmark reported unsupported without a reason"
        )
    if payload.get("status") != "ok":
        raise KeyError(
            f"nana-gpu-scene-benchmark status={payload.get('status')} is not an encoded GPU frame"
        )
    if payload.get("scenario_id") != scenario["id"]:
        raise KeyError(
            f"nana-gpu-scene-benchmark scenario_id {payload.get('scenario_id')!r} "
            f"does not match catalog {scenario['id']}"
        )
    if payload.get("composition") != "UiOnly":
        raise KeyError(
            f"nana-gpu-scene-benchmark composition {payload.get('composition')!r} "
            "does not match gpu-scene-ui UiOnly"
        )
    _require_ui_only_materialization(scenario, payload)
    _require_ran_gpu_frame_stages(payload)
    work = gpu_counters_from_observed(payload.get("gpu_work"))
    stages = payload.get("stages") if isinstance(payload.get("stages"), dict) else {}
    metrics = {
        "batch_ms": percentile_fields(stages.get("batch_ms")),
        "gpu_upload_ms": percentile_fields(stages.get("gpu_upload_ms")),
        "encode_ms": percentile_fields(stages.get("encode_ms")),
        "submit_ms": percentile_fields(stages.get("submit_ms")),
    }
    missing = [key for key, value in metrics.items() if value is None]
    if missing:
        raise KeyError(
            f"nana-gpu-scene-benchmark FrameProfiler ran GPU stages but omitted {missing}"
        )
    notes = [
        "Loaded perf/scenarios/gpu-scene-ui.json and materialized its UiOnly params "
        "(viewport, host_texture slot, ui_nodes).",
        "RuntimeDocument flush + HostTexture content slot + SceneWgpuPainter encode/submit. "
        "Not a private hosted-gpu-demo tree. No CPU readback.",
        "gpu_upload_bytes counts observed queue.write_buffer on that path. cryoglyph atlas "
        "uploads are not estimated.",
        "Batch / GpuUpload / Encode / Submit are FrameProfiler status=ran on the same "
        "encode/submit. Runtime-only drains keep those stages unsupported and omit GPU keys.",
    ]
    if payload.get("adapter"):
        notes.append(f"Adapter: {payload['adapter']}.")
    report = envelope(
        runner="nana",
        status="ok",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence="same-scenario",
        source_binary="nana-gpu-scene-benchmark",
        source_report=str(source_paths.get("gpu", "")),
        mapping_notes=notes,
        metrics=metrics,
        work_counters=work,
    )
    report["frame_stages"] = {
        name: {"status": "ran"} for name in GPU_HOST_STAGES
    }
    return report



def _require_ui_only_materialization(
    scenario: Mapping[str, Any],
    payload: Mapping[str, Any],
) -> None:
    params = scenario["params"]
    echoed = payload.get("materialization")
    if not isinstance(echoed, dict):
        raise KeyError(
            "nana-gpu-scene-benchmark must echo scenario materialization "
            "(viewport, host_texture, ui_nodes); do not run a private demo tree"
        )
    for key in ("viewport", "host_texture", "ui_nodes"):
        if echoed.get(key) != params.get(key):
            raise KeyError(
                f"nana-gpu-scene-benchmark materialization.{key}={echoed.get(key)!r} "
                f"does not match scenario JSON {params.get(key)!r}"
            )
    kinds = echoed.get("scene_primitive_kinds") or []
    if "host-texture" not in kinds:
        raise KeyError(
            "UiOnly scene must contain a host-texture primitive for the GPU content slot"
        )
    if not any(kind in kinds for kind in ("text", "quad")):
        raise KeyError(
            "UiOnly scene must contain UI primitives (text/quad), not a slot-only triangle"
        )
    entities = echoed.get("ui_entity_count")
    expected = len(params.get("ui_nodes") or [])
    if not isinstance(entities, int) or isinstance(entities, bool) or entities < expected:
        raise KeyError(
            f"UiOnly ui_entity_count={entities!r} must cover catalog ui_nodes ({expected})"
        )



def _require_ran_gpu_frame_stages(payload: Mapping[str, Any]) -> None:
    stages = payload.get("frame_stages")
    if not isinstance(stages, dict):
        raise KeyError(
            "nana-gpu-scene-benchmark ok report must include frame_stages from FrameProfiler "
            "on the same encode/submit"
        )
    for name in GPU_HOST_STAGES:
        status = (stages.get(name) or {}).get("status")
        if status != "ran":
            raise KeyError(
                f"gpu-scene-ui FrameStage.{name} status={status!r}; "
                "export timings only when a host encoded/submitted (status=ran). "
                "Runtime-only profilers keep these unsupported."
            )



def extract_iced(
    scenario: Mapping[str, Any],
    report: Mapping[str, Any],
    *,
    source_path: Path,
) -> dict[str, Any]:
    if is_iced_scenario_bench_report(report):
        return _extract_iced_scenario_bench(scenario, report, source_path=source_path)
    kind = scenario["kind"]
    cases = {case.get("scenario"): case for case in report.get("cases", [])}
    if kind == "StaticTree":
        nodes = scenario["params"]["nodes"]
        name = {100: "list-100", 1000: "list-1000"}.get(nodes)
        if name is None or name not in cases:
            raise KeyError(
                f"ui-benchmark has no list case for StaticTree nodes={nodes}; "
                "5k/10k/50k are unsupported on this legacy path. "
                "Use archived iced scenario-bench fixtures for same-scenario StaticTree."
            )
        case = cases[name]
        return envelope(
            runner="iced",
            status="ok",
            scenario_id=scenario["id"],
            scenario=scenario,
            equivalence="closest-legacy-reference",
            source_binary="ui-benchmark",
            source_report=str(source_path),
            mapping_notes=[
                f"Mapped onto Gallery ui-benchmark `{name}`.",
                "Gallery lists still layout every item (legacy full-layout). This is a reference, not virtualization.",
                "Current ui-benchmark paints through SceneWgpuPainter. "
                "same-scenario StaticTree uses archived scenario-bench fixtures, not this Gallery wrap.",
            ],
            metrics={
                "cpu_frame_ms": percentile_fields(case.get("cpu_total_ms")),
                "total_ms": percentile_fields(case.get("total_ms")),
                "view_construction_ms": percentile_fields(case.get("view_construction_ms")),
                "layout_diff_ms": percentile_fields(case.get("layout_diff_ms")),
            },
        )
    if kind in {
        "Mutation",
        "Hover",
        "VirtualList",
        "VirtualTree",
        "Table",
        "Animation",
        "Ime",
        "DockWorkspace",
        "Overlay",
        "TextEditor",
        "GpuScene",
    }:
        raise KeyError(
            f"ui-benchmark / archived iced scenario-bench has no same-Scenario {kind} adapter. "
            "Gallery lists are not a substitute. Issue #12 observation / not implemented. "
            "Fake Iced numbers are forbidden."
        )
    raise KeyError(f"no Iced mapping for {scenario['id']}")



def extract_gpui(
    scenario: Mapping[str, Any],
    report: Mapping[str, Any],
    *,
    source_path: Path,
) -> dict[str, Any]:
    """Map a gpui-scenario-bench dump. Observation only."""
    if not is_gpui_scenario_bench_report(report):
        raise KeyError(
            f"GPUI extractor requires source=gpui-scenario-bench; got {report.get('source')!r}. "
            "Fake GPUI numbers are forbidden."
        )
    return _extract_iced_scenario_bench(
        scenario,
        report,
        source_path=source_path,
        runner="gpui",
        source_name="gpui-scenario-bench",
        skip_reason=gpui_scenario_bench_skip_reason,
    )



def _extract_iced_scenario_bench(
    scenario: Mapping[str, Any],
    report: Mapping[str, Any],
    *,
    source_path: Path,
    runner: str = "iced",
    source_name: str = "iced-scenario-bench",
    skip_reason: Callable[[Mapping[str, Any]], str | None] | None = None,
) -> dict[str, Any]:
    skip_fn = skip_reason or iced_scenario_bench_skip_reason
    if report.get("status") == "unsupported":
        raise KeyError(
            report.get("unsupported_reason")
            or f"{source_name} unsupported for {scenario['id']}"
        )
    skip = skip_fn(scenario)
    if skip:
        raise KeyError(skip)
    if runner == "gpui":
        if report.get("present_ms") is not None:
            raise KeyError("GPUI must omit present_ms rather than emit 0")
        if report.get("frames_after_idle") is not None:
            raise KeyError("GPUI must omit frames_after_idle rather than emit 0")
        if report.get("gpu_present") is not False:
            raise KeyError("GPUI ok dump must declare gpu_present=false")
    kind = scenario["kind"]
    reported_id = report.get("scenario_id")
    if reported_id not in (None, scenario["id"]):
        reported_nodes = report.get("nodes")
        expected_nodes = (scenario.get("params") or {}).get("nodes") or (
            scenario.get("params") or {}
        ).get("tree_nodes")
        detail = ""
        if reported_nodes is not None or expected_nodes is not None:
            detail = f" (report nodes={reported_nodes}, scenario nodes={expected_nodes})"
        raise KeyError(
            f"{source_name} scenario_id={reported_id!r} does not match "
            f"{scenario['id']!r}{detail}"
        )
    cpu = percentile_fields(report.get("cpu_frame_ms"))
    if cpu is None:
        raise KeyError(
            f"{source_name} ok report missing cpu_frame_ms percentiles; "
            "fake or empty timings are forbidden"
        )
    notes = [str(note) for note in (report.get("notes") or []) if note is not None]
    metrics: dict[str, Any] = {
        "cpu_frame_ms": cpu,
        "view_construction_ms": percentile_fields(report.get("view_construction_ms")),
        "layout_ms": percentile_fields(report.get("layout_ms")),
        "draw_ms": percentile_fields(report.get("draw_ms")),
        "present_ms": percentile_fields(report.get("present_ms")),
        "window_ms": percentile_fields(report.get("window_ms")),
    }
    work_counters: dict[str, Any] | None = None
    if kind == "StaticTree":
        nodes = scenario["params"]["nodes"]
        reported = report.get("nodes")
        if reported != nodes:
            raise KeyError(
                f"{source_name} nodes={reported} does not match StaticTree nodes={nodes}"
            )
        tree = report.get("tree")
        if not is_shared_static_tree(tree if isinstance(tree, Mapping) else None, nodes):
            raise KeyError(
                f"{source_name} ok report is not the shared StaticTree heap "
                "(generation=complete-binary-heap, parent(i)=i//2, element-div, no text). "
                "A column of N text leaves is not same-scenario."
            )
        notes.append(
            f"Mapped onto {source_name} static_tree / static_tree_parent, the same "
            "complete-binary-heap rule as nana-runtime-benchmark::tree_mutations."
        )
        if runner == "iced" or "frames_after_idle" in report:
            metrics["frames_after_idle"] = read_frames_after_idle(
                report, required=True, source=f"{source_name} StaticTree"
            )
            busy_probe = report.get("busy_probe_frames")
            if isinstance(busy_probe, bool) or not isinstance(busy_probe, int) or busy_probe < 1:
                raise KeyError(
                    f"{source_name} StaticTree must export busy_probe_frames > 0 from a live "
                    "busy-redraw probe; refusing a stuffed frames_after_idle=0"
                )
            metrics["busy_probe_frames"] = busy_probe
        else:
            notes.append("GPUI cannot observe idle redraw; frames_after_idle omitted, not 0.")
    elif kind == "Mutation":
        nodes = scenario["params"]["tree_nodes"]
        mutation_kind = scenario["params"]["kind"]
        if report.get("nodes") != nodes:
            raise KeyError(
                f"{source_name} nodes={report.get('nodes')} does not match "
                f"Mutation tree_nodes={nodes}"
            )
        mutation = report.get("mutation") if isinstance(report.get("mutation"), Mapping) else {}
        if mutation.get("kind") != mutation_kind:
            raise KeyError(
                f"{source_name} mutation.kind={mutation.get('kind')!r} does not match "
                f"{mutation_kind!r}"
            )
        if mutation.get("single_node") is not True:
            raise KeyError(f"{source_name} Mutation report must set mutation.single_node=true")
        tree = report.get("tree")
        if not is_shared_static_tree(tree if isinstance(tree, Mapping) else None, nodes):
            raise KeyError(
                f"{source_name} Mutation tree is not the shared complete-binary-heap"
            )
        engine = "Iced" if runner == "iced" else "GPUI"
        notes.append(
            f"Mapped onto {source_name} Mutation {mutation_kind} at "
            f"tree_nodes={nodes}, same heap as Nana tree_mutations. Single-node change; "
            f"{engine} has no WorkCounters.layout_nodes so paint/a11y layout invariants stay "
            "not-evaluable."
        )
    elif kind == "Hover":
        nodes = scenario["params"]["nodes"]
        if report.get("nodes") != nodes:
            raise KeyError(
                f"{source_name} nodes={report.get('nodes')} does not match Hover nodes={nodes}"
            )
        tree = report.get("tree")
        if not is_shared_static_tree(tree if isinstance(tree, Mapping) else None, nodes):
            raise KeyError(
                f"{source_name} Hover tree is not the shared complete-binary-heap"
            )
        engine = "Iced" if runner == "iced" else "GPUI"
        notes.append(
            f"Mapped onto {source_name} Hover at nodes={nodes}. "
            "Same heap as Nana tree_mutations; last two nodes toggle hover style. "
            f"{engine} has no WorkCounters.layout_nodes; hover_without_size_change stays "
            "not-evaluable."
        )
        work_counters = {"nodes": nodes}
    elif kind == "VirtualList":
        params = scenario["params"]
        items = params["items"]
        virtual = report.get("virtualization")
        if not isinstance(virtual, Mapping):
            raise KeyError(
                f"{source_name} VirtualList ok report missing virtualization block"
            )
        if virtual.get("logical_items") != items:
            raise KeyError(
                f"{source_name} logical_items={virtual.get('logical_items')} "
                f"does not match VirtualList items={items}"
            )
        live = virtual.get("live_ui_entities")
        bound = virtual.get("live_ui_entities_bound")
        if not isinstance(live, int) or live <= 0:
            raise KeyError(
                f"{source_name} VirtualList must report a positive live_ui_entities count"
            )
        if live == items:
            raise KeyError(
                f"{source_name} VirtualList live_ui_entities={live} equals logical "
                f"items={items}; that is a full widget list, not Nana virtualization"
            )
        if isinstance(bound, int) and live > bound:
            raise KeyError(
                f"{source_name} live_ui_entities={live} exceeds bound={bound}"
            )
        window = catalog_virtual_list_window(params)
        if virtual.get("visible") != window["visible"]:
            raise KeyError(
                f"{source_name} VirtualList visible={virtual.get('visible')} "
                f"does not match catalog visible={window['visible']}"
            )
        if virtual.get("overscan") != window["overscan"]:
            raise KeyError(
                f"{source_name} VirtualList overscan={virtual.get('overscan')} "
                f"does not match catalog overscan={window['overscan']} items"
            )
        if not _same_number(virtual.get("item_extent_px"), window["item_extent_px"]):
            raise KeyError(
                f"{source_name} VirtualList item_extent_px={virtual.get('item_extent_px')} "
                f"does not match catalog item_extent_px={window['item_extent_px']}"
            )
        notes.append(
            f"Mapped onto {source_name} VirtualList items={items} "
            f"visible={window['visible']} overscan={window['overscan']} "
            f"({window['overscan_px']}px) item_extent={window['item_extent_px']} "
            f"(viewport {window['viewport_px']}px). Only the catalog window is materialized."
        )
        work_counters = {
            "live_ui_entities": live,
            "live_ui_entities_bound": bound,
            "visible_rows": virtual.get("visible"),
            "overscan_rows": virtual.get("overscan"),
        }
    elif kind == "Table":
        params = scenario["params"]
        rows = params["rows"]
        columns = params["columns"]
        virtual = report.get("virtualization")
        if not isinstance(virtual, Mapping):
            raise KeyError(
                f"{source_name} Table ok report missing virtualization block"
            )
        if virtual.get("logical_rows") != rows or virtual.get("logical_columns") != columns:
            raise KeyError(
                f"{source_name} Table logical={virtual.get('logical_rows')}x"
                f"{virtual.get('logical_columns')} does not match catalog "
                f"{rows}x{columns}"
            )
        live = virtual.get("live_ui_entities")
        bound = virtual.get("live_ui_entities_bound")
        if not isinstance(live, int) or live <= 0:
            raise KeyError(
                f"{source_name} Table must report a positive live_ui_entities count"
            )
        if live == rows * columns:
            raise KeyError(
                f"{source_name} Table live_ui_entities={live} equals logical "
                f"cells={rows}x{columns}; that is a full table, not Nana virtualization"
            )
        if isinstance(bound, int) and live > bound:
            raise KeyError(
                f"{source_name} Table live_ui_entities={live} exceeds bound={bound}"
            )
        window = catalog_table_window(params)
        if virtual.get("visible_rows") != window["visible_rows"]:
            raise KeyError(
                f"{source_name} Table visible_rows={virtual.get('visible_rows')} "
                f"does not match catalog visible_rows={window['visible_rows']}"
            )
        if virtual.get("overscan_rows") != window["overscan_rows"]:
            raise KeyError(
                f"{source_name} Table overscan_rows={virtual.get('overscan_rows')} "
                f"does not match catalog overscan_rows={window['overscan_rows']}"
            )
        if virtual.get("visible_columns") != window["visible_columns"]:
            raise KeyError(
                f"{source_name} Table visible_columns={virtual.get('visible_columns')} "
                f"does not match catalog visible_columns={window['visible_columns']}"
            )
        if virtual.get("overscan_columns") != window["overscan_columns"]:
            raise KeyError(
                f"{source_name} Table overscan_columns={virtual.get('overscan_columns')} "
                f"does not match catalog overscan_columns={window['overscan_columns']}"
            )
        invented_shape = None
        if isinstance(report.get("work_counters"), Mapping):
            invented_shape = report["work_counters"].get("text_shaped")
        if invented_shape is not None:
            raise KeyError(
                f"{source_name} must not invent WorkCounters.text_shaped; "
                "leave the catalog invariant not-evaluable"
            )
        notes.append(
            f"Mapped onto {source_name} Table {rows}x{columns} "
            f"visible={window['visible_rows']}x{window['visible_columns']} "
            f"overscan={window['overscan_rows']}x{window['overscan_columns']} "
            f"(viewport {window['viewport_width_px']}x{window['viewport_height_px']}px, "
            f"overscan {window['overscan_x_px']}x{window['overscan_y_px']}px). "
            "Only the catalog window is materialized."
        )
        work_counters = {
            "live_ui_entities": live,
            "live_ui_entities_bound": bound,
            "visible_rows": virtual.get("visible_rows"),
            "overscan_rows": virtual.get("overscan_rows"),
            "visible_columns": virtual.get("visible_columns"),
            "overscan_columns": virtual.get("overscan_columns"),
        }
    else:
        raise KeyError(
            f"{source_name} has no same-scenario mapping for {scenario['id']} "
            f"(kind={kind})"
        )
    payload = envelope(
        runner=runner,
        status="ok",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence="same-scenario",
        source_binary=source_name,
        source_report=str(source_path),
        mapping_notes=notes,
        metrics={key: value for key, value in metrics.items() if value is not None},
        work_counters=work_counters,
    )
    identity = (
        report.get("machine_identity")
        if isinstance(report.get("machine_identity"), Mapping)
        else {}
    )
    machine = payload.get("machine")
    if isinstance(machine, dict):
        machine["fixed_benchmark_machine"] = (
            identity.get("fixed_benchmark_machine") is True
        )
    return payload



def _scale_token(nodes: int) -> str:
    if nodes >= 1000 and nodes % 1000 == 0:
        return f"{nodes // 1000}k"
    return str(nodes)



def _extract_nana_framework_fixture(
    root: Path, scenario: Mapping[str, Any] | str, fixture_name: str
) -> dict[str, Any]:
    path = root / "perf" / "fixtures" / fixture_name
    loaded = load_scenario(scenario, root) if isinstance(scenario, str) else scenario
    return extract_nana(
        loaded,
        {"framework": load_json(path)},
        source_paths={"framework": path},
    )



def _require_same_scenario(
    errors: list[str],
    report: Mapping[str, Any],
    label: str,
    *,
    live_ui_entities: int | None = None,
    shared_note: str | None = None,
) -> None:
    if report.get("status") != "ok" or report.get("equivalence") != "same-scenario":
        errors.append(f"{label} live dump with catalog window must be same-scenario")
    if report.get("relative_gate_enforceable") is not False:
        errors.append(f"{label} same-scenario must keep relative_gate_enforceable False")
    if live_ui_entities is not None and (report.get("work_counters") or {}).get(
        "live_ui_entities"
    ) != live_ui_entities:
        errors.append(f"{label} must copy live_ui_entities={live_ui_entities}")
    if shared_note is not None and not any(
        shared_note in str(note) for note in (report.get("mapping_notes") or [])
    ):
        errors.append(f"{label} notes must mention {shared_note}")



def _named_invariant(report: Mapping[str, Any], name: str) -> dict[str, Any] | None:
    for item in report.get("invariants") or []:
        if item.get("name") == name and isinstance(item, dict):
            return item
    return None



def _named_invariant_status(report: Mapping[str, Any], name: str) -> str | None:
    item = _named_invariant(report, name)
    if item is None or item.get("status") is None:
        return None
    return str(item.get("status"))



def _named_invariant_ok_measured(report: Mapping[str, Any], name: str, measured: Any) -> bool:
    item = _named_invariant(report, name)
    return (
        item is not None
        and item.get("status") == "ok"
        and item.get("measured") == measured
    )



def gpui_unsupported(scenario: Mapping[str, Any]) -> dict[str, Any]:
    """Envelope when ``adapter.py`` is missing. Exit 2; not a Nana #8 gate."""
    return envelope(
        runner="gpui",
        status="unsupported",
        scenario_id=scenario["id"],
        scenario=scenario,
        equivalence="unsupported",
        unsupported_reason=(
            "GPUI adapter missing. Exit 2; fake numbers forbidden. Not a Nana #8 gate."
        ),
        plug_in="Issue #12: restore adapter.py; do not invent timings.",
    )
