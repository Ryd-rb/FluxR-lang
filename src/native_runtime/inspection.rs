//! Optional local MCP over the application's inherited stdin/stdout.
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, sync_channel},
};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Frame {
    pub action: i64,
    pub arg: i64,
    pub text: String,
    pub resize: Option<[u32; 2]>,
    pub capture: Option<String>,
    pub pointer: Option<[i32; 2]>,
    pub click: bool,
    pub wheel: Option<f32>,
    pub drag: Option<[f32; 2]>,
    pub pressed: Vec<i64>,
    pub profile: Option<bool>,
}

#[derive(Default)]
pub(super) struct FrameTiming(Option<Vec<f32>>);

impl FrameTiming {
    pub fn frame(&mut self, profile: Option<bool>, seconds: f32) -> Option<Value> {
        const MAX_SAMPLES: usize = 8192;
        if profile == Some(true) {
            self.0 = Some(Vec::with_capacity(MAX_SAMPLES));
            return None;
        }
        if let Some(samples) = &mut self.0
            && samples.len() < MAX_SAMPLES
        {
            samples.push(seconds * 1000.0);
        }
        if profile != Some(false) {
            return None;
        }
        let mut samples = self.0.take()?;
        samples.sort_unstable_by(f32::total_cmp);
        let p95 = samples.get(samples.len() * 95 / 100);
        Some(json!({"samples": samples.len(), "p95_ms": p95}))
    }
}

pub(super) enum Request {
    Frame(Value, Frame),
    Close(Value),
}

pub(super) struct Inspection {
    pub root: PathBuf,
    pub requests: Receiver<Request>,
}

fn reply(id: Value, key: &str, value: Value) -> io::Result<()> {
    let mut out = io::stdout().lock();
    writeln!(out, "{}", json!({"jsonrpc":"2.0","id":id,key:value}))?;
    out.flush()
}

pub(super) fn result(id: Value, value: Value) -> io::Result<()> {
    reply(
        id,
        "result",
        json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value}),
    )
}

pub(super) fn fail(id: Value, error: impl std::fmt::Display) -> io::Result<()> {
    reply(
        id,
        "error",
        json!({"code":-32602,"message":error.to_string()}),
    )
}

impl Frame {
    fn check(self) -> Result<Self, &'static str> {
        if self.text.chars().count() > 128 {
            return Err("event text exceeds 128 characters");
        }
        if self.pressed.len() > 16 || self.pressed.iter().any(|key| !(0..=0xffff).contains(key)) {
            return Err("unsupported keyboard event");
        }
        if self.wheel.is_some_and(|n| !(-64.0..=64.0).contains(&n))
            || self
                .drag
                .is_some_and(|d| d.into_iter().any(|n| !(-4096.0..=4096.0).contains(&n)))
        {
            return Err("unsupported pointer motion");
        }
        if let Some([w, h]) = self.resize
            && (!(256..=3840).contains(&w) || !(256..=2160).contains(&h))
        {
            return Err("unsupported logical window size");
        }
        if let Some(name) = &self.capture
            && (name.is_empty()
                || name.len() > 72
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        {
            return Err("capture must be a simple file stem");
        }
        Ok(self)
    }
}

impl Inspection {
    pub fn open(path: &Path) -> io::Result<Self> {
        let root = path.canonicalize()?;
        if !root.is_dir() {
            return Err(io::Error::other("capture path must be a directory"));
        }
        let (send, requests) = sync_channel(4);
        std::thread::Builder::new().name("application-mcp".into()).spawn(move || {
            let _ = (|| -> io::Result<()> {
                let mut input = io::stdin().lock();
                loop {
                    let mut line = String::new();
                    let n = (&mut input).take(8192).read_line(&mut line)?;
                    if n == 0 || n == 8192 { break; }
                    let value: Value = match serde_json::from_str(&line) {
                        Ok(value) => value,
                        Err(_) => {
                            reply(Value::Null, "error", json!({"code":-32700,"message":"invalid JSON"}))?;
                            continue;
                        }
                    };
                    let Some(id) = value.get("id").cloned() else { continue; };
                    match value["method"].as_str() {
                        Some("initialize") => reply(id, "result", json!({
                            "protocolVersion":"2024-11-05", "capabilities":{"tools":{}},
                            "serverInfo":{"name":"fluxr-application","version":env!("CARGO_PKG_VERSION")}
                        }))?,
                        Some("ping") => reply(id, "result", json!({}))?,
                        Some("tools/list") => reply(id, "result", json!({"tools":[
                            {"name":"app_frame","description":"Inspect the next rendered frame; optionally dispatch an application event, resize or capture.",
                             "inputSchema":{"type":"object","additionalProperties":false,"properties":{
                                 "action":{"type":"integer"},"arg":{"type":"integer"},"resize":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"integer"}},
                                 "text":{"type":"string","maxLength":128},"capture":{"type":"string"},
                                 "pointer":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"integer"}},
                                 "click":{"type":"boolean"},
                                 "wheel":{"type":"number"},
                                 "drag":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"number"}},
                                 "pressed":{"type":"array","maxItems":16,"items":{"type":"integer"}},
                                 "profile":{"type":"boolean","description":"True starts frame timing. False stops and reports it."}}}},
                            {"name":"app_close","description":"Close this application window.","inputSchema":{"type":"object","properties":{}}}
                        ]}))?,
                        Some("tools/call") => match value["params"]["name"].as_str() {
                            Some("app_frame") => {
                                let args = value["params"].get("arguments").cloned().unwrap_or(json!({}));
                                match serde_json::from_value::<Frame>(args)
                                    .map_err(|error| error.to_string())
                                    .and_then(|frame| frame.check().map_err(str::to_owned))
                                {
                                    Ok(frame) => if send.send(Request::Frame(id, frame)).is_err() { break; },
                                    Err(error) => fail(id, error)?,
                                }
                            }
                            Some("app_close") => {
                                let _ = send.send(Request::Close(id));
                                break;
                            }
                            _ => fail(id, "unknown tool")?,
                        },
                        _ => reply(id, "error", json!({"code":-32601,"message":"unknown method"}))?,
                    }
                }
                Ok(())
            })();
        })?;
        Ok(Self { root, requests })
    }
}

#[cfg(test)]
mod tests {
    use super::FrameTiming;

    #[test]
    fn frame_profile_is_opt_in_and_reports_the_upper_percentile() {
        let mut timing = FrameTiming::default();
        assert!(timing.frame(Some(false), 1.0).is_none());
        assert!(timing.frame(Some(true), 1.0).is_none());
        for _ in 0..19 {
            assert!(timing.frame(None, 0.01).is_none());
        }
        let result = timing.frame(Some(false), 0.05).unwrap();
        assert_eq!(result["samples"], 20);
        assert_eq!(result["p95_ms"], 50.0);
        assert!(timing.frame(Some(false), 1.0).is_none());
    }
}
