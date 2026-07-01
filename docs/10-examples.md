# 10 — Examples and DoD Tests

This document walks through every example and DoD test in `scripts/`,
explaining what each verifies and how to run it.

## Smoke test

### `main.py`

```bash
python3 main.py
```

Output:
```
Physics.calculate_force(10.0, 9.8) = 98.0
Hello, Nigga!
```

Loads `libglucore_core.so`, dlopens `libphysics.so`, calls
`physics.calculate_force(10.0, 9.8)` (returns 98.0) and
`physics.greet("Nigga")` (returns "Hello, Nigga!"). Verifies the basic
Python → Rust path works end-to-end.

## Task DoD tests (Phase 1 + Phase 2)

### `scripts/task1_dod.py` — Rust panic safety

Verifies:
- `calculate_force(10.0, 9.8)` returns 98.0.
- `add_ints(7, 35)` returns 42.
- `boom()` panics inside Rust, but the panic is caught by
  `catch_unwind` in the wrapper and returned as
  `GluResult { Runtime, "panic in boom" }`. The process survives.

```bash
python3 scripts/task1_dod.py
```

### `scripts/task3_dod.py` — String/Buffer round-trips

Verifies:
- `greet("World")` returns "Hello, World!".
- `greet("Müller")` returns "Hello, Müller!" (UTF-8 round-trip).
- `echo_bytes(b'\x00\x01\x02\x03\xff\xfe\x00\x07')` returns the same
  bytes (including NULs and high bytes).
- `echo_bytes(b'')` returns `b''` (empty buffer).
- `shout("hello")` returns "HELLO" (borrowed `&str` arg).
- `first_byte([42, 0, 1])` returns 42 (borrowed `&[u8]` arg).

```bash
python3 scripts/task3_dod.py
```

### `scripts/task4_dod.py` — Pre-call type validation

Verifies that bad args raise `TypeError` BEFORE reaching the ctypes
boundary:
- `calculate_force("not a number", 9.8)` raises TypeError naming
  "Float" and "String".
- `calculate_force(10.0)` (wrong arg count) raises TypeError naming
  "2" and "1".
- `calculate_force(10, 9)` (ints where floats expected) raises
  TypeError.
- `greet(42)` raises TypeError.
- `echo_bytes("not bytes")` raises TypeError.
- `physics.nonexistent_fn(...)` raises AttributeError.

```bash
python3 scripts/task4_dod.py
```

### `scripts/task5_dod.py` — Link enforcement

Backs up `glucore.toml`, removes the `python → physics` link, reloads
the core, verifies `calculate_force` is denied with `LinkDenied`, then
restores the link and verifies it works again.

```bash
python3 scripts/task5_dod.py
```

### `scripts/task6a_dod.py` — No RSS leak

Runs 120,000 iterations of `greet(big_str)` + `echo_bytes(big_buf)` (both
return String/Buffer allocations that must be freed via
`glucore_free_buffer`) and samples RSS periodically. Passes if RSS
stays within 3x of the initial sample. A `Box::leak` regression would
show RSS growing by hundreds or thousands of times.

```bash
python3 scripts/task6a_dod.py
```

### `scripts/task7_multicaller_dod.py` — Multi-caller mesh

Verifies in ONE run:
- `python → physics` succeeds (link declared).
- `python → cpp_engine` succeeds (link declared).
- `physics → cpp_engine` succeeds (nested Rust→C++ call, link declared).
- `cpp_engine → physics` DENIED (link NOT declared — Python impersonates
  cpp_engine to test this).
- `python → physics` still succeeds after the denial (state not
  corrupted).

Note: this test uses Python impersonation for item 4. The Part 0a test
does the REAL C++-initiated version.

```bash
python3 scripts/task7_multicaller_dod.py
```

## Part 0 re-verification tests

### `scripts/part0a_dod.py` — REAL C++ caller denied

The original task7 test used Python impersonation. This test uses a
REAL C++-initiated call: `cpp_engine.attempt_physics_call_from_cpp()` is
a C++ function that calls `glucore_call("physics", ...)` directly from
C++ code. With `cpp_engine → []` in glucore.toml, the call must be
denied.

```bash
python3 scripts/part0a_dod.py
```

### `scripts/part0b_dod.py` — Caller-stack restore

The Phase 2 report had a discrepancy: `python → physics` returned 98.0
earlier and 19.6 later for the same args. This test re-runs the exact
sequence and reports literal output. The same args `(10.0, 9.8)` return
98.0 at every probe — the 19.6 was a transcription error, not a
Drop-restore bug.

