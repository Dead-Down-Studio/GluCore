/*
 * glucore.h — THE public contract for GluCore v2.
 *
 * Every adapter (Python, Rust, C++, Java) and every user sees this file.
 * Nobody should need to read any other file to use GluCore.
 *
 * This header is the C ABI mirror of the Rust types in core/src/types.rs.
 * Layout MUST match exactly. The C++ adapter (adapters/cpp/include/
 * glucore_export.hpp) has a static_assert for this; the Python adapter
 * (adapters/python/bindings.py) verifies via round-trip tests.
 *
 * For the full API reference, see doc/09-api-reference.md.
 */
#ifndef GLUCORE_H
#define GLUCORE_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Type tags ---- */

typedef enum {
    GLUC_INT    = 0,
    GLUC_FLOAT  = 1,
    GLUC_STRING = 2,
    GLUC_BUFFER = 3,
    GLUC_HANDLE = 4,  /* reserved for Phase 3 */
    GLUC_VOID   = 5,  /* return-only */
} GluTypeTag;

/* ---- Status codes ---- */

typedef enum {
    GLUC_OK              = 0,
    GLUC_ERR_RUNTIME     = 1,
    GLUC_ERR_INVALID_ARGS = 2,
    GLUC_ERR_NOT_FOUND   = 3,
    GLUC_ERR_LINK_DENIED = 4,
} GluStatus;

/* ---- Core value types ---- */

/* Pointer + length pair for String/Buffer. 16 bytes. */
typedef struct GluSlice {
    const uint8_t* ptr;
    size_t         len;
} GluSlice;

/* Tagged union. 16 bytes. The active member is determined by the
 * signature's type tag, NOT embedded in the value. */
typedef union GluValue {
    double    float_v;   /* GluTypeTag::Float */
    int64_t   int_v;     /* GluTypeTag::Int */
    GluSlice  string;    /* GluTypeTag::String */
    GluSlice  buffer;    /* GluTypeTag::Buffer */
} GluValue;

/* Result of a call. 32 bytes: status(4) + pad(4) + value(16) + message(8). */
typedef struct GluResult {
    GluStatus   status;     /* @0 */
    /* 4 bytes padding to align value to 8 */
    GluValue    value;      /* @8 */
    const char* message;    /* @24, NUL-terminated error msg (NULL if Ok) */
} GluResult;

/* ---- Signature metadata ---- */

typedef struct GluSignatureFFI {
    const GluTypeTag* param_types;
    size_t            param_count;
    GluTypeTag        return_type;
} GluSignatureFFI;

/* ---- Module registration ---- */

typedef GluResult (*GluWrapper)(const GluValue* args, size_t argc);

typedef struct GluExportEntry {
    const char*     name;       /* function name (NUL-terminated) */
    GluWrapper      wrapper;    /* function pointer */
    GluSignatureFFI signature;  /* param types + return type */
} GluExportEntry;

typedef struct GluModule {
    const char*           name;     /* module name (NUL-terminated) */
    const GluExportEntry* entries;  /* export table */
    size_t                count;    /* export count */
} GluModule;

/* ---- Core API: call / dispatch ---- */

/* Make a cross-module call. The caller identity must be set via
 * glucore_set_caller_identity before calling this. */
GluResult glucore_call(
    const char* module,
    const char* function,
    const GluValue* args,
    size_t argc
);

/* Set the current caller identity. Must be called before glucore_call. */
void glucore_set_caller_identity(const char* name);

/* ---- Topology ---- */

/* Add a (caller -> callee) link. Called once per declared link at startup. */
void glucore_add_link(const char* caller, const char* callee);

/* ---- Module registration ---- */

/* Register an ARTIFACT module (dlopen'd .so). */
void glucore_register_module(GluModule module);

/* Register a PROCESS module (separate process, IPC). Unix only.
 * Returns 0 on success, negative on failure. */
int32_t glucore_register_process_module(
    const char* module_name,
    const char* socket_path,
    const char* spawn_cmd
);

/* ---- Signature introspection ---- */

GluSignatureFFI glucore_get_signature(const char* module, const char* function);
size_t glucore_get_module_export_count(const char* module);
const char* glucore_get_module_export_name(const char* module, size_t index);

/* ---- Memory management ---- */

/* Free a Rust-allocated String/Buffer return value. */
void glucore_free_buffer(void* ptr, size_t len);

/* ---- Debug / introspection (Task 9) ---- */

uint64_t glucore_shared_state_checksum(void);
size_t   glucore_link_table_size(void);
uint64_t glucore_links_address(void);

#ifdef __cplusplus
}
#endif

#endif /* GLUCORE_H */
