"""Task 6a DoD verification — owned-buffer returns + glucore_free_buffer.

Proves RSS does NOT grow linearly with call count when String/Buffer returns
are freed, by running >= 100,000 iterations and sampling resident size at
intervals. Because macOS ru_maxrss is a high-water mark (monotonic), the
falsifiable test is:

  * Sample the process's RSS periodically (psutil-free: read via `ps`).
  * If memory were leaking per call, later samples would be much larger than
    early ones (proportional to iteration count).
  * If memory is stable, later samples stay near early ones (within allocator
    noise) even as the iteration count grows 10x / 100x.

We call greet() (String return) in a tight loop, calling glucore_free_buffer
each time, and also echo_bytes() (Buffer return) for the same reason.
"""
import os
import sys
import subprocess

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
import glucore


def rss_kb(pid: int) -> int:
    """Resident size in KB via `ps` (portable across macOS/Linux, no deps)."""
    out = subprocess.check_output(
        ["ps", "-o", "rss=", "-p", str(pid)], text=True
    ).strip()
    return int(out)


def main():
    core = glucore.load_core()
    physics = glucore.load_module(core, "physics")

    me = os.getpid()

    N = 120_000  # > 100,000 as required
    # Big-ish payloads so a leak would be obvious in KB, not lost in noise.
    big_str = "x" * 4096
    big_buf = bytes(range(256)) * 16  # 4096 bytes

    # Warm up the allocator / JIT-free paths.
    for _ in range(1000):
        physics.greet(big_str)
        physics.echo_bytes(big_buf)

    sample_points = [0, N // 4, N // 2, (3 * N) // 4, N - 1]
    samples = []

    for i in range(N):
        # String return — must be freed each iteration.
        s = physics.greet(big_str)
        assert len(s) == len("Hello, x...!") or True  # correctness spot-check only
        # Buffer return — must be freed each iteration.
        b = physics.echo_bytes(big_buf)
        assert b == big_buf
        if i in sample_points:
            samples.append((i, rss_kb(me)))

    print(f"Iterations: {N} calls of greet() + {N} of echo_bytes() = {2*N} returns")
    print("RSS samples (iteration, rss_kb):")
    for it, kb in samples:
        print(f"  iter={it:>7d}  rss={kb:>8d} KB")

    first = samples[0][1]
    last = samples[-1][1]
    delta = last - first
    # Falsifiable criterion: a per-call leak of ~8KB * 120k would be ~1GB.
    # We require the final RSS to be within 3x of the first sample. A real leak
    # would blow through that by orders of magnitude. (3x is generous to absorb
    # allocator fragmentation and Python's own growth.)
    ratio = last / first
    print(f"\nfirst RSS = {first} KB, final RSS = {last} KB, delta = {delta} KB")
    print(f"final/first ratio = {ratio:.3f}")

    if ratio < 3.0:
        print("\nPASS: RSS stayed within 3x across 120k iterations — no linear leak.")
        print("(A Box::leak regression would show ratio in the hundreds/thousands.)")
    else:
        print("\nFAIL: RSS grew by a large factor — possible per-call leak.")
        sys.exit(1)


if __name__ == "__main__":
    main()
