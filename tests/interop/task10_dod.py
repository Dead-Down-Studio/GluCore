"""Task 10 Definition of Done — Java initiates a call outward.

Task 10 is the stretch goal from the Phase 2 Addendum: Java has only ever
been a CALLEE (answers CALL, sends RESULT). Task 10 makes Java a CALLER:
while handling an inbound CALL, Java sends a CALLBACK_CALL asking Rust to
perform a glucore_call on Java's behalf. Rust performs the call (full
permission check, normal dispatch), sends back a CALLBACK_RESULT, then
resumes waiting for the original outer RESULT.

This is genuinely harder than Task 9 (which added a new caller using the
existing request/response shape). Task 10 requires nested exchanges on
the SAME connection while the original outer call is still logically
"in flight."

DoD items (from the handoff):
  1. A Java method calls a Rust-resolvable target (e.g. physics) mid-
     handling of an inbound call from Rust, via a real CALLBACK_CALL, and
     receives a correct CALLBACK_RESULT — confirmed with actual values.
  2. The original outer call's real RESULT still arrives correctly
     afterward, confirmed by the outermost caller seeing the right final
     answer.
  3. A denied link inside a CALLBACK_CALL produces a clean error back to
     Java, confirmed by triggering it and observing the connection still
     usable for a subsequent normal call afterward.
  4. Self-reentrant java_renderer -> java_renderer via CALLBACK_CALL is
     denied cleanly (not hung).
  5. No regression anywhere in Phase 1, Phase 2, Part 0, or Task 9.

This script verifies items 1-4 in one run. Item 5 is verified by running
the other DoD scripts separately.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
import glucore
from glucore import GluStatus

SOCKET_PATH = f"/tmp/glucore_java_renderer_task10_{os.getpid()}.sock"
JAVA_CP = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "examples/java_renderer", "build",
)
ADAPTER_CP = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "adapters", "java", "target", "classes")
SPAWN_CMD = f"java -cp {ADAPTER_CP}:{JAVA_CP} Renderer {SOCKET_PATH}"


def main():
    core = glucore.load_core()
    physics = glucore.load_module(core, "physics")
    cpp = glucore.load_module(core, "cpp_engine")
    java = glucore.load_process_module(core, "java_renderer", SOCKET_PATH, SPAWN_CMD)
    core.set_caller_identity("python")

    print("=" * 78)
    print("TASK 10 — Java initiates a call outward (CALLBACK_CALL / CALLBACK_RESULT)")
    print("=" * 78)
    print()
    print("Wire protocol extension (Task 10):")
    print("  CALLBACK_CALL    (Java → Rust) = 0x02")
    print("  CALLBACK_RESULT  (Rust → Java) = 0x03")
    print()
    print("Sequencing for one nested round-trip:")
    print("  Rust  --CALL(java_renderer, fn, args)-->            Java")
    print("  Java starts executing fn(); fn() wants to call glucore_call('physics', ...)")
    print("  Java  --CALLBACK_CALL(physics, ...)-->              Rust")
    print("  Rust performs the actual glucore_call (full permission check)")
    print("  Rust  --CALLBACK_RESULT(...)-->                     Java")
    print("  Java's fn() resumes, finishes, sends:")
    print("  Java  --RESULT(...)-->                              Rust  (answer to ORIGINAL outer CALL)")
    print()

    results = []

    # --- DoD item 1: Java calls physics mid-handling, gets correct result ---
    # scale_via_physics(mass, accel) calls physics::calculate_force(mass, accel)
    # via CALLBACK_CALL and returns the result. calculate_force(m, a) = m*a.
    # So scale_via_physics(10.0, 9.8) should return 98.0.
    print("--- DoD #1: Java calls physics mid-handling of inbound CALL ---")
    try:
        r = java.scale_via_physics(10.0, 9.8)
        ok = (abs(r - 98.0) < 1e-9)
        results.append(("java_renderer -> physics (CALLBACK_CALL)", "SUCCEED", ok,
                        f"scale_via_physics(10.0, 9.8) = {r}  (expected 98.0)"))
        print(f"  [{'PASS' if ok else 'FAIL'}] scale_via_physics(10.0, 9.8) = {r}  (expected 98.0)")
    except Exception as e:
        results.append(("java_renderer -> physics (CALLBACK_CALL)", "SUCCEED", False, f"threw {e!r}"))
        print(f"  [FAIL] threw {e!r}")

    # --- DoD item 2: original outer call's RESULT still arrives correctly ---
    # After the nested callback, the outer CALL's RESULT must still come back.
    # We verify this by making ANOTHER call to java_renderer immediately after
    # the callback test — if the outer-RESULT path was broken by the callback,
    # this call would fail or hang.
    print()
    print("--- DoD #2: original outer call's RESULT still arrives correctly ---")
    try:
        r = java.scale(3.0, 2.0)  # plain call, no callback
        ok = (abs(r - 6.0) < 1e-9)
        results.append(("outer RESULT after callback", "SUCCEED", ok,
                        f"scale(3.0, 2.0) = {r}  (expected 6.0)"))
        print(f"  [{'PASS' if ok else 'FAIL'}] scale(3.0, 2.0) = {r}  (expected 6.0)")
    except Exception as e:
        results.append(("outer RESULT after callback", "SUCCEED", False, f"threw {e!r}"))
        print(f"  [FAIL] threw {e!r}")

    # --- DoD item 3: denied link inside CALLBACK_CALL produces clean error ---
    # attempt_undeclared_call() tries to call cpp_engine::accelerate via
    # CALLBACK_CALL. java_renderer -> cpp_engine is NOT in glucore.toml's
    # [links], so the Rust core must refuse with LinkDenied. The function
    # returns the status byte (non-zero = error). The connection must still
    # be usable afterward.
    print()
    print("--- DoD #3: denied link inside CALLBACK_CALL produces clean error ---")
    try:
        status = java.attempt_undeclared_call()
        # status = 0 means the CALLBACK_CALL succeeded (BUG — should be denied).
        # status != 0 means the CALLBACK_CALL was denied (expected).
        ok = (status != 0)
        results.append(("java_renderer -> cpp_engine (denied CALLBACK_CALL)", "DENY", ok,
                        f"attempt_undeclared_call() returned status={status} (non-zero = denied as required)"))
        print(f"  [{'PASS' if ok else 'FAIL'}] attempt_undeclared_call() returned status={status} (non-zero = denied as required)")
    except Exception as e:
        results.append(("java_renderer -> cpp_engine (denied CALLBACK_CALL)", "DENY", False, f"threw {e!r}"))
        print(f"  [FAIL] threw {e!r}")

    # Verify the connection is still usable after the denied callback.
    try:
        r = java.scale(7.0, 3.0)
        ok = (abs(r - 21.0) < 1e-9)
        results.append(("connection usable after denied callback", "SUCCEED", ok,
                        f"scale(7.0, 3.0) = {r}  (expected 21.0)"))
        print(f"  [{'PASS' if ok else 'FAIL'}] connection still usable: scale(7.0, 3.0) = {r}  (expected 21.0)")
    except Exception as e:
        results.append(("connection usable after denied callback", "SUCCEED", False, f"threw {e!r}"))
        print(f"  [FAIL] threw {e!r}")

    # --- DoD item 4: self-reentrant denied cleanly ---
    # attempt_self_call() tries to call java_renderer::scale via CALLBACK_CALL.
    # This is self-reentrant (java_renderer -> java_renderer) and must be
    # denied cleanly — NOT hung. The Rust core detects this and returns an
    # error CALLBACK_RESULT without dispatching.
    print()
    print("--- DoD #4: self-reentrant java_renderer -> java_renderer denied cleanly ---")
    try:
        status = java.attempt_self_call()
        ok = (status != 0)
        results.append(("java_renderer -> java_renderer (self-reentrant)", "DENY", ok,
                        f"attempt_self_call() returned status={status} (non-zero = denied cleanly, not hung)"))
        print(f"  [{'PASS' if ok else 'FAIL'}] attempt_self_call() returned status={status} (non-zero = denied cleanly, not hung)")
    except Exception as e:
        results.append(("java_renderer -> java_renderer (self-reentrant)", "DENY", False, f"threw {e!r}"))
        print(f"  [FAIL] threw {e!r}")

    # Verify the connection is still usable after the self-reentrant denial.
    try:
        r = java.scale(2.0, 4.0)
        ok = (abs(r - 8.0) < 1e-9)
        results.append(("connection usable after self-reentrant denial", "SUCCEED", ok,
                        f"scale(2.0, 4.0) = {r}  (expected 8.0)"))
        print(f"  [{'PASS' if ok else 'FAIL'}] connection still usable: scale(2.0, 4.0) = {r}  (expected 8.0)")
    except Exception as e:
        results.append(("connection usable after self-reentrant denial", "SUCCEED", False, f"threw {e!r}"))
        print(f"  [FAIL] threw {e!r}")

    # --- Final verdict ---
    print()
    print("=" * 78)
    all_ok = all(r[2] for r in results)
    if all_ok:
        print("TASK 10 COMPLETE — Java can initiate outward calls via CALLBACK_CALL.")
        print()
        print("  - DoD #1: Java calls physics mid-handling, gets correct result (98.0).")
        print("  - DoD #2: Outer CALL's RESULT still arrives correctly after callback.")
        print("  - DoD #3: Denied link inside CALLBACK_CALL produces clean error;")
        print("            connection stays usable for subsequent calls.")
        print("  - DoD #4: Self-reentrant java_renderer -> java_renderer denied cleanly")
        print("            (not hung); connection stays usable.")
        print()
        print("The wire protocol now supports nested exchanges on the same connection")
        print("while the original outer call is still logically in flight. This is the")
        print("foundation for any language module to be both a caller and a callee over IPC.")
    else:
        print("TASK 10 INCOMPLETE — some checks failed.")
        for label, expect, ok, detail in results:
            if not ok:
                print(f"  [FAIL] {label}: {detail}")
        sys.exit(1)


if __name__ == "__main__":
    main()
