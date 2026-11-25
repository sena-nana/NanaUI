#!/usr/bin/env python3
"""Check each component family in isolation, including absent Rust APIs and dependencies."""
from pathlib import Path
from typing import NamedTuple
import argparse
import json
import os
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


class Family(NamedTuple):
    # Rust that must fail to compile while the family is off, the item rustc
    # must name in that error and the error code it reports. `None` for a
    # family with no public API of its own.
    probe: str | None
    missing: str | None
    error: str | None
    # Crates only this family links: absent from the base graph, present once
    # the family is on (so the absence check cannot pass on a stale name).
    dependencies: tuple[str, ...]


def missing_type(name, dependencies=()):
    return Family(f"pub use nana_ui::runtime::{name};", name, "E0432", dependencies)


# One row per feature in nana-ui's `components`; `table_problems` keeps the two
# lists equal, so a new family cannot ship without an isolation check.
FAMILIES = {
    "area-layout": missing_type("AreaLayout"),
    "calendar": missing_type("CalendarHeatmap"),
    "charts": missing_type("Chart", ("nana-ui-charts",)),
    "controls": missing_type("ReorderList"),
    "diff-view": missing_type("DiffView"),
    # The host adapters (`nana_ui::dock`, `runtime_dock_window_update`) go with it.
    "dock": missing_type("DockWorkspace"),
    "graph-canvas": missing_type("GraphCanvas"),
    "image-viewer": missing_type("ImageViewer"),
    "paint-script": missing_type("PaintScript"),
    "property-grid": missing_type("PropertyGrid"),
    # `QrCodeCanvas` paints a module matrix in every build; only the encoder is optional.
    "qr-code": Family(
        'pub fn probe() { let _ = nana_ui::runtime::QrCode::encode("x", 1.0); }',
        "encode",
        "E0599",
        ("qrcode",),
    ),
    "rich-text": missing_type("NativeMarkdown", ("pulldown-cmark", "nana-ui-diagram")),
    "terminal": missing_type("TerminalView"),
    "timeline": missing_type("TimelineEditor"),
}
# Optional dependencies of features outside `components` that a bare
# `nana-ui` must not link either.
OTHER_OPTIONAL = ("two-face", "mermaid-svg", "icu_properties", "ureq", "swash", "yazi")
PACKAGES = (
    ("nana-ui-runtime", "declarations_match_installed_component_registry"),
    ("nana-ui", "component_support::"),
)


def table_problems(components):
    """Differences between FAMILIES and nana-ui's `components` feature list."""
    listed = set(components)
    problems = [f"`{family}` is in nana-ui's `components` but has no row in FAMILIES" for family in sorted(listed - FAMILIES.keys())]
    problems += [f"FAMILIES row `{family}` is not in nana-ui's `components`" for family in sorted(FAMILIES.keys() - listed)]
    return problems


def tree_crates(output):
    """Crate names in `cargo tree --prefix none` output."""
    return {line.split()[0] for line in output.splitlines() if line.strip()}


def run(args, **kwargs):
    return subprocess.run(args, cwd=ROOT, text=True, capture_output=True, **kwargs)


def checked(result, what):
    if result.returncode:
        raise SystemExit(f"{what} failed:\n{result.stdout}{result.stderr}")
    return result


def package_features(offline):
    result = checked(run(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked", *offline]), "cargo metadata")
    return {package["name"]: package["features"] for package in json.loads(result.stdout)["packages"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    cargo = ["cargo"]
    offline = ["--offline"] if args.offline else []
    features = package_features(offline)
    problems = table_problems(features["nana-ui"]["components"])
    if problems:
        raise SystemExit("\n".join(problems))
    for family in [None, *FAMILIES, "components"]:
        for package, test in PACKAGES:
            # A family only some layers define.
            if family and family not in features[package]:
                continue
            family_features = ["--features", family] if family else []
            command = cargo + ["test", "-p", package, "--lib", "--no-default-features", "--locked", *offline, *family_features, test]
            checked(run(command), " ".join(command))
        print(f"isolated {family or 'base'}: registry and host OK", flush=True)
    with tempfile.TemporaryDirectory(prefix="nanaui-feature-probe-") as directory:
        probe = Path(directory)
        (probe / "src").mkdir()
        shutil.copyfile(ROOT / "Cargo.lock", probe / "Cargo.lock")
        environment = {**os.environ, "CARGO_TARGET_DIR": str(ROOT / "target/component-feature-probe")}
        manifest = '[package]\nname="nanaui-feature-probe"\nversion="0.0.0"\nedition="2024"\n[dependencies]\nnana-ui={path='+json.dumps(str(ROOT / "crates/nana-ui"))+',default-features=false}\n'
        (probe / "Cargo.toml").write_text(manifest)
        probe_cargo = ["--manifest-path", str(probe / "Cargo.toml"), *offline]
        probes = {family: row for family, row in FAMILIES.items() if row.probe}
        for family, row in probes.items():
            (probe / "src/lib.rs").write_text(row.probe + "\n")
            result = run(cargo + ["check", *probe_cargo], env=environment)
            if result.returncode == 0 or row.error not in result.stderr or row.missing not in result.stderr:
                raise SystemExit(f"disabled {family} remains reachable or probe failed unexpectedly:\n{result.stderr}")
        # The same probes compile with every family on: a misspelt probe would
        # otherwise fail for the wrong reason and pass the loop above.
        (probe / "src/lib.rs").write_text("".join(row.probe + "\n" for row in probes.values()))
        checked(run(cargo + ["check", *probe_cargo, "--features", "nana-ui/components"], env=environment), "probes with every family on")
        result = checked(run(cargo + ["tree", *probe_cargo, "--prefix", "none"], env=environment), "cargo tree (base)")
        forbidden = {dependency for row in FAMILIES.values() for dependency in row.dependencies} | set(OTHER_OPTIONAL)
        retained = sorted(forbidden & tree_crates(result.stdout))
        if retained:
            raise SystemExit(f"base build retains family-only dependencies: {', '.join(retained)}")
        for family, row in FAMILIES.items():
            if not row.dependencies:
                continue
            result = checked(run(cargo + ["tree", *probe_cargo, "--prefix", "none", "--features", f"nana-ui/{family}"], env=environment), f"cargo tree ({family})")
            absent = sorted(set(row.dependencies) - tree_crates(result.stdout))
            if absent:
                raise SystemExit(f"{family} no longer links {', '.join(absent)}; update its FAMILIES row")
    print("disabled Rust APIs and family-only dependencies: OK")


if __name__ == "__main__":
    main()
