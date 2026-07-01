// handles.rs — Module registry and signature introspection.
//
// Tracks all registered modules (both ARTIFACT and PROCESS) in a global
// REGISTRY. The registry is a Vec<GluModule> — small in the demo, and
// the linear scan is cache-friendly. A HashMap would help only at N >> 100
// modules.

use crate::types::*;
use std::os::raw::c_char;

// --- Module registry -------------------------------------------------------

static mut REGISTRY: Vec<GluModule> = Vec::new();

#[no_mangle]
pub extern "C" fn glucore_register_module(module: GluModule) {
    unsafe {
        let reg = &raw mut REGISTRY;
        (*reg).push(module);
    }
}

/// Look up a function's signature by (module, function name).
/// Task 4: Python calls this for every exported function at load_module time
/// to build a local signature dict, then validates calls against it before
/// touching ctypes.
///
/// Returns GluSignatureFFI::empty() if the function or module is not found.
#[no_mangle]
pub extern "C" fn glucore_get_signature(
    module: *const c_char,
    function: *const c_char,
) -> GluSignatureFFI {
    if module.is_null() || function.is_null() {
        return GluSignatureFFI::empty();
    }
    let module_name = unsafe { std::ffi::CStr::from_ptr(module).to_bytes() };
    let fn_name = unsafe { std::ffi::CStr::from_ptr(function).to_bytes() };
    let registry = &raw const REGISTRY;
    let registry = unsafe { &*registry };
    for m in registry.iter() {
        let m_name = unsafe { std::ffi::CStr::from_ptr(m.name).to_bytes() };
        if m_name != module_name {
            continue;
        }
        let entries = unsafe { std::slice::from_raw_parts(m.entries, m.count) };
        for e in entries.iter() {
            let e_name = unsafe { std::ffi::CStr::from_ptr(e.name).to_bytes() };
            if e_name == fn_name {
                return e.signature;
            }
        }
    }
    GluSignatureFFI::empty()
}

/// Return the number of exported functions in a module (0 if module not found).
/// Task 9: lets the Python adapter enumerate an IPC module's exports (which
/// it can't dlopen) to build a GluProxy for them.
#[no_mangle]
pub extern "C" fn glucore_get_module_export_count(module: *const c_char) -> usize {
    if module.is_null() {
        return 0;
    }
    let module_name = unsafe { std::ffi::CStr::from_ptr(module).to_bytes() };
    let registry = &raw const REGISTRY;
    let registry = unsafe { &*registry };
    for m in registry.iter() {
        let m_name = unsafe { std::ffi::CStr::from_ptr(m.name).to_bytes() };
        if m_name == module_name {
            return m.count;
        }
    }
    0
}

/// Return a pointer to the i-th export's name in a module, or null if
/// module/index is out of range.
#[no_mangle]
pub extern "C" fn glucore_get_module_export_name(
    module: *const c_char,
    index: usize,
) -> *const c_char {
    if module.is_null() {
        return std::ptr::null();
    }
    let module_name = unsafe { std::ffi::CStr::from_ptr(module).to_bytes() };
    let registry = &raw const REGISTRY;
    let registry = unsafe { &*registry };
    for m in registry.iter() {
        let m_name = unsafe { std::ffi::CStr::from_ptr(m.name).to_bytes() };
        if m_name != module_name {
            continue;
        }
        if index >= m.count {
            return std::ptr::null();
        }
        let entries = unsafe { std::slice::from_raw_parts(m.entries, m.count) };
        return entries[index].name;
    }
    std::ptr::null()
}

/// Public accessor for the registry — used by the router to find modules.
pub(crate) unsafe fn registry() -> &'static Vec<GluModule> {
    let reg = &raw const REGISTRY;
    &*reg
}
