//! CPU sampling profiler for the JS engine.
//!
//! Sampling is performed inside JS via `setInterval` + `new Error().stack`, so it is accurate
//! for async and I/O-bound programs. Tight CPU loops with no Tokio yield points will show
//! reduced sample fidelity — this is a known QuickJS constraint (no C-level stack walk API).
//!
//! Output formats:
//! - `.cpuprofile` — V8-compatible JSON, loadable in Chrome DevTools / speedscope.app
//! - Folded stacks — inferno input format used to generate SVG flamegraphs

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A single stack frame captured during sampling.
#[derive(Debug, Clone)]
pub struct ProfileFrame {
    pub function_name: String,
    pub url: String,
    pub line_number: i32,
    pub column_number: i32,
}

/// One profiler sample: a timestamp (ms since profiling started) and a call stack.
#[derive(Debug, Clone)]
pub struct ProfileSample {
    pub timestamp_ms: u64,
    pub frames: Vec<ProfileFrame>,
    /// Optional label set by `console.profile` / `console.profileEnd`
    pub label: Option<String>,
}

/// Shared profiler state — written by the JS side, read by the Rust side on exit.
#[derive(Debug, Default)]
pub struct ProfilerState {
    pub samples: Vec<ProfileSample>,
    pub start_time_ms: u64,
    pub end_time_ms: u64,
}

/// Thread-safe handle to the profiler state.
#[derive(Clone, Debug)]
pub struct Profiler(pub Arc<Mutex<ProfilerState>>);

impl Default for Profiler {
    fn default() -> Self {
        Self::new()
    }
}

impl Profiler {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(ProfilerState::default())))
    }

    /// Push a raw sample parsed from a JS `Error().stack` string.
    pub fn push_raw(&self, timestamp_ms: u64, stack_str: &str, label: Option<String>) {
        let frames = parse_quickjs_stack(stack_str);
        let mut state = self.0.lock().unwrap();
        if state.start_time_ms == 0 {
            state.start_time_ms = timestamp_ms;
        }
        state.end_time_ms = timestamp_ms;
        state.samples.push(ProfileSample {
            timestamp_ms,
            frames,
            label,
        });
    }

    /// Serialize to V8-compatible `.cpuprofile` JSON.
    pub fn to_cpuprofile(&self) -> String {
        let state = self.0.lock().unwrap();
        build_cpuprofile(&state)
    }

    /// Serialize to inferno folded-stacks format (one line per sample).
    pub fn to_folded_stacks(&self) -> String {
        let state = self.0.lock().unwrap();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for sample in &state.samples {
            if sample.frames.is_empty() {
                continue;
            }
            // Build the folded stack: bottom frame first, top frame last.
            let frames: Vec<String> = sample
                .frames
                .iter()
                .rev()
                .map(|f| {
                    let name = if f.function_name.is_empty() {
                        "(anonymous)".to_string()
                    } else {
                        f.function_name.clone()
                    };
                    if f.url.is_empty() || f.line_number < 0 {
                        name
                    } else {
                        format!("{} ({}:{})", name, f.url, f.line_number)
                    }
                })
                .collect();
            let folded = frames.join(";");
            *counts.entry(folded).or_default() += 1;
        }
        let mut lines: Vec<String> = counts
            .into_iter()
            .map(|(stack, count)| format!("{stack} {count}"))
            .collect();
        lines.sort();
        lines.join("\n")
    }

    /// Generate a flamegraph SVG using the inferno crate.
    pub fn to_flamegraph_svg(&self) -> anyhow::Result<String> {
        let folded = self.to_folded_stacks();
        if folded.is_empty() {
            return Ok(String::new());
        }
        let lines: Vec<&str> = folded.lines().collect();
        let mut svg = Vec::new();
        let mut opts = inferno::flamegraph::Options::default();
        opts.title = "3va CPU Profile".to_string();
        inferno::flamegraph::from_lines(&mut opts, lines, &mut svg)?;
        Ok(String::from_utf8(svg)?)
    }

    /// Print a human-readable top-N summary of hot functions.
    pub fn top_functions(&self, n: usize) -> Vec<(String, usize)> {
        let state = self.0.lock().unwrap();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for sample in &state.samples {
            // Only count the leaf (top) frame.
            if let Some(frame) = sample.frames.first() {
                let name = if frame.function_name.is_empty() {
                    "(anonymous)".to_string()
                } else {
                    frame.function_name.clone()
                };
                *counts.entry(name).or_default() += 1;
            }
        }
        let total = counts.values().sum::<usize>().max(1);
        let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
        ranked.sort_by_key(|b| std::cmp::Reverse(b.1));
        ranked.truncate(n);
        // Convert raw counts to percentages (stored as per-mille for integer display).
        ranked
            .into_iter()
            .map(|(name, count)| (name, count * 100 / total))
            .collect()
    }

    pub fn sample_count(&self) -> usize {
        self.0.lock().unwrap().samples.len()
    }
}

