# KNOWN_FOOTGUNS — Permanent Record of Mistakes That Have Already Happened

This file documents footguns that have actually bitten the GluCore project at
least once. Each entry describes the failure mode, how it manifested, and the
permanent rule that prevents it from happening again. New contributors SHOULD
read this file before adding any new module or language binding.

---

## Footgun #1: Rust rlib duplication of `glucore_core` statics

**Has happened:** twice (Phase 1 with `REGISTRY`/`LINKS`, Phase 2 with
`CALLER_STACK`).

**Failure mode:** A Rust crate (e.g. `physics`) that needs to call into
`glucore_core`'s shared state at runtime (`glucore_call`, the caller stack,
the link table) pulls those symbols in through the rlib. The rlib gives the
crate its own PRIVATE copy of every `static` in `glucore_core`. Those
private copies start empty and stay empty — they are NOT the same memory as
the statics in the `libglucore_core.so` Python loaded at startup. The
symptom is silent divergence: the module thinks it made a call, but the
real registry / link table / caller-identity is unchanged. Link-enforcement
checks pass against an empty link table (everything denied) or fail against
an empty registry (nothing found), depending on which path is taken. There
is no compile-time warning.

**Rule:** Any Rust crate that needs to call into `glucore_core`'s shared
state at runtime MUST resolve those symbols dynamically (via `libloading`
or `dlopen`/`dlsym` against the already-loaded `libglucore_core.{so,dylib}`)
and MUST NOT pull them in through the rlib. The `physics` crate's
`coreffi::abi()` function in `physics/src/lib.rs` is the canonical example
to copy: it builds a candidate list of paths to `libglucore_core.{so,dylib}`,
calls `dlopen(...)` with `RTLD_NOLOAD | RTLD_GLOBAL` first (to find the
already-loaded image) and falls back to a fresh `dlopen` (dyld dedups by
path, so this returns the same image — no second copy, no second REGISTRY).

**This does NOT apply to C++ or other non-Rust callers.** C++ links against
the `.so` the normal way (`target_link_libraries` in CMake), and that gives
it the one real shared instance. Task 9's state-sharing check
(`cpp_engine_shared_state_checksum` vs `glucore_shared_state_checksum` in
`scripts/task9_dod.py`) verifies this distinction directly: if C++ had its
own private copy, the checksums would diverge.

**Verification template for any new Rust caller module:**

1. Add a function to the new module that returns a checksum of `LINKS` /
   `CALLER_IDENTITY` from its own view of the core (via whatever path it
   uses — rlib or dlopen).
2. Call that function from Python in the same run where Python has already
   set up the link table and caller identity.
3. Compare the two checksums. If they match, the module is hitting the real
   shared instance. If they diverge, the module has its own private copy —
   switch it to the dlopen path before doing anything else.

The `glucore_shared_state_checksum` and `glucore_link_table_size` exports
in `glucore_core/src/lib.rs` exist for exactly this purpose. Use them.

---

## Footgun #2: `connect_with_retry` returning a hardcoded fd

**Has happened:** once (Phase 2 IPC).

