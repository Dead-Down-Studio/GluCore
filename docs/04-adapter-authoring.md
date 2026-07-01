# 04 — Adapter Authoring Guide

This document explains how to write a new language adapter for GluCore.
It's the same methodology used by the four existing adapters (Python,
Rust, C++, Java), distilled into 5 steps plus a pitfalls section.

## The 5-step methodology

Every adapter does the same 5 things. The order matters — earlier steps
are prerequisites for later ones.

### Step 1: Define type mappings

Create a table mapping your language's native types to GluTypeTag values.
This is the contract that everything else depends on.

| GluTypeTag | Your language's type(s) | Notes |
|---|---|---|
| `Int` | (your signed 64-bit int type) | Accept bool too if your language treats bool as int |
| `Float` | (your 64-bit float type) | NOT int — int→float is a hard error |
| `String` | (your string type) | Must round-trip UTF-8 |
| `Buffer` | (your byte-array type) | Raw bytes, no encoding |
| `Handle` | (future — opaque u64) | Reserved for Phase 3 |
| `Void` | (your unit type) | Return-only |

For Python:
```python
PY_TYPE_TO_TAG = {
    bool: "Int",  # bool is a subclass of int — must come first
    int: "Int",
    float: "Float",
    str: "String",
    bytes: "Buffer",
    bytearray: "Buffer",
}
```

For Rust (in the proc-macro):
```rust
("f64", _) => Tag::Float,
("i64", _) => Tag::Int,
("String", _) => Tag::StringOwned,
("&str", _) => Tag::StringBorrowed,  // maps to GluTypeTag::String
("Vec<u8>", _) => Tag::BufferOwned,
("&[u8]", _) => Tag::BufferBorrowed,  // maps to GluTypeTag::Buffer
```

### Step 2: Write bindings (load libglucore, declare C function signatures)

Your language needs a way to call C functions in `libglucore_core.so`.
The minimum set of functions you MUST bind:

| Function | Why |
|---|---|
| `glucore_call(module, fn, args, argc) → GluResult` | Make a call |
| `glucore_register_module(GluModule)` | Register your module's exports |
| `glucore_get_signature(module, fn) → GluSignatureFFI` | Discover another module's signature |
| `glucore_set_caller_identity(name)` | Set who is making calls (do this at startup) |
| `glucore_add_link(caller, callee)` | Declare topology |
| `glucore_free_buffer(ptr, len)` | Free Rust-allocated String/Buffer returns |

Optional but useful:
| Function | Why |
|---|---|
| `glucore_get_module_export_count(module)` | Enumerate a PROCESS module's exports (can't dlopen it) |
| `glucore_get_module_export_name(module, idx)` | Same |
| `glucore_shared_state_checksum()` | State-sharing check (Task 9 Requirement 1) |
| `glucore_link_table_size()` | Same |

For Python (ctypes):
```python
class GluCore:
    def __init__(self, core_path: str):
        self._lib = ctypes.CDLL(core_path, mode=os.RTLD_GLOBAL)
        self._lib.glucore_call.argtypes = [
            ctypes.c_char_p, ctypes.c_char_p,
            ctypes.POINTER(GluValue), ctypes.c_size_t,
        ]
        self._lib.glucore_call.restype = GluResult
        # ... bind the rest
```

For Rust (in a module that needs to call out):
```rust
mod coreffi {
    extern "C" {
        fn glucore_call(module: *const c_char, fn_: *const c_char,
                        args: *const GluValue, argc: usize) -> GluResult;
        fn glucore_set_caller_identity(name: *const c_char);
    }
}
```

For C++ (linking directly):
```cpp
extern "C" {
    glucore::GluResult glucore_call(const char* module, const char* fn,
                                    const glucore::GluValue* args, size_t argc);
    void glucore_set_caller_identity(const char* name);
}
```

### Step 3: Implement type conversion (pack/unpack for each GluType)

For each GluType, you need:
- A **packer** that takes a native value and produces a `GluValue` (for
  args going INTO the core).
- An **unpacker** that takes a `GluValue` and produces a native value
  (for returns coming OUT of the core).

The packer is responsible for keeping the underlying buffer alive for
the duration of the FFI call. The unpacker is responsible for copying
bytes out of Rust-allocated memory and then calling
`glucore_free_buffer` to release the Rust allocation.

