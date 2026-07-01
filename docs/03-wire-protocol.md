# 03 — Wire Protocol

This document specifies the binary wire protocol used between the Rust
core and PROCESS modules (currently only the Java renderer). The
protocol is little-endian, length-prefixed, and synchronous.

## Message types

| Byte | Name | Direction | Purpose |
|---|---|---|---|
| 0x00 | `CALL` | Rust → Process | Invoke a function in the process module |
| 0x01 | `RESULT` | Process → Rust | Return the result of a CALL |
| 0x02 | `CALLBACK_CALL` | Process → Rust | (Task 10) Process asks Rust to perform a glucore_call on its behalf |
| 0x03 | `CALLBACK_RESULT` | Rust → Process | (Task 10) Rust's answer to a CALLBACK_CALL |

## Framing

Every message is framed as:

```
[ 4 bytes: total_msg_len (u32 LE) ][ total_msg_len bytes: message body ]
```

The length prefix is the size of the message body, NOT including the
length prefix itself. The body always starts with a 1-byte message type.

This framing is symmetric — both directions use it. A misimplemented
framing on either side causes the other side to read garbage as a
length, typically hanging or crashing. See footgun #7 in
`KNOWN_FOOTGUNS.md`.

## CALL message (0x00)

Sent by Rust to invoke a function in the process module.

```
[ 1 byte: msg_type = 0x00 ]
[ 4 bytes: payload_len (u32 LE) ]
[ payload_len bytes: payload ]
```

Payload layout:

```
[ 4 bytes: module name len (u32 LE) ][ N bytes: module name (UTF-8) ]
[ 4 bytes: function name len (u32 LE) ][ N bytes: function name (UTF-8) ]
[ 4 bytes: caller id len (u32 LE) ][ N bytes: caller id (UTF-8) ]
[ 1 byte: arg count ]
per arg:
  [ 1 byte: GluTypeTag ]
  tag-dependent value:
    Int:    [ 8 bytes: i64 LE ]
    Float:  [ 8 bytes: f64 LE (IEEE 754 bits) ]
    String: [ 4 bytes: len (u32 LE) ][ N bytes: UTF-8 ]
    Buffer: [ 4 bytes: len (u32 LE) ][ N bytes: raw ]
    Handle: [ 8 bytes: u64 ]
    Void:   (no payload — shouldn't appear as an arg)
```

**Important:** the tag and value are INTERLEAVED per arg, not packed as
"all tags then all values." See footgun #6 in `KNOWN_FOOTGUNS.md`.

## RESULT message (0x01)

Sent by the process module to return the result of a CALL.

```
[ 1 byte: msg_type = 0x01 ]
[ 4 bytes: payload_len (u32 LE) ]
[ payload_len bytes: payload ]
```

Payload layout (OK case):

```
[ 1 byte: status = 0 (OK) ]
[ 1 byte: return tag (GluTypeTag) ]
tag-dependent value (same encoding as CALL args)
```

Payload layout (error case):

```
[ 1 byte: status = 1 (error) ]
[ 4 bytes: msg len (u32 LE) ][ N bytes: error message (UTF-8) ]
```

