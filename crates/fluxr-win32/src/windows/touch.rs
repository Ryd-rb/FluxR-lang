//! Contact ownership and routing, separate from editable control behavior.
//! Pointer input uses Windows' implicit contact capture. Compatibility mouse
//! contacts capture only this window, never install hooks or inject input.
use super::*;
use std::{
    cell::RefCell,
    collections::{HashSet, VecDeque},
};
use windows_sys::Win32::UI::Input::{IMDT_TOUCH, Pointer::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Pointer(u16),
    Compatibility,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Down,
    Move,
    Up,
    Cancel,
    Hover,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Event {
    pub source: Source,
    pub phase: Phase,
    pub x: i32,
    pub y: i32,
    pub primary: bool,
    pub owner: HWND,
}
#[derive(Default, Debug)]
pub(super) struct Effect {
    pub consumed: bool,
    pub delta: i32,
    pub begin: bool,
    pub end: bool,
    pub capture: bool,
    pub release: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Pending,
    Vertical,
    Other,
}
#[derive(Debug)]
struct Contact {
    source: Source,
    owner: HWND,
    x: i32,
    y: i32,
    last_y: i32,
    slop: i32,
    axis: Axis,
}
#[derive(Default, Debug)]
pub(super) struct Pan {
    active: Option<Contact>,
    owned: HashSet<u16>,
    recent: VecDeque<u16>,
    compatibility_suppressed: bool,
    compatibility_owned: bool,
}
impl Pan {
    pub(super) fn owner(&self) -> Option<HWND> {
        self.active.as_ref().map(|a| a.owner)
    }
    pub(super) fn mouse_active(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|a| a.source == Source::Compatibility)
    }
    pub(super) fn forget(&mut self, id: u16) -> Effect {
        self.owned.remove(&id);
        self.recent.retain(|previous| *previous != id);
        if self
            .active
            .as_ref()
            .is_some_and(|a| a.source == Source::Pointer(id))
        {
            self.active = None;
            Effect {
                consumed: true,
                end: true,
                ..Effect::default()
            }
        } else {
            Effect::default()
        }
    }
    fn remember(&mut self, id: u16) {
        self.recent.retain(|previous| *previous != id);
        self.recent.push_back(id);
        while self.recent.len() > 32 {
            self.recent.pop_front();
        }
    }
    pub(super) fn cancel(&mut self) -> Effect {
        let owned = !self.owned.is_empty();
        for id in std::mem::take(&mut self.owned) {
            self.remember(id);
        }
        let active = self.active.take();
        self.compatibility_owned = false;
        self.compatibility_suppressed = false;
        Effect {
            consumed: active.is_some() || owned,
            end: active.is_some(),
            release: active.is_some_and(|a| a.source == Source::Compatibility),
            ..Effect::default()
        }
    }
    pub(super) fn handle(&mut self, event: Event, eligible: bool, slop: i32) -> Effect {
        let mut result = Effect::default();
        match event.source {
            Source::Pointer(id) => {
                if event.phase == Phase::Down && !eligible {
                    self.forget(id);
                    return result;
                }
                let known = self.owned.contains(&id) || self.recent.contains(&id);
                if event.phase != Phase::Down && !known {
                    return result;
                }
                if event.phase == Phase::Down && eligible {
                    self.owned.insert(id);
                    self.recent.retain(|previous| *previous != id);
                }
                result.consumed = true;
                if event.phase == Phase::Up || event.phase == Phase::Cancel {
                    self.owned.remove(&id);
                    self.remember(id);
                }
            }
            Source::Compatibility => {
                if !eligible && !self.compatibility_owned {
                    return result;
                }
                result.consumed = true;
                if event.phase == Phase::Down && eligible {
                    self.compatibility_owned = true;
                    self.compatibility_suppressed = !self.owned.is_empty();
                }
                if !self.owned.is_empty() || self.compatibility_suppressed {
                    // Suppress only this overlapping promoted contact, never
                    // disable a future independent compatibility-only drag.
                    if matches!(event.phase, Phase::Up | Phase::Cancel) {
                        self.compatibility_owned = false;
                        self.compatibility_suppressed = false;
                    }
                    return result;
                }
                if event.phase == Phase::Up {
                    self.compatibility_owned = false;
                }
            }
        }
        if event.phase == Phase::Down {
            if !eligible || !event.primary {
                return result;
            }
            if self
                .active
                .as_ref()
                .is_some_and(|a| matches!(a.source, Source::Pointer(_)) && a.source != event.source)
            {
                return result;
            }
            result.release = self
                .active
                .as_ref()
                .is_some_and(|a| a.source == Source::Compatibility);
            if matches!(event.source, Source::Pointer(_)) && self.mouse_active() {
                let active = self.active.as_mut().unwrap();
                active.source = event.source;
                active.owner = event.owner;
                self.compatibility_suppressed = self.compatibility_owned;
                return result;
            }
            self.active = Some(Contact {
                source: event.source,
                owner: event.owner,
                x: event.x,
                y: event.y,
                last_y: event.y,
                slop: slop.max(1),
                axis: Axis::Pending,
            });
            result.begin = true;
            result.capture = event.source == Source::Compatibility;
            return result;
        }
        let Some(active) = self.active.as_mut().filter(|a| a.source == event.source) else {
            return result;
        };
        if matches!(event.phase, Phase::Move | Phase::Up) {
            let dx = event.x.saturating_sub(active.x).unsigned_abs();
            let dy = event.y.saturating_sub(active.y).unsigned_abs();
            if active.axis == Axis::Pending && dx.max(dy) >= active.slop as u32 {
                active.axis = if dy >= dx {
                    Axis::Vertical
                } else {
                    Axis::Other
                };
            }
            if active.axis == Axis::Vertical {
                result.delta = active.last_y.saturating_sub(event.y);
            }
            if active.axis != Axis::Pending {
                active.last_y = event.y;
            }
        }
        if matches!(event.phase, Phase::Up | Phase::Cancel) {
            result.end = true;
            result.release = active.source == Source::Compatibility;
            self.active = None;
            if event.phase == Phase::Up {
                self.compatibility_owned = false;
            }
        }
        result
    }
}
pub(super) const MESSAGE: u32 = WM_APP + 18;
pub(super) enum Packet {
    Contact(Event),
    Cancel,
    MouseCaptureLost,
    OwnerDestroyed(HWND),
    ForgetPointer(u16),
    UnavailablePointer(u16),
}
struct Delivery {
    root: HWND,
    nonce: usize,
    packet: Packet,
}
thread_local! {
    static DELIVERY:RefCell<Option<Delivery>>=const{RefCell::new(None)};
    static NONCE:Cell<usize>=const{Cell::new(0)};
}
pub(super) fn take(root: HWND, nonce: usize) -> Option<Packet> {
    DELIVERY.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|d| d.root == root && d.nonce == nonce)
        {
            slot.take().map(|d| d.packet)
        } else {
            None
        }
    })
}

