# GluCore v2 — Documentation Index

This directory contains the comprehensive wiki for GluCore v2. Each document
is focused on one topic so you can read what you need without scrolling
through unrelated material.

## Start here

| Document | What it covers |
| --- | --- |
| [01-architecture.md](01-architecture.md) | High-level architecture, component roles, design principles |
| [02-type-system.md](02-type-system.md) | GluTypeTag, GluValue, GluResult, GluSlice — the universal IR |
| [03-wire-protocol.md](03-wire-protocol.md) | CALL / RESULT / CALLBACK_CALL / CALLBACK_RESULT message formats (incl. Task 10) |
| [04-adapter-authoring.md](04-adapter-authoring.md) | How to write a new language adapter (Python / Rust / C++ / Java / next language) |
| [05-link-enforcement.md](05-link-enforcement.md) | Topology, link graph, caller identity, denied-link behavior |
| [06-ipc-transport.md](06-ipc-transport.md) | Unix domain socket transport, byte-counter evidence, portability |
| [07-performance.md](07-performance.md) | Benchmarks, profiling, optimization techniques applied |
| [08-known-footguns.md](08-known-footguns.md) | The 8 footguns documented in KNOWN_FOOTGUNS.md, with verification templates |
| [09-api-reference.md](09-api-reference.md) | Every public C ABI function exposed by libglucore_core |
| [10-examples.md](10-examples.md) | Walk-throughs of every example and DoD test in scripts/ (incl. Task 10) |

## Quick lookup

- "How do I add a new language?" → [04-adapter-authoring.md](04-adapter-authoring.md)
- "What's the wire format for an IPC call?" → [03-wire-protocol.md](03-wire-protocol.md)
- "Why was my denied link not actually denied?" → [05-link-enforcement.md](05-link-enforcement.md)
- "How do I verify the Rust core never sends bytes for a denied call?" → [06-ipc-transport.md](06-ipc-transport.md)
- "Why is GluCore slower than raw ctypes?" → [07-performance.md](07-performance.md)
- "What bugs have already bitten this project?" → [08-known-footguns.md](08-known-footguns.md)
- "What functions does libglucore_core.so export?" → [09-api-reference.md](09-api-reference.md)

## Project layout

```
glucore_v2_proj/
├── glucore_core/         Rust core (libglucore_core.so)
│   └── src/
│       ├── lib.rs        All core code: types, dispatch, IPC, link check
│       └── ipc_transport.rs  IpcStream / IpcListener abstraction (Task 11)
├── glucore_macro/        Rust proc-macro for #[export] (auto-registration)
├── physics/              Rust ARTIFACT module (calculate_force, greet, etc.)
├── cpp_engine/           C++ ARTIFACT module (accelerate, accelerate_via_render)
│   ├── include/glucore_export.hpp  C++ adapter macros (GLUCORE_EXPORT)
│   └── CMakeLists.txt   Links against libglucore_core.so
├── java_renderer/        Java PROCESS module (scale, scale_via_physics)
│   └── src/main/java/Renderer.java
├── glucore.py            Python adapter (ctypes bindings, GluProxy, GluCore)
├── glucore.toml          Topology manifest (modules + links)
├── main.py               Smoke test
├── scripts/              All DoD test scripts + benchmarks
├── build.sh              Universal build script (Rust + C++ + Java + smoke)
├── KNOWN_FOOTGUNS.md     8 documented footguns (referenced by doc/08)
├── PHASE2_ADDENDUM_REPORT.md  Full stage-by-stage report
└── doc/                  This wiki
```

## Reporting bugs

If you find a new footgun, add it to `KNOWN_FOOTGUNS.md` and to
`doc/08-known-footguns.md` with the same failure-mode / rule / verification-
template structure. The point of the footguns file is that each entry is
a permanent constraint on future work — not just a historical note.
