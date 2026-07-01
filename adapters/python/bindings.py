"""bindings.py — ctypes struct definitions and GluCore class.

Mirrors the C ABI structs defined in core/src/types.rs. Layout MUST match
exactly — see doc/02-type-system.md for the authoritative reference.
"""

from __future__ import annotations

import ctypes
import os


# ---- C structs mirrored from core/src/types.rs ----------------------------

class GluStatus(ctypes.c_int):
    OK = 0
    RUNTIME = 1
    INVALID_ARGS = 2
    NOT_FOUND = 3
    LINK_DENIED = 4


class GluTypeTag(ctypes.c_int):
    INT = 0
    FLOAT = 1
    STRING = 2
    BUFFER = 3
    HANDLE = 4
    VOID = 5

    @classmethod
    def name(cls, value: int) -> str:
        return {
            cls.INT: "Int",
            cls.FLOAT: "Float",
            cls.STRING: "String",
            cls.BUFFER: "Buffer",
            cls.HANDLE: "Handle",
            cls.VOID: "Void",
        }.get(value, f"<unknown:{value}>")


class GluSlice(ctypes.Structure):
    """Mirror of glucore_core::GluSlice — pointer + length pair.

    `ptr` is typed as c_void_p (not POINTER(c_ubyte)) so we can assign
    an integer address returned by ctypes.cast(..., c_void_p).value
    directly. See doc/07-performance.md for why this matters.
    """
    _fields_ = [
        ("ptr", ctypes.c_void_p),
        ("len", ctypes.c_size_t),
    ]


class GluValue(ctypes.Union):
    """Mirror of glucore_core::GluValue (union)."""
    _fields_ = [
        ("float", ctypes.c_double),
        ("int", ctypes.c_longlong),
        ("string", GluSlice),
        ("buffer", GluSlice),
    ]


class GluResult(ctypes.Structure):
    _fields_ = [
        ("status", ctypes.c_int),
        ("value", GluValue),
        ("message", ctypes.c_char_p),
    ]


class GluSignatureFFI(ctypes.Structure):
    """Mirror of glucore_core::GluSignatureFFI."""
    _fields_ = [
        ("param_types", ctypes.POINTER(ctypes.c_int)),
        ("param_count", ctypes.c_size_t),
        ("return_type", ctypes.c_int),
    ]


GluWrapper = ctypes.CFUNCTYPE(GluResult, ctypes.POINTER(GluValue), ctypes.c_size_t)


class GluExportEntry(ctypes.Structure):
    _fields_ = [
        ("name", ctypes.c_char_p),
        ("wrapper", GluWrapper),
        ("signature", GluSignatureFFI),
    ]


class GluModule(ctypes.Structure):
    _fields_ = [
        ("name", ctypes.c_char_p),
        ("entries", ctypes.POINTER(GluExportEntry)),
        ("count", ctypes.c_size_t),
    ]


# ---- Core adapter ----------------------------------------------------------

class GluCore:
    def __init__(self, core_path: str):
        # Load with RTLD_GLOBAL so modules loaded later (libcpp_engine.so,
        # which has a NEEDED entry for libglucore_core.so) reuse THIS
        # instance rather than triggering a second dlopen. See doc/09-api-reference.md
        # and KNOWN_FOOTGUNS.md #1.
        self._lib = ctypes.CDLL(core_path, mode=os.RTLD_GLOBAL)
        self._lib.glucore_call.argtypes = [
            ctypes.c_char_p, ctypes.c_char_p,
            ctypes.POINTER(GluValue), ctypes.c_size_t,
        ]
        self._lib.glucore_call.restype = GluResult
        self._lib.glucore_register_module.argtypes = [GluModule]
        self._lib.glucore_register_module.restype = None
        self._lib.glucore_get_signature.argtypes = [
            ctypes.c_char_p, ctypes.c_char_p,
        ]
        self._lib.glucore_get_signature.restype = GluSignatureFFI
        self._lib.glucore_set_caller_identity.argtypes = [ctypes.c_char_p]
        self._lib.glucore_set_caller_identity.restype = None
        self._lib.glucore_add_link.argtypes = [
            ctypes.c_char_p, ctypes.c_char_p,
        ]
        self._lib.glucore_add_link.restype = None
        self._lib.glucore_free_buffer.argtypes = [
            ctypes.c_void_p, ctypes.c_size_t,
        ]
        self._lib.glucore_free_buffer.restype = None
        # Bind the raw ctypes function pointer directly for the hot path.
        self._glucore_call = self._lib.glucore_call
        self._glucore_free_buffer = self._lib.glucore_free_buffer
        self._null_ptr = ctypes.POINTER(GluValue)()

    def free_buffer(self, ptr: int, length: int) -> None:
        self._glucore_free_buffer(ptr, length)

    def register_module(self, module_lib_path: str) -> GluModule:
        mod = ctypes.CDLL(module_lib_path)
        info_fn = mod.glucore_module_info
        info_fn.argtypes = []
        info_fn.restype = GluModule
        module_info: GluModule = info_fn()
        self._lib.glucore_register_module(module_info)
        return module_info

    def register_process_module(self, module_name: str,
                                 socket_path: str, spawn_cmd: str) -> int:
        if not hasattr(self, "_glucore_register_process_module"):
            self._lib.glucore_register_process_module.argtypes = [
                ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p,
            ]
            self._lib.glucore_register_process_module.restype = ctypes.c_int
            self._glucore_register_process_module = (
                self._lib.glucore_register_process_module
            )
        return self._glucore_register_process_module(
            module_name.encode(), socket_path.encode(), spawn_cmd.encode()
        )

    def get_signature(self, module: str, function: str):
        from glu_types import Signature
        ffi: GluSignatureFFI = self._lib.glucore_get_signature(
            module.encode(), function.encode()
        )
        return Signature.from_ffi(ffi)

    def set_caller_identity(self, name: str) -> None:
        self._lib.glucore_set_caller_identity(name.encode())

    def add_link(self, caller: str, callee: str) -> None:
        self._lib.glucore_add_link(caller.encode(), callee.encode())

    def call(self, module: str, function: str, args: list) -> GluResult:
        argc = len(args)
        arr = (GluValue * argc)(*[args[i] for i in range(argc)]) if argc else None
        ptr = ctypes.cast(arr, ctypes.POINTER(GluValue)) if arr else None
        return self._glucore_call(
            module.encode(), function.encode(), ptr, argc
        )
