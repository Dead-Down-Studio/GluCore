"""glucore — Python adapter for GluCore v2.

Re-exports everything from the submodules so `import glucore` works the
same as before the split. Test scripts do:

    sys.path.insert(0, '<project_root>/adapters/python')
    import glucore

and then use glucore.load_core(), glucore.GluValue, glucore.pack_arg, etc.
"""

from bindings import (
    GluStatus, GluTypeTag, GluSlice, GluValue, GluResult,
    GluSignatureFFI, GluWrapper, GluExportEntry, GluModule, GluCore,
)
from glu_types import (
    Signature, pack_arg, unpack_return, PY_TYPE_TO_TAG,
    _make_packer, err_prefix,
)
from adapter import (
    GluProxy, load_core, load_module, load_process_module,
    _platform_lib_filename,
)

__all__ = [
    # Bindings
    "GluStatus", "GluTypeTag", "GluSlice", "GluValue", "GluResult",
    "GluSignatureFFI", "GluWrapper", "GluExportEntry", "GluModule", "GluCore",
    # Types
    "Signature", "pack_arg", "unpack_return", "PY_TYPE_TO_TAG",
    # Adapter
    "GluProxy", "load_core", "load_module", "load_process_module",
]
