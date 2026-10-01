//! Build-side remapping of rustc JSON diagnostics to `.vue` locations.

use serde_json::Value;

/// Remap one rustc JSON diagnostic line. Invalid JSON and diagnostics without
/// a generated-file span are returned byte-for-byte unchanged.
pub fn remap_json_line(line: &str, map_json: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(line) else {
        return line.to_owned();
    };
    let Ok(map) = serde_json::from_str::<Value>(map_json) else {
        return line.to_owned();
    };
    if map.get("schema").and_then(Value::as_str) != Some("nana-sfc-source-map/1") {
        return line.to_owned();
    }
    let generated = map
        .get("generated")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let segments = map
        .get("segments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut changed = false;
    if let Some(spans) = value.get_mut("spans").and_then(Value::as_array_mut) {
        for span in spans {
            changed |= remap_span(span, generated, &segments);
        }
    }
    if !changed {
        return line.to_owned();
    }
    if let Some(rendered) = value
        .get("rendered")
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
        let mut text = rendered;
        for segment in &segments {
            let source = segment
                .get("source")
                .and_then(|v| v.get("file"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !source.is_empty() {
                text = text.replace(generated, source);
            }
        }
        value["rendered"] = Value::String(text);
    }
    serde_json::to_string(&value).unwrap_or_else(|_| line.to_owned())
}

fn remap_span(span: &mut Value, generated: &str, segments: &[Value]) -> bool {
    let Some(file_name) = span.get("file_name").and_then(Value::as_str) else {
        return false;
    };
    if file_name != generated
        && !file_name.ends_with(&format!("\\{generated}"))
        && !file_name.ends_with(&format!("/{generated}"))
    {
        return false;
    }
    let line = span.get("line_start").and_then(Value::as_u64).unwrap_or(0) as usize;
    let Some(segment) = segments.iter().find(|segment| {
        let range = segment.get("generated").unwrap_or(&Value::Null);
        let start = range.get("start_line").and_then(Value::as_u64).unwrap_or(0) as usize;
        let end = range.get("end_line").and_then(Value::as_u64).unwrap_or(0) as usize;
        line >= start && line < end
    }) else {
        return false;
    };
    let source = segment.get("source").unwrap_or(&Value::Null);
    let file = source
        .get("file")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source_line = source.get("line").and_then(Value::as_u64).unwrap_or(1);
    let source_column = source.get("column").and_then(Value::as_u64).unwrap_or(1);
    span["file_name"] = Value::String(file.to_owned());
    span["line_start"] = Value::from(source_line);
    span["line_end"] = Value::from(source_line);
    span["column_start"] = Value::from(source_column);
    span["column_end"] = Value::from(source_column);
    true
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
