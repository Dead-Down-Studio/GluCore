// fctp.rs — Function Call Transport Protocol.
//
// The binary wire protocol used between the Rust core and PROCESS modules
// (currently only the Java renderer). Little-endian, length-prefixed,
// synchronous.
//
// Message types:
//   0x00 CALL             (Rust → Process)  Invoke a function
//   0x01 RESULT           (Process → Rust)  Return the result of a CALL
//   0x02 CALLBACK_CALL    (Process → Rust)  Task 10: Process asks Rust to
//                                           perform a glucore_call on its behalf
//   0x03 CALLBACK_RESULT  (Rust → Process)  Task 10: Rust's answer to CALLBACK_CALL

use crate::types::*;
use std::os::raw::c_char;

/// Wire-protocol constants.
pub const MSG_CALL: u8 = 0x00;
pub const MSG_RESULT: u8 = 0x01;
pub const MSG_CALLBACK_CALL: u8 = 0x02;
pub const MSG_CALLBACK_RESULT: u8 = 0x03;

// --- Encoding helpers (little-endian) --------------------------------------

pub(crate) fn write_u32_le(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

pub(crate) fn write_u64_le(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

pub(crate) fn write_len_bytes(buf: &mut Vec<u8>, data: &[u8]) {
    write_u32_le(buf, data.len() as u32);
    buf.extend_from_slice(data);
}

pub(crate) fn write_len_str(buf: &mut Vec<u8>, s: &str) {
    write_len_bytes(buf, s.as_bytes());
}

// --- Decoding helpers ------------------------------------------------------

pub(crate) fn read_u32_le(data: &[u8], off: &mut usize) -> u32 {
    let v = u32::from_le_bytes(data[*off..*off + 4].try_into().unwrap());
    *off += 4;
    v
}

pub(crate) fn read_u64_le(data: &[u8], off: &mut usize) -> u64 {
    let v = u64::from_le_bytes(data[*off..*off + 8].try_into().unwrap());
    *off += 8;
    v
}

pub(crate) fn read_len_bytes<'a>(data: &'a [u8], off: &mut usize) -> &'a [u8] {
    let len = read_u32_le(data, off) as usize;
    let slice = &data[*off..*off + len];
    *off += len;
    slice
}

// --- CALL message encoding -------------------------------------------------

/// Encode a call message per the wire protocol spec.
#[cfg(unix)]
pub(crate) fn encode_call(
    module: &str,
    function: &str,
    caller: &str,
    args: &[GluValue],
    arg_tags: &[GluTypeTag],
) -> Vec<u8> {
    let mut payload = Vec::new();
    write_len_str(&mut payload, module);
    write_len_str(&mut payload, function);
    write_len_str(&mut payload, caller);
    payload.push(args.len() as u8);
    for (arg, tag) in args.iter().zip(arg_tags.iter()) {
        payload.push(*tag as u8);
        match tag {
            GluTypeTag::Float => {
                let v = unsafe { arg.float };
                write_u64_le(&mut payload, v.to_bits());
            }
            GluTypeTag::Int => {
                let v = unsafe { arg.int };
                write_u64_le(&mut payload, v as u64);
            }
            GluTypeTag::String => {
                let s = unsafe { &arg.string };
                let bytes = unsafe { std::slice::from_raw_parts(s.ptr, s.len) };
                write_len_bytes(&mut payload, bytes);
            }
            GluTypeTag::Buffer => {
                let b = unsafe { &arg.buffer };
                let bytes = unsafe { std::slice::from_raw_parts(b.ptr, b.len) };
                write_len_bytes(&mut payload, bytes);
            }
            GluTypeTag::Handle | GluTypeTag::Void => {
                // Shouldn't appear as an argument, but gracefully skip.
            }
        }
    }

    let mut msg = Vec::new();
    msg.push(MSG_CALL);
    write_u32_le(&mut msg, payload.len() as u32);
    msg.extend_from_slice(&payload);
    msg
}

// --- RESULT message decoding -----------------------------------------------

/// Decode a result message from Java. Returns a GluResult.
#[cfg(unix)]
pub(crate) fn decode_result(data: &[u8]) -> Result<GluResult, String> {
    if data.is_empty() {
        return Err("empty result message".into());
    }
    if data[0] != MSG_RESULT {
        return Err(format!("unexpected msg_type {:02x}", data[0]));
    }
    let mut off = 1usize;
    let _payload_len = read_u32_le(data, &mut off);
    let status = data[off];
    off += 1;
    if status == 0 {
        // OK — read return tag + value
        if off >= data.len() {
            return Ok(GluResult::ok(GluValue { int: 0 }));
        }
        let tag = data[off];
        off += 1;
        let tag = GluTypeTag::from_u8(tag)?;
        match tag {
            GluTypeTag::Float => {
                let bits = read_u64_le(data, &mut off);
                Ok(GluResult::ok(GluValue { float: f64::from_bits(bits) }))
            }
            GluTypeTag::Int => {
                let v = read_u64_le(data, &mut off);
                Ok(GluResult::ok(GluValue { int: v as i64 }))
            }
            GluTypeTag::String | GluTypeTag::Buffer => {
                let slice = read_len_bytes(data, &mut off);
                let mut buf = slice.to_vec();
                let ptr = buf.as_mut_ptr();
                let len = buf.len();
                std::mem::forget(buf);
                let glu_slice = GluSlice { ptr, len };
                let mut val = GluValue { int: 0 };
                if tag == GluTypeTag::String {
                    unsafe { val.string = glu_slice; }
                } else {
                    unsafe { val.buffer = glu_slice; }
                }
                Ok(GluResult::ok(val))
            }
            GluTypeTag::Void => Ok(GluResult::ok(GluValue { int: 0 })),
            GluTypeTag::Handle => Ok(GluResult::ok(GluValue { int: 0 })),
        }
    } else {
        let msg_bytes = read_len_bytes(data, &mut off);
        let msg = std::string::String::from_utf8_lossy(msg_bytes).into_owned();
        Ok(GluResult::err(GluStatus::Runtime, &msg))
    }
}

