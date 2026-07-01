"""Task 1 DoD verification script — POST-TASK-4 (return_tag auto-determined)."""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
import glucore


def main():
    core = glucore.load_core()
    physics = glucore.load_module(core, "physics")

    # DoD #4: happy path still works
    r1 = physics.calculate_force(10.0, 9.8)
    print(f"[DoD #4] calculate_force(10.0, 9.8) = {r1}  (expected 98.0)")
    assert r1 == 98.0, "REGRESSION: calculate_force broken"

    # DoD #1: i64 return type produces correct output
    r2 = physics.add_ints(7, 35)
    print(f"[DoD #1] add_ints(7, 35) = {r2}  (expected 42)")
    assert r2 == 42, f"i64 return broken: got {r2}, expected 42"

    # DoD #3: panic in native fn returns error result, doesn't crash process
    try:
        physics.boom()
        print("[DoD #3] FAIL: boom() did not raise")
        sys.exit(1)
    except RuntimeError as e:
        print(f"[DoD #3] boom() raised as expected: {e}")
        assert "panic in boom" in str(e), f"unexpected error message: {e}"

    print("[DoD #3] process survived panic — FFI boundary is panic-safe")
    print("\nAll Task 1 DoD items verified by running code.")


if __name__ == "__main__":
    main()
