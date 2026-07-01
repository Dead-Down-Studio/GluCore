# 06 — IPC Transport

This document covers how the Rust core talks to PROCESS modules
(currently only the Java renderer) over a Unix domain socket, the
byte-counter evidence for Constraint #8, and the portability story.

## Transport selection

GluCore selects the transport at module-load time based on how the
module was registered:

| Registration path | Transport | Example |
|---|---|---|
| `glucore.load_module(core, name)` | ARTIFACT (direct function pointer) | physics, cpp_engine |
| `glucore.load_process_module(core, name, socket, cmd)` | PROCESS (Unix domain socket) | java_renderer |

The dispatch path is the same up to the wrapper call. For ARTIFACT
modules, the wrapper is a real function pointer in the dlopen'd `.so`.
For PROCESS modules, the wrapper is `ipc_dispatch_wrapper` — a single
function that reads its context (export index + socket fd) from a
thread-local and performs an IPC round-trip.

## The IPC round-trip (without callbacks)

```
Python:  java.scale(3.0, 2.0)
   │
   ▼
glucore.py: pack args, call lib.glucore_call("java_renderer", "scale", ...)
   ▼
libglucore_core.so: dispatch
   │  caller = "python", callee = "java_renderer", link OK
   │  → find "java_renderer" in REGISTRY
   │  → find "scale" in entries
   │  → entry.wrapper is ipc_dispatch_wrapper
   │  → set IPC_CALL_CTX thread-local = (export_idx, sock_fd)
   │  → call ipc_dispatch_wrapper(args, argc)
   ▼
ipc_dispatch_wrapper:
   │  read (export_idx, sock_fd) from thread-local
   │  look up IpcExportMeta[export_idx] for module name, fn name, signature
   │  encode CALL message
   │  call ipc_roundtrip(sock_fd, msg)
   ▼
ipc_roundtrip:
   │  write [u32 msg_len][msg] to socket
   │  read [u32 resp_len][resp] from socket
   │  decode resp as RESULT
   │  return GluResult
   ▼
(back up the stack)
   ▼
Python: result = 6.0
```

## The IPC round-trip (with Task 10 callbacks)

```
ipc_roundtrip:
   │  write [u32 msg_len][CALL msg] to socket
   │  loop:
   │    read [u32 resp_len][resp] from socket
   │    if resp[0] == MSG_RESULT (0x01):
   │      decode and return GluResult
   │    if resp[0] == MSG_CALLBACK_CALL (0x02):
   │      decode the callback request
   │      if self-reentrant (java_renderer -> java_renderer):
   │        send error CALLBACK_RESULT, continue loop
   │      save current caller, set caller = "java_renderer"
   │      dispatch(module, function, args, argc)  ← may recurse into ipc_roundtrip if target is also IPC
   │      restore previous caller
   │      send CALLBACK_RESULT with the nested result
   │      continue loop
```

The loop is the key Task 10 addition. Without it, Rust would read the
CALLBACK_CALL as if it were a RESULT, fail to decode it, and return a
Runtime error — leaving Java blocked waiting for a CALLBACK_RESULT that
never comes.

## Unix domain socket specifics

- The Java process binds a Unix domain socket at a path passed as
  `argv[1]`. The Rust core connects to it (with bounded retry, 5s
  timeout, 50ms interval).
- The Rust core reads the registration message immediately on connect.
  The registration message lists the module's exports and their
  signatures.
- After registration, the socket is used for CALL/RESULT traffic in
  both directions.
- The socket fd is owned by the core (stored in `IPC_MODULE_SOCKETS`).
  It's reused for every call to that module — NOT reconnected per call.
- The Java process exits when the socket closes (EOF on read).

## The byte-counter evidence (Constraint #8)

Constraint #8: "the Rust core checks caller→callee links BEFORE sending
anything over the socket."

The original Phase 2 report supported this claim only with "the call
failed fast" — which doesn't distinguish "denied before sending" from
"sent, and something else made it fail quickly."

Part 0c adds the missing evidence: a static byte-counter on the Java
side (`Renderer.bytesReadFromSocket`) that increments on every byte
read from the socket. The counter is exposed via the
`glucore_byte_read_count` export so Python can read it after triggering
a denied-link case.

The Part 0c DoD test:
1. Makes a successful call → counter increases by 64 bytes (the size of
   a CALL message for `scale(Float, Float)`).
2. Triggers a denied-link call (impersonating an undeclared caller) →
   counter increases by 0 bytes.
3. Makes another successful call → counter increases by 64 bytes again,
   proving the socket is still usable.

The "counter-query overhead" (each call to `glucore_byte_read_count`
itself sends a CALL that adds 64 bytes to the counter) is calibrated
out by measuring two consecutive counter queries with nothing in
between.

