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
`validate_frame(&serde_json::Value)` checks the same ABI without creating a
window, including on non-Windows hosts.

## Optional dark presentation

`theme:"dark"` opts into graphite client and rail surfaces, quiet panel borders,
warm white text, slate metadata and teal action/active-navigation treatments.
Omitting theme or using `theme:"system"` retains native system painting and the
existing side-column behavior. High-contrast Windows settings take precedence
over dark painting and title-bar treatment, including changes while running.
The renderer does not change process-wide or user theme settings.

`layout:"workspace"` explicitly selects the default workspace arrangement.
Dark workspace content is bounded to 960 logical pixels by default, with a
224-pixel rail when `side:true` controls exist and the client is at least 760
logical pixels wide. A sidebar that fits the viewport remains fixed while the
main content scrolls, and is excluded from transcript scroll extent; a taller sidebar scrolls with content so every control
remains reachable. Narrow dark workspaces stack content and arrange contiguous
side navigation buttons into compact wrapping rows. Legacy system frames keep
their original side-column threshold and system painting.
Dark rail buttons use 36-pixel hit targets with compact section spacing.
A `selected` button with `icon:"folder"` represents a selected container using
quiet accent text; other selected navigation keeps its filled active-view
treatment and leading indicator. Labels and native activation remain intact.
Dark workspace padding is 24 logical pixels, with a 14-pixel Segoe UI base,
30-pixel display titles, 19-pixel headings and 11-pixel captions. Quiet, primary
and segmented actions use distinct compact dimensions and native disabled,
pressed, hover, checked and keyboard-focus state. The shared slate/graphite
surfaces use restrained teal emphasis; high contrast keeps native system colors.

`role:"tab"` is an opt-in segmented page/action switcher on `kind:"button"`.
Give consecutive segments the same `row` and set `selected:true` on the active
segment. Each remains a real native radio-style button with its own accessible
name, checked state, keyboard group and ordinary `click` ID. Native left/right
arrow-group and Tab handling is retained through intermediate viewports; the
controller sets selection in its next frame. The host does not invent page
semantics. Segments wrap at narrow widths and keep stable HWNDs. Their optional
explicit `width` uses the existing width contract.

`section:"stable-group-key"` groups consecutive main-content controls with that
same nonempty key into one quietly outlined surface with 16-pixel inner padding.
It supports dark workspace, Welcome and conversation transcript layouts, can
span multiple rows, and ends at a different/missing key. Header, composer and
rail controls keep their existing dedicated surfaces. Section keys are opaque,
not visible titles: supply a heading/caption control for an accessible title.
These are presentation groups; the native children retain their names, values,
selection and focus. System mode ignores decorative section geometry.

`layout:"welcome"` creates a centered form bounded to 560 logical pixels.
At regular dark widths, a subtle panel encloses it with 32-pixel internal room;
below 720 logical pixels the panel boundary disappears and the content adapts
to available width. The form centers vertically when it fits, and starts at a
comfortable top inset with ordinary viewport scrolling when it does not.
`max_width` overrides either content bound (320..1600 logical pixels). Initial
window dimensions scale with DPI and are bounded to the desktop work area.

Controls accept these optional semantic `role` values:

| Role | Dark presentation |
| --- | --- |
| `brand` | Wordmark typography and a generic first-letter accent tile |
| `caption` | Small metadata or section caption |
| `title` | Prominent page title |
| `muted` | Secondary text |
| `primary` | Strong teal action |
| `nav` | Quiet, left-aligned navigation |
| `selected` | Active navigation surface and accent indicator |
| `quiet` | Low-emphasis action |
| `body` | Readable selectable prose; a detail control becomes borderless |
| `incoming` | Selectable, automatically wrapped and sized transcript prose |
| `outgoing` | Right-aligned, selectable transcript bubble |

