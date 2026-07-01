# 09 — API Reference

Every public C ABI function exposed by `libglucore_core.so`. These are
the symbols adapters call into. The `#[no_mangle]` attribute on each
ensures the C symbol name matches the Rust function name.

## Core call / dispatch

### `glucore_call`

```c
GluResult glucore_call(
    const char* module,
    const char* function,
    const GluValue* args,
    size_t argc
);
```

Make a cross-module call. The caller identity is whatever is currently
set (via `glucore_set_caller_identity`). The link check is performed
inside; if denied, returns `GluResult { status: LinkDenied, message: "..." }`.

**Safety:** `module` and `function` must be valid NUL-terminated C
strings (or NULL, which returns `InvalidArgs`). `args` must point to an
array of at least `argc` `GluValue`s (or NULL if `argc == 0`).

### `glucore_set_caller_identity`

```c
void glucore_set_caller_identity(const char* name);
```

Set the current caller identity. MUST be called before any
`glucore_call` — a NULL identity causes `LinkDenied`. Pass NULL to
clear.

**Note:** This is process-global in v2. A multi-threaded version would
use thread-local. Modules making nested calls MUST save and restore the
previous identity (see `CallerGuard` in Rust, manual save/restore in
C++).

## Module registration

### `glucore_register_module`

```c
void glucore_register_module(GluModule module);
```

Register an ARTIFACT module (dlopen'd `.so`). The `GluModule` struct
contains the module name, a pointer to the export table, and the export
count. The export table's pointers must remain valid for the process
lifetime (the core does NOT copy them).

### `glucore_register_process_module`

```c
int32_t glucore_register_process_module(
    const char* module_name,
    const char* socket_path,
    const char* spawn_cmd
);
```

Register a PROCESS module (separate process). Spawns the process via
`sh -c spawn_cmd`, connects to the Unix domain socket at `socket_path`
(with bounded retry, 5s timeout, 50ms intervals), reads the registration
message, and builds GluExportEntry wrappers backed by IPC.

Returns 0 on success, negative on failure:
- `-1`: null argument
- `-2`: failed to spawn
- `-3`: timed out connecting
- `-4..-7`: registration read/parse failures

**Unix only.** `#[cfg(unix)]`-gated.

## Signature introspection

### `glucore_get_signature`

```c
GluSignatureFFI glucore_get_signature(
    const char* module,
    const char* function
);
```

Look up a function's signature (param types + return type). Returns
`GluSignatureFFI::empty()` (null param_types, 0 count, Void return) if
the module or function is not found.

### `glucore_get_module_export_count`

```c
size_t glucore_get_module_export_count(const char* module);
```

Return the number of exported functions in a module. Used by the Python
adapter to enumerate a PROCESS module's exports (which it can't dlopen).

### `glucore_get_module_export_name`

```c
const char* glucore_get_module_export_name(
    const char* module,
    size_t index
);
```

Return a pointer to the i-th export's name in a module, or NULL if the
module or index is out of range. The returned pointer is owned by the
core and lives for the process lifetime.

## Topology

### `glucore_add_link`

```c
void glucore_add_link(const char* caller, const char* callee);
```

Add a single (caller → callee) link to the topology graph. Called once
per declared link in `glucore.toml`'s `[links]` table at startup.

## Memory management

### `glucore_free_buffer`

```c
void glucore_free_buffer(void* ptr, size_t len);
```

Reclaim a Rust-allocated byte buffer previously handed to the caller as
a String/Buffer return value. `ptr`/`len` are exactly the `ptr`/`len`
carried in the `GluSlice` of a returned `GluValue`. Safe to call with
NULL (no-op). MUST NOT be called twice on the same pointer (double-free)
or with a length other than the one Rust handed out (UB).

## Debug / introspection (Task 9 additions)

### `glucore_shared_state_checksum`

```c
uint64_t glucore_shared_state_checksum(void);
```

Return a u64 summarizing the current shared state: link-table length in
the low 32 bits, current caller identity's byte-length in the high 32
bits. Two callers hitting the same instance see the same value;
callers hitting private (rlib-duplicated) copies diverge.

