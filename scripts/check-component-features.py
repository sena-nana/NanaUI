#!/usr/bin/env python3
"""Check each component family in isolation, including absent Rust APIs and dependencies."""
from pathlib import Path
import argparse
import json
import os
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FAMILIES = {
    "calendar": "CalendarHeatmap", "charts": "TimeSeriesChart", "controls": "ReorderList",
    "graph-canvas": "GraphCanvas", "image-viewer": "ImageViewer", "rich-text": "NativeMarkdown",
}

def run(args, **kwargs):
    return subprocess.run(args, cwd=ROOT, text=True, capture_output=True, **kwargs)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    cargo = ["cargo"]
    offline = ["--offline"] if args.offline else []
    for family in [None, *FAMILIES, "components"]:
        features = ["--features", family] if family else []
        for package, test in [("nana-ui-runtime", "declarations_match_installed_component_registry"), ("nana-ui-vue", "optional_tags_report_missing_features")]:
            result = run(cargo + ["test", "-p", package, "--lib", "--no-default-features", "--locked", *offline, *features, test])
            if result.returncode:
                raise SystemExit(result.stdout + result.stderr)
        result = run(cargo + ["check", "-p", "nana-ui", "--lib", "--no-default-features", "--locked", *offline, *features])
        if result.returncode:
            raise SystemExit(result.stderr)
        print(f"isolated {family or 'base'}: registry, Vue and host OK", flush=True)
    with tempfile.TemporaryDirectory(prefix="nanaui-feature-probe-") as directory:
        probe = Path(directory)
        (probe / "src").mkdir()
        shutil.copyfile(ROOT / "Cargo.lock", probe / "Cargo.lock")
        environment = {**os.environ, "CARGO_TARGET_DIR": str(ROOT / "target/component-feature-probe")}
        manifest = '[package]\nname="nanaui-feature-probe"\nversion="0.0.0"\nedition="2024"\n[dependencies]\nnana-ui={path='+json.dumps(str(ROOT / "crates/nana-ui"))+',default-features=false}\n'
        (probe / "Cargo.toml").write_text(manifest)
        for family, component in FAMILIES.items():
            (probe / "src/lib.rs").write_text(f"pub use nana_ui::runtime::{component};\n")
            result = run(cargo + ["check", "--manifest-path", str(probe / "Cargo.toml"), *offline], env=environment)
            if result.returncode == 0 or "E0432" not in result.stderr or component not in result.stderr:
                raise SystemExit(f"disabled {family} remains reachable or probe failed unexpectedly:\n{result.stderr}")
        result = run(cargo + ["tree", "--manifest-path", str(probe / "Cargo.toml"), "--prefix", "none", *offline], env=environment)
        if result.returncode or "pulldown-cmark" in result.stdout or "two-face" in result.stdout:
            raise SystemExit("base build retains family-only dependencies\n" + result.stderr)
    print("disabled Rust APIs and family-only dependencies: OK")

if __name__ == "__main__":
    main()
