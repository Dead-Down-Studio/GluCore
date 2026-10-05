// runtime.rs — IPC execution, process module registration, transport.
//
// This module handles:
//   - IPC round-trips (send CALL, receive RESULT or CALLBACK_CALL)
//   - Process module registration (spawn, connect, read registration message)
//   - The IPC export metadata table and thread-local call context
//   - Socket connection with bounded retry
//
// All IPC code is #[cfg(unix)]-gated — Windows would need a named-pipe
// transport (documented gap, see transport.rs / KNOWN_FOOTGUNS.md).

use crate::types::*;
use crate::fctp;
use crate::router;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::{OnceLock, RwLock};

#[cfg(unix)]
use std::io::{Read, Write};

// --- IPC export metadata (Unix only) ---------------------------------------

/// Per-IPC-export metadata. Stored globally; wrappers look this up by index
/// to know which socket/function to call.
#[cfg(unix)]
pub(crate) struct IpcExportMeta {
    pub sock_fd: i32,
    pub module_name: String,
    pub function_name: String,
    pub sig: GluSignatureFFI,
}

#[cfg(unix)]
#[derive(Default)]
struct IpcState {
    exports: Vec<IpcExportMeta>,
    module_sockets: HashMap<Vec<u8>, i32>,
    module_base_idx: HashMap<Vec<u8>, usize>,
}

#[cfg(unix)]
static IPC_STATE: OnceLock<RwLock<IpcState>> = OnceLock::new();

#[cfg(unix)]
fn ipc_state() -> &'static RwLock<IpcState> {
    IPC_STATE.get_or_init(|| RwLock::new(IpcState::default()))
}

/// Thread-local context set by dispatch when about to call an IPC wrapper.
#[cfg(unix)]
thread_local! {
    pub(crate) static IPC_CALL_CTX: std::cell::RefCell<Option<(usize, i32)>> =
        std::cell::RefCell::new(None);
}

// --- IPC call execution ---------------------------------------------------

/// Perform a single IPC round-trip: send a CALL message on the socket and
/// read the response. The response may be either:
///   - MSG_RESULT (0x01): the final answer to the outer CALL — return it.
///   - MSG_CALLBACK_CALL (0x02): a nested request from Java asking Rust to
///     perform a glucore_call on Java's behalf (Task 10).
///
/// See doc/03-wire-protocol.md for the full sequencing rules.
#[cfg(unix)]
pub(crate) fn ipc_roundtrip(sock_fd: i32, msg: &[u8]) -> GluResult {
    let mut stream = unsafe { crate::transport::IpcStream::from_raw_fd(sock_fd) };
    let len_buf = (msg.len() as u32).to_le_bytes();
    if stream.write_all(&len_buf).is_err() || stream.write_all(msg).is_err() {
        stream.forget();
        return GluResult::err(GluStatus::Runtime, "ipc_write_failed");
    }
    loop {
        let mut len_arr = [0u8; 4];
        if stream.read_exact(&mut len_arr).is_err() {
            stream.forget();
            return GluResult::err(GluStatus::Runtime, "ipc_read_len_failed");
        }
        let resp_len = u32::from_le_bytes(len_arr) as usize;
        if resp_len > 16 * 1024 * 1024 {
            stream.forget();
            return GluResult::err(GluStatus::Runtime, "ipc_response_too_large");
        }
        let mut resp = vec![0u8; resp_len];
        if stream.read_exact(&mut resp).is_err() {
            stream.forget();
            return GluResult::err(GluStatus::Runtime, "ipc_read_body_failed");
        }
        if resp.is_empty() {
            stream.forget();
            return GluResult::err(GluStatus::Runtime, "ipc_empty_response");
        }
        match resp[0] {
            fctp::MSG_RESULT => {
                stream.forget();
                return fctp::decode_result(&resp).unwrap_or_else(|e| {
                    GluResult::err(GluStatus::Runtime, &format!("ipc_decode_result_failed:{e}"))
                });
            }
            fctp::MSG_CALLBACK_CALL => {
                let cb_request = match fctp::decode_callback_call(&resp) {
                    Ok(c) => c,
                    Err(e) => {
                        let err_payload = fctp::encode_callback_result_error(
                            &format!("callback_decode_error:{e}")
                        );
                        let err_len = (err_payload.len() as u32).to_le_bytes();
                        let _ = stream.write_all(&err_len);
                        let _ = stream.write_all(&err_payload);
                        continue;
                    }
                };

                if cb_request.module == "java_renderer" {
                    let err_payload = fctp::encode_callback_result_error(
                        "callback_self_reentry_denied"
                    );
                    let err_len = (err_payload.len() as u32).to_le_bytes();
                    let _ = stream.write_all(&err_len);
                    let _ = stream.write_all(&err_payload);
                    continue;
                }

                let _guard = router::CallerGuard::enter("java_renderer");
                let nested_result = crate::router::dispatch(
                    &cb_request.module,
                    &cb_request.function,
                    cb_request.args.as_ptr(),
                    cb_request.args.len(),
                );

                let cb_response = fctp::encode_callback_result(&nested_result);
                let cb_len = (cb_response.len() as u32).to_le_bytes();
                if stream.write_all(&cb_len).is_err() || stream.write_all(&cb_response).is_err() {
                    stream.forget();
                    return GluResult::err(GluStatus::Runtime, "ipc_write_callback_result_failed");
                }
            }
            other => {
                stream.forget();
                return GluResult::err(
                    GluStatus::Runtime,
                    &format!("ipc_unexpected_msg_type:{other:02x}"),
                );
            }
        }
    }
}