Omitted role is neutral. Typography is implemented with native Segoe UI fonts
and scales with monitor DPI. Standard native button, checkbox, edit, list and
combobox classes retain their input/selection, keyboard, IME and accessibility
implementation. Dark painting adds hover, pressed, disabled and focus cues;
`expanded:true|false` optionally draws a disclosure chevron on a button without
changing its accessible label. It does not add an accessibility expand/collapse
pattern to a standard button. Edit selections and scrollbars continue to use
the system's supported native theme behavior; scrollbar appearance varies by
Windows version. The supported dark DWM title-bar request is best effort on
older Windows. No undocumented app/process-wide dark-mode hooks are used.

For compact actions, set `width` on each control (32..1600 logical pixels).
Consecutive controls with the same `row` are arranged together; `row_align` on
the first control is `start` (default), `center`, or `end`. Explicit widths are
bounded by the available viewport. Dark ordinary/quiet buttons otherwise use
bounded intrinsic widths; standalone primary and navigation buttons can fill
the available column. Other unspecified widths share remaining row space.
Rows wrap at narrow widths so actions cannot escape the client horizontally.

Buttons may specify a decorative `icon`: `chat`, `activity`, `graph`, `models`,
`dictionary`, `tools`, `folder`, `add`, `attachment`, or `send`. Glyphs use native
DPI-scaled line drawing and preserve the complete native label and accessible
name. System/high-contrast painting retains the label without custom icons.

```json
{"theme":"dark","layout":"welcome","controls":[
  {"id":"brand","kind":"label","label":"Atlas","role":"brand"},
  {"id":"title","kind":"heading","label":"Open your workspace","role":"title"},
  {"id":"folder","kind":"input","label":"Folder","value":""},
  {"id":"open","kind":"button","label":"Open","role":"primary","width":160,
   "row":"actions","row_align":"center"},
  {"id":"help","kind":"button","label":"Learn more","role":"quiet","width":140,"row":"actions"}
]}
```

## Conversation viewport and composer

`layout:"conversation"` uses a bounded transcript above a bottom composer.
Mark composer fields, attachment actions and send buttons with `area:"composer"`;
other main controls are transcript (`area:"transcript"` is an explicit alias).
The dark composer is one rounded, focus-accented surface enclosing the native
input and actions, with a quiet borderless edit instead of nested heavy frames.
When a composer command row fits and ends in a primary action, leading actions
stay on the left and the trailing primary action anchors to the right. Wrapped
rows retain ordinary ordering and spacing.
Native outlines return in high contrast. Side navigation retains the workspace
rail rules. When the footer would consume over three fifths of the client or
leave under 140 logical pixels of transcript, all controls use ordinary shared
scrolling. This keeps controls reachable in short or compact windows.

`conversation` is an optional stable string identifying the conversation/page.
With `follow_tail:true` (default), an update follows new content when the user
has actually reached the bottom, or when identity changes. Any explicit upward
scroll disables following immediately, including a fractional movement or a
coarse-wheel motion that has not yet produced its first animation tick. An
identical refresh therefore cannot snap a reader back from near the bottom.
Reading above that threshold keeps the first surviving visible transcript
control anchored where scroll bounds permit, including bounded windows that
drop older items and native resize/DPI reflow. `follow_tail:false` never requests
tail following; a changed identity resets to the top, suitable for older pages.
Use stable control IDs and change the conversation identity at page boundaries.

`kind:"detail"` with `role:"incoming"` or `role:"outgoing"` uses the actual
native edit line count at its assigned width to size selectable, wrapped text.
`lines` is ignored for these roles. A nonempty `label` appears exactly once as
the native caption above the body unless `caption:false` is supplied.
Incoming prose is an unthemed borderless native read-only edit in dark mode,
including its client and nonclient paint paths. All edits are constructed
without the classic control's cached client border; ordinary fields receive a
mutable window outline afterward, using system colors outside dark mode.
This also preserves borderless painting after theme/role changes on the same
HWND. Native client/print paint calls and ordinary nonclient outlines have a
bounded CDB call-trace contract; public style bits alone are insufficient.
The default conversation column is 744 logical pixels; workspace bounds are
unchanged. Outgoing bubbles size to their unwrapped
text plus padding, at most three quarters of the column (capped at 640 logical
pixels), and align to the right unless `width` explicitly overrides sizing.
Extremely long individual bodies are bounded to the smaller of 12,000
logical/20,000 physical pixels and retain native internal scrolling. Text is
never shortened by this sizing policy.