// ── QuickJS stack string parser ───────────────────────────────────────────────
//
// QuickJS `Error().stack` looks like:
//   at functionName (file.js:10:5)
//   at (eval):3:1
//   at <anonymous> (file.js:1:1)
//
fn parse_quickjs_stack(stack: &str) -> Vec<ProfileFrame> {
    let mut frames = Vec::new();
    for line in stack.lines() {
        let line = line.trim();
        if !line.starts_with("at ") {
            continue;
        }
        let rest = &line[3..];

        // Try to parse "name (url:line:col)" or "name (url:line)"
        if let Some((name_part, location)) = rest.rsplit_once('(') {
            let function_name = name_part.trim().trim_end_matches('<').trim().to_string();
            let location = location.trim_end_matches(')');
            let (url, line_number, column_number) = parse_location(location);
            frames.push(ProfileFrame {
                function_name: if function_name == "<anonymous>" {
                    String::new()
                } else {
                    function_name
                },
                url,
                line_number,
                column_number,
            });
        } else {
            // No parentheses — try "url:line:col" directly
            let (url, line_number, column_number) = parse_location(rest);
            frames.push(ProfileFrame {
                function_name: String::new(),
                url,
                line_number,
                column_number,
            });
        }
    }
    frames
}

fn parse_location(loc: &str) -> (String, i32, i32) {
    let parts: Vec<&str> = loc.rsplitn(3, ':').collect();
    match parts.as_slice() {
        [col, line, url] => (
            url.to_string(),
            line.parse().unwrap_or(-1),
            col.parse().unwrap_or(-1),
        ),
        [line, url] => (url.to_string(), line.parse().unwrap_or(-1), -1),
        _ => (loc.to_string(), -1, -1),
    }
}

