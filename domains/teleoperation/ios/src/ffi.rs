//! The C interface SwiftUI calls (`include/t1s.h`). Strings are UTF-8, NUL-terminated; a string
//! the station returns is the caller's to give back with [`t1s_string_free`].

use crate::{Station, control};
use std::ffi::{CStr, CString, c_char};
use std::sync::Arc;

unsafe fn text<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() { None } else { unsafe { CStr::from_ptr(p) }.to_str().ok() }
}

/// Start the station on `address`. With a `control_port` above 0 the control server listens on
/// it (the acceptance driver's way in). Never returns null.
///
/// # Safety
/// `address` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn t1s_start(address: *const c_char, control_port: u16) -> *const Station {
    let address = unsafe { text(address) }.unwrap_or("tcpout:127.0.0.1:5760");
    let station = Arc::new(Station::start(address));
    if control_port > 0
        && let Err(e) = control::serve(station.clone(), control_port)
    {
        eprintln!("t1s: no control server on port {control_port}: {e}");
    }
    // The app runs as long as the station does.
    Arc::into_raw(station)
}

/// # Safety
/// `station` came from [`t1s_start`]; `id` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn t1s_press(station: *const Station, id: *const c_char) {
    if let (Some(s), Some(id)) = (unsafe { station.as_ref() }, unsafe { text(id) }) {
        s.press(id);
    }
}

/// The state called `key`, or null if there is none.
///
/// # Safety
/// `station` came from [`t1s_start`]; `key` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn t1s_observe(station: *const Station, key: *const c_char) -> *mut c_char {
    let value = unsafe { station.as_ref() }.zip(unsafe { text(key) }).and_then(|(s, k)| s.observe(k));
    value.and_then(|v| CString::new(v).ok()).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// # Safety
/// `s` came from this library, and is not used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn t1s_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}
