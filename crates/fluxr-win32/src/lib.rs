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
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
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
}
fn yes() -> bool {
    true
}
fn one() -> f64 {
    1.0
}
#[derive(Debug, Deserialize)]
struct Frame {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    controls: Vec<Control>,
    #[serde(default)]
    focus: Option<String>,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    ack: u64,
    #[serde(default)]
    close: bool,
}
fn parse(value: Value) -> Result<Frame> {
    let frame: Frame = serde_json::from_value(value)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn validates_typed_keyed_contract() {
        assert!(parse(json!({"controls":[{"id":"x","kind":"multi","value":["a"]}]})).is_ok());
        assert!(
            parse(json!({"controls":[{"id":"x","kind":"input"},{"id":"x","kind":"button"}]}))
                .is_err()
        );
        assert!(parse(json!({"controls":[{"id":"x","kind":"check","value":"yes"}]})).is_err());
        assert!(parse(json!({"controls":[{"id":"x","kind":"list","zoom":4}]})).is_err());
    }
}
