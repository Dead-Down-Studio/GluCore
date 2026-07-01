# 01 — Architecture

GluCore is a language-agnostic runtime coordination layer. Its job is to
route calls between modules written in arbitrary languages, enforce a
declared link topology, and own nothing itself.

## The one-sentence summary

> Every language talks through GluCore. GluCore owns nothing. It only routes.

## Component roles

```
┌─────────────────────────────────────────────────────────────────┐
│                          Python (host)                          │
│  main.py / scripts/*.py                                         │
│      │                                                          │
│      ▼                                                          │
│  glucore.py (adapter)                                           │
│      │  ctypes.CDLL("libglucore_core.so", RTLD_GLOBAL)          │
│      │  glucore_call(module, fn, args, argc) → GluResult        │
│      │                                                          │
└──────┼──────────────────────────────────────────────────────────┘
       │
       ▼
┌──────────────────────────────────────────────────────────────────┐
│                      libglucore_core.so (Rust)                   │
│                                                                  │
│  ┌────────────┐  ┌────────────┐  ┌──────────────┐  ┌──────────┐  │
│  │ glucore_   │  │ dispatch() │  │ link check   │  │ REGISTRY │  │
│  │ call()     │→ │            │→ │ (caller,     │→ │ (modules │  │
│  └────────────┘  └────────────┘  │  callee)     │  │  + fns)  │  │
│                                  └──────────────┘  └──────────┘  │
│                                          │                       │
│                       ┌──────────────────┴────────────┐          │
│                       ▼                               ▼          │
│              ARTIFACT path                    PROCESS path       │
│              (dlopen'd .so)                   (Unix socket)      │
└──────────────────────┬───────────────────────────────────┬───────┘
                       │                                   │
            ┌──────────┴─────────┐                ┌────────┴────────┐
            ▼                    ▼                ▼                 ▼
    ┌──────────────┐    ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
    │ libphysics.so│    │libcpp_engine │  │ Java JVM     │  │ (any future  │
    │   (Rust)     │    │   .so (C++)  │  │(java_renderer│  │  IPC module) │
    └──────────────┘    └──────────────┘  └──────────────┘  └──────────────┘
```

## Design principles

1. **The core owns no application state.** It holds a REGISTRY of module
   export tables, a LINKS table of declared caller→callee pairs, and a
   CALLER_IDENTITY for the current call. That's it. No business logic.

2. **The core is the ONLY router.** Every cross-module call goes through
   `glucore_call` → `dispatch` → link check → transport. Modules never
   call each other directly, even when they're in the same process. This
   is what makes the link enforcement real (rather than advisory).

3. **The core is language-agnostic.** It speaks C ABI. Every language
   that can call C can be an adapter. The core doesn't know whether a
   module is Rust, C++, or Java — it only knows the module's export
   table (a `GluModule` struct) and the wrapper function pointers in it.

