#![allow(unsafe_op_in_unsafe_fn)]
use super::*;
use serde_json::json;
use std::{
    cell::Cell,
    collections::HashMap,
    ptr::null,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::*,
    System::SystemServices::{SS_ELLIPSISMASK, SS_ENDELLIPSIS, SS_LEFTNOWORDWRAP, SS_TYPEMASK},
    UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};
mod access;
mod dark;
mod motion;
mod touch;
use motion::Motion;
use windows_sys::Win32::UI::Input::{
    GetCurrentInputMessageSource, IMDT_TOUCHPAD, INPUT_MESSAGE_SOURCE,
};
const SCROLL_TIMER: usize = 0x46525343;
const CANVAS_SPAN: i32 = 60000;
const ORIGIN_STEP: i32 = 30000;
fn client_animation_enabled() -> bool {
    unsafe {
        let mut enabled = 1i32;
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            &mut enabled as *mut _ as *mut _,
            0,
        );
        enabled != 0
    }
}
type NativeMove = (HWND, i32, i32, i32, i32, bool);
fn section_key(c: &Control) -> Option<&str> {
    (!c.side && !matches!(c.area, Some(Area::Header | Area::Composer)))
        .then(|| c.section.as_deref().filter(|s| !s.is_empty()))
        .flatten()
}
use windows_sys::Win32::UI::Input::Ime::{
    GCS_COMPSTR, ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext,
};
fn enter_sends(
    ctrl: bool,
    shift: bool,
    alt: bool,
    repeat: bool,
    designated: bool,
    composing: bool,
) -> bool {
    !composing && !alt && !repeat && (ctrl || designated && !shift)
}
fn composing(w: &Widget) -> bool {
    unsafe {
        if w.surface.ime.get() {
            return true;
        }
        let context = ImmGetContext(w.hwnd);
        if context == 0 {
            return false;
        }
        let active = ImmGetCompositionStringW(context, GCS_COMPSTR, std::ptr::null_mut(), 0) > 0;
        ImmReleaseContext(w.hwnd, context);
        active
    }
}

