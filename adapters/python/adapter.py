"""adapter.py — GluProxy (module proxy) and load functions.

The GluProxy builds per-function caller closures at construction time,
binding them as real attributes so the hot path skips __getattr__ entirely.
The callers are generated via exec() with inlined type checks and packing
for maximum performance. See doc/07-performance.md for details.
"""

from __future__ import annotations

import ctypes
import os
import tomllib
from pathlib import Path
from typing import Any

from bindings import (
    GluStatus, GluTypeTag, GluSlice, GluValue, GluResult,
    GluSignatureFFI, GluWrapper, GluExportEntry, GluModule, GluCore,
)
from glu_types import (
    Signature, pack_arg, unpack_return, PY_TYPE_TO_TAG,
    _make_packer, err_prefix,
)


def _platform_lib_filename(base_name: str) -> str:
    """Return the platform-correct shared-library filename."""
    import sys
    if sys.platform.startswith("win"):
        return f"{base_name}.dll"
    if sys.platform == "darwin":
        return f"lib{base_name}.dylib"
    return f"lib{base_name}.so"


def _project_root() -> Path:
    """Return the project root directory.

    The adapter lives at <root>/adapters/python/adapter.py, so the project
    root is two levels up from this file.
    """
    return Path(__file__).resolve().parent.parent.parent