4. **The core is transport-agnostic.** ARTIFACT modules (dlopen'd .so)
   are called via function pointer. PROCESS modules (separate process)
   are called via IPC. The dispatch path is the same up to the wrapper
   call — the wrapper either runs the function directly (ARTIFACT) or
   sends a CALL message over a socket (PROCESS).

5. **The core never trusts the caller.** The link check uses the CURRENT
   caller identity (set by `glucore_set_caller_identity`), not the
   caller identity the caller claims to have. Even a Rust module calling
   out has to set its own identity first — the core doesn't infer it.

## Module kinds

| Kind | Examples | Transport | Lifetime |
|---|---|---|---|
| ARTIFACT | `physics` (Rust), `cpp_engine` (C++) | Direct function pointer | Same process as core |
| PROCESS | `java_renderer` (Java) | Unix domain socket | Separate process, spawned by core |

A module declares its kind implicitly by how it's loaded:
- `glucore.load_module(core, name)` → dlopen's `lib{name}.so`, calls
  `glucore_module_info()` to get the export table. ARTIFACT.
- `glucore.load_process_module(core, name, socket, cmd)` → spawns the
  process via `sh -c cmd`, connects to the socket, reads the registration
  message. PROCESS.

Both kinds end up in the same REGISTRY. Dispatch doesn't care which kind
a module is until the last moment — it checks `ipc_socket_for_module()`
to decide whether to set the IPC thread-local context before calling the
wrapper.

## Call flow (single call, ARTIFACT callee)

```
Python:  physics.calculate_force(10.0, 9.8)
   │
   ▼
glucore.py: GluProxy.calculate_force (inlined caller)
   │  pack args into GluValue[]
   │  call lib.glucore_call("physics", "calculate_force", args, 2)
   ▼
libglucore_core.so: glucore_call()
   │  → dispatch("physics", "calculate_force", args, 2)
   │     → current_caller_bytes() = "python"
   │     → is_link_allowed_bytes("python", "physics") = true
   │     → find "physics" in REGISTRY
   │     → find "calculate_force" in physics's entries
   │     → call entry.wrapper(args, argc)
   ▼
libphysics.so: __wrapper_calculate_force()
   │  → catch_unwind { calculate_force(args[0].float, args[1].float) }
   │  → pack return as GluValue { float: 98.0 }
   │  → return GluResult { Ok, value, null }
   ▼
libglucore_core.so: dispatch returns GluResult
   ▼
glucore.py: unpack_return(result.value, "Float", free_buf)
   │  return 98.0
   ▼
Python: result = 98.0
```

## Call flow (nested, PROCESS callee with callback)

```
Python:  cpp.accelerate_via_render(5.0)
   │
   ▼
libcpp_engine.so: accelerate_via_render(5.0)
   │  glucore_set_caller_identity("cpp_engine")
   │  glucore_call("java_renderer", "scale", [5.0, 2.0])
   ▼
libglucore_core.so: dispatch
   │  caller="cpp_engine", callee="java_renderer", link OK
   │  → IPC wrapper: encode CALL, send over socket
   ▼
Java JVM: dispatchCall("scale", [5.0, 2.0])
   │  scale(5.0, 2.0) = 10.0
   │  send RESULT(10.0)
   ▼
libglucore_core.so: decode RESULT, return GluResult { 10.0 }
   ▼
libcpp_engine.so: scaled = 10.0; return 5.0 + 10.0 = 15.0
   ▼
Python: result = 15.0
```

For a Task 10 callback (Java calls outward mid-handling):

```
Python:  java.scale_via_physics(10.0, 9.8)
   │
   ▼
libglucore_core.so: dispatch
   │  caller="python", callee="java_renderer", link OK
   │  → IPC wrapper: encode CALL, send over socket
   │  → wait for response...
   ▼
Java JVM: dispatchCall("scale_via_physics", [10.0, 9.8])
   │  scale_via_physics(10.0, 9.8)
   │     │
   │     ▼  (Java sends CALLBACK_CALL)
   │  send CALLBACK_CALL("physics", "calculate_force", [10.0, 9.8])
   │     │
   │     ▼  (Rust receives CALLBACK_CALL while waiting for outer RESULT)
   │  libglucore_core.so: decode CALLBACK_CALL
   │     → save prev caller ("python"), set caller="java_renderer"
   │     → dispatch("physics", "calculate_force", [10.0, 9.8])
   │     → physics returns 98.0
   │     → restore caller ("python")
   │     → send CALLBACK_RESULT(98.0)
   │     │
   │     ▼  (Java receives CALLBACK_RESULT)
   │  Java: scale_via_physics returns 98.0
   │  send RESULT(98.0)  ← the outer CALL's RESULT, finally
   ▼
libglucore_core.so: decode RESULT, return GluResult { 98.0 }
   ▼
Python: result = 98.0
```

## What GluCore does NOT do

- It does NOT manage object lifetimes across languages. GluHandle exists
  in the spec (see `glucore/core/include/glucore.h` in the new project
  skeleton) but is not implemented in v2. The current model is "copy,
  don't borrow" — see Constraint 4 in the Phase 1 handoff.

- It does NOT support concurrent IPC calls on the same connection. The
  Task 10 callback protocol is strictly synchronous and single-threaded.
  Multiple in-flight nested calls would require correlation IDs and
  queuing, which is out of scope.

- It does NOT do schema validation across modules. Type signatures are
  declared per-function and validated pre-call on the Python side, but
  there's no project-wide schema check. The new project skeleton
  (`glucore/schemas/`) reserves space for this.

- It does NOT support Windows IPC. Unix domain sockets work on Linux and
  macOS. Windows would need named pipes — documented as a gap in
  `ipc_transport.rs` and `KNOWN_FOOTGUNS.md`.

## When to use GluCore vs. just using FFI directly

Use GluCore when:
- You have 3+ languages that need to call each other.
- You need to enforce a topology (module A may call B but not C).
- You want to swap a module's implementation language without changing
  the callers.
- You need cross-language panic/exception safety at a boundary.

Don't use GluCore when:
- You have 2 languages and a fixed call direction (just use FFI).
- You need maximum performance (GluCore adds ~3 µs per call over raw
  ctypes — see `doc/07-performance.md`).
- You don't need topology enforcement.
