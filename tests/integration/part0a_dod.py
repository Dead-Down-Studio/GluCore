"""Part 0a re-verification: REAL cpp_engine -> physics attempt (denied),
in the same run as python -> physics (succeeds) and physics -> cpp_engine
(succeeds).

The original task7_multicaller_dod tested item (4) by IMPERSONATING the
caller — Python called glucore_set_caller_identity("cpp_engine") and then
made a Python-initiated glucore_call into physics. That's a weaker claim
than the DoD actually asked for: a router that only checks "is `physics`
the target of any declared link at all" would also pass that test and
still be wrong.

This script does the REAL version: cpp_engine.attempt_physics_call_from_cpp()
is a genuine C++ function that calls glucore_call("physics", ...) directly
from C++ code. With cpp_engine -> [] in glucore.toml, the core MUST refuse
with LinkDenied.

Items, in one run, on one core instance:
  (1) python  -> physics        : SUCCEED  (link declared)
  (2) python  -> cpp_engine     : SUCCEED  (link declared)
  (3) physics -> cpp_engine     : SUCCEED  (nested Rust->C++ call, link declared)
  (4) cpp_engine -> physics     : DENY     (REAL C++-initiated call, link NOT declared)

The point: (3) and (4) target modules that are both reachable from *someone*;
what matters is WHO THE CALLER IS. Item (4) is now a real C++ call, not a
Python impersonation.
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

    # (4) REAL cpp_engine -> physics : DENY. This is the part that was missing
    # in the original Phase 2 verification. cpp_engine.attempt_physics_call_from_cpp()
    # is a genuine C++ function that calls glucore_call("physics", ...) directly
    # from C++ — not a Python impersonation. It returns the GluStatus code.
    try:
        status_code = cpp.attempt_physics_call_from_cpp()
        # GluStatus.LinkDenied == 4
        if status_code == GluStatus.LINK_DENIED:
            ok = True
            detail = f"REAL C++ caller got LinkDenied(status={status_code}) as required"
        elif status_code == GluStatus.OK:
            ok = False
            detail = f"BUG: REAL C++ caller to physics SUCCEEDED (status={status_code}) — link check broken"
        else:
            ok = False
            detail = f"unexpected status={status_code}"
        results.append(("cpp_engine->physics (REAL C++)", "DENY", ok, detail))
    except Exception as e:
        results.append(("cpp_engine->physics (REAL C++)", "DENY", False, f"threw {e!r}"))

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
    print("=" * 78)
    print("PART 0a RE-VERIFICATION — REAL C++ caller, single run, single core")
    print("=" * 78)
    all_ok = True
    for label, expect, ok, detail in results:
        flag = "PASS" if ok else "FAIL"
        if not ok:
            all_ok = False
        print(f"  [{flag}] {label:<32} expected={expect:<7}  {detail}")
    print("=" * 78)
    if all_ok:
        print("ALL CHECKS PASSED in the same run — the router discriminates")
        print("between callers using a REAL C++-initiated call (item 4), not a")
        print("Python impersonation. The original Phase 2 weakness is fixed.")
    else:
        print("SOME CHECKS FAILED.")
        sys.exit(1)


if __name__ == "__main__":
    main()
