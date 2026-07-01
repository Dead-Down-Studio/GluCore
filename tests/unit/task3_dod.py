"""Task 3 DoD verification script — POST-TASK-4 (return_tag auto-determined)."""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
import glucore


def main():
    core = glucore.load_core()
    physics = glucore.load_module(core, "physics")

    # DoD #3: existing f64/i64 paths still work
    r_force = physics.calculate_force(10.0, 9.8)
    print(f"[DoD #3] calculate_force(10.0, 9.8) = {r_force}  (expected 98.0)")
    assert r_force == 98.0, "REGRESSION: f64 broken"

    r_int = physics.add_ints(7, 35)
    print(f"[DoD #3] add_ints(7, 35) = {r_int}  (expected 42)")
    assert r_int == 42, "REGRESSION: i64 broken"

    # DoD #1: String round-trip
    r_greet = physics.greet("World")
    print(f"[DoD #1] greet('World') = {r_greet!r}  (expected 'Hello, World!')")
    assert r_greet == "Hello, World!", f"String round-trip broken: got {r_greet!r}"

    # DoD #1 with non-ASCII (multi-byte UTF-8)
    r_greet_non_ascii = physics.greet("Müller")
    print(f"[DoD #1] greet('Müller') = {r_greet_non_ascii!r}")
    assert r_greet_non_ascii == "Hello, Müller!", f"non-ASCII String broken"

    # DoD #2: Buffer round-trip
    payload = bytes([0, 1, 2, 3, 255, 254, 0, 7])
    r_echo = physics.echo_bytes(payload)
    print(f"[DoD #2] echo_bytes({payload!r}) = {r_echo!r}")
    assert r_echo == payload, f"Buffer round-trip broken: got {r_echo!r}"

    # Empty buffer edge case
    r_empty = physics.echo_bytes(b"")
    print(f"[DoD #2] echo_bytes(b'') = {r_empty!r}")
    assert r_empty == b"", f"empty Buffer broken: got {r_empty!r}"

    # Borrowed variants
    r_shout = physics.shout("hello")
    print(f"[borrowed] shout('hello') = {r_shout!r}  (expected 'HELLO')")
    assert r_shout == "HELLO", f"borrowed &str broken: got {r_shout!r}"

    r_first = physics.first_byte(bytes([42, 0, 1]))
    print(f"[borrowed] first_byte([42,0,1]) = {r_first}  (expected 42)")
    assert r_first == 42, f"borrowed &[u8] broken: got {r_first}"

    print("\nAll Task 3 DoD items verified by running code.")


if __name__ == "__main__":
    main()