Python packer (optimized version from glucore.py):
```python
def _make_packer(i, expected_tag, ...):
    if expected_tag == "Float":
        def pack(v):
            if type(v) is not float: raise TypeError(...)
            gv = GluValue()
            gv.float = v
            return gv, None  # no keep-alive needed
        return pack
    if expected_tag == "String":
        def pack(v):
            if type(v) is not str: raise TypeError(...)
            b = v.encode('utf-8')
            cstr = ctypes.c_char_p(b)  # zero-copy: c_char_p holds a ref to b
            gv = GluValue()
            gv.string = GluSlice(ptr=ctypes.cast(cstr, ctypes.c_void_p).value, len=len(b))
            return gv, cstr  # keep cstr alive past the FFI call
        return pack
```

Rust packer (in the proc-macro):
```rust
Tag::Float => quote! { glucore_core::GluValue { float: ret } },
Tag::StringOwned => quote! {
    let boxed: Box<[u8]> = ret.into_bytes().into_boxed_slice();
    let len = boxed.len();
    let ptr = Box::into_raw(boxed) as *mut u8;
    glucore_core::GluValue { string: glucore_core::GluSlice { ptr, len } }
},
```

### Step 4: Implement module registration

Your adapter needs a way for users to mark functions as "exported to
GluCore." This typically takes the form of an attribute/macro that:

1. Generates a wrapper function with the C ABI signature
   `extern "C" fn(*const GluValue, usize) -> GluResult`.
2. The wrapper unpacks args, calls the user's function, packs the return,
   all inside `catch_unwind` / `try-catch` so no exception escapes the
   C boundary.
3. Registers the wrapper + its signature in a static table.
4. Exposes a `glucore_module_info()` entry point that returns the table
   as a `GluModule` struct.

Rust (proc-macro):
```rust
#[glucore::export]
fn calculate_force(mass: f64, accel: f64) -> f64 {
    mass * accel
}
// Expands to:
//   - the original fn
//   - a __wrapper_calculate_force extern "C" fn
//   - a static __SIG_CALCULATE_FORCE: GluSignature
//   - an inventory::submit! that registers (name, wrapper, signature)
```

C++ (template macro):
```cpp
double accelerate(double v, double dt) { return v + 9.8 * dt; }
GLUCORE_EXPORT(accelerate)
// Expands to a static initializer that calls register_export<GlucoreTag_accelerate>
// which builds a wrapper + signature and pushes them into the registry vector.
```

Java (manual in registration message):
```java
// No macro — declare exports in sendRegistration():
String[][] exports = {
    {"scale", "1", "1,1"},  // (Float, Float) -> Float
    {"identity", "1", "1"},
};
```

### Step 5: Implement GC pinning (for Handle types — Phase 2/3)

**v2 does NOT implement this** because v2 has no `GluHandle` type. This
section is forward-looking — when you add Handle support, you'll need it.

The problem: Python (or Java) creates an object, GluCore gives it a
Handle, Python's GC later collects the object, GluCore tries to use the
Handle → segfault.

The solution: each language adapter registers `pin` and `unpin` callbacks
with GluCore at startup. GluCore calls `pin(handle)` when a handle
crosses a language boundary (preventing GC) and `unpin(handle)` when the
handle is released.

For Python:
```python
# gc_pin.py
def pin(handle_id: int, obj: object) -> None:
    _PINNED[handle_id] = obj  # hold a reference so GC doesn't collect

def unpin(handle_id: int) -> None:
    _PINNED.pop(handle_id, None)
```

For Rust: not needed — Rust's ownership model handles this via lifetimes.

For Java:
```java
// GcPin.java
private static final Map<Long, Object> PINNED = new ConcurrentHashMap<>();
public static void pin(long handle, Object obj) {
    PINNED.put(handle, obj);  // create a global ref via JNI NewGlobalRef
}
public static void unpin(long handle) {
    PINNED.remove(handle);
}
```

## Common pitfalls by paradigm

### GC languages (Python, Java)

- **Object lifetime vs handle lifetime.** If a GC'd object is passed by
  reference (Handle), the GC must not collect it while GluCore holds the
  handle. Implement pinning in Step 5.
- **GIL / JIL deadlock.** If your IPC transport receives a call on a
  background thread and tries to run Python/Java code without acquiring
  the GIL/JIL, you'll get a deadlock. The adapter MUST call
  `PyGILState_Ensure()` (Python) or attach the thread (Java) before
  touching any language object.
- v2 sidesteps both issues by being single-threaded and using copy
  semantics (no Handles). A multi-threaded or Handle-based adapter must
  solve them.

