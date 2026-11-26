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
    UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};
mod dark;

const FRAME: u32 = WM_APP + 17;
// SS_NOPREFIX is a STATIC style: labels are literal text, not mnemonics.
const STATIC_LITERAL: u32 = 0x80;
type Placement = (String, i32, i32, i32, i32, i32);
thread_local! { static UPDATING: Cell<bool> = const { Cell::new(false) }; }
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
}
impl Drop for Widget {
    fn drop(&mut self) {
        unsafe {
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
}
impl State {
    fn scale(&self, n: i32) -> i32 {
        ((n as i64 * self.dpi as i64) / 96) as i32
    }
    fn create(&self, spec: Control) -> Result<Widget> {
        unsafe {
            let (class, style) = match spec.kind {
                Kind::Label | Kind::Heading => ("STATIC", STATIC_LITERAL),
                Kind::Button => ("BUTTON", BS_PUSHBUTTON as u32 | WS_TABSTOP),
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
            let instance = GetModuleHandleW(null());
            let hwnd = CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                0,
                0,
                0,
                0,
                self.hwnd,
                0,
                instance,
                null(),
            );
            if hwnd == 0 {
                bail!("CreateWindowExW failed: {}", GetLastError())
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
                    WS_CHILD | WS_VISIBLE | STATIC_LITERAL,
                    0,
                    0,
                    0,
                    0,
                    self.hwnd,
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
            };
            self.update(&mut widget, spec, true, u64::MAX);
            Ok(widget)
        }
    }
    fn font(&self, c: &Control) -> HFONT {
        unsafe {
            let modern = self.theme == Theme::Dark;
            let logical = if modern {
                match c.role {
                    Role::Title => 34,
                    Role::Brand => 22,
                    Role::Caption => 12,
                    Role::Muted => 14,
                    Role::Body => 16,
                    _ => {
                        if c.kind == Kind::Heading {
                            21
                        } else {
                            15
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
                if matches!(
                    c.role,
                    Role::Brand | Role::Title | Role::Primary | Role::Selected
                ) || c.kind == Kind::Heading
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
            if w.surface.expanded.replace(c.expanded) != c.expanded {
                InvalidateRect(w.hwnd, null(), 0);
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
            if c.zoom != w.spec.zoom || c.role != w.spec.role {
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
            let mut order = Vec::new();
            for c in f.controls {
                order.push(c.id.clone());
                let mut old = self.widgets.remove(&c.id);
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
            // Maintain native tab order after keyed moves, without recreating controls.
            let mut after = HWND_TOP;
            if reordered {
                for id in &self.order {
                    let w = &self.widgets[id];
                    if w.caption != 0 {
                        SetWindowPos(
                            w.caption,
                            after,
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                        );
                        after = w.caption;
                    }
                    SetWindowPos(
                        w.hwnd,
                        after,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                    after = w.hwnd;
                }
            }
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
            let mut group_end = first + 1;
            if let Some(row) = self.widgets[&ids[first]].spec.row.as_ref() {
                while group_end < ids.len()
                    && self.widgets[&ids[group_end]].spec.row.as_ref() == Some(row)
                {
                    group_end += 1;
                }
            }
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
                    let fixed = explicit.or_else(|| {
                        compact.then(|| {
                            (self.text_width(w)
                                + self.scale(if c.expanded.is_some() { 54 } else { 34 }))
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
                let align = self.widgets[&ids[first]].spec.row_align;
                let mut x = left
                    + if flex > 0 {
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
                    let line = (self.scale(if modern && c.role == Role::Body {
                        25
                    } else {
                        22
                    }) as f64
                        * c.zoom)
                        .round() as i32;
                    let height = match c.kind {
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
                            let base = if modern {
                                match c.role {
                                    Role::Title => 46,
                                    Role::Brand => 42,
                                    Role::Caption => 20,
                                    Role::Muted => 23,
                                    _ => {
                                        if c.kind == Kind::Heading {
                                            30
                                        } else {
                                            24
                                        }
                                    }
                                }
                            } else if c.kind == Kind::Heading {
                                36
                            } else {
                                30
                            };
                            let lines = if c.role == Role::Brand {
                                1
                            } else {
                                ((self.text_width(&self.widgets[id]) + cw - 1) / cw.max(1))
                                    .clamp(1, 4)
                            };
                            self.scale(base) * lines
                        }
                        Kind::Button => self.scale(if modern {
                            if c.role == Role::Primary {
                                44
                            } else if c.role == Role::Quiet {
                                36
                            } else {
                                40
                            }
                        } else {
                            30
                        }),
                        _ => {
                            if modern {
                                self.scale(40)
                            } else {
                                line + self.scale(8)
                            }
                        }
                    };
                    let caption = if self.widgets[id].caption != 0 {
                        self.scale(if modern { 26 } else { 22 })
                    } else {
                        0
                    };
                    row_height = row_height.max(height + caption);
                    if modern {
                        row_gap = row_gap.max(self.scale(match c.role {
                            Role::Brand => 28,
                            Role::Title => 18,
                            Role::Caption => 4,
                            _ => 12,
                        }));
                    }
                    out.push((id.clone(), x, y, cw, height, caption));
                    x += cw + gap;
                }
                y += row_height + row_gap;
                begin = end;
            }
            first = group_end;
        }
        (out, y)
    }
    fn layout(&mut self) {
        self.layout_pass(true);
    }
    fn layout_pass(&mut self, reconcile_scrollbar: bool) {
        unsafe {
            let _guard = UpdateGuard::enter();
            let mut client: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut client);
            let modern = self.theme == Theme::Dark;
            let welcome = self.layout_mode == Some(Layout::Welcome);
            let pad = self
                .scale(if modern || welcome { 32 } else { 16 })
                .min((client.right / 6).max(self.scale(8)));
            let gap = self.scale(if modern { 12 } else { 8 });
            let rail = !welcome
                && self.order.iter().any(|id| self.widgets[id].spec.side)
                && client.right >= self.scale(if modern { 760 } else { 600 });
            let rail_width = if rail {
                if modern {
                    self.scale(248)
                } else {
                    self.scale(250).min((client.right - pad * 2) / 3)
                }
            } else {
                0
            };
            let painted_rail = if modern { rail_width } else { 0 };
            let previous_rail = self.background.rail_width.replace(painted_rail);
            let content_left = rail_width + pad + if rail && !modern { gap } else { 0 };
            let available = (client.right - content_left - pad).max(1);
            let cap = if modern || welcome {
                self.scale(self.max_width.unwrap_or(if welcome { 560 } else { 960 }) as i32)
            } else {
                available
            };
            let content_width = available.min(cap).max(1);
            let left = content_left + (available - content_width) / 2;
            let main_ids: Vec<_> = self
                .order
                .iter()
                .filter(|id| !rail || !self.widgets[*id].spec.side)
                .cloned()
                .collect();
            let (mut placements, mut bottom) =
                self.column(&main_ids, left, pad, content_width, gap, modern);
            if welcome {
                bottom = placements
                    .iter()
                    .map(|(_, _, y, _, h, caption)| y + h + caption)
                    .max()
                    .unwrap_or(pad);
                let height = bottom + pad;
                let offset = ((client.bottom - height) / 2).max(0);
                for (_, _, y, _, _, _) in &mut placements {
                    *y += offset;
                }
                bottom += offset;
            }
            if rail {
                let rail_ids: Vec<_> = self
                    .order
                    .iter()
                    .filter(|id| self.widgets[*id].spec.side)
                    .cloned()
                    .collect();
                let railpad = if modern { self.scale(20) } else { pad };
                let (mut items, rail_bottom) = self.column(
                    &rail_ids,
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
                bottom = bottom.max(rail_bottom);
                self.fixed_rail = modern && rail_bottom + pad <= client.bottom;
            } else {
                self.fixed_rail = false;
            }
            self.content_height = bottom + pad;
            self.scroll = self
                .scroll
                .clamp(0, (self.content_height - client.bottom).max(0));
            let panel = modern && welcome && client.right >= self.scale(720);
            let panel_rect = if panel {
                let top = placements
                    .iter()
                    .map(|(_, _, y, _, _, _)| *y)
                    .min()
                    .unwrap_or(pad)
                    - pad
                    - self.scroll;
                Some((
                    left - pad,
                    top,
                    left + content_width + pad,
                    bottom + pad - self.scroll,
                ))
            } else {
                None
            };
            let previous_panel = self.background.panel_rect.replace(panel_rect);
            let combo_extra = self.scale(220);
            for (id, x, y, width, height, caption) in placements {
                let w = self.widgets.get_mut(&id).unwrap();
                let side_surface = modern && rail && w.spec.side;
                if w.surface.side.replace(side_surface) != side_surface {
                    InvalidateRect(w.hwnd, null(), 1);
                }
                if w.surface.panel.replace(panel) != panel {
                    InvalidateRect(w.hwnd, null(), 1);
                }
                if let Some(caption) = &w.caption_surface
                    && caption.panel.replace(panel) != panel
                {
                    InvalidateRect(w.caption, null(), 1);
                }
                if let Some(caption) = &w.caption_surface
                    && caption.side.replace(side_surface) != side_surface
                {
                    InvalidateRect(w.caption, null(), 1);
                }
                w.rect = (x, y + caption, width, height);
                let native_height = if w.spec.kind == Kind::Select {
                    height + combo_extra
                } else {
                    height
                };
                let scroll = if self.fixed_rail && w.spec.side {
                    0
                } else {
                    self.scroll
                };
                let placement = (x, y - scroll, width, native_height, caption);
                if w.placed != Some(placement) {
                    if w.caption != 0 {
                        MoveWindow(w.caption, x, y - scroll, width, caption, 1);
                    }
                    MoveWindow(w.hwnd, x, y + caption - scroll, width, native_height, 1);
                    w.placed = Some(placement);
                }
            }
            let info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                nMin: 0,
                nMax: self.content_height - 1,
                nPage: client.bottom.max(0) as u32,
                nPos: self.scroll,
                nTrackPos: 0,
            };
            SetScrollInfo(self.hwnd, SB_VERT, &info, 1);
            if previous_rail != painted_rail || previous_panel != panel_rect {
                InvalidateRect(self.hwnd, null(), 1);
            }
            if reconcile_scrollbar {
                let mut after: RECT = std::mem::zeroed();
                GetClientRect(self.hwnd, &mut after);
                if after.right != client.right || after.bottom != client.bottom {
                    self.layout_pass(false);
                }
            }
        }
    }
    fn reveal(&mut self, hwnd: HWND) {
        unsafe {
            let Some(w) = self.widgets.values().find(|w| w.hwnd == hwnd) else {
                return;
            };
            if self.fixed_rail && w.spec.side {
                return;
            }
            let mut r = std::mem::zeroed::<RECT>();
            GetClientRect(self.hwnd, &mut r);
            let (_, y, _, h) = w.rect;
            if y < self.scroll {
                self.scroll = y.saturating_sub(self.scale(16));
            } else if y + h > self.scroll + r.bottom {
                self.scroll = (y + h - r.bottom + self.scale(16)).max(0);
            }
            self.layout();
        }
    }
    fn command(&mut self, hwnd: HWND, code: u32) {
        unsafe {
            let Some((id, w)) = self.widgets.iter_mut().find(|(_, w)| w.hwnd == hwnd) else {
                return;
            };
            let mut event = match w.spec.kind {
                Kind::Button if code == BN_CLICKED => json!({"type":"click","id":id}),
                Kind::Check if code == BN_CLICKED => {
                    json!({"type":"change","id":id,"value":SendMessageW(hwnd,BM_GETCHECK,0,0)==BST_CHECKED as isize})
                }
                Kind::Input | Kind::Multiline if code == EN_CHANGE => {
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
        }
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
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
        WM_CLOSE => {
            PostQuitMessage(0);
            0
        }
        WM_SIZE => {
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
            s.scroll = match (wp & 0xffff) as i32 {
                SB_LINEUP => s.scroll - s.scale(32),
                SB_LINEDOWN => s.scroll + s.scale(32),
                SB_PAGEUP => s.scroll - info.nPage as i32,
                SB_PAGEDOWN => s.scroll + info.nPage as i32,
                SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos,
                _ => s.scroll,
            }
            .clamp(0, (s.content_height - info.nPage as i32).max(0));
            s.layout();
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp >> 16) as u16 as i16) as i32;
            s.scroll = (s.scroll - delta * s.scale(48) / 120).max(0);
            s.layout();
            0
        }
        WM_DPICHANGED => {
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
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut *s as *mut State as isize);
        Ok(s)
    }
}
// Keep the allocation stable until its GWLP_USERDATA pointer is detached.
#[allow(clippy::boxed_local)]
fn destroy(mut s: Box<State>) {
    unsafe {
        SetWindowLongPtrW(s.hwnd, GWLP_USERDATA, 0);
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
                    let button = if ctrl {
                        s.default
                            .as_ref()
                            .and_then(|id| s.widgets.get(id))
                            .filter(|w| w.spec.kind == Kind::Button)
                            .map(|w| w.hwnd)
                    } else {
                        s.widgets
                            .values()
                            .find(|w| w.hwnd == GetFocus() && w.spec.kind == Kind::Button)
                            .map(|w| w.hwnd)
                    };
                    if let Some(button) = button {
                        if IsWindowEnabled(button) != 0 {
                            SendMessageW(button, BM_CLICK, 0, 0);
                        }
                        continue;
                    }
                }
                if (msg.wParam == VK_TAB as usize
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
    use std::time::Instant;
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
            assert!(s.background.panel_rect.get().is_some());
            assert_eq!(s.widgets["open"].rect.2, s.scale(180));
            assert!(s.widgets["open"].rect.0 > s.widgets["folder"].rect.0);
            // Resizing a hidden native parent exercises actual child geometry, without desktop interaction.
            MoveWindow(s.hwnd, 0, 0, s.scale(370), s.scale(280), 0);
            s.layout();
            GetClientRect(s.hwnd, &mut client);
            assert!(s.content_height > client.bottom);
            assert!(s.background.panel_rect.get().is_none());
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
