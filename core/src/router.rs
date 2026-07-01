// router.rs — Call routing, link enforcement, caller identity.
//
// Receives a call, checks the caller→callee link, finds the right module,
// picks the right transport (direct vs IPC), dispatches, collects result.
//
// The link permission check is the core security guarantee: if
// glucore.toml does not declare `link A -> B`, then A calling B is an
// error — not silently allowed.

use crate::types::*;
use std::ffi::CString;
use std::os::raw::c_char;

// --- Topology / link graph (Task 5) ---------------------------------------

pub(crate) static mut CALLER_IDENTITY: Option<std::ffi::CString> = None;
static mut LINKS: Vec<(std::ffi::CString, std::ffi::CString)> = Vec::new();

// --- Debug introspection (Task 9, Requirement 1) --------------------------

/// Return a u64 summarizing the current shared state.
#[no_mangle]
pub extern "C" fn glucore_shared_state_checksum() -> u64 {
    unsafe {
        let links = &raw const LINKS;
        let links = &*links;
        let link_count = links.len() as u64;

        let ci = &raw const CALLER_IDENTITY;
        let ci = &*ci;
        let caller_len = ci.as_ref().map(|c| c.to_bytes().len() as u64).unwrap_or(0);

        (caller_len << 32) | (link_count & 0xFFFF_FFFF)
    }
}

/// Return the current link-table size.
#[no_mangle]
pub extern "C" fn glucore_link_table_size() -> usize {
    unsafe {
        let links = &raw const LINKS;
        (*links).len()
    }
}

/// Return the address of the LINKS static as a u64.
#[no_mangle]
pub extern "C" fn glucore_links_address() -> u64 {
    unsafe {
        let links = &raw const LINKS;
        let p: *const Vec<(std::ffi::CString, std::ffi::CString)> = links;
        p as u64
    }
}

/// Set the current caller identity. Must be called BEFORE any `glucore_call`.
#[no_mangle]
pub extern "C" fn glucore_set_caller_identity(name: *const c_char) {
    if name.is_null() {
        unsafe {
            let ci = &raw mut CALLER_IDENTITY;
            *ci = None;
        }
        return;
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(name) }.to_owned();
    unsafe {
        let ci = &raw mut CALLER_IDENTITY;
        *ci = Some(cstr);
    }
}

/// Add a single (caller → callee) link to the topology graph.
#[no_mangle]
pub extern "C" fn glucore_add_link(caller: *const c_char, callee: *const c_char) {
    if caller.is_null() || callee.is_null() {
        return;
    }
    let caller = unsafe { std::ffi::CStr::from_ptr(caller) }.to_owned();
    let callee = unsafe { std::ffi::CStr::from_ptr(callee) }.to_owned();
    unsafe {
        let l = &raw mut LINKS;
        (*l).push((caller, callee));
    }
}

// --- Caller identity helpers (zero-allocation where possible) -------------

fn current_caller_bytes() -> Option<std::ffi::CString> {
    unsafe {
        let ci = &raw const CALLER_IDENTITY;
        (*ci).as_ref().map(|c| c.clone())
    }
}

fn current_caller() -> Option<String> {
    unsafe {
        let ci = &raw const CALLER_IDENTITY;
        (*ci).as_ref().map(|c| c.to_string_lossy().into_owned())
    }
}

// --- Caller-identity save/restore (Task 6b) --------------------------------

/// RAII guard: while alive, the current caller identity is `new_caller`;
/// on drop it is restored to whatever it was before `enter`.
pub struct CallerGuard {
    previous: Option<std::ffi::CString>,
}

impl CallerGuard {
    pub fn enter(new_caller: &str) -> Self {
        let previous = unsafe {
            let ci = &raw mut CALLER_IDENTITY;
            (*ci).replace(
                std::ffi::CString::new(new_caller)
                    .unwrap_or_else(|_| std::ffi::CString::new("invalid").unwrap()),
            )
        };
        CallerGuard { previous }
    }
}

impl Drop for CallerGuard {
    fn drop(&mut self) {
        unsafe {
            let ci = &raw mut CALLER_IDENTITY;
            *ci = self.previous.take();
        }
    }
}

// --- Link check ------------------------------------------------------------

/// Byte-slice variant of is_link_allowed — accepts the caller as &[u8]
/// (zero-allocation) and the callee as &[u8].
fn is_link_allowed_bytes(caller: &[u8], callee: &[u8]) -> bool {
    unsafe {
        let l = &raw const LINKS;
        let l = &*l;
        for (c, ce) in l.iter() {
            if c.to_bytes() == caller && ce.to_bytes() == callee {
                return true;
            }
        }
    }
    false
}