### Ownership languages (Rust)

- **Lifetimes vs handles.** When Rust passes a value to GluCore via a
  Handle, Rust must ensure the value lives at least as long as the
  handle. The adapter enforces this via lifetime annotations on the
  `register_export` function.
- **rlib duplication.** This is the footgun that has bitten this project
  twice (Phase 1, Phase 2). See `KNOWN_FOOTGUNS.md` #1. The rule: any
  Rust crate that calls into `glucore_core`'s shared state at runtime
  MUST resolve symbols dynamically via `libloading`, NOT via the rlib.

### Exception languages (Python, C++, Java)

- **NEVER let an exception unwind into C.** A C++ exception that
  unwinds into the C router is undefined behavior. A Python exception
  that escapes the ctypes boundary aborts the process. A Java throwable
  that escapes the JNI boundary aborts the JVM.
- Every adapter MUST catch all exceptions at the boundary and convert
  them to a `GluResult { status: Runtime, message: "..." }`. The
  `#[glucore::export]` proc-macro does this with `catch_unwind`. The
  C++ `GLUCORE_EXPORT` macro does it with `try/catch(...)`. The Java
  dispatch loop does it with `try/catch(Exception)`.

### C ABI languages (C, C++, Rust)

- **Layout must match exactly.** The `GluValue` union, `GluSlice`
  struct, and `GluResult` struct must have IDENTICAL memory layout
  across Rust, C++, and Python's ctypes mirror. Any drift causes
  silent corruption. The C++ adapter has a static_assert for this.
- **No padding surprises.** `GluResult` is 32 bytes: status (4) +
  padding (4) + value (16) + message (8). If your language's ABI
  inserts different padding, you'll get garbage. Verify with a
  round-trip test.

## Testing checklist for new adapters

When you add a new language adapter, verify each of these in order:

1. **Type round-trip.** Each GluType can be packed by your adapter,
   passed through `glucore_call` to a trivial echo function in another
   module, and unpacked back to the original value.
2. **Panic/exception safety.** A function in your adapter that
   panics/throws returns a `GluResult { Runtime }` — the caller
   doesn't crash.
3. **Link enforcement.** Your adapter, when set as the caller, can
   call declared callees but is denied for undeclared ones.
4. **State-sharing check (if Rust).** Verify your adapter shares the
   ONE real instance of REGISTRY/LINKS, not a private rlib-duplicated
   copy. Use `glucore_shared_state_checksum` from both sides.
5. **Nested calls.** Your adapter can call another module which calls
   a third module, and the caller identity is correctly restored
   afterward.
6. **IPC callback (if PROCESS module).** If your adapter is a PROCESS
   module (separate process), verify it can send CALLBACK_CALL and
   receive CALLBACK_RESULT mid-handling of an inbound CALL.
7. **No RSS leak.** 100k+ calls don't grow memory. See `task6a_dod.py`
   for the template.

## Reference: complete type mapping tables

### Python → GluTypeTag

| Python type | GluTypeTag | Notes |
|---|---|---|
| `bool` | `Int` | `bool` is a subclass of `int` — must check `bool` first |
| `int` | `Int` | |
| `float` | `Float` | `int` is NOT auto-coerced to `Float` |
| `str` | `String` | UTF-8 encoded on the wire |
| `bytes` | `Buffer` | |
| `bytearray` | `Buffer` | Copied to `bytes` first (ctypes can't hold a ref to a mutable bytearray) |
| `None` | `Void` | Return-only |

### Rust → GluTypeTag

| Rust type | GluTypeTag | Notes |
|---|---|---|
| `i64` | `Int` | |
| `f64` | `Float` | |
| `String` | `String` | Owned |
| `&str` | `String` | Borrowed — packed the same way on the wire |
| `Vec<u8>` | `Buffer` | Owned |
| `&[u8]` | `Buffer` | Borrowed |
| `()` | `Void` | Return-only |

### C++ → GluTypeTag

| C++ type | GluTypeTag | Notes |
|---|---|---|
| `int64_t` | `Int` | |
| `double` | `Float` | |
| `std::string` | `String` | |
| `std::vector<uint8_t>` | `Buffer` | |
| `void` | `Void` | Return-only |

### Java → GluTypeTag

| Java type | GluTypeTag | Notes |
|---|---|---|
| `long` | `Int` | |
| `double` | `Float` | |
| `String` | `String` | |
| `byte[]` | `Buffer` | |
| `void` | `Void` | Return-only |