class GluProxy:
    """Stand-in for a module.

    On construction, queries the core for the signature of every exported
    function in the module and stores them in a local dict. Each call goes
    through pre-call validation: arg count and per-arg Python type are
    checked against the stored signature BEFORE the ctypes boundary.

    PERFORMANCE NOTE: caller closures are built ONCE in __init__ and bound
    as real attributes via setattr. The hot path is `proxy.foo(args)` →
    direct attribute lookup → caller(args), skipping __getattr__ entirely.
    """

    def __init__(self, core: GluCore, module_name: str, module_info=None):
        object.__setattr__(self, "_core", core)
        object.__setattr__(self, "_module", module_name)
        object.__setattr__(self, "_module_bytes", module_name.encode())
        signatures: dict[str, Signature] = {}
        if module_info is not None:
            n = module_info.count
            entries_ptr = module_info.entries
            for i in range(n):
                entry = entries_ptr[i]
                fn_name = ctypes.string_at(entry.name).decode()
                sig = core.get_signature(module_name, fn_name)
                signatures[fn_name] = sig
        else:
            if not hasattr(core, "_glucore_get_module_export_count"):
                core._lib.glucore_get_module_export_count.argtypes = [ctypes.c_char_p]
                core._lib.glucore_get_module_export_count.restype = ctypes.c_size_t
                core._lib.glucore_get_module_export_name.argtypes = [
                    ctypes.c_char_p, ctypes.c_size_t,
                ]
                core._lib.glucore_get_module_export_name.restype = ctypes.c_char_p
                core._glucore_get_module_export_count = (
                    core._lib.glucore_get_module_export_count
                )
                core._glucore_get_module_export_name = (
                    core._lib.glucore_get_module_export_name
                )
            module_b = module_name.encode()
            n = core._glucore_get_module_export_count(module_b)
            for i in range(n):
                name_ptr = core._glucore_get_module_export_name(module_b, i)
                if not name_ptr:
                    continue
                fn_name = ctypes.string_at(name_ptr).decode()
                sig = core.get_signature(module_name, fn_name)
                signatures[fn_name] = sig
        object.__setattr__(self, "_signatures", signatures)
        for fn_name, sig in signatures.items():
            caller = self._make_caller(fn_name, sig)
            object.__setattr__(self, fn_name, caller)

    def _make_caller(self, fn_name: str, sig: Signature):
        module_b = self._module_bytes
        fn_b = fn_name.encode()
        core = self._core
        module_str = self._module
        param_tags = sig.param_tags
        nparams = len(param_tags)
        return_tag = sig.return_tag
        sig_repr = repr(sig)
        null_ptr = core._null_ptr
        glucore_call = core._glucore_call
        free_buf = core._glucore_free_buffer

        if nparams == 0:
            def caller_zero():
                result = glucore_call(module_b, fn_b, null_ptr, 0)
                if result.status != GluStatus.OK:
                    msg = (ctypes.string_at(result.message).decode()
                           if result.message else "(no message)")
                    raise RuntimeError(
                        f"glucore call failed: status={result.status} msg={msg}"
                    )
                return unpack_return(result.value, return_tag, free_buf)
            return caller_zero

        arg_names = [f"a{i}" for i in range(nparams)]
        lines = []
        lines.append(f"def caller(*args):")
        lines.append(f"    if len(args) != {nparams}:")
        lines.append(f"        raise TypeError(")
        lines.append(f"            f'{module_str}.{fn_name}{sig_repr} expected '")
        lines.append(f"            f'{nparams} argument(s), got {{len(args)}}'")
        lines.append(f"        )")
        if nparams == 1:
            lines.append(f"    {arg_names[0]}, = args")
        elif nparams > 1:
            lines.append(f"    {', '.join(arg_names)} = args")
        lines.append(f"    arr = reusable_arr")

        keep_vars = []
        for i, tag in enumerate(param_tags):
            arg = arg_names[i]
            if tag == "Float":
                lines.append(f"    if type({arg}) is float:")
                lines.append(f"        arr[{i}].float = {arg}")
                lines.append(f"    elif isinstance({arg}, (bool, int)):")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected Float, got Int (python type {{type({arg}).__name__}})'")
                lines.append(f"        )")
                lines.append(f"    elif isinstance({arg}, str):")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected Float, got String (python type {{type({arg}).__name__}})'")
                lines.append(f"        )")
                lines.append(f"    elif isinstance({arg}, (bytes, bytearray)):")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected Float, got Buffer (python type {{type({arg}).__name__}})'")
                lines.append(f"        )")
                lines.append(f"    else:")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected Float, got unsupported python type {{type({arg}).__name__}}'")
                lines.append(f"        )")
            elif tag == "Int":
                lines.append(f"    if type({arg}) is bool:")
                lines.append(f"        arr[{i}].int = int({arg})")
                lines.append(f"    elif type({arg}) is int:")
                lines.append(f"        arr[{i}].int = {arg}")
                lines.append(f"    elif isinstance({arg}, float):")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected Int, got Float (python type {{type({arg}).__name__}})'")
                lines.append(f"        )")
                lines.append(f"    elif isinstance({arg}, str):")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected Int, got String (python type {{type({arg}).__name__}})'")
                lines.append(f"        )")
                lines.append(f"    elif isinstance({arg}, (bytes, bytearray)):")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected Int, got Buffer (python type {{type({arg}).__name__}})'")
                lines.append(f"        )")
                lines.append(f"    else:")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected Int, got unsupported python type {{type({arg}).__name__}}'")
                lines.append(f"        )")
            elif tag == "String":
                lines.append(f"    if type({arg}) is str:")
                lines.append(f"        _b{i} = {arg}.encode('utf-8')")
                lines.append(f"        _c{i} = ctypes.c_char_p(_b{i})")
                lines.append(f"        arr[{i}].string = GluSlice(ptr=ctypes.cast(_c{i}, ctypes.c_void_p).value, len=len(_b{i}))")
                lines.append(f"    elif isinstance({arg}, (bool, int)):")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected String, got Int (python type {{type({arg}).__name__}})'")
                lines.append(f"        )")
                lines.append(f"    elif isinstance({arg}, float):")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected String, got Float (python type {{type({arg}).__name__}})'")
                lines.append(f"        )")
                lines.append(f"    elif isinstance({arg}, (bytes, bytearray)):")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected String, got Buffer (python type {{type({arg}).__name__}})'")
                lines.append(f"        )")
                lines.append(f"    else:")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected String, got unsupported python type {{type({arg}).__name__}}'")
                lines.append(f"        )")
                keep_vars.append(f"_c{i}")
            elif tag == "Buffer":
                lines.append(f"    if type({arg}) is bytes:")
                lines.append(f"        _b{i} = {arg}")
                lines.append(f"    elif type({arg}) is bytearray:")
                lines.append(f"        _b{i} = bytes({arg})")
                lines.append(f"    elif isinstance({arg}, (bool, int)):")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected Buffer, got Int (python type {{type({arg}).__name__}})'")
                lines.append(f"        )")
                lines.append(f"    elif isinstance({arg}, float):")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected Buffer, got Float (python type {{type({arg}).__name__}})'")
                lines.append(f"        )")
                lines.append(f"    elif isinstance({arg}, str):")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected Buffer, got String (python type {{type({arg}).__name__}})'")
                lines.append(f"        )")
                lines.append(f"    else:")
                lines.append(f"        raise TypeError(")
                lines.append(f"            f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"            f'expected Buffer, got unsupported python type {{type({arg}).__name__}}'")
                lines.append(f"        )")
                lines.append(f"    _c{i} = ctypes.c_char_p(_b{i})")
                lines.append(f"    arr[{i}].buffer = GluSlice(ptr=ctypes.cast(_c{i}, ctypes.c_void_p).value, len=len(_b{i}))")
                keep_vars.append(f"_c{i}")
            else:
                lines.append(f"    raise TypeError(")
                lines.append(f"        f{err_prefix(module_str, fn_name, sig_repr, i)}")
                lines.append(f"        f'unsupported expected tag {tag}'")
                lines.append(f"    )")

        lines.append(f"    result = glucore_call(module_b, fn_b, arr, {nparams})")
        if keep_vars:
            lines.append(f"    if False: _ = ({', '.join(keep_vars)},)")
        lines.append(f"    if result.status != {GluStatus.OK}:")
        lines.append(f"        msg = (ctypes.string_at(result.message).decode()")
        lines.append(f"               if result.message else '(no message)')")
        lines.append(f"        raise RuntimeError(")
        lines.append(f"            f'glucore call failed: status={{result.status}} msg={{msg}}'")
        lines.append(f"        )")
        lines.append(f"    return unpack_return(result.value, return_tag, free_buf)")

        src = "\n".join(lines)
        namespace = {
            "reusable_arr": (GluValue * nparams)(),
            "module_b": module_b,
            "fn_b": fn_b,
            "glucore_call": glucore_call,
            "free_buf": free_buf,
            "return_tag": return_tag,
            "ctypes": ctypes,
            "GluValue": GluValue,
            "GluSlice": GluSlice,
            "GluStatus": GluStatus,
            "unpack_return": unpack_return,
            "isinstance": isinstance,
        }
        exec(compile(src, f"<glucore caller for {module_str}.{fn_name}>", "exec"), namespace)
        return namespace["caller"]

    def __getattr__(self, name: str):
        if name.startswith("_"):
            raise AttributeError(name)
        raise AttributeError(
            f"module '{self._module}' has no exported function '{name}'"
        )

    def functions(self) -> dict[str, Signature]:
        return dict(self._signatures)


# ---- Load functions --------------------------------------------------------

def load_core() -> GluCore:
    """Load the core library and apply topology from glucore.toml."""
    root = _project_root()
    core_path = root / "target" / "release" / _platform_lib_filename("glucore_core")
    if not core_path.exists():
        raise FileNotFoundError(f"glucore_core not built: {core_path}")
    core = GluCore(str(core_path))

    toml_path = root / "glucore.toml"
    if toml_path.exists():
        with open(toml_path, "rb") as f:
            manifest = tomllib.load(f)
        links = manifest.get("links", {})
        for caller, callees in links.items():
            for callee in callees:
                core.add_link(caller, callee)

    core.set_caller_identity("python")
    return core


def load_module(core: GluCore, module_name: str) -> GluProxy:
    root = _project_root()
    mod_path = root / "target" / "release" / _platform_lib_filename(module_name)
    if not mod_path.exists():
        raise FileNotFoundError(f"module not built: {mod_path}")
    module_info = core.register_module(str(mod_path))
    return GluProxy(core, module_name, module_info)


def load_process_module(core: GluCore, module_name: str,
                        socket_path: str, spawn_cmd: str) -> GluProxy:
    """Register a PROCESS module (separate process, IPC) and return a GluProxy."""
    rc = core.register_process_module(module_name, socket_path, spawn_cmd)
    if rc != 0:
        raise RuntimeError(
            f"glucore_register_process_module({module_name!r}) failed with rc={rc}"
        )
    return GluProxy(core, module_name, module_info=None)