// --- CALLBACK_CALL decoding (Task 10) --------------------------------------

/// Decoded CALLBACK_CALL request from Java.
#[cfg(unix)]
pub(crate) struct CallbackCall {
    pub module: String,
    pub function: String,
    pub args: Vec<GluValue>,
    pub arg_tags: Vec<GluTypeTag>,
}

/// Decode a CALLBACK_CALL message body.
#[cfg(unix)]
pub(crate) fn decode_callback_call(data: &[u8]) -> Result<CallbackCall, String> {
    if data.is_empty() {
        return Err("empty callback call message".into());
    }
    if data[0] != MSG_CALLBACK_CALL {
        return Err(format!("expected CALLBACK_CALL (0x02), got {:02x}", data[0]));
    }
    let mut off = 1usize;
    let _payload_len = read_u32_le(data, &mut off);
    let module = std::string::String::from_utf8_lossy(read_len_bytes(data, &mut off)).into_owned();
    let function = std::string::String::from_utf8_lossy(read_len_bytes(data, &mut off)).into_owned();
    let _caller = read_len_bytes(data, &mut off);
    let argc = data.get(off).copied().ok_or("missing argc")? as usize;
    off += 1;

    let mut args = Vec::with_capacity(argc);
    let mut arg_tags = Vec::with_capacity(argc);
    for _ in 0..argc {
        let tag_byte = data.get(off).copied().ok_or("missing arg tag")?;
        off += 1;
        let tag = GluTypeTag::from_u8(tag_byte)?;
        let value = match tag {
            GluTypeTag::Float => {
                let bits = read_u64_le(data, &mut off);
                GluValue { float: f64::from_bits(bits) }
            }
            GluTypeTag::Int => {
                let v = read_u64_le(data, &mut off);
                GluValue { int: v as i64 }
            }
            GluTypeTag::String | GluTypeTag::Buffer => {
                let slice = read_len_bytes(data, &mut off);
                let mut buf = slice.to_vec();
                let ptr = buf.as_mut_ptr();
                let len = buf.len();
                std::mem::forget(buf);
                let glu_slice = GluSlice { ptr, len };
                let mut val = GluValue { int: 0 };
                if tag == GluTypeTag::String {
                    unsafe { val.string = glu_slice; }
                } else {
                    unsafe { val.buffer = glu_slice; }
                }
                val
            }
            GluTypeTag::Void | GluTypeTag::Handle => GluValue { int: 0 },
        };
        args.push(value);
        arg_tags.push(tag);
    }
    Ok(CallbackCall { module, function, args, arg_tags })
}

// --- CALLBACK_RESULT encoding (Task 10) ------------------------------------

/// Encode a GluResult as a CALLBACK_RESULT message body.
#[cfg(unix)]
pub(crate) fn encode_callback_result(result: &GluResult) -> Vec<u8> {
    let mut payload = Vec::new();
    if result.status == GluStatus::Ok {
        payload.push(0u8); // status OK
        payload.push(GluTypeTag::Float as u8);
        let bits = unsafe { result.value.float }.to_bits();
        write_u64_le(&mut payload, bits);
    } else {
        payload.push(1u8); // status error
        let msg = if result.message.is_null() {
            "(no message)".as_bytes()
        } else {
            unsafe { std::ffi::CStr::from_ptr(result.message).to_bytes() }
        };
        write_len_bytes(&mut payload, msg);
    }
    let mut msg = Vec::new();
    msg.push(MSG_CALLBACK_RESULT);
    write_u32_le(&mut msg, payload.len() as u32);
    msg.extend_from_slice(&payload);
    msg
}

/// Encode an error CALLBACK_RESULT with a message.
#[cfg(unix)]
pub(crate) fn encode_callback_result_error(msg: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(1u8);
    write_len_bytes(&mut payload, msg.as_bytes());
    let mut out = Vec::new();
    out.push(MSG_CALLBACK_RESULT);
    write_u32_le(&mut out, payload.len() as u32);
    out.extend_from_slice(&payload);
    out
}
