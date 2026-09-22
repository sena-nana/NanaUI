//! Human and machine views of a decoded `.nlog`. These are export formats;
//! nothing on a hot path produces text.

use std::fmt::Write as _;

use crate::files::iso8601;
use crate::metric::bucket_floor;
use crate::nlog::{Entry, MetricSample, NlogFile, Value};
use crate::schema::SchemaKey;

#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Replace the user's home directory with `~` in free-text fields (fault
    /// messages, session info, markers).
    pub redact_home: bool,
}

struct Redactor {
    home: Option<String>,
}

impl Redactor {
    fn new(options: &ExportOptions) -> Self {
        let home = options
            .redact_home
            .then(|| {
                std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .ok()
            })
            .flatten()
            .filter(|home| home.len() > 1);
        Self { home }
    }

    fn apply<'a>(&self, text: &'a str) -> std::borrow::Cow<'a, str> {
        match &self.home {
            Some(home) if text.contains(home.as_str()) => text.replace(home.as_str(), "~").into(),
            _ => text.into(),
        }
    }
}

fn event_label(file: &NlogFile, key: SchemaKey) -> String {
    file.event_name(key)
        .map_or_else(|| format!("{}:{}", key.domain.0, key.id), str::to_owned)
}

fn metric_label(file: &NlogFile, key: SchemaKey) -> String {
    file.metric_name(key)
        .map_or_else(|| format!("{}:{}", key.domain.0, key.id), str::to_owned)
}

fn thread_label(file: &NlogFile, thread: u32) -> String {
    file.threads
        .get(&thread)
        .cloned()
        .unwrap_or_else(|| format!("#{thread}"))
}

fn field_names(file: &NlogFile, key: SchemaKey, count: usize) -> Vec<String> {
    let schema = file.event_schemas.get(&key);
    (0..count)
        .map(|i| {
            schema
                .and_then(|s| s.fields.get(i))
                .map_or_else(|| format!("f{i}"), |(name, _)| name.clone())
        })
        .collect()
}

/// Approximate quantile from log2 buckets: the lower bound of the bucket
/// holding it.
pub fn histogram_quantile(buckets: &[(u8, u64)], count: u64, q: f64) -> u64 {
    if count == 0 {
        return 0;
    }
    let target = ((count as f64) * q).ceil().max(1.0) as u64;
    let mut seen = 0u64;
    for (bucket, n) in buckets {
        seen = seen.saturating_add(*n);
        if seen >= target {
            return bucket_floor(*bucket);
        }
    }
    buckets.last().map_or(0, |(b, _)| bucket_floor(*b))
}

