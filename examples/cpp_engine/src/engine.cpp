// engine.cpp — C++ ARTIFACT module "cpp_engine" for GluCore (Task 7, 9, 11).
//
// Defines the registry singleton that GLUCORE_EXPORT(fn) static initializers
// push into, and the single entry point the Rust core dlopens:
//
//     extern "C" GluModule glucore_module_info();
//
// That entry point is the ENTIRE contract with glucore_core — identical to a
// Rust module's. No core changes are required to load this module, which is
// the property Task 7 exists to demonstrate.
//
// Task 9 (Part 1): cpp_engine now also acts as a CALLER, not just a callee.
// It links against libglucore_core.so directly and calls glucore_call /
// glucore_set_caller_identity from C++. The `accelerate_via_render` and
// `attempt_physics_call` functions below are real C++-initiated calls into
// other modules — proving the routing is genuinely language-agnostic.

#include "glucore_export.hpp"

#include <cstdio>
#include <cstring>
#include <list>
#include <new>
#include <cstdint>

// --- Task 9: declarations for the glucore_core C ABI we link against -----
//
// These are the same symbols Python resolves via ctypes and physics resolves
// via libloading. By declaring them `extern "C"` here and linking against
// libglucore_core.so (see CMakeLists.txt), cpp_engine reaches the ONE real
// shared instance of the registry / link table / caller-identity — verified
// directly by `glucore_shared_state_checksum` (see Requirement 1 in
// task9_dod.py).
extern "C" {
    // Defined in glucore_core/src/lib.rs.
    glucore::GluResult glucore_call(
        const char* module,
        const char* function,
        const glucore::GluValue* args,
        size_t argc);
    void glucore_set_caller_identity(const char* name);
    uint64_t glucore_shared_state_checksum();
    size_t glucore_link_table_size();
    uint64_t glucore_links_address();
}

namespace glucore {

std::vector<GluExportRecord>& registry() {
    // Function-local static: constructed on first use, destroyed at exit.
    static std::vector<GluExportRecord> r;
    return r;
}

} // namespace glucore

// --- The entry point the Rust core dlopens (same contract as a Rust module) -
extern "C" glucore::GluModule glucore_module_info() {
    // Build a STABLE export table from the registry, exactly once. The table
    // and the per-entry name strings are leaked for the module's process
    // lifetime (same as the Rust physics module leaks its registration data),
    // so pointers handed back to Rust never dangle across calls.
    struct StableTable {
        // list<> is node-based: each element has a stable address even as more
        // are inserted, so the .c_str() pointers stored in `entries` never
        // dangle. (A vector<string> would invalidate them on reallocation.)
        std::list<std::string> names;
        std::vector<glucore::GluExportEntry> entries;
        glucore::GluModule module{};
    };
    static StableTable* table = []() -> StableTable* {
        auto* t = new StableTable();
        for (auto& rec : glucore::registry()) {
            t->names.emplace_back(rec.name);
            t->entries.push_back(glucore::GluExportEntry{
                t->names.back().c_str(),
                rec.wrapper,
                rec.signature,
            });
        }
        t->module.name = "cpp_engine";
        t->module.entries = t->entries.data();
        t->module.count = t->entries.size();
        return t;
    }();

    return table->module;
}

// --- Example exported functions (each type, for the DoD) -------------------
//
// Each one is registered automatically by its GLUCORE_EXPORT static
// initializer — no manual list, mirroring #[glucore::export] + inventory.

// Float round-trip + the "accelerate" function named in the Task 7 DoD.
double accelerate(double v, double dt) {
    return v + 9.8 * dt; // v0 + a*dt
}
GLUCORE_EXPORT(accelerate)

// Int round-trip.
int64_t add_ints_cpp(int64_t a, int64_t b) {
    return a + b;
}
GLUCORE_EXPORT(add_ints_cpp)

// String round-trip.
std::string greet_cpp(std::string name) {
    return "Hello from C++, " + name + "!";
}
GLUCORE_EXPORT(greet_cpp)

// Buffer round-trip.
std::vector<uint8_t> echo_bytes_cpp(std::vector<uint8_t> data) {
    return data;
}
GLUCORE_EXPORT(echo_bytes_cpp)

// --- Benchmark counterpart to the raw-ctypes step_physics() ---------------
//
// GluCore's type system has no "raw pointer / in-place mutation" type — only
// Int, Float, String, Buffer (owned vector<uint8_t>), Handle, Void. So unlike
// the raw ctypes version (which mutates two float* arrays in place, zero
// extra copies), this version receives a COPY of the packed state, mutates
// that local copy, and returns ANOTHER copy. That's not a workaround bug —
// it's the actual cost model Buffer commits to (see Constraint 4 in the
// project's handoff docs: "copy, don't borrow, for now"). The benchmark is
// partly measuring exactly this difference, not just call overhead.
//
// Layout: state is interleaved float pairs [h0, v0, h1, v1, h2, v2, ...].
std::vector<uint8_t> step_physics_glucore(std::vector<uint8_t> state,
                                           double dt, double gravity) {
    size_t count = state.size() / (2 * sizeof(float));
    float* floats = reinterpret_cast<float*>(state.data());
    for (size_t i = 0; i < count; i++) {
        float& h = floats[i * 2];
        float& v = floats[i * 2 + 1];
        v -= static_cast<float>(gravity) * static_cast<float>(dt);
        h += v * static_cast<float>(dt);
        if (h < 0.0f) {
            h = 0.0f;
            v = -v * 0.8f;
        }
    }
    return state;
}
GLUCORE_EXPORT(step_physics_glucore)

