// glucore_core — the Rust core of GluCore v2.
//
// Language-agnostic runtime coordination layer. Every language talks
// through this core. The core owns nothing — it only routes.
//
// Module structure (mirrors the C-based project skeleton):
//   types.rs     — GluTypeTag, GluValue, GluSlice, GluResult, GluStatus, etc.
//   errors.rs    — GluResult construction helpers, glucore_free_buffer
//   handles.rs   — Module registry, signature introspection
//   router.rs    — Dispatch, link enforcement, caller identity
//   runtime.rs   — IPC execution, process module registration, transport
//   fctp.rs      — Wire protocol (CALL/RESULT/CALLBACK_CALL/CALLBACK_RESULT)
//   transport.rs — IpcStream / IpcListener abstraction (Task 11 portability)
//   trace.rs     — Call chain tracking (stub — reserved for future use)

pub mod types;
pub mod errors;
pub mod handles;
pub mod router;
pub mod fctp;
pub mod runtime;
pub mod trace;

// Re-export the transport abstraction module (Task 11).
// On non-Unix platforms this compiles as a stub.
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod transport;

// Re-export the most-used types at the crate root so adapters can do
// `use glucore_core::{GluValue, GluResult, ...}` without deep paths.
pub use types::*;
pub use errors::glucore_free_buffer;
pub use router::{glucore_call, glucore_set_caller_identity, glucore_add_link, call_as, CallerGuard};
pub use handles::{glucore_register_module, glucore_get_signature, glucore_get_module_export_count, glucore_get_module_export_name};