// --- Dispatch --------------------------------------------------------------

/// Core dispatch shared by `glucore_call` (external) and `call_as` (internal).
/// Assumes the caller identity has already been set up by the caller; performs
/// the link check against the *current* identity and invokes the wrapper.
pub(crate) fn dispatch(module: &str, function: &str, args: *const GluValue, argc: usize) -> GluResult {
    let caller_bytes: Option<std::ffi::CString> = current_caller_bytes();
    let caller_ok = match &caller_bytes {
        Some(c) => is_link_allowed_bytes(c.to_bytes(), module.as_bytes()),
        None => {
            return GluResult::err(
                GluStatus::LinkDenied,
                "no caller identity set — call glucore_set_caller_identity first",
            );
        }
    };
    if !caller_ok {
        return GluResult::err(
            GluStatus::LinkDenied,
            &format!(
                "link denied: caller '{}' is not allowed to call module '{}'",
                caller_bytes.as_ref().map(|c| c.to_string_lossy().into_owned()).unwrap_or_default(),
                module
            ),
        );
    }

    let module_bytes = module.as_bytes();
    let fn_bytes = function.as_bytes();

    // Check if this is an IPC module.
    #[cfg(unix)]
    let ipc_sock = crate::runtime::ipc_socket_for_module(module);
    #[cfg(unix)]
    let is_ipc = ipc_sock.is_some();
    #[cfg(not(unix))]
    let is_ipc = false;

    let registry = unsafe { crate::handles::registry() };
    for m in registry.iter() {
        let m_name_bytes = unsafe { std::ffi::CStr::from_ptr(m.name).to_bytes() };
        if m_name_bytes != module_bytes {
            continue;
        }
        let entries = unsafe { std::slice::from_raw_parts(m.entries, m.count) };
        for (entry_idx, e) in entries.iter().enumerate() {
            let e_name_bytes = unsafe { std::ffi::CStr::from_ptr(e.name).to_bytes() };
            if e_name_bytes != fn_bytes {
                continue;
            }
            let result = if is_ipc {
                #[cfg(unix)]
                {
                    let base = crate::runtime::ipc_base_idx_for_module(module).unwrap_or(0);
                    let export_idx = base + entry_idx;
                    let sock_fd = ipc_sock.unwrap();
                    crate::runtime::IPC_CALL_CTX.with(|c| *c.borrow_mut() = Some((export_idx, sock_fd)));
                    let r = (e.wrapper)(args, argc);
                    crate::runtime::IPC_CALL_CTX.with(|c| *c.borrow_mut() = None);
                    r
                }
                #[cfg(not(unix))]
                {
                    unreachable!("is_ipc is always false on non-Unix")
                }
            } else {
                (e.wrapper)(args, argc)
            };
            return result;
        }
        return GluResult::err(GluStatus::NotFound, "function not found in module");
    }
    GluResult::err(GluStatus::NotFound, "module not found")
}

// --- Public call entry points ----------------------------------------------

/// Call `module.function(args)` while acting as `caller` (Task 6b/7).
/// Safe Rust API for Rust modules linked against glucore_core's rlib.
/// NOTE: Rust modules that use this MUST resolve glucore_core dynamically
/// (via libloading) to avoid the rlib-duplication footgun. See
/// KNOWN_FOOTGUNS.md #1.
pub fn call_as(
    caller: &str,
    module: &str,
    function: &str,
    args: &[GluValue],
) -> GluResult {
    let _guard = CallerGuard::enter(caller);
    dispatch(module, function, args.as_ptr(), args.len())
}

/// The main C ABI entry point for making a call. Used by Python (via
/// ctypes) and by C++ (via normal linking).
#[no_mangle]
pub extern "C" fn glucore_call(
    module: *const c_char,
    function: *const c_char,
    args: *const GluValue,
    argc: usize,
) -> GluResult {
    let module_bytes: &[u8] = unsafe {
        if module.is_null() {
            return GluResult::err(GluStatus::InvalidArgs, "null module name");
        }
        std::ffi::CStr::from_ptr(module).to_bytes()
    };
    let fn_bytes: &[u8] = unsafe {
        if function.is_null() {
            return GluResult::err(GluStatus::InvalidArgs, "null function name");
        }
        std::ffi::CStr::from_ptr(function).to_bytes()
    };
    let module_name: &str = unsafe { std::str::from_utf8_unchecked(module_bytes) };
    let fn_name: &str = unsafe { std::str::from_utf8_unchecked(fn_bytes) };
    dispatch(module_name, fn_name, args, argc)
}