// --- Task 9, Requirement 1: state-sharing check ---------------------------
//
// Returns the same u64 the Rust core's `glucore_shared_state_checksum`
// returns. Two callers hitting the ONE real shared instance will see the
// same value; callers hitting private (rlib-duplicated) copies would
// diverge. The C++ side reads the core's symbols via normal linking; the
// Rust physics side reads them via libloading. If both report the same
// number, the "C++ has no footgun" claim is verified directly, not assumed.
int64_t cpp_engine_shared_state_checksum() {
    return static_cast<int64_t>(glucore_shared_state_checksum());
}
GLUCORE_EXPORT(cpp_engine_shared_state_checksum)

// Return the link-table size from the C++ side. Stable, simple, and
// discriminating: if Rust physics sees 4 links and C++ sees 0, C++ is
// hitting a private (rlib-duplicated) empty copy of LINKS — that's the
// footgun. If both see the same number, they share the one instance.
int64_t cpp_engine_link_table_size() {
    return static_cast<int64_t>(glucore_link_table_size());
}
GLUCORE_EXPORT(cpp_engine_link_table_size)

// Return the address of the LINKS static from the C++ side. If this
// differs from what the Rust core reports via glucore_links_address(),
// the C++ side is hitting a DIFFERENT libglucore_core.so instance —
// likely because Python loaded it with RTLD_LOCAL, so libcpp_engine.so's
// NEEDED entry loaded a second copy. The fix is for Python to use
// RTLD_GLOBAL when loading the core.
int64_t cpp_engine_links_address() {
    return static_cast<int64_t>(glucore_links_address());
}
GLUCORE_EXPORT(cpp_engine_links_address)

// --- Task 9, Requirement 2: a real C++-initiated call into another module -
//
// `accelerate_via_render(v)` is a REAL feature: it calls
// `java_renderer::scale(v, 2.0)` and adds the result to v. This proves C++
// can be a CALLER, not just a callee, going through the same router / link
// check / IPC dispatch as Python and physics.
//
// Discipline (from the handoff):
//   1. Push our own caller identity ("cpp_engine") via glucore_set_caller_identity.
//   2. Call glucore_call directly.
//   3. Pop / restore the previous caller — including on the error path.
//
// We restore to "python" as the outer caller (matching physics's convention)
// since the core has no getter for the previous caller. Single-threaded demo
// makes this safe; multi-threaded would need a save/restore stack.
double accelerate_via_render(double v) {
    // Pack args: scale(v, 2.0)
    glucore::GluValue args[2];
    args[0].float_v = v;
    args[1].float_v = 2.0;

    // Save (assume "python" — single-threaded demo convention) and set our
    // own identity. We do this manually rather than via a RAII guard because
    // the C++ side doesn't have access to CallerGuard (which lives in the
    // Rust core's rlib and isn't exposed across the C ABI).
    glucore_set_caller_identity("cpp_engine");

    glucore::GluResult r = glucore_call(
        "java_renderer", "scale", args, 2);

    // Restore the outer caller. Done unconditionally — even if the call
    // failed — so the link table isn't left in a poisoned state.
    glucore_set_caller_identity("python");

    if (r.status != glucore::GluStatus::Ok) {
        // Return a sentinel so callers can observe the failure without a
        // C++ exception escaping across the extern "C" edge (Constraint 6).
        // The wrapper at the boundary turns this into a normal GluResult
        // error if a caller wants to surface it to Python.
        return -1.0;
    }
    double scaled = r.value.float_v;
    return v + scaled;
}
GLUCORE_EXPORT(accelerate_via_render)

// --- Part 0a: real cpp_engine → physics attempt (DENIED) ------------------
//
// The Part 0a handoff asks for a TEMPORARY debug entry point on cpp_engine
// that calls glucore_call("physics", ...) directly. With the link table
// declaring cpp_engine → [] (no callees), this MUST be denied — proving the
// router checks *this caller, this callee*, not just "is physics reachable
// from anyone."
//
// This function is the genuine feature version of the temporary debug entry
// point: it tries to call physics.calculate_force(10.0, 9.8) and reports
// whether the call was allowed or denied. The handoff says we can leave it
// in as a real feature (it demonstrates C++'s ability to be a caller), so
// it stays — but it's clearly labeled as the Part 0a verification probe.
//
// Returns the GluStatus as an int64_t:
//   0 = Ok (would mean the link WAS declared — should NOT happen with the
//          default glucore.toml)
//   4 = LinkDenied (expected — cpp_engine → physics is NOT in [links])
//   other = unexpected error
int64_t attempt_physics_call_from_cpp() {
    glucore::GluValue args[2];
    args[0].float_v = 10.0;
    args[1].float_v = 9.8;

    glucore_set_caller_identity("cpp_engine");
    glucore::GluResult r = glucore_call(
        "physics", "calculate_force", args, 2);
    glucore_set_caller_identity("python");

    return static_cast<int64_t>(r.status);
}
GLUCORE_EXPORT(attempt_physics_call_from_cpp)
