//! CSS layout parity harness（测试专用）。
//!
//! - Nana：`LayoutStyle` → [`nana_ui_vue::measure_layout`]
//! - 参照：可选 `webview-ref`（wry/WKWebView）或 fixture 内嵌 `expected` 盒
//! - **不**进入 `nana-ui` 默认依赖 / 产品运行时

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nana_ui_vue::{LayoutNode, LayoutStyle, LayoutStyleCss, MeasuredBox, measure_layout};
use serde::{Deserialize, Serialize};

pub mod cases;

#[cfg(feature = "webview-ref")]
pub mod webview;

/// 默认容差（逻辑像素）。
pub const DEFAULT_TOLERANCE_PX: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaseStatus {
    /// 当前可测且应通过。
    Pass,
    /// 实现缺口，默认 `#[ignore]`。
    Ignore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureNode {
    pub id: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub class: Vec<String>,
    #[serde(default)]
    pub children: Vec<FixtureNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedBox {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureCase {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub status: CaseStatus,
    /// 盘点缺口编号，如 `P0-1`；`ignore` 用例必填。
    #[serde(default)]
    pub gap: Option<String>,
    #[serde(default)]
    pub gap_note: Option<String>,
    pub viewport: [f32; 2],
    #[serde(default = "default_tolerance")]
    pub tolerance_px: f32,
    pub tree: FixtureNode,
    /// CSS/WebView 期望盒；缺省时仅跑 Nana 自洽量测（结构非空）。
    #[serde(default)]
    pub expected: Vec<ExpectedBox>,
}

fn default_tolerance() -> f32 {
    DEFAULT_TOLERANCE_PX
}

impl Default for CaseStatus {
    fn default() -> Self {
        Self::Pass
    }
}

#[derive(Debug, Clone)]
pub struct BoxDelta {
    pub id: String,
    pub field: &'static str,
    pub expected: f32,
    pub actual: f32,
    pub delta: f32,
}

#[derive(Debug, Clone)]
pub struct CompareReport {
    pub case_id: String,
    pub ok: bool,
    pub deltas: Vec<BoxDelta>,
    pub nana: BTreeMap<String, MeasuredBox>,
}

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

pub fn load_fixture(path: &Path) -> Result<FixtureCase, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load_all_fixtures() -> Result<Vec<(PathBuf, FixtureCase)>, String> {
    let dir = fixtures_dir();
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let c = load_fixture(&p)?;
            Ok((p, c))
        })
        .collect()
}

pub fn fixture_to_layout_node(node: &FixtureNode) -> LayoutNode {
    let mut style = LayoutStyle::default();
    let classes: Vec<String> = node.class.clone();
    style.apply_class_layout_hints(&classes);
    // Percent resolved against parent during measure — do not bake with viewport here.
    style.apply_css_text(&node.style, None, None);
    let children = node.children.iter().map(fixture_to_layout_node).collect();
    LayoutNode::with_children(node.id.clone(), style, children)
}

pub fn measure_nana(case: &FixtureCase) -> BTreeMap<String, MeasuredBox> {
    let root = fixture_to_layout_node(&case.tree);
    measure_layout(&root, case.viewport[0], case.viewport[1])
        .into_iter()
        .collect()
}

pub fn compare_to_expected(case: &FixtureCase) -> CompareReport {
    let nana = measure_nana(case);
    let tol = case.tolerance_px;
    let mut deltas = Vec::new();
    for exp in &case.expected {
        let Some(got) = nana.get(&exp.id) else {
            deltas.push(BoxDelta {
                id: exp.id.clone(),
                field: "missing",
                expected: 0.0,
                actual: 0.0,
                delta: f32::INFINITY,
            });
            continue;
        };
        check_field(&mut deltas, &exp.id, "x", exp.x, got.x, tol);
        check_field(&mut deltas, &exp.id, "y", exp.y, got.y, tol);
        check_field(&mut deltas, &exp.id, "w", exp.w, got.width, tol);
        check_field(&mut deltas, &exp.id, "h", exp.h, got.height, tol);
    }
    CompareReport {
        case_id: case.id.clone(),
        ok: deltas.is_empty(),
        deltas,
        nana,
    }
}

fn check_field(
    deltas: &mut Vec<BoxDelta>,
    id: &str,
    field: &'static str,
    expected: f32,
    actual: f32,
    tol: f32,
) {
    let delta = (expected - actual).abs();
    if delta > tol {
        deltas.push(BoxDelta {
            id: id.to_string(),
            field,
            expected,
            actual,
            delta,
        });
    }
}

/// 生成 WebView 参照用 HTML（`data-id` + inline style + box-sizing:border-box）。
pub fn fixture_to_html(case: &FixtureCase) -> String {
    let mut body = String::new();
    render_html_node(&case.tree, &mut body);
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8"/>
<style>
  html, body {{ margin: 0; padding: 0; width: {w}px; height: {h}px; }}
  * {{ box-sizing: border-box; }}
</style>
</head>
<body>
{body}
</body>
</html>
"#,
        w = case.viewport[0],
        h = case.viewport[1],
        body = body
    )
}

