//! Build-side remapping of rustc JSON diagnostics to `.vue` locations.

use serde_json::Value;

use crate::source_map::{SOURCE_MAP_SCHEMA, SourceMap};

/// Remap one rustc JSON diagnostic line. Invalid JSON and diagnostics without
/// a generated-file span are returned byte-for-byte unchanged.
pub fn remap_json_line(line: &str, map_json: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(line) else {
        return line.to_owned();
    };
    let Ok(map) = SourceMap::from_json(map_json) else {
        return line.to_owned();
    };
    if map.schema != SOURCE_MAP_SCHEMA {
        return line.to_owned();
    }
    let generated = &map.generated;
    let mut changed = false;
    let mut first_source: Option<String> = None;
    if let Some(spans) = value.get_mut("spans").and_then(Value::as_array_mut) {
        for span in spans {
            if let Some(file) = remap_span(span, generated, &map) {
                changed = true;
                if first_source.is_none() {
                    first_source = Some(file);
                }
            }
        }
    }
    if !changed {
        return line.to_owned();
    }
    if let Some(rendered) = value
        .get("rendered")
        .and_then(Value::as_str)
        .map(str::to_owned)
        && let Some(source) = first_source
        && !source.is_empty()
    {
        value["rendered"] = Value::String(rendered.replace(generated.as_str(), &source));
    }
    serde_json::to_string(&value).unwrap_or_else(|_| line.to_owned())
}

fn remap_span(span: &mut Value, generated: &str, map: &SourceMap) -> Option<String> {
    let file_name = span.get("file_name").and_then(Value::as_str)?;
    if file_name != generated
        && !file_name.ends_with(&format!("\\{generated}"))
        && !file_name.ends_with(&format!("/{generated}"))
    {
        return None;
    }
    let line_start = span.get("line_start").and_then(Value::as_u64).unwrap_or(0) as usize;
    let line_end = span
        .get("line_end")
        .and_then(Value::as_u64)
        .unwrap_or(line_start as u64) as usize;
    let col_start = span
        .get("column_start")
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize;
    let col_end = span
        .get("column_end")
        .and_then(Value::as_u64)
        .unwrap_or(col_start as u64) as usize;
    let resolved = map.generated_range_to_vue(line_start, col_start, line_end, col_end)?;
    span["file_name"] = Value::String(resolved.file.clone());
    span["line_start"] = Value::from(resolved.start_line as u64);
    span["line_end"] = Value::from(resolved.end_line as u64);
    span["column_start"] = Value::from(resolved.start_column as u64);
    span["column_end"] = Value::from(resolved.end_column as u64);
    Some(resolved.file)
}

#[cfg(test)]
mod tests {
    use super::remap_json_line;

    #[test]
    fn remaps_primary_and_child_spans() {
        let map = r#"{"schema":"nana-sfc-source-map/1","generated":"nana_views.rs","segments":[{"generated":{"start_line":3,"start_column":1,"end_line":8,"end_column":1},"source":{"file":"views/App.vue","line":12,"column":7}}]}"#;
        let input = r#"{"message":"m","spans":[{"file_name":"C:\\target\\nana_views.rs","line_start":4,"line_end":4,"column_start":2,"column_end":4,"is_primary":true},{"file_name":"other.rs","line_start":1,"line_end":1,"column_start":1,"column_end":2,"is_primary":false}],"rendered":"error: nana_views.rs:4:2"}"#;
        let out = remap_json_line(input, map);
        assert!(out.contains("views/App.vue"));
        assert!(out.contains("other.rs"));
        assert!(!out.contains("nana_views.rs:4:2"));
    }

    #[test]
    fn leaves_unknown_input_unchanged() {
        let map = r#"{"schema":"nana-sfc-source-map/1","generated":"nana_views.rs","segments":[]}"#;
        let input = r#"{"message":"m","spans":[]}"#;
        assert_eq!(remap_json_line(input, map), input);
    }
}
