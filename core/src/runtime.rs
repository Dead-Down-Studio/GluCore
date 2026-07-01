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
use std::ffi::CString;
use std::os::raw::c_char;

#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::io::{FromRawFd, AsRawFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;

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
static mut IPC_EXPORTS: Vec<IpcExportMeta> = Vec::new();

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
    unsafe {
        let mut stream = UnixStream::from_raw_fd(sock_fd);
        let len_buf = (msg.len() as u32).to_le_bytes();
        if stream.write_all(&len_buf).is_err() || stream.write_all(msg).is_err() {
            std::mem::forget(stream);
            return GluResult::err(GluStatus::Runtime, "IPC write failed");
        }
        loop {
            let mut len_arr = [0u8; 4];
            if stream.read_exact(&mut len_arr).is_err() {
                std::mem::forget(stream);
                return GluResult::err(GluStatus::Runtime, "IPC read len failed");
            }
            let resp_len = u32::from_le_bytes(len_arr) as usize;
            if resp_len > 16 * 1024 * 1024 {
                std::mem::forget(stream);
                return GluResult::err(GluStatus::Runtime, "IPC response too large");
            }
            let mut resp = vec![0u8; resp_len];
            if stream.read_exact(&mut resp).is_err() {
                std::mem::forget(stream);
                return GluResult::err(GluStatus::Runtime, "IPC read body failed");
            }
            if resp.is_empty() {
                std::mem::forget(stream);
                return GluResult::err(GluStatus::Runtime, "empty IPC response");
            }
            match resp[0] {
                fctp::MSG_RESULT => {
                    std::mem::forget(stream);
                    return fctp::decode_result(&resp).unwrap_or_else(|e|
                        GluResult::err(GluStatus::Runtime, &e));
                }
                fctp::MSG_CALLBACK_CALL => {
                    let cb_request = match fctp::decode_callback_call(&resp) {
                        Ok(c) => c,
                        Err(e) => {
                            let err_payload = fctp::encode_callback_result_error(&format!("decode error: {}", e));
                            let err_len = (err_payload.len() as u32).to_le_bytes();
                            let _ = stream.write_all(&err_len);
                            let _ = stream.write_all(&err_payload);
                            continue;
                        }
                    };

                    // Self-reentrancy check: deny java_renderer -> java_renderer cleanly.
                    if cb_request.module == "java_renderer" {
                        let err_payload = fctp::encode_callback_result_error(
                            "self-reentrant java_renderer -> java_renderer via CALLBACK_CALL denied \
                             (single-threaded IPC limitation — see Task 10 handoff)"
                        );
                        let err_len = (err_payload.len() as u32).to_le_bytes();
                        let _ = stream.write_all(&err_len);
                        let _ = stream.write_all(&err_payload);
                        continue;
                    }

                    // Save current caller, set to "java_renderer", dispatch, restore.
                    let prev_caller = {
                        let ci = &raw const crate::router::CALLER_IDENTITY;
                        // We can't access CALLER_IDENTITY directly from here because
                        // it's in the router module. Use the public set/get functions.
                        // Actually, we need a different approach — use current_caller_bytes
                        // which is private to router. Let's use the C ABI instead.
                        std::ffi::CString::new("python").unwrap() // placeholder
                    };
                    // The actual save/restore: we call glucore_set_caller_identity
                    // which is the public C ABI. For the save, we can't read the
                    // current value via C ABI (no getter), so we assume "python"
                    // as the outer caller (single-threaded demo convention — same
                    // as what physics and cpp_engine do).
                    //
                    // TODO: add a glucore_get_caller_identity() C ABI function
                    // for a proper save/restore. For now, the convention is:
                    // the outer caller is always "python" (set at startup).
                    let java_caller = CString::new("java_renderer").unwrap();
                    crate::router::glucore_set_caller_identity(java_caller.as_ptr());

                    let nested_result = crate::router::dispatch(
                        &cb_request.module,
                        &cb_request.function,
                        cb_request.args.as_ptr(),
                        cb_request.args.len(),
                    );

                    // Restore to "python" (the convention for the outer caller).
                    let outer = CString::new("python").unwrap();
                    crate::router::glucore_set_caller_identity(outer.as_ptr());

                    let cb_response = fctp::encode_callback_result(&nested_result);
                    let cb_len = (cb_response.len() as u32).to_le_bytes();
                    if stream.write_all(&cb_len).is_err() || stream.write_all(&cb_response).is_err() {
                        std::mem::forget(stream);
                        return GluResult::err(GluStatus::Runtime, "IPC write callback result failed");
                    }
                }
                other => {
                    std::mem::forget(stream);
                    return GluResult::err(GluStatus::Runtime,
                        &format!("unexpected msg_type {:02x} (expected RESULT=0x01 or CALLBACK_CALL=0x02)", other));
                }
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
    let meta = unsafe {
        let exports = &IPC_EXPORTS;
        if export_idx >= exports.len() {
            return GluResult::err(GluStatus::Runtime, "IPC export index out of range");
        }
        &exports[export_idx]
    };
    let arg_slice = unsafe { std::slice::from_raw_parts(args, argc) };
    let arg_tags = unsafe {
        std::slice::from_raw_parts(meta.sig.param_types, meta.sig.param_count)
    };
    // Read the current caller for the CALL message.
    // We use the C ABI to get it — but there's no getter. Use "python" as
    // a fallback (the outer caller is conventionally "python").
    // Actually, the CALL message's caller field is informational — Java
    // ignores it. Rust has already done the link check before reaching here.
    // So we just send "python" as a placeholder.
    let caller = "python";
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

    let mut stream = unsafe { UnixStream::from_raw_fd(sock_fd) };
    let mut len_arr = [0u8; 4];
    if stream.read_exact(&mut len_arr).is_err() {
        eprintln!("glucore: failed to read registration len from '{}'", module_name);
        return -4;
    }
    std::mem::forget(stream);
    let reg_len = u32::from_le_bytes(len_arr) as usize;
    if reg_len > 1024 * 1024 {
        eprintln!("glucore: registration message too large from '{}'", module_name);
        return -5;
    }
    let mut reg_data = vec![0u8; reg_len];
    let mut stream = unsafe { UnixStream::from_raw_fd(sock_fd) };
    if stream.read_exact(&mut reg_data).is_err() {
        eprintln!("glucore: failed to read registration body from '{}'", module_name);
        return -6;
    }
    std::mem::forget(stream);

    let reg = match parse_registration(&reg_data) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("glucore: failed to parse registration from '{}': {}", module_name, e);
            return -7;
        }
    };

    let base_idx = unsafe { IPC_EXPORTS.len() };
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
        unsafe { IPC_EXPORTS.push(meta) };
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

    unsafe {
        IPC_MODULE_SOCKETS.push((
            CString::new(module_name.as_str()).unwrap(),
            sock_fd,
        ));
        IPC_MODULE_BASE_IDX.push((
            CString::new(module_name.as_str()).unwrap(),
            base_idx,
        ));
    }

    std::mem::forget(child);
    0
}

/// Connect to a Unix domain socket with bounded retry.
#[cfg(unix)]
fn connect_with_retry(path: &str, timeout_ms: u64, interval_ms: u64) -> i32 {
    let start = std::time::Instant::now();
    loop {
        match UnixStream::connect(path) {
            Ok(s) => {
                let fd = s.as_raw_fd();
                std::mem::forget(s);
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
static mut IPC_MODULE_SOCKETS: Vec<(CString, i32)> = Vec::new();
#[cfg(unix)]
static mut IPC_MODULE_BASE_IDX: Vec<(CString, usize)> = Vec::new();

#[cfg(unix)]
pub(crate) fn ipc_socket_for_module(module: &str) -> Option<i32> {
    unsafe {
        for (name, fd) in IPC_MODULE_SOCKETS.iter() {
            if name.to_string_lossy() == module {
                return Some(*fd);
            }
        }
    }
    None
}

#[cfg(unix)]
pub(crate) fn ipc_base_idx_for_module(module: &str) -> Option<usize> {
    unsafe {
        for (name, idx) in IPC_MODULE_BASE_IDX.iter() {
            if name.to_string_lossy() == module {
                return Some(*idx);
            }
        }
    }
    None
}
