"""Task 7 multi-caller Definition of Done (the critical Phase 2 evidence).

Runs ALL of these in ONE process, in sequence, on a single core instance:
  (1) python -> physics        : SUCCEED  (link declared)
  (2) python -> cpp_engine     : SUCCEED  (link declared)
  (3) physics -> cpp_engine    : SUCCEED  (nested Rust->C++ call, link declared)
  (4) cpp_engine -> physics    : DENY     (link NOT declared — must be refused)

The point: prove the permission check DISCRIMINATES BETWEEN CALLERS, not just
"does any link exist." (3) and (4) target modules that are both reachable from
*someone*; what matters is who the caller is. Item (4) is the one that was
missing in Phase 1.

For (4) we model a caller impersonating "cpp_engine" attempting "physics": we
set the caller identity to "cpp_engine" and attempt a call into physics. Since
glucore.toml declares cpp_engine -> [] (no callees), the core must refuse with
GluStatus::LinkDenied — and crucially items (1)-(3) must STILL work in the same
run, proving the denial didn't poison anything.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
import glucore
from glucore import GluStatus


def main():
    core = glucore.load_core()
    physics = glucore.load_module(core, "physics")
    cpp = glucore.load_module(core, "cpp_engine")
    core.set_caller_identity("python")

    results = []

    # (1) python -> physics
    try:
        r = physics.calculate_force(10.0, 9.8)
        ok = (r == 98.0)
        results.append(("python->physics", "SUCCEED", ok, f"got {r}"))
    except Exception as e:
        results.append(("python->physics", "SUCCEED", False, f"threw {e!r}"))

    # (2) python -> cpp_engine
    try:
        r = cpp.accelerate(5.0, 2.0)
        ok = (abs(r - 24.6) < 1e-9)
        results.append(("python->cpp_engine", "SUCCEED", ok, f"got {r}"))
    except Exception as e:
        results.append(("python->cpp_engine", "SUCCEED", False, f"threw {e!r}"))

    # (3) physics -> cpp_engine (nested Rust->C++ call). boost_with_cpp sets its
    # own caller identity to "physics" for the outbound call.
    try:
        r = physics.boost_with_cpp(3.0, 2.0)  # 6.0 + 22.6
        ok = (abs(r - 28.6) < 1e-9)
        results.append(("physics->cpp_engine", "SUCCEED", ok, f"got {r}"))
    except Exception as e:
        results.append(("physics->cpp_engine", "SUCCEED", False, f"threw {e!r}"))

    # (4) cpp_engine -> physics : DENY. Impersonate caller "cpp_engine" and try
    # to reach physics. Must be refused with LinkDenied, AND not crash.
    core.set_caller_identity("cpp_engine")
    denied_ok = False
    detail = ""
    try:
        # Direct dispatch attempt via the core (bypassing GluProxy's own checks
        # so we observe the CORE's decision, which is what matters).
        packed = [glucore.pack_arg(10.0)[0], glucore.pack_arg(9.8)[0]]
        r = core.call("physics", "calculate_force", packed)
        if r.status == GluStatus.LINK_DENIED:
            denied_ok = True
            detail = f"core returned LinkDenied(status={r.status}) as required"
        else:
            detail = f"EXPECTED LinkDenied but got status={r.status} (BUG)"
    except Exception as e:
        # GluProxy raises on non-OK; but here we read raw status. An exception
        # would indicate an unexpected path.
        detail = f"unexpected exception {e!r}"
    results.append(("cpp_engine->physics", "DENY", denied_ok, detail))

    # Restore python identity and confirm (1) STILL works after the denial —
    # proves the denial didn't corrupt state.
    core.set_caller_identity("python")
    try:
        r = physics.calculate_force(2.0, 10.0)
        after_ok = (r == 20.0)
        results.append(("python->physics (post-deny)", "SUCCEED", after_ok, f"got {r}"))
    except Exception as e:
        results.append(("python->physics (post-deny)", "SUCCEED", False, f"threw {e!r}"))

    # --- report ---
    print("=" * 70)
    print("TASK 7 MULTI-CALLER DoD — single run, single core instance")
    print("=" * 70)
    all_ok = True
    for label, expect, ok, detail in results:
        flag = "PASS" if ok else "FAIL"
        if not ok:
            all_ok = False
        print(f"  [{flag}] {label:<28} expected={expect:<7}  {detail}")
    print("=" * 70)
    if all_ok:
        print("ALL CHECKS PASSED in the same run — permission check discriminates")
        print("between callers (items 3 & 4 differ only by caller identity).")
    else:
        print("SOME CHECKS FAILED.")
        sys.exit(1)


if __name__ == "__main__":
    main()
