# 07 — Performance

This document covers GluCore v2's performance characteristics, the
optimizations applied, and what the remaining overhead is.

## The benchmark

`scripts/bench_final.py` runs the same workload (1000 frames, 50
entities, 5 runs each) through two paths:

- **raw ctypes**: direct function pointer call, in-place float[]
  mutation, no copy. This is the fastest possible FFI path.
- **GluCore**: registry lookup + link check + type validation + Buffer
  copy in/out. This is what every GluCore call goes through.

The workload is a bouncing-ball physics simulation. The raw version
mutates two `float[]` arrays in place. The GluCore version packs the
state into a `bytes` buffer, passes it as a `Buffer` arg, the C++ side
copies it into a `std::vector<uint8_t>`, mutates it, returns it, the
Python side copies it back into a `bytes` object.

## Results (median of 5 runs)

```
raw ctypes:
  per-call (µs):  min=0.587  median=0.633  max=0.677
  calls/sec:      min=1,478,100  median=1,578,659  max=1,702,417

GluCore:
  per-call (µs):  min=2.960  median=3.002  max=3.020
  calls/sec:      min=331,083  median=333,095  max=337,869

--- Comparison ---
  median per-call: raw=0.633µs  glucore=3.002µs  ratio=4.7x slower
  median calls/sec: raw=1,578,659  glucore=333,095

Original Phase 2 report: 37x slower (per-call 0.0014ms vs 0.0516ms).
This run:                  4.7x slower (per-call 0.0006ms vs 0.0030ms).
```

## What was optimized

The 37x → 4.7x improvement came from four categories of optimization.

### 1. Eliminate per-call Python overhead

**Before:** `GluProxy.__getattr__` rebuilt the caller closure on every
call. Cost: ~5 µs/call in closure construction + dict lookup + AttributeError eval.

**After:** Caller closures are built ONCE in `__init__` and bound as
real attributes via `object.__setattr__`. The hot path is
`proxy.foo(args)` → direct attribute lookup → caller(args), skipping
`__getattr__` entirely.

### 2. Inline arg packing via code generation

**Before:** Each arg was packed by calling a per-arg packer closure.
Cost: ~0.5 µs/arg in function-call overhead.

**After:** The caller is generated via `exec()` with each arg's type
check and pack code INLINED as straight-line code. Zero per-call
function-call overhead for arg packing.

```python
# Generated caller for scale(Float, Float) -> Float:
def caller(*args):
    if len(args) != 2: raise TypeError(...)
    a0, a1 = args
    arr = reusable_arr
    if type(a0) is not float: raise TypeError(...)
    arr[0].float = a0
    if type(a1) is not float: raise TypeError(...)
    arr[1].float = a1
    result = glucore_call(module_b, fn_b, arr, 2)
    if result.status != 0: raise RuntimeError(...)
    return unpack_return(result.value, return_tag, free_buf)
```

### 3. Zero-copy Buffer/String args

**Before:** `pack_arg` used `(c_ubyte * len).from_buffer_copy(b)` which
allocated a new ctypes array and memcpy'd the entire buffer per call.

**After:** `pack_arg` uses `ctypes.c_char_p(b)` which is zero-copy —
ctypes stores a reference to the `bytes` object and exposes its
internal buffer pointer directly. The `GluSlice.ptr` field type was
changed from `POINTER(c_ubyte)` to `c_void_p` to accept the integer
address directly.

### 4. Reusable ctypes array

**Before:** Each call allocated a new `(GluValue * argc)(...)` ctypes
array.

**After:** Each caller pre-allocates ONE `reusable_arr = ArrT()` in
`_make_caller` and reuses it across calls. Single-threaded demo: the
caller is not re-entered, so this is safe.

### 5. Rust: zero-allocation dispatch

**Before:** `dispatch` allocated three `String`s per call via
`to_string_lossy().into_owned()` (module name, function name, caller).

**After:** Uses `CStr::to_bytes()` (`&[u8]` view, zero allocation, zero
UTF-8 validation) and compares byte slices directly. The `&str`
parameters to `dispatch` are constructed via
`str::from_utf8_unchecked` over the byte views — safe because Python's
`str.encode()` is guaranteed UTF-8.

## What the remaining 4.7x is

The 4.7x gap that remains is dominated by:

1. **Buffer copy-in/copy-out (Constraint 4).** The GluCore type system
   commits to copy semantics for Buffer/String. The raw ctypes version
   passes a pointer and mutates in place — zero copy. This is the
   biggest single factor for the benchmark workload (which passes a
   400-byte buffer per call).

2. **Pre-call type validation (Task 4).** GluCore validates arg count
   and per-arg Python type BEFORE the ctypes boundary. The raw version
   does no validation.

3. **Link check (Task 5).** GluCore checks the caller→callee pair
   against the link table on every call. The raw version has no
   permission check.

4. **ctypes overhead.** Even with all the optimizations, ctypes has
   inherent per-call overhead (argument marshalling, GIL release, etc.)
   that raw ctypes avoids by being a single function pointer call.

## Closing the gap further

To go below 4.7x, you'd need one of:

- **Add a "raw pointer" GluType.** Let modules pass raw pointers (with
  explicit lifetime management) instead of copying buffers. This breaks
  the safety guarantees but would close most of the gap for buffer-
  heavy workloads. Out of scope for v2.

- **Generate C extension code instead of using ctypes.** PyO3 / nanobind
  generate C code that calls the Rust core directly, avoiding ctypes'
  per-call marshalling. This would close the ctypes overhead gap. Out
  of scope for v2.

- **Lazy type validation.** Skip the Python-side type check and let the
  Rust side reject bad args. This would save ~0.5 µs/call but breaks
  the Task 4 guarantee that bad calls never reach the FFI boundary.

## Profiling

`scripts/profile_bench.py` runs the GluCore benchmark under cProfile
and prints the cumulative-time top 30. Use it to find new hotspots:

```bash
cd scripts && python3 profile_bench.py
```

Sample output (after optimization):
```
         9001 function calls in 0.007 seconds
   ncalls  tottime  percall  cumtime  percall filename:lineno(function)
     1000    0.002    0.000    0.007    0.000 glucore.py:456(caller_3)
     1000    0.001    0.000    0.002    0.000 glucore.py:597(pack)
     2000    0.001    0.000    0.001    0.000 ctypes/__init__.py:520(cast)
     1000    0.001    0.000    0.001    0.000 glucore.py:287(unpack_return)
     ...
```

The remaining hotspots are mostly inside ctypes itself (the `cast` and
`string_at` calls), which can't be optimized further without dropping
ctypes for a C extension.

## Memory leak check

`scripts/task6a_dod.py` runs 120,000 iterations of `greet()` + 120,000
of `echo_bytes()` (both return String/Buffer allocations that must be
freed via `glucore_free_buffer`) and samples RSS periodically. The test
passes if RSS stays within 3x of the initial sample.

A `Box::leak` regression (the Phase 1 bug that motivated
`glucore_free_buffer` in the first place) would show RSS growing by
hundreds or thousands of times. The current code shows ratio = 1.000
(zero growth).
