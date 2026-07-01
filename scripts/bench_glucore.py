"""
bench_glucore.py — the GluCore side of the benchmark.

Same bouncing-ball workload as pycpp_test/main.py, but routed through the
real GluCore call path: registry lookup, link permission check, type-tag
validation, and Buffer copy-in/copy-out — instead of raw ctypes mutating a
pointer in place.

Run this from the glucore_v2 project root (it imports glucore.py from there).
Requires the Rust core + cpp_engine to actually be built first:
    cargo build --release          (builds glucore_core + physics)
    cmake -S cpp_engine -B cpp_engine/build && cmake --build cpp_engine/build

This was NOT executed end-to-end in the environment that wrote it — there's
no Rust toolchain available there. The C++ addition (step_physics_glucore in
engine.cpp) was verified to compile cleanly against the real glucore_export.hpp
header. Run this yourself and treat the first run as the real verification,
not this comment.
"""

import struct
import time
import sys
import os
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import glucore


COUNT = 50          # entities, matches the raw-ctypes benchmark
FRAMES = 1000        # frames, matches the raw-ctypes benchmark
DT = 0.2
GRAVITY = 9.8


def pack_state(heights: list[float], velocities: list[float]) -> bytes:
    """Interleave [h0, v0, h1, v1, ...] and pack as raw float bytes —
    the layout step_physics_glucore expects."""
    interleaved = []
    for h, v in zip(heights, velocities):
        interleaved.append(h)
        interleaved.append(v)
    return struct.pack(f"<{len(interleaved)}f", *interleaved)


def unpack_state(data: bytes) -> tuple[list[float], list[float]]:
    n = len(data) // 4
    flat = struct.unpack(f"<{n}f", data)
    heights    = list(flat[0::2])
    velocities = list(flat[1::2])
    return heights, velocities


def main():
    core = glucore.load_core()
    cpp = glucore.load_module(core, "cpp_engine")

    heights    = [10.0, 8.0, 6.0, 4.0, 2.0] * (COUNT // 5)
    velocities = [0.0] * COUNT

    state = pack_state(heights, velocities)

    print(f"Running {FRAMES} frames, {COUNT} entities, through GluCore's "
          f"actual call path (registry lookup + link check + type "
          f"validation + Buffer copy in/out each call)...")

    start = time.perf_counter()
    for _frame in range(FRAMES):
        state = cpp.step_physics_glucore(state, DT, GRAVITY)
    elapsed = time.perf_counter() - start

    heights, velocities = unpack_state(state)

    print(f"\nFinal heights (first 5): {[round(h, 2) for h in heights[:5]]}")
    print(f"\nTotal time:        {elapsed:.4f}s")
    print(f"Per-frame avg:     {(elapsed / FRAMES) * 1000:.4f}ms")
    print(f"Calls/sec:         {FRAMES / elapsed:.0f}")
    print(f"\nCompare this number directly against pycpp_test's raw-ctypes "
          f"run for the same {COUNT} entities / {FRAMES} frames — that's "
          f"the actual benchmark.")


if __name__ == "__main__":
    main()
