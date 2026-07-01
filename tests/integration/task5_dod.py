"""Task 5 DoD verification script.

Verifies all Definition-of-Done items for Task 5 by actually running code:

  1. A call from a module with no declared link to the callee returns an
     error result with a clear status. Verified by temporarily rewriting
     glucore.toml to remove `physics` from python's allowed callees,
     reloading the core (which re-reads the toml), and observing the call
     fail with GluStatus::LinkDenied.
  2. Restoring the link entry makes the call succeed again.
  3. The existing demo (`python3 main.py`) still passes with the real,
     intended glucore.toml links in place — verified separately by running
     main.py.

This script writes a temporary glucore.toml, runs a fresh Python subprocess
against it (so the core is loaded fresh and re-reads the toml), and checks
the output.
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
TOML = REPO / "glucore.toml"
BACKUP = REPO / "glucore.toml.bak"

REAL_TOML = """\
# glucore.toml — GluCore manifest

[modules]
physics = "physics"

[links]
python = ["physics"]
physics = []
"""

# Toml with the python -> physics link removed. The call from python to
# physics should be denied.
NO_LINK_TOML = """\
# glucore.toml — GluCore manifest (Task 5 DoD test: link removed)

[modules]
physics = "physics"

[links]
python = []
physics = []
"""

# Test runner: a small Python script that loads the core and tries the call.
# We embed it as a subprocess so each test gets a fresh process with a fresh
# core (no leftover link graph from a prior run).
TEST_RUNNER = """
import sys, os
sys.path.insert(0, os.environ.get("GLUCORE_ROOT", "."))
import glucore

core = glucore.load_core()
physics = glucore.load_module(core, "physics")
try:
    r = physics.calculate_force(10.0, 9.8)
    print(f"CALL_OK: {r}")
except RuntimeError as e:
    print(f"CALL_DENIED: {e}")
"""


def run_test(toml_content: str) -> str:
    """Write toml_content to glucore.toml, run TEST_RUNNER, return stdout."""
    TOML.write_text(toml_content)
    env = dict(os.environ)
    env["GLUCORE_ROOT"] = str(REPO)
    proc = subprocess.run(
        [sys.executable, "-c", TEST_RUNNER],
        capture_output=True, text=True, timeout=30, env=env,
    )
    return proc.stdout.strip()


def main():
    # Back up the real toml.
    shutil.copy(TOML, BACKUP)
    try:
        # DoD #3: with the real toml, the call succeeds.
        out_real = run_test(REAL_TOML)
        print(f"[DoD #3] with real toml: {out_real}")
        assert "CALL_OK" in out_real and "98.0" in out_real, (
            f"real toml call failed: {out_real}"
        )

        # DoD #1: with the link removed, the call is denied with LinkDenied.
        out_no_link = run_test(NO_LINK_TOML)
        print(f"[DoD #1] with link removed: {out_no_link}")
        assert "CALL_DENIED" in out_no_link, (
            f"link removal did not deny the call: {out_no_link}"
        )
        assert "status=4" in out_no_link, (
            f"denied but wrong status (expected 4=LinkDenied): {out_no_link}"
        )
        assert "link denied" in out_no_link.lower(), (
            f"denied but message doesn't say 'link denied': {out_no_link}"
        )

        # DoD #2: restoring the link makes the call succeed again.
        out_restored = run_test(REAL_TOML)
        print(f"[DoD #2] after restore: {out_restored}")
        assert "CALL_OK" in out_restored and "98.0" in out_restored, (
            f"restore did not re-allow the call: {out_restored}"
        )

        print("\nAll Task 5 DoD items verified by running code.")
    finally:
        # Restore the real toml no matter what.
        shutil.copy(BACKUP, TOML)
        BACKUP.unlink()


if __name__ == "__main__":
    main()