`composer:"draft-id"` opts one multiline field into Enter-to-send using the
button named by frame `default`. Shift+Enter keeps ordinary native newline
editing. Ctrl+Enter remains supported. Repeated keydown does not duplicate a
send, and native IME composition notifications plus the current IMM composition
buffer prevent Return from submitting while text is being composed. Without
`composer`, multiline Enter retains its previous behavior. Actual IME and
assistive-technology qualification remains separate from the bounded contracts.
If the designated composer has a nonempty unacknowledged local edit while the
default button still has a disabled presentation, Enter can emit a single
ordered click intent with `pending_edit:<edit serial>`. The preceding change
event is already in the same outgoing channel. This avoids swallowing the
first-keystroke/Enter sequence while the controller updates readiness; the
controller must still enforce its authoritative submission conditions. This
fallback is limited to one intent per edit serial and never enables the native
button or permits disabled mouse activation. Empty/acknowledged drafts, IME
composition and repeated keys do not use the fallback.

An optional single `kind:"heading",area:"header"` control pins the conversation or workspace
identity above the transcript. `role:"title"` uses compact 20-pixel typography,
a 52-pixel header and quiet separator; a long title is single-line ellipsized
while retaining its full native label. Header and composer remain outside the
clipped transcript. At very short heights the header returns to ordinary flow.
The composer uses compact inner padding and action rows; `lines:2` is a useful
starting point. Its borderless inner multiline edit shows an internal scrollbar
only when actual native text overflows. If there is too little room to pin the
composer, it returns to the scrollable flow with ordinary native field outlines
and keyboard reveal, preserving HWNDs, focus, text and selection.

Touchscreen contacts on read-only conversation text (including incoming and
outgoing bodies), captions and exposed transcript background pan the viewport.
An 8-DIP direction threshold avoids jitter; movement then follows physical input
coordinates directly, without extra DPI scaling or invented momentum/zoom.
The handled contact, including tap/end/cancel and pointer-capture loss, stays
out of native text selection and click processing. Mouse and pen selection,
keyboard focus/copy and editable composer gestures retain their native paths.
Fixed composer, header and navigation areas are excluded from transcript pans.

