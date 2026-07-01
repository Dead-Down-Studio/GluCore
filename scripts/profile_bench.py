"""Profile the GluCore benchmark to find perf hotspots."""
import cProfile
import pstats
import io
import struct
import time
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import glucore

COUNT = 50
FRAMES = 1000
DT = 0.2
GRAVITY = 9.8


def pack_state(heights, velocities):
    interleaved = []
    for h, v in zip(heights, velocities):
        interleaved.append(h)
        interleaved.append(v)
    return struct.pack(f"<{len(interleaved)}f", *interleaved)


def main():
    core = glucore.load_core()
    cpp = glucore.load_module(core, "cpp_engine")

    heights = [10.0, 8.0, 6.0, 4.0, 2.0] * (COUNT // 5)
    velocities = [0.0] * COUNT
    state = pack_state(heights, velocities)

    # Warm up
    for _ in range(100):
        state = cpp.step_physics_glucore(state, DT, GRAVITY)

    pr = cProfile.Profile()
    pr.enable()
    for _ in range(FRAMES):
        state = cpp.step_physics_glucore(state, DT, GRAVITY)
    pr.disable()

    s = io.StringIO()
    ps = pstats.Stats(pr, stream=s).sort_stats("cumulative")
    ps.print_stats(30)
    print(s.getvalue())


if __name__ == "__main__":
    main()
