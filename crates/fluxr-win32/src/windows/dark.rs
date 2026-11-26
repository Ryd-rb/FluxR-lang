//! Optional painting around standard Win32 controls. Native classes retain all
//! input, IME, selection, scrolling, check-state and accessibility behavior.
use super::*;
use std::{cell::RefCell, rc::Rc, sync::OnceLock};
use windows_sys::Win32::{Graphics::Dwm::DwmSetWindowAttribute, UI::Accessibility::*};
#[cfg(test)]
thread_local! {pub(super) static MESSAGES:Cell<[u64;4]>=const{Cell::new([0;4])};}
#[cfg(test)]
thread_local! {pub(super) static EDIT_SIZES:Cell<u64>=const{Cell::new(0)};}
#[cfg(test)]
thread_local! {pub(super) static EDIT_PASSES:Cell<[u64;3]>=const{Cell::new([0;3])};}

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
const CLIENT: u32 = rgb(24, 27, 32);
const RAIL: u32 = rgb(17, 20, 24);
const FIELD: u32 = rgb(34, 39, 46);
const PANEL: u32 = rgb(29, 33, 39);
const HOVER: u32 = rgb(43, 50, 59);
const BORDER: u32 = rgb(54, 62, 72);
const TEXT: u32 = rgb(237, 240, 239);
const MUTED: u32 = rgb(151, 164, 178);
const DISABLED: u32 = rgb(113, 130, 141);
const ACCENT: u32 = rgb(62, 202, 177);
const PRIMARY: u32 = rgb(21, 117, 105);
const SELECTED: u32 = rgb(30, 55, 56);
fn key() -> *const u16 {
    static KEY: OnceLock<Vec<u16>> = OnceLock::new();
    KEY.get_or_init(|| wide("FluxR.NativeSurface.v1")).as_ptr()
}
pub(super) fn high_contrast() -> bool {
    unsafe {
        let mut hc: HIGHCONTRASTW = std::mem::zeroed();
        hc.cbSize = std::mem::size_of::<HIGHCONTRASTW>() as u32;
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            &mut hc as *mut _ as *mut _,
            0,
        ) != 0
            && hc.dwFlags & HCF_HIGHCONTRASTON != 0
    }
}
pub(super) fn enabled(theme: Theme, high_contrast: bool) -> bool {
    theme == Theme::Dark && !high_contrast
}
pub(super) struct Palette {
    client: HBRUSH,
    rail: HBRUSH,
    field: HBRUSH,
    selected: HBRUSH,
    panel: HBRUSH,
}
impl Palette {
    pub(super) fn new() -> Rc<Self> {
        unsafe {
            Rc::new(Self {
                client: CreateSolidBrush(CLIENT),
                rail: CreateSolidBrush(RAIL),
                field: CreateSolidBrush(FIELD),
                selected: CreateSolidBrush(SELECTED),
                panel: CreateSolidBrush(PANEL),
            })
        }
    }
}
impl Drop for Palette {
    fn drop(&mut self) {
        unsafe {
            for b in [
                self.client,
                self.rail,
                self.field,
                self.selected,
                self.panel,
            ] {
                DeleteObject(b);
            }
        }
    }
}
pub(super) struct Surface {
    hwnd: HWND,
    kind: Option<Kind>,
    palette: Rc<Palette>,
    pub(super) active: Cell<bool>,
    pub(super) host: Cell<bool>,
    role: Cell<Role>,
    pub(super) side: Cell<bool>,
    dpi: Cell<u32>,
    hover: Cell<bool>,
    pub(super) rail_width: Cell<i32>,
    pub(super) expanded: Cell<Option<bool>>,
    pub(super) panel: Cell<bool>,
    pub(super) panel_rect: Cell<Option<(i32, i32, i32, i32)>>,
    pub(super) footer_rect: Cell<Option<(i32, i32, i32, i32)>>,
    pub(super) footer_card: Cell<Option<(i32, i32, i32, i32)>>,
    pub(super) composer_focus: Cell<bool>,
    pub(super) header_rect: Cell<Option<(i32, i32, i32, i32)>>,
    pub(super) sections: RefCell<Vec<(i32, i32, i32, i32)>>,
    pub(super) tabs: RefCell<Vec<(i32, i32, i32, i32)>>,
    pub(super) selected: Cell<bool>,
    placeholder: RefCell<String>,
    pub(super) icon: Cell<Option<Icon>>,
    composer: Cell<bool>,
    pub(super) ime: Cell<bool>,
    overflow: Cell<bool>,
}
impl Surface {
    pub(super) fn attach(hwnd: HWND, kind: Option<Kind>, palette: Rc<Palette>) -> Box<Self> {
        unsafe {
            let surface = Box::new(Self {
                hwnd,
                kind,
                palette,
                active: Cell::new(false),
                host: Cell::new(false),
                role: Cell::new(Role::Neutral),
                side: Cell::new(false),
                dpi: Cell::new(96),
                hover: Cell::new(false),
                rail_width: Cell::new(0),
                expanded: Cell::new(None),
                panel: Cell::new(false),
                panel_rect: Cell::new(None),
                footer_rect: Cell::new(None),
                footer_card: Cell::new(None),
                composer_focus: Cell::new(false),
                header_rect: Cell::new(None),
                sections: RefCell::new(Vec::new()),
                tabs: RefCell::new(Vec::new()),
                selected: Cell::new(false),
                placeholder: RefCell::new(String::new()),
                icon: Cell::new(None),
                composer: Cell::new(false),
                ime: Cell::new(false),
                overflow: Cell::new(false),
            });
            let ptr = &*surface as *const Self as usize;
            SetPropW(hwnd, key(), ptr as isize);
            SetWindowSubclass(hwnd, Some(subclass), 1, ptr);
            surface
        }
    }
    pub(super) fn configure(&self, active: bool, role: Role, side: bool, dpi: u32) {
        unsafe {
            let changed = self.active.replace(active) != active
                || self.role.get() != role
                || self.side.get() != side
                || self.dpi.get() != dpi;
            self.role.set(role);
            self.side.set(side);
            self.dpi.set(dpi);
            if changed {
                // The single-line edit reserves custom nonclient padding; do
                // not let a themed inner edge paint a second, partial frame.
                let plain_edit = active
                    && matches!(
                        self.kind,
                        Some(Kind::Input | Kind::Multiline | Kind::Detail)
                    );
                let theme_name = wide(if plain_edit { "" } else { "DarkMode_Explorer" });
                let no_parts = wide("");
                SetWindowTheme(
                    self.hwnd,
                    if active { theme_name.as_ptr() } else { null() },
                    if plain_edit {
                        no_parts.as_ptr()
                    } else {
                        null()
                    },
                );
                if matches!(self.kind, Some(Kind::Detail)) {
                    let old = GetWindowLongW(self.hwnd, GWL_STYLE) as u32;
                    let body =
                        active && matches!(role, Role::Body | Role::Incoming | Role::Outgoing);
                    let style = if body {
                        old & !(WS_BORDER | WS_VSCROLL)
                            | if self.overflow.get() { WS_VSCROLL } else { 0 }
                    } else {
                        old | WS_BORDER | WS_VSCROLL
                    };
                    if old != style {
                        SetWindowLongW(self.hwnd, GWL_STYLE, style as i32);
                        SetWindowPos(
                            self.hwnd,
                            0,
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE
                                | SWP_NOSIZE
                                | SWP_NOZORDER
                                | SWP_NOACTIVATE
                                | SWP_FRAMECHANGED,
                        );
                    }
                }
                if matches!(
                    self.kind,
                    Some(Kind::Input | Kind::Multiline | Kind::Detail)
                ) {
                    let margin = if active && !matches!(role, Role::Body | Role::Incoming) {
                        self.scale(10)
                    } else {
                        0
                    };
                    SendMessageW(
                        self.hwnd,
                        EM_SETMARGINS,
                        (EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize,
                        (margin | (margin << 16)) as isize,
                    );
                }
                RedrawWindow(self.hwnd, null(), 0, RDW_INVALIDATE | RDW_FRAME | RDW_ERASE);
                if self.kind == Some(Kind::Input) {
                    SetWindowPos(
                        self.hwnd,
                        0,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    );
                }
                if active && self.kind == Some(Kind::Select) {
                    SendMessageW(
                        self.hwnd,
                        CB_SETITEMHEIGHT,
                        usize::MAX,
                        self.scale(30) as isize,
                    );
                    SendMessageW(self.hwnd, CB_SETITEMHEIGHT, 0, self.scale(26) as isize);
                }
                self.format_chat();
                self.composer_style();
            }
        }
    }
    pub(super) fn set_composer(&self, value: bool) {
        if self.composer.replace(value) != value {
            unsafe {
                self.composer_style();
                self.format_chat();
                InvalidateRect(self.hwnd, null(), 1);
            }
        }
    }
    unsafe fn composer_style(&self) {
        if matches!(self.kind, Some(Kind::Input | Kind::Multiline)) {
            let old = GetWindowLongW(self.hwnd, GWL_STYLE) as u32;
            let style = if self.active.get() && self.composer.get() {
                old & !(WS_BORDER | WS_VSCROLL)
            } else {
                old | WS_BORDER
                    | if self.kind == Some(Kind::Multiline) {
                        WS_VSCROLL
                    } else {
                        0
                    }
            };
            if old != style {
                SetWindowLongW(self.hwnd, GWL_STYLE, style as i32);
                SetWindowPos(
                    self.hwnd,
                    0,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE
                        | SWP_NOSIZE
                        | SWP_NOZORDER
                        | SWP_NOACTIVATE
                        | SWP_NOREDRAW
                        | SWP_FRAMECHANGED,
                );
            }
        }
    }
    pub(super) fn set_overflow(&self, overflow: bool) {
        unsafe {
            if self.overflow.replace(overflow) != overflow {
                let old = GetWindowLongW(self.hwnd, GWL_STYLE) as u32;
                let style = if overflow {
                    old | WS_VSCROLL
                } else {
                    old & !WS_VSCROLL
                };
                SetWindowLongW(self.hwnd, GWL_STYLE, style as i32);
                SetWindowPos(
                    self.hwnd,
                    0,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE
                        | SWP_NOSIZE
                        | SWP_NOZORDER
                        | SWP_NOACTIVATE
                        | SWP_NOREDRAW
                        | SWP_FRAMECHANGED,
                );
            }
        }
    }
    unsafe fn format_chat(&self) {
        if self.kind == Some(Kind::Detail)
            && matches!(self.role.get(), Role::Incoming | Role::Outgoing)
            || self.active.get() && self.composer.get() && self.kind == Some(Kind::Multiline)
        {
            let mut rect: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut rect);
            let pad = self.scale(if self.composer.get() {
                6
            } else if self.role.get() == Role::Outgoing {
                12
            } else {
                2
            });
            rect.left += pad;
            rect.top += pad;
            rect.right = (rect.right - pad).max(rect.left + 1);
            rect.bottom = (rect.bottom - pad).max(rect.top + 1);
            SendMessageW(self.hwnd, EM_SETRECTNP, 0, &rect as *const _ as isize);
        }
    }
    pub(super) fn set_placeholder(&self, value: &str) {
        if self.placeholder.borrow().as_str() != value {
            self.placeholder.replace(value.to_owned());
            unsafe {
                InvalidateRect(self.hwnd, null(), 0);
            }
        }
    }
    pub(super) fn show_placeholder(&self) -> bool {
        matches!(self.kind, Some(Kind::Input | Kind::Multiline))
            && !self.placeholder.borrow().is_empty()
            && !self.ime.get()
            && unsafe { GetWindowTextLengthW(self.hwnd) == 0 }
    }
    unsafe fn placeholder(&self, dc: HDC) {
        let mut rect: RECT = std::mem::zeroed();
        if self.kind == Some(Kind::Multiline) {
            SendMessageW(self.hwnd, EM_GETRECT, 0, &mut rect as *mut _ as isize);
        } else {
            GetClientRect(self.hwnd, &mut rect);
            let margins = SendMessageW(self.hwnd, EM_GETMARGINS, 0, 0) as u32;
            rect.left += (margins & 0xffff) as i32;
            rect.right -= (margins >> 16) as i32;
        }
        let old = SelectObject(dc, SendMessageW(self.hwnd, WM_GETFONT, 0, 0));
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(
            dc,
            if self.active.get() {
                MUTED
            } else {
                GetSysColor(COLOR_GRAYTEXT)
            },
        );
        draw_text(
            dc,
            &self.placeholder.borrow(),
            &mut rect,
            DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(dc, old);
    }
    unsafe fn edit_client(&self, dc: HDC) {
        let saved = SaveDC(dc);
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(self.hwnd, &mut rect);
        FillRect(dc, &rect, self.brush());
        self.colors(dc);
        #[cfg(test)]
        EDIT_PASSES.with(|p| {
            let mut n = p.get();
            n[0] += 1;
            n[1] += 1;
            p.set(n)
        });
        // Keep native text, selection, scrolling and IME rendering, but supply
        // a defined full-client backdrop and scoped DC state for every value.
        let native = SaveDC(dc);
        DefSubclassProc(self.hwnd, WM_PRINTCLIENT, dc as usize, PRF_CLIENT as isize);
        if native != 0 {
            RestoreDC(dc, native);
        }
        if self.show_placeholder() {
            #[cfg(test)]
            EDIT_PASSES.with(|p| {
                let mut n = p.get();
                n[2] += 1;
                p.set(n)
            });
            self.placeholder(dc);
        }
        if saved != 0 {
            RestoreDC(dc, saved);
        }
    }
    unsafe fn edit_window(&self, dc: HDC, flags: i32) {
        let saved = SaveDC(dc);
        self.colors(dc);
        let mut window: RECT = std::mem::zeroed();
        GetWindowRect(self.hwnd, &mut window);
        let mut origin = POINT { x: 0, y: 0 };
        ClientToScreen(self.hwnd, &mut origin);
        let (x, y) = (origin.x - window.left, origin.y - window.top);
        let (left, top) = (window.left, window.top);
        OffsetRect(&mut window, -left, -top);
        if flags & PRF_NONCLIENT != 0 {
            FillRect(dc, &window, self.brush());
        }
        DefSubclassProc(
            self.hwnd,
            WM_PRINT,
            dc as usize,
            (flags & !PRF_CLIENT & !PRF_ERASEBKGND) as isize,
        );
        OffsetViewportOrgEx(dc, x, y, std::ptr::null_mut());
        self.edit_client(dc);
        if saved != 0 {
            RestoreDC(dc, saved);
        }
        if flags & PRF_NONCLIENT != 0
            && GetWindowLongW(self.hwnd, GWL_STYLE) as u32 & WS_BORDER != 0
        {
            edit_outline_rect(self, dc);
        }
    }
    pub(super) unsafe fn composer_overflow(&self) {
        if !self.active.get() || !self.composer.get() || self.kind != Some(Kind::Multiline) {
            return;
        }
        let mut rect: RECT = std::mem::zeroed();
        SendMessageW(self.hwnd, EM_GETRECT, 0, &mut rect as *mut _ as isize);
        let dc = GetDC(self.hwnd);
        let old = SelectObject(dc, SendMessageW(self.hwnd, WM_GETFONT, 0, 0));
        let mut metrics: TEXTMETRICW = std::mem::zeroed();
        GetTextMetricsW(dc, &mut metrics);
        SelectObject(dc, old);
        ReleaseDC(self.hwnd, dc);
        let height = metrics.tmHeight.max(1);
        let overflow =
            SendMessageW(self.hwnd, EM_GETLINECOUNT, 0, 0) as i32 * height > rect.bottom - rect.top;
        let visible = GetWindowLongW(self.hwnd, GWL_STYLE) as u32 & WS_VSCROLL != 0;
        if overflow != visible {
            ShowScrollBar(self.hwnd, SB_VERT, i32::from(overflow));
            self.format_chat();
        }
    }
    fn scale(&self, n: i32) -> i32 {
        n * self.dpi.get() as i32 / 96
    }
    fn background(&self) -> u32 {
        if self.composer.get() || self.role.get() == Role::Tab {
            PANEL
        } else if self.side.get() {
            RAIL
        } else if self.panel.get() {
            PANEL
        } else {
            CLIENT
        }
    }
    fn brush(&self) -> HBRUSH {
        if self.composer.get() {
            return self.palette.panel;
        }
        if matches!(
            self.kind,
            Some(Kind::Input | Kind::Multiline | Kind::Select | Kind::List | Kind::Multi)
        ) || self.kind == Some(Kind::Detail)
            && !matches!(self.role.get(), Role::Body | Role::Incoming)
        {
            self.palette.field
        } else if self.side.get() {
            self.palette.rail
        } else if self.panel.get() {
            self.palette.panel
        } else {
            self.palette.client
        }
    }
    unsafe fn colors(&self, dc: HDC) -> HBRUSH {
        SetTextColor(
            dc,
            if IsWindowEnabled(self.hwnd) == 0 {
                DISABLED
            } else if matches!(self.role.get(), Role::Muted | Role::Caption) {
                MUTED
            } else {
                TEXT
            },
        );
        SetBkColor(
            dc,
            if self.brush() == self.palette.field {
                FIELD
            } else {
                self.background()
            },
        );
        self.brush()
    }
    unsafe fn root(&self, dc: HDC) {
        let mut r: RECT = std::mem::zeroed();
        GetClientRect(self.hwnd, &mut r);
        if !self.active.get() {
            FillRect(dc, &r, GetSysColorBrush(COLOR_WINDOW));
            return;
        }
        FillRect(dc, &r, self.palette.client);
        for (rects, radius) in [(&self.sections, 16), (&self.tabs, 12)] {
            for &(l, t, right, bottom) in rects.borrow().iter() {
                let pen = CreatePen(PS_SOLID, 1, BORDER);
                let old_pen = SelectObject(dc, pen);
                let old_brush = SelectObject(dc, self.palette.panel);
                RoundRect(
                    dc,
                    l,
                    t,
                    right,
                    bottom,
                    self.scale(radius),
                    self.scale(radius),
                );
                SelectObject(dc, old_brush);
                SelectObject(dc, old_pen);
                DeleteObject(pen);
            }
        }
        if let Some((left, top, right, bottom)) = self.panel_rect.get() {
            let pen = CreatePen(PS_SOLID, 1, BORDER);
            let oldpen = SelectObject(dc, pen);
            let oldbrush = SelectObject(dc, self.palette.panel);
            let radius = self.scale(16);
            RoundRect(dc, left, top, right, bottom, radius, radius);
            SelectObject(dc, oldbrush);
            SelectObject(dc, oldpen);
            DeleteObject(pen);
        }
        if let Some((left, top, right, bottom)) = self.footer_rect.get() {
            let rect = RECT {
                left,
                top,
                right,
                bottom,
            };
            FillRect(dc, &rect, self.palette.client);
        }
        if let Some((left, top, right, bottom)) = self.header_rect.get() {
            let rect = RECT {
                left,
                top,
                right,
                bottom,
            };
            FillRect(dc, &rect, self.palette.client);
            let separator = RECT {
                top: bottom - 1,
                ..rect
            };
            fill(dc, &separator, BORDER);
        }
        if let Some((left, top, right, bottom)) = self.footer_card.get() {
            let pen = CreatePen(
                PS_SOLID,
                self.scale(1).max(1),
                if self.composer_focus.get() {
                    ACCENT
                } else {
                    BORDER
                },
            );
            let oldpen = SelectObject(dc, pen);
            let oldbrush = SelectObject(dc, self.palette.panel);
            let radius = self.scale(20);
            RoundRect(dc, left, top, right, bottom, radius, radius);
            SelectObject(dc, oldbrush);
            SelectObject(dc, oldpen);
            DeleteObject(pen);
        }
        if self.rail_width.get() > 0 {
            r.right = self.rail_width.get();
            FillRect(dc, &r, self.palette.rail);
            r.left = r.right - 1;
            fill(dc, &r, BORDER);
        }
    }
    unsafe fn paint(&self, dc: HDC) {
        let mut r: RECT = std::mem::zeroed();
        GetClientRect(self.hwnd, &mut r);
        if self.kind.is_none() {
            self.root(dc);
            return;
        }
        let font = SendMessageW(self.hwnd, WM_GETFONT, 0, 0);
        let old = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        let disabled = IsWindowEnabled(self.hwnd) == 0;
        let focus = GetFocus() == self.hwnd;
        let mut content = r;
        if matches!(self.kind, Some(Kind::List | Kind::Multi)) {
            FillRect(dc, &r, self.palette.field);
            let count = SendMessageW(self.hwnd, LB_GETCOUNT, 0, 0).max(0);
            let top = SendMessageW(self.hwnd, LB_GETTOPINDEX, 0, 0).max(0);
            let caret = SendMessageW(self.hwnd, LB_GETCARETINDEX, 0, 0);
            let horizontal = GetScrollPos(self.hwnd, SB_HORZ);
            for i in top..count {
                let mut item: RECT = std::mem::zeroed();
                if SendMessageW(
                    self.hwnd,
                    LB_GETITEMRECT,
                    i as usize,
                    &mut item as *mut _ as isize,
                ) < 0
                    || item.top >= r.bottom
                {
                    break;
                }
                let selected = if self.kind == Some(Kind::Multi) {
                    SendMessageW(self.hwnd, LB_GETSEL, i as usize, 0) > 0
                } else {
                    SendMessageW(self.hwnd, LB_GETCURSEL, 0, 0) == i
                };
                if selected {
                    FillRect(dc, &item, self.palette.selected);
                }
                SetTextColor(dc, if disabled { DISABLED } else { TEXT });
                let len = SendMessageW(self.hwnd, LB_GETTEXTLEN, i as usize, 0).max(0) as usize;
                let mut buf = vec![0u16; len + 1];
                SendMessageW(self.hwnd, LB_GETTEXT, i as usize, buf.as_mut_ptr() as isize);
                let mut label = item;
                label.left += self.scale(10) - horizontal;
                label.right += horizontal;
                DrawTextW(
                    dc,
                    buf.as_ptr(),
                    len as i32,
                    &mut label,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                );
                if focus && caret == i {
                    DrawFocusRect(dc, &item);
                }
            }
        } else if self.kind == Some(Kind::Select) {
            FillRect(dc, &r, self.palette.field);
            let mut label = r;
            label.left += self.scale(12);
            label.right -= self.scale(30);
            SetTextColor(dc, if disabled { DISABLED } else { TEXT });
            draw_text(
                dc,
                &text(self.hwnd),
                &mut label,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            let x = r.right - self.scale(17);
            let y = (r.bottom + r.top) / 2;
            let d = self.scale(4);
            let pen = CreatePen(PS_SOLID, 1, if disabled { DISABLED } else { MUTED });
            let saved = SelectObject(dc, pen);
            MoveToEx(dc, x - d, y - d / 2, std::ptr::null_mut());
            LineTo(dc, x, y + d / 2);
            LineTo(dc, x + d, y - d / 2);
            SelectObject(dc, saved);
            DeleteObject(pen);
            border(dc, &r, if focus { ACCENT } else { BORDER });
        } else if self.kind == Some(Kind::Check) {
            fill(dc, &r, self.background());
            let side = self.scale(18);
            let cy = (r.bottom - side) / 2;
            let boxr = RECT {
                left: self.scale(2),
                top: cy,
                right: self.scale(2) + side,
                bottom: cy + side,
            };
            let checked = SendMessageW(self.hwnd, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
            fill(dc, &boxr, if checked { PRIMARY } else { FIELD });
            border(dc, &boxr, if focus { ACCENT } else { BORDER });
            if checked {
                let pen = CreatePen(PS_SOLID, self.scale(2).max(1), TEXT);
                let saved = SelectObject(dc, pen);
                MoveToEx(
                    dc,
                    boxr.left + side / 4,
                    cy + side / 2,
                    std::ptr::null_mut(),
                );
                LineTo(dc, boxr.left + side * 2 / 5, cy + side * 3 / 4);
                LineTo(dc, boxr.left + side * 4 / 5, cy + side / 4);
                SelectObject(dc, saved);
                DeleteObject(pen);
            }
            content.left += side + self.scale(12);
            SetTextColor(dc, if disabled { DISABLED } else { TEXT });
            draw_text(
                dc,
                &text(self.hwnd).replace("&&", "&"),
                &mut content,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            if focus {
                DrawFocusRect(dc, &content);
            }
        } else if self.kind == Some(Kind::Button) {
            let role = self.role.get();
            let tab = role == Role::Tab;
            let selected_container =
                role == Role::Selected && self.icon.get() == Some(Icon::Folder);
            let pressed = SendMessageW(self.hwnd, BM_GETSTATE, 0, 0) & BST_PUSHED as isize != 0;
            let bg = if disabled {
                FIELD
            } else if pressed {
                SELECTED
            } else if role == Role::Primary {
                PRIMARY
            } else if tab && self.selected.get() {
                FIELD
            } else if selected_container {
                if self.hover.get() {
                    HOVER
                } else {
                    self.background()
                }
            } else if role == Role::Selected {
                SELECTED
            } else if self.hover.get() {
                HOVER
            } else if matches!(role, Role::Quiet | Role::Nav | Role::Tab) {
                self.background()
            } else {
                FIELD
            };
            fill(dc, &r, self.background());
            let pen = CreatePen(
                PS_SOLID,
                1,
                if focus {
                    ACCENT
                } else if matches!(role, Role::Quiet | Role::Nav | Role::Selected | Role::Tab) {
                    bg
                } else {
                    BORDER
                },
            );
            let brush = CreateSolidBrush(bg);
            let oldpen = SelectObject(dc, pen);
            let oldbrush = SelectObject(dc, brush);
            let radius = self.scale(10);
            RoundRect(dc, r.left, r.top, r.right, r.bottom, radius, radius);
            SelectObject(dc, oldbrush);
            SelectObject(dc, oldpen);
            DeleteObject(brush);
            DeleteObject(pen);
            if role == Role::Selected && !selected_container {
                let indicator = RECT {
                    left: r.left,
                    top: r.top + self.scale(10),
                    right: r.left + self.scale(3),
                    bottom: r.bottom - self.scale(10),
                };
                fill(dc, &indicator, ACCENT);
            }
            if tab && self.selected.get() {
                let indicator = RECT {
                    left: r.left + self.scale(12),
                    right: r.right - self.scale(12),
                    top: r.bottom - self.scale(3),
                    bottom: r.bottom - self.scale(1),
                };
                fill(dc, &indicator, ACCENT);
            }
            SetTextColor(
                dc,
                if disabled {
                    DISABLED
                } else if role == Role::Selected {
                    ACCENT
                } else {
                    TEXT
                },
            );
            content.left += self.scale(14);
            content.right -= self.scale(14);
            if let Some(expanded) = self.expanded.get() {
                let x = content.left + self.scale(3);
                let y = (r.top + r.bottom) / 2;
                let d = self.scale(3);
                let pen = CreatePen(PS_SOLID, self.scale(2).max(1), MUTED);
                let saved = SelectObject(dc, pen);
                if expanded {
                    MoveToEx(dc, x - d, y - d / 2, std::ptr::null_mut());
                    LineTo(dc, x, y + d);
                    LineTo(dc, x + d, y - d / 2);
                } else {
                    MoveToEx(dc, x - d / 2, y - d, std::ptr::null_mut());
                    LineTo(dc, x + d, y);
                    LineTo(dc, x - d / 2, y + d);
                }
                SelectObject(dc, saved);
                DeleteObject(pen);
                content.left += self.scale(18);
            }
            if let Some(icon) = self.icon.get() {
                draw_icon(
                    dc,
                    icon,
                    content.left,
                    (r.top + r.bottom - self.scale(16)) / 2,
                    self.scale(16),
                    if disabled {
                        DISABLED
                    } else if role == Role::Selected {
                        ACCENT
                    } else {
                        TEXT
                    },
                );
                content.left += self.scale(24);
            }
            draw_text(
                dc,
                &text(self.hwnd).replace("&&", "&"),
                &mut content,
                DT_VCENTER
                    | DT_SINGLELINE
                    | DT_END_ELLIPSIS
                    | if matches!(role, Role::Nav | Role::Selected) {
                        DT_LEFT
                    } else {
                        DT_CENTER
                    },
            );
            if focus {
                let mut f = r;
                InflateRect(&mut f, -self.scale(4), -self.scale(4));
                DrawFocusRect(dc, &f);
            }
        } else if self.role.get() == Role::Brand {
            fill(dc, &r, self.background());
            let side = self.scale(36).min(r.bottom);
            let tile = RECT {
                left: 0,
                top: (r.bottom - side) / 2,
                right: side,
                bottom: (r.bottom + side) / 2,
            };
            fill(dc, &tile, PRIMARY);
            let label = text(self.hwnd);
            let letter = label
                .chars()
                .find(|c| c.is_alphanumeric())
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "·".into());
            let mut letterr = tile;
            SetTextColor(dc, TEXT);
            draw_text(
                dc,
                &letter,
                &mut letterr,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            content.left = side + self.scale(12);
            draw_text(
                dc,
                &label,
                &mut content,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        }
        SelectObject(dc, old);
    }
}
unsafe fn draw_icon(dc: HDC, icon: Icon, x: i32, y: i32, size: i32, color: u32) {
    let pen = CreatePen(PS_SOLID, ((size + 8) / 16).max(1), color);
    let oldpen = SelectObject(dc, pen);
    let oldbrush = SelectObject(dc, GetStockObject(NULL_BRUSH));
    let px = |n: i32| x + (n * size + 8) / 16;
    let py = |n: i32| y + (n * size + 8) / 16;
    let path = |nodes: &[(i32, i32)]| {
        let points: Vec<POINT> = nodes
            .iter()
            .map(|(a, b)| POINT {
                x: px(*a),
                y: py(*b),
            })
            .collect();
        Polyline(dc, points.as_ptr(), points.len() as i32);
    };
    match icon {
        Icon::Chat => {
            RoundRect(dc, px(1), py(2), px(15), py(12), size / 4, size / 4);
            path(&[(4, 12), (4, 15), (8, 12)]);
        }
        Icon::Activity => path(&[(0, 9), (3, 9), (5, 3), (8, 14), (11, 6), (13, 9), (16, 9)]),
        Icon::Graph => {
            path(&[(4, 4), (12, 5), (8, 12), (4, 4)]);
            for (a, b) in [(4, 4), (12, 5), (8, 12)] {
                Ellipse(dc, px(a - 2), py(b - 2), px(a + 2), py(b + 2));
            }
        }
        Icon::Models => {
            path(&[(1, 5), (8, 1), (15, 5), (8, 9), (1, 5)]);
            path(&[(1, 8), (8, 12), (15, 8)]);
            path(&[(1, 11), (8, 15), (15, 11)]);
        }
        Icon::Dictionary => {
            path(&[
                (8, 4),
                (5, 2),
                (1, 2),
                (1, 13),
                (5, 13),
                (8, 15),
                (11, 13),
                (15, 13),
                (15, 2),
                (11, 2),
                (8, 4),
                (8, 15),
            ]);
        }
        Icon::Tools => {
            path(&[
                (10, 1),
                (9, 5),
                (12, 8),
                (16, 7),
                (13, 10),
                (10, 10),
                (4, 16),
                (1, 13),
                (7, 7),
                (7, 4),
                (10, 1),
            ]);
        }
        Icon::Folder => path(&[(1, 14), (1, 3), (6, 3), (8, 5), (15, 5), (15, 14), (1, 14)]),
        Icon::Add => {
            path(&[(8, 2), (8, 14)]);
            path(&[(2, 8), (14, 8)]);
        }
        Icon::Attachment => path(&[
            (5, 12),
            (3, 10),
            (3, 5),
            (6, 2),
            (9, 2),
            (12, 5),
            (12, 11),
            (9, 14),
            (6, 14),
            (5, 13),
            (5, 6),
            (7, 4),
            (9, 6),
            (9, 10),
            (7, 12),
        ]),
        Icon::Send => {
            path(&[(1, 2), (15, 8), (1, 14), (4, 8), (1, 2)]);
            path(&[(4, 8), (15, 8)]);
        }
    }
    SelectObject(dc, oldbrush);
    SelectObject(dc, oldpen);
    DeleteObject(pen);
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.hwnd) != 0 {
                RemoveWindowSubclass(self.hwnd, Some(subclass), 1);
                RemovePropW(self.hwnd, key());
            }
        }
    }
}
unsafe fn surface(hwnd: HWND) -> Option<&'static Surface> {
    let ptr = GetPropW(hwnd, key()) as *const Surface;
    if ptr.is_null() { None } else { Some(&*ptr) }
}
unsafe fn fill(dc: HDC, r: &RECT, color: u32) {
    let b = CreateSolidBrush(color);
    FillRect(dc, r, b);
    DeleteObject(b);
}
unsafe fn border(dc: HDC, r: &RECT, color: u32) {
    let b = CreateSolidBrush(color);
    FrameRect(dc, r, b);
    DeleteObject(b);
}
unsafe fn draw_text(dc: HDC, s: &str, r: &mut RECT, flags: u32) {
    let value = wide(s);
    DrawTextW(
        dc,
        value.as_ptr(),
        value.len() as i32 - 1,
        r,
        flags | DT_NOPREFIX,
    );
}
unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _id: usize,
    data: usize,
) -> isize {
    #[cfg(test)]
    MESSAGES.with(|m| {
        let mut n = m.get();
        if msg == WM_PAINT {
            n[0] += 1;
            if UPDATING.with(Cell::get) {
                n[3] += 1;
            }
        } else if msg == WM_ERASEBKGND {
            n[1] += 1;
        } else if msg == WM_NCPAINT {
            n[2] += 1;
        }
        m.set(n);
    });
    let s = &*(data as *const Surface);
    let participant = matches!(
        s.kind,
        None | Some(Kind::Detail | Kind::Label | Kind::Heading)
    );
    if touch::route(hwnd, msg, wp, lp, participant)
        && !matches!(
            msg,
            WM_NCDESTROY | WM_CANCELMODE | WM_ACTIVATEAPP | WM_CAPTURECHANGED
        )
    {
        return 0;
    }
    #[cfg(test)]
    if msg == WM_SIZE && matches!(s.kind, Some(Kind::Input | Kind::Multiline | Kind::Detail)) {
        EDIT_SIZES.with(|n| n.set(n.get() + 1));
    }
    if matches!(msg, WM_SETFOCUS | WM_KILLFOCUS)
        && s.composer.get()
        && let Some(parent) = surface(GetParent(hwnd))
    {
        parent
            .composer_focus
            .set(msg == WM_SETFOCUS || surface(wp as HWND).is_some_and(|next| next.composer.get()));
        InvalidateRect(parent.hwnd, null(), 0);
    }
    if msg == WM_IME_STARTCOMPOSITION {
        s.ime.set(true);
        if !s.placeholder.borrow().is_empty() {
            InvalidateRect(hwnd, null(), 0);
        }
    } else if msg == WM_IME_ENDCOMPOSITION {
        s.ime.set(false);
        if !s.placeholder.borrow().is_empty() {
            InvalidateRect(hwnd, null(), 0);
        }
    }
    if msg == WM_NCDESTROY {
        RemovePropW(hwnd, key());
        RemoveWindowSubclass(hwnd, Some(subclass), 1);
        return DefSubclassProc(hwnd, msg, wp, lp);
    }
    if msg == WM_MOUSEWHEEL && s.kind.is_some() && s.kind != Some(Kind::Select) {
        let delta = ((wp >> 16) as u16 as i16) as i32;
        let mut info: SCROLLINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SCROLLINFO>() as u32;
        info.fMask = SIF_RANGE | SIF_PAGE | SIF_POS;
        let can_scroll = GetScrollInfo(hwnd, SB_VERT, &mut info) != 0
            && if delta > 0 {
                info.nPos > info.nMin
            } else {
                info.nPos < (info.nMax - info.nPage.saturating_sub(1) as i32).max(info.nMin)
            };
        let transcript = self_kind_body(s) && !s.overflow.get();
        if transcript || !can_scroll {
            let parent = GetParent(hwnd);
            if parent != 0 {
                return forward_wheel(parent, wp, lp);
            }
        }
    }
    if matches!(msg, WM_SIZE | WM_SETFONT)
        && (s.kind == Some(Kind::Detail) && matches!(s.role.get(), Role::Incoming | Role::Outgoing)
            || s.composer.get() && s.kind == Some(Kind::Multiline))
    {
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        s.format_chat();
        s.composer_overflow();
        return result;
    }
    let dark_edit = s.active.get() && matches!(s.kind, Some(Kind::Input | Kind::Multiline));
    if dark_edit && msg == WM_ERASEBKGND {
        return 1;
    }
    if dark_edit && msg == WM_PAINT {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        if dc != 0 {
            s.edit_client(dc);
        }
        EndPaint(hwnd, &ps);
        return 0;
    }
    if dark_edit && msg == WM_PRINTCLIENT {
        if wp != 0 {
            s.edit_client(wp as HDC);
        }
        return 0;
    }
    if dark_edit && msg == WM_PRINT && lp as i32 & PRF_CLIENT != 0 {
        if wp != 0 {
            s.edit_window(wp as HDC, lp as i32);
        }
        return 0;
    }
    if msg == WM_PAINT && s.show_placeholder() {
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        let dc = GetDC(hwnd);
        let saved = SaveDC(dc);
        HideCaret(hwnd);
        s.placeholder(dc);
        ShowCaret(hwnd);
        if saved != 0 {
            RestoreDC(dc, saved);
        }
        ReleaseDC(hwnd, dc);
        return result;
    }
    if matches!(
        msg,
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN
    ) {
        if let Some(child) = surface(lp)
            && child.active.get()
        {
            return child.colors(wp as HDC);
        }
        if s.active.get() && s.kind == Some(Kind::Select) {
            return s.colors(wp as HDC);
        }
    }
    let edit_outline = matches!(msg, WM_PRINT | WM_NCPAINT)
        && matches!(s.kind, Some(Kind::Input | Kind::Multiline | Kind::Detail))
        && GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_BORDER != 0;
    if msg == WM_PRINT && lp as i32 & PRF_NONCLIENT != 0 && edit_outline {
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        edit_outline_rect(s, wp as HDC);
        return result;
    }
    if msg == WM_NCPAINT && !s.active.get() && edit_outline {
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        let dc = GetWindowDC(hwnd);
        edit_outline_rect(s, dc);
        ReleaseDC(hwnd, dc);
        return result;
    }
    if s.active.get() || s.host.get() {
        if msg == WM_NCCALCSIZE && s.kind == Some(Kind::Input) {
            let result = DefSubclassProc(hwnd, msg, wp, lp);
            let rect = if wp != 0 {
                &mut (*(lp as *mut NCCALCSIZE_PARAMS)).rgrc[0]
            } else {
                &mut *(lp as *mut RECT)
            };
            let dc = GetDC(hwnd);
            let font = SendMessageW(hwnd, WM_GETFONT, 0, 0);
            let old = SelectObject(dc, font);
            let mut metrics: TEXTMETRICW = std::mem::zeroed();
            GetTextMetricsW(dc, &mut metrics);
            SelectObject(dc, old);
            ReleaseDC(hwnd, dc);
            let margin = ((rect.bottom - rect.top - metrics.tmHeight) / 2).max(0);
            rect.top += margin;
            rect.bottom -= margin;
            return result;
        }
        if msg == WM_SETFONT && matches!(s.kind, Some(Kind::Input | Kind::Select)) {
            let result = DefSubclassProc(hwnd, msg, wp, lp);
            if s.kind == Some(Kind::Input) {
                SetWindowPos(
                    hwnd,
                    0,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            } else {
                SendMessageW(hwnd, CB_SETITEMHEIGHT, usize::MAX, s.scale(30) as isize);
                SendMessageW(hwnd, CB_SETITEMHEIGHT, 0, s.scale(26) as isize);
            }
            return result;
        }
        let custom = s.kind.is_none()
            || matches!(
                s.kind,
                Some(Kind::Button | Kind::Check | Kind::Select | Kind::List | Kind::Multi)
            )
            || s.role.get() == Role::Brand;
        if msg == WM_ERASEBKGND && custom {
            // WM_PAINT supplies the complete clipped background. Painting it
            // again during erase exposes an intermediate blank surface.
            return 1;
        }
        if msg == WM_PAINT && custom {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            s.paint(dc);
            EndPaint(hwnd, &ps);
            return 0;
        }
        if msg == WM_MOUSEMOVE && !s.hover.replace(true) {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut track);
            InvalidateRect(hwnd, null(), 0);
        }
        if msg == WM_MOUSELEAVE {
            s.hover.set(false);
            InvalidateRect(hwnd, null(), 0);
        }
        if matches!(
            msg,
            WM_SETFOCUS | WM_KILLFOCUS | WM_ENABLE | BM_SETSTATE | BM_SETCHECK
        ) {
            let result = DefSubclassProc(hwnd, msg, wp, lp);
            RedrawWindow(hwnd, null(), 0, RDW_INVALIDATE | RDW_FRAME);
            return result;
        }
        if msg == WM_NCPAINT
            && matches!(
                s.kind,
                Some(Kind::Input | Kind::Multiline | Kind::Detail | Kind::List | Kind::Multi)
            )
        {
            if self_kind_body(s) && !s.overflow.get() {
                // A borderless prose control has no nonclient decoration.
                // Avoid native theme fallback drawing an input-like edge.
                return 0;
            }
            let result = DefSubclassProc(hwnd, msg, wp, lp);
            if s.kind == Some(Kind::Input)
                || GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_BORDER != 0
            {
                let dc = GetWindowDC(hwnd);
                let mut r: RECT = std::mem::zeroed();
                GetWindowRect(hwnd, &mut r);
                let mut origin = POINT { x: 0, y: 0 };
                ClientToScreen(hwnd, &mut origin);
                let mut client: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut client);
                let client_left = origin.x - r.left;
                let client_top = origin.y - r.top;
                let left = r.left;
                let top = r.top;
                OffsetRect(&mut r, -left, -top);
                if s.kind == Some(Kind::Input) {
                    let client = RECT {
                        left: client_left.clamp(0, r.right),
                        top: client_top.clamp(0, r.bottom),
                        right: (client_left + client.right).clamp(0, r.right),
                        bottom: (client_top + client.bottom).clamp(0, r.bottom),
                    };
                    for band in [
                        RECT {
                            left: 0,
                            top: 0,
                            right: r.right,
                            bottom: client.top,
                        },
                        RECT {
                            left: 0,
                            top: client.bottom,
                            right: r.right,
                            bottom: r.bottom,
                        },
                        RECT {
                            left: 0,
                            top: client.top,
                            right: client.left,
                            bottom: client.bottom,
                        },
                        RECT {
                            left: client.right,
                            top: client.top,
                            right: r.right,
                            bottom: client.bottom,
                        },
                    ] {
                        FillRect(dc, &band, s.brush());
                    }
                }
                if GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_BORDER != 0 {
                    border(dc, &r, if GetFocus() == hwnd { ACCENT } else { BORDER });
                }
                ReleaseDC(hwnd, dc);
            }
            return result;
        }
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}
fn self_kind_body(s: &Surface) -> bool {
    s.kind == Some(Kind::Detail)
        && matches!(s.role.get(), Role::Body | Role::Incoming | Role::Outgoing)
}
unsafe fn edit_outline_rect(s: &Surface, dc: HDC) {
    let mut rect: RECT = std::mem::zeroed();
    GetWindowRect(s.hwnd, &mut rect);
    let (x, y) = (rect.left, rect.top);
    OffsetRect(&mut rect, -x, -y);
    if s.active.get() {
        border(
            dc,
            &rect,
            if GetFocus() == s.hwnd { ACCENT } else { BORDER },
        );
    } else {
        FrameRect(dc, &rect, GetSysColorBrush(COLOR_WINDOWFRAME));
    }
}
pub(super) fn title_bar(hwnd: HWND, active: bool) {
    unsafe {
        let enabled = i32::from(active);
        let _ = DwmSetWindowAttribute(
            hwnd,
            20,
            &enabled as *const _ as *const _,
            std::mem::size_of::<i32>() as u32,
        );
        let caption = if active { RAIL } else { u32::MAX };
        let _ = DwmSetWindowAttribute(
            hwnd,
            35,
            &caption as *const _ as *const _,
            std::mem::size_of::<u32>() as u32,
        );
    }
}
