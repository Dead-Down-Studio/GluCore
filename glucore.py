"""glucore.py — shim that re-exports from adapters/python/.

This file exists so that test scripts doing `import glucore` (with the
project root on sys.path) continue to work after the adapter was split
into the adapters/python/ package. The real code lives in
adapters/python/{bindings,types,adapter,gc_pin}.py.
"""
import os
import sys

_adapters_python = os.path.join(os.path.dirname(os.path.abspath(__file__)), "adapters", "python")
if _adapters_python not in sys.path:
    sys.path.insert(0, _adapters_python)

# Import the submodules directly (adapters/python/ is on sys.path).
from bindings import *  # noqa: F401,F403
from glu_types import *  # noqa: F401,F403
from adapter import *  # noqa: F401,F403

# Also import specific names for `from glucore import X` usage
from bindings import (  # noqa: F401
    GluStatus, GluTypeTag, GluSlice, GluValue, GluResult,
    GluSignatureFFI, GluWrapper, GluExportEntry, GluModule, GluCore,
)
from glu_types import (  # noqa: F401
    Signature, pack_arg, unpack_return, PY_TYPE_TO_TAG,
    _make_packer, err_prefix,
)
from adapter import (  # noqa: F401
    GluProxy, load_core, load_module, load_process_module,
    _platform_lib_filename,
)