fn render_html_node(node: &FixtureNode, out: &mut String) {
    let class = node.class.join(" ");
    out.push_str(&format!(
        r#"<div data-id="{id}" class="{class}" style="{style}">"#,
        id = html_escape(&node.id),
        class = html_escape(&class),
        style = html_escape(&node.style),
    ));
    for child in &node.children {
        render_html_node(child, out);
    }
    out.push_str("</div>");
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// JS snippet：读取全部 `[data-id]` 的 getBoundingClientRect。
pub const WEBVIEW_MEASURE_JS: &str = r#"
JSON.stringify(Array.from(document.querySelectorAll('[data-id]')).map((el) => {
  const r = el.getBoundingClientRect();
  return { id: el.getAttribute('data-id'), x: r.x, y: r.y, w: r.width, h: r.height };
}))
"#;

pub fn parse_webview_boxes(json: &str) -> Result<Vec<ExpectedBox>, String> {
    serde_json::from_str(json).map_err(|e| e.to_string())
}

pub fn compare_maps(
    case_id: &str,
    expected: &BTreeMap<String, MeasuredBox>,
    actual: &BTreeMap<String, MeasuredBox>,
    tol: f32,
) -> CompareReport {
    let mut deltas = Vec::new();
    for (id, exp) in expected {
        let Some(got) = actual.get(id) else {
            deltas.push(BoxDelta {
                id: id.clone(),
                field: "missing",
                expected: 0.0,
                actual: 0.0,
                delta: f32::INFINITY,
            });
            continue;
        };
        check_field(&mut deltas, id, "x", exp.x, got.x, tol);
        check_field(&mut deltas, id, "y", exp.y, got.y, tol);
        check_field(&mut deltas, id, "w", exp.width, got.width, tol);
        check_field(&mut deltas, id, "h", exp.height, got.height, tol);
    }
    CompareReport {
        case_id: case_id.to_string(),
        ok: deltas.is_empty(),
        deltas,
        nana: actual.clone(),
    }
}

pub fn expected_to_map(boxes: &[ExpectedBox]) -> BTreeMap<String, MeasuredBox> {
    boxes
        .iter()
        .map(|b| (b.id.clone(), MeasuredBox::new(b.x, b.y, b.w, b.h)))
        .collect()
}

pub fn format_report(report: &CompareReport) -> String {
    if report.ok {
        return format!("{} OK", report.case_id);
    }
    let mut lines = vec![format!(
        "{} FAIL ({} deltas)",
        report.case_id,
        report.deltas.len()
    )];
    for d in &report.deltas {
        lines.push(format!(
            "  {} {}: expected {:.2} got {:.2} (Δ={:.2})",
            d.id, d.field, d.expected, d.actual, d.delta
        ));
    }
    lines.join("\n")
}