The status byte uses 0/1, NOT the `GluStatus` enum values. (The
`GluStatus::LinkDenied = 4` value is for the Rust-internal result, not
the wire protocol — a denied-link CALL never reaches the process module
in the first place, per Constraint #8.)

## CALLBACK_CALL message (0x02) — Task 10

Sent by the process module WHILE HANDLING an inbound CALL, asking Rust
to perform a glucore_call on its behalf. Layout is identical to CALL's
payload:

```
[ 1 byte: msg_type = 0x02 ]
[ 4 bytes: payload_len (u32 LE) ]
[ payload_len bytes: payload ]
```

Payload (same as CALL):

```
[ 4 bytes: module name len ][ N bytes: module name ]
[ 4 bytes: function name len ][ N bytes: function name ]
[ 4 bytes: caller id len ][ N bytes: caller id ]  ← ignored by Rust; Rust sets caller to the process module's name itself
[ 1 byte: arg count ]
per arg: [ 1 byte tag ][ tag-dependent value ]
```

The caller id field is included for symmetry with CALL but Rust ignores
it — Rust always sets the caller identity to the process module's name
(e.g. `"java_renderer"`) before dispatching the nested call, so the link
check sees the correct caller. This prevents a process module from
impersonating another caller.

## CALLBACK_RESULT message (0x03) — Task 10

Rust's answer to a CALLBACK_CALL. Layout is identical to RESULT's
payload:

```
[ 1 byte: msg_type = 0x03 ]
[ 4 bytes: payload_len (u32 LE) ]
[ payload_len bytes: payload ]
```

Payload (OK):
```
[ 1 byte: status = 0 (OK) ]
[ 1 byte: return tag ]
tag-dependent value
```

Payload (error):
```
[ 1 byte: status = 1 (error) ]
[ 4 bytes: msg len ][ N bytes: error message ]
```

The error message for a denied link includes the literal denial reason
(e.g. `"link denied: caller 'java_renderer' is not allowed to call
module 'cpp_engine'"`). For a self-reentrant denial, the message is
`"self-reentrant java_renderer -> java_renderer via CALLBACK_CALL denied
(single-threaded IPC limitation — see Task 10 handoff)"`.

## Sequencing rules (Task 10)

The protocol is strictly synchronous and single-threaded:

1. Rust sends a CALL.
2. Rust enters a loop reading messages:
   - If it reads a `RESULT` → decode and return (the outer call is done).
   - If it reads a `CALLBACK_CALL` → decode, perform the nested
     `glucore_call`, send a `CALLBACK_RESULT`, continue the loop.
3. The process module, while executing the function called via the outer
   CALL, may send a `CALLBACK_CALL` and then MUST block reading until it
   receives the corresponding `CALLBACK_RESULT`. It must NOT send a
   second `CALLBACK_CALL` until the first one's `CALLBACK_RESULT` is
   received.
4. After the process module's function returns, it sends the outer
   `RESULT`. Rust receives this in the loop and returns.

This means at most ONE nested round-trip is in flight at any time.
Concurrent/interleaved nested calls would require correlation IDs and
queuing — explicitly out of scope for Task 10.

## Self-reentrancy

A `CALLBACK_CALL` whose target is the same process module that sent it
(e.g. `java_renderer → java_renderer`) would deadlock the synchronous
protocol: Rust would block sending to the socket while the process
module is blocked waiting for the outer CALL's response.

Rust detects this case and denies it cleanly: the `CALLBACK_CALL` is
NOT dispatched; instead, Rust sends an error `CALLBACK_RESULT` with the
message `"self-reentrant ... denied (single-threaded IPC limitation)"`.
The process module receives this as a normal error it can handle or
propagate.

## Endianness

All multi-byte integers are little-endian. This matches x86_64 and
ARM64 (the only platforms currently tested). If GluCore is ever ported
to a big-endian platform, the wire protocol stays little-endian (use
`to_le_bytes` / `from_le_bytes`); only the in-memory representation
changes.

## Versioning

There is currently no explicit version field in the protocol. The
registration message (sent once at process startup) implicitly versions
the module by listing its exports and their signatures. If the protocol
itself changes (e.g. adding a new message type), the registration
message would need a version field — reserved for future work.

## Reference: encode_call in Rust

```rust
fn encode_call(module: &str, function: &str, caller: &str,
               args: &[GluValue], arg_tags: &[GluTypeTag]) -> Vec<u8> {
    let mut payload = Vec::new();
    write_len_str(&mut payload, module);
    write_len_str(&mut payload, function);
    write_len_str(&mut payload, caller);
    payload.push(args.len() as u8);
    for (arg, tag) in args.iter().zip(arg_tags.iter()) {
        payload.push(*tag as u8);
        match tag {
            GluTypeTag::Float => write_u64_le(&mut payload, unsafe { arg.float }.to_bits()),
            GluTypeTag::Int   => write_u64_le(&mut payload, unsafe { arg.int } as u64),
            GluTypeTag::String => write_len_bytes(&mut payload, unsafe { std::slice::from_raw_parts(arg.string.ptr, arg.string.len) }),
            GluTypeTag::Buffer => write_len_bytes(&mut payload, unsafe { std::slice::from_raw_parts(arg.buffer.ptr, arg.buffer.len) }),
            GluTypeTag::Handle | GluTypeTag::Void => {} // shouldn't appear as an arg
        }
    }
    let mut msg = Vec::new();
    msg.push(MSG_CALL);
    write_u32_le(&mut msg, payload.len() as u32);
    msg.extend_from_slice(&payload);
    msg
}
```

## Reference: dispatchCall in Java

```java
private static byte[] dispatchCall(byte[] payload) throws Exception {
    int[] off = new int[]{0};
    String module = new String(readLenBytes(payload, off), "UTF-8");
    String function = new String(readLenBytes(payload, off), "UTF-8");
    String caller = new String(readLenBytes(payload, off), "UTF-8"); // ignored
    int argc = payload[off[0]] & 0xFF; off[0] += 1;
    // ... dispatch on `function` name, reading args per the interleaved tag+value layout
}
```
