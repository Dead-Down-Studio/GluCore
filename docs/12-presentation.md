---
marp: true
theme: default
paginate: true
size: 16:9
---

# GluCore v2

## A language-agnostic runtime for coordinating native modules

**The core idea:** every language talks through GluCore.  
GluCore owns no application state; it only routes calls.

---

## Why GluCore exists

Cross-language systems usually force a choice:

- tightly coupled FFI for speed, or
- a separate service/protocol for isolation.

GluCore aims to provide both:

- direct in-process calls for compiled modules
- process-based IPC for runtimes such as Java
- one routing and permission model for both

---

## The project in one picture

```text
 Python host
     |
     | ctypes / C ABI
     v
 Rust core: registry + router + policy
     |                         |
     | direct function call    | Unix socket + binary protocol
     v                         v
 Rust physics             Java renderer
     |
     | direct function call
     v
 C++ engine
```

The Rust core is the only component allowed to route cross-module calls.

---

## Core architecture

The runtime has four responsibilities:

1. **Registry** - loaded modules and exported functions
2. **Dispatch** - find the target function and invoke it
3. **Link enforcement** - check caller -> callee permissions
4. **Transport** - direct function pointers or IPC

It intentionally does **not** contain business logic or own application state.

---

## Two module kinds

| Module kind | Example | Transport | Runtime location |
|---|---|---|---|
| `ARTIFACT` | Rust physics, C++ engine | function pointer | same process |
| `PROCESS` | Java renderer | Unix domain socket | child process |

Both module kinds register exports and participate in the same topology.
The caller does not need a different API for the two transports.

---

## The topology is a permission graph

`glucore.toml` describes modules and allowed links:

```toml
[links]
python = ["physics", "cpp_engine", "java_renderer"]
physics = ["cpp_engine"]
cpp_engine = ["java_renderer"]
java_renderer = ["physics"]
```

Every call is checked before dispatch.  
Undeclared calls return `LinkDenied`; denied IPC calls send **zero bytes**.

---

## One universal value model

The C ABI uses a small intermediate representation:

```text
Int     i64
Float   f64
String  UTF-8 bytes
Buffer  raw bytes
Handle  reserved for a future phase
Void    return-only
```

The same layout is mirrored by Rust, Python `ctypes`, C++, and Java.
Current semantics are deliberately simple: values are copied across boundaries.

---

## Types come from native code

There is no second schema language for exported function signatures.

```rust
#[glucore::export]
pub fn calculate_force(mass: f64, accel: f64) -> f64 {
    mass * accel
}
```

The export macro generates:

- the C-ABI wrapper
- signature metadata
- type checks at the boundary

Incorrect argument counts or types fail before the FFI call.

---

## A normal call

```text
Python physics.calculate_force(10.0, 9.8)
  -> pack values using the function signature
  -> glucore_call("physics", "calculate_force", ...)
  -> identify caller: python
  -> check python -> physics
  -> locate export in the registry
  -> call the Rust wrapper
  -> return GluResult { Float: 98.0 }
```

The same dispatch path works whether the final step is a function pointer
or an IPC wrapper.

---

## IPC and nested callbacks

PROCESS modules use a synchronous, length-prefixed binary protocol:

```text
CALL            Rust -> process
RESULT          process -> Rust
CALLBACK_CALL   process -> Rust, while handling a CALL
CALLBACK_RESULT Rust -> process
```

This allows Java to call back into Rust during an inbound request.
The core still applies the normal caller identity and link checks.

---

## Verified three-language mesh

The repository demonstrates this chain in one runtime:

```text
Python -> Rust physics -> C++ engine -> Java renderer
```

Representative results:

- `physics.calculate_force(10.0, 9.8) = 98.0`
- nested Rust -> C++ call succeeds
- nested C++ -> Java IPC call succeeds
- undeclared C++ -> Rust call is denied

Java can also call Rust through `CALLBACK_CALL`.

---

## Reliability and safety guarantees

- Rust wrappers catch panics and return runtime errors.
- Return strings and buffers have an explicit free path.
- Caller identity is thread-local and restored after nested calls.
- Link checks happen before work or IPC transmission.
- Type mismatches are rejected before crossing the FFI boundary.
- ABI and wire-protocol versions are exposed for compatibility checks.

These guarantees are the reason GluCore is slower than raw FFI.

---

## Performance and tradeoffs

Measured median for the documented buffer workload:

| Path | Per call | Calls/sec |
|---|---:|---:|
| Raw `ctypes` | 0.633 us | 1,578,659 |
| GluCore | 3.002 us | 333,095 |

**Cost:** about 4.7x over raw `ctypes`.

The cost buys type validation, topology enforcement, registry dispatch,
and copy-in/copy-out ownership semantics.

---

## What GluCore is not

- Not a general object-lifetime manager yet (`Handle` is reserved).
- Not a concurrent IPC broker; nested IPC is synchronous and single-threaded.
- Not a project-wide schema validator.
- Not a Windows PROCESS runtime today.
- Not always the right choice for a two-language, performance-critical call path.

The repository documents these limits instead of hiding them.

---

## How to explore the repository

```bash
./build.sh
python3 main.py
python3 tests/interop/task9_dod.py
python3 tests/interop/task10_dod.py
python3 scripts/bench_final.py
```

Most useful files:

- `core/src/` - Rust runtime and dispatch
- `adapters/` - Python, Rust, C++, and Java integration
- `glucore.toml` - module topology
- `examples/` - representative modules
- `tests/` - unit, integration, and interop verification
- `docs/` - detailed architecture and protocol contracts

---

## The design lesson

GluCore grew out of an earlier custom-language experiment.

The key change was moving the abstraction boundary:

```text
Before: build a language that can call other languages
Now:    keep native languages and coordinate their modules
```

The native function signature is the contract.  
The manifest declares topology.  
The runtime routes, validates, and stays out of the application.

---

# Takeaway

## GluCore is a policy-aware bridge between languages

It combines:

- one C-compatible contract
- compile-time export metadata
- explicit caller-to-callee permissions
- direct and process-based transports
- measurable, documented tradeoffs

That makes polyglot composition explicit, testable, and replaceable.