The host consumes a claimed `PT_TOUCH` contact through its native pointer
lifecycle and uses Windows' implicit capture. Touch-promoted compatibility mouse
contacts have a signature/input-source fallback with explicit, owned mouse
capture. Only overlapping promoted/native contacts are deduplicated: subsequent
independent compatibility-only drags remain usable. Cancellation, owner removal,
conversation/layout change and destruction clean up capture; no assumption of
paired down/up messages is required. Internal routing uses a scoped thread-local
packet with a root/nonce check, not a message-supplied pointer. No frame/event
ABI changes are needed. See Microsoft's [pointer contact documentation](https://learn.microsoft.com/en-us/windows/win32/inputmsg/wm-pointerdown)
and [promoted mouse signature](https://learn.microsoft.com/en-us/windows/win32/tablet/system-events-and-mouse-messages).
Hidden routing/state contracts and CDB traces do not establish hardware gesture
qualification; actual touch-device review remains separate.

```json
{"theme":"dark","layout":"conversation","conversation":"project/newest",
 "composer":"draft","default":"send","follow_tail":true,"controls":[
  {"id":"reply","kind":"detail","role":"incoming","label":"Assistant","value":"Readable prose."},
  {"id":"draft","kind":"multiline","area":"composer","label":"Message","lines":3,"value":""},
  {"id":"attach","kind":"button","area":"composer","role":"quiet","label":"Attach","icon":"attachment","row":"actions","width":120,"row_align":"end"},
  {"id":"send","kind":"button","area":"composer","role":"primary","label":"Send","icon":"send","row":"actions","width":100}
 ]}
```

Scrollable controls live under a dedicated clipped, composited native viewport
and a translated content host, separated from the stationary rail and composer.
Pure scrolling moves that single host with no sizing, copying old contents,
control remeasurement or per-control viewport-region changes. Only the viewport
subtree is invalidated, without a full-root erase/frame redraw; custom background
painting occurs once in WM_PAINT rather than also during WM_ERASEBKGND. The
ancestor viewport clips bodies/captions above the footer. Rounded bubble shapes
remain independent of scroll position.

Win32 caps native window height, so the content host uses a bounded 60,000-pixel
canvas and a moving 30,000-pixel origin. Crossing a band repositions descendants
without recreating HWNDs, preserving logical history beyond 65,535 pixels.
Off-band controls remain reachable through native keyboard navigation and
reveal. Both intermediate windows use WS_EX_CONTROLPARENT; native commands,
colors, focus/selection, Tab order, wheel events and file drops retain their
routes. Single-line dark edits paint all their reserved nonclient padding and
use one custom border, avoiding partially themed inner frames.

Passive body wheel messages
reach the outer viewport; editable fields/lists retain native internal scrolling
while they have range, then pass the wheel outward. Windows wheel line/page
preferences and fractional wheel deltas are respected. Precision/touchpad input
remains immediate. Coarse detents use a 120-ms time-based trajectory when client
area animation is enabled; the nominal 10-ms timer exists only while motion is
active. Reversal retargets from the current position, direct scrollbar dragging
and explicit edits/activations take immediate control, and reduced-motion
settings bypass the trajectory. Input-source identity survives native parent
forwarding. Changing page/layout cancels motion. No display-refresh-rate or
frame-time guarantee is implied. Default window messages
are delegated outside the mutable-state guard so native nested resize messages
are delivered.

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
controls. `caption:false` hides this visible caption without removing the
native edit/selection control's accessible name: MSAA dynamic annotations use
the full `label` (and are cleared at destruction). If annotation fails, the
visible native caption is retained. Toggling caption visibility keeps HWNDs.
`placeholder:"Write a message…"` adds an empty-value hint to `input`/`multiline`.
The hint never becomes text, selection, a value or a change event, and yields
to native text and IME composition. It uses system text treatment in high
contrast/system mode. Native standard controls expose their accessibility and IME
behavior. No screen-reader or visual qualification is claimed.
Labels are literal: static captions disable prefix processing and button/check
labels escape Win32 accelerator ampersands. Empty choice arrays are accepted.

In legacy system frames, controls sharing a non-null `row` normally share
available width; explicit widths and compact wrapping are also supported.
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

Validation is limited to typed frame checks and explicitly hidden,
in-process native lifecycle and paint-message contracts. They check content,
UTF16 offsets, stale edits, HWND/selection retention, coalescing, dark/legacy
layout, native wrapping, nested viewport/Tab/color/command/drop routing,
large-content origin changes, explicit scroll-away intent, deterministic motion
endpoints/reversal/timer shutdown, footer/rail clipping, history anchors,
wheel routing, modifier/IME send policy, resize and destruction. They neither
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

The updated investigation used CDB 10.0.28000.2705 and a source-bound hidden probe
at baseline `b7d560e`: a synthetic 33-character outgoing message, a 1,013-character
incoming message with four newlines, a composer/rail, and an ordinary workspace.
It reproduced precision-scroll/refresh snap-back (307 to 294 to 307 physical
pixels), 269 pixels of blank short-chat range from the fixed rail, and one
workspace wheel event doing a full layout, six region changes, 40 child moves
and full-root subtree redraw. The candidate preserves 307 to 294 to 294, has
zero blank short-chat range, and uses a scroll-only translated-host path with
zero layout/region/root-redraw/edit-size work. Coarse movement is sampled over
time with deterministic monotonicity, endpoint and reversal contracts.
Hidden windows produce no ordinary spontaneous paint stream, so these are
mechanism/state guarantees and debugger evidence, not live visual acceptance
or a frame-time benchmark.
