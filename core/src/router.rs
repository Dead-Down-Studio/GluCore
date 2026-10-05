// router.rs — Call routing, link enforcement, caller identity.
//
// Production hardening notes:
// - Caller identity is per-thread state (thread-local stack) to support
//   nested/callback flows without global cross-thread races.
// - Link checks use an indexed HashSet for O(1)-ish membership checks.
// - Module/function resolution uses indexed lookups from handles.rs.

use crate::types::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::os::raw::c_char;
use std::sync::{OnceLock, RwLock};

// --- Topology / link graph --------------------------------------------------

static LINKS: OnceLock<RwLock<HashMap<Vec<u8>, HashSet<Vec<u8>>>>> = OnceLock::new();

fn links() -> &'static RwLock<HashMap<Vec<u8>, HashSet<Vec<u8>>>> {
    LINKS.get_or_init(|| RwLock::new(HashMap::new()))
}

thread_local! {
    static CALLER_STACK: RefCell<Vec<std::ffi::CString>> = const { RefCell::new(Vec::new()) };
}

// --- Debug introspection ----------------------------------------------------

/// Return a u64 summarizing the current shared state.
#[no_mangle]
pub extern "C" fn glucore_shared_state_checksum() -> u64 {
    let link_count = links()
        .read()
        .map(|l| l.values().map(|s| s.len() as u64).sum::<u64>())
        .unwrap_or(0);
    let caller_len = CALLER_STACK.with(|s| {
        s.borrow()
            .last()
            .map(|c| c.to_bytes().len() as u64)
            .unwrap_or(0)
    });
    (caller_len << 32) | (link_count & 0xFFFF_FFFF)
}

/// Return the current link-table size.
#[no_mangle]
pub extern "C" fn glucore_link_table_size() -> usize {
    links()
        .read()
        .map(|l| l.values().map(|s| s.len()).sum())
        .unwrap_or(0)
}

/// Return the address of the LINKS state as a u64.
#[no_mangle]
pub extern "C" fn glucore_links_address() -> u64 {
    let p = links() as *const RwLock<HashMap<Vec<u8>, HashSet<Vec<u8>>>>;
    p as u64
}

/// Set the current caller identity for this thread.
#[no_mangle]
pub extern "C" fn glucore_set_caller_identity(name: *const c_char) {
    CALLER_STACK.with(|s| {
        let mut stack = s.borrow_mut();
        stack.clear();
        if !name.is_null() {
            let cstr = unsafe { std::ffi::CStr::from_ptr(name) }.to_owned();
            stack.push(cstr);
        }
    });
}

/// Add a single (caller → callee) link to the topology graph.
#[no_mangle]
pub extern "C" fn glucore_add_link(caller: *const c_char, callee: *const c_char) {
    if caller.is_null() || callee.is_null() {
        return;
    }
    let caller = unsafe { std::ffi::CStr::from_ptr(caller) }.to_bytes().to_vec();
    let callee = unsafe { std::ffi::CStr::from_ptr(callee) }.to_bytes().to_vec();
    if let Ok(mut l) = links().write() {
        l.entry(caller).or_default().insert(callee);
    }
}

// --- Caller identity helpers ------------------------------------------------

pub(crate) fn current_caller_bytes() -> Option<Vec<u8>> {
    CALLER_STACK.with(|s| s.borrow().last().map(|c| c.to_bytes().to_vec()))
}

fn is_link_allowed_bytes(caller: &[u8], callee: &[u8]) -> bool {
    links()
        .read()
        .map(|l| {
            l.get(caller)
                .map(|targets| targets.contains(callee))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

// --- Caller-identity save/restore ------------------------------------------

pub struct CallerGuard {
    had_previous: bool,
}

impl CallerGuard {
    pub fn enter(new_caller: &str) -> Self {
        CALLER_STACK.with(|s| {
            let mut stack = s.borrow_mut();
            let c = std::ffi::CString::new(new_caller)
                .unwrap_or_else(|_| std::ffi::CString::new("invalid").unwrap());
            stack.push(c);
            CallerGuard {
                had_previous: stack.len() > 1,
            }
        })
    }
}

impl Drop for CallerGuard {
    fn drop(&mut self) {
        CALLER_STACK.with(|s| {
            let mut stack = s.borrow_mut();
            let _ = stack.pop();
            if !self.had_previous && stack.len() > 1 {
                stack.truncate(1);
            }
        });
    }
}

// --- Dispatch ---------------------------------------------------------------

pub(crate) fn dispatch(module: &str, function: &str, args: *const GluValue, argc: usize) -> GluResult {
    let caller_bytes = match current_caller_bytes() {
        Some(c) => c,
        None => {
            return GluResult::err(
                GluStatus::LinkDenied,
                "no caller identity set — call glucore_set_caller_identity first",
            );
        }
    };

    if !is_link_allowed_bytes(&caller_bytes, module.as_bytes()) {
        let caller_str = String::from_utf8_lossy(&caller_bytes);
        return GluResult::err(
            GluStatus::LinkDenied,
            &format!(
                "link denied: caller '{}' is not allowed to call module '{}'",
                caller_str, module
            ),
        );
    }

    #[cfg(unix)]
    let ipc_sock = crate::runtime::ipc_socket_for_module(module);
    #[cfg(unix)]
    let is_ipc = ipc_sock.is_some();
    #[cfg(not(unix))]
    let is_ipc = false;

    let module_bytes = module.as_bytes();
    let fn_bytes = function.as_bytes();
    let (entry_idx, wrapper) = match crate::handles::find_export(module_bytes, fn_bytes) {
        Some(v) => v,
        None => {
            let module_exists = crate::handles::glucore_get_module_export_count(
                std::ffi::CString::new(module).unwrap().as_ptr(),
            ) > 0;
            return if module_exists {
                GluResult::err(GluStatus::NotFound, "function not found in module")
            } else {
                GluResult::err(GluStatus::NotFound, "module not found")
            };
        }
    };

    if is_ipc {
        #[cfg(unix)]
        {
            let base = crate::runtime::ipc_base_idx_for_module(module).unwrap_or(0);
            let export_idx = base + entry_idx;
            let sock_fd = ipc_sock.unwrap();
            crate::runtime::IPC_CALL_CTX.with(|c| *c.borrow_mut() = Some((export_idx, sock_fd)));
            let r = wrapper(args, argc);
            crate::runtime::IPC_CALL_CTX.with(|c| *c.borrow_mut() = None);
            return r;
        }
        #[cfg(not(unix))]
        {
            unreachable!("is_ipc is always false on non-Unix")
        }
    }

    wrapper(args, argc)
}

// --- Public call entry points ----------------------------------------------

pub fn call_as(
    caller: &str,
    module: &str,
    function: &str,
    args: &[GluValue],
) -> GluResult {
    let _guard = CallerGuard::enter(caller);
    dispatch(module, function, args.as_ptr(), args.len())
}

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