pub(super) fn promoted_touch(extra: usize, device: u32) -> bool {
    let value = extra as u32;
    if value & 0xffffff00 == 0xff515700 {
        value & 0x80 != 0
    } else {
        device == IMDT_TOUCH as u32
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn event(source: Source, phase: Phase, x: i32, y: i32) -> Event {
        Event {
            source,
            phase,
            x,
            y,
            primary: true,
            owner: 101,
        }
    }
    #[test]
    fn contact_ownership_deduplicates_only_overlap_and_cancels_cleanly() {
        let mut pan = Pan::default();
        let pointer = Source::Pointer(7);
        assert!(
            !pan.handle(event(pointer, Phase::Move, 10, 0), true, 12)
                .consumed
        );
        let mouse = Source::Compatibility;
        let down = pan.handle(event(pointer, Phase::Down, 100, 200), true, 8);
        assert!(down.begin && !down.capture);
        assert!(
            !pan.handle(event(mouse, Phase::Down, 100, 200), true, 8)
                .begin
        );
        assert_eq!(
            pan.handle(event(pointer, Phase::Move, 100, 150), true, 8)
                .delta,
            50
        );
        assert_eq!(
            pan.handle(event(mouse, Phase::Move, 100, 150), true, 8)
                .delta,
            0
        );
        assert!(pan.handle(event(pointer, Phase::Up, 100, 150), true, 8).end);
        assert_eq!(
            pan.handle(event(mouse, Phase::Up, 100, 150), true, 8).delta,
            0
        );
        let next = pan.handle(event(mouse, Phase::Down, 100, 200), true, 8);
        assert!(next.begin && next.capture);
        assert_eq!(
            pan.handle(event(mouse, Phase::Move, 100, 160), true, 8)
                .delta,
            40
        );
        assert_eq!(
            pan.handle(event(mouse, Phase::Move, 100, 180), true, 8)
                .delta,
            -20
        );
        let cancelled = pan.cancel();
        assert!(cancelled.release && cancelled.end);
        assert!(!pan.compatibility_owned && !pan.compatibility_suppressed);
        assert!(
            !pan.handle(event(mouse, Phase::Move, 100, 140), false, 8)
                .consumed
        );
        assert!(
            !pan.handle(event(pointer, Phase::Down, 0, 0), false, 8)
                .consumed
        );
        assert!(
            !pan.handle(event(pointer, Phase::Cancel, 0, 0), false, 8)
                .consumed
        );
        assert!(pan.owner().is_none());
        assert!(promoted_touch(0xff515780, 0));
        assert!(!promoted_touch(0xff515700, IMDT_TOUCH as u32));
        assert!(!promoted_touch(0, 0));
        assert!(promoted_touch(0, IMDT_TOUCH as u32));
    }
    #[test]
    fn slop_secondary_contacts_id_reuse_and_capture_takeover_are_bounded() {
        let mut pan = Pan::default();
        let pointer = Source::Pointer(9);
        pan.handle(event(pointer, Phase::Down, 100, 200), true, 12);
        assert_eq!(
            pan.handle(event(pointer, Phase::Move, 102, 195), true, 12)
                .delta,
            0
        );
        let mut secondary = event(Source::Pointer(10), Phase::Down, 120, 200);
        secondary.primary = false;
        assert!(!pan.handle(secondary, true, 12).begin);
        assert_eq!(
            pan.handle(event(Source::Pointer(10), Phase::Move, 120, 50), true, 12)
                .delta,
            0
        );
        assert_eq!(
            pan.handle(event(pointer, Phase::Move, 102, 180), true, 12)
                .delta,
            20
        );
        assert!(
            pan.handle(event(pointer, Phase::Cancel, 0, 0), false, 12)
                .end
        );
        pan.handle(event(Source::Pointer(10), Phase::Up, 120, 50), false, 12);
        assert!(
            !pan.handle(event(pointer, Phase::Down, 10, 10), false, 12)
                .consumed
        );
        assert!(
            !pan.handle(event(pointer, Phase::Move, 10, 0), false, 12)
                .consumed
        );
        let mouse = Source::Compatibility;
        pan.handle(event(mouse, Phase::Down, 100, 200), true, 8);
        assert_eq!(
            pan.handle(event(mouse, Phase::Move, 100, 160), true, 8)
                .delta,
            40
        );
        assert!(
            pan.handle(event(pointer, Phase::Down, 100, 200), true, 8)
                .release
        );
        assert_eq!(
            pan.handle(event(pointer, Phase::Move, 100, 150), true, 8)
                .delta,
            10
        );
        pan.cancel();
        pan.handle(event(mouse, Phase::Down, 100, 200), true, 8);
        assert_eq!(
            pan.handle(event(mouse, Phase::Move, 140, 199), true, 8)
                .delta,
            0
        );
        assert_eq!(
            pan.handle(event(mouse, Phase::Move, 140, 100), true, 8)
                .delta,
            0
        );
        assert!(pan.handle(event(mouse, Phase::Up, 140, 100), true, 8).end);
        assert!(pan.owner().is_none());
        for slop in [8, 12, 16, 24] {
            pan.handle(event(pointer, Phase::Down, 100, 200), true, slop);
            assert_eq!(
                pan.handle(event(pointer, Phase::Move, 100, 200 - slop + 1), true, slop)
                    .delta,
                0
            );
            assert_eq!(
                pan.handle(event(pointer, Phase::Move, 100, 120), true, slop)
                    .delta,
                80
            );
            pan.cancel();
        }
    }
    #[test]
    fn scoped_delivery_requires_matching_root_and_one_shot_nonce() {
        DELIVERY.with(|slot| {
            slot.replace(Some(Delivery {
                root: 101,
                nonce: 7,
                packet: Packet::Cancel,
            }))
        });
        assert!(take(102, 7).is_none());
        assert!(take(101, 8).is_none());
        assert!(take(101, 7).is_some());
        assert!(take(101, 7).is_none());
    }
}
unsafe fn send(hwnd: HWND, packet: Packet) -> bool {
    let root = GetAncestor(hwnd, GA_ROOT);
    if root == 0 || UPDATING.with(Cell::get) {
        return false;
    }
    let nonce = NONCE.with(|n| {
        let next = n.get().wrapping_add(1).max(1);
        n.set(next);
        next
    });
    if !DELIVERY.with(|slot| {
        if slot.borrow().is_some() {
            false
        } else {
            slot.replace(Some(Delivery {
                root,
                nonce,
                packet,
            }));
            true
        }
    }) {
        return false;
    }
    let handled = SendMessageW(root, MESSAGE, nonce, 0) != 0;
    take(root, nonce);
    handled
}
#[cfg(test)]
thread_local! {
    // Input metadata seam only. No hardware contacts or OS input are created.
    pub(super) static POINTER_SAMPLE:Cell<Option<POINTER_INFO>>=const{Cell::new(None)};
}
unsafe fn pointer_info(id: u16) -> Option<POINTER_INFO> {
    #[cfg(test)]
    if let Some(info) = POINTER_SAMPLE.with(Cell::get) {
        return Some(info);
    }
    let mut info: POINTER_INFO = std::mem::zeroed();
    if GetPointerInfo(id as u32, &mut info) != 0 {
        Some(info)
    } else {
        None
    }
}
pub(super) unsafe fn route(hwnd: HWND, msg: u32, wp: usize, lp: isize, participant: bool) -> bool {
    if !participant {
        return false;
    }
    if msg == WM_CANCELMODE || msg == WM_ACTIVATEAPP && wp == 0 {
        return send(hwnd, Packet::Cancel);
    }
    if msg == WM_CAPTURECHANGED {
        return send(hwnd, Packet::MouseCaptureLost);
    }
    if msg == WM_NCDESTROY {
        return send(hwnd, Packet::OwnerDestroyed(hwnd));
    }
    let event = if matches!(
        msg,
        WM_POINTERDOWN
            | WM_POINTERUPDATE
            | WM_POINTERUP
            | WM_POINTERCAPTURECHANGED
            | WM_POINTERENTER
            | WM_POINTERLEAVE
    ) {
        let id = (wp & 0xffff) as u16;
        if msg == WM_POINTERCAPTURECHANGED {
            Event {
                source: Source::Pointer(id),
                phase: Phase::Cancel,
                x: 0,
                y: 0,
                primary: false,
                owner: hwnd,
            }
        } else {
            let info = pointer_info(id);
            let mut kind = info.as_ref().map_or(0, |i| i.pointerType);
            if kind == 0 {
                GetPointerType(id as u32, &mut kind);
            }
            if kind != PT_TOUCH {
                if msg == WM_POINTERDOWN {
                    send(hwnd, Packet::ForgetPointer(id));
                } else if kind == 0 {
                    return send(hwnd, Packet::UnavailablePointer(id));
                }
                return false;
            }
            let point = info.as_ref().map_or(
                POINT {
                    x: lp as i16 as i32,
                    y: (lp >> 16) as i16 as i32,
                },
                |i| i.ptPixelLocation,
            );
            let flags = info.as_ref().map_or(0, |i| i.pointerFlags);
            let phase = if flags & POINTER_FLAG_CANCELED != 0
                || msg == WM_POINTERLEAVE && flags & POINTER_FLAG_INCONTACT == 0
            {
                Phase::Cancel
            } else {
                match msg {
                    WM_POINTERDOWN => Phase::Down,
                    WM_POINTERUP => Phase::Up,
                    WM_POINTERUPDATE => Phase::Move,
                    _ => Phase::Hover,
                }
            };
            Event {
                source: Source::Pointer(id),
                phase,
                x: point.x,
                y: point.y,
                primary: info.is_none() || flags & POINTER_FLAG_PRIMARY != 0,
                owner: hwnd,
            }
        }
    } else if matches!(
        msg,
        WM_LBUTTONDOWN
            | WM_LBUTTONDBLCLK
            | WM_LBUTTONUP
            | WM_MOUSEMOVE
            | WM_RBUTTONDOWN
            | WM_RBUTTONUP
            | WM_RBUTTONDBLCLK
            | WM_CONTEXTMENU
    ) {
        let mut source: INPUT_MESSAGE_SOURCE = std::mem::zeroed();
        GetCurrentInputMessageSource(&mut source);
        if !promoted_touch(GetMessageExtraInfo() as usize, source.deviceType as u32) {
            return false;
        }
        let mut point = POINT {
            x: lp as i16 as i32,
            y: (lp >> 16) as i16 as i32,
        };
        if msg == WM_CONTEXTMENU {
            if lp == -1 {
                return false;
            }
        } else {
            ClientToScreen(hwnd, &mut point);
        }
        Event {
            source: Source::Compatibility,
            phase: match msg {
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => Phase::Down,
                WM_LBUTTONUP | WM_RBUTTONUP => Phase::Up,
                WM_MOUSEMOVE => Phase::Move,
                _ => Phase::Hover,
            },
            x: point.x,
            y: point.y,
            primary: true,
            owner: hwnd,
        }
    } else {
        return false;
    };
    send(hwnd, Packet::Contact(event))
}