**Failure mode:** The original `connect_with_retry` had a TODO comment and
returned a hardcoded fd `390` instead of the actual socket fd. Every IPC
call then wrote to / read from whatever file descriptor happened to be
#390 in the calling process — often a stale stdin/stdout dup. IPC "worked"
in the sense that the test didn't crash, but no actual IPC happened. The
Phase 2 IPC tests must not have actually exercised this path; they
probably passed because the test only checked that registration succeeded
(which doesn't use the fd) and never made a real CALL.

**Rule:** Any function that returns a resource handle (fd, pointer, id)
MUST actually return the real handle. TODOs in code that returns handles
are forbidden — write the correct code or don't write the function. A test
that exercises a code path with a TODO return value is not a passing test.

**Verification template:** For any IPC-like code path, write a test that
sends a known payload and reads back the expected response. If the test
passes by accident (because the wrong handle happened to be valid), the
payload/response check will catch it.

The current `connect_with_retry` extracts the fd via `s.as_raw_fd()` and
`std::mem::forget(s)` to prevent `Drop` from closing it.

---

## Footgun #3: Leaking `entries` but not `name_storage`

**Has happened:** once (Phase 2 IPC registration).

**Failure mode:** `glucore_register_process_module` built a `Vec<CString>`
of function names and a `Vec<GluExportEntry>` whose `name` fields pointed
INTO those CStrings. The comment said "leak the entries + name storage"
but only `entries` was actually leaked. When the function returned,
`name_storage` was dropped, the CStrings' buffers were freed, and the
`entries[i].name` pointers became dangling. Subsequent calls to
`glucore_get_module_export_name` returned garbage bytes from freed memory.

**Rule:** When you build a struct that holds pointers into a backing
allocation, BOTH the struct AND the backing allocation must live for the
same lifetime. If you leak the struct, leak the backing allocation too.
Don't rely on comments — verify by reading the actual `leak()` calls.

**Verification template:** For any code that returns pointers to
registration data, write a test that reads those pointers AFTER the
function returns. If the test reads garbage (or crashes), the backing
allocation was freed prematurely. The `glucore_get_module_export_name`
function exists in part to make this test easy to write.

The current code calls `name_storage.leak()` alongside `entries.leak()`.

---

## Footgun #4: Wrong `RTLD_NOLOAD` value on Linux

**Has happened:** once (Phase 2 physics module).

**Failure mode:** The physics module's `coreffi::abi()` had
`const RTLD_NOLOAD: i32 = 0x8;` with a comment claiming "macOS/Linux."
On Linux, `0x8` is `RTLD_DEEPBIND`, NOT `RTLD_NOLOAD` (which is `0x4`).
The first `dlopen` call used `RTLD_NOLOAD | RTLD_GLOBAL = 0x108` — which
on Linux is `RTLD_DEEPBIND | RTLD_GLOBAL`, a valid combination that does
NOT mean "only return handle if already loaded." The fallback fresh
`dlopen` used `RTLD_GLOBAL = 0x100` alone — which on Linux glibc is an
INVALID mode (glibc requires `RTLD_LAZY` or `RTLD_NOW` to be OR'd in),
returning `NULL` with `dlerror()` = "invalid mode for dlopen(): Invalid
argument." The net effect: the physics module could never resolve the
core's ABI on Linux, and `physics->cpp_engine` always failed with a
panic at `physics/src/lib.rs:149`.

**Rule:** Platform-specific constants must be `cfg(target_os = ...)`
-gated, not hardcoded with a comment claiming cross-platform behavior.
The constants in `<bits/dlfcn.h>` (Linux) and `<dlfcn.h>` (macOS) are
NOT the same — verify against the actual header, not against memory.

**Verification template:** For any dlopen-based code, test on the actual
target platform. A test that "works" on macOS but fails on Linux (or
vice versa) is a sign of platform-constant drift.

The current code uses `#[cfg(target_os = "linux")] const RTLD_NOLOAD: i32 = 0x4;`
and includes `RTLD_LAZY` in every dlopen call (required by glibc).

---

## Footgun #5: Python `glucore.py` hardcoded `.dylib` (macOS only)

**Has happened:** once (Phase 2 portability).

**Failure mode:** `load_core()` and `load_module()` both constructed
paths ending in `.dylib` (macOS-only). On Linux, the files end in `.so`,
so both functions raised `FileNotFoundError` and no test could run. The
code had clearly only ever been run on macOS.

**Rule:** Anywhere a shared-library filename is constructed, use either
`std::env::consts::{DLL_PREFIX, DLL_SUFFIX}` (Rust) or the centralized
`_platform_lib_filename()` helper (Python, in `glucore.py`). Do not
hardcode `.dylib`/`.so`/`.dll` in three slightly different inline guesses
across three places.

**Verification template:** Run the test suite on at least one non-macOS
platform before claiming the project works. "Built and running" without
naming the platform is not an acceptable status — see the reporting rules
in `AGENT_HANDOFF_PHASE2_ADDENDUM.md` (Task 11).

---

## Footgun #6: Wire-protocol byte order assumptions (Java side)

**Has happened:** once (Task 8 IPC, found during Part 0c work).

**Failure mode:** The Java renderer's `dispatchCall` for `scale(v, f)`
read `tag0`, then `tag1`, THEN read the two 8-byte float values. But the
Rust wire format is interleaved: `tag0, value0, tag1, value1` — tag
immediately followed by its value. Reading tag0 then tag1 read the FIRST
BYTE of value0 as tag1, which was always 0x00 (the low byte of a typical
float). The "scale expects (Float, Float)" check always failed, even
though the actual tags WERE both Float. The wire-format spec in
`glucore_core/src/lib.rs` is correct; the Java implementation just didn't
match it.

**Rule:** When implementing a wire protocol in a new language, write a
byte-level round-trip test BEFORE writing the dispatch logic. The test
should send a known payload, capture the raw bytes received, and verify
they match the spec. Catching a byte-order / interleaving bug at the test
layer is much faster than debugging it through the dispatch path.

**Verification template:** For each wire-protocol function, encode a
known-fixed input and dump the resulting bytes. Compare against the spec.
This is also useful for catching endianness bugs (the protocol is
little-endian; Java's `ByteBuffer` defaults to big-endian).

The current Java code reads `tag, value, tag, value` (interleaved) per
the spec.

---

## Footgun #8: C++ template `static s_fn` shared across same-signature functions

**Has happened:** once (Task 9, found via Requirement-1 state-sharing check).

**Failure mode:** The C++ `GLUCORE_EXPORT` macro's `Registrar` template was
parameterized ONLY by the function signature (`Ret(*)(Args...)`), not by
the function itself. The `static Ret(*s_fn)(Args...) = nullptr;` inside
`make_wrapper` was therefore SHARED across all functions with the same
signature. Each call to `make_wrapper` OVERWROTE `s_fn`, so all
same-signature exports ended up bound to the LAST registered function's
wrapper. The bug manifested as `cpp_engine_link_table_size`,
`cpp_engine_shared_state_checksum`, `cpp_engine_links_address`, and
`attempt_physics_call_from_cpp` all returning `4` — the GluStatus::LinkDenied
code from `attempt_physics_call_from_cpp`, the last-registered `int64_t()`
function.

**Rule:** A C++ template that stores per-function state (like a function
pointer for a captureless lambda) MUST be parameterized by something
unique per function, not just by the function signature. The standard
pattern is to add a unique TAG type (an empty struct generated by the
macro) as a template parameter. Each function then gets its own template
instantiation and its own static.

**Verification template:** For any code that generates wrappers for
multiple functions with the same signature, write a test that calls EACH
function and verifies it returns the RIGHT value (not just any value).
A function-pointer-collision bug like this one passes the "does it return
a value" test but fails the "does it return the RIGHT value" test.

The current `GLUCORE_EXPORT` macro generates `struct GlucoreTag_##fn {};`
per function and passes it as `register_export<GlucoreTag_##fn>(#fn, &fn)`,
forcing a distinct template instantiation per function.

---

## Footgun #7: Response framing — missing length prefix

**Has happened:** once (Task 8 IPC, found during Part 0c work).

**Failure mode:** The Java renderer's response path built a `fullMsg`
byte array containing `MSG_RESULT + payload_len + payload`, then wrote
`fullMsg` to the socket WITHOUT a length prefix. The Rust side's
`ipc_roundtrip` reads a 4-byte length prefix first, then reads that many
bytes. With no prefix from Java, Rust read the first 4 bytes of
`fullMsg` (the `0x01` MSG_RESULT byte plus 3 bytes of `payload_len`) as
the length, getting a garbage value (typically very large), then tried
to read that many bytes — which either hung forever or failed when Java
had nothing more to send.

**Rule:** Wire-protocol framing must be SYMMETRIC. If the request has a
length prefix, the response must have one too. Document this in the
protocol spec (the Rust comments do; the Java code didn't follow it).
A round-trip test that sends a request and reads back the response
catches this immediately.

**Verification template:** For any IPC round-trip test, verify that the
response length matches what was actually sent. A timeout or
short-read on the response side is a strong sign of a framing mismatch.

The current Java code calls `writeU32LE(out, fullMsg.length)` before
writing `fullMsg`, matching the Rust reader's expectation.
