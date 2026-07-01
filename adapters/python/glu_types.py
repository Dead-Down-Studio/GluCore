"""types.py — Python ↔ GluType conversion, signature handling.

Provides:
  - Signature: Python-side view of a function's signature
  - PY_TYPE_TO_TAG: maps Python types to GluTypeTag names
  - pack_arg / unpack_return: convert between Python values and GluValue
  - _make_packer / err_prefix: helpers for the inlined caller generation
"""

from __future__ import annotations

import ctypes
from typing import Any

from bindings import (
    GluStatus, GluTypeTag, GluSlice, GluValue, GluResult,
    GluSignatureFFI, GluWrapper, GluExportEntry, GluModule, GluCore,
)


# ---- Signature registry (Python side) -------------------------------------

class Signature:
    """Python-side view of a function's signature, with type validation."""

    def __init__(self, param_tags: list[str], return_tag: str):
        self.param_tags = param_tags  # e.g. ["Float", "Float"]
        self.return_tag = return_tag  # e.g. "Float"

    def __repr__(self) -> str:
        params = ", ".join(self.param_tags) if self.param_tags else ""
        return f"({params}) -> {self.return_tag}"

    @classmethod
    def from_ffi(cls, ffi: GluSignatureFFI) -> "Signature":
        param_tags = []
        for i in range(ffi.param_count):
            tag_int = ffi.param_types[i]
            param_tags.append(GluTypeTag.name(tag_int))
        return_tag = GluTypeTag.name(ffi.return_type)
        return cls(param_tags, return_tag)


# ---- Python-side packing helpers ------------------------------------------

# Map Python types to the GluTypeTag names they satisfy.
PY_TYPE_TO_TAG = {
    bool: "Int",
    int: "Int",
    float: "Float",
    str: "String",
    bytes: "Buffer",
    bytearray: "Buffer",
}


def pack_arg(value: Any) -> tuple[GluValue, Any]:
    """Pack a Python value into a GluValue for FFI.

    Returns (GluValue, keep_alive_obj). The caller MUST hold keep_alive_obj
    for the duration of the FFI call.
    """
    v = GluValue()
    keep = None
    if isinstance(value, bool):
        v.int = int(value)
    elif isinstance(value, float):
        v.float = value
    elif isinstance(value, int):
        v.int = value
    elif isinstance(value, str):
        b = value.encode("utf-8")
        cstr = ctypes.c_char_p(b)
        v.string = GluSlice(
            ptr=ctypes.cast(cstr, ctypes.c_void_p).value,
            len=len(b),
        )
        keep = cstr
    elif isinstance(value, (bytes, bytearray)):
        b = bytes(value)
        cstr = ctypes.c_char_p(b)
        v.buffer = GluSlice(
            ptr=ctypes.cast(cstr, ctypes.c_void_p).value,
            len=len(b),
        )
        keep = cstr
    else:
        raise TypeError(f"unsupported python arg type: {type(value).__name__}")
    return v, keep


def unpack_return(result_value: GluValue, return_tag: str, freer) -> Any:
    """Convert a GluValue returned from Rust into a Python value.

    `freer` is a callable(ptr, len) that releases the Rust allocation backing
    a String/Buffer return.
    """
    if return_tag == "Float":
        return result_value.float
    if return_tag == "Int":
        return result_value.int
    if return_tag == "String":
        s = result_value.string
        if s.len == 0:
            return ""
        py = ctypes.string_at(s.ptr, s.len).decode("utf-8")
        freer(s.ptr, s.len)
        return py
    if return_tag == "Buffer":
        s = result_value.buffer
        if s.len == 0:
            return b""
        py = ctypes.string_at(s.ptr, s.len)
        freer(s.ptr, s.len)
        return py
    if return_tag == "Void":
        return None
    raise TypeError(f"unknown return tag: {return_tag}")


# ---- Helpers for inlined caller generation --------------------------------

def _err_prefix(module_str: str, fn_name: str, sig_repr: str, i: int) -> str:
    return f"'{module_str}.{fn_name}{sig_repr} arg {i}: '"


def err_prefix(module_str: str, fn_name: str, sig_repr: str, i: int) -> str:
    return _err_prefix(module_str, fn_name, sig_repr, i)


def _make_packer(i: int, expected_tag: str, module_str: str, fn_name: str, sig_repr: str):
    """[DEPRECATED] Return a per-arg packer closure. Kept for backwards compat."""
    if expected_tag == "Float":
        def pack(v):
            if type(v) is not float:
                if isinstance(v, bool) or isinstance(v, int):
                    raise TypeError(
                        f"{module_str}.{fn_name}{sig_repr} arg {i}: "
                        f"expected Float, got Int (python type {type(v).__name__})"
                    )
                raise TypeError(
                    f"{module_str}.{fn_name}{sig_repr} arg {i}: "
                    f"expected Float, got unsupported python type {type(v).__name__}"
                )
            gv = GluValue()
            gv.float = v
            return gv, None
        return pack
    if expected_tag == "Int":
        def pack(v):
            if type(v) is bool:
                gv = GluValue()
                gv.int = int(v)
                return gv, None
            if type(v) is int:
                gv = GluValue()
                gv.int = v
                return gv, None
            if isinstance(v, float):
                raise TypeError(
                    f"{module_str}.{fn_name}{sig_repr} arg {i}: expected Int, got Float"
                )
            raise TypeError(
                f"{module_str}.{fn_name}{sig_repr} arg {i}: "
                f"expected Int, got unsupported python type {type(v).__name__}"
            )
        return pack
    if expected_tag == "String":
        def pack(v):
            if type(v) is not str:
                raise TypeError(
                    f"{module_str}.{fn_name}{sig_repr} arg {i}: "
                    f"expected String, got unsupported python type {type(v).__name__}"
                )
            b = v.encode("utf-8")
            cstr = ctypes.c_char_p(b)
            gv = GluValue()
            gv.string = GluSlice(
                ptr=ctypes.cast(cstr, ctypes.c_void_p).value,
                len=len(b),
            )
            return gv, cstr
        return pack
    if expected_tag == "Buffer":
        def pack(v):
            if type(v) is bytes:
                b = v
            elif type(v) is bytearray:
                b = bytes(v)
            else:
                raise TypeError(
                    f"{module_str}.{fn_name}{sig_repr} arg {i}: "
                    f"expected Buffer, got unsupported python type {type(v).__name__}"
                )
            cstr = ctypes.c_char_p(b)
            gv = GluValue()
            gv.buffer = GluSlice(
                ptr=ctypes.cast(cstr, ctypes.c_void_p).value,
                len=len(b),
            )
            return gv, cstr
        return pack
    def pack(v):
        raise TypeError(
            f"{module_str}.{fn_name}{sig_repr} arg {i}: "
            f"unsupported expected tag {expected_tag}"
        )
    return pack
