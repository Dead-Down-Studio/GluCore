# 08 — Known Footguns

This document mirrors `KNOWN_FOOTGUNS.md` and adds verification templates
for each. The footguns file is the permanent record of mistakes that
have actually bitten this project. New contributors SHOULD read it
before adding any new module or language binding.

## Footgun #1: Rust rlib duplication of `glucore_core` statics

**Has happened:** twice (Phase 1 `REGISTRY`/`LINKS`, Phase 2 `CALLER_STACK`).

**Failure mode:** A Rust crate (e.g. `physics`) that needs to call into
`glucore_core`'s shared state at runtime pulls those symbols in through
the rlib. The rlib gives the crate its own PRIVATE copy of every
`static` — separate from the `libglucore_core.so` Python loaded. The
symptom is silent divergence: the module thinks it made a call, but the
real registry/link-table/caller-identity is unchanged.

**Rule:** Any Rust crate that calls into `glucore_core`'s shared state
at runtime MUST resolve symbols dynamically (via `libloading` /
`dlopen`/`dlsym`) and MUST NOT pull them in through the rlib. See
`physics/src/lib.rs::coreffi::abi()` for the canonical pattern.

**Does NOT apply to C++** — C++ links against the `.so` the normal way
and gets the one real instance. Verified directly by Task 9's
state-sharing check (same `LINKS` address from both sides).

**Verification template:**
1. Add a function to the new Rust module that returns a checksum of
   `LINKS`/`CALLER_IDENTITY` from its own view of the core.
2. Call it from Python in the same run where Python has set up the link
   table.
3. Compare the checksums. If they match, the module shares the real
   instance. If they diverge, switch to the dlopen path.

The `glucore_shared_state_checksum` and `glucore_links_address` exports
exist for this. Use them.

## Footgun #2: `connect_with_retry` returning a hardcoded fd

**Has happened:** once (Phase 2 IPC).

**Failure mode:** The original code had `return 390; // TODO` instead
of the actual socket fd. Every IPC call wrote to whatever fd was #390
in the calling process — often a stale stdin/stdout dup. IPC "worked"
in tests because they didn't exercise the actual round-trip path.

**Rule:** Any function returning a resource handle MUST return the real
handle. TODOs in handle-returning code are forbidden.

**Verification template:** Send a known payload, read back the expected
response. A test that passes by accident (because the wrong handle
happened to be valid) will fail this check.

## Footgun #3: Leaking `entries` but not `name_storage`

**Has happened:** once (Phase 2 IPC registration).

**Failure mode:** `glucore_register_process_module` built a
`Vec<CString>` of function names and `Vec<GluExportEntry>` whose `name`
fields pointed INTO those CStrings. Only `entries` was leaked —
`name_storage` was dropped, dangling the pointers. Subsequent calls to
`glucore_get_module_export_name` returned garbage from freed memory.

**Rule:** When a struct holds pointers into a backing allocation, BOTH
must live for the same lifetime. If you leak the struct, leak the
backing allocation too.

**Verification template:** Read the registration pointers AFTER the
function returns. If they read garbage or crash, the backing allocation
was freed prematurely.

## Footgun #4: Wrong `RTLD_NOLOAD` value on Linux

**Has happened:** once (Phase 2 physics module).

**Failure mode:** The physics module had `const RTLD_NOLOAD: i32 = 0x8;`
with a comment claiming "macOS/Linux." On Linux, `0x8` is
`RTLD_DEEPBIND`, NOT `RTLD_NOLOAD` (which is `0x4`). The fallback fresh
`dlopen` used `RTLD_GLOBAL = 0x100` alone, which on Linux glibc is
INVALID (requires `RTLD_LAZY` or `RTLD_NOW`), returning NULL.

**Rule:** Platform-specific constants must be `cfg(target_os = ...)`
-gated, not hardcoded with a comment claiming cross-platform behavior.

**Verification template:** Test on the actual target platform. A test
that "works" on macOS but fails on Linux is a sign of platform-constant
drift.

