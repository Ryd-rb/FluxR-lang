//! Optional painting around standard Win32 controls. Native classes retain all
//! input, IME, selection, scrolling, check-state and accessibility behavior.
use super::*;
use std::{rc::Rc, sync::OnceLock};
use windows_sys::Win32::{Graphics::Dwm::DwmSetWindowAttribute, UI::Accessibility::*};
#[cfg(test)]
thread_local! {pub(super) static MESSAGES:Cell<[u64;4]>=const{Cell::new([0;4])};}

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
const CLIENT: u32 = rgb(21, 27, 32);
const RAIL: u32 = rgb(15, 21, 25);
const FIELD: u32 = rgb(29, 38, 45);
const PANEL: u32 = rgb(25, 33, 39);
const HOVER: u32 = rgb(39, 52, 60);
const BORDER: u32 = rgb(57, 70, 79);
const TEXT: u32 = rgb(237, 240, 239);
const MUTED: u32 = rgb(156, 173, 183);
const DISABLED: u32 = rgb(113, 130, 141);
const ACCENT: u32 = rgb(62, 202, 177);
const PRIMARY: u32 = rgb(13, 112, 102);
const SELECTED: u32 = rgb(27, 61, 59);
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
                let theme_name = wide("DarkMode_Explorer");
                SetWindowTheme(
                    self.hwnd,
                    if active { theme_name.as_ptr() } else { null() },
                    null(),
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
                InvalidateRect(self.hwnd, null(), 1);
            }
        }
    }
    unsafe fn composer_style(&self) {
        if matches!(self.kind, Some(Kind::Input | Kind::Multiline)) {
            let old = GetWindowLongW(self.hwnd, GWL_STYLE) as u32;
            let style = if self.active.get() && self.composer.get() {
                old & !WS_BORDER
            } else {
                old | WS_BORDER
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
        {
            let mut rect: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut rect);
            let pad = self.scale(if self.role.get() == Role::Outgoing {
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
    fn scale(&self, n: i32) -> i32 {
        n * self.dpi.get() as i32 / 96
    }
    fn background(&self) -> u32 {
        if self.side.get() {
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
        FillRect(dc, &r, self.palette.client);
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
            let pressed = SendMessageW(self.hwnd, BM_GETSTATE, 0, 0) & BST_PUSHED as isize != 0;
            let bg = if disabled {
                FIELD
            } else if pressed {
                SELECTED
            } else if role == Role::Primary {
                PRIMARY
            } else if role == Role::Selected {
                SELECTED
            } else if self.hover.get() {
                HOVER
            } else if matches!(role, Role::Quiet | Role::Nav) {
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
                } else if matches!(role, Role::Quiet | Role::Nav | Role::Selected) {
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
            if role == Role::Selected {
                let indicator = RECT {
                    left: r.left,
                    top: r.top + self.scale(10),
                    right: r.left + self.scale(3),
                    bottom: r.bottom - self.scale(10),
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
    } else if msg == WM_IME_ENDCOMPOSITION {
        s.ime.set(false);
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
                return SendMessageW(parent, msg, wp, lp);
            }
        }
    }
    if matches!(msg, WM_SIZE | WM_SETFONT)
        && s.kind == Some(Kind::Detail)
        && matches!(s.role.get(), Role::Incoming | Role::Outgoing)
    {
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        s.format_chat();
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
    if s.active.get() {
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
            if s.kind.is_none() {
                s.root(wp as HDC);
            }
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
            let result = DefSubclassProc(hwnd, msg, wp, lp);
            if GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_BORDER != 0 {
                let dc = GetWindowDC(hwnd);
                let mut r: RECT = std::mem::zeroed();
                GetWindowRect(hwnd, &mut r);
                let left = r.left;
                let top = r.top;
                OffsetRect(&mut r, -left, -top);
                border(dc, &r, if GetFocus() == hwnd { ACCENT } else { BORDER });
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