/// The single IPC wrapper function. All IPC export entries point at this
/// function. It reads its context (export index + socket FD) from the
/// thread-local IPC_CALL_CTX.
#[cfg(unix)]
extern "C" fn ipc_dispatch_wrapper(args: *const GluValue, argc: usize) -> GluResult {
    let (export_idx, sock_fd) = IPC_CALL_CTX.with(|c| {
        c.borrow().unwrap_or((0, -1))
    });
    if sock_fd < 0 {
        return GluResult::err(GluStatus::Runtime, "IPC wrapper called without context");
    }
    let state = match ipc_state().read() {
        Ok(s) => s,
        Err(_) => return GluResult::err(GluStatus::Runtime, "ipc_state_poisoned"),
    };
    let meta = if export_idx >= state.exports.len() {
        return GluResult::err(GluStatus::Runtime, "IPC export index out of range");
    } else {
        &state.exports[export_idx]
    };
    let arg_slice = unsafe { std::slice::from_raw_parts(args, argc) };
    let arg_tags = unsafe {
        std::slice::from_raw_parts(meta.sig.param_types, meta.sig.param_count)
    };
    let caller_buf = crate::router::current_caller_bytes().unwrap_or_else(|| b"unknown".to_vec());
    let caller = std::str::from_utf8(&caller_buf).unwrap_or("unknown");
    let msg = fctp::encode_call(
        &meta.module_name,
        &meta.function_name,
        caller,
        arg_slice,
        arg_tags,
    );
    ipc_roundtrip(sock_fd, &msg)
}

#[cfg(unix)]
fn get_ipc_wrapper() -> GluWrapper {
    ipc_dispatch_wrapper
}

// --- Registration message parsing ------------------------------------------

#[cfg(unix)]
struct IpcRegistration {
    module_name: String,
    exports: Vec<(String, GluSignatureFFI)>,
}

/// Parse the registration message from a PROCESS module.
#[cfg(unix)]
fn parse_registration(data: &[u8]) -> Result<IpcRegistration, String> {
    let mut off = 0usize;
    let name_bytes = read_len_bytes_pub(data, &mut off);
    let module_name = std::string::String::from_utf8_lossy(name_bytes).into_owned();
    if off >= data.len() {
        return Ok(IpcRegistration { module_name, exports: vec![] });
    }
    let export_count = data[off] as usize;
    off += 1;
    let mut exports = Vec::with_capacity(export_count);
    for _ in 0..export_count {
        let fn_bytes = read_len_bytes_pub(data, &mut off);
        let fn_name = std::string::String::from_utf8_lossy(fn_bytes).into_owned();
        let ret_tag = GluTypeTag::from_u8(data[off])?;
        off += 1;
        let param_count = data[off] as usize;
        off += 1;
        let mut tags = Vec::with_capacity(param_count);
        for _ in 0..param_count {
            tags.push(GluTypeTag::from_u8(data[off])?);
            off += 1;
        }
        let tags: &'static [GluTypeTag] = tags.leak();
        let sig = GluSignatureFFI {
            param_types: tags.as_ptr(),
            param_count,
            return_type: ret_tag,
        };
        exports.push((fn_name, sig));
    }
    Ok(IpcRegistration { module_name, exports })
}

#[cfg(unix)]
fn read_len_bytes_pub<'a>(data: &'a [u8], off: &mut usize) -> &'a [u8] {
    fctp::read_len_bytes(data, off)
}

// --- Registration entry point for PROCESS modules --------------------------

