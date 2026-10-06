// handles.rs — Module registry and signature introspection.
//
// Tracks all registered modules (both ARTIFACT and PROCESS) in a global
// REGISTRY. The registry is a Vec<GluModule> — small in the demo, and
// the linear scan is cache-friendly. A HashMap would help only at N >> 100
// modules.

use crate::types::*;
use std::collections::HashMap;
use std::os::raw::c_char;
use std::sync::{OnceLock, RwLock};

// --- Module registry -------------------------------------------------------

#[derive(Default)]
struct RegistryState {
    modules: Vec<GluModule>,
    module_index: HashMap<Vec<u8>, usize>,
    export_index: HashMap<usize, HashMap<Vec<u8>, usize>>,
}

// SAFETY: module pointers are process-lifetime registration data owned by
// adapters/modules; this state only stores and reads those immutable pointers.
unsafe impl Send for RegistryState {}
unsafe impl Sync for RegistryState {}

static REGISTRY: OnceLock<RwLock<RegistryState>> = OnceLock::new();

fn registry_state() -> &'static RwLock<RegistryState> {
    REGISTRY.get_or_init(|| RwLock::new(RegistryState::default()))
}

#[no_mangle]
pub extern "C" fn glucore_register_module(module: GluModule) {
    let module_name = unsafe { std::ffi::CStr::from_ptr(module.name).to_bytes().to_vec() };
    let entries = unsafe { std::slice::from_raw_parts(module.entries, module.count) };

    let mut state = registry_state().write().expect("registry poisoned");
    let module_idx = state.modules.len();
    state.modules.push(module);
    state.module_index.insert(module_name.clone(), module_idx);
    let mut by_fn: HashMap<Vec<u8>, usize> = HashMap::new();
    for (entry_idx, entry) in entries.iter().enumerate() {
        let fn_name = unsafe { std::ffi::CStr::from_ptr(entry.name).to_bytes().to_vec() };
        by_fn.insert(fn_name, entry_idx);
    }
    state.export_index.insert(module_idx, by_fn);
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
    let module_name = unsafe { std::ffi::CStr::from_ptr(module).to_bytes().to_vec() };
    let fn_name = unsafe { std::ffi::CStr::from_ptr(function).to_bytes().to_vec() };
    let state = registry_state().read().expect("registry poisoned");
    if let Some(module_idx) = state.module_index.get(module_name.as_slice()) {
        if let Some(by_fn) = state.export_index.get(module_idx) {
            if let Some(entry_idx) = by_fn.get(fn_name.as_slice()) {
                let m = &state.modules[*module_idx];
                let entries = unsafe { std::slice::from_raw_parts(m.entries, m.count) };
                if let Some(e) = entries.get(*entry_idx) {
                    return e.signature;
                }
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
    let module_name = unsafe { std::ffi::CStr::from_ptr(module).to_bytes().to_vec() };
    let state = registry_state().read().expect("registry poisoned");
    if let Some(module_idx) = state.module_index.get(&module_name) {
        if let Some(m) = state.modules.get(*module_idx) {
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
    let module_name = unsafe { std::ffi::CStr::from_ptr(module).to_bytes().to_vec() };
    let state = registry_state().read().expect("registry poisoned");
    if let Some(module_idx) = state.module_index.get(&module_name) {
        if let Some(m) = state.modules.get(*module_idx) {
            if index >= m.count {
                return std::ptr::null();
            }
            let entries = unsafe { std::slice::from_raw_parts(m.entries, m.count) };
            return entries[index].name;
        }
    }
    std::ptr::null()
}

pub(crate) fn find_export(module: &[u8], function: &[u8]) -> Option<(usize, GluWrapper)> {
    let state = registry_state().read().ok()?;
    let module_idx = *state.module_index.get(module)?;
    let entry_idx = *state.export_index.get(&module_idx)?.get(function)?;
    let m = state.modules.get(module_idx)?;
    let entries = unsafe { std::slice::from_raw_parts(m.entries, m.count) };
    let e = entries.get(entry_idx)?;
    Some((entry_idx, e.wrapper))
}