## Footgun #5: Python `glucore.py` hardcoded `.dylib` (macOS only)

**Has happened:** once (Phase 2 portability).

**Failure mode:** `load_core()` and `load_module()` both constructed
paths ending in `.dylib`. On Linux, the files end in `.so`, so both
raised `FileNotFoundError`. The code had only ever been run on macOS.

**Rule:** Anywhere a shared-library filename is constructed, use
`std::env::consts::{DLL_PREFIX, DLL_SUFFIX}` (Rust) or the centralized
`_platform_lib_filename()` helper (Python).

**Verification template:** Run the test suite on at least one non-macOS
platform before claiming the project works.

## Footgun #6: Wire-protocol byte order assumptions (Java side)

**Has happened:** once (Task 8 IPC, found during Part 0c).

**Failure mode:** Java's `dispatchCall` for `scale(v, f)` read `tag0`
then `tag1` THEN read the two 8-byte float values. But the Rust wire
format is interleaved: `tag0, value0, tag1, value1`. Reading tag0 then
tag1 read the FIRST BYTE of value0 as tag1, which was always 0x00.

**Rule:** When implementing a wire protocol in a new language, write a
byte-level round-trip test BEFORE writing the dispatch logic.

**Verification template:** For each wire-protocol function, encode a
known-fixed input, dump the raw bytes received, compare against the
spec.

## Footgun #7: Response framing — missing length prefix

**Has happened:** once (Task 8 IPC, found during Part 0c).

**Failure mode:** Java built a `fullMsg` byte array containing
`MSG_RESULT + payload_len + payload`, then wrote `fullMsg` WITHOUT a
length prefix. Rust read the first 4 bytes of `fullMsg` as the length,
getting a garbage value (typically very large), then tried to read that
many bytes — which either hung or failed.

**Rule:** Wire-protocol framing must be SYMMETRIC. If the request has a
length prefix, the response must have one too.

**Verification template:** For any IPC round-trip test, verify that the
response length matches what was actually sent. A timeout or short-read
on the response side is a strong sign of a framing mismatch.

## Footgun #8: C++ template `static s_fn` shared across same-signature functions

**Has happened:** once (Task 9, found via state-sharing check).

**Failure mode:** The C++ `GLUCORE_EXPORT` macro's `Registrar` template
was parameterized ONLY by the function signature, not by the function
itself. The `static Ret(*s_fn)(Args...) = nullptr;` inside
`make_wrapper` was therefore SHARED across all functions with the same
signature. Each `make_wrapper` call OVERWROTE `s_fn`, making every
same-signature export call the LAST registered function's wrapper. The
bug manifested as `cpp_engine_link_table_size`,
`cpp_engine_shared_state_checksum`, `cpp_engine_links_address`, and
`attempt_physics_call_from_cpp` all returning `4` (the
`GluStatus::LinkDenied` code from `attempt_physics_call_from_cpp`, the
last-registered `int64_t()` function).

**Rule:** A C++ template that stores per-function state MUST be
parameterized by something unique per function, not just by the
function signature. The standard pattern is a unique TAG type (an empty
struct generated by the macro) as a template parameter.

**Verification template:** For any code that generates wrappers for
multiple functions with the same signature, write a test that calls
EACH function and verifies it returns the RIGHT value (not just any
value).

## How to use this document

When you hit a bug in GluCore:
1. Check this document first — the bug may already be documented.
2. If it's new, add a new entry with the same structure (failure mode,
   rule, verification template).
3. Add a test that would have caught it.
4. The point of the footguns file is that each entry is a PERMANENT
   constraint on future work — not just a historical note.

When you add a new language adapter:
1. Read footgun #1 (rlib duplication) if your adapter is Rust.
2. Read footguns #6 and #7 (wire protocol) if your adapter is a PROCESS
   module.
3. Read footgun #8 (C++ template statics) if your adapter uses C++
   templates for wrapper generation.
4. Read footgun #5 (platform filenames) if your adapter loads shared
   libraries.
