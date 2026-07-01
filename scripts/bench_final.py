"""Final consolidated benchmark for the Phase 2 Addendum report.

Runs both raw-ctypes and GluCore benchmarks with the SAME workload,
multiple iterations, and reports min/median/max for each. This is the
number the report cites.
"""
import os
import sys
import time
import ctypes
import statistics

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

COUNT = 50
FRAMES = 1000
DT = 0.2
GRAVITY = 9.8
RUNS = 5


def bench_raw():
    """Raw ctypes: direct function pointer, in-place mutation, no copy."""
    # Load the raw engine directly (scripts/engine.so built by build.sh)
    import platform
    ext = {"Linux": "so", "Darwin": "dylib"}.get(platform.system(), "so")
    engine_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), f"engine.{ext}")
    engine = ctypes.CDLL(engine_path)
    engine.step_physics.argtypes = [
        ctypes.POINTER(ctypes.c_float),
        ctypes.POINTER(ctypes.c_float),
        ctypes.c_int, ctypes.c_float, ctypes.c_float,
    ]
    engine.step_physics.restype = None

    base = [10.0, 8.0, 6.0, 4.0, 2.0] * (COUNT // 5)
    heights = (ctypes.c_float * COUNT)(*base)
    velocities = (ctypes.c_float * COUNT)(*[0.0] * COUNT)

    # Warm-up
    for _ in range(100):
        engine.step_physics(heights, velocities, COUNT, DT, GRAVITY)

    times = []
    for _run in range(RUNS):
        # Reset state
        for i in range(COUNT):
            heights[i] = base[i]
            velocities[i] = 0.0
        start = time.perf_counter()
        for _frame in range(FRAMES):
            engine.step_physics(heights, velocities, COUNT, DT, GRAVITY)
        times.append(time.perf_counter() - start)
    return times


def bench_glucore():
    """GluCore: registry lookup + link check + type validation + Buffer copy."""
    import glucore
    import struct

    core = glucore.load_core()
    cpp = glucore.load_module(core, "cpp_engine")

    def pack_state(heights, velocities):
        interleaved = []
        for h, v in zip(heights, velocities):
            interleaved.append(h)
            interleaved.append(v)
        return struct.pack(f"<{len(interleaved)}f", *interleaved)

    heights = [10.0, 8.0, 6.0, 4.0, 2.0] * (COUNT // 5)
    velocities = [0.0] * COUNT

    # Warm-up
    state = pack_state(heights, velocities)
    for _ in range(100):
        state = cpp.step_physics_glucore(state, DT, GRAVITY)

    times = []
    for _run in range(RUNS):
        state = pack_state(heights, velocities)
        start = time.perf_counter()
        for _frame in range(FRAMES):
            state = cpp.step_physics_glucore(state, DT, GRAVITY)
        times.append(time.perf_counter() - start)
    return times


def report(name, times):
    per_call_us = [(t / FRAMES) * 1e6 for t in times]
    calls_per_sec = [FRAMES / t for t in times]
    print(f"\n{name}:")
    print(f"  per-call (µs):  min={min(per_call_us):.3f}  median={statistics.median(per_call_us):.3f}  max={max(per_call_us):.3f}")
    print(f"  calls/sec:      min={min(calls_per_sec):,.0f}  median={statistics.median(calls_per_sec):,.0f}  max={max(calls_per_sec):,.0f}")
    return statistics.median(per_call_us), statistics.median(calls_per_sec)


def main():
    print(f"Workload: {FRAMES} frames, {COUNT} entities, {RUNS} runs per benchmark")
    print(f"  raw:    direct ctypes fn pointer, in-place float[] mutation, no copy")
    print(f"  glucore: registry + link check + type validation + Buffer copy in/out")

    raw_us, raw_cps = report("raw ctypes", bench_raw())
    gc_us, gc_cps = report("GluCore", bench_glucore())

    print(f"\n--- Comparison ---")
    print(f"  median per-call: raw={raw_us:.3f}µs  glucore={gc_us:.3f}µs  ratio={gc_us/raw_us:.1f}x slower")
    print(f"  median calls/sec: raw={raw_cps:,.0f}  glucore={gc_cps:,.0f}")
    print(f"\nOriginal Phase 2 report: 37x slower (per-call 0.0014ms vs 0.0516ms).")
    print(f"This run:                  {gc_us/raw_us:.1f}x slower (per-call {raw_us/1000:.4f}ms vs {gc_us/1000:.4f}ms).")


if __name__ == "__main__":
    main()
