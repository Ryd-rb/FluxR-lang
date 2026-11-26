//! Optional native Windows host. See `docs/native-windows-ui.md` for the JSON ABI.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::mpsc::{Receiver, Sender};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct Choice {
    label: String,
    value: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Label,
    Heading,
    Button,
    Input,
    Multiline,
    Check,
    Select,
    Detail,
    Multi,
    List,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Role {
    #[default]
    Neutral,
    Brand,
    Caption,
    Title,
    Muted,
    Primary,
    Nav,
    Selected,
    Quiet,
    Body,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Align {
    #[default]
    Start,
    Center,
    End,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Theme {
    #[default]
    System,
    Dark,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Layout {
    Welcome,
    Workspace,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct Control {
    id: String,
    kind: Kind,
    #[serde(default)]
    label: String,
    #[serde(default)]
    value: Value,
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    row: Option<String>,
    #[serde(default)]
    lines: Option<u32>,
    #[serde(default)]
    side: bool,
    #[serde(default = "one")]
    zoom: f64,
    #[serde(default)]
    role: Role,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    row_align: Align,
    #[serde(default)]
    expanded: Option<bool>,
}
fn yes() -> bool {
    true
}
fn one() -> f64 {
    1.0
}
#[derive(Debug, Deserialize)]
struct Selection {
    id: String,
    start: u32,
    end: u32,
}
#[derive(Debug, Deserialize)]
struct Frame {
    #[serde(default)]
    theme: Theme,
    #[serde(default)]
    layout: Option<Layout>,
    #[serde(default)]
    max_width: Option<u32>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    controls: Vec<Control>,
    #[serde(default)]
    focus: Option<String>,
    #[serde(default)]
    selection: Option<Selection>,
    #[serde(skip)]
    focus_ack: Option<u64>,
    #[serde(skip)]
    selection_ack: Option<u64>,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    ack: u64,
    #[serde(default)]
    close: bool,
}
fn parse(value: Value) -> Result<Frame> {
    let frame: Frame = serde_json::from_value(value)?;
    if frame.selection.as_ref().is_some_and(|s| s.start > s.end) {
        bail!("selection start must not exceed end")
    }
    if frame.max_width.is_some_and(|w| !(320..=1600).contains(&w)) {
        bail!("max_width must be between 320 and 1600")
    }
    let mut ids = std::collections::HashSet::new();
    for c in &frame.controls {
        if c.id.is_empty() || !ids.insert(&c.id) {
            bail!("control IDs must be nonempty and unique")
        }
        if !c.zoom.is_finite() || !(0.5..=3.0).contains(&c.zoom) {
            bail!("zoom must be between 0.5 and 3")
        }
        if c.lines.is_some_and(|n| !(1..=1000).contains(&n)) {
            bail!("lines must be between 1 and 1000")
        }
        if c.width.is_some_and(|w| !(32..=1600).contains(&w)) {
            bail!("control width must be between 32 and 1600")
        }
        let valid = match c.kind {
            Kind::Check => c.value.is_null() || c.value.is_boolean(),
            Kind::Multi => {
                c.value.is_null()
                    || c.value
                        .as_array()
                        .is_some_and(|a| a.iter().all(Value::is_string))
            }
            _ => c.value.is_null() || c.value.is_string(),
        };
        if !valid {
            bail!("invalid value for control {}", c.id)
        }
    }
    if frame.controls.len() > 4096 {
        bail!("too many controls")
    }
    Ok(frame)
}

/// Run a native Windows message loop on the calling thread until the window closes.
/// Incoming frames replace the desired control tree. An overloaded producer is
/// coalesced to its latest frame. Native controls and local edits are retained by ID.
pub fn run(
    title: &str,
    initial: Value,
    outgoing: Sender<Value>,
    incoming: Receiver<Value>,
) -> Result<()> {
    let frame = parse(initial)?;
    #[cfg(windows)]
    {
        windows::run(title, frame, outgoing, incoming)
    }
    #[cfg(not(windows))]
    {
        let _ = (title, frame, outgoing, incoming);
        bail!("fluxr-win32 requires Windows")
    }
}

#[cfg(windows)]
mod windows;

/// Validate the presentation ABI without creating a native window (on any OS).
pub fn validate_frame(frame: &Value) -> Result<()> {
    parse(frame.clone()).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn validates_typed_keyed_contract() {
        assert!(parse(json!({"theme":"dark","layout":"workspace","controls":[{"id":"a","kind":"detail","role":"body"}]})).is_ok());
        assert!(parse(json!({"theme":"dark","layout":"welcome","max_width":560,"controls":[{"id":"a","kind":"button","role":"primary","width":160,"row_align":"end","expanded":false}]})).is_ok());
        assert!(parse(json!({"theme":"unknown"})).is_err());
        assert!(parse(json!({"controls":[{"id":"a","kind":"button","width":0}]})).is_err());
        assert!(parse(json!({"controls":[{"id":"x","kind":"multi","value":["a"]}]})).is_ok());
        assert!(
            parse(json!({"controls":[{"id":"x","kind":"input"},{"id":"x","kind":"button"}]}))
                .is_err()
        );
        assert!(parse(json!({"controls":[{"id":"x","kind":"check","value":"yes"}]})).is_err());
        assert!(parse(json!({"controls":[{"id":"x","kind":"list","zoom":4}]})).is_err());
        assert!(parse(json!({"selection":{"id":"x","start":3,"end":2}})).is_err());
        assert!(parse(json!({"selection":{"id":"x","start":0,"end":4294967296u64}})).is_err());
    }
}