/// Register a PROCESS (separate-process) module. Spawns the given command,
/// connects to its Unix domain socket, reads the registration message, and
/// builds GluExportEntry wrappers backed by IPC.
#[cfg(unix)]
#[no_mangle]
pub extern "C" fn glucore_register_process_module(
    module_name: *const c_char,
    socket_path: *const c_char,
    spawn_cmd: *const c_char,
) -> i32 {
    let module_name = unsafe {
        if module_name.is_null() { return -1; }
        std::ffi::CStr::from_ptr(module_name).to_string_lossy().into_owned()
    };
    let socket_path = unsafe {
        if socket_path.is_null() { return -1; }
        std::ffi::CStr::from_ptr(socket_path).to_string_lossy().into_owned()
    };
    let spawn_cmd = unsafe {
        if spawn_cmd.is_null() { return -1; }
        std::ffi::CStr::from_ptr(spawn_cmd).to_string_lossy().into_owned()
    };

    let _ = std::fs::remove_file(&socket_path);

    let child = match std::process::Command::new("sh")
        .arg("-c")
        .arg(&spawn_cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("glucore: failed to spawn process module '{}': {}", module_name, e);
            return -2;
        }
    };

    let sock_fd = connect_with_retry(&socket_path, 5000, 50);
    if sock_fd < 0 {
        eprintln!("glucore: timed out connecting to process module '{}'", module_name);
        return -3;
    }

    let mut stream = unsafe { crate::transport::IpcStream::from_raw_fd(sock_fd) };
    let mut len_arr = [0u8; 4];
    if stream.read_exact(&mut len_arr).is_err() {
        eprintln!("glucore: failed to read registration len from '{}'", module_name);
        return -4;
    }
    stream.forget();
    let reg_len = u32::from_le_bytes(len_arr) as usize;
    if reg_len > 1024 * 1024 {
        eprintln!("glucore: registration message too large from '{}'", module_name);
        return -5;
    }
    let mut reg_data = vec![0u8; reg_len];
    let mut stream = unsafe { crate::transport::IpcStream::from_raw_fd(sock_fd) };
    if stream.read_exact(&mut reg_data).is_err() {
        eprintln!("glucore: failed to read registration body from '{}'", module_name);
        return -6;
    }
    stream.forget();

    let reg = match parse_registration(&reg_data) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("glucore: failed to parse registration from '{}': {}", module_name, e);
            return -7;
        }
    };

    let mut state = match ipc_state().write() {
        Ok(s) => s,
        Err(_) => {
            eprintln!("glucore: IPC state poisoned");
            return -8;
        }
    };
    let base_idx = state.exports.len();
    let mut entries: Vec<GluExportEntry> = Vec::new();
    let mut name_storage: Vec<CString> = Vec::new();
    for (i, (fn_name, sig)) in reg.exports.iter().enumerate() {
        let export_idx = base_idx + i;
        let wrapper = get_ipc_wrapper();
        name_storage.push(CString::new(fn_name.as_str()).unwrap());
        let meta = IpcExportMeta {
            sock_fd,
            module_name: module_name.clone(),
            function_name: fn_name.clone(),
            sig: *sig,
        };
        state.exports.push(meta);
        entries.push(GluExportEntry {
            name: name_storage.last().unwrap().as_ptr(),
            wrapper,
            signature: *sig,
        });
    }

    let entries: &'static [GluExportEntry] = entries.leak();
    let name_storage: &'static [CString] = name_storage.leak();
    let _ = name_storage.as_ptr();
    let c_name = CString::new(module_name.as_str()).unwrap();
    let glu_mod = GluModule {
        name: c_name.into_raw(),
        entries: entries.as_ptr(),
        count: entries.len(),
    };
    unsafe { crate::handles::glucore_register_module(glu_mod) };

    state.module_sockets.insert(module_name.as_bytes().to_vec(), sock_fd);
    state.module_base_idx.insert(module_name.as_bytes().to_vec(), base_idx);

    std::mem::forget(child);
    0
}

/// Connect to a Unix domain socket with bounded retry.
#[cfg(unix)]
fn connect_with_retry(path: &str, timeout_ms: u64, interval_ms: u64) -> i32 {
    let start = std::time::Instant::now();
    loop {
        match crate::transport::IpcStream::connect(path) {
            Ok(s) => {
                let fd = s.as_raw_fd();
                s.forget();
                return fd;
            }
            Err(_) => {
                if start.elapsed().as_millis() as u64 >= timeout_ms {
                    return -1;
                }
                std::thread::sleep(std::time::Duration::from_millis(interval_ms));
            }
        }
    }
}

// --- IPC module lookup helpers ---------------------------------------------

#[cfg(unix)]
pub(crate) fn ipc_socket_for_module(module: &str) -> Option<i32> {
    let state = ipc_state().read().ok()?;
    state.module_sockets.get(module.as_bytes()).copied()
}

#[cfg(unix)]
pub(crate) fn ipc_base_idx_for_module(module: &str) -> Option<usize> {
    let state = ipc_state().read().ok()?;
    state.module_base_idx.get(module.as_bytes()).copied()
}
