"""Part 0b re-verification: caller-stack restore cleanliness.

The original Phase 2 report stated `python -> physics` returned 98.0 earlier
and 19.6 later in the same paragraph, for what reads as the same call. This
script re-runs the exact sequence — the denied nested call, then the same
`python -> physics` call with the SAME arguments immediately after — and
reports the literal output.

If both calls return 98.0, the caller-stack restore (CallerGuard::drop /
the manual restore in cpp_engine.attempt_physics_call_from_cpp) is clean.
If they differ, that's a real bug in the Drop-based restore and needs to
be fixed, not narrated around.

Sequence:
  (1) python -> physics(10.0, 9.8)  : expected 98.0
  (2) cpp_engine -> physics(10.0, 9.8) : DENIED (cpp_engine has no link to physics)
  (3) python -> physics(10.0, 9.8)  : expected 98.0 (SAME args as (1), must match)
  (4) python -> physics(2.0, 10.0)  : expected 20.0 (different args, sanity check)
  (5) physics -> cpp_engine(3.0, 2.0) : expected 28.6 (nested Rust->C++)
  (6) python -> physics(10.0, 9.8)  : expected 98.0 (after a DIFFERENT nested call)

Items (1) and (3) use the same args and must return the same value.
Item (6) confirms the restore is clean even after a SUCCESSFUL nested call.
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

    print("=" * 78)
    print("PART 0b RE-VERIFICATION — caller-stack restore cleanliness")
    print("=" * 78)
    print()
    print("Args used at every python->physics probe: (10.0, 9.8)  -> expected 98.0")
    print("Args used at the denied C++ probe:       (10.0, 9.8)  -> same args")
    print()

    all_ok = True

    # (1) python -> physics(10.0, 9.8) — baseline
    r1 = physics.calculate_force(10.0, 9.8)
    ok1 = (r1 == 98.0)
    print(f"  (1) python->physics(10.0, 9.8)         = {r1}    expected 98.0   "
          f"[{'PASS' if ok1 else 'FAIL'}]")
    if not ok1: all_ok = False

    # (2) cpp_engine -> physics(10.0, 9.8) — DENIED. Same args as (1).
    status2 = cpp.attempt_physics_call_from_cpp()
    ok2 = (status2 == GluStatus.LINK_DENIED)
    print(f"  (2) cpp_engine->physics(10.0, 9.8)     = status={status2}    expected 4 (LinkDenied)   "
          f"[{'PASS' if ok2 else 'FAIL'}]")
    if not ok2: all_ok = False

    # (3) python -> physics(10.0, 9.8) — SAME args as (1), must return the SAME value.
    r3 = physics.calculate_force(10.0, 9.8)
    ok3 = (r3 == 98.0 and r3 == r1)
    print(f"  (3) python->physics(10.0, 9.8)         = {r3}    expected 98.0 (same as (1))   "
          f"[{'PASS' if ok3 else 'FAIL'}]")
    if not ok3: all_ok = False

    # (4) python -> physics(2.0, 10.0) — different args, sanity check.
    r4 = physics.calculate_force(2.0, 10.0)
    ok4 = (r4 == 20.0)
    print(f"  (4) python->physics(2.0, 10.0)         = {r4}    expected 20.0   "
          f"[{'PASS' if ok4 else 'FAIL'}]")
    if not ok4: all_ok = False

    # (5) physics -> cpp_engine(3.0, 2.0) — SUCCESSFUL nested Rust->C++ call.
    # This is the case where the caller identity IS temporarily changed to
    # "physics" and then restored. The restore here exercises the same path
    # the denied call did, but on the success branch.
    r5 = physics.boost_with_cpp(3.0, 2.0)
    ok5 = (abs(r5 - 28.6) < 1e-9)
    print(f"  (5) physics->cpp_engine(3.0, 2.0)      = {r5}   expected 28.6   "
          f"[{'PASS' if ok5 else 'FAIL'}]")
    if not ok5: all_ok = False

    # (6) python -> physics(10.0, 9.8) — same args as (1) and (3), after a
    # successful nested call. Must STILL be 98.0.
    r6 = physics.calculate_force(10.0, 9.8)
    ok6 = (r6 == 98.0 and r6 == r1)
    print(f"  (6) python->physics(10.0, 9.8)         = {r6}    expected 98.0 (same as (1) and (3))   "
          f"[{'PASS' if ok6 else 'FAIL'}]")
    if not ok6: all_ok = False

    print()
    print("=" * 78)
    if all_ok:
        print("ALL CHECKS PASSED. Caller-stack restore is clean:")
        print("  - After a DENIED nested call, the outer caller is restored.")
        print("  - After a SUCCESSFUL nested call, the outer caller is restored.")
        print("  - The same args (10.0, 9.8) return 98.0 every time.")
        print()
        print("The '98.0 vs 19.6' discrepancy in the original Phase 2 report")
        print("was a transcription error, NOT a Drop-restore bug. The literal")
        print("output above shows 98.0 at every probe.")
    else:
        print("SOME CHECKS FAILED — caller-stack restore has a real bug.")
        sys.exit(1)


if __name__ == "__main__":
    main()
