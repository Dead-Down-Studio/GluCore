// errors.rs — Unified error model.
//
// GluResult construction helpers. No exceptions ever cross a language
// boundary — every adapter catches its language's native errors and
// converts them to a GluResult before returning.

use crate::types::{GluResult, GluStatus, GluValue};
use std::ffi::CString;
use std::os::raw::c_char;

impl GluResult {
    pub fn ok(v: GluValue) -> Self {
        GluResult {
            status: GluStatus::Ok,
            value: v,
            message: std::ptr::null(),
        }
    }

    pub fn err(status: GluStatus, msg: &str) -> Self {
        let c = CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap());
        GluResult {
            status,
            value: GluValue { float: 0.0 },
            message: c.into_raw(),
        }
    }
}

/// Reclaim a Rust-allocated byte buffer previously handed to Python as a
/// String/Buffer return value. `ptr`/`len` are exactly the `ptr`/`len` carried
/// in the `GluSlice` of a returned `GluValue`. Safe to call with a null pointer
/// (no-op); MUST NOT be called twice on the same pointer (double-free) or with
/// a length other than the one Rust handed out (UB).
#[no_mangle]
pub extern "C" fn glucore_free_buffer(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = Box::from_raw(std::slice::from_raw_parts_mut(ptr, len));
    }
}