const FRAME: u32 = WM_APP + 17;
// SS_NOPREFIX is a STATIC style: labels are literal text, not mnemonics.
const STATIC_LITERAL: u32 = 0x80;
type Placement = (String, i32, i32, i32, i32, i32);
thread_local! { static UPDATING: Cell<bool> = const { Cell::new(false) }; }
thread_local! {static WHEEL_TOUCHPAD:Cell<Option<bool>>=const{Cell::new(None)};}
fn input_is_touchpad() -> bool {
    WHEEL_TOUCHPAD
        .with(|saved| saved.get())
        .unwrap_or_else(|| unsafe {
            let mut source: INPUT_MESSAGE_SOURCE = std::mem::zeroed();
            GetCurrentInputMessageSource(&mut source) != 0 && source.deviceType == IMDT_TOUCHPAD
        })
}
fn forward_wheel(parent: HWND, wp: usize, lp: isize) -> isize {
    unsafe {
        // SendMessage can obscure the original input source. Carry it across the
        // native parent chain without changing the wheel message's payload.
        let touchpad = input_is_touchpad();
        let previous = WHEEL_TOUCHPAD.with(|source| source.replace(Some(touchpad)));
        let result = SendMessageW(parent, WM_MOUSEWHEEL, wp, lp);
        WHEEL_TOUCHPAD.with(|source| source.set(previous));
        result
    }
}
#[cfg(test)]
thread_local! {static SCROLL_TRACE:Cell<[u64;4]>=const{Cell::new([0;4])};}
#[cfg(test)]
fn trace_slot(index: usize) {
    SCROLL_TRACE.with(|c| {
        let mut n = c.get();
        n[index] += 1;
        c.set(n);
    });
}
struct UpdateGuard(bool);
impl UpdateGuard {
    fn enter() -> Self {
        Self(UPDATING.with(|v| v.replace(true)))
    }
}
impl Drop for UpdateGuard {
    fn drop(&mut self) {
        UPDATING.with(|v| v.set(self.0));
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn text(hwnd: HWND) -> String {
    unsafe {
        let mut buf = vec![0u16; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}
fn set_text(hwnd: HWND, s: &str) {
    unsafe {
        SetWindowTextW(hwnd, wide(s).as_ptr());
    }
}
fn clip_window(
    hwnd: HWND,
    bounds: (i32, i32, i32, i32),
    viewport: &RECT,
    radius: i32,
    cache: &mut Option<(i32, i32, i32, i32, i32)>,
) -> bool {
    unsafe {
        let (x, y, width, height) = bounds;
        let left = (viewport.left - x).clamp(0, width);
        let top = (viewport.top - y).clamp(0, height);
        let right = (viewport.right - x).clamp(left, width);
        let bottom = (viewport.bottom - y).clamp(top, height);
        let next = (left, top, right, bottom, radius);
        if *cache == Some(next) {
            return false;
        }
        #[cfg(test)]
        trace_slot(1);
        if next == (0, 0, width, height, 0) {
            SetWindowRgn(hwnd, 0, 0);
            *cache = Some(next);
            return true;
        }
        let region = CreateRectRgn(left, top, right, bottom);
        if region == 0 {
            return false;
        }
        if radius > 0 {
            let rounded = CreateRoundRectRgn(0, 0, width + 1, height + 1, radius, radius);
            if rounded != 0 {
                CombineRgn(region, region, rounded, RGN_AND);
                DeleteObject(rounded);
            }
        }
        if SetWindowRgn(hwnd, region, 0) == 0 {
            DeleteObject(region);
        } else {
            *cache = Some(next);
        }
        true
    }
}
struct Widget {
    hwnd: HWND,
    caption: HWND,
    font: HFONT,
    caption_font: HFONT,
    surface: Box<dark::Surface>,
    caption_surface: Option<Box<dark::Surface>>,
    spec: Control,
    serial: u64,
    rect: (i32, i32, i32, i32),
    placed: Option<(i32, i32, i32, i32, i32)>,
    clip: Option<(i32, i32, i32, i32, i32)>,
    caption_clip: Option<(i32, i32, i32, i32, i32)>,
    chat_height: Option<(i32, i32)>,
    fixed: bool,
    native_placed: Option<(HWND, i32, i32, i32, i32, i32)>,
}
impl Drop for Widget {
    fn drop(&mut self) {
        unsafe {
            if self.caption != 0 {
                access::clear(self.hwnd);
            }
            DestroyWindow(self.hwnd);
            if self.caption != 0 {
                DestroyWindow(self.caption);
            }
            DeleteObject(self.font);
            if self.caption_font != 0 {
                DeleteObject(self.caption_font);
            }
        }
    }
}
#[derive(Default)]
struct Pending {
    frame: Option<Frame>,
    error: Option<String>,
    posted: bool,
}
impl Pending {
    fn push(&mut self, mut frame: Frame) {
        frame.focus_ack = frame.focus.as_ref().map(|_| frame.ack);
        frame.selection_ack = frame.selection.as_ref().map(|_| frame.ack);
        if let Some(previous) = self.frame.take() {
            if frame.focus.is_none() {
                frame.focus = previous.focus;
                frame.focus_ack = previous.focus_ack;
            }
            if frame.selection.is_none() {
                frame.selection = previous.selection;
                frame.selection_ack = previous.selection_ack;
            }
            frame.close |= previous.close;
        }
        self.frame = Some(frame);
    }
}
// Clamp to existing text and never bisect a UTF16 surrogate pair.
fn caret_offset(value: &str, requested: u32) -> u32 {
    let mut at = 0;
    for c in value.chars() {
        let next = at + c.len_utf16() as u32;
        if next > requested {
            return at;
        }
        at = next;
    }
    at
}
struct State {
    hwnd: HWND,
    widgets: HashMap<String, Widget>,
    order: Vec<String>,
    outgoing: Sender<Value>,
    pending: Arc<Mutex<Pending>>,
    serial: u64,
    default: Option<String>,
    scroll: i32,
    content_height: i32,
    dpi: u32,
    previous_dpi: DPI_AWARENESS_CONTEXT,
    palette: std::rc::Rc<dark::Palette>,
    background: Box<dark::Surface>,
    theme: Theme,
    layout_mode: Option<Layout>,
    max_width: Option<u32>,
    fixed_rail: bool,
    composer_fixed: bool,
    composer: Option<String>,
    conversation: Option<String>,
    follow_tail: bool,
    follow_on_layout: bool,
    anchor: Option<(String, i32)>,
    viewport_height: i32,
    wheel_remainder: i64,
    viewport: HWND,
    content: HWND,
    viewport_surface: Box<dark::Surface>,
    content_surface: Box<dark::Surface>,
    view_rect: (i32, i32, i32, i32),
    origin: i32,
    content_panel: Option<(i32, i32, i32, i32)>,
    section_rects: Vec<(i32, i32, i32, i32)>,
    tab_rects: Vec<(i32, i32, i32, i32)>,
    scroll_rail: i32,
    viewport_box: Option<(i32, i32, i32, i32)>,
    content_box: Option<(i32, i32, i32, i32)>,
    follow_intent: bool,
    tab_dirty: bool,
    scene_dirty: bool,
    motion: Motion,
    motion_clock: std::time::Instant,
    timer_active: bool,
    acknowledged: u64,
    last_keyboard_intent: u64,
    touch: touch::Pan,
}
impl State {
    fn scale(&self, n: i32) -> i32 {
        ((n as i64 * self.dpi as i64) / 96) as i32
    }
    fn create(&self, spec: Control) -> Result<Widget> {
        unsafe {
            let (class, style) = match spec.kind {
                Kind::Label | Kind::Heading => ("STATIC", STATIC_LITERAL),
                Kind::Button => (
                    "BUTTON",
                    (if spec.role == Role::Tab {
                        BS_RADIOBUTTON | BS_PUSHLIKE
                    } else {
                        BS_PUSHBUTTON
                    }) as u32
                        | WS_TABSTOP,
                ),
                Kind::Check => ("BUTTON", BS_AUTOCHECKBOX as u32 | WS_TABSTOP),
                Kind::Select => (
                    "COMBOBOX",
                    CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP,
                ),
                Kind::Multi => (
                    "LISTBOX",
                    LBS_EXTENDEDSEL as u32
                        | LBS_NOTIFY as u32
                        | WS_VSCROLL
                        | WS_HSCROLL
                        | WS_TABSTOP
                        | WS_BORDER,
                ),
                Kind::List => (
                    "LISTBOX",
                    LBS_NOTIFY as u32 | WS_VSCROLL | WS_HSCROLL | WS_TABSTOP | WS_BORDER,
                ),
                Kind::Multiline => (
                    "EDIT",
                    ES_MULTILINE as u32
                        | ES_AUTOVSCROLL as u32
                        | ES_WANTRETURN as u32
                        | WS_VSCROLL
                        | WS_TABSTOP
                        | WS_BORDER,
                ),
                Kind::Detail => (
                    "EDIT",
                    ES_MULTILINE as u32
                        | ES_READONLY as u32
                        | ES_AUTOVSCROLL as u32
                        | WS_VSCROLL
                        | WS_TABSTOP
                        | WS_BORDER,
                ),
                Kind::Input => ("EDIT", ES_AUTOHSCROLL as u32 | WS_TABSTOP | WS_BORDER),
            };
            let edit = matches!(spec.kind, Kind::Input | Kind::Multiline | Kind::Detail);
            // Classic EDIT caches WS_BORDER during creation and draws that
            // frame inside WM_PAINT/WM_PRINTCLIENT even after its public style
            // is cleared. Create without that immutable internal decoration;
            // ordinary modes use the mutable window/nonclient outline below.
            let creation_style = if edit { style & !WS_BORDER } else { style };
            let instance = GetModuleHandleW(null());
            let hwnd = CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | creation_style,
                0,
                0,
                0,
                0,
                self.content,
                0,
                instance,
                null(),
            );
            if hwnd == 0 {
                bail!("CreateWindowExW failed: {}", GetLastError())
            }
            if edit && style & WS_BORDER != 0 {
                let current = GetWindowLongW(hwnd, GWL_STYLE) as u32;
                SetWindowLongW(hwnd, GWL_STYLE, (current | WS_BORDER) as i32);
                SetWindowPos(
                    hwnd,
                    0,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
            let labelled = matches!(
                spec.kind,
                Kind::Input
                    | Kind::Multiline
                    | Kind::Select
                    | Kind::Multi
                    | Kind::List
                    | Kind::Detail
            ) && !spec.label.is_empty();
            let caption = if labelled {
                CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide(&spec.label).as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | STATIC_LITERAL,
                    0,
                    0,
                    0,
                    0,
                    self.content,
                    0,
                    instance,
                    null(),
                )
            } else {
                0
            };
            let font = self.font(&spec);
            let mut caption_spec = spec.clone();
            caption_spec.role = Role::Caption;
            let caption_font = if caption != 0 {
                self.font(&caption_spec)
            } else {
                0
            };
            let surface = dark::Surface::attach(hwnd, Some(spec.kind), self.palette.clone());
            let caption_surface = if caption != 0 {
                Some(dark::Surface::attach(
                    caption,
                    Some(Kind::Label),
                    self.palette.clone(),
                ))
            } else {
                None
            };
            SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
            if caption != 0 {
                SendMessageW(caption, WM_SETFONT, caption_font as usize, 1);
            }
            if matches!(spec.kind, Kind::Input | Kind::Multiline | Kind::Detail) {
                SendMessageW(hwnd, EM_SETLIMITTEXT, 16 * 1024 * 1024, 0);
            }
            let mut widget = Widget {
                hwnd,
                caption,
                font,
                caption_font,
                surface,
                caption_surface,
                spec: spec.clone(),
                serial: 0,
                rect: (0, 0, 0, 0),
                placed: None,
                clip: None,
                caption_clip: None,
                chat_height: None,
                fixed: false,
                native_placed: None,
            };
            self.update(&mut widget, spec, true, u64::MAX);
            Ok(widget)
        }
    }
    fn font(&self, c: &Control) -> HFONT {
        unsafe {
            let modern = self.theme == Theme::Dark;
            let logical = if modern && c.area == Some(Area::Header) {
                20
            } else if modern {
                match c.role {
                    Role::Title => 30,
                    Role::Brand => 22,
                    Role::Caption => 11,
                    Role::Muted => 14,
                    Role::Body | Role::Incoming | Role::Outgoing => 16,
                    _ => {
                        if c.kind == Kind::Heading {
                            19
                        } else {
                            14
                        }
                    }
                }
            } else if c.kind == Kind::Heading {
                20
            } else {
                16
            };
            let size = (self.scale(logical) as f64 * c.zoom).round() as i32;
            CreateFontW(
                -size,
                0,
                0,
                0,
                if (matches!(
                    c.role,
                    Role::Brand | Role::Title | Role::Primary | Role::Selected
                ) && !(c.role == Role::Selected && c.icon == Some(Icon::Folder)))
                    || c.role == Role::Tab && c.selected
                    || c.kind == Kind::Heading
                {
                    600
                } else {
                    400
                },
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                wide("Segoe UI").as_ptr(),
            )
        }
    }
    fn update(&self, w: &mut Widget, mut c: Control, first: bool, ack: u64) {
        unsafe {
            if c.value != w.spec.value || c.role != w.spec.role || c.zoom != w.spec.zoom {
                w.chat_height = None;
            }
            if w.surface.expanded.replace(c.expanded) != c.expanded {
                InvalidateRect(w.hwnd, null(), 0);
            }
            if w.surface.icon.replace(c.icon) != c.icon {
                InvalidateRect(w.hwnd, null(), 0);
            }
            if w.surface.selected.replace(c.selected) != c.selected {
                InvalidateRect(w.hwnd, null(), 0);
            }
            if c.kind == Kind::Button {
                if first || c.role != w.spec.role {
                    let old = GetWindowLongW(w.hwnd, GWL_STYLE) as u32;
                    let style = if c.role == Role::Tab {
                        BS_RADIOBUTTON | BS_PUSHLIKE
                    } else {
                        BS_PUSHBUTTON
                    } as u32;
                    SetWindowLongW(
                        w.hwnd,
                        GWL_STYLE,
                        ((old & !(BS_TYPEMASK as u32 | BS_PUSHLIKE as u32)) | style) as i32,
                    );
                }
                if c.role == Role::Tab
                    && (first || c.selected != w.spec.selected || c.role != w.spec.role)
                {
                    SendMessageW(
                        w.hwnd,
                        BM_SETCHECK,
                        if c.selected {
                            BST_CHECKED
                        } else {
                            BST_UNCHECKED
                        } as usize,
                        0,
                    );
                }
            }
            w.surface
                .configure(self.background.active.get(), c.role, c.side, self.dpi);
            if let Some(caption) = &w.caption_surface {
                caption.configure(
                    self.background.active.get(),
                    Role::Caption,
                    c.side,
                    self.dpi,
                );
            }
            let protected = w.serial > ack;
            if protected {
                c.value = w.spec.value.clone();
                c.choices = w.spec.choices.clone();
            }
            if first || c.enabled != w.spec.enabled {
                EnableWindow(w.hwnd, i32::from(c.enabled));
            }
            if first || c.label != w.spec.label {
                if w.caption != 0 {
                    set_text(w.caption, &c.label);
                }
                if matches!(
                    c.kind,
                    Kind::Label | Kind::Heading | Kind::Button | Kind::Check
                ) {
                    let label = if matches!(c.kind, Kind::Button | Kind::Check) {
                        c.label.replace('&', "&&")
                    } else {
                        c.label.clone()
                    };
                    set_text(w.hwnd, &label);
                }
            }
            if (first || c.label != w.spec.label || c.caption != w.spec.caption) && w.caption != 0 {
                if !access::name(w.hwnd, &c.label) && !c.caption {
                    // Preserve the native preceding-label fallback if MSAA
                    // annotation is unavailable on this Windows installation.
                    c.caption = true;
                }
                ShowWindow(
                    w.caption,
                    if c.caption {
                        SW_SHOWNOACTIVATE
                    } else {
                        SW_HIDE
                    },
                );
            }
            if c.kind == Kind::Heading && (first || c.area != w.spec.area) {
                let old = GetWindowLongW(w.hwnd, GWL_STYLE) as u32;
                let header = if c.area == Some(Area::Header) {
                    SS_LEFTNOWORDWRAP | SS_ENDELLIPSIS
                } else {
                    0
                };
                SetWindowLongW(
                    w.hwnd,
                    GWL_STYLE,
                    ((old & !(SS_TYPEMASK | SS_ELLIPSISMASK)) | header) as i32,
                );
            }
            w.surface.set_placeholder(&c.placeholder);
            let choices_changed = first || c.choices != w.spec.choices;
            let top = if matches!(c.kind, Kind::List | Kind::Multi) {
                SendMessageW(w.hwnd, LB_GETTOPINDEX, 0, 0)
            } else {
                0
            };
            let horizontal = GetScrollPos(w.hwnd, SB_HORZ);
            if choices_changed && matches!(c.kind, Kind::Select | Kind::List | Kind::Multi) {
                let combo = c.kind == Kind::Select;
                SendMessageW(
                    w.hwnd,
                    if combo {
                        CB_RESETCONTENT
                    } else {
                        LB_RESETCONTENT
                    },
                    0,
                    0,
                );
                let mut longest = 0;
                for choice in &c.choices {
                    SendMessageW(
                        w.hwnd,
                        if combo { CB_ADDSTRING } else { LB_ADDSTRING },
                        0,
                        wide(&choice.label).as_ptr() as isize,
                    );
                    longest =
                        longest.max(choice.label.encode_utf16().count() as i32 * self.scale(9));
                }
                if !combo {
                    SendMessageW(w.hwnd, LB_SETHORIZONTALEXTENT, longest as usize, 0);
                }
            }
            if first || c.value != w.spec.value || choices_changed {
                match c.kind {
                    Kind::Check => {
                        SendMessageW(
                            w.hwnd,
                            BM_SETCHECK,
                            if c.value.as_bool().unwrap_or(false) {
                                BST_CHECKED as usize
                            } else {
                                BST_UNCHECKED as usize
                            },
                            0,
                        );
                    }
                    Kind::Select | Kind::List => {
                        let index = c
                            .choices
                            .iter()
                            .position(|x| Some(x.value.as_str()) == c.value.as_str())
                            .map(|n| n as isize)
                            .unwrap_or(-1);
                        SendMessageW(
                            w.hwnd,
                            if c.kind == Kind::Select {
                                CB_SETCURSEL
                            } else {
                                LB_SETCURSEL
                            },
                            index as usize,
                            0,
                        );
                    }
                    Kind::Multi => {
                        SendMessageW(w.hwnd, LB_SETSEL, 0, -1);
                        for (i, choice) in c.choices.iter().enumerate() {
                            if c.value.as_array().is_some_and(|a| {
                                a.iter().any(|x| x.as_str() == Some(&choice.value))
                            }) {
                                SendMessageW(w.hwnd, LB_SETSEL, 1, i as isize);
                            }
                        }
                    }
                    Kind::Input | Kind::Multiline | Kind::Detail => {
                        let value = c.value.as_str().unwrap_or("");
                        if text(w.hwnd) != value {
                            let mut start = 0u32;
                            let mut end = 0u32;
                            SendMessageW(
                                w.hwnd,
                                EM_GETSEL,
                                &mut start as *mut _ as usize,
                                &mut end as *mut _ as isize,
                            );
                            let line = SendMessageW(w.hwnd, EM_GETFIRSTVISIBLELINE, 0, 0);
                            set_text(w.hwnd, value);
                            let len = value.encode_utf16().count() as u32;
                            SendMessageW(
                                w.hwnd,
                                EM_SETSEL,
                                start.min(len) as usize,
                                end.min(len) as isize,
                            );
                            let current = SendMessageW(w.hwnd, EM_GETFIRSTVISIBLELINE, 0, 0);
                            SendMessageW(w.hwnd, EM_LINESCROLL, 0, line - current);
                            w.surface.composer_overflow();
                        }
                    }
                    _ => {}
                }
            }
            if choices_changed && matches!(c.kind, Kind::List | Kind::Multi) {
                SendMessageW(w.hwnd, LB_SETTOPINDEX, top as usize, 0);
                SendMessageW(
                    w.hwnd,
                    WM_HSCROLL,
                    (SB_THUMBPOSITION as usize) | ((horizontal as usize) << 16),
                    0,
                );
            }
            if c.zoom != w.spec.zoom
                || c.role != w.spec.role
                || c.area != w.spec.area
                || c.role == Role::Tab && c.selected != w.spec.selected
            {
                let font = self.font(&c);
                SendMessageW(w.hwnd, WM_SETFONT, font as usize, 1);
                if w.caption != 0 {
                    let mut spec = c.clone();
                    spec.role = Role::Caption;
                    let caption_font = self.font(&spec);
                    SendMessageW(w.caption, WM_SETFONT, caption_font as usize, 1);
                    DeleteObject(w.caption_font);
                    w.caption_font = caption_font;
                }
                DeleteObject(w.font);
                w.font = font;
            }
            w.spec = c;
        }
    }
    fn apply(&mut self, f: Frame) -> Result<()> {
        let _guard = UpdateGuard::enter();
        self.apply_inner(f)
    }
    fn apply_inner(&mut self, f: Frame) -> Result<()> {
        unsafe {
            if f.close {
                PostQuitMessage(0);
                return Ok(());
            }
            if let Some(title) = f.title
                && text(self.hwnd) != title
            {
                set_text(self.hwnd, &title);
            }
            let theme_changed = self.theme != f.theme;
            self.acknowledged = self.acknowledged.max(f.ack.min(self.serial));
            let surviving: std::collections::HashSet<_> =
                f.controls.iter().map(|c| c.id.as_str()).collect();
            let same_conversation = self.conversation == f.conversation;
            let layout_changed = self.layout_mode != f.layout;
            let at_tail = self.follow_intent && self.scroll == self.scroll_limit();
            self.follow_on_layout = f.layout == Some(Layout::Conversation)
                && f.follow_tail
                && (!same_conversation || layout_changed || at_tail);
            self.anchor = if f.layout == Some(Layout::Conversation)
                && same_conversation
                && !self.follow_on_layout
            {
                self.order
                    .iter()
                    .filter_map(|id| {
                        self.widgets
                            .get(id)
                            .filter(|w| {
                                !w.spec.side
                                    && !matches!(w.spec.area, Some(Area::Composer | Area::Header))
                                    && w.rect.1 + w.rect.3 > self.scroll
                                    && w.rect.1 < self.scroll + self.viewport_height
                                    && surviving.contains(id.as_str())
                            })
                            .map(|w| (id.clone(), self.scroll - w.rect.1))
                    })
                    .next()
            } else {
                None
            };
            if !same_conversation || layout_changed {
                self.cancel_touch();
                self.stop_motion();
                self.follow_intent = true;
                if f.layout == Some(Layout::Conversation) || layout_changed {
                    self.scroll = 0;
                }
                self.wheel_remainder = 0;
            }
            self.conversation = f.conversation;
            self.follow_tail = f.follow_tail;
            self.composer = f.composer;
            self.theme = f.theme;
            self.layout_mode = f.layout;
            self.max_width = f.max_width;
            self.refresh_theme(dark::high_contrast(), theme_changed);
            self.default = f.default;
            let old_focus = GetFocus();
            let focus_id = self
                .widgets
                .iter()
                .find(|(_, w)| w.hwnd == old_focus)
                .map(|(id, _)| id.clone());
            let mut next = HashMap::new();
            let mut recreated = false;
            let mut navigation_changed = false;
            let mut order = Vec::new();
            for c in f.controls {
                order.push(c.id.clone());
                let mut old = self.widgets.remove(&c.id);
                navigation_changed |= old
                    .as_ref()
                    .is_some_and(|w| w.spec.role != c.role || w.spec.row != c.row);
                // Caption availability and class determine HWND lifetime; ordinary content changes do not.
                let labelled = |x: &Control| {
                    !x.label.is_empty()
                        && matches!(
                            x.kind,
                            Kind::Input
                                | Kind::Multiline
                                | Kind::Select
                                | Kind::Multi
                                | Kind::List
                                | Kind::Detail
                        )
                };
                let w = if old
                    .as_ref()
                    .is_some_and(|w| w.spec.kind == c.kind && labelled(&w.spec) == labelled(&c))
                {
                    let mut w = old.take().unwrap();
                    self.update(&mut w, c.clone(), false, f.ack);
                    w
                } else {
                    recreated = true;
                    self.create(c.clone())?
                };
                next.insert(c.id, w);
            }
            let reordered = recreated || self.order != order;
            self.widgets = next;
            self.order = order;
            if self.touch.owner().is_some_and(|owner| IsWindow(owner) == 0) {
                self.cancel_touch();
            }
            self.tab_dirty |= reordered || navigation_changed;
            self.scene_dirty |= reordered;
            self.layout();
            let focus = f
                .focus
                .filter(|_| self.serial <= f.focus_ack.unwrap_or(f.ack));
            if let Some(id) = focus.or(focus_id.filter(|_| GetFocus() == 0))
                && let Some(w) = self.widgets.get(&id)
            {
                SetFocus(w.hwnd);
                self.reveal(w.hwnd);
            }
            if let Some(selection) = f.selection
                && self.serial <= f.selection_ack.unwrap_or(f.ack)
                && let Some(w) = self.widgets.get(&selection.id)
                && w.serial <= f.selection_ack.unwrap_or(f.ack)
                && matches!(w.spec.kind, Kind::Input | Kind::Multiline | Kind::Detail)
            {
                let value = text(w.hwnd);
                SendMessageW(
                    w.hwnd,
                    EM_SETSEL,
                    caret_offset(&value, selection.start) as usize,
                    caret_offset(&value, selection.end) as isize,
                );
                SendMessageW(w.hwnd, EM_SCROLLCARET, 0, 0);
            }
            Ok(())
        }
    }
    fn refresh_theme(&mut self, high_contrast: bool, rebuild_fonts: bool) {
        unsafe {
            let _guard = UpdateGuard::enter();
            let active = dark::enabled(self.theme, high_contrast);
            self.background
                .configure(active, Role::Neutral, false, self.dpi);
            self.viewport_surface
                .configure(active, Role::Neutral, false, self.dpi);
            self.content_surface
                .configure(active, Role::Neutral, false, self.dpi);
            dark::title_bar(self.hwnd, active);
            for w in self.widgets.values_mut() {
                w.surface
                    .configure(active, w.spec.role, w.spec.side, self.dpi);
                if let Some(caption) = &w.caption_surface {
                    caption.configure(active, Role::Caption, w.spec.side, self.dpi);
                }
            }
            if rebuild_fonts {
                let ids = self.order.clone();
                for id in ids {
                    let spec = self.widgets[&id].spec.clone();
                    let font = self.font(&spec);
                    let mut caption_spec = spec.clone();
                    caption_spec.role = Role::Caption;
                    let caption_font = if self.widgets[&id].caption != 0 {
                        self.font(&caption_spec)
                    } else {
                        0
                    };
                    let w = self.widgets.get_mut(&id).unwrap();
                    SendMessageW(w.hwnd, WM_SETFONT, font as usize, 1);
                    DeleteObject(w.font);
                    w.font = font;
                    w.chat_height = None;
                    if w.caption != 0 {
                        SendMessageW(w.caption, WM_SETFONT, caption_font as usize, 1);
                        DeleteObject(w.caption_font);
                        w.caption_font = caption_font;
                    }
                }
            }
        }
    }
    fn text_width(&self, w: &Widget) -> i32 {
        unsafe {
            let dc = GetDC(w.hwnd);
            let old = SelectObject(dc, w.font);
            let value = wide(&w.spec.label);
            let mut size: SIZE = std::mem::zeroed();
            GetTextExtentPoint32W(
                dc,
                value.as_ptr(),
                value.len().saturating_sub(1) as i32,
                &mut size,
            );
            SelectObject(dc, old);
            ReleaseDC(w.hwnd, dc);
            size.cx
        }
    }
    fn outgoing_width(&self, w: &Widget, available: i32) -> i32 {
        unsafe {
            let dc = GetDC(w.hwnd);
            let old = SelectObject(dc, w.font);
            let mut widest = 0;
            for line in w.spec.value.as_str().unwrap_or("").split('\n') {
                let value = wide(line.trim_end_matches('\r'));
                let mut size: SIZE = std::mem::zeroed();
                GetTextExtentPoint32W(
                    dc,
                    value.as_ptr(),
                    value.len().saturating_sub(1) as i32,
                    &mut size,
                );
                widest = widest.max(size.cx);
            }
            SelectObject(dc, old);
            ReleaseDC(w.hwnd, dc);
            (widest + self.scale(28))
                .max(self.scale(72))
                .min((available * 3 / 4).min(self.scale(640)).max(1))
        }
    }
    fn scroll_limit(&self) -> i32 {
        (self.content_height - self.viewport_height).max(0)
    }
    fn visible_anchor(&self) -> Option<(String, i32)> {
        self.order.iter().find_map(|id| {
            let w = &self.widgets[id];
            (!w.spec.side
                && !matches!(w.spec.area, Some(Area::Composer | Area::Header))
                && w.rect.1 + w.rect.3 > self.scroll
                && w.rect.1 < self.scroll + self.viewport_height)
                .then(|| (id.clone(), self.scroll - w.rect.1))
        })
    }
    fn motion_now(&self) -> u64 {
        self.motion_clock.elapsed().as_millis() as u64
    }
    fn stop_motion(&mut self) {
        unsafe {
            if self.timer_active {
                KillTimer(self.hwnd, SCROLL_TIMER);
                self.timer_active = false;
            }
        }
        self.motion.stop(self.scroll);
    }
    fn scroll_direct(&mut self, position: i32) {
        self.stop_motion();
        self.wheel_remainder = 0;
        self.scroll = position.clamp(0, self.scroll_limit());
        self.follow_intent = self.scroll == self.scroll_limit();
        self.place_scene(true);
    }
    fn tick_at(&mut self, now: u64) {
        if !self.motion.active() {
            if self.timer_active {
                unsafe {
                    KillTimer(self.hwnd, SCROLL_TIMER);
                }
                self.timer_active = false;
            }
            return;
        }
        self.scroll = self.motion.sample(now).clamp(0, self.scroll_limit());
        self.place_scene(true);
        if !self.motion.active() {
            if self.scroll == self.scroll_limit() {
                self.follow_intent = true;
            }
            unsafe {
                KillTimer(self.hwnd, SCROLL_TIMER);
            }
            self.timer_active = false;
        }
    }
    fn wheel(&mut self, delta: i32) {
        let touchpad = input_is_touchpad();
        let smooth =
            delta.abs() >= 120 && delta % 120 == 0 && !touchpad && client_animation_enabled();
        self.wheel_with_policy(delta, smooth);
    }
    fn wheel_with_policy(&mut self, delta: i32, smooth: bool) {
        unsafe {
            let mut lines = 3u32;
            SystemParametersInfoW(
                SPI_GETWHEELSCROLLLINES,
                0,
                &mut lines as *mut _ as *mut _,
                0,
            );
            let step = if lines == u32::MAX {
                self.viewport_height
            } else {
                self.scale(24).saturating_mul(lines.min(100) as i32)
            };
            if delta > 0 {
                self.follow_intent = false;
            }
            self.wheel_remainder += delta as i64 * step as i64;
            let amount = (self.wheel_remainder / 120) as i32;
            self.wheel_remainder %= 120;
            let same_direction = self.motion.active()
                && (self.motion.target() - self.scroll).signum() == (-amount).signum();
            let base = if smooth && same_direction {
                self.motion.target()
            } else {
                self.scroll
            };
            let target = base.saturating_sub(amount).clamp(0, self.scroll_limit());
            let now = self.motion_now();
            self.motion.request(self.scroll, target, now, smooth);
            if self.motion.active() {
                if !self.timer_active {
                    self.timer_active = SetTimer(self.hwnd, SCROLL_TIMER, 10, None) != 0;
                }
                if !self.timer_active {
                    self.scroll = target;
                    self.motion.stop(target);
                    self.place_scene(true);
                }
            } else {
                self.stop_motion();
                self.scroll = target;
                if delta < 0 && self.scroll == self.scroll_limit() {
                    self.follow_intent = true;
                }
                self.place_scene(true);
            }
        }
    }
    fn font_height(&self, w: &Widget) -> i32 {
        unsafe {
            let dc = GetDC(w.hwnd);
            let old = SelectObject(dc, w.font);
            let mut m: TEXTMETRICW = std::mem::zeroed();
            GetTextMetricsW(dc, &mut m);
            SelectObject(dc, old);
            ReleaseDC(w.hwnd, dc);
            (m.tmHeight + m.tmExternalLeading).max(1)
        }
    }
    fn chat_height(&self, w: &Widget, width: i32) -> i32 {
        unsafe {
            if let Some((measured_width, height)) = w.chat_height
                && measured_width == width
            {
                return height;
            }
            let padding = self.scale(if w.spec.role == Role::Outgoing { 12 } else { 2 });
            let dc = GetDC(w.hwnd);
            let old = SelectObject(dc, w.font);
            let value = wide(w.spec.value.as_str().unwrap_or(""));
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: (width - padding * 2).max(1),
                bottom: 0,
            };
            DrawTextW(
                dc,
                value.as_ptr(),
                value.len().saturating_sub(1) as i32,
                &mut rect,
                DT_CALCRECT | DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX,
            );
            SelectObject(dc, old);
            ReleaseDC(w.hwnd, dc);
            (rect.bottom.max(self.font_height(w)) + padding * 2 + self.scale(4))
                .min(self.scale(12000).min(20000))
        }
    }
    fn column(
        &self,
        ids: &[String],
        left: i32,
        top: i32,
        width: i32,
        gap: i32,
        modern: bool,
    ) -> (Vec<Placement>, i32) {
        let mut out = Vec::new();
        let mut y = top;
        let mut first = 0;
        while first < ids.len() {
            let section = if modern {
                section_key(&self.widgets[&ids[first]].spec)
            } else {
                None
            };
            let inset = if section.is_some() {
                self.scale(16).min((width - 1).max(0) / 2)
            } else {
                0
            };
            if section.is_some()
                && (first == 0 || section_key(&self.widgets[&ids[first - 1]].spec) != section)
            {
                y += inset + if first == 0 { 0 } else { gap };
            }
            let left = left + inset;
            let width = (width - inset * 2).max(1);
            let mut group_end = first + 1;
            if let Some(row) = self.widgets[&ids[first]].spec.row.as_ref() {
                while group_end < ids.len()
                    && self.widgets[&ids[group_end]].spec.row.as_ref() == Some(row)
                    && (!modern
                        || section_key(&self.widgets[&ids[group_end]].spec)
                            == section_key(&self.widgets[&ids[first]].spec))
                {
                    group_end += 1;
                }
            }
            let gap = if modern
                && ids[first..group_end]
                    .iter()
                    .all(|id| self.widgets[id].spec.role == Role::Tab)
            {
                self.scale(4)
            } else {
                gap
            };
            if group_end == first + 1
                && self.widgets[&ids[first]].spec.row.is_none()
                && modern
                && self.layout_mode != Some(Layout::Welcome)
                && self.background.rail_width.get() == 0
            {
                let compact_nav = |id: &String| {
                    let c = &self.widgets[id].spec;
                    c.side
                        && c.kind == Kind::Button
                        && matches!(c.role, Role::Nav | Role::Selected)
                        && c.row.is_none()
                };
                if compact_nav(&ids[first]) {
                    while group_end < ids.len() && compact_nav(&ids[group_end]) {
                        group_end += 1;
                    }
                }
            }
            let mut begin = first;
            while begin < group_end {
                let mut widths = Vec::new();
                let mut end = begin;
                let mut used = 0;
                while end < group_end {
                    let w = &self.widgets[&ids[end]];
                    let c = &w.spec;
                    let compact = modern
                        && c.kind == Kind::Button
                        && !matches!(c.role, Role::Nav | Role::Selected)
                        && !(c.role == Role::Primary && group_end - first == 1);
                    let explicit = c.width.map(|n| self.scale(n as i32));
                    let fixed = explicit
                        .or_else(|| {
                            (modern && c.role == Role::Tab).then(|| {
                                (self.text_width(w) + self.scale(28))
                                    .clamp(self.scale(64), self.scale(220))
                            })
                        })
                        .or_else(|| {
                            (c.kind == Kind::Detail && c.role == Role::Outgoing)
                                .then(|| self.outgoing_width(w, width))
                        })
                        .or_else(|| {
                            compact.then(|| {
                                (self.text_width(w)
                                    + self.scale(
                                        (if c.expanded.is_some() { 54 } else { 34 })
                                            + if c.icon.is_some() { 24 } else { 0 },
                                    ))
                                .clamp(self.scale(84), self.scale(360))
                            })
                        });
                    let minimum = fixed
                        .unwrap_or_else(|| {
                            self.scale(if c.kind == Kind::Button && !(modern && c.side) {
                                92
                            } else {
                                144
                            })
                        })
                        .min(width)
                        .max(1);
                    let needed = minimum + if end > begin { gap } else { 0 };
                    if end > begin && used + needed > width {
                        break;
                    }
                    used += needed;
                    widths.push((minimum, fixed.is_none()));
                    end += 1;
                }
                let flex = widths.iter().filter(|(_, f)| *f).count() as i32;
                let remainder = (width - used).max(0);
                let each = if flex > 0 { remainder / flex } else { 0 };
                let split_actions = flex == 0
                    && end - begin > 1
                    && begin == first
                    && end == group_end
                    && self.widgets[&ids[end - 1]].spec.role == Role::Primary
                    && ids[begin..end]
                        .iter()
                        .all(|id| self.widgets[id].spec.area == Some(Area::Composer));
                let align =
                    if end - begin == 1 && self.widgets[&ids[begin]].spec.role == Role::Outgoing {
                        Align::End
                    } else {
                        self.widgets[&ids[first]].spec.row_align
                    };
                let mut x = left
                    + if flex > 0 || split_actions {
                        0
                    } else {
                        match align {
                            Align::Start => 0,
                            Align::Center => remainder / 2,
                            Align::End => remainder,
                        }
                    };
                let mut row_height = 0;
                let mut row_gap = gap;
                for (index, (minimum, grow)) in widths.into_iter().enumerate() {
                    let id = &ids[begin + index];
                    let c = &self.widgets[id].spec;
                    let cw = minimum + if grow { each } else { 0 };
                    if split_actions && begin + index == end - 1 {
                        x = left + width - cw;
                    }
                    let line = (self.scale(if modern && c.role == Role::Body {
                        25
                    } else {
                        22
                    }) as f64
                        * c.zoom)
                        .round() as i32;
                    let height = match c.kind {
                        Kind::Detail if matches!(c.role, Role::Incoming | Role::Outgoing) => {
                            self.chat_height(&self.widgets[id], cw)
                        }
                        Kind::Multiline if modern && c.area == Some(Area::Composer) => {
                            line * c.lines.unwrap_or(2) as i32 + self.scale(12)
                        }
                        Kind::Multiline | Kind::Detail => {
                            line * c.lines.unwrap_or(5) as i32
                                + if modern && c.role == Role::Body {
                                    0
                                } else {
                                    self.scale(10)
                                }
                        }
                        Kind::Multi | Kind::List => {
                            line * c.lines.unwrap_or(6) as i32 + self.scale(8)
                        }
                        Kind::Heading | Kind::Label => {
                            let base = if modern && c.area == Some(Area::Header) {
                                28
                            } else if modern {
                                match c.role {
                                    Role::Title => 42,
                                    Role::Brand => 42,
                                    Role::Caption => 18,
                                    Role::Muted => 23,
                                    _ => {
                                        if c.kind == Kind::Heading {
                                            28
                                        } else {
                                            22
                                        }
                                    }
                                }
                            } else if c.kind == Kind::Heading {
                                36
                            } else {
                                30
                            };
                            let lines = if c.role == Role::Brand || c.area == Some(Area::Header) {
                                1
                            } else {
                                ((self.text_width(&self.widgets[id]) + cw - 1) / cw.max(1))
                                    .clamp(1, 4)
                            };
                            self.scale(base) * lines
                        }
                        Kind::Button => self.scale(if modern {
                            if c.role == Role::Tab {
                                34
                            } else if c.area == Some(Area::Composer) || c.side {
                                36
                            } else if c.role == Role::Primary {
                                40
                            } else if c.role == Role::Quiet {
                                32
                            } else {
                                36
                            }
                        } else {
                            30
                        }),
                        _ => {
                            if modern {
                                self.scale(36)
                            } else {
                                line + self.scale(8)
                            }
                        }
                    };
                    let caption = if self.widgets[id].caption != 0 && c.caption {
                        self.scale(22)
                    } else {
                        0
                    };
                    row_height = row_height.max(height + caption);
                    if modern {
                        row_gap = row_gap.max(self.scale(match c.role {
                            Role::Brand => 28,
                            Role::Title => 18,
                            Role::Caption => 4,
                            _ if c.area == Some(Area::Composer) || c.side => 6,
                            _ => 10,
                        }));
                    }
                    out.push((id.clone(), x, y, cw, height, caption));
                    x += cw + gap;
                }
                y += row_height + row_gap;
                begin = end;
            }
            if section.is_some()
                && (group_end == ids.len()
                    || section_key(&self.widgets[&ids[group_end]].spec) != section)
            {
                y += inset;
            }
            first = group_end;
        }
        (out, y)
    }
    fn native_tab_order(&mut self) {
        unsafe {
            if !self.tab_dirty {
                return;
            }
            let mut after: HashMap<HWND, HWND> = HashMap::new();
            let mut viewport_inserted = false;
            let mut tab_group: Option<(HWND, Option<&str>)> = None;
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOREDRAW | SWP_NOCOPYBITS;
            for id in &self.order {
                let w = &self.widgets[id];
                let parent = GetParent(w.hwnd);
                if w.spec.kind == Kind::Button {
                    let group =
                        (w.spec.role == Role::Tab).then_some((parent, w.spec.row.as_deref()));
                    let first = group.is_some() && group != tab_group;
                    let old = GetWindowLongW(w.hwnd, GWL_STYLE) as u32;
                    let style = (old & !WS_GROUP) | if first { WS_GROUP } else { 0 };
                    if old != style {
                        SetWindowLongW(w.hwnd, GWL_STYLE, style as i32);
                    }
                    tab_group = group;
                } else {
                    tab_group = None;
                }
                if parent == self.content && !viewport_inserted {
                    let previous = *after.get(&self.hwnd).unwrap_or(&HWND_TOP);
                    SetWindowPos(self.viewport, previous, 0, 0, 0, 0, flags);
                    after.insert(self.hwnd, self.viewport);
                    viewport_inserted = true;
                }
                for child in [w.caption, w.hwnd] {
                    if child != 0 {
                        let previous = *after.get(&parent).unwrap_or(&HWND_TOP);
                        SetWindowPos(child, previous, 0, 0, 0, 0, flags);
                        after.insert(parent, child);
                    }
                }
            }
            self.tab_dirty = false;
        }
    }
    fn place_scene(&mut self, scroll_only: bool) {
        unsafe {
            let _guard = UpdateGuard::enter();
            let next_origin = self.scroll.div_euclid(ORIGIN_STEP) * ORIGIN_STEP;
            let rebase = next_origin != self.origin;
            self.origin = next_origin;
            let (left, top, width, height) = self.view_rect;
            let residual = self.scroll - self.origin;
            let canvas_height = (self.content_height - self.origin)
                .max(residual + height)
                .clamp(1, CANVAS_SPAN);
            let mut moves = Vec::new();
            let mut dirty = self.scene_dirty || rebase;
            self.scene_dirty = false;
            let viewport_box = (left, top, width, height);
            if self.viewport_box != Some(viewport_box) {
                let no_size = self
                    .viewport_box
                    .is_some_and(|(_, _, w, h)| w == width && h == height);
                moves.push((self.viewport, left, top, width, height, no_size));
                self.viewport_box = Some(viewport_box);
            }
            let content_box = (0, -residual, width, canvas_height);
            if self.content_box != Some(content_box) {
                let no_size = self
                    .content_box
                    .is_some_and(|(_, _, w, h)| w == width && h == canvas_height);
                moves.push((self.content, 0, -residual, width, canvas_height, no_size));
                self.content_box = Some(content_box);
            }
            let panel = self
                .content_panel
                .map(|(l, t, r, b)| (l - left, t - self.origin, r - left, b - self.origin));
            dirty |= self.content_surface.panel_rect.replace(panel) != panel;
            dirty |= self.content_surface.rail_width.replace(self.scroll_rail) != self.scroll_rail;
            dirty |= self.viewport_surface.rail_width.replace(self.scroll_rail) != self.scroll_rail;
            let transform = |rects: &[(i32, i32, i32, i32)]| {
                rects
                    .iter()
                    .map(|&(l, t, r, b)| (l - left, t - self.origin, r - left, b - self.origin))
                    .collect::<Vec<_>>()
            };
            let sections = transform(&self.section_rects);
            if *self.content_surface.sections.borrow() != sections {
                self.content_surface.sections.replace(sections);
                dirty = true;
            }
            let tabs = transform(&self.tab_rects);
            if *self.content_surface.tabs.borrow() != tabs {
                self.content_surface.tabs.replace(tabs);
                dirty = true;
            }
            for w in self.widgets.values_mut() {
                let (x, body_y, ww, body_height) = w.rect;
                let caption_height = w.placed.map(|p| p.4).unwrap_or(0);
                let caption_y = body_y - caption_height;
                let native_height = if w.spec.kind == Kind::Select {
                    body_height + 220 * self.dpi as i32 / 96
                } else {
                    body_height
                };
                let offset = if w.fixed { 0 } else { self.scroll };
                w.placed = Some((
                    x,
                    caption_y - offset + if w.fixed { 0 } else { top },
                    ww,
                    native_height,
                    caption_height,
                ));
                if scroll_only && !rebase {
                    continue;
                }
                let parent = if w.fixed { self.hwnd } else { self.content };
                if GetParent(w.hwnd) != parent {
                    let focused = GetFocus() == w.hwnd;
                    let text_control =
                        matches!(w.spec.kind, Kind::Input | Kind::Multiline | Kind::Detail);
                    let mut start = 0u32;
                    let mut end = 0u32;
                    if text_control {
                        SendMessageW(
                            w.hwnd,
                            EM_GETSEL,
                            &mut start as *mut _ as usize,
                            &mut end as *mut _ as isize,
                        );
                    }
                    SetParent(w.hwnd, parent);
                    if w.caption != 0 {
                        SetParent(w.caption, parent);
                    }
                    w.native_placed = None;
                    self.tab_dirty = true;
                    if text_control {
                        SendMessageW(w.hwnd, EM_SETSEL, start as usize, end as isize);
                    }
                    if focused {
                        SetFocus(w.hwnd);
                    }
                }
                let outside = !w.fixed
                    && (body_y + body_height < self.origin
                        || caption_y > self.origin + CANVAS_SPAN);
                let px = if w.fixed { x } else { x - left };
                let py = if w.fixed {
                    caption_y
                } else if outside {
                    -30000
                } else {
                    caption_y - self.origin
                };
                let next = (parent, px, py, ww, native_height, caption_height);
                if w.native_placed != Some(next) {
                    let no_size = w.native_placed.is_some_and(|(_, _, _, width, height, _)| {
                        width == ww && height == native_height
                    });
                    if w.caption != 0 {
                        moves.push((w.caption, px, py, ww, caption_height, no_size));
                    }
                    moves.push((w.hwnd, px, py + caption_height, ww, native_height, no_size));
                    w.native_placed = Some(next);
                }
                let radius = if w.spec.role == Role::Outgoing && self.background.active.get() {
                    (16 * self.dpi as i32 / 96).max(1)
                } else {
                    0
                };
                let region = if outside {
                    RECT {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    }
                } else {
                    RECT {
                        left: 0,
                        top: 0,
                        right: ww,
                        bottom: native_height,
                    }
                };
                dirty |= clip_window(
                    w.hwnd,
                    (0, 0, ww, native_height),
                    &region,
                    radius,
                    &mut w.clip,
                );
                if w.caption != 0 {
                    let region = if outside {
                        RECT {
                            left: 0,
                            top: 0,
                            right: 0,
                            bottom: 0,
                        }
                    } else {
                        RECT {
                            left: 0,
                            top: 0,
                            right: ww,
                            bottom: caption_height,
                        }
                    };
                    dirty |= clip_window(
                        w.caption,
                        (0, 0, ww, caption_height),
                        &region,
                        0,
                        &mut w.caption_clip,
                    );
                }
            }
            dirty |= !moves.is_empty();
            move_windows(&moves);
            self.native_tab_order();
            SetScrollPos(self.hwnd, SB_VERT, self.scroll, 1);
            if dirty {
                RedrawWindow(
                    self.viewport,
                    null(),
                    0,
                    RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_NOERASE,
                );
            }
        }
    }
    fn layout(&mut self) {
        let old_scroll = self.scroll;
        self.layout_pass(0);
        if self.motion.active() {
            self.motion
                .shift(self.scroll - old_scroll, self.scroll_limit());
        }
        if self.timer_active && !self.motion.active() {
            unsafe {
                KillTimer(self.hwnd, SCROLL_TIMER);
            }
            self.timer_active = false;
        }
        self.follow_on_layout = false;
        self.anchor = None;
    }
    fn layout_pass(&mut self, pass: u8) {
        #[cfg(test)]
        trace_slot(0);
        unsafe {
            let _guard = UpdateGuard::enter();
            let mut client: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut client);
            let modern = self.theme == Theme::Dark;
            let welcome = self.layout_mode == Some(Layout::Welcome);
            let conversation = self.layout_mode == Some(Layout::Conversation);
            let pad = self
                .scale(if welcome {
                    32
                } else if modern {
                    24
                } else {
                    16
                })
                .min((client.right / 6).max(self.scale(8)));
            let gap = self.scale(if modern { 10 } else { 8 });
            let rail = !welcome
                && self.order.iter().any(|id| self.widgets[id].spec.side)
                && client.right >= self.scale(if modern { 760 } else { 600 });
            let rail_width = if rail {
                if modern {
                    self.scale(224)
                } else {
                    self.scale(250).min((client.right - pad * 2) / 3)
                }
            } else {
                0
            };
            let painted_rail = if modern { rail_width } else { 0 };
            let old_rail = self.background.rail_width.replace(painted_rail);
            let content_left = rail_width + pad + if rail && !modern { gap } else { 0 };
            let available = (client.right - content_left - pad).max(1);
            let cap = if modern || welcome || conversation {
                self.scale(self.max_width.unwrap_or(if welcome {
                    560
                } else if conversation {
                    744
                } else {
                    960
                }) as i32)
            } else {
                available
            };
            let width = available.min(cap).max(1);
            let left = content_left + (available - width) / 2;
            let header_ids: Vec<_> = self
                .order
                .iter()
                .filter(|id| {
                    !welcome
                        && self.widgets[*id].spec.area == Some(Area::Header)
                        && !self.widgets[*id].spec.side
                })
                .cloned()
                .collect();
            let header_height = if !header_ids.is_empty() && client.bottom >= self.scale(180) {
                self.scale(52)
            } else {
                0
            };
            let (mut header, _) =
                self.column(&header_ids, left, self.scale(12), width, gap, modern);
            let composer_ids: Vec<_> = self
                .order
                .iter()
                .filter(|id| {
                    conversation
                        && self.widgets[*id].spec.area == Some(Area::Composer)
                        && !self.widgets[*id].spec.side
                })
                .cloned()
                .collect();
            let (mut footer, footer_bottom) = self.column(
                &composer_ids,
                left,
                if modern { self.scale(18) } else { pad },
                width,
                if modern { self.scale(6) } else { gap },
                modern,
            );
            let footer_height = if composer_ids.is_empty() {
                0
            } else {
                footer_bottom + if modern { self.scale(12) } else { pad }
            };
            self.composer_fixed = conversation
                && footer_height > 0
                && footer_height <= client.bottom * 3 / 5
                && client.bottom - footer_height - header_height >= self.scale(140);
            let footer_top = if self.composer_fixed {
                client.bottom - footer_height
            } else {
                client.bottom
            };
            if self.composer_fixed {
                for (_, _, y, _, _, _) in &mut footer {
                    *y += footer_top;
                }
            }
            let main_ids: Vec<_> = self
                .order
                .iter()
                .filter(|id| {
                    (!rail || !self.widgets[*id].spec.side)
                        && (header_height == 0 || self.widgets[*id].spec.area != Some(Area::Header))
                        && (!self.composer_fixed
                            || self.widgets[*id].spec.area != Some(Area::Composer))
                })
                .cloned()
                .collect();
            let (mut placements, mut bottom) =
                self.column(&main_ids, left, pad, width, gap, modern);
            if welcome {
                bottom = placements
                    .iter()
                    .map(|(_, _, y, _, h, c)| y + h + c)
                    .max()
                    .unwrap_or(pad);
                let offset = ((client.bottom - bottom - pad) / 2).max(0);
                for (_, _, y, _, _, _) in &mut placements {
                    *y += offset;
                }
                bottom += offset;
            }
            if rail {
                let ids: Vec<_> = self
                    .order
                    .iter()
                    .filter(|id| self.widgets[*id].spec.side)
                    .cloned()
                    .collect();
                let railpad = if modern { self.scale(20) } else { pad };
                let (mut items, rail_bottom) = self.column(
                    &ids,
                    railpad,
                    if modern { self.scale(28) } else { pad },
                    if modern {
                        (rail_width - railpad * 2).max(1)
                    } else {
                        rail_width
                    },
                    self.scale(6),
                    modern,
                );
                placements.append(&mut items);
                self.fixed_rail = modern && rail_bottom + pad <= client.bottom;
                if !self.fixed_rail {
                    bottom = bottom.max(rail_bottom);
                }
            } else {
                self.fixed_rail = false;
            }
            if self.composer_fixed {
                placements.append(&mut footer);
            }
            if header_height > 0 {
                placements.append(&mut header);
            }
            self.section_rects.clear();
            self.tab_rects.clear();
            let mut active_section: Option<&str> = None;
            let mut active_tabs: Option<(&str, i32)> = None;
            if modern {
                for (id, x, y, cw, height, caption) in &placements {
                    let c = &self.widgets[id].spec;
                    let section = section_key(c);
                    if let Some(key) = section {
                        let inset = self.scale(16).min((width - 1).max(0) / 2);
                        let bottom = y + height + caption + inset;
                        if active_section == Some(key) {
                            if let Some(rect) = self.section_rects.last_mut() {
                                rect.3 = rect.3.max(bottom);
                            }
                        } else {
                            self.section_rects
                                .push((left, y - inset, left + width, bottom));
                        }
                    }
                    active_section = section;
                    let tabs = (c.role == Role::Tab && !c.side && c.area != Some(Area::Composer))
                        .then(|| c.row.as_deref().map(|key| (key, *y)))
                        .flatten();
                    if let Some(key) = tabs {
                        let right = x + cw + self.scale(3);
                        if active_tabs == Some(key) {
                            if let Some(rect) = self.tab_rects.last_mut() {
                                rect.2 = rect.2.max(right);
                            }
                        } else {
                            self.tab_rects.push((
                                x - self.scale(3),
                                y - self.scale(3),
                                x + cw + self.scale(3),
                                y + height + self.scale(3),
                            ));
                        }
                    }
                    active_tabs = tabs;
                }
            }
            self.content_height = bottom + pad;
            self.viewport_height = (footer_top - header_height).max(1);
            if self.follow_on_layout {
                self.scroll = self.scroll_limit();
            } else if let Some((id, offset)) = &self.anchor
                && let Some((_, _, y, _, _, caption)) =
                    placements.iter().find(|(key, ..)| key == id)
            {
                self.scroll = *y + *caption + *offset;
            }
            self.scroll = self.scroll.clamp(0, self.scroll_limit());
            let welcome_panel = modern && welcome && client.right >= self.scale(720);
            let panel_rect = if welcome_panel {
                let top = placements
                    .iter()
                    .map(|(_, _, y, _, _, _)| *y)
                    .min()
                    .unwrap_or(pad)
                    - pad;
                Some((left - pad, top, left + width + pad, bottom + pad))
            } else {
                None
            };
            let old_panel = self.content_panel;
            self.content_panel = panel_rect;
            self.background.panel_rect.set(None);
            let footer_rect = self.composer_fixed.then_some((
                rail_width,
                footer_top,
                client.right,
                client.bottom,
            ));
            let old_footer = self.background.footer_rect.replace(footer_rect);
            let card = if self.composer_fixed {
                Some((
                    left - self.scale(16),
                    footer_top + self.scale(6),
                    left + width + self.scale(16),
                    client.bottom - self.scale(6),
                ))
            } else {
                None
            };
            let old_card = self.background.footer_card.replace(card);
            let header_rect =
                (header_height > 0).then_some((rail_width, 0, client.right, header_height));
            let old_header = self.background.header_rect.replace(header_rect);
            let composer_focus = self.composer_fixed
                && self
                    .widgets
                    .values()
                    .any(|w| w.hwnd == GetFocus() && w.spec.area == Some(Area::Composer));
            let old_focus = self.background.composer_focus.replace(composer_focus);
            self.view_rect = (
                if self.fixed_rail { rail_width } else { 0 },
                header_height,
                (client.right - if self.fixed_rail { rail_width } else { 0 }).max(1),
                self.viewport_height,
            );
            self.scroll_rail = if self.fixed_rail { 0 } else { painted_rail };
            let mut dirty = old_rail != painted_rail
                || old_footer != footer_rect
                || old_card != card
                || old_header != header_rect
                || old_focus != composer_focus;
            for (id, x, y, width, height, caption_height) in placements {
                let w = self.widgets.get_mut(&id).unwrap();
                let sticky_composer =
                    self.composer_fixed && w.spec.area == Some(Area::Composer) && !w.spec.side;
                let fixed = self.fixed_rail && w.spec.side
                    || sticky_composer
                    || header_height > 0 && w.spec.area == Some(Area::Header);
                if fixed != w.fixed {
                    dirty = true;
                }
                w.fixed = fixed;
                let panel = welcome_panel
                    || sticky_composer
                    || modern
                        && !w.spec.side
                        && w.spec.area != Some(Area::Header)
                        && w.spec.section.as_ref().is_some_and(|s| !s.is_empty());
                w.surface.set_composer(sticky_composer);
                let side_surface = modern && rail && w.spec.side;
                let side_changed = w.surface.side.replace(side_surface) != side_surface;
                let panel_changed = w.surface.panel.replace(panel) != panel;
                if side_changed || panel_changed {
                    InvalidateRect(w.hwnd, null(), 0);
                }
                if let Some(caption) = &w.caption_surface {
                    let changed = caption.side.replace(side_surface) != side_surface;
                    let changed = caption.panel.replace(panel) != panel || changed;
                    if changed {
                        InvalidateRect(w.caption, null(), 0);
                    }
                }
                let native_height = if w.spec.kind == Kind::Select {
                    height + 220 * self.dpi as i32 / 96
                } else {
                    height
                };
                let next = (
                    x,
                    y - if fixed { 0 } else { self.scroll },
                    width,
                    native_height,
                    caption_height,
                );
                if fixed && w.placed != Some(next) {
                    dirty = true;
                }
                w.rect = (x, y + caption_height, width, height);
                w.placed = Some(next);
            }
            self.place_scene(false);
            let mut reflow = false;
            let measured: Vec<_> = self
                .order
                .iter()
                .filter_map(|id| {
                    let w = &self.widgets[id];
                    if w.spec.kind != Kind::Detail
                        || !matches!(w.spec.role, Role::Incoming | Role::Outgoing)
                    {
                        return None;
                    }
                    let padding = self.scale(if w.spec.role == Role::Outgoing { 12 } else { 2 });
                    let raw = (SendMessageW(w.hwnd, EM_GETLINECOUNT, 0, 0).max(1) as i32)
                        .saturating_mul(self.font_height(w))
                        .saturating_add(padding * 2 + self.scale(4));
                    Some((
                        id.clone(),
                        w.rect.2,
                        raw.min(self.scale(12000).min(20000)),
                        raw > self.scale(12000).min(20000),
                    ))
                })
                .collect();
            for (id, width, height, overflow) in measured {
                let w = self.widgets.get_mut(&id).unwrap();
                w.surface.set_overflow(overflow);
                if w.rect.3 != height {
                    reflow = true;
                }
                w.chat_height = Some((width, height));
            }
            let info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                nMin: 0,
                nMax: self.content_height - 1,
                nPage: self.viewport_height as u32,
                nPos: self.scroll,
                nTrackPos: 0,
            };
            SetScrollInfo(self.hwnd, SB_VERT, &info, 1);
            let mut after: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut after);
            if pass < 3 && (reflow || after.right != client.right || after.bottom != client.bottom)
            {
                self.layout_pass(pass + 1);
            }
            if dirty {
                #[cfg(test)]
                trace_slot(3);
                RedrawWindow(
                    self.hwnd,
                    null(),
                    0,
                    RDW_INVALIDATE
                        | if self.background.active.get() {
                            RDW_NOERASE
                        } else {
                            RDW_ERASE
                        },
                );
            }
            if old_panel != panel_rect {
                RedrawWindow(
                    self.viewport,
                    null(),
                    0,
                    RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_NOERASE,
                );
            }
        }
    }
    fn reveal(&mut self, hwnd: HWND) {
        unsafe {
            let Some(w) = self.widgets.values().find(|w| w.hwnd == hwnd) else {
                return;
            };
            if self.fixed_rail && w.spec.side
                || self.composer_fixed && w.spec.area == Some(Area::Composer)
                || self.view_rect.1 > 0 && w.spec.area == Some(Area::Header)
            {
                return;
            }
            let mut r = std::mem::zeroed::<RECT>();
            GetClientRect(self.hwnd, &mut r);
            let (_, y, _, h) = w.rect;
            if y < self.scroll {
                self.scroll = y.saturating_sub(self.scale(16));
            } else if y + h > self.scroll + self.viewport_height
                && (h <= self.viewport_height || y >= self.scroll + self.viewport_height)
            {
                self.scroll = if h > self.viewport_height {
                    y - self.scale(16)
                } else {
                    y + h - self.viewport_height + self.scale(16)
                }
                .max(0);
            }
            let position = self.scroll;
            self.scroll_direct(position);
        }
    }
    fn command(&mut self, hwnd: HWND, code: u32) {
        unsafe {
            if self.widgets.values().any(|w| {
                w.hwnd == hwnd
                    && match w.spec.kind {
                        Kind::Input | Kind::Multiline | Kind::Detail => code == EN_SETFOCUS,
                        Kind::Select => code == CBN_SETFOCUS,
                        Kind::List | Kind::Multi => code == LBN_SETFOCUS,
                        _ => false,
                    }
            }) {
                self.stop_motion();
                return;
            }
            let Some((id, w)) = self.widgets.iter_mut().find(|(_, w)| w.hwnd == hwnd) else {
                return;
            };
            if IsWindowEnabled(hwnd) == 0 {
                return;
            }
            let mut event = match w.spec.kind {
                Kind::Button if code == BN_CLICKED => json!({"type":"click","id":id}),
                Kind::Check if code == BN_CLICKED => {
                    json!({"type":"change","id":id,"value":SendMessageW(hwnd,BM_GETCHECK,0,0)==BST_CHECKED as isize})
                }
                Kind::Input | Kind::Multiline if code == EN_CHANGE => {
                    w.surface.composer_overflow();
                    let mut start = 0u32;
                    let mut end = 0u32;
                    SendMessageW(
                        hwnd,
                        EM_GETSEL,
                        &mut start as *mut _ as usize,
                        &mut end as *mut _ as isize,
                    );
                    json!({"type":"change","id":id,"value":text(hwnd),"start":start,"end":end})
                }
                Kind::Select | Kind::List if code == CBN_SELCHANGE || code == LBN_SELCHANGE => {
                    let n = SendMessageW(
                        hwnd,
                        if w.spec.kind == Kind::Select {
                            CB_GETCURSEL
                        } else {
                            LB_GETCURSEL
                        },
                        0,
                        0,
                    );
                    json!({"type":"change","id":id,"value":w.spec.choices.get(n as usize).map(|c|c.value.clone()).unwrap_or_default()})
                }
                Kind::Multi if code == LBN_SELCHANGE => {
                    let values: Vec<_> = w
                        .spec
                        .choices
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| SendMessageW(hwnd, LB_GETSEL, *i, 0) > 0)
                        .map(|(_, c)| c.value.clone())
                        .collect();
                    json!({"type":"change","id":id,"value":values})
                }
                _ => return,
            };
            self.serial += 1;
            event["serial"] = json!(self.serial);
            if event["type"] == "change" {
                w.serial = self.serial;
                w.spec.value = event["value"].clone();
            }
            let _ = self.outgoing.send(event);
            // An explicit edit/activation takes over from kinetic movement.
            self.stop_motion();
        }
    }
    fn pending_keyboard_activation(&mut self, button: HWND, focus: HWND) {
        unsafe {
            let Some(default_id) = self.default.as_ref() else {
                return;
            };
            if !self.widgets.get(default_id).is_some_and(|w| {
                w.hwnd == button && w.spec.kind == Kind::Button && IsWindowEnabled(button) == 0
            }) {
                return;
            }
            let Some(composer_id) = self.composer.as_ref() else {
                return;
            };
            let Some(w) = self.widgets.get(composer_id) else {
                return;
            };
            if w.hwnd != focus
                || w.spec.kind != Kind::Multiline
                || IsWindowEnabled(focus) == 0
                || w.serial <= self.acknowledged
                || w.serial <= self.last_keyboard_intent
                || text(focus).trim().is_empty()
                || composing(w)
            {
                return;
            }
            let edit = w.serial;
            self.last_keyboard_intent = edit;
            self.serial += 1;
            let _ = self.outgoing.send(
                json!({"type":"click","id":default_id,"serial":self.serial,"pending_edit":edit}),
            );
            self.stop_motion();
        }
    }
    fn touch_effect(&mut self, effect: touch::Effect) -> bool {
        if !effect.consumed {
            return false;
        }
        unsafe {
            if effect.begin {
                self.stop_motion();
                self.wheel_remainder = 0;
                self.follow_intent = false;
            }
            if effect.delta != 0 {
                self.scroll_direct(self.scroll.saturating_add(effect.delta));
            }
            if effect.end {
                self.follow_intent = self.scroll == self.scroll_limit();
            }
            if effect.release && GetCapture() == self.hwnd {
                ReleaseCapture();
            }
            if effect.capture {
                SetCapture(self.hwnd);
            }
        }
        true
    }
    fn cancel_touch(&mut self) {
        let _guard = UpdateGuard::enter();
        let effect = self.touch.cancel();
        self.touch_effect(effect);
    }
    fn touch_packet(&mut self, packet: &touch::Packet) -> bool {
        let effect = match *packet {
            touch::Packet::Contact(event) => {
                let mut point = POINT {
                    x: event.x,
                    y: event.y,
                };
                unsafe {
                    ScreenToClient(self.hwnd, &mut point);
                }
                let (x, y, w, h) = self.view_rect;
                let eligible = self.layout_mode == Some(Layout::Conversation)
                    && point.x >= x
                    && point.x < x + w
                    && point.y >= y
                    && point.y < y + h;
                self.touch.handle(event, eligible, self.scale(8))
            }
            touch::Packet::MouseCaptureLost if self.touch.mouse_active() => self.touch.cancel(),
            touch::Packet::OwnerDestroyed(hwnd) if self.touch.owner() == Some(hwnd) => {
                self.touch.cancel()
            }
            touch::Packet::Cancel => self.touch.cancel(),
            touch::Packet::ForgetPointer(id) => self.touch.forget(id),
            touch::Packet::UnavailablePointer(id) => self.touch.handle(
                touch::Event {
                    source: touch::Source::Pointer(id),
                    phase: touch::Phase::Cancel,
                    x: 0,
                    y: 0,
                    primary: false,
                    owner: self.hwnd,
                },
                false,
                1,
            ),
            _ => return false,
        };
        self.touch_effect(effect)
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    // Delegation must happen without a State borrow or reentrancy guard:
    // DefWindowProc(WM_WINDOWPOSCHANGED) synchronously dispatches WM_SIZE.
    if !matches!(
        msg,
        WM_CLOSE
            | WM_SIZE
            | WM_COMMAND
            | FRAME
            | touch::MESSAGE
            | WM_VSCROLL
            | WM_MOUSEWHEEL
            | WM_TIMER
            | WM_DPICHANGED
            | WM_SETTINGCHANGE
            | WM_SYSCOLORCHANGE
            | WM_THEMECHANGED
            | WM_DROPFILES
    ) {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    // Native control changes can synchronously notify their parent. Never form
    // a second mutable state reference while processing such a callback.
    if UPDATING.with(Cell::get) {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let _guard = UpdateGuard::enter();
    let s = &mut *ptr;
    match msg {
        touch::MESSAGE => {
            touch::take(hwnd, wp).map_or(0, |packet| isize::from(s.touch_packet(&packet)))
        }
        WM_CLOSE => {
            PostQuitMessage(0);
            0
        }
        WM_SIZE => {
            s.follow_on_layout = s.layout_mode == Some(Layout::Conversation)
                && s.follow_tail
                && s.follow_intent
                && s.scroll == s.scroll_limit();
            if s.layout_mode == Some(Layout::Conversation) && !s.follow_on_layout {
                s.anchor = s.visible_anchor();
            }
            s.layout();
            0
        }
        WM_COMMAND => {
            s.command(lp, ((wp >> 16) & 0xffff) as u32);
            0
        }
        FRAME => {
            let (frame, error) = {
                let mut p = s.pending.lock().unwrap();
                p.posted = false;
                (p.frame.take(), p.error.take())
            };
            if let Some(error) = error {
                let _ = s.outgoing.send(json!({"type":"error","message":error}));
            }
            if let Some(frame) = frame
                && let Err(error) = s.apply(frame)
            {
                let _ = s
                    .outgoing
                    .send(json!({"type":"error","message":error.to_string()}));
            }
            0
        }
        WM_VSCROLL => {
            let mut info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                ..std::mem::zeroed()
            };
            GetScrollInfo(hwnd, SB_VERT, &mut info);
            let position = match (wp & 0xffff) as i32 {
                SB_LINEUP => s.scroll - s.scale(32),
                SB_LINEDOWN => s.scroll + s.scale(32),
                SB_PAGEUP => s.scroll - info.nPage as i32,
                SB_PAGEDOWN => s.scroll + info.nPage as i32,
                SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos,
                _ => s.scroll,
            }
            .clamp(0, (s.content_height - info.nPage as i32).max(0));
            s.scroll_direct(position);
            0
        }
        WM_TIMER if wp == SCROLL_TIMER => {
            if s.timer_active {
                let now = if client_animation_enabled() {
                    s.motion_now()
                } else {
                    u64::MAX
                };
                s.tick_at(now);
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp >> 16) as u16 as i16) as i32;
            s.wheel(delta);
            0
        }
        WM_DPICHANGED => {
            s.follow_on_layout = s.layout_mode == Some(Layout::Conversation)
                && s.follow_tail
                && s.follow_intent
                && s.scroll == s.scroll_limit();
            if s.layout_mode == Some(Layout::Conversation) && !s.follow_on_layout {
                s.anchor = s.visible_anchor();
            }
            s.dpi = (wp & 0xffff) as u32;
            s.refresh_theme(dark::high_contrast(), true);
            let r = &*(lp as *const RECT);
            SetWindowPos(
                hwnd,
                0,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            s.layout();
            0
        }
        WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
            s.refresh_theme(dark::high_contrast(), false);
            s.layout();
            InvalidateRect(hwnd, null(), 1);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_DROPFILES => {
            let drop = wp as isize;
            let n = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
            let mut paths = Vec::new();
            for i in 0..n {
                let len = DragQueryFileW(drop, i, std::ptr::null_mut(), 0);
                let mut buf = vec![0u16; len as usize + 1];
                DragQueryFileW(drop, i, buf.as_mut_ptr(), buf.len() as u32);
                paths.push(String::from_utf16_lossy(&buf[..len as usize]));
            }
            DragFinish(drop);
            let _ = s.outgoing.send(json!({"type":"drop","paths":paths}));
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if matches!(msg, WM_COMMAND | WM_MOUSEWHEEL | WM_DROPFILES) {
        let root = GetAncestor(hwnd, GA_ROOT);
        if root != 0 && root != hwnd {
            if msg == WM_MOUSEWHEEL {
                return forward_wheel(root, wp, lp);
            }
            return SendMessageW(root, msg, wp, lp);
        }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
fn create_host(
    parent: HWND,
    composited: bool,
    palette: std::rc::Rc<dark::Palette>,
) -> Result<(HWND, Box<dark::Surface>)> {
    unsafe {
        let name = wide("FluxR.ClippedViewport.v1");
        let instance = GetModuleHandleW(null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(host_proc),
            hInstance: instance,
            hCursor: LoadCursorW(0, IDC_ARROW),
            lpszClassName: name.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT | WS_EX_ACCEPTFILES | if composited { WS_EX_COMPOSITED } else { 0 },
            name.as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            1,
            1,
            parent,
            0,
            instance,
            null(),
        );
        if hwnd == 0 {
            bail!("native viewport creation failed: {}", GetLastError());
        }
        let surface = dark::Surface::attach(hwnd, None, palette);
        surface.host.set(true);
        Ok((hwnd, surface))
    }
}
fn move_windows(moves: &[NativeMove]) {
    unsafe {
        let mut groups: HashMap<HWND, Vec<NativeMove>> = HashMap::new();
        for item in moves {
            groups.entry(GetParent(item.0)).or_default().push(*item);
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_by_key(|(parent, _)| {
            let mut depth = 0;
            let mut window = *parent;
            while window != 0 {
                depth += 1;
                window = GetParent(window);
            }
            depth
        });
        let flags = |no_size: bool| {
            SWP_NOREDRAW
                | SWP_NOCOPYBITS
                | SWP_NOZORDER
                | SWP_NOACTIVATE
                | if no_size { SWP_NOSIZE } else { 0 }
        };
        for (_, group) in groups {
            let mut batch = BeginDeferWindowPos(group.len() as i32);
            if batch != 0 {
                for (hwnd, x, y, w, h, no_size) in &group {
                    batch = DeferWindowPos(batch, *hwnd, 0, *x, *y, *w, *h, flags(*no_size));
                    #[cfg(test)]
                    trace_slot(2);
                    if batch == 0 {
                        break;
                    }
                }
            }
            if batch == 0 || EndDeferWindowPos(batch) == 0 {
                for (hwnd, x, y, w, h, no_size) in &group {
                    SetWindowPos(*hwnd, 0, *x, *y, *w, *h, flags(*no_size));
                }
            }
        }
    }
}

fn window(
    title: &str,
    outgoing: Sender<Value>,
    pending: Arc<Mutex<Pending>>,
) -> Result<Box<State>> {
    unsafe {
        // Thread-local DPI awareness does not mutate the host process's DPI policy.
        let previous_dpi = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let name = wide("FluxR.NativeControls.v1");
        let instance = GetModuleHandleW(null());
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(proc),
            hInstance: instance,
            hCursor: LoadCursorW(0, IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            lpszClassName: name.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&class);
        let initial_dpi = GetDpiForSystem().max(96) as i32;
        let mut work_area: RECT = std::mem::zeroed();
        let has_work_area =
            SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work_area as *mut _ as *mut _, 0) != 0;
        let mut initial_width = 1100 * initial_dpi / 96;
        let mut initial_height = 800 * initial_dpi / 96;
        if has_work_area {
            initial_width = initial_width.min((work_area.right - work_area.left) * 9 / 10);
            initial_height = initial_height.min((work_area.bottom - work_area.top) * 9 / 10);
        }
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT | WS_EX_ACCEPTFILES,
            name.as_ptr(),
            wide(title).as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VSCROLL | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            initial_width,
            initial_height,
            0,
            0,
            instance,
            null(),
        );
        if hwnd == 0 {
            let error = GetLastError();
            if previous_dpi != 0 {
                SetThreadDpiAwarenessContext(previous_dpi);
            }
            bail!("native window creation failed: {}", error)
        }
        let palette = dark::Palette::new();
        let background = dark::Surface::attach(hwnd, None, palette.clone());
        let hosts = (|| -> Result<_> {
            let (viewport, viewport_surface) = create_host(hwnd, true, palette.clone())?;
            let (content, content_surface) = create_host(viewport, false, palette.clone())?;
            Ok((viewport, viewport_surface, content, content_surface))
        })();
        let (viewport, viewport_surface, content, content_surface) = match hosts {
            Ok(v) => v,
            Err(error) => {
                DestroyWindow(hwnd);
                if previous_dpi != 0 {
                    SetThreadDpiAwarenessContext(previous_dpi);
                }
                return Err(error);
            }
        };
        let mut s = Box::new(State {
            hwnd,
            widgets: HashMap::new(),
            order: Vec::new(),
            outgoing,
            pending,
            serial: 0,
            default: None,
            scroll: 0,
            content_height: 0,
            dpi: GetDpiForWindow(hwnd).max(96),
            previous_dpi,
            palette,
            background,
            theme: Theme::System,
            layout_mode: None,
            max_width: None,
            fixed_rail: false,
            composer_fixed: false,
            composer: None,
            conversation: None,
            follow_tail: true,
            follow_on_layout: false,
            anchor: None,
            viewport_height: 0,
            wheel_remainder: 0,
            viewport,
            content,
            viewport_surface,
            content_surface,
            view_rect: (0, 0, 1, 1),
            origin: 0,
            content_panel: None,
            section_rects: Vec::new(),
            tab_rects: Vec::new(),
            scroll_rail: 0,
            viewport_box: None,
            content_box: None,
            follow_intent: true,
            tab_dirty: true,
            scene_dirty: true,
            motion: Motion::default(),
            motion_clock: std::time::Instant::now(),
            timer_active: false,
            acknowledged: 0,
            last_keyboard_intent: 0,
            touch: touch::Pan::default(),
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut *s as *mut State as isize);
        Ok(s)
    }
}
// Keep the allocation stable until its GWLP_USERDATA pointer is detached.
#[allow(clippy::boxed_local)]
fn destroy(mut s: Box<State>) {
    unsafe {
        s.cancel_touch();
        SetWindowLongPtrW(s.hwnd, GWLP_USERDATA, 0);
        KillTimer(s.hwnd, SCROLL_TIMER);
        s.widgets.clear();
        DestroyWindow(s.hwnd);
        if s.previous_dpi != 0 {
            SetThreadDpiAwarenessContext(s.previous_dpi);
        }
    }
}

pub(super) fn run(
    title: &str,
    initial: Frame,
    outgoing: Sender<Value>,
    incoming: Receiver<Value>,
) -> Result<()> {
    unsafe {
        if initial.close {
            return Ok(());
        }
        let pending = Arc::new(Mutex::new(Pending::default()));
        let mut s = window(title, outgoing, pending.clone())?;
        if let Err(error) = s.apply(initial) {
            destroy(s);
            return Err(error);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let handle = s.hwnd;
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match incoming.recv_timeout(Duration::from_millis(100)) {
                    Ok(value) => {
                        let parsed = parse(value);
                        let mut p = pending.lock().unwrap();
                        match parsed {
                            Ok(frame) => p.push(frame),
                            Err(error) => p.error = Some(error.to_string()),
                        }
                        if !p.posted {
                            p.posted = true;
                            PostMessageW(handle, FRAME, 0, 0);
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => break,
                }
            }
        });
        ShowWindow(s.hwnd, SW_SHOW);
        UpdateWindow(s.hwnd);
        let mut msg = std::mem::zeroed::<MSG>();
        let mut result = Ok(());
        loop {
            let n = GetMessageW(&mut msg, 0, 0, 0);
            if n == 0 {
                break;
            }
            if n < 0 {
                result = Err(anyhow::anyhow!("GetMessageW failed"));
                break;
            }
            if msg.message == WM_KEYDOWN {
                if msg.wParam == VK_RETURN as usize {
                    let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
                    let shift = GetKeyState(VK_SHIFT as i32) < 0;
                    let alt = GetKeyState(VK_MENU as i32) < 0;
                    let repeat = msg.lParam as usize & (1 << 30) != 0;
                    let focus = GetFocus();
                    let focused = s.widgets.iter().find(|(_, w)| w.hwnd == focus);
                    let designated = focused.is_some_and(|(id, w)| {
                        s.composer.as_ref() == Some(id) && w.spec.kind == Kind::Multiline
                    });
                    let ime = focused.is_some_and(|(_, w)| composing(w));
                    let shortcut = enter_sends(ctrl, shift, alt, repeat, designated, ime);
                    let default = if shortcut || repeat && (ctrl || designated && !shift) && !ime {
                        s.default
                            .as_ref()
                            .and_then(|id| s.widgets.get(id))
                            .filter(|w| w.spec.kind == Kind::Button)
                            .map(|w| w.hwnd)
                    } else {
                        None
                    };
                    let button = default.or_else(|| {
                        (!ctrl && !designated)
                            .then(|| {
                                focused
                                    .filter(|(_, w)| w.spec.kind == Kind::Button)
                                    .map(|(_, w)| w.hwnd)
                            })
                            .flatten()
                    });
                    if let Some(button) = button {
                        if !repeat && IsWindowEnabled(button) != 0 {
                            SendMessageW(button, BM_CLICK, 0, 0);
                        } else if shortcut && designated {
                            s.pending_keyboard_activation(button, focus);
                        }
                        continue;
                    }
                }
                if (msg.wParam == VK_TAB as usize
                    || msg.wParam == VK_LEFT as usize
                    || msg.wParam == VK_RIGHT as usize
                    || msg.wParam == VK_UP as usize
                    || msg.wParam == VK_DOWN as usize)
                    && IsDialogMessageW(s.hwnd, &msg) != 0
                {
                    s.reveal(GetFocus());
                    continue;
                }
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        stop.store(true, Ordering::Relaxed);
        let _ = worker.join();
        destroy(s);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_edit_paint_transaction_preserves_text_caret_ime_and_caller_dc_state() {
        unsafe fn select(hwnd: HWND) -> (u32, u32) {
            let (mut a, mut b) = (0u32, 0u32);
            SendMessageW(
                hwnd,
                EM_GETSEL,
                &mut a as *mut _ as usize,
                &mut b as *mut _ as isize,
            );
            (a, b)
        }
        unsafe fn paint(hwnd: HWND, hint: bool) {
            let before = text(hwnd);
            let selection = select(hwnd);
            let mut caret: POINT = std::mem::zeroed();
            let had_caret = GetCaretPos(&mut caret) != 0;
            dark::EDIT_PASSES.with(|p| p.set([0; 3]));
            InvalidateRect(hwnd, null(), 1);
            SendMessageW(hwnd, WM_PAINT, 0, 0);
            assert_eq!(dark::EDIT_PASSES.with(Cell::get), [1, 1, u64::from(hint)]);
            assert_eq!(text(hwnd), before);
            assert_eq!(select(hwnd), selection);
            if had_caret {
                let mut after: POINT = std::mem::zeroed();
                assert_ne!(GetCaretPos(&mut after), 0);
                assert_eq!((after.x, after.y), (caret.x, caret.y));
            }
            let dc = CreateCompatibleDC(0);
            SetBkMode(dc, OPAQUE as i32);
            SetViewportOrgEx(dc, 3, 4, std::ptr::null_mut());
            let font = GetCurrentObject(dc, OBJ_FONT as u32);
            let mut clip: RECT = std::mem::zeroed();
            GetClipBox(dc, &mut clip);
            for msg in [WM_PRINTCLIENT, WM_PRINT] {
                dark::EDIT_PASSES.with(|p| p.set([0; 3]));
                SendMessageW(
                    hwnd,
                    msg,
                    dc as usize,
                    (PRF_CLIENT | PRF_NONCLIENT) as isize,
                );
                assert_eq!(dark::EDIT_PASSES.with(Cell::get), [1, 1, u64::from(hint)]);
                assert_eq!(GetCurrentObject(dc, OBJ_FONT as u32), font);
                assert_eq!(GetBkMode(dc), OPAQUE as i32);
                let mut origin: POINT = std::mem::zeroed();
                GetViewportOrgEx(dc, &mut origin);
                assert_eq!((origin.x, origin.y), (3, 4));
                let mut after: RECT = std::mem::zeroed();
                GetClipBox(dc, &mut after);
                assert_eq!(
                    (after.left, after.top, after.right, after.bottom),
                    (clip.left, clip.top, clip.right, clip.bottom)
                );
                assert_eq!(text(hwnd), before);
                assert_eq!(select(hwnd), selection);
            }
            DeleteDC(dc);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        unsafe {
            let mut s = window(
                "Hidden edit paint contract",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            s.apply(parse(json!({"theme":"dark","layout":"conversation","controls":[
                {"id":"body","kind":"detail","role":"incoming","value":"Answer"},
                {"id":"draft","kind":"multiline","area":"composer","label":"Message","caption":false,"placeholder":"Write a message…","lines":2,"value":""},
                {"id":"input","kind":"input","area":"composer","label":"Search","placeholder":"Find…","value":""}
            ]})).unwrap()).unwrap();
            let draft = s.widgets["draft"].hwnd;
            let input = s.widgets["input"].hwnd;
            SetFocus(draft);
            paint(draft, true);
            assert_eq!(GetFocus(), draft);
            // Native first-character editing and clearing, not a custom text model.
            SendMessageW(draft, WM_CHAR, 'A' as usize, 0);
            assert_eq!(text(draft), "A");
            paint(draft, false);
            let first = rx.try_recv().unwrap();
            assert_eq!(first["value"], "A");
            SendMessageW(draft, EM_SETSEL, 0, -1);
            SendMessageW(draft, WM_CLEAR, 0, 0);
            assert_eq!(text(draft), "");
            paint(draft, true);
            assert_eq!(rx.try_recv().unwrap()["value"], "");
            let serial = s.serial;
            s.widgets["draft"].surface.ime.set(true);
            paint(draft, false);
            s.widgets["draft"].surface.ime.set(false);
            EnableWindow(draft, 0);
            paint(draft, true);
            EnableWindow(draft, 1);
            SetFocus(input);
            paint(input, true);
            paint(draft, true);
            assert_eq!(s.serial, serial);
            SendMessageW(
                input,
                WM_SETTEXT,
                0,
                wide("A Unicode café query 🙂").as_ptr() as isize,
            );
            assert_eq!(rx.try_recv().unwrap()["value"], "A Unicode café query 🙂");
            let serial = s.serial;
            SendMessageW(input, EM_SETSEL, 2, 7);
            paint(input, false);
            let mut p = POINT { x: 12, y: 12 };
            let lp = ((p.x as u16 as u32) | ((p.y as u16 as u32) << 16)) as isize;
            SendMessageW(draft, WM_LBUTTONDOWN, 1, lp);
            SendMessageW(draft, WM_LBUTTONUP, 0, lp);
            paint(draft, true);
            assert_eq!(s.serial, serial);
            assert!(rx.try_recv().is_err());
            s.refresh_theme(true, false);
            dark::EDIT_PASSES.with(|p| p.set([0; 3]));
            SendMessageW(draft, WM_PAINT, 0, 0);
            assert_eq!(dark::EDIT_PASSES.with(Cell::get), [0; 3]);
            assert_eq!(text(draft), "");
            GetCaretPos(&mut p);
            destroy(s);
        }
    }
    #[test]
    fn hidden_focused_composer_paint_message_probe() {
        unsafe extern "system" {
            fn OutputDebugStringW(message: *const u16);
        }
        let (tx, _rx) = std::sync::mpsc::channel();
        unsafe {
            let mut s = window(
                "Composer paint probe",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            s.apply(parse(json!({"theme":"dark","layout":"conversation","composer":"draft","controls":[
                {"id":"body","kind":"detail","role":"incoming","value":"A retained answer."},
                {"id":"draft","kind":"multiline","area":"composer","caption":false,"label":"Message","placeholder":"Write a message…","lines":2,"value":""},
                {"id":"other","kind":"input","area":"composer","label":"Other","value":"Field"}
            ]})).unwrap()).unwrap();
            let draft = s.widgets["draft"].hwnd;
            let paint = |stage: &str| {
                eprintln!(
                    "composer_paint_probe {stage} hwnd={draft:x} text={:?}",
                    text(draft)
                );
                OutputDebugStringW(wide(&format!("PHASE {stage}-paint")).as_ptr());
                InvalidateRect(draft, null(), 1);
                SendMessageW(draft, WM_PAINT, 0, 0);
                let dc = CreateCompatibleDC(0);
                OutputDebugStringW(wide(&format!("PHASE {stage}-printclient")).as_ptr());
                SendMessageW(draft, WM_PRINTCLIENT, dc as usize, PRF_CLIENT as isize);
                DeleteDC(dc);
            };
            SetFocus(draft);
            paint("focused-empty");
            set_text(draft, "Café 🙂");
            SendMessageW(draft, EM_SETSEL, 2, 4);
            paint("focused-nonempty");
            set_text(draft, "");
            paint("focused-cleared");
            s.widgets["draft"].surface.ime.set(true);
            paint("ime-policy");
            s.widgets["draft"].surface.ime.set(false);
            SetFocus(s.widgets["other"].hwnd);
            paint("unfocused-empty");
            s.refresh_theme(true, false);
            paint("high-contrast-policy");
            OutputDebugStringW(wide("PHASE composer-paint-finished").as_ptr());
            destroy(s);
        }
    }
    #[test]
    fn hidden_touch_pan_routes_children_background_and_cancellation_without_edit_selection() {
        use windows_sys::Win32::UI::Input::Pointer::*;
        unsafe fn pointer(
            target: HWND,
            kind: i32,
            id: u16,
            msg: u32,
            point: POINT,
            primary: bool,
            cancelled: bool,
        ) {
            let mut info: POINTER_INFO = std::mem::zeroed();
            info.pointerType = kind;
            info.pointerId = id as u32;
            info.ptPixelLocation = point;
            info.pointerFlags = POINTER_FLAG_INCONTACT
                | if primary { POINTER_FLAG_PRIMARY } else { 0 }
                | if cancelled { POINTER_FLAG_CANCELED } else { 0 };
            touch::POINTER_SAMPLE.with(|sample| sample.set(Some(info)));
            SendMessageW(target, msg, id as usize, 0);
            touch::POINTER_SAMPLE.with(|sample| sample.set(None));
        }
        unsafe fn mouse(target: HWND, msg: u32, point: POINT) {
            let mut client = point;
            ScreenToClient(target, &mut client);
            let lp = ((client.x as u16 as u32) | ((client.y as u16 as u32) << 16)) as isize;
            SendMessageW(
                target,
                msg,
                if msg == WM_LBUTTONUP {
                    0
                } else {
                    windows_sys::Win32::System::SystemServices::MK_LBUTTON as usize
                },
                lp,
            );
        }
        unsafe fn selection(hwnd: HWND) -> (u32, u32) {
            let mut a = 0u32;
            let mut b = 0u32;
            SendMessageW(
                hwnd,
                EM_GETSEL,
                &mut a as *mut _ as usize,
                &mut b as *mut _ as isize,
            );
            (a, b)
        }
        let (tx, rx) = std::sync::mpsc::channel();
        unsafe {
            let mut s = window(
                "Hidden touch contract",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            MoveWindow(s.hwnd, 0, 0, s.scale(1200), s.scale(800), 0);
            let mut frame = json!({"theme":"dark","layout":"conversation","conversation":"touch-page","default":"send","composer":"draft","controls":[
                {"id":"rail","kind":"button","role":"selected","side":true,"label":"Navigation"},
                {"id":"title","kind":"heading","area":"header","role":"title","label":"Conversation"},
                {"id":"answer","kind":"detail","role":"incoming","value":"Selectable café and 🙂 prose remains native. ".repeat(400)},
                {"id":"outgoing","kind":"detail","role":"outgoing","value":"An outgoing answer"},
                {"id":"draft","kind":"multiline","area":"composer","caption":false,"label":"Message","lines":2,"value":"Keep this editable draft"},
                {"id":"send","kind":"button","area":"composer","label":"Send"}
            ]});
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(IsWindowVisible(s.hwnd), 0);
            let body = s.widgets["answer"].hwnd;
            let draft = s.widgets["draft"].hwnd;
            let outgoing = s.widgets["outgoing"].hwnd;
            SendMessageW(body, EM_SETSEL, 11, 19);
            SendMessageW(draft, EM_SETSEL, 2, 6);
            SetFocus(draft);
            s.scroll_direct(0);
            let (left, top, _, _) = s.view_rect;
            let mut start = POINT {
                x: left + s.scale(100),
                y: top + s.scale(160),
            };
            ClientToScreen(s.hwnd, &mut start);
            let moved = POINT {
                x: start.x,
                y: start.y - 100,
            };
            pointer(body, PT_TOUCH, 17, WM_POINTERDOWN, start, true, false);
            assert_eq!(s.touch.owner(), Some(body));
            assert_eq!(GetCapture(), 0); // Implicit pointer capture is not mouse capture.
            let extra = SetMessageExtraInfo(0xff515780u32 as isize);
            mouse(body, WM_LBUTTONDOWN, start);
            mouse(body, WM_MOUSEMOVE, moved);
            assert_eq!(s.scroll, 0); // Compatibility stream must not duplicate native pointer motion.
            SetMessageExtraInfo(extra);
            pointer(body, PT_TOUCH, 17, WM_POINTERUPDATE, moved, true, false);
            assert_eq!(s.scroll, 100);
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(s.scroll, 100);
            let moved = POINT {
                x: start.x,
                y: start.y - 140,
            };
            pointer(body, PT_TOUCH, 17, WM_POINTERUPDATE, moved, true, false);
            assert_eq!(s.scroll, 140);
            pointer(body, PT_TOUCH, 18, WM_POINTERDOWN, start, false, false);
            pointer(body, PT_TOUCH, 18, WM_POINTERUPDATE, moved, false, false);
            assert_eq!(s.scroll, 140);
            pointer(body, PT_TOUCH, 17, WM_POINTERUP, moved, true, false);
            pointer(body, PT_TOUCH, 18, WM_POINTERUP, moved, false, false);
            SendMessageW(body, WM_POINTERCAPTURECHANGED, 17, 0);
            let extra = SetMessageExtraInfo(0xff515780u32 as isize);
            mouse(body, WM_LBUTTONUP, moved);
            // An independent promoted-only gesture after a native one still pans.
            mouse(body, WM_LBUTTONDOWN, start);
            assert_eq!(GetCapture(), s.hwnd);
            let finish = POINT {
                x: start.x,
                y: start.y - 50,
            };
            mouse(s.hwnd, WM_MOUSEMOVE, finish);
            mouse(s.hwnd, WM_LBUTTONUP, finish);
            assert_eq!(GetCapture(), 0);
            assert_eq!(s.scroll, 190);
            mouse(body, WM_LBUTTONDOWN, start);
            assert_eq!(GetCapture(), s.hwnd);
            SendMessageW(s.hwnd, WM_CANCELMODE, 0, 0);
            assert!(s.touch.owner().is_none());
            assert_eq!(GetCapture(), 0);
            mouse(body, WM_LBUTTONDOWN, start);
            assert_eq!(GetCapture(), s.hwnd);
            SetCapture(draft);
            assert!(s.touch.owner().is_none());
            assert_eq!(GetCapture(), draft);
            ReleaseCapture();
            SetMessageExtraInfo(extra);
            assert_eq!(selection(body), (11, 19));
            assert_eq!(selection(draft), (2, 6));
            assert_eq!(GetFocus(), draft);
            assert_eq!(s.serial, 0);
            assert!(rx.try_recv().is_err());
            // Forged notifications have no pointer payload to dereference.
            assert_eq!(SendMessageW(s.hwnd, touch::MESSAGE, usize::MAX, 1), 0);
            s.reveal(outgoing);
            let (x, y, w, h, _) = s.widgets["outgoing"].placed.unwrap();
            let mut p = POINT {
                x: x + w / 2,
                y: y + h / 2,
            };
            ClientToScreen(s.hwnd, &mut p);
            let before = s.scroll;
            SendMessageW(outgoing, EM_SETSEL, 3, 7);
            pointer(outgoing, PT_TOUCH, 31, WM_POINTERDOWN, p, true, false);
            p.y += 40;
            pointer(outgoing, PT_TOUCH, 31, WM_POINTERUPDATE, p, true, false);
            pointer(outgoing, PT_TOUCH, 31, WM_POINTERUP, p, true, false);
            assert_eq!(s.scroll, before - 40);
            assert_eq!(selection(outgoing), (3, 7));
            for target in [s.viewport, s.content, s.hwnd] {
                s.scroll_direct(0);
                let mut p = POINT {
                    x: left + 4,
                    y: top + 80,
                };
                ClientToScreen(s.hwnd, &mut p);
                pointer(target, PT_TOUCH, 40, WM_POINTERDOWN, p, true, false);
                p.y -= 30;
                pointer(target, PT_TOUCH, 40, WM_POINTERUPDATE, p, true, false);
                pointer(target, PT_TOUCH, 40, WM_POINTERUP, p, true, false);
                assert_eq!(s.scroll, 30);
            }
            s.scroll_direct(0);
            pointer(body, PT_TOUCH, 50, WM_POINTERDOWN, start, true, false);
            pointer(body, PT_TOUCH, 50, WM_POINTERUPDATE, moved, true, true);
            assert!(s.touch.owner().is_none());
            assert_eq!(s.scroll, 0);
            pointer(body, PT_TOUCH, 51, WM_POINTERDOWN, start, true, false);
            assert!(touch::route(body, WM_POINTERUPDATE, 51, 0, true));
            assert!(s.touch.owner().is_none());
            pointer(body, PT_TOUCH, 51, WM_POINTERDOWN, start, true, false);
            SendMessageW(body, WM_POINTERCAPTURECHANGED, 51, 0);
            assert!(s.touch.owner().is_none());
            // Reused IDs on the pinned composer/background are not claimed.
            let mut outside = POINT {
                x: left + 50,
                y: s.view_rect.1 + s.view_rect.3 + 20,
            };
            ClientToScreen(s.hwnd, &mut outside);
            let event = touch::Event {
                source: touch::Source::Pointer(51),
                phase: touch::Phase::Down,
                x: outside.x,
                y: outside.y,
                primary: true,
                owner: s.hwnd,
            };
            assert!(!s.touch_packet(&touch::Packet::Contact(event)));
            let mut pen: POINTER_INFO = std::mem::zeroed();
            pen.pointerType = PT_PEN;
            touch::POINTER_SAMPLE.with(|sample| sample.set(Some(pen)));
            assert!(!touch::route(body, WM_POINTERDOWN, 51, 0, true));
            touch::POINTER_SAMPLE.with(|sample| sample.set(None));
            assert!(!touch::route(draft, WM_POINTERDOWN, 51, 0, false));
            // Unmarked mouse and pen-promoted editing still reach native EDIT.
            for marker in [0isize, 0xff515700u32 as isize] {
                let extra = SetMessageExtraInfo(marker);
                SendMessageW(body, EM_SETSEL, 0, 0);
                mouse(body, WM_LBUTTONDOWN, start);
                mouse(body, WM_MOUSEMOVE, moved);
                mouse(body, WM_LBUTTONUP, moved);
                SetMessageExtraInfo(extra);
                assert_ne!(selection(body), (0, 0));
            }
            // Removing a captured owner cancels state without requiring paired Up.
            pointer(body, PT_TOUCH, 60, WM_POINTERDOWN, start, true, false);
            frame["controls"]
                .as_array_mut()
                .unwrap()
                .retain(|c| c["id"] != "answer");
            s.apply(parse(frame).unwrap()).unwrap();
            assert!(s.touch.owner().is_none());
            assert_eq!(IsWindow(body), 0);
            let root = s.hwnd;
            let extra = SetMessageExtraInfo(0xff515780u32 as isize);
            let mut p = POINT {
                x: s.view_rect.0 + 4,
                y: s.view_rect.1 + 30,
            };
            ClientToScreen(root, &mut p);
            mouse(root, WM_LBUTTONDOWN, p);
            assert_eq!(GetCapture(), root);
            SetMessageExtraInfo(extra);
            destroy(s);
            assert_eq!(IsWindow(root), 0);
            assert_ne!(GetCapture(), root);
        }
    }
    #[test]
    fn hidden_touch_compatibility_probe() {
        unsafe extern "system" {
            fn OutputDebugStringW(message: *const u16);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        unsafe {
            let mut s = window(
                "Hidden touch route",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            MoveWindow(s.hwnd, 0, 0, s.scale(1100), s.scale(650), 0);
            s.apply(parse(json!({"theme":"dark","layout":"conversation","controls":[
                {"id":"answer","kind":"detail","role":"incoming","value":"A selectable answer with Unicode café and 🙂. ".repeat(300)},
                {"id":"draft","kind":"multiline","area":"composer","lines":2,"value":"Unchanged composer"}
            ]})).unwrap()).unwrap();
            s.scroll_direct(0);
            let child = s.widgets["answer"].hwnd;
            SendMessageW(child, EM_SETSEL, 0, 0);
            let mut start = POINT { x: 20, y: 120 };
            ClientToScreen(child, &mut start);
            let finish = POINT {
                x: start.x,
                y: start.y - 100,
            };
            let previous = SetMessageExtraInfo(0xff515780u32 as isize);
            let deliver = |target: HWND, msg: u32, point: POINT| {
                let mut client = point;
                ScreenToClient(target, &mut client);
                let lp = ((client.x as u16 as u32) | ((client.y as u16 as u32) << 16)) as isize;
                SendMessageW(
                    target,
                    msg,
                    if msg == WM_LBUTTONUP {
                        0
                    } else {
                        windows_sys::Win32::System::SystemServices::MK_LBUTTON as usize
                    },
                    lp,
                );
            };
            OutputDebugStringW(wide("PHASE compatibility-touch-down").as_ptr());
            deliver(child, WM_LBUTTONDOWN, start);
            let captured = GetCapture();
            OutputDebugStringW(wide("PHASE compatibility-touch-move").as_ptr());
            deliver(
                if captured == 0 { child } else { captured },
                WM_MOUSEMOVE,
                finish,
            );
            OutputDebugStringW(wide("PHASE compatibility-touch-up").as_ptr());
            let captured = GetCapture();
            deliver(
                if captured == 0 { child } else { captured },
                WM_LBUTTONUP,
                finish,
            );
            SetMessageExtraInfo(previous);
            let mut a = 0u32;
            let mut b = 0u32;
            SendMessageW(
                child,
                EM_GETSEL,
                &mut a as *mut _ as usize,
                &mut b as *mut _ as isize,
            );
            eprintln!(
                "touch_compat_probe root={:x} answer={child:x} scroll={} selection={a}..{b} capture={:x} serial={} event={:?}",
                s.hwnd,
                s.scroll,
                GetCapture(),
                s.serial,
                rx.try_recv().ok()
            );
            OutputDebugStringW(wide("PHASE compatibility-touch-finished").as_ptr());
            assert_eq!(s.scroll, 100);
            assert_eq!((a, b), (0, 0));
            assert_eq!(GetCapture(), 0);
            assert_eq!(s.serial, 0);
            destroy(s);
        }
    }
    #[test]
    fn hidden_sections_segments_and_workspace_header_keep_native_state_and_geometry() {
        let (tx, rx) = std::sync::mpsc::channel();
        unsafe {
            let mut s = window(
                "Toolkit contracts",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            MoveWindow(s.hwnd, 0, 0, s.scale(1200), s.scale(650), 0);
            let mut frame = json!({"theme":"dark","layout":"workspace","controls":[
                {"id":"header","kind":"heading","role":"title","area":"header","label":"Workspace"},
                {"id":"nav","kind":"button","role":"selected","side":true,"label":"Tools","icon":"tools"},
                {"id":"library","kind":"button","role":"tab","row":"pages","label":"Library","selected":true},
                {"id":"versions","kind":"button","role":"tab","row":"pages","label":"Versions"},
                {"id":"changes","kind":"button","role":"tab","row":"pages","label":"Changes"},
                {"id":"search","kind":"input","section":"library","label":"Search","value":"A query"},
                {"id":"list","kind":"list","section":"library","label":"Results","choices":[{"label":"A tool","value":"a"}],"value":"a","lines":6},
                {"id":"caption","kind":"heading","role":"caption","section":"settings","label":"Settings"},
                {"id":"check","kind":"check","section":"settings","label":"Enabled","value":true},
                {"id":"details","kind":"detail","role":"body","section":"settings","value":"Readable selected detail","lines":18}
            ]});
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(IsWindowVisible(s.hwnd), 0);
            assert!(s.widgets["header"].fixed);
            assert_eq!(s.view_rect.1, s.scale(52));
            assert_eq!(s.section_rects.len(), 2);
            assert!(s.section_rects[1].1 > s.section_rects[0].3);
            assert_eq!(s.tab_rects.len(), 1);
            let a = s.widgets["library"].hwnd;
            let b = s.widgets["versions"].hwnd;
            let field = s.widgets["search"].hwnd;
            assert_eq!(SendMessageW(a, BM_GETCHECK, 0, 0), BST_CHECKED as isize);
            assert_eq!(SendMessageW(b, BM_GETCHECK, 0, 0), BST_UNCHECKED as isize);
            assert_eq!(access::native_name(a), "Library");
            assert_eq!(
                access::native_variant(a, 13),
                windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_RADIOBUTTON as i32
            );
            assert_ne!(
                access::native_variant(a, 14) as u32 & STATE_SYSTEM_CHECKED,
                0
            );
            assert_eq!(GetNextDlgGroupItem(s.content, a, 0), b);
            assert_ne!(GetWindowLongW(a, GWL_STYLE) as u32 & WS_GROUP, 0);
            assert_eq!(GetWindowLongW(b, GWL_STYLE) as u32 & WS_GROUP, 0);
            assert!(s.widgets["search"].rect.0 >= s.section_rects[0].0 + s.scale(16));
            assert!(
                s.widgets["search"].rect.0 + s.widgets["search"].rect.2
                    <= s.section_rects[0].2 - s.scale(16)
            );
            SetFocus(field);
            SendMessageW(field, EM_SETSEL, 2, 5);
            s.command(b, BN_CLICKED);
            assert_eq!(rx.try_recv().unwrap()["id"], "versions");
            frame["controls"][2]["selected"] = json!(false);
            frame["controls"][3]["selected"] = json!(true);
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(s.widgets["library"].hwnd, a);
            assert_eq!(s.widgets["versions"].hwnd, b);
            assert_eq!(s.widgets["search"].hwnd, field);
            assert_eq!(GetFocus(), field);
            assert_ne!(
                access::native_variant(b, 14) as u32 & STATE_SYSTEM_CHECKED,
                0
            );
            assert_eq!(
                access::native_variant(a, 14) as u32 & STATE_SYSTEM_CHECKED,
                0
            );
            let y = s.widgets["header"].placed.unwrap().1;
            s.scroll_direct(s.scroll_limit());
            assert_eq!(s.widgets["header"].placed.unwrap().1, y);
            MoveWindow(s.hwnd, 0, 0, s.scale(360), s.scale(500), 0);
            s.layout();
            for rect in &s.section_rects {
                assert!(rect.2 > rect.0 && rect.3 > rect.1);
            }
            assert!(!s.tab_rects.is_empty());
            s.reveal(field);
            assert_eq!(GetFocus(), field);
            let mut lo = 0u32;
            let mut hi = 0u32;
            SendMessageW(
                field,
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (2, 5));
            frame["layout"] = json!("welcome");
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(s.section_rects.len(), 2);
            frame["theme"] = json!("system");
            s.apply(parse(frame).unwrap()).unwrap();
            assert!(s.section_rects.is_empty());
            assert!(s.tab_rects.is_empty());
            assert_eq!(text(field), "A query");
            destroy(s);
        }
    }
    #[test]
    fn hidden_edit_paint_message_probe() {
        unsafe extern "system" {
            fn OutputDebugStringW(message: *const u16);
        }
        let (tx, _rx) = std::sync::mpsc::channel();
        unsafe {
            let mut s = window(
                "Edit paint probe",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            MoveWindow(s.hwnd, 0, 0, s.scale(1200), s.scale(900), 0);
            let mut frame = json!({"theme":"dark","layout":"conversation","composer":"draft","controls":[
                {"id":"body","kind":"detail","role":"incoming","value":"Selectable native prose.\r\nA second line."},
                {"id":"out","kind":"detail","role":"outgoing","value":"A short reply."},
                {"id":"draft","kind":"multiline","area":"composer","label":"Message","caption":false,"placeholder":"Message…","value":"","lines":2},
                {"id":"field","kind":"input","label":"Field","value":"Editable field"}
            ]});
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            let handles: Vec<_> = ["body", "out", "draft", "field"]
                .iter()
                .map(|id| s.widgets[*id].hwnd)
                .collect();
            let probe = |s: &State, stage: &str| {
                for id in ["body", "out", "draft", "field"] {
                    let hwnd = s.widgets[id].hwnd;
                    // A DC only: no bitmap allocation, capture or pixel inspection.
                    // CDB observes native drawing calls and their stacks.
                    let dc = CreateCompatibleDC(0);
                    eprintln!(
                        "edit_paint_probe stage={stage} {id} hwnd={hwnd:x} style={:x} exstyle={:x}",
                        GetWindowLongW(hwnd, GWL_STYLE),
                        GetWindowLongW(hwnd, GWL_EXSTYLE)
                    );
                    OutputDebugStringW(
                        wide(&format!("PHASE {stage}-{id}-native-client-paint")).as_ptr(),
                    );
                    InvalidateRect(hwnd, null(), 0);
                    SendMessageW(hwnd, WM_PAINT, 0, 0);
                    OutputDebugStringW(
                        wide(&format!("PHASE {stage}-{id}-native-printclient")).as_ptr(),
                    );
                    SendMessageW(hwnd, WM_PRINTCLIENT, dc as usize, PRF_CLIENT as isize);
                    OutputDebugStringW(wide(&format!("PHASE {stage}-{id}-native-print")).as_ptr());
                    SendMessageW(
                        hwnd,
                        WM_PRINT,
                        dc as usize,
                        (PRF_CLIENT | PRF_ERASEBKGND | PRF_NONCLIENT) as isize,
                    );
                    OutputDebugStringW(
                        wide(&format!("PHASE {stage}-{id}-native-nonclient")).as_ptr(),
                    );
                    SendMessageW(hwnd, WM_NCPAINT, 1, 0);
                    DeleteDC(dc);
                }
            };
            probe(&s, "dark");
            SendMessageW(handles[0], EM_SETSEL, 3, 11);
            SetFocus(handles[3]);
            SendMessageW(handles[3], EM_SETSEL, 1, 5);
            frame["controls"][0]["role"] = json!("neutral");
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_ne!(GetWindowLongW(handles[0], GWL_STYLE) as u32 & WS_BORDER, 0);
            probe(&s, "ordinary-role");
            frame["theme"] = json!("system");
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            probe(&s, "system");
            frame["theme"] = json!("dark");
            frame["controls"][0]["role"] = json!("incoming");
            s.apply(parse(frame).unwrap()).unwrap();
            probe(&s, "dark-return");
            MoveWindow(s.hwnd, 0, 0, s.scale(420), s.scale(220), 0);
            s.layout();
            assert!(!s.composer_fixed);
            probe(&s, "compact");
            MoveWindow(s.hwnd, 0, 0, s.scale(1200), s.scale(900), 0);
            s.layout();
            assert!(s.composer_fixed);
            probe(&s, "dark-restored");
            for (id, hwnd) in ["body", "out", "draft", "field"].iter().zip(handles.iter()) {
                assert_eq!(s.widgets[*id].hwnd, *hwnd);
            }
            assert_eq!(GetFocus(), handles[3]);
            assert_eq!(text(handles[3]), "Editable field");
            let mut lo = 0u32;
            let mut hi = 0u32;
            SendMessageW(
                handles[3],
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (1, 5));
            SendMessageW(
                handles[0],
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (3, 11));
            OutputDebugStringW(wide("PHASE edit-paint-finished").as_ptr());
            destroy(s);
        }
    }
    #[test]
    fn composer_first_keystroke_can_submit_before_readiness_ack_without_mouse_bypass() {
        let (tx, rx) = std::sync::mpsc::channel();
        unsafe {
            let mut s = window(
                "Pending composer intent",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            let mut frame = json!({"theme":"dark","layout":"conversation","composer":"draft","default":"send","controls":[
                {"id":"draft","kind":"multiline","area":"composer","value":"","caption":false,"label":"Message","lines":2},
                {"id":"send","kind":"button","area":"composer","enabled":false,"label":"Send"}
            ]});
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            let draft = s.widgets["draft"].hwnd;
            let send = s.widgets["send"].hwnd;
            s.pending_keyboard_activation(send, draft);
            assert!(rx.try_recv().is_err());
            set_text(draft, "x");
            s.command(draft, EN_CHANGE);
            let change = rx.try_recv().unwrap();
            assert_eq!(change["value"], "x");
            s.command(send, BN_CLICKED);
            assert!(rx.try_recv().is_err());
            s.pending_keyboard_activation(send, draft);
            let click = rx.try_recv().unwrap();
            assert_eq!(click["type"], "click");
            assert_eq!(click["id"], "send");
            assert_eq!(click["pending_edit"], change["serial"]);
            assert!(click["serial"].as_u64().unwrap() > change["serial"].as_u64().unwrap());
            assert_eq!(IsWindowEnabled(send), 0);
            s.pending_keyboard_activation(send, draft);
            assert!(rx.try_recv().is_err());
            frame["ack"] = click["serial"].clone();
            frame["controls"][0]["value"] = json!("x");
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            s.pending_keyboard_activation(send, draft);
            assert!(rx.try_recv().is_err());
            set_text(draft, "   ");
            s.command(draft, EN_CHANGE);
            let blank = rx.try_recv().unwrap();
            s.pending_keyboard_activation(send, draft);
            assert!(rx.try_recv().is_err());
            frame["ack"] = blank["serial"].clone();
            frame["controls"][0]["value"] = json!("   ");
            frame["controls"][1]["enabled"] = json!(true);
            s.apply(parse(frame).unwrap()).unwrap();
            s.command(send, BN_CLICKED);
            assert_eq!(rx.try_recv().unwrap()["type"], "click");
            destroy(s);
        }
    }
    #[test]
    fn hidden_chat_components_preserve_native_prose_cues_header_and_compact_editing() {
        let (tx, rx) = std::sync::mpsc::channel();
        unsafe {
            let mut s = window(
                "Chat components",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            MoveWindow(s.hwnd, 0, 0, s.scale(1280), s.scale(900), 0);
            let mut frame = json!({"theme":"dark","layout":"conversation","conversation":"components","composer":"draft","default":"send","controls":[
                {"id":"title","kind":"heading","area":"header","role":"title","label":"A project"},
                {"id":"rail","kind":"button","side":true,"role":"selected","label":"Chat"},
                {"id":"answer","kind":"detail","role":"incoming","label":"Assistant","value":"Selectable prose with café, 日本語 and 🙂.\n\nAnother paragraph. ".repeat(35)},
                {"id":"short","kind":"detail","role":"outgoing","label":"You","caption":false,"value":"A short reply 🙂"},
                {"id":"draft","kind":"multiline","area":"composer","caption":false,"label":"Message","placeholder":"Write a message…","value":"","lines":2},
                {"id":"attach","kind":"button","area":"composer","role":"quiet","label":"Attach","row":"commands","icon":"attachment"},
                {"id":"send","kind":"button","area":"composer","role":"primary","label":"Send","row":"commands","enabled":false,"icon":"send"}
            ]});
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            let prose = s.widgets["answer"].hwnd;
            let draft = s.widgets["draft"].hwnd;
            let title = s.widgets["title"].hwnd;
            assert!(s.composer_fixed);
            assert!(s.widgets["title"].fixed);
            assert_eq!(GetParent(title), s.hwnd);
            assert_ne!(GetWindowLongW(title, GWL_STYLE) as u32 & SS_ENDELLIPSIS, 0);
            assert_eq!(s.view_rect.1, s.scale(52));
            assert_eq!(
                GetWindowLongW(prose, GWL_STYLE) as u32 & (WS_BORDER | WS_VSCROLL),
                0
            );
            assert_eq!(
                GetWindowLongW(prose, GWL_EXSTYLE) as u32 & (WS_EX_CLIENTEDGE | WS_EX_STATICEDGE),
                0
            );
            assert_eq!(
                GetWindowLongW(draft, GWL_STYLE) as u32 & (WS_BORDER | WS_VSCROLL),
                0
            );
            assert_eq!(
                GetWindowLongW(s.widgets["draft"].caption, GWL_STYLE) as u32 & WS_VISIBLE,
                0
            );
            assert_eq!(access::native_name(draft), "Message");
            assert_eq!(access::native_name(s.widgets["short"].hwnd), "You");
            frame["controls"][4]["caption"] = json!(true);
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(s.widgets["draft"].hwnd, draft);
            assert_ne!(
                GetWindowLongW(s.widgets["draft"].caption, GWL_STYLE) as u32 & WS_VISIBLE,
                0
            );
            frame["controls"][4]["caption"] = json!(false);
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(s.widgets["draft"].hwnd, draft);
            assert!(s.widgets["draft"].surface.show_placeholder());
            SendMessageW(draft, WM_PAINT, 0, 0);
            assert_eq!(text(draft), "");
            assert_eq!(s.serial, 0);
            assert!(rx.try_recv().is_err());
            assert!(s.widgets["short"].rect.2 < s.widgets["answer"].rect.2 / 2);
            assert_eq!(
                s.widgets["short"].rect.0 + s.widgets["short"].rect.2,
                s.widgets["answer"].rect.0 + s.widgets["answer"].rect.2
            );
            assert!(s.widgets["draft"].rect.3 <= s.scale(64));
            assert!(
                s.background.footer_card.get().unwrap().3
                    - s.background.footer_card.get().unwrap().1
                    < s.scale(160)
            );
            SendMessageW(prose, EM_SETSEL, 4, 17);
            SendMessageW(prose, WM_NCPAINT, 1, 0);
            let header_y = s.widgets["title"].placed.unwrap().1;
            s.scroll_direct(0);
            SendMessageW(prose, WM_MOUSEWHEEL, (-30i16 as u16 as usize) << 16, 0);
            assert!(s.scroll > 0);
            let reading_offset = s.scroll;
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(s.scroll, reading_offset);
            assert_eq!(s.widgets["title"].placed.unwrap().1, header_y);
            let mut start = 0;
            let mut end = 0;
            SendMessageW(
                prose,
                EM_GETSEL,
                &mut start as *mut _ as usize,
                &mut end as *mut _ as isize,
            );
            assert_eq!((start, end), (4u32, 17u32));
            s.command(s.widgets["send"].hwnd, BN_CLICKED);
            assert!(rx.try_recv().is_err());
            SetFocus(draft);
            set_text(draft, "Line1\r\nLine2\r\nLine3\r\nLine4\r\nLine5");
            s.command(draft, EN_CHANGE);
            assert!(!s.widgets["draft"].surface.show_placeholder());
            assert_ne!(GetWindowLongW(draft, GWL_STYLE) as u32 & WS_VSCROLL, 0);
            let event = rx.try_recv().unwrap();
            assert_eq!(event["type"], "change");
            frame["ack"] = event["serial"].clone();
            frame["controls"][4]["value"] = event["value"].clone();
            frame["controls"][4]["placeholder"] = json!("A different hint");
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(s.widgets["draft"].hwnd, draft);
            assert_eq!(GetFocus(), draft);
            assert_eq!(access::native_name(draft), "Message");
            MoveWindow(s.hwnd, 0, 0, s.scale(420), s.scale(240), 0);
            s.layout();
            assert!(!s.composer_fixed);
            s.reveal(draft);
            assert_eq!(GetFocus(), draft);
            assert!(s.widgets["draft"].rect.1 < s.scroll + s.viewport_height);
            assert_eq!(
                GetNextDlgTabItem(s.hwnd, draft, 0),
                s.widgets["attach"].hwnd
            );
            frame["theme"] = json!("system");
            s.apply(parse(frame).unwrap()).unwrap();
            assert_ne!(GetWindowLongW(prose, GWL_STYLE) as u32 & WS_BORDER, 0);
            destroy(s);
        }
    }
    use std::time::Instant;
    #[test]
    fn nested_viewport_preserves_commands_colors_tab_selection_and_drop_routing() {
        unsafe {
            use windows_sys::Win32::System::Memory::{
                GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
            };
            let (tx, rx) = std::sync::mpsc::channel();
            let mut s = window(
                "hidden nested contracts",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            MoveWindow(s.hwnd, 0, 0, s.scale(1100), s.scale(800), 0);
            s.apply(parse(json!({"theme":"dark","layout":"conversation","conversation":"routing","composer":"draft","default":"send","controls":[
            {"id":"rail","kind":"button","role":"nav","side":true,"label":"Navigation"},
            {"id":"input","kind":"input","label":"Search","value":"Original"},
            {"id":"action","kind":"button","label":"Transcript action"},
            {"id":"draft","kind":"multiline","area":"composer","label":"Message","value":"Draft 🙂 text","lines":3},
            {"id":"send","kind":"button","area":"composer","role":"primary","label":"Send","width":100}
        ]})).unwrap()).unwrap();
            let input = s.widgets["input"].hwnd;
            let action = s.widgets["action"].hwnd;
            let draft = s.widgets["draft"].hwnd;
            assert_eq!(GetParent(input), s.content);
            assert_eq!(GetParent(s.content), s.viewport);
            assert_eq!(GetAncestor(input, GA_ROOT), s.hwnd);
            assert_eq!(GetParent(draft), s.hwnd);
            for hwnd in [s.viewport, s.content] {
                assert_ne!(
                    GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_CONTROLPARENT,
                    0
                );
            }
            assert_ne!(
                GetWindowLongW(s.viewport, GWL_EXSTYLE) as u32 & WS_EX_COMPOSITED,
                0
            );
            assert_eq!(GetNextDlgTabItem(s.hwnd, input, 0), action);
            assert_eq!(GetNextDlgTabItem(s.hwnd, action, 0), draft);
            assert_eq!(GetNextDlgTabItem(s.hwnd, draft, 1), action);
            {
                let _guard = UpdateGuard::enter();
                set_text(input, "Changed café");
            }
            SendMessageW(input, EM_SETSEL, 2, 7);
            SendMessageW(s.content, WM_COMMAND, (EN_CHANGE as usize) << 16, input);
            let event = rx.try_recv().unwrap();
            assert_eq!(event["id"], "input");
            assert_eq!(event["value"], "Changed café");
            let dc = GetDC(input);
            assert_ne!(
                SendMessageW(s.content, WM_CTLCOLOREDIT, dc as usize, input),
                0
            );
            ReleaseDC(input, dc);
            SendMessageW(input, WM_NCPAINT, 1, 0);
            let mut lo = 0u32;
            let mut hi = 0u32;
            SendMessageW(
                input,
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (2, 7));
            assert_eq!(text(input), "Changed café");
            let path = "C:\\Synthetic\\example.txt";
            let mut encoded = wide(path);
            encoded.push(0);
            let header = std::mem::size_of::<DROPFILES>();
            let memory = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, header + encoded.len() * 2);
            assert!(!memory.is_null());
            let ptr = GlobalLock(memory) as *mut u8;
            assert!(!ptr.is_null());
            (ptr as *mut DROPFILES).write(DROPFILES {
                pFiles: header as u32,
                pt: POINT { x: 0, y: 0 },
                fNC: 0,
                fWide: 1,
            });
            std::ptr::copy_nonoverlapping(
                encoded.as_ptr() as *const u8,
                ptr.add(header),
                encoded.len() * 2,
            );
            GlobalUnlock(memory);
            SendMessageW(s.content, WM_DROPFILES, memory as usize, 0);
            let event = rx.try_recv().unwrap();
            assert_eq!(event, json!({"type":"drop","paths":[path]}));
            SendMessageW(draft, EM_SETSEL, 3, 8);
            SetFocus(draft);
            let focus_before = GetFocus();
            assert_eq!(focus_before, draft);
            MoveWindow(s.hwnd, 0, 0, s.scale(370), s.scale(280), 0);
            assert_eq!(GetParent(draft), s.content);
            assert_eq!(s.widgets["draft"].hwnd, draft);
            SendMessageW(
                draft,
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (3, 8));
            assert_eq!(GetFocus(), focus_before);
            s.reveal(draft);
            let mut actual = POINT { x: 0, y: 0 };
            MapWindowPoints(draft, s.hwnd, &mut actual, 1);
            assert!(actual.y >= 0 && actual.y < s.viewport_height);
            destroy(s);
        }
    }
    #[test]
    fn moving_native_origin_reaches_history_beyond_the_window_height_cap() {
        unsafe {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut s = window(
                "hidden large native history",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            s.apply(parse(json!({"theme":"dark","layout":"workspace","controls":[
            {"id":"one","kind":"detail","role":"body","value":"First retained text","lines":1000},
            {"id":"two","kind":"detail","role":"body","value":"Middle retained text","lines":1000},
            {"id":"three","kind":"detail","role":"body","value":"Last retained text","lines":1000}
        ]})).unwrap()).unwrap();
            assert!(s.content_height > 65535);
            let first = s.widgets["one"].hwnd;
            let last = s.widgets["three"].hwnd;
            s.scroll_direct(0);
            assert_eq!(s.origin, 0);
            let mut origin = POINT { x: 0, y: 0 };
            MapWindowPoints(first, s.hwnd, &mut origin, 1);
            assert!(origin.y >= 0 && origin.y < s.viewport_height);
            s.scroll_direct(s.scroll_limit());
            assert!(s.origin >= ORIGIN_STEP);
            assert_eq!(s.widgets["one"].hwnd, first);
            assert_eq!(s.widgets["three"].hwnd, last);
            let mut canvas: RECT = std::mem::zeroed();
            GetClientRect(s.content, &mut canvas);
            assert!(canvas.bottom <= CANVAS_SPAN);
            let mut actual = POINT { x: 0, y: 0 };
            MapWindowPoints(last, s.hwnd, &mut actual, 1);
            assert_eq!(actual.y, s.widgets["three"].rect.1 - s.scroll);
            assert!(actual.y + s.widgets["three"].rect.3 <= s.viewport_height);
            assert_eq!(GetNextDlgTabItem(s.hwnd, last, 0), first);
            s.reveal(first);
            assert_eq!(s.origin, 0);
            assert!(s.scroll <= s.widgets["one"].rect.1);
            assert_eq!(text(last), "Last retained text");
            s.scroll_direct(ORIGIN_STEP - 1);
            let before = s.widgets["two"].rect.1 - s.scroll;
            s.scroll_direct(ORIGIN_STEP + 1);
            let mut actual = POINT { x: 0, y: 0 };
            MapWindowPoints(s.widgets["two"].hwnd, s.hwnd, &mut actual, 1);
            assert_eq!(actual.y, before - 2);
            destroy(s);
        }
    }
    #[test]
    fn precise_scroll_refresh_and_native_timer_motion_keep_explicit_user_intent() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut s = window(
            "hidden scroll intent",
            tx,
            Arc::new(Mutex::new(Pending::default())),
        )
        .unwrap();
        let mut frame = json!({"theme":"dark","layout":"conversation","conversation":"same","controls":[{"id":"long","kind":"detail","role":"incoming","value":"Native wrapped prose. ".repeat(600)}]});
        s.apply(parse(frame.clone()).unwrap()).unwrap();
        assert!(s.scroll_limit() > s.scale(200));
        s.scroll_direct(s.scroll_limit());
        let tail = s.scroll;
        s.wheel_with_policy(15, false);
        let away = s.scroll;
        assert!(away < tail && away > tail - s.scale(24));
        assert!(!s.follow_intent);
        s.apply(parse(frame.clone()).unwrap()).unwrap();
        assert_eq!(s.scroll, away);
        s.scroll_direct(s.scroll_limit());
        let tail = s.scroll;
        let child = s.widgets["long"].hwnd;
        let previous = WHEEL_TOUCHPAD.with(|source| source.replace(Some(true)));
        unsafe {
            SendMessageW(child, WM_MOUSEWHEEL, (120u32 << 16) as usize, 0);
        }
        WHEEL_TOUCHPAD.with(|source| source.set(previous));
        assert!(s.scroll < tail);
        assert!(!s.motion.active() && !s.timer_active);
        s.scroll_direct(s.scroll_limit());
        s.wheel_with_policy(120, true);
        assert!(s.timer_active && s.motion.active());
        assert!(!s.follow_intent);
        let start = s.motion_now();
        let target = s.motion.target();
        s.apply(parse(frame.clone()).unwrap()).unwrap();
        s.tick_at(start + 30);
        assert!(s.scroll < tail && s.scroll > target);
        let middle = s.scroll;
        s.apply(parse(frame.clone()).unwrap()).unwrap();
        assert_eq!(s.scroll, middle);
        s.tick_at(start + 250);
        assert_eq!(s.scroll, target);
        assert!(!s.timer_active && !s.motion.active());
        s.wheel_with_policy(-120, true);
        let reversing = s.motion_now();
        let before = s.scroll;
        s.tick_at(reversing + 25);
        assert!(s.scroll >= before);
        s.wheel_with_policy(120, true);
        let reverse = s.motion_now();
        let before = s.scroll;
        s.tick_at(reverse + 25);
        assert!(s.scroll <= before);
        s.scroll_direct(42);
        assert_eq!(s.scroll, 42);
        assert!(!s.timer_active);
        s.scroll_direct(tail);
        s.wheel_with_policy(120, false);
        let detent = s.scroll;
        s.scroll_direct(tail);
        for _ in 0..8 {
            s.wheel_with_policy(15, false);
        }
        assert_eq!(s.scroll, detent);
        s.scroll_direct(0);
        s.wheel_with_policy(120, true);
        assert_eq!(s.scroll, 0);
        assert!(!s.timer_active);
        s.scroll_direct(s.scroll_limit());
        s.wheel_with_policy(120, true);
        let mut navigation = frame.clone();
        navigation["layout"] = json!("workspace");
        navigation["conversation"] = Value::Null;
        s.apply(parse(navigation).unwrap()).unwrap();
        assert!(!s.timer_active && !s.motion.active());
        s.apply(parse(frame.clone()).unwrap()).unwrap();
        s.scroll_direct(s.scroll_limit());
        s.wheel_with_policy(120, true);
        frame["controls"] =
            json!([{"id":"short","kind":"detail","role":"incoming","value":"Short"}]);
        s.apply(parse(frame).unwrap()).unwrap();
        assert_eq!(s.scroll, 0);
        assert!(!s.timer_active);
        destroy(s);
    }
    #[test]
    fn hidden_scroll_containment_probe() {
        unsafe {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn OutputDebugStringW(message: *const u16);
            }
            let mark = |message: &str| {
                OutputDebugStringW(wide(message).as_ptr());
                eprintln!("{message}");
            };
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut s = window(
                "hidden current-path scroll diagnostic",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            MoveWindow(s.hwnd, 0, 0, s.scale(1000), s.scale(600), 0);
            let rail:Vec<_>=(0..8).map(|i|json!({"id":format!("rail-{i}"),"kind":"button","role":"nav","side":true,"label":format!("Project view {i}")})).collect();
            let mut chars: Vec<char> = "Synthetic answer with readable words and native wrapping. "
                .repeat(30)
                .chars()
                .take(1013)
                .collect();
            for i in [200, 400, 600, 800] {
                chars[i] = '\n';
            }
            let answer: String = chars.into_iter().collect();
            let mut controls = rail.clone();
            controls.extend([
            json!({"id":"outgoing","kind":"detail","role":"outgoing","label":"You","value":"Synthetic outgoing message text!!"}),
            json!({"id":"incoming","kind":"detail","role":"incoming","label":"Assistant","value":answer}),
            json!({"id":"follow","kind":"button","role":"quiet","label":"Follow-up action"}),
            json!({"id":"target","kind":"label","role":"caption","label":"Target · Local project","area":"composer"}),
            json!({"id":"draft","kind":"multiline","label":"Message","value":"Draft selection remains native.","lines":3,"area":"composer"}),
            json!({"id":"new","kind":"button","role":"quiet","label":"New topic","area":"composer","row":"commands","width":110}),
            json!({"id":"attach","kind":"button","role":"quiet","label":"Attach","area":"composer","row":"commands","width":110}),
            json!({"id":"send","kind":"button","role":"primary","label":"Send","area":"composer","row":"commands","width":90})
        ]);
            let mut frame = json!({"theme":"dark","layout":"conversation","conversation":"synthetic","composer":"draft","default":"send","controls":controls});
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            let before = s.scroll;
            let limit = s.scroll_limit();
            let child = s.widgets["incoming"].hwnd;
            let root = s.hwnd;
            SCROLL_TRACE.with(|c| c.set([0; 4]));
            dark::EDIT_SIZES.with(|c| c.set(0));
            mark("PHASE conversation-precision-scroll");
            SendMessageW(child, WM_MOUSEWHEEL, (15u32 << 16) as usize, 0);
            let away = s.scroll;
            let trace = SCROLL_TRACE.with(Cell::get);
            let edit_sizes = dark::EDIT_SIZES.with(Cell::get);
            assert_eq!(edit_sizes, 0);
            assert_eq!(trace, [0, 0, 1, 0]);
            mark("PHASE conversation-identical-refresh");
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            let refreshed = s.scroll;
            assert_eq!(refreshed, away);
            assert!(away < before);
            eprintln!(
                "containment_probe viewport={:x} canvas={:x} edit_size_messages={edit_sizes}",
                s.viewport, s.content
            );
            eprintln!(
                "containment_probe case=conversation root={root:x} body_parent={:x} footer_parent={:x} limit={limit} before={before} precision_away={away} after_refresh={refreshed} layout_passes={} region_updates={} child_moves={} root_redraws={}",
                GetParent(child),
                GetParent(s.widgets["draft"].hwnd),
                trace[0],
                trace[1],
                trace[2],
                trace[3]
            );
            SCROLL_TRACE.with(|c| c.set([0; 4]));
            mark("PHASE conversation-coarse-scroll");
            let coarse_before = s.scroll;
            SendMessageW(child, WM_MOUSEWHEEL, (120u32 << 16) as usize, 0);
            let trajectory_start = s.motion_now();
            eprintln!(
                "containment_probe case=coarse delta=120 before={coarse_before} immediate_after={} trace={:?}",
                s.scroll,
                SCROLL_TRACE.with(Cell::get)
            );
            for elapsed in [30, 60, 90, 150] {
                s.tick_at(trajectory_start + elapsed);
                eprintln!(
                    "containment_trajectory case=conversation ms={elapsed} offset={} active={}",
                    s.scroll,
                    s.motion.active()
                );
            }
            assert!(!s.timer_active);
            let mut short = rail.clone();
            short.extend(
                frame["controls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|c| c["area"] == "composer")
                    .cloned(),
            );
            short.push(json!({"id":"short","kind":"detail","role":"incoming","label":"Assistant","value":"A short transcript."}));
            frame["controls"] = json!(short);
            frame["conversation"] = json!("short");
            mark("PHASE short-transcript-tall-fitting-rail");
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(s.scroll_limit(), 0);
            eprintln!(
                "containment_probe case=short fixed_rail={} footer={} blank_limit={}",
                s.fixed_rail,
                s.composer_fixed,
                s.scroll_limit()
            );
            let mut page = rail;
            page.extend((0..20).map(|i|json!({"id":format!("field-{i}"),"kind":"input","label":format!("Setting {i}"),"value":"Native settings value"})));
            frame = json!({"theme":"dark","layout":"workspace","controls":page});
            mark("PHASE workspace-prepare");
            s.apply(parse(frame).unwrap()).unwrap();
            s.scroll = s.scroll_limit() / 2;
            s.layout();
            SCROLL_TRACE.with(|c| c.set([0; 4]));
            dark::EDIT_SIZES.with(|c| c.set(0));
            mark("PHASE workspace-scroll");
            let workspace_before = s.scroll;
            SendMessageW(s.hwnd, WM_MOUSEWHEEL, (120u32 << 16) as usize, 0);
            let trajectory_start = s.motion_now();
            for elapsed in [30, 60, 90, 150] {
                s.tick_at(trajectory_start + elapsed);
                eprintln!(
                    "containment_trajectory case=workspace ms={elapsed} offset={} active={}",
                    s.scroll,
                    s.motion.active()
                );
            }
            eprintln!(
                "containment_probe case=workspace before={workspace_before} after={} trace={:?}",
                s.scroll,
                SCROLL_TRACE.with(Cell::get)
            );
            assert_eq!(dark::EDIT_SIZES.with(Cell::get), 0);
            let trace = SCROLL_TRACE.with(Cell::get);
            assert_eq!(trace[0], 0);
            assert_eq!(trace[1], 0);
            assert_eq!(trace[3], 0);
            assert!(!s.timer_active);
            let sizing = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("").as_ptr(),
                WS_CHILD,
                0,
                0,
                64,
                200000,
                s.hwnd,
                0,
                GetModuleHandleW(null()),
                null(),
            );
            let mut extent: RECT = std::mem::zeroed();
            GetClientRect(sizing, &mut extent);
            eprintln!("containment_probe native_content_height={}", extent.bottom);
            DestroyWindow(sizing);
            assert_eq!(IsWindowVisible(s.hwnd), 0);
            destroy(s);
        }
    }
    #[test]
    fn composer_enter_policy_preserves_modifiers_repeats_and_ime() {
        assert!(enter_sends(false, false, false, false, true, false));
        assert!(!enter_sends(false, true, false, false, true, false));
        assert!(!enter_sends(false, false, false, false, false, false));
        assert!(!enter_sends(false, false, false, false, true, true));
        assert!(!enter_sends(true, false, false, false, true, true));
        assert!(!enter_sends(false, false, true, false, true, false));
        assert!(!enter_sends(false, false, false, true, true, false));
        assert!(enter_sends(true, false, false, false, false, false));
    }
    #[test]
    fn hidden_conversation_footer_tail_history_wrap_and_compact_fallback() {
        unsafe {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut s = window(
                "hidden conversation",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            MoveWindow(s.hwnd, 0, 0, s.scale(1200), s.scale(900), 0);
            let prose =
                "A long Unicode paragraph with café, 日本語 and 🙂 remains selectable. ".repeat(70);
            let mut frame = json!({"theme":"dark","layout":"conversation","conversation":"one","composer":"draft","default":"send","controls":[
            {"id":"rail","kind":"button","side":true,"role":"selected","label":"Current project","icon":"chat"},
                {"id":"first","kind":"detail","role":"incoming","label":"Assistant","value":prose},
                {"id":"out","kind":"detail","role":"outgoing","label":"You","value":"A reply with 🙂 and café.\r\nA second line."},
                {"id":"draft","kind":"multiline","area":"composer","label":"Message","lines":3,"value":"draft café"},
            {"id":"attach","kind":"button","area":"composer","role":"quiet","label":"Attach","width":110,"row":"commands","row_align":"end","icon":"attachment"},
            {"id":"send","kind":"button","area":"composer","role":"primary","label":"Send","width":90,"row":"commands","icon":"send"}
            ]});
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert!(s.composer_fixed);
            assert!(s.fixed_rail);
            assert!(s.scroll > 0);
            assert_eq!(s.scroll, s.scroll_limit());
            assert!(s.background.footer_card.get().is_some());
            assert_eq!(
                GetWindowLongW(s.widgets["draft"].hwnd, GWL_STYLE) as u32 & WS_BORDER,
                0
            );
            assert_eq!(text(s.widgets["send"].hwnd), "Send");
            assert_eq!(s.widgets["send"].surface.icon.get(), Some(Icon::Send));
            let draft = s.widgets["draft"].hwnd;
            let first = s.widgets["first"].hwnd;
            SendMessageW(draft, EM_SETSEL, 3, 7);
            SendMessageW(first, EM_SETSEL, 5, 12);
            let composer_y = s.widgets["draft"].placed.unwrap().1;
            assert_eq!(s.widgets["attach"].rect.0, s.widgets["draft"].rect.0);
            assert_eq!(
                s.widgets["send"].rect.0 + s.widgets["send"].rect.2,
                s.widgets["draft"].rect.0 + s.widgets["draft"].rect.2
            );
            let rail_y = s.widgets["rail"].placed.unwrap().1;
            assert!(s.widgets["out"].rect.0 > s.widgets["first"].rect.0);
            assert!(s.widgets["first"].caption != 0);
            assert_eq!(text(s.widgets["first"].caption), "Assistant");
            let native_lines = SendMessageW(first, EM_GETLINECOUNT, 0, 0);
            assert!(native_lines > 10);
            assert!(
                s.widgets["first"].rect.3
                    >= native_lines as i32 * s.font_height(&s.widgets["first"])
            );
            s.scroll = 0;
            s.layout();
            assert_eq!(s.widgets["draft"].placed.unwrap().1, composer_y);
            assert_eq!(s.widgets["rail"].placed.unwrap().1, rail_y);
            assert_eq!(GetParent(first), s.content);
            assert_eq!(GetParent(s.content), s.viewport);
            let mut clipped: RECT = std::mem::zeroed();
            GetClientRect(s.viewport, &mut clipped);
            assert_eq!(clipped.bottom, s.viewport_height);
            frame["controls"].as_array_mut().unwrap().insert(3,json!({"id":"new","kind":"detail","role":"incoming","label":"Assistant","value":"A new message arriving below the reader."}));
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(
                s.scroll, 0,
                "Appending must not pull a history reader to the tail"
            );
            s.scroll = s.scroll_limit();
            s.layout();
            let old_limit = s.scroll_limit();
            frame["controls"].as_array_mut().unwrap().insert(4,json!({"id":"next","kind":"detail","role":"incoming","label":"Assistant","value":"Another message."}));
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert!(s.scroll_limit() > old_limit);
            assert_eq!(s.scroll, s.scroll_limit());
            s.scroll = 0;
            s.layout();
            frame["conversation"] = json!("two");
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(s.scroll, s.scroll_limit());
            frame["conversation"] = json!("history");
            frame["follow_tail"] = json!(false);
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(s.scroll, 0);
            // A bounded message window may drop the first visible item. Keep
            // the next surviving visible item's viewport position when feasible.
            for i in 0..20 {
                frame["controls"].as_array_mut().unwrap().insert(3+i,json!({"id":format!("anchor-{i}"),"kind":"detail","role":"incoming","label":"Assistant","value":"A short message in a bounded history window."}));
            }
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            s.scroll = s.widgets["anchor-3"].rect.1 + s.scale(8);
            s.layout();
            let anchor_y = s.widgets["anchor-4"].rect.1 - s.scroll;
            frame["controls"]
                .as_array_mut()
                .unwrap()
                .retain(|c| c["id"] != "anchor-3");
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(s.widgets["anchor-4"].rect.1 - s.scroll, anchor_y);
            let old_height = s.widgets["first"].rect.3;
            let before_resize = s.widgets["anchor-4"].rect.1 - s.scroll;
            MoveWindow(s.hwnd, 0, 0, s.scale(700), s.scale(650), 0);
            assert!(s.composer_fixed);
            assert!(s.widgets["first"].rect.3 > old_height);
            assert_eq!(s.widgets["anchor-4"].rect.1 - s.scroll, before_resize);
            assert_eq!(text(first), prose);
            let mut lo = 0u32;
            let mut hi = 0u32;
            SendMessageW(
                draft,
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (3, 7));
            SendMessageW(
                first,
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (5, 12));
            dark::MESSAGES.with(|m| m.set([0; 4]));
            SendMessageW(s.hwnd, WM_PAINT, 0, 0);
            SendMessageW(first, WM_PAINT, 0, 0);
            let messages = dark::MESSAGES.with(Cell::get);
            assert!(messages[0] >= 2);
            assert_eq!(messages[3], 0);
            MoveWindow(s.hwnd, 0, 0, s.scale(370), s.scale(280), 0);
            assert!(!s.composer_fixed);
            assert_ne!(GetWindowLongW(draft, GWL_STYLE) as u32 & WS_BORDER, 0);
            assert!(s.background.footer_rect.get().is_none());
            s.reveal(draft);
            assert!(s.scroll > 0);
            assert_eq!(s.widgets["draft"].hwnd, draft);
            SendMessageW(
                draft,
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (3, 7));
            assert_eq!(IsWindowVisible(s.hwnd), 0);
            destroy(s);
        }
    }
    #[test]
    fn hidden_scroll_message_probe() {
        unsafe {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut s = window(
                "hidden scroll probe",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            let mut controls = vec![
                json!({"id":"rail","kind":"button","role":"nav","side":true,"label":"Navigation"}),
            ];
            controls.extend((0..8).map(|i|json!({"id":format!("body-{i}"),"kind":"detail","role":"body","value":format!("Message {i}: editable selection remains native."),"lines":5})));
            s.apply(parse(json!({"theme":"dark","controls":controls})).unwrap())
                .unwrap();
            s.scroll = s.scale(200);
            s.layout();
            let before = s.scroll;
            let child = s.widgets["body-1"].hwnd;
            let wheel = (30u32 << 16) as usize;
            dark::MESSAGES.with(|m| m.set([0; 4]));
            SendMessageW(child, WM_MOUSEWHEEL, wheel, 0);
            let child_scroll = s.scroll;
            s.scroll = before;
            SendMessageW(s.hwnd, WM_MOUSEWHEEL, wheel, 0);
            let parent_scroll = s.scroll;
            s.scroll = s.scale(180);
            s.layout();
            let messages = dark::MESSAGES.with(Cell::get);
            eprintln!(
                "scroll_probe baseline={} child_wheel={} parent_wheel={} paint_messages={} erase_messages={} nonclient_messages={} paint_during_layout={}",
                before,
                child_scroll,
                parent_scroll,
                messages[0],
                messages[1],
                messages[2],
                messages[3]
            );
            assert_eq!(IsWindowVisible(s.hwnd), 0);
            assert!(parent_scroll < before);
            assert_eq!(
                child_scroll, parent_scroll,
                "Passive native bodies must route their wheel to the same outer viewport"
            );
            destroy(s);
        }
    }
    #[test]
    fn hidden_legacy_side_columns_and_compact_dark_navigation_remain_reachable() {
        unsafe {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut s = window(
                "hidden side layout",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            s.apply(parse(json!({"controls":[{"id":"side","kind":"button","label":"Project","side":true},{"id":"main","kind":"button","label":"Content"}]})).unwrap()).unwrap();
            assert!(!s.background.active.get());
            assert_eq!(s.background.rail_width.get(), 0);
            assert!(s.widgets["main"].rect.0 > s.widgets["side"].rect.0 + s.widgets["side"].rect.2);
            assert_eq!(s.widgets["main"].rect.1, s.widgets["side"].rect.1);
            MoveWindow(s.hwnd, 0, 0, s.scale(500), s.scale(400), 0);
            s.layout();
            assert_eq!(s.widgets["main"].rect.0, s.widgets["side"].rect.0);
            assert!(s.widgets["main"].rect.1 > s.widgets["side"].rect.1);
            let controls:Vec<_>=(0..12).map(|i|json!({"id":format!("nav-{i}"),"kind":"button","label":format!("Project {i}"),"role":"nav","side":true})).collect();
            s.apply(
                parse(json!({"theme":"dark","layout":"workspace","controls":controls})).unwrap(),
            )
            .unwrap();
            assert_eq!(s.widgets["nav-0"].rect.1, s.widgets["nav-1"].rect.1);
            let mut client: RECT = std::mem::zeroed();
            GetClientRect(s.hwnd, &mut client);
            for w in s.widgets.values() {
                assert!(w.rect.0 >= 0 && w.rect.0 + w.rect.2 <= client.right);
            }
            MoveWindow(s.hwnd, 0, 0, s.scale(1000), s.scale(400), 0);
            s.layout();
            assert!(!s.fixed_rail);
            s.scroll = 200;
            s.layout();
            assert!(s.widgets["nav-0"].placed.unwrap().1 < s.widgets["nav-0"].rect.1);
            destroy(s);
        }
    }
    #[test]
    fn hidden_dark_welcome_and_workspace_preserve_native_contracts_at_compact_sizes() {
        unsafe {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut s = window(
                "hidden dark lifecycle",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            MoveWindow(s.hwnd, 0, 0, s.scale(1200), s.scale(900), 0);
            let mut frame = json!({"theme":"dark","layout":"welcome","controls":[
                {"id":"brand","kind":"label","label":"Atlas","role":"brand"},
                {"id":"title","kind":"heading","label":"Keep your work in focus","role":"title"},
                {"id":"intro","kind":"label","label":"A quiet place for your projects.","role":"muted"},
                {"id":"folder","kind":"input","label":"Workspace folder","value":"C:\\Café"},
                {"id":"open","kind":"button","label":"Open workspace","role":"primary","width":180,"row":"actions","row_align":"center"},
                {"id":"help","kind":"button","label":"Learn more","role":"quiet","width":140,"row":"actions"}
            ]});
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(IsWindowVisible(s.hwnd), 0);
            let input = s.widgets["folder"].hwnd;
            assert_eq!(text(input), "C:\\Café");
            SendMessageW(input, EM_SETSEL, 3, 5);
            let mut client: RECT = std::mem::zeroed();
            GetClientRect(s.hwnd, &mut client);
            let (x, y, w, _) = s.widgets["brand"].rect;
            assert!(y > s.scale(64));
            assert!(w <= s.scale(560));
            assert!((client.right - w) / 2 - x <= 2);
            assert!(s.content_surface.panel_rect.get().is_some());
            assert_eq!(s.widgets["open"].rect.2, s.scale(180));
            assert!(s.widgets["open"].rect.0 > s.widgets["folder"].rect.0);
            // Resizing a hidden native parent exercises actual child geometry, without desktop interaction.
            MoveWindow(s.hwnd, 0, 0, s.scale(370), s.scale(280), 0);
            s.layout();
            GetClientRect(s.hwnd, &mut client);
            assert!(s.content_height > client.bottom);
            assert!(s.content_surface.panel_rect.get().is_none());
            assert!(s.widgets["help"].rect.1 > s.widgets["open"].rect.1);
            for w in s.widgets.values() {
                assert!(
                    w.rect.0 >= 0 && w.rect.0 + w.rect.2 <= client.right,
                    "Control escaped compact viewport: {:?}",
                    w.rect
                );
            }
            // Open-project transition retains keyed native field contents and selection.
            frame["layout"] = json!("workspace");
            frame["controls"].as_array_mut().unwrap().insert(0,json!({"id":"nav","kind":"button","label":"Current project","role":"selected","side":true,"expanded":true}));
            frame["controls"].as_array_mut().unwrap().push(json!({"id":"body","kind":"detail","role":"body","value":"Readable selectable prose.","lines":3}));
            frame["controls"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"check","kind":"check","label":"Watch changes","value":true}));
            frame["controls"].as_array_mut().unwrap().push(json!({"id":"combo","kind":"select","label":"Mode","choices":[{"label":"Native","value":"n"}],"value":"n"}));
            frame["controls"].as_array_mut().unwrap().push(json!({"id":"list","kind":"list","choices":[{"label":"One","value":"1"}],"value":"1"}));
            MoveWindow(s.hwnd, 0, 0, s.scale(1200), s.scale(900), 0);
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert_eq!(s.widgets["folder"].hwnd, input);
            assert!(s.background.rail_width.get() > 0);
            assert!(s.widgets["folder"].rect.0 > s.background.rail_width.get());
            let mut lo = 0u32;
            let mut hi = 0u32;
            SendMessageW(
                input,
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (3, 5));
            assert_eq!(
                GetWindowLongW(s.widgets["body"].hwnd, GWL_STYLE) as u32 & WS_BORDER,
                0
            );
            assert_ne!(
                GetWindowLongW(s.widgets["check"].hwnd, GWL_STYLE) & BS_AUTOCHECKBOX,
                0
            );
            assert_eq!(
                SendMessageW(s.widgets["check"].hwnd, BM_GETCHECK, 0, 0),
                BST_CHECKED as isize
            );
            assert_eq!(SendMessageW(s.widgets["combo"].hwnd, CB_GETCURSEL, 0, 0), 0);
            assert_eq!(SendMessageW(s.widgets["list"].hwnd, LB_GETCURSEL, 0, 0), 0);
            let body = frame["controls"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|c| c["id"] == "body")
                .unwrap();
            body["lines"] = json!(50);
            s.apply(parse(frame.clone()).unwrap()).unwrap();
            assert!(s.fixed_rail);
            let nav = s.widgets["nav"].hwnd;
            let nav_y = s.widgets["nav"].placed.unwrap().1;
            s.scroll = 200;
            s.layout();
            assert_eq!(s.widgets["nav"].placed.unwrap().1, nav_y);
            s.reveal(nav);
            assert_eq!(s.scroll, 200);
            // High contrast is simulated only at the renderer policy boundary; no user setting changes.
            s.refresh_theme(true, false);
            assert!(!s.background.active.get());
            assert!(!s.widgets["folder"].surface.active.get());
            assert_eq!(s.widgets["folder"].hwnd, input);
            SendMessageW(
                input,
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (3, 5));
            assert_ne!(
                GetWindowLongW(s.widgets["body"].hwnd, GWL_STYLE) as u32 & WS_BORDER,
                0
            );
            frame["theme"] = json!("system");
            s.apply(parse(frame).unwrap()).unwrap();
            assert!(!s.background.active.get());
            assert_eq!(text(input), "C:\\Café");
            assert!(!dark::enabled(Theme::Dark, true));
            assert!(!dark::enabled(Theme::System, false));
            destroy(s);
            assert_eq!(IsWindow(input), 0);
        }
    }
    #[test]
    fn coalescing_retains_one_shot_intent_without_acknowledgment_laundering() {
        let mut pending = Pending::default();
        pending.push(
            parse(json!({"ack":2,"focus":"a","selection":{"id":"a","start":4,"end":4}})).unwrap(),
        );
        pending.push(parse(json!({"ack":5,"focus":null,"selection":null})).unwrap());
        let frame = pending.frame.take().unwrap();
        assert_eq!(frame.ack, 5);
        assert_eq!(frame.focus.as_deref(), Some("a"));
        assert_eq!(frame.focus_ack, Some(2));
        assert_eq!(frame.selection_ack, Some(2));
        assert_eq!(frame.selection.unwrap().start, 4);
        pending.push(parse(json!({"ack":6})).unwrap());
        assert!(pending.frame.as_ref().unwrap().selection.is_none());
        assert_eq!(caret_offset("x😀y", 2), 1);
        assert_eq!(caret_offset("x😀y", u32::MAX), 4);
    }
    #[test]
    fn hidden_native_lifecycle_preserves_handles_edits_and_layout() {
        unsafe {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut s = window(
                "hidden lifecycle",
                tx,
                Arc::new(Mutex::new(Pending::default())),
            )
            .unwrap();
            assert_eq!(IsWindowVisible(s.hwnd), 0);
            let first = json!({"controls":[
                {"id":"a","kind":"input","label":"Name","value":"hello"},
                {"id":"b","kind":"multi","value":["y"],"choices":[{"label":"Yes","value":"y"}]},
                {"id":"c","kind":"list","choices":[{"label":"Path","value":"p"}],"value":"p","lines":30},
                {"id":"check","kind":"check","label":"Enabled","value":true},
                {"id":"select","kind":"select","label":"Choice","choices":[{"label":"Selected","value":"s"}],"value":"s"},
                {"id":"button","kind":"button","label":"Save & Close","enabled":false},
                {"id":"multiline","kind":"multiline","label":"Content","value":"one\r\ntwo"},
                {"id":"detail","kind":"detail","label":"Details","value":"Read only"},
                {"id":"label","kind":"label","label":"Caption"},
                {"id":"heading","kind":"heading","label":"Heading"}
            ]});
            let start = Instant::now();
            s.apply(parse(first.clone()).unwrap()).unwrap();
            let create = start.elapsed();
            let input = s.widgets["a"].hwnd;
            let list = s.widgets["b"].hwnd;
            assert_eq!(text(input), "hello");
            assert_eq!(SendMessageW(list, LB_GETSEL, 0, 0), 1);
            assert_eq!(
                SendMessageW(s.widgets["check"].hwnd, BM_GETCHECK, 0, 0),
                BST_CHECKED as isize
            );
            assert_eq!(
                SendMessageW(s.widgets["select"].hwnd, CB_GETCURSEL, 0, 0),
                0
            );
            assert_eq!(IsWindowEnabled(s.widgets["button"].hwnd), 0);
            assert_eq!(text(s.widgets["button"].hwnd), "Save && Close");
            assert_ne!(
                GetWindowLongW(s.widgets["label"].hwnd, GWL_STYLE) & STATIC_LITERAL as i32,
                0
            );
            assert_eq!(text(s.widgets["multiline"].hwnd), "one\r\ntwo");
            assert_ne!(
                GetWindowLongW(s.widgets["detail"].hwnd, GWL_STYLE) & ES_READONLY,
                0
            );
            assert_eq!(text(s.widgets["label"].hwnd), "Caption");
            SendMessageW(input, EM_SETSEL, 1, 3);
            let update = Instant::now();
            s.apply(parse(first.clone()).unwrap()).unwrap();
            let unchanged = update.elapsed();
            assert_eq!(s.widgets["a"].hwnd, input);
            assert_eq!(s.widgets["b"].hwnd, list);
            let mut lo = 0u32;
            let mut hi = 0u32;
            SendMessageW(
                input,
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (1, 3));
            // In-process content notification exercises state and UTF16 offsets; no desktop UI is driven.
            UPDATING.with(|x| x.set(true));
            set_text(input, "x😀y");
            UPDATING.with(|x| x.set(false));
            SendMessageW(input, EM_SETSEL, 3, 3);
            s.command(input, EN_CHANGE);
            let event = rx.try_recv().unwrap();
            assert_eq!(event["value"], "x😀y");
            assert_eq!(event["start"], 3);
            assert_eq!(event["serial"], 1);
            s.apply(parse(first.clone()).unwrap()).unwrap();
            assert_eq!(text(input), "x😀y");
            let mut acknowledged = first;
            acknowledged["ack"] = json!(1);
            s.apply(parse(acknowledged).unwrap()).unwrap();
            assert_eq!(text(input), "hello");
            assert!(s.content_height > 700);
            s.scroll = 200;
            s.layout();
            let scroll = s.scroll;
            let mut keep = json!({"ack":1,"controls":s.order.iter().map(|id|serde_json::to_value(&s.widgets[id].spec).unwrap()).collect::<Vec<_>>()});
            keep["title"] = json!("updated");
            s.apply(parse(keep).unwrap()).unwrap();
            assert_eq!(s.scroll, scroll);
            assert_eq!(text(s.hwnd), "updated");
            let mut requested = json!({"ack":1,"controls":s.order.iter().map(|id|serde_json::to_value(&s.widgets[id].spec).unwrap()).collect::<Vec<_>>(),"focus":"a","selection":{"id":"a","start":5,"end":5}});
            s.apply(parse(requested.clone()).unwrap()).unwrap();
            SendMessageW(
                input,
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (5, 5));
            {
                let _guard = UpdateGuard::enter();
                set_text(input, "x😀y");
            }
            SendMessageW(input, EM_SETSEL, 3, 3);
            s.command(input, EN_CHANGE);
            // A stale request remains stale when a newer pending frame acknowledges
            // this edit, even when that request also targets focus.
            let mut pending = Pending::default();
            pending.push(parse(requested.clone()).unwrap());
            let latest = json!({"ack":2,"controls":s.order.iter().map(|id|serde_json::to_value(&s.widgets[id].spec).unwrap()).collect::<Vec<_>>()});
            pending.push(parse(latest.clone()).unwrap());
            s.apply(pending.frame.take().unwrap()).unwrap();
            SendMessageW(
                input,
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (3, 3));
            requested = latest;
            requested["selection"] = json!({"id":"a","start":2,"end":u32::MAX});
            s.apply(parse(requested).unwrap()).unwrap();
            SendMessageW(
                input,
                EM_GETSEL,
                &mut lo as *mut _ as usize,
                &mut hi as *mut _ as isize,
            );
            assert_eq!((lo, hi), (1, 4));
            let layout = Instant::now();
            s.layout();
            let layout = layout.elapsed();
            eprintln!(
                "hidden native metrics create_us={} unchanged_update_us={} layout_us={}",
                create.as_micros(),
                unchanged.as_micros(),
                layout.as_micros()
            );
            destroy(s);
            assert_eq!(IsWindow(input), 0);
        }
    }
}
