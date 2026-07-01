"""
bench_raw.py — the raw-ctypes side of the benchmark.

Same workload, same COUNT/FRAMES as bench_glucore.py, so the two numbers are
directly comparable: no registry lookup, no link check, no type validation,
no copy — just a direct function pointer call mutating memory in place.
"""

import os
import time
import ctypes
import platform


COUNT  = 50    # must match bench_glucore.py
FRAMES = 1000  # must match bench_glucore.py
DT = 0.2
GRAVITY = 9.8


def load_engine():
    """Load the raw engine (scripts/engine.so) directly."""
    ext = {"Linux": "so", "Darwin": "dylib"}.get(platform.system(), "so")
    here = os.path.dirname(os.path.abspath(__file__))
    lib = ctypes.CDLL(os.path.join(here, f"engine.{ext}"))
    lib.step_physics.argtypes = [
        ctypes.POINTER(ctypes.c_float),
        ctypes.POINTER(ctypes.c_float),
        ctypes.c_int, ctypes.c_float, ctypes.c_float,
    ]
    lib.step_physics.restype = None
    return lib


def main():
    engine = load_engine()

    base = [10.0, 8.0, 6.0, 4.0, 2.0] * (COUNT // 5)
    heights    = (ctypes.c_float * COUNT)(*base)
    velocities = (ctypes.c_float * COUNT)(*[0.0] * COUNT)

    print(f"Running {FRAMES} frames, {COUNT} entities, through raw ctypes "
          f"(direct function pointer call, in-place mutation, no copy)...")

    start = time.perf_counter()
    for _frame in range(FRAMES):
        engine.step_physics(heights, velocities, COUNT, DT, GRAVITY)
    elapsed = time.perf_counter() - start

    print(f"\nFinal heights (first 5): {[round(heights[i], 2) for i in range(5)]}")
    print(f"\nTotal time:        {elapsed:.4f}s")
    print(f"Per-frame avg:     {(elapsed / FRAMES) * 1000:.4f}ms")
    print(f"Calls/sec:         {FRAMES / elapsed:.0f}")


if __name__ == "__main__":
    main()
