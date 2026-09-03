"""CLI argument handling and exit-code contract."""
from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence
from .comparison import (
    evaluate_relative_paths,
)
from .invariants import (
    evaluate_runner_invariant_paths,
)
from .reports import (
    envelope,
)
from .schema import (
    EXIT_ERROR,
    EXIT_OK,
    EXIT_UNSUPPORTED,
    REPO_ROOT,
    UsageError,
    dump_json,
    list_harness_ids,
    validate_all_scenarios,
)




def add_common_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--scenario", help="Scenario id from perf/scenarios/")
    parser.add_argument("--all", action="store_true", help="Run every harness scenario id")
    parser.add_argument("--list", action="store_true", help="Print harness scenario ids")
    parser.add_argument("--check-schema", action="store_true", help="Validate scenario JSON")
    parser.add_argument("--output", type=Path, help="Write one JSON report")
    parser.add_argument("--output-dir", type=Path, help="Write one JSON file per scenario")
    parser.add_argument("--from-report", type=Path, help="Map an existing bench JSON; do not invoke cargo")
    parser.add_argument("--print-plan", action="store_true", help="Print cargo/commands only")
    parser.add_argument("--repo-root", type=Path, default=REPO_ROOT)



def selected_ids(args: argparse.Namespace) -> list[str]:
    if args.all:
        return list_harness_ids(args.repo_root)
    if args.scenario:
        return [args.scenario]
    raise UsageError("provide --scenario <id>, --all, --list, or --check-schema")



def write_one(args: argparse.Namespace, report: Mapping[str, Any]) -> Path | None:
    if args.output_dir is not None:
        path = args.output_dir / f"{report['runner']}-{report['scenario_id']}.json"
        dump_json(path, report)
        return path
    if args.output is not None:
        dump_json(args.output, report)
        return args.output
    dump_json(None, report)
    return None



def run_cli(
    *,
    runner: str,
    argv: Sequence[str] | None,
    plan: Callable[[str, argparse.Namespace], list[str]],
    execute: Callable[[str, argparse.Namespace], dict[str, Any]],
) -> int:
    if runner == "nana":
        description = "Issue #8 Nana scenario runner"
    elif runner in {"iced", "gpui"}:
        description = (
            f"Issue #12 {runner} observation runner (not a Nana #8 gate)"
        )
    else:
        description = f"{runner} scenario runner"
    parser = argparse.ArgumentParser(description=description)
    add_common_args(parser)
    args = parser.parse_args(argv)
    args.repo_root = args.repo_root.resolve()

    if args.check_schema:
        errors = validate_all_scenarios(args.repo_root)
        if errors:
            for error in errors:
                print(error, file=sys.stderr)
            return EXIT_ERROR
        print("scenario schema: OK")
        return EXIT_OK

    if args.list:
        for scenario_id in list_harness_ids(args.repo_root):
            print(scenario_id)
        return EXIT_OK

    try:
        ids = selected_ids(args)
    except UsageError as exc:
        print(exc, file=sys.stderr)
        return EXIT_ERROR
    if args.print_plan:
        for scenario_id in ids:
            for line in plan(scenario_id, args):
                print(line)
        return EXIT_OK

    statuses: list[str] = []
    for scenario_id in ids:
        try:
            report = execute(scenario_id, args)
        except FileNotFoundError as exc:
            report = envelope(
                runner=runner,
                status="error",
                scenario_id=scenario_id,
                error=str(exc),
                equivalence="unsupported",
            )
        except subprocess.CalledProcessError as exc:
            report = envelope(
                runner=runner,
                status="error",
                scenario_id=scenario_id,
                error=f"command failed with exit {exc.returncode}: {exc.cmd}",
                command=[str(part) for part in exc.cmd] if isinstance(exc.cmd, list) else None,
            )
        statuses.append(report["status"])
        if args.all and args.output is None and args.output_dir is None:
            args.output_dir = args.repo_root / "perf" / "reports"
        if args.all:
            path = (args.output_dir or (args.repo_root / "perf" / "reports")) / (
                f"{runner}-{scenario_id}.json"
            )
            dump_json(path, report)
            print(path)
        else:
            write_one(args, report)

    if "error" in statuses:
        return EXIT_ERROR
    if "unsupported" in statuses:
        return EXIT_UNSUPPORTED
    return EXIT_OK



def main(argv: Sequence[str] | None = None) -> int:
    from .self_tests import self_test
    parser = argparse.ArgumentParser(
        description="Issue #8 Nana gates plus Issue #12 observation helpers"
    )
    parser.add_argument("--check-schema", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--evaluate-invariants",
        nargs="+",
        metavar="REPORT",
        default=None,
        help=(
            "Judge §8.1 work invariants from runner envelope JSON files or directories. "
            "Same evaluate_invariants engine runners already attach."
        ),
    )
    parser.add_argument(
        "--evaluate-relative",
        nargs="+",
        metavar="REPORT",
        default=None,
        help="Issue #12: compare Iced/GPUI (optional Nana) dumps; honesty fail-closed observation",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="Write evaluate summary JSON (default: stdout)",
    )
    parser.add_argument("--repo-root", type=Path, default=REPO_ROOT)
    args = parser.parse_args(argv)
    root = args.repo_root.resolve()
    if args.self_test and (args.evaluate_invariants or args.evaluate_relative):
        parser.error("use --self-test or an evaluate flag, not both")
    if args.evaluate_invariants and args.evaluate_relative:
        parser.error("use --evaluate-invariants or --evaluate-relative, not both")
    if args.self_test:
        errors = self_test(root)
        if errors:
            for error in errors:
                print(error, file=sys.stderr)
            return EXIT_ERROR
        print("perf contract self-test: OK")
        return EXIT_OK
    if args.evaluate_invariants:
        try:
            summary, code = evaluate_runner_invariant_paths(
                args.evaluate_invariants,
                root=root,
            )
        except FileNotFoundError as exc:
            print(str(exc), file=sys.stderr)
            return EXIT_ERROR
        dump_json(args.output, summary)
        return code
    if args.evaluate_relative:
        try:
            summary, code = evaluate_relative_paths(
                args.evaluate_relative,
                root=root,
            )
        except FileNotFoundError as exc:
            print(str(exc), file=sys.stderr)
            return EXIT_ERROR
        dump_json(args.output, summary)
        return code
    if args.check_schema:
        errors = validate_all_scenarios(root)
        if errors:
            for error in errors:
                print(error, file=sys.stderr)
            return EXIT_ERROR
        print("scenario schema: OK")
        return EXIT_OK
    parser.error(
        "provide --check-schema, --self-test, --evaluate-invariants, or --evaluate-relative"
    )
    return EXIT_ERROR