// ── .cpuprofile serializer ────────────────────────────────────────────────────
//
// Minimal V8 CPU profile format that Chrome DevTools and speedscope.app accept.
//
fn build_cpuprofile(state: &ProfilerState) -> String {
    // Build a trie of call frames so that equal stacks share nodes.
    // Node 1 is always the synthetic "(root)" top-level node.
    struct Node {
        id: usize,
        frame: CpuFrame,
        children: HashMap<String, usize>, // key → child node id
        hit_count: usize,
    }
    #[derive(Clone)]
    struct CpuFrame {
        function_name: String,
        url: String,
        script_id: String,
        line_number: i32,
        column_number: i32,
    }

    let mut nodes: Vec<Node> = Vec::new();
    // Node 0 placeholder (ids are 1-based).
    nodes.push(Node {
        id: 0,
        frame: CpuFrame {
            function_name: String::new(),
            url: String::new(),
            script_id: "0".to_string(),
            line_number: -1,
            column_number: -1,
        },
        children: HashMap::new(),
        hit_count: 0,
    });
    // Node 1: root.
    nodes.push(Node {
        id: 1,
        frame: CpuFrame {
            function_name: "(root)".to_string(),
            url: String::new(),
            script_id: "0".to_string(),
            line_number: -1,
            column_number: -1,
        },
        children: HashMap::new(),
        hit_count: 0,
    });

    let mut sample_ids: Vec<usize> = Vec::new();
    let mut time_deltas: Vec<u64> = Vec::new();
    let mut prev_ts = state.start_time_ms;

    for sample in &state.samples {
        let delta = sample.timestamp_ms.saturating_sub(prev_ts);
        prev_ts = sample.timestamp_ms;

        // Walk from root downward (frames are top-of-stack first, so reverse).
        let mut current = 1usize;
        for frame in sample.frames.iter().rev() {
            let key = format!(
                "{}|{}|{}|{}",
                frame.function_name, frame.url, frame.line_number, frame.column_number
            );
            let next_id = if let Some(&id) = nodes[current].children.get(&key) {
                id
            } else {
                let new_id = nodes.len();
                let script_id = if frame.url.is_empty() {
                    "0".to_string()
                } else {
                    new_id.to_string()
                };
                nodes.push(Node {
                    id: new_id,
                    frame: CpuFrame {
                        function_name: if frame.function_name.is_empty() {
                            "(anonymous)".to_string()
                        } else {
                            frame.function_name.clone()
                        },
                        url: frame.url.clone(),
                        script_id,
                        line_number: frame.line_number,
                        column_number: frame.column_number,
                    },
                    children: HashMap::new(),
                    hit_count: 0,
                });
                nodes[current].children.insert(key, new_id);
                new_id
            };
            current = next_id;
        }
        nodes[current].hit_count += 1;
        sample_ids.push(current);
        time_deltas.push(delta * 1_000); // V8 uses microseconds
    }

    // Serialize nodes.
    let mut out = String::with_capacity(4096);
    out.push_str("{\"nodes\":[");
    for (i, node) in nodes.iter().enumerate().skip(1) {
        if i > 1 {
            out.push(',');
        }
        let children: Vec<String> = node.children.values().map(|id| id.to_string()).collect();
        let fn_name = escape_json(&node.frame.function_name);
        let url = escape_json(&node.frame.url);
        out.push_str(&format!(
            "{{\"id\":{id},\"callFrame\":{{\"functionName\":\"{fn_name}\",\
             \"scriptId\":\"{sid}\",\"url\":\"{url}\",\
             \"lineNumber\":{line},\"columnNumber\":{col}}},\
             \"hitCount\":{hits},\"children\":[{ch}]}}",
            id = node.id,
            fn_name = fn_name,
            sid = node.frame.script_id,
            url = url,
            line = node.frame.line_number,
            col = node.frame.column_number,
            hits = node.hit_count,
            ch = children.join(","),
        ));
    }
    out.push_str("],\"startTime\":");
    out.push_str(&(state.start_time_ms * 1_000).to_string());
    out.push_str(",\"endTime\":");
    out.push_str(&(state.end_time_ms * 1_000).to_string());
    out.push_str(",\"samples\":[");
    let samples_str: Vec<String> = sample_ids.iter().map(|id| id.to_string()).collect();
    out.push_str(&samples_str.join(","));
    out.push_str("],\"timeDeltas\":[");
    let deltas_str: Vec<String> = time_deltas.iter().map(|d| d.to_string()).collect();
    out.push_str(&deltas_str.join(","));
    out.push_str("]}");
    out
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Parse a `.cpuprofile` JSON file and return top-N hot functions as
/// `(function_name, hit_count)` sorted descending.
pub fn analyze_cpuprofile(json: &str, top_n: usize) -> anyhow::Result<Vec<(String, usize)>> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let nodes = v["nodes"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing 'nodes'"))?;

    let mut hit_by_id: HashMap<u64, (String, usize)> = HashMap::new();
    for node in nodes {
        let id = node["id"].as_u64().unwrap_or(0);
        let name = node["callFrame"]["functionName"]
            .as_str()
            .unwrap_or("(anonymous)")
            .to_string();
        let hits = node["hitCount"].as_u64().unwrap_or(0) as usize;
        hit_by_id.insert(id, (name, hits));
    }

    let mut ranked: Vec<(String, usize)> =
        hit_by_id.into_values().filter(|(_, h)| *h > 0).collect();
    ranked.sort_by_key(|b| std::cmp::Reverse(b.1));
    ranked.truncate(top_n);
    Ok(ranked)
}

/// JS source injected when `--prof` is active.
///
/// Samples are pushed directly to Rust via `__profilerPush(ts_ms, stack_str, label)`,
/// which is registered as a native function before this script runs.
/// `__profilerStop()` is called by the Rust side after `eval_file` completes to
/// flush any in-flight interval and stop further sampling.
pub fn profiler_js(interval_ms: u32) -> String {
    format!(
        r#"(function() {{
    var __profilerActiveLabel = null;
    var __profilerIntervalId = null;
    var __profilerStartTs = Date.now();

    function __profileCapture() {{
        try {{
            throw new Error('__3va_prof__');
        }} catch (e) {{
            var stack = (e && e.stack) ? e.stack : '';
            var filtered = stack.split('\n').filter(function(l) {{
                return l.indexOf('__3va_prof') === -1 && l.indexOf('__profileCapture') === -1;
            }}).join('\n');
            var ts = Date.now() - __profilerStartTs;
            if (typeof __profilerPush === 'function') {{
                __profilerPush(ts, filtered, __profilerActiveLabel);
            }}
        }}
    }}

    globalThis.__profilerStop = function() {{
        if (__profilerIntervalId !== null) {{
            clearInterval(__profilerIntervalId);
            __profilerIntervalId = null;
        }}
    }};

    // Attach console.profile / console.profileEnd
    var _orig = globalThis.console || {{}};
    globalThis.console = Object.assign({{}}, _orig, {{
        profile: function(label) {{
            __profilerActiveLabel = label || '(profile)';
            if (_orig.log) _orig.log('[profile] start: ' + __profilerActiveLabel);
        }},
        profileEnd: function(label) {{
            if (_orig.log) _orig.log('[profile] end: ' + (label || __profilerActiveLabel));
            __profilerActiveLabel = null;
        }}
    }});

    __profilerIntervalId = setInterval(__profileCapture, {interval});
    __profileCapture();
}})();"#,
        interval = interval_ms
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quickjs_stack_basic() {
        let stack = "Error: __3va_prof__\n    at foo (app.js:10:5)\n    at bar (app.js:5:1)\n    at <anonymous> (app.js:1:1)";
        let frames = parse_quickjs_stack(stack);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].function_name, "foo");
        assert_eq!(frames[0].url, "app.js");
        assert_eq!(frames[0].line_number, 10);
        assert_eq!(frames[0].column_number, 5);
        assert_eq!(frames[1].function_name, "bar");
        assert!(frames[2].function_name.is_empty()); // <anonymous> → ""
    }

    #[test]
    fn parse_location_full() {
        let (url, line, col) = parse_location("app.js:10:5");
        assert_eq!(url, "app.js");
        assert_eq!(line, 10);
        assert_eq!(col, 5);
    }

    #[test]
    fn parse_location_no_col() {
        let (url, line, col) = parse_location("app.js:3");
        assert_eq!(url, "app.js");
        assert_eq!(line, 3);
        assert_eq!(col, -1);
    }

    #[test]
    fn to_folded_stacks_basic() {
        let profiler = Profiler::new();
        {
            let mut s = profiler.0.lock().unwrap();
            s.samples.push(ProfileSample {
                timestamp_ms: 10,
                frames: vec![
                    ProfileFrame {
                        function_name: "inner".into(),
                        url: "a.js".into(),
                        line_number: 5,
                        column_number: 1,
                    },
                    ProfileFrame {
                        function_name: "outer".into(),
                        url: "a.js".into(),
                        line_number: 1,
                        column_number: 1,
                    },
                ],
                label: None,
            });
            s.samples.push(ProfileSample {
                timestamp_ms: 20,
                frames: vec![
                    ProfileFrame {
                        function_name: "inner".into(),
                        url: "a.js".into(),
                        line_number: 5,
                        column_number: 1,
                    },
                    ProfileFrame {
                        function_name: "outer".into(),
                        url: "a.js".into(),
                        line_number: 1,
                        column_number: 1,
                    },
                ],
                label: None,
            });
        }
        let folded = profiler.to_folded_stacks();
        assert!(folded.contains("outer") && folded.contains("inner"));
        assert!(folded.ends_with(" 2") || folded.contains("2"));
    }

    #[test]
    fn cpuprofile_is_valid_json() {
        let profiler = Profiler::new();
        {
            let mut s = profiler.0.lock().unwrap();
            s.start_time_ms = 0;
            s.end_time_ms = 100;
            s.samples.push(ProfileSample {
                timestamp_ms: 10,
                frames: vec![ProfileFrame {
                    function_name: "myFn".into(),
                    url: "app.js".into(),
                    line_number: 3,
                    column_number: 1,
                }],
                label: None,
            });
        }
        let json = profiler.to_cpuprofile();
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(parsed["nodes"].is_array());
        assert!(parsed["samples"].is_array());
    }

    #[test]
    fn analyze_cpuprofile_basic() {
        let profiler = Profiler::new();
        {
            let mut s = profiler.0.lock().unwrap();
            s.start_time_ms = 0;
            s.end_time_ms = 100;
            for _ in 0..5 {
                s.samples.push(ProfileSample {
                    timestamp_ms: 10,
                    frames: vec![ProfileFrame {
                        function_name: "hotFn".into(),
                        url: "a.js".into(),
                        line_number: 1,
                        column_number: 1,
                    }],
                    label: None,
                });
            }
        }
        let json = profiler.to_cpuprofile();
        let top = analyze_cpuprofile(&json, 3).unwrap();
        assert!(!top.is_empty());
        assert_eq!(top[0].0, "hotFn");
    }

    #[test]
    fn profiler_js_contains_interval() {
        let js = profiler_js(15);
        assert!(js.contains("setInterval(__profileCapture, 15)"));
    }
}