pub fn to_text(file: &NlogFile, options: &ExportOptions) -> String {
    let redact = Redactor::new(options);
    let h = &file.header;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# {} {} ({}) build={} framework={} reason={}",
        h.app_name, h.app_version, h.app_id, h.build_id, h.framework_version, h.reason
    );
    let _ = writeln!(
        out,
        "# session={:016x} pid={} os={} arch={} started={}",
        h.session_id,
        h.pid,
        h.os,
        h.arch,
        iso8601(h.wall_start_unix_ns)
    );
    for (key, value) in &h.extra {
        let _ = writeln!(out, "# {key}={}", redact.apply(value));
    }
    if file.truncated {
        let _ = writeln!(
            out,
            "# file is truncated; entries end at the last intact chunk"
        );
    }
    for entry in &file.entries {
        let ts = entry.ts_ns();
        let _ = write!(
            out,
            "{} +{:>10.3}s ",
            iso8601(h.wall_start_unix_ns.saturating_add(ts)),
            ts as f64 / 1e9
        );
        match entry {
            Entry::Event {
                key,
                thread,
                values,
                ..
            }
            | Entry::Fault {
                key,
                thread,
                values,
                ..
            } => {
                let severity = file
                    .event_schemas
                    .get(key)
                    .map_or("?", |s| s.severity.as_str());
                let tag = if matches!(entry, Entry::Fault { .. }) {
                    "FAULT "
                } else {
                    ""
                };
                let _ = write!(
                    out,
                    "[{severity:<5}] {tag}{} thread={}",
                    event_label(file, *key),
                    thread_label(file, *thread)
                );
                for (name, value) in field_names(file, *key, values.len()).iter().zip(values) {
                    let _ = write!(out, " {name}={value}");
                }
                if let Entry::Fault {
                    message: Some(message),
                    ..
                } = entry
                {
                    let _ = write!(out, " :: {}", redact.apply(message));
                }
                out.push('\n');
            }
            Entry::Metrics { samples, .. } => {
                let _ = writeln!(out, "[metrics]");
                for sample in samples {
                    let name = metric_label(file, sample.key());
                    let unit = file
                        .metric_schemas
                        .get(&sample.key())
                        .map_or("", |s| s.unit.as_str());
                    match sample {
                        MetricSample::Counter { total, delta, .. } => {
                            let _ = writeln!(out, "    {name} total={total} +{delta} {unit}");
                        }
                        MetricSample::Gauge { value, .. } => {
                            let _ = writeln!(out, "    {name} = {value} {unit}");
                        }
                        MetricSample::Histogram { sample, .. } => {
                            let mean = sample.sum / sample.count.max(1);
                            let _ = writeln!(
                                out,
                                "    {name} n={} min={} p50~{} p99~{} max={} mean={mean} {unit}",
                                sample.count,
                                sample.min,
                                histogram_quantile(&sample.buckets, sample.count, 0.5),
                                histogram_quantile(&sample.buckets, sample.count, 0.99),
                                sample.max,
                            );
                        }
                    }
                }
            }
            Entry::SessionInfo { pairs, .. } => {
                let _ = write!(out, "[info]");
                for (key, value) in pairs {
                    let _ = write!(out, " {key}={}", redact.apply(value));
                }
                out.push('\n');
            }
            Entry::ClockSync { wall_unix_ns, .. } => {
                let _ = writeln!(out, "[clock] wall={}", iso8601(*wall_unix_ns));
            }
            Entry::Dropped { threads, stats, .. } => {
                let _ = write!(out, "[dropped]");
                for (thread, events, faults) in threads {
                    let _ = write!(
                        out,
                        " {}:events={events},faults={faults}",
                        thread_label(file, *thread)
                    );
                }
                for (name, value) in stats {
                    let _ = write!(out, " {name}={value}");
                }
                out.push('\n');
            }
            Entry::Marker { text, .. } => {
                let _ = writeln!(out, "[marker] {}", redact.apply(text));
            }
        }
    }
    out
}

