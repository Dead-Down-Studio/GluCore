"""Task 11+ production hardening checks.

Verifies:
  1) Runtime exposes explicit ABI/Wire contract versions.
  2) Contract versions are parseable and stable tuples.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
import glucore


def main():
    core = glucore.load_core()
    versions = core.contract_versions()

    print(f"contract versions: {versions}")
    assert "abi" in versions and "wire" in versions, "missing version keys"
    assert len(versions["abi"]) == 3 and len(versions["wire"]) == 3, "version tuple shape invalid"
    assert all(isinstance(v, int) for v in versions["abi"]), "abi tuple is not numeric"
    assert all(isinstance(v, int) for v in versions["wire"]), "wire tuple is not numeric"
    assert versions["abi"][0] >= 1, "abi major must be >= 1"
    assert versions["wire"][0] >= 1, "wire major must be >= 1"

    print("production contract version checks passed.")


if __name__ == "__main__":
    main()
