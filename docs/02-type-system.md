# 02 — Type System

GluCore's type system is a minimal universal IR. Every language's native
types must round-trip through it. The design goal is "small enough that
every language can map to it, big enough to be useful."

## The six GluTypes

| Tag | Enum name | Rust type | C type | Python type | Java type |
|---|---|---|---|---|---|
| 0 | `GluTypeTag::Int` | `i64` | `int64_t` | `int`, `bool` | `long` |
| 1 | `GluTypeTag::Float` | `f64` | `double` | `float` | `double` |
| 2 | `GluTypeTag::String` | `String` / `&str` | `char*` + len | `str` | `String` |
| 3 | `GluTypeTag::Buffer` | `Vec<u8>` / `&[u8]` | `uint8_t*` + len | `bytes`, `bytearray` | `byte[]` |
| 4 | `GluTypeTag::Handle` | (future) | `uint64_t` | (future) | (future) |
| 5 | `GluTypeTag::Void` | `()` | (return only) | `None` | `void` |

`Handle` is reserved for cross-language object references (Phase 3 of the
new project skeleton). v2 doesn't implement it — every value is copied
across the boundary (Constraint 4: "copy, don't borrow, for now").

## The Rust definitions (the authoritative source)

```rust
// glucore_core/src/lib.rs

#[repr(C)]
#[derive(Clone, Copy)]
pub union GluValue {
    pub float: f64,
    pub int: i64,
    pub string: GluSlice,
    pub buffer: GluSlice,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GluSlice {
    pub ptr: *const u8,
    pub len: usize,
}

#[repr(C)]
pub struct GluResult {
    pub status: GluStatus,
    pub value: GluValue,
    pub message: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GluStatus {
    Ok = 0,
    Runtime = 1,
    InvalidArgs = 2,
    NotFound = 3,
    LinkDenied = 4,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GluSignatureFFI {
    pub param_types: *const GluTypeTag,
    pub param_count: usize,
    pub return_type: GluTypeTag,
}
```

## The Python mirror (ctypes)

```python
# glucore.py

class GluSlice(ctypes.Structure):
    _fields_ = [
        ("ptr", ctypes.c_void_p),   # was POINTER(c_ubyte) — see perf doc
        ("len", ctypes.c_size_t),
    ]

class GluValue(ctypes.Union):
    _fields_ = [
        ("float", ctypes.c_double),
        ("int", ctypes.c_longlong),
        ("string", GluSlice),
        ("buffer", GluSlice),
    ]

class GluResult(ctypes.Structure):
    _fields_ = [
        ("status", ctypes.c_int),
        ("value", GluValue),
        ("message", ctypes.c_char_p),
    ]
```

**Layout must match the Rust exactly.** The C++ adapter verifies this
with a static_assert in `glucore_export.hpp`. If you change one side,
change the others.

## The C++ mirror

```cpp
// cpp_engine/include/glucore_export.hpp

namespace glucore {
enum class GluTypeTag : int32_t { Int=0, Float=1, String=2, Buffer=3, Handle=4, Void=5 };
enum class GluStatus : int32_t { Ok=0, Runtime=1, InvalidArgs=2, NotFound=3, LinkDenied=4 };

struct GluSlice {
    const uint8_t* ptr;
    size_t         len;
};

union GluValue {
    double    float_v;
    int64_t   int_v;
    GluSlice  string;
    GluSlice  buffer;
};

struct GluResult {
    GluStatus status;       // @0
    GluValue  value;        // @8 (4 bytes padding)
    const char* message;    // @24
};
}
```

## Ownership rules (Constraint 4)

| Direction | Who owns the buffer | Lifetime |
|---|---|---|
| Argument (Python → Rust) | Python owns; Rust borrows for the call duration | Python's `bytes` object must outlive the FFI call |
| Argument (Rust → Rust, nested) | Same as above — the outer wrapper's borrow | Same |
| Return (Rust → Python) | Rust allocates; Python copies and frees | Rust-allocated memory is freed via `glucore_free_buffer(ptr, len)` |
| Return (Rust → Rust, nested) | Same — the inner packer allocates, the outer wrapper borrows | The outer wrapper's GluResult holds the allocation |

The "copy, don't borrow" rule means:
- A `String` argument is COPIED into a Rust `String` (UTF-8 validated) on
  the Rust side. The original Python buffer can be freed immediately
  after the FFI call returns.
- A `String` return is allocated by Rust (via `Box::into_raw`), handed
  to Python as a raw pointer+len, copied into a Python `str`, then freed
  by Python calling `glucore_free_buffer(ptr, len)`. The Rust allocation
  does NOT outlive the free call.

This is intentionally simple. A future "zero-copy Buffer" optimization
(Phase 4 of the new project skeleton) would add a `GluHandle` type that
lets Rust and Python share a buffer without copying — but that requires
GC pinning, which is Phase 2/3 work.

## Signature metadata

Every exported function carries a `GluSignatureFFI`:

```rust
pub struct GluSignatureFFI {
    pub param_types: *const GluTypeTag,
    pub param_count: usize,
    pub return_type: GluTypeTag,
}
```

The `#[glucore::export]` proc-macro generates this automatically from
the Rust function's signature. The C++ `GLUCORE_EXPORT` macro generates
it via template argument deduction on the function pointer. The Java
side declares it manually in the registration message.

Python queries the signature via `glucore_get_signature(module, fn)` at
`load_module` time and builds a local dict. The `GluProxy.__getattr__`
path (or the inlined caller, in v2's optimized version) uses the
signature to validate arg count and per-arg Python type BEFORE the
ctypes boundary — a type mismatch raises a Python `TypeError` naming the
expected signature, never reaching the FFI call.

## Type coercion

GluCore does NOT coerce types automatically. `int → float` is a hard
error on the Python side (passing `10` where `Float` is expected raises
`TypeError: expected Float, got Int`). This is intentional — silent
coercion across language boundaries hides bugs.

If you want coercion, do it explicitly in the caller:
```python
physics.calculate_force(float(mass), float(accel))
```

## Adding a new type

To add a new GluType (e.g. `GluComplex`):

1. Add the variant to `GluTypeTag` in `glucore_core/src/lib.rs`.
2. Add the corresponding field to the `GluValue` union.
3. Update `GluTypeTag::from_u8` to recognize the new tag byte.
4. Update the `#[glucore::export]` proc-macro's `match_type` to accept
   the new Rust type (e.g. `Complex64`).
5. Update the C++ `type_tag` template specializations in
   `glucore_export.hpp`.
6. Update the Python `PY_TYPE_TO_TAG` dict and `pack_arg`/`unpack_return`
   in `glucore.py`.
7. Update the Java wire-protocol encode/decode in `Renderer.java`.
8. Bump the wire-protocol version (currently implicit; would be explicit
   in a future FCTP packet header).

The type system is deliberately small. Adding a type touches every
adapter — that's the cost of a universal IR.