fn json_str(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn json_value(out: &mut String, value: &Value) {
    match value {
        Value::U64(v) => {
            let _ = write!(out, "{v}");
        }
        Value::I64(v) => {
            let _ = write!(out, "{v}");
        }
        Value::F64(v) if v.is_finite() => {
            let _ = write!(out, "{v}");
        }
        Value::F64(_) => out.push_str("null"),
        Value::Bool(v) => {
            let _ = write!(out, "{v}");
        }
    }
}

/// JSON Lines: a `header` object, then one object per entry.
pub fn to_json_lines(file: &NlogFile, options: &ExportOptions) -> String {
    let redact = Redactor::new(options);
    let h = &file.header;
    let mut out = String::new();
    out.push_str("{\"type\":\"header\",\"app_id\":");
    json_str(&mut out, &h.app_id);
    out.push_str(",\"app_name\":");
    json_str(&mut out, &h.app_name);
    out.push_str(",\"app_version\":");
    json_str(&mut out, &h.app_version);
    out.push_str(",\"build_id\":");
    json_str(&mut out, &h.build_id);
    out.push_str(",\"framework_version\":");
    json_str(&mut out, &h.framework_version);
    out.push_str(",\"reason\":");
    json_str(&mut out, &h.reason);
    out.push_str(",\"os\":");
    json_str(&mut out, &h.os);
    out.push_str(",\"arch\":");
    json_str(&mut out, &h.arch);
    let _ = write!(
        out,
        ",\"pid\":{},\"session_id\":\"{:016x}\",\"started\":",
        h.pid, h.session_id
    );
    json_str(&mut out, &iso8601(h.wall_start_unix_ns));
    let _ = write!(out, ",\"truncated\":{},\"extra\":{{", file.truncated);
    for (i, (key, value)) in h.extra.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        json_str(&mut out, key);
        out.push(':');
        json_str(&mut out, &redact.apply(value));
    }
    out.push_str("}}\n");

    for entry in &file.entries {
        let ts = entry.ts_ns();
        let kind = match entry {
            Entry::Event { .. } => "event",
            Entry::Fault { .. } => "fault",
            Entry::Metrics { .. } => "metrics",
            Entry::SessionInfo { .. } => "session_info",
            Entry::ClockSync { .. } => "clock_sync",
            Entry::Dropped { .. } => "dropped",
            Entry::Marker { .. } => "marker",
        };
        let _ = write!(out, "{{\"type\":\"{kind}\",\"ts_ns\":{ts},\"time\":");
        json_str(&mut out, &iso8601(h.wall_start_unix_ns.saturating_add(ts)));
        match entry {
            Entry::Event {
                key,
                thread,
                values,
                ..
            }
            | Entry::Fault {
                key,
                thread,
                values,
                ..
            } => {
                out.push_str(",\"name\":");
                json_str(&mut out, &event_label(file, *key));
                let severity = file
                    .event_schemas
                    .get(key)
                    .map_or("unknown", |s| s.severity.as_str());
                let _ = write!(
                    out,
                    ",\"domain\":{},\"id\":{},\"severity\":\"{severity}\",\"thread\":",
                    key.domain.0, key.id
                );
                json_str(&mut out, &thread_label(file, *thread));
                out.push_str(",\"fields\":{");
                for (i, (name, value)) in field_names(file, *key, values.len())
                    .iter()
                    .zip(values)
                    .enumerate()
                {
                    if i > 0 {
                        out.push(',');
                    }
                    json_str(&mut out, name);
                    out.push(':');
                    json_value(&mut out, value);
                }
                out.push('}');
                if let Entry::Fault {
                    message: Some(message),
                    ..
                } = entry
                {
                    out.push_str(",\"message\":");
                    json_str(&mut out, &redact.apply(message));
                }
            }
            Entry::Metrics { samples, .. } => {
                out.push_str(",\"samples\":[");
                for (i, sample) in samples.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str("{\"name\":");
                    json_str(&mut out, &metric_label(file, sample.key()));
                    match sample {
                        MetricSample::Counter { total, delta, .. } => {
                            let _ = write!(
                                out,
                                ",\"kind\":\"counter\",\"total\":{total},\"delta\":{delta}}}"
                            );
                        }
                        MetricSample::Gauge { value, .. } => {
                            let _ = write!(out, ",\"kind\":\"gauge\",\"value\":{value}}}");
                        }
                        MetricSample::Histogram { sample, .. } => {
                            let _ = write!(
                                out,
                                ",\"kind\":\"histogram\",\"count\":{},\"sum\":{},\"min\":{},\"max\":{},\"buckets\":[",
                                sample.count, sample.sum, sample.min, sample.max
                            );
                            for (j, (bucket, count)) in sample.buckets.iter().enumerate() {
                                if j > 0 {
                                    out.push(',');
                                }
                                let _ = write!(out, "[{},{count}]", bucket_floor(*bucket));
                            }
                            out.push_str("]}");
                        }
                    }
                }
                out.push(']');
            }
            Entry::SessionInfo { pairs, .. } => {
                out.push_str(",\"values\":{");
                for (i, (key, value)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    json_str(&mut out, key);
                    out.push(':');
                    json_str(&mut out, &redact.apply(value));
                }
                out.push('}');
            }
            Entry::ClockSync { wall_unix_ns, .. } => {
                out.push_str(",\"wall\":");
                json_str(&mut out, &iso8601(*wall_unix_ns));
            }
            Entry::Dropped { threads, stats, .. } => {
                out.push_str(",\"threads\":[");
                for (i, (thread, events, faults)) in threads.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str("{\"thread\":");
                    json_str(&mut out, &thread_label(file, *thread));
                    let _ = write!(out, ",\"events\":{events},\"faults\":{faults}}}");
                }
                out.push_str("],\"stats\":{");
                for (i, (name, value)) in stats.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    json_str(&mut out, name);
                    let _ = write!(out, ":{value}");
                }
                out.push('}');
            }
            Entry::Marker { text, .. } => {
                out.push_str(",\"text\":");
                json_str(&mut out, &redact.apply(text));
            }
        }
        out.push_str("}\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantile_reads_bucket_floor() {
        // 90 samples in [4, 8), 10 in [1024, 2048).
        let buckets = [(3u8, 90u64), (11, 10)];
        assert_eq!(histogram_quantile(&buckets, 100, 0.5), 4);
        assert_eq!(histogram_quantile(&buckets, 100, 0.99), 1024);
        assert_eq!(histogram_quantile(&[], 0, 0.5), 0);
    }

    #[test]
    fn json_escapes_control_characters() {
        let mut out = String::new();
        json_str(&mut out, "a\"b\\c\n\u{1}");
        assert_eq!(out, "\"a\\\"b\\\\c\\n\\u0001\"");
    }
}
