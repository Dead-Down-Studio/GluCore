# 05 — Link Enforcement

GluCore enforces a declared topology: a module may only call another
module if the call is explicitly allowed in `glucore.toml`'s `[links]`
section. This document explains how enforcement works, what the
guarantees are, and how to verify them.

## The topology manifest

`glucore.toml` declares the topology. The `[links]` section is a list
of `caller = ["callee1", "callee2", ...]` pairs:

```toml
[links]
python = ["physics", "cpp_engine", "java_renderer"]
physics = ["cpp_engine"]
cpp_engine = ["java_renderer"]
java_renderer = ["physics"]
```

This means:
- Python may call physics, cpp_engine, or java_renderer.
- physics may call cpp_engine.
- cpp_engine may call java_renderer.
- java_renderer may call physics.
- Any other pairing is denied.

Python reads this file at startup (`glucore.load_core()`) and pushes
each (caller, callee) pair into the Rust core via `glucore_add_link`.
The core stores them in a `static mut LINKS: Vec<(CString, CString)>`.

## The caller identity

Every call has a CURRENT caller identity — a string set via
`glucore_set_caller_identity`. The identity is process-global (in v2;
would be thread-local in a multi-threaded version).

| Caller | When it's set |
|---|---|
| `"python"` | At startup, by `glucore.load_core()` |
| `"physics"` | Inside `physics::boost_with_cpp`, before the nested call to cpp_engine |
| `"cpp_engine"` | Inside `cpp_engine::accelerate_via_render`, before the nested call to java_renderer |
| `"java_renderer"` | Inside Rust's `ipc_roundtrip`, before dispatching a CALLBACK_CALL from Java |

The link check inside `dispatch()` reads the current caller identity
and compares it against the callee:

```rust
fn dispatch(module: &str, function: &str, args: *const GluValue, argc: usize) -> GluResult {
    let caller_bytes = current_caller_bytes();
    let caller_ok = match &caller_bytes {
        Some(c) => is_link_allowed_bytes(c.to_bytes(), module.as_bytes()),
        None => return GluResult::err(GluStatus::LinkDenied, "no caller identity set"),
    };
    if !caller_ok {
        return GluResult::err(GluStatus::LinkDenied,
            &format!("link denied: caller '{}' is not allowed to call module '{}'", ...));
    }
    // ... proceed to dispatch
}
```

## The save/restore discipline (CallerGuard)

When a module makes a nested call, it temporarily changes the caller
identity. The previous identity MUST be restored afterward — including
on the error path. v2 does this two ways:

1. **Rust modules** use `CallerGuard::enter(caller)` which saves the
   previous identity on construction and restores it on `Drop`. The
   `Drop` runs even if the wrapped code panics.

2. **C++ modules** do it manually (no RAII guard exposed across the C
   ABI):
   ```cpp
   glucore_set_caller_identity("cpp_engine");
   GluResult r = glucore_call("java_renderer", "scale", args, 2);
   glucore_set_caller_identity("python");  // restore unconditionally
   ```

3. **Java modules** (Task 10) — Rust does it for them. When Rust
   receives a CALLBACK_CALL, it saves the current caller, sets it to
   `"java_renderer"`, dispatches, restores. Java never sees the
   save/restore.

The Part 0b DoD test verifies the restore is clean by calling
`physics.calculate_force(10.0, 9.8)` (which returns 98.0) before and
after a denied nested call. If the restore was broken, the second call
would see a different caller identity and might be denied or return a
different value. The literal output shows 98.0 every time.

## What "denied" means

A denied call returns `GluResult { status: LinkDenied, message: "link
denied: caller 'X' is not allowed to call module 'Y'" }`. On the Python
side, this is raised as `RuntimeError`.

Critically, the denial happens BEFORE any work is done:
- The target module's wrapper is NOT called.
- For IPC modules, no bytes are sent over the socket (Constraint #8).
- The link table is not modified.

This is verified by:
- **Part 0a**: A REAL C++-initiated call to physics (denied) returns
  `GluStatus::LinkDenied` without corrupting state. Python can still
  call physics immediately afterward.
- **Part 0b**: The same args return the same value before and after a
  denied nested call.
- **Part 0c**: The Java side's byte-read counter stays at the same
  value after a denied-link attempt (0 bytes exchanged for the denied
  call).

## The discrimination guarantee

The link check is `(caller, callee)`-pair-specific, NOT "is the callee
reachable from anyone." This means:

- If `python → physics` is declared and `cpp_engine → physics` is NOT,
  then python can call physics but cpp_engine cannot — even though
  physics is "reachable" from someone.
- The Part 0a DoD verifies this directly: in the same run, python
  calls physics (succeeds), physics calls cpp_engine (succeeds), and
  cpp_engine attempts to call physics (denied). The denied call uses a
  REAL C++-initiated `glucore_call`, not a Python impersonation.

This is the property that makes the link enforcement real rather than
advisory. A router that only checked "is the callee registered" would
pass the impersonation test but fail the real-caller test.

## Self-reentrant IPC denial (Task 10)

A special case: `java_renderer → java_renderer` via CALLBACK_CALL. The
synchronous single-threaded IPC protocol would deadlock if Rust tried
to dispatch this (Rust would block sending to the socket while Java is
blocked waiting for the outer CALL's response).

Rust detects this case BEFORE dispatching and denies it cleanly:
```rust
if cb_request.module == "java_renderer" {
    let err_payload = encode_callback_result_error(
        "self-reentrant java_renderer -> java_renderer via CALLBACK_CALL denied ..."
    );
    // send err_payload as CALLBACK_RESULT, continue the loop
}
```

The Java side receives this as a normal error from its `glucoreCall`
helper — no hang, no dropped connection. The Task 10 DoD verifies this
with `attempt_self_call()`.

## Topology changes at runtime

v2 does NOT support adding/removing links at runtime. The `[links]`
table is read once at startup and pushed into the core. A future
"dynamic topology" feature would need:
- A `glucore_remove_link(caller, callee)` API.
- Thread-safety for the LINKS static (currently `static mut`, single-
  threaded assumption).
- A way to verify the new topology is acyclic.

## Verifying link enforcement

The DoD scripts that verify link enforcement:

| Script | What it verifies |
|---|---|
| `scripts/task5_dod.py` | Removing a link from glucore.toml denies the call; restoring it allows the call again. |
| `scripts/task7_multicaller_dod.py` | Multi-caller mesh: python→physics (ok), python→cpp_engine (ok), physics→cpp_engine (ok), cpp_engine→physics (DENIED via Python impersonation). |
| `scripts/part0a_dod.py` | REAL C++ caller denied: cpp_engine attempts physics via a C++-initiated glucore_call, gets LinkDenied. |
| `scripts/part0b_dod.py` | Caller-stack restore: same args return same value before/after a denied nested call. |
| `scripts/part0c_dod.py` | Byte-level evidence: 0 bytes exchanged for a denied-link IPC call. |
| `scripts/task9_dod.py` | Three-language mesh + state-sharing check + denied-first topology discipline. |
| `scripts/task10_dod.py` | Denied link inside CALLBACK_CALL produces clean error; self-reentrant denied cleanly. |