This is byte-level evidence, not inference. The denied-link case
exchanged exactly 0 bytes — proving the Rust core never sent anything
to Java for that call.

## Portability

### Linux

- Fully supported. Unix domain sockets work the same as on macOS.
- `RTLD_NOLOAD = 0x4`, `RTLD_LAZY = 0x1`, `RTLD_GLOBAL = 0x100`.
- `/proc/self/maps` is used by the physics module's `coreffi::abi()` to
  find its own `.so` path (so it can locate `libglucore_core.so` in the
  same directory).

### macOS

- Fully supported. Unix domain sockets work the same as on Linux.
- `RTLD_NOLOAD = 0x8` (different from Linux!), `RTLD_LAZY = 0x1`,
  `RTLD_GLOBAL = 0x100`.
- `/proc/self/maps` doesn't exist — the physics module falls back to
  cwd-relative path lookup.
- The CMakeLists.txt sets `BUILD_RPATH` to `@loader_path` (macOS
  equivalent of `$ORIGIN`).

### Windows

- NOT supported. Documented gap, not a silent one.
- Unix domain sockets are NOT reliably available across all Windows
  versions and JDKs. Java 22+ has some support but earlier versions
  don't.
- The idiomatic Windows IPC primitive is named pipes (`\\.\pipe\name`).
  A future Windows port would implement `IpcStream` /
  `IpcListener` for named pipes behind the same trait — see
  `glucore_core/src/ipc_transport.rs`.
- A loopback-TCP fallback is an acceptable interim solution per the
  Task 11 handoff. Silently dropping Windows IPC support without saying
  so is NOT.

### The IpcTransport abstraction

`glucore_core/src/ipc_transport.rs` defines an `IpcStream` /
`IpcListener` trait with:

| Method | Unix impl | Windows (future) |
|---|---|---|
| `IpcStream::connect(path)` | `UnixStream::connect` | `CreateFile` on pipe name |
| `IpcListener::bind(path)` | `UnixListener::bind` | `CreateNamedPipe` |
| `IpcStream::as_raw_fd()` | `as_raw_fd()` | (would return HANDLE) |
| `cleanup_endpoint(path)` | `fs::remove_file` | no-op (pipes don't outlive process) |

The current code uses `UnixStream` directly in `ipc_roundtrip` rather
than `IpcStream` — the abstraction is in place for future porting but
not yet wired into the dispatch path. Wiring it in is a mechanical
refactor that doesn't change behavior.

## Connection lifecycle

```
1. Python: java = glucore.load_process_module(core, "java_renderer", sock, cmd)
2. Rust core: glucore_register_process_module("java_renderer", sock, cmd)
   a. Remove stale socket file if present.
   b. Spawn the Java process via `sh -c "java -cp ... Renderer <sock>"`.
   c. Connect to the socket with bounded retry (5s, 50ms intervals).
   d. Read the registration message: [u32 len][body].
   e. Parse the body: module name, export count, per-export (name, ret tag, param tags).
   f. Build GluExportEntry wrappers (all pointing at ipc_dispatch_wrapper).
   g. Leak the entries + name storage for module lifetime.
   h. Push the module into REGISTRY.
   i. Record (module_name, sock_fd) in IPC_MODULE_SOCKETS for dispatch lookup.
   j. Forget the child handle so the JVM outlives the registration function.
3. Python: GluProxy(core, "java_renderer", module_info=None)
   a. Enumerate exports via glucore_get_module_export_count / _name (Task 9 additions).
   b. For each export, query signature via glucore_get_signature.
   c. Build an inlined caller closure per export, bind as a real attribute.
4. Python: java.scale(3.0, 2.0)  ← ready to call
```

## What can go wrong

See `KNOWN_FOOTGUNS.md` for the full list. The IPC-specific footguns:

- **#2: `connect_with_retry` returning a hardcoded fd.** The original
  code returned `390` instead of the actual socket fd. Every IPC call
  then wrote to whatever fd happened to be #390 — often a stale stdin
  dup. Fixed by `s.as_raw_fd()` + `mem::forget(s)`.

- **#3: Leaking `entries` but not `name_storage`.** The registration
  code built `Vec<CString>` of function names and `Vec<GluExportEntry>`
  whose `name` fields pointed INTO those CStrings. Only `entries` was
  leaked — `name_storage` was dropped, dangling the pointers. Fixed by
  leaking both.

- **#6: Wire-protocol byte order.** Java's `dispatchCall` for `scale`
  read tag0 then tag1 THEN read the values, but the wire format is
  interleaved (tag0, value0, tag1, value1). Fixed by reading
  tag-then-value per arg.

- **#7: Missing response length prefix.** Java wrote the response body
  without the 4-byte length prefix Rust expected. Rust read the first 4
  bytes of the body as the length, getting garbage. Fixed by writing
  `writeU32LE(out, fullMsg.length)` before the body.
