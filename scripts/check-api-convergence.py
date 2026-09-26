#!/usr/bin/env python3
"""Guard the retained-tree and compatibility boundaries.

This is intentionally a small source-contract check.  It does not try to
replace Rust type checking; it prevents the documented migration boundaries
from silently drifting back into a second public API or stale BrowserView
contract.
"""

from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]


def fail(message: str) -> None:
    print(f"API convergence: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> int:
    nana_ui = (ROOT / "crates/nana-ui/src/lib.rs").read_text(encoding="utf-8")
    framework = (
        ROOT / "crates/nana-ui-runtime/src/framework.rs"
    ).read_text(encoding="utf-8")
    gpu_slots = (
        ROOT / "crates/nana-ui-runtime/src/gpu_slots.rs"
    ).read_text(encoding="utf-8")
    architecture = (ROOT / "docs/architecture.md").read_text(encoding="utf-8")
    components = (ROOT / "docs/components.md").read_text(encoding="utf-8")
    application_api = (ROOT / "docs/application-api.md").read_text(encoding="utf-8")
    readme = (ROOT / "docs/README.md").read_text(encoding="utf-8")
    vue = (ROOT / "docs/vue.md").read_text(encoding="utf-8")

    if "pub use nana_ui_runtime::*" in nana_ui:
        fail("wildcard nana_ui_runtime re-export bypasses the compatibility surface")
    if "Compatibility widget surface" in nana_ui:
        fail("deprecated root widget surface must be removed")
    if "pub fn world_mut" in framework:
        fail("deprecated AppContext::world_mut must be removed")
    if "pub fn compat_world_mut" not in framework:
        fail("AppContext::compat_world_mut is missing")
    if "TextArea as Textarea" in nana_ui:
        fail("deprecated nana_ui::Textarea alias must be removed")
    if "应用内打开网页" in architecture and "未实现" in architecture:
        fail("architecture.md still describes BrowserView as unimplemented")
    if "没有应用内浏览器控件" in components:
        fail("components.md still contradicts the BrowserView contract")
    for name, text in (("application-api.md", application_api), ("README.md", readme), ("vue.md", vue)):
        if any("应用内打开网页" in line and "未实现" in line for line in text.splitlines()):
            fail(f"{name} still describes the BrowserView path as unimplemented")
        if any("nana.webview" in line and "目前未实现" in line for line in text.splitlines()):
            fail(f"{name} still describes the obsolete Vue webview proposal")
    if "proposed `WebView` (unimplemented)" in gpu_slots:
        fail("gpu_slots.rs still carries the obsolete WebView contract")

    print("API convergence: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
