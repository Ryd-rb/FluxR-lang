# Optional native Windows host

`crates/fluxr-win32` is a separate, optional Rust host library. Applications
explicitly depend on its path; the compiler, VM and default FluxR build have no
GUI dependency. The host uses Win32 system controls and a native message loop,
without JavaScript, a browser, or a product-specific workflow. Trusted Rust can
connect the JSON channel to checked FluxR host services.

```rust,ignore
fluxr_win32::run(title, initial_frame, outgoing_sender, incoming_receiver)?;
```

The function runs on the calling UI thread and returns when the user closes the
window or a frame contains `"close": true`. Call once per dedicated UI thread.
Non-Windows targets return an unsupported-platform error. The host changes only
the calling thread's DPI context and restores it on exit.

Frames are complete desired trees:

```json
{"title":"Example","ack":0,"default":"save","controls":[
  {"id":"nav","kind":"list","label":"Files","side":true,"lines":12,
   "choices":[{"label":"  notes.txt","value":"notes"}],"value":"notes"},
  {"id":"body","kind":"multiline","label":"Notes","lines":12,"value":"Hello"},
  {"id":"save","kind":"button","label":"Save","row":"actions"},
  {"id":"cancel","kind":"button","label":"Cancel","row":"actions"}
]}
```

Each control needs a nonempty unique string `id` and a `kind`: `label`,
`heading`, `button`, `input`, `multiline`, `check`, `select`, `detail`, `list`,
or `multi`. Unknown fields are ignored. `enabled` defaults to true. All values
are strings, except `check` uses Boolean and `multi` uses an array of strings.
`choices` entries contain string `label` and `value`. `list` is a native single
selection listbox; `multi` supports ordinary Windows Ctrl/Shift selection.
`detail` is selectable, read-only multiline text. `lines` controls the height
of multiline/list controls (default five/six, range 1..1000). `zoom` scales the
control's font/line height (default 1, range 0.5..3).

Input, multiline, detail and selection controls draw their nonempty `label`
above the native field as an adjacent static label. Do not emit duplicate label
controls. Native standard controls expose their standard accessibility and IME
behavior. No screen-reader or visual qualification is claimed.
Labels are literal: static captions disable prefix processing and button/check
labels escape Win32 accelerator ampersands. Empty choice arrays are accepted.

Adjacent controls sharing a non-null `row` occupy equal horizontal cells.
Other controls use the available width. At widths above 600 logical pixels,
`side:true` places controls in a left column; smaller windows stack everything.
The outer viewport has a native vertical scrollbar and mouse-wheel scrolling;
listboxes support horizontal scrolling and fields retain native internal
scrolling. Native resize and per-monitor DPI changes recalculate geometry.

Stable IDs and kinds retain HWNDs. Changed text/selection/enabled state is
applied incrementally, and unchanged geometry does not move native controls.
Changing kind or adding/removing a field label recreates that control. Existing
focus, text selection, edit/list scroll and viewport offset are retained where
the updated content permits. `focus` requests a particular control's focus and
reveals it. Tab/Shift+Tab follow frame order. Enter activates a focused button;
Ctrl+Enter invokes the enabled button named by frame `default`. Multiline Enter
remains ordinary native text editing.

`selection:{"id":"body","start":4,"end":4}` requests a one-shot caret or
selection on an input, multiline or detail control, applied after focus. Offsets
are native UTF16 units, including CRLF. Ranges must be nonnegative u32 values
with start <= end; malformed ranges reject the frame. Valid offsets clamp to
current text length and snap backward if they bisect a surrogate pair. Unknown
or non-text targets are ignored. Explicit focus and selection requests are
ignored when newer local click/change events exist beyond the request's `ack`.
This prevents delayed completion responses from moving a newer edit's caret.

Outgoing events:

```json
{"type":"click","id":"save","serial":1}
{"type":"change","id":"body","value":"x😀y","start":3,"end":3,"serial":2}
{"type":"change","id":"nav","value":"notes","serial":3}
{"type":"change","id":"tags","value":["a","b"],"serial":4}
{"type":"drop","paths":["C:\\notes.txt"]}
{"type":"error","message":"invalid value for control body"}
```

`serial` increases across click/change events for the window. Frames echo the
last handled serial as `ack` (default zero). A field with a newer local change
retains its local value and choices until acknowledged, preventing stale frames
from erasing typing or selection. Text and caret offsets use the raw native
Windows edit value: CRLF line endings and UTF16 code units. Normalize text and
offsets together in the application if its model uses LF. Drops contain only
paths supplied by Windows `WM_DROPFILES`; they do not read or import files.

Incoming delivery holds at most one pending complete frame and one posted
wakeup. A faster producer replaces the pending tree. Unconsumed one-shot focus
and selection requests survive replacement (a newer explicit request wins),
and retain their original `ack` so newer frames cannot authorize stale caret
movement. A close request also survives replacement. Invalid later frames emit
an error without discarding an already pending valid frame. Requests are consumed
once when the native loop takes that frame. The UI blocks in GetMessage
when idle. A receiver thread waits on the Rust channel and stops on close;
outgoing uses the caller-provided unbounded standard Sender, so applications
must drain it. Invalid initial frames return errors; invalid subsequent frames
emit an error event. OS allocation failures can leave a partially reconciled
tree and should be treated as a host failure by the application.

Validation is limited to typed frame checks and an explicitly hidden,
in-process native lifecycle test. It checks content, UTF16 offsets, stale-edit
protection, HWND reuse, one-shot UTF16 selection/coalescing, layout, scrolling and destruction. It neither
drives desktop UI nor records screenshots or image metadata. Reproduce with an
external Cargo target directory:

```sh
cargo test --locked --release --manifest-path crates/fluxr-win32/Cargo.toml -- --nocapture
```

The lifecycle test prints create/update/layout durations for its small native
control workload. These are diagnostic samples, not a cross-platform latency
guarantee or a completed FluxR release qualification. A custom graph canvas,
hierarchical tree widget and visual styling beyond native controls are outside
this slice.
