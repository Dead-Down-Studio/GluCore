"""Task 4 DoD verification script.

Verifies all Definition-of-Done items for Task 4 by actually running code:

  1. Physics.calculate_force("not a number", 9.8) raises TypeError with a
     message naming the expected types.
  2. Physics.calculate_force(10.0) (wrong arg count) raises TypeError.
  3. The correct call still returns 98.0.
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
import glucore


def main():
    core = glucore.load_core()
    physics = glucore.load_module(core, "physics")

    # DoD #3: correct call still works (after the validation layer is in place)
    r = physics.calculate_force(10.0, 9.8)
    print(f"[DoD #3] calculate_force(10.0, 9.8) = {r}  (expected 98.0)")
    assert r == 98.0, f"correct call broken: got {r}"

    # DoD #1: type mismatch raises TypeError naming the expected signature
    try:
        physics.calculate_force("not a number", 9.8)
        print("[DoD #1] FAIL: type mismatch did not raise")
        sys.exit(1)
    except TypeError as e:
        msg = str(e)
        print(f"[DoD #1] TypeError: {msg}")
        assert "Float" in msg, f"error message doesn't name expected type: {msg}"
        assert "String" in msg, f"error message doesn't name actual type: {msg}"
        assert "calculate_force" in msg, f"error message doesn't name function: {msg}"

    # DoD #2: wrong arg count raises TypeError
    try:
        physics.calculate_force(10.0)
        print("[DoD #2] FAIL: wrong arg count did not raise")
        sys.exit(1)
    except TypeError as e:
        msg = str(e)
        print(f"[DoD #2] TypeError: {msg}")
        assert "2" in msg and "1" in msg, (
            f"error message doesn't name expected vs actual arg count: {msg}"
        )
        assert "calculate_force" in msg, f"error message doesn't name function: {msg}"

    # Extra: int passed where Float expected (Python int is Int, not Float)
    try:
        physics.calculate_force(10, 9)  # both ints
        print("[extra] FAIL: int-instead-of-float did not raise")
        sys.exit(1)
    except TypeError as e:
        print(f"[extra] int-instead-of-float correctly rejected: {e}")

    # Extra: String function with wrong type
    try:
        physics.greet(42)  # int instead of String
        print("[extra] FAIL: greet(42) did not raise")
        sys.exit(1)
    except TypeError as e:
        print(f"[extra] greet(42) correctly rejected: {e}")

    # Extra: Buffer function with wrong type
    try:
        physics.echo_bytes("not bytes")  # str instead of Buffer
        print("[extra] FAIL: echo_bytes('not bytes') did not raise")
        sys.exit(1)
    except TypeError as e:
        print(f"[extra] echo_bytes('not bytes') correctly rejected: {e}")

    # Extra: unknown function name raises AttributeError (not TypeError)
    try:
        physics.nonexistent_fn(1, 2, 3)
        print("[extra] FAIL: nonexistent function did not raise")
        sys.exit(1)
    except AttributeError as e:
        print(f"[extra] nonexistent function correctly rejected: {e}")

    # Sanity: list the discovered signatures (proves the registry is populated)
    print("\nDiscovered signatures:")
    for name, sig in sorted(physics.functions().items()):
        print(f"  physics.{name}{sig!r}")

    print("\nAll Task 4 DoD items verified by running code.")


if __name__ == "__main__":
    main()
