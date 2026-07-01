# GluCore v2

> A language-agnostic runtime coordination layer.
> Every language talks through GluCore. GluCore owns nothing. It only routes.

GluCore lets modules written in different languages (Python, Rust, C++,
Java) call each other's functions through a central Rust core that
enforces a declared link topology, validates types before the FFI
boundary, and routes calls either directly (ARTIFACT modules) or over
IPC (PROCESS modules).

## Project structure

```
glucore_v2_proj/
├── core/                   Rust core crate (libglucore_core.so)
│   ├── include/glucore.h   Public C ABI contract
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs          Module declarations + re-exports
│       ├── types.rs        GluTypeTag, GluValue, GluSlice, GluResult, etc.
│       ├── errors.rs       GluResult helpers, glucore_free_buffer
│       ├── handles.rs      Module registry, signature introspection
│       ├── router.rs       Dispatch, link enforcement, caller identity
│       ├── runtime.rs      IPC execution, process module registration
│       ├── fctp.rs         Wire protocol (CALL/RESULT/CALLBACK_*)
│       ├── transport.rs    IpcStream / IpcListener abstraction
│       └── trace.rs        Call chain tracking (stub)
│
├── adapters/
│   ├── python/             Python adapter (ctypes bindings + GluProxy)
│   │   ├── __init__.py     Re-exports everything
│   │   ├── bindings.py     ctypes struct definitions, GluCore class
│   │   ├── glu_types.py    pack_arg, unpack_return, Signature
│   │   ├── adapter.py      GluProxy, load_core, load_module
│   │   └── gc_pin.py       GC pinning (stub — no GluHandle in v2)
│   ├── rust/               Rust proc-macro (#[glucore::export])
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── cpp/                C++ adapter header
│   │   └── include/glucore_export.hpp
│   └── java/               (reserved — Java adapter is fused with module)
│
├── examples/
│   ├── physics/            Example Rust ARTIFACT module
│   ├── cpp_engine/         Example C++ ARTIFACT module
│   └── java_renderer/      Example Java PROCESS module
│
├── tests/
│   ├── unit/               Single-module tests (task1, task3, task4, task6a)
│   ├── integration/        Multi-module same-process tests (task5, task7, part0a/b)
│   └── interop/            Cross-language IPC tests (part0c, task9, task10)
│
├── scripts/                Benchmarks
├── docs/                   Wiki (11 documents)
├── parser/                 Stub (v2 uses TOML, not .gluc)
├── env/                    Stub (v2 has no env management)
├── cli/                    Stub (v2 uses Python scripts)
├── schemas/                Stub (reserved for Phase 3)
│
├── glucore.py              Shim that re-exports from adapters/python/
├── glucore.toml            Topology manifest (modules + links)
├── main.py                 Smoke test
├── build.sh                Universal build script
├── Cargo.toml              Rust workspace
├── KNOWN_FOOTGUNS.md       8 documented footguns
├── PHASE2_ADDENDUM_REPORT.md  Full stage-by-stage report
└── shell.nix               Nix environment
```

## Quick start

```bash
./build.sh                    # Builds everything + runs smoke test
python3 main.py               # Basic smoke test
python3 scripts/bench_final.py  # Performance benchmark
python3 tests/interop/task9_dod.py   # Three-language mesh test
```

## Key features

- **Language-agnostic routing:** Python, Rust, C++, and Java modules all
  go through the same Rust core via C ABI.
- **Link enforcement:** The core checks caller→callee pairs against a
  declared topology. Undeclared calls are denied before any work is done.
- **Type safety:** Pre-call type validation in Python catches type
  mismatches before the FFI boundary.
- **IPC transport:** PROCESS modules (Java) communicate over Unix domain
  sockets with a binary wire protocol. Supports nested callbacks (Task 10).
- **Performance:** 4.7x overhead vs raw ctypes (down from 37x after
  optimization — see docs/07-performance.md).

## Documentation

See `docs/README.md` for the full wiki index. Key documents:
- `docs/01-architecture.md` — High-level architecture
- `docs/04-adapter-authoring.md` — How to write a new language adapter
- `docs/03-wire-protocol.md` — IPC wire protocol spec (incl. Task 10 callbacks)
- `docs/09-api-reference.md` — Every public C ABI function

## Requirements

- Rust (cargo + rustc) — https://rustup.rs/
- CMake >= 3.15
- g++ or clang++ (C++17)
- JDK >= 16 (for Java PROCESS module tests)
- Python >= 3.11 (for tomllib)