Used by the Task 9 state-sharing check to verify that C++ (which links
the core the normal way) shares the ONE real instance, not a private
copy. The corresponding C++ wrapper is
`cpp_engine_shared_state_checksum`.

### `glucore_link_table_size`

```c
size_t glucore_link_table_size(void);
```

Return the current link-table size. Stable after startup (the link
table doesn't change per call, unlike the caller identity). Useful for
the state-sharing check because it's a small discriminating number.

### `glucore_links_address`

```c
uint64_t glucore_links_address(void);
```

Return the address of the `LINKS` static as a u64. Definitive same-
instance test: if two callers see the same address, they share the one
real instance. If they differ, they're hitting different copies of
`libglucore_core.so` (likely due to `RTLD_LOCAL` on the Python side
preventing symbol reuse — fixed by loading with `RTLD_GLOBAL`).

## Data types

### `GluValue` (union, 16 bytes)

```c
union GluValue {
    double    float;    // GluTypeTag::Float
    int64_t   int;      // GluTypeTag::Int
    GluSlice  string;   // GluTypeTag::String
    GluSlice  buffer;   // GluTypeTag::Buffer
};
```

The active member is determined by the signature's type tag, NOT
embedded in the value. The same 16 bytes can hold any of the four
variants; the caller must know which one is active.

### `GluSlice` (struct, 16 bytes)

```c
struct GluSlice {
    const uint8_t* ptr;
    size_t         len;
};
```

Pointer + length pair for String/Buffer. The pointer references either:
- Argument direction (Python → Rust): the caller's buffer, valid for
  the duration of the call.
- Return direction (Rust → Python): Rust-allocated memory, must be freed
  via `glucore_free_buffer(ptr, len)` after the caller copies the bytes.

### `GluResult` (struct, 32 bytes)

```c
struct GluResult {
    GluStatus   status;     // @0 (4 bytes)
    // 4 bytes padding
    GluValue    value;      // @8 (16 bytes)
    const char* message;    // @24 (8 bytes)
};
```

`status == Ok` means `value` is valid. `status != Ok` means `message`
is a NUL-terminated error string (may be NULL). The message is owned by
Rust and lives for the process lifetime (leaked via `CString::into_raw`)
— the caller does NOT free it.

### `GluStatus` (enum, 4 bytes)

```c
enum GluStatus {
    Ok          = 0,
    Runtime     = 1,  // panic, exception, or IPC failure
    InvalidArgs = 2,  // null pointer, wrong arg count
    NotFound    = 3,  // module or function not in registry
    LinkDenied  = 4,  // caller→callee pair not in [links]
};
```

### `GluTypeTag` (enum, 4 bytes)

```c
enum GluTypeTag {
    Int    = 0,
    Float  = 1,
    String = 2,
    Buffer = 3,
    Handle = 4,  // reserved for Phase 3
    Void   = 5,  // return-only
};
```

### `GluSignatureFFI` (struct, 24 bytes)

```c
struct GluSignatureFFI {
    const GluTypeTag* param_types;
    size_t            param_count;
    GluTypeTag        return_type;
};
```

FFI-safe variant of `GluSignature` (which uses `&'static [GluTypeTag]` —
a Rust slice, not C-ABI-compatible). The `param_types` pointer is owned
by the module and lives for the process lifetime.

### `GluModule` (struct, 24 bytes)

```c
struct GluModule {
    const char*           name;     // module name (NUL-terminated)
    const GluExportEntry* entries;  // export table
    size_t                count;    // export count
};
```

### `GluExportEntry` (struct, 40 bytes)

```c
struct GluExportEntry {
    const char*     name;       // function name (NUL-terminated)
    GluWrapper      wrapper;    // function pointer
    GluSignatureFFI signature;  // param types + return type
};
```

### `GluWrapper` (function pointer type)

```c
typedef GluResult (*GluWrapper)(const GluValue* args, size_t argc);
```

The wrapper is what `dispatch` actually calls. For ARTIFACT modules,
it's the macro-generated function (e.g. `__wrapper_calculate_force`).
For PROCESS modules, it's always `ipc_dispatch_wrapper` (which reads
its context from the `IPC_CALL_CTX` thread-local).
