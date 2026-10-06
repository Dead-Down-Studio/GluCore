# 11 — Production Readiness Contract

This document defines the production compatibility contract for GluCore:

- C-ABI versioning policy
- Wire-protocol versioning policy
- Adapter and PROCESS-module compatibility rules
- Production support matrix

## Contract version identifiers

GluCore now exposes explicit runtime contract versions through C ABI:

- `glucore_abi_version_{major,minor,patch}`
- `glucore_wire_version_{major,minor,patch}`

Current baseline:

- ABI: `1.0.0`
- Wire protocol: `1.0.0`

Adapters and PROCESS modules SHOULD check these versions during startup
and fail fast on unsupported major versions.

## Compatibility policy

### C ABI compatibility

- **Major**: breaking changes. Older adapters are not guaranteed to work.
- **Minor**: additive, backward-compatible changes only.
- **Patch**: bug fixes and clarifications, no ABI layout/signature break.

ABI compatibility target:

- A `1.x.y` adapter must work with any `1.a.b` runtime where required APIs
  are present.
- Runtime can reject adapter/runtime pairs when major versions differ.

### Wire protocol compatibility

- **Major**: framing or semantic breaks in CALL/RESULT/CALLBACK messages.
- **Minor**: additive fields/messages with backward-safe decoding rules.
- **Patch**: bug fixes without wire-format break.

Wire compatibility target:

- PROCESS modules must match wire major version.
- Minor/patch mismatches are supported when decoding remains backward-safe.

## Concurrency guarantees

GluCore runtime state guarantees:

- Registry and topology link graph are synchronized shared state.
- Caller identity is thread-local and stack-based for nested call safety.
- IPC session metadata is synchronized shared state.

No adapter/module should rely on unsynchronized global mutable state.

## Production support matrix

| Area | Supported | Notes |
|---|---|---|
| OS (core + ARTIFACT) | Linux, macOS | Primary supported platforms |
| OS (PROCESS IPC) | Linux, macOS | Unix domain sockets |
| Windows PROCESS IPC | Not yet GA | Named-pipe path planned; unsupported today |
| Rust toolchain | Stable Rust (workspace default) | Build via `cargo build --release` |
| C++ compiler | g++ / clang++ (C++17) | Built through CMake |
| Python adapter | Python 3.11+ | Uses `tomllib` |
| Java PROCESS module | JDK 16+ | Current interop baseline |
| Module kinds | ARTIFACT, PROCESS | Same routing + policy enforcement |

## Release gates (production path)

Before GA, releases must pass:

1. Compatibility gates (ABI/wire checks)
2. Reliability gates (regression + stress suites)
3. Security gates (policy + validation checks)
4. Performance gates (benchmark thresholds)
