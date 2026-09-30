#!/usr/bin/env python3
"""Guard the retained-tree and compatibility boundaries.

This is intentionally a small source-contract check.  It does not try to
replace Rust type checking; it prevents the documented migration boundaries
from silently drifting back into a second public API or stale BrowserView
contract.
"""

from __future__ import annotations

from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1]


# Names that left the `nana_ui` root when the compatibility surface was
# removed; they live under `nana_ui::runtime`.  Consumers outside the
# default workspace build (Android, docs, rustdoc links) are not compiled by
# every CI job, so a stale root path must be caught textually.
REMOVED_ROOT_NAMES = frozenset(
    """
    AboutMetadata AboutSection ActionMenu ActionMenuItem AnchoredActionMenu AppShell AppTitleBar AppTitleBarControls
    AppearanceSection Avatar BrowseRequested Button CalendarHeatmap CalendarHeatmapActiveCell CalendarHeatmapCell CalendarHeatmapCellPaint
    CalendarHeatmapDatum CalendarHeatmapDayLabel CalendarHeatmapEvent CalendarHeatmapLabelPaint CalendarHeatmapModel CalendarHeatmapMonthLabel CalendarHeatmapOptions CalendarLevelResolver
    CalendarLevelStrategy CalendarMonthFormatter CalendarTitleFormatter CapturedStroke Card Checkbox Chip ChipDismissed
    ColorChanged ColorField ColorInput CommandPalette ConfirmDialog ContextMenu ContextMenuEvent ContextMenuItem
    DesktopShell Dialog Dock DockFloatingSurface DockPanel DockSurfaceSpec DockWorkspaceEvent DonutChart
    DonutSlice Drawer Dropdown DropdownEvent DropdownOption DropdownSelection EmptyState FormField
    GpuTextureView GpuView GpuViewMode GpuViewPalette GraphCanvas GraphCanvasAdjustment GraphCanvasEvent GraphCanvasHit
    GraphInteraction GraphMinimap GraphMinimapEvent GraphNodeContent GraphPointerButton GraphScrollDelta HIGHLIGHT_PRESENTER HighlightPresentation
    HostedTextarea IconButton IconGlyph ImageViewer ImageViewerContent ImageViewerEvent ImageViewerGeometry ImageViewerHit
    ImageViewerOffset InteractiveCard KeyCaptureEvent KeyCaptureLayer KeyInput KeymapLayer LabeledValue LevelMeter
    ListItem MarkdownBlock MarkdownBlockKind MarkdownImage MarkdownSpan MarkdownTable MarkdownTableAlignment MediaTransportBar
    MediaTransportDensity MediaTransportEvent MediaTransportIcons MediaTransportPlacement MediaTransportSlots NativeMarkdown OVERLAY_IDLE OverlayHost
    OverlayLocks OverlayVisibility OverlayVisibilityConfig PaneChrome PaneChromeAction PaneChromeActionKind PaneTree PaneTreeNode
    PathField Popover Progress ProgressCancelled QrCode QrCodeError RangeField ReorderItem
    ReorderList ReorderListEvent ReorderListPointer ReorderRowPaint RichSpan RichTextEvent SearchDropdown SearchDropdownEvent
    SearchDropdownOption SegmentedControl Select SelectOption SelectableRichText SettingsCard SettingsCollapsibleCard SettingsRow
    SidebarFooter SidebarFooterButton SidebarFrame SidebarRow SidebarRowState SidebarRowTone SidebarSection SidebarSectionSlots
    SidebarSectionState Skeleton Spinner SplitPane StatusBadge Switch SyntectHighlighter TabDragGroup
    TabDragLease TabDragSurface TabOption Tabs TabsEvent Text TextArea TextInput
    TextSelectionGroup TextSelectionGroupId TextSelectionSnapshot Textarea Thumbnail ThumbnailState TimeSeriesChart TimeSeriesLayer
    Toast Tooltip TreeDropIntent TreeDropPosition TreeNavigation TreeNode TreeView TreeViewEvent
    ValidationMessage Workspace WorkspaceRegionSlot WorkspaceResizeHandle XYPad
    """.split()
)
SCANNED_ROOTS = ("crates", "examples", "platform", "packages", "tools", "docs")
SCANNED_SUFFIXES = {".rs", ".md", ".vue", ".ts", ".js"}
SKIPPED_PARTS = {"target", "node_modules", "dist", ".git"}


def stale_root_paths() -> list[str]:
    pattern = re.compile(r"(?<![:\w])nana_ui::(\w+)\b")
    hits = []
    for top in SCANNED_ROOTS:
        for path in (ROOT / top).rglob("*"):
            if path.suffix not in SCANNED_SUFFIXES or not path.is_file():
                continue
            if SKIPPED_PARTS.intersection(path.parts):
                continue
            # Upgrade notes describe the removal itself.
            if path.name.startswith("consumer-upgrade-"):
                continue
            text = path.read_text(encoding="utf-8", errors="ignore")
            for number, line in enumerate(text.splitlines(), 1):
                # Migration tables name the old path next to its removal.
                if "已删除" in line or "removed" in line.lower():
                    continue
                for match in pattern.finditer(line):
                    if match.group(1) in REMOVED_ROOT_NAMES:
                        rel = path.relative_to(ROOT)
                        hits.append(f"{rel}:{number}: nana_ui::{match.group(1)}")
    return hits


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
    architecture = (ROOT / "docs/reference/architecture.md").read_text(encoding="utf-8")
    components = (ROOT / "docs/reference/components.md").read_text(encoding="utf-8")
    application_api = (ROOT / "docs/reference/application-api.md").read_text(encoding="utf-8")
    readme = (ROOT / "docs/index.md").read_text(encoding="utf-8")
    vue = (ROOT / "docs/reference/vue.md").read_text(encoding="utf-8")

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

    stale = stale_root_paths()
    if stale:
        fail(
            "removed root names must be referenced via nana_ui::runtime:\n  "
            + "\n  ".join(stale)
        )

    print("API convergence: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
