//! MSAA dynamic annotations preserve native edit names when captions are hidden.
//! Only the documented IUnknown/IAccPropServices ABI prefix is needed by sys.
use super::*;
use std::{ffi::c_void, ptr::null_mut};
use windows_sys::{
    Win32::{System::Com::*, UI::Accessibility::*},
    core::GUID,
};
const IID: GUID = GUID::from_u128(0x6e26e776_04f0_495d_80e4_3330352e3169);
#[repr(C)]
struct Services {
    before: [usize; 7],
    set_name: unsafe extern "system" fn(*mut c_void, HWND, u32, u32, GUID, *const u16) -> i32,
    set_server: usize,
    clear: unsafe extern "system" fn(*mut c_void, HWND, u32, u32, *const GUID, i32) -> i32,
}
pub(super) fn name(hwnd: HWND, value: &str) -> bool {
    services(|object, table| unsafe {
        ((*table).set_name)(
            object,
            hwnd,
            OBJID_CLIENT as u32,
            0,
            PROPID_ACC_NAME,
            wide(value).as_ptr(),
        ) >= 0
    })
}
pub(super) fn clear(hwnd: HWND) {
    services(|object, table| unsafe {
        ((*table).clear)(object, hwnd, OBJID_CLIENT as u32, 0, &PROPID_ACC_NAME, 1) >= 0
    });
}
fn services(call: impl FnOnce(*mut c_void, *const Services) -> bool) -> bool {
    unsafe {
        let init = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        let mut object = std::ptr::null_mut();
        let result = CoCreateInstance(
            &CLSID_AccPropServices,
            null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID,
            &mut object,
        );
        let ok = if result >= 0 && !object.is_null() {
            let table = *(object as *const *const Services);
            let ok = call(object, table);
            let release: unsafe extern "system" fn(*mut c_void) -> u32 =
                std::mem::transmute((*table).before[2]);
            release(object);
            ok
        } else {
            false
        };
        if init >= 0 {
            CoUninitialize();
        }
        ok
    }
}
#[cfg(test)]
pub(super) unsafe fn native_name(hwnd: HWND) -> String {
    use windows_sys::Win32::System::Variant::{VARIANT, VT_I4};
    let iid = GUID::from_u128(0x618736e0_3c3d_11cf_810c_00aa00389b71);
    let mut object = null_mut();
    // AccessibleObjectFromWindow includes dynamic annotation support.
    assert!(AccessibleObjectFromWindow(hwnd, OBJID_CLIENT as u32, &iid, &mut object) >= 0);
    let table = *(object as *const *const usize);
    let get: unsafe extern "system" fn(*mut c_void, VARIANT, *mut *const u16) -> i32 =
        std::mem::transmute(*table.add(10));
    let mut child: VARIANT = std::mem::zeroed();
    child.Anonymous.Anonymous.vt = VT_I4;
    let mut value = std::ptr::null();
    assert!(get(object, child, &mut value) >= 0);
    let mut len = 0;
    while !value.is_null() && *value.add(len) != 0 {
        len += 1;
    }
    let name = if value.is_null() {
        String::new()
    } else {
        String::from_utf16_lossy(std::slice::from_raw_parts(value, len))
    };
    SysFreeString(value);
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(*table.add(2));
    release(object);
    name
}