```bash
python3 scripts/part0b_dod.py
```

### `scripts/part0c_dod.py` — Byte-level IPC evidence

Constraint #8: "the Rust core checks caller→callee links BEFORE sending
anything over the socket." This test verifies it at the byte level
using a static byte-counter on the Java side.

```bash
python3 scripts/part0c_dod.py
```

Output (key lines):
```
(1) python->java_renderer.scale(3.0, 2.0) = 6.0
    bytes exchanged for this call (scale): 64
(2) physics->java_renderer.scale(3.0, 2.0) [impersonated caller]
    bytes exchanged for this DENIED call: 0
    [PASS] zero bytes exchanged — Constraint #8 verified at byte level
```

## Task 9 — Three-language mesh

### `scripts/task9_dod.py`

Verifies in ONE run:
- **Requirement 1 (state-sharing):** `glucore_shared_state_checksum()`
  from Rust and `cpp_engine_shared_state_checksum()` from C++ report
  the SAME value (same link count, same `LINKS` address). C++ does NOT
  have Rust's rlib-duplication footgun.
- **Requirement 3 (denied-first):** An undeclared caller (`rogue_caller`)
  attempting `java_renderer.scale` is denied.
- **Three-language mesh:**
  - `python → physics` (Rust) succeeds: 98.0
  - `physics → cpp_engine` (Rust→C++) succeeds: 28.6
  - `cpp_engine → java_renderer` (C++→Java via IPC) succeeds: 15.0
  - `cpp_engine → physics` (REAL C++ caller) DENIED

```bash
python3 scripts/task9_dod.py
```

## Task 10 — Java IPC caller

### `scripts/task10_dod.py`

Verifies in ONE run that Java can be a CALLER, not just a callee:
- **DoD #1:** `java.scale_via_physics(10.0, 9.8)` calls
  `physics::calculate_force(10.0, 9.8)` via CALLBACK_CALL and returns
  98.0.
- **DoD #2:** After the callback, the outer CALL's RESULT still arrives
  correctly (verified by a subsequent `java.scale(3.0, 2.0)` = 6.0).
- **DoD #3:** `java.attempt_undeclared_call()` tries to call
  `cpp_engine::accelerate` via CALLBACK_CALL — denied (link not
  declared). The connection stays usable afterward.
- **DoD #4:** `java.attempt_self_call()` tries to call
  `java_renderer::scale` via CALLBACK_CALL — denied cleanly (self-
  reentrant, not hung). The connection stays usable afterward.

```bash
python3 scripts/task10_dod.py
```

## Benchmarks

### `scripts/bench_raw.py` — Raw ctypes baseline

The bouncing-ball workload through direct ctypes (no GluCore
machinery). This is the fastest possible FFI path.

```bash
python3 scripts/bench_raw.py
```

### `scripts/bench_glucore.py` — GluCore path

The same workload through GluCore's full call path (registry lookup +
link check + type validation + Buffer copy in/out).

```bash
python3 scripts/bench_glucore.py
```

### `scripts/bench_final.py` — Consolidated benchmark

Runs both benchmarks 5 times each and reports min/median/max. This is
the number cited in the report.

```bash
python3 scripts/bench_final.py
```

Output:
```
median per-call: raw=0.633µs  glucore=3.002µs  ratio=4.7x slower
median calls/sec: raw=1,578,659  glucore=333,095

Original Phase 2 report: 37x slower (per-call 0.0014ms vs 0.0516ms).
This run:                  4.7x slower (per-call 0.0006ms vs 0.0030ms).
```

### `scripts/profile_bench.py` — cProfile hotspot analysis

Runs the GluCore benchmark under cProfile and prints the cumulative-
time top 30. Use it to find new hotspots after changes.

```bash
python3 scripts/profile_bench.py
```

## Running everything

The fastest way to verify the project is healthy:

```bash
cd /path/to/glucore_v2_proj
./build.sh                    # builds everything + runs smoke test
cd scripts
for t in task1_dod task3_dod task4_dod task5_dod task6a_dod \
         task7_multicaller_dod part0a_dod part0b_dod part0c_dod \
         task9_dod task10_dod; do
    echo "=== $t ==="
    timeout 30 python3 $t.py 2>&1 | tail -3
done
echo "=== bench ==="
python3 bench_final.py
```

All tests should pass. If any fail, the failure message names the
specific check that didn't pass — see the corresponding doc section for
what that check verifies.
