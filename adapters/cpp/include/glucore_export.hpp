// glucore_export.hpp — C++ ARTIFACT adapter for GluCore (Task 7).
//
// A C++ module presents the SAME entry-point contract as a Rust module:
//   extern "C" GluModule glucore_module_info();
// returning an export table of GluExportEntry. Because the Rust core's
// dispatch path (glucore_call -> dispatch) only consumes that contract, a
// dlopen'd C++ module requires NO changes to glucore_core — which is the
// thing Task 7's mission asks us to prove.
//
// "No type declared twice" is achieved the C++ way: template argument
// deduction on a function pointer (the same effect `syn` gives Rust). The type
// information comes from the actual function signature, not from a second
// declaration. An unsupported type fails with a clear static_assert at compile
// time (Constraint 7).
//
// C++ exceptions thrown inside an exported method are caught at the wrapper
// boundary and turned into an error GluResult — they never unwind into Rust,
// which would be UB across the extern "C" edge (Constraint 6).
//
// !!! Layout must match glucore_core EXACTLY — verified by a round-trip
// test (see scripts/task7_layout.cpp), not by visual comparison. The numbers:
//   GluValue  16 bytes (union: f64 | i64 | GluSlice{ptr,len})
//   GluSlice  16 bytes (ptr@0, len@8)
//   GluResult 32 bytes (status@0 i32, pad, value@8, message@24)
//   GluSignatureFFI 24 (param_types*, param_count, return_type)
//   GluExportEntry 40 (name*, wrapper*, signature)
//   GluModule 24 (name*, entries*, count)

#pragma once

#include <cstdint>
#include <cstddef>
#include <cstring>
#include <string>
#include <vector>
#include <exception>
#include <type_traits>
#include <utility>
#include <initializer_list>

namespace glucore {

// --- ABI structs (mirror glucore_core, repr(C)) ---------------------------

enum class GluTypeTag : int32_t {
    Int    = 0,
    Float  = 1,
    String = 2,
    Buffer = 3,
    Handle = 4,
    Void   = 5,
};

enum class GluStatus : int32_t {
    Ok          = 0,
    Runtime     = 1,
    InvalidArgs = 2,
    NotFound    = 3,
    LinkDenied  = 4,
};

struct GluSlice {
    const uint8_t* ptr;
    size_t         len;
};

// 16-byte union. The active member is determined by the signature tag, exactly
// as on the Rust side (the tag is NOT embedded in the value).
union GluValue {
    double    float_v;   // GluTypeTag::Float
    int64_t   int_v;     // GluTypeTag::Int
    GluSlice  string;    // GluTypeTag::String
    GluSlice  buffer;    // GluTypeTag::Buffer
};

struct GluSignatureFFI {
    const GluTypeTag* param_types;
    size_t            param_count;
    GluTypeTag        return_type;
};

struct GluResult {
    GluStatus status;       // @0
    // 4 bytes padding to align value to 8
    GluValue  value;        // @8
    const char* message;    // @24
};

// Wrapper signature identical to the Rust macro's generated wrapper.
typedef GluResult (*GluWrapper)(const GluValue* args, size_t argc);

struct GluExportEntry {
    const char*     name;       // @0
    GluWrapper      wrapper;    // @8
    GluSignatureFFI signature;  // @16
};

struct GluModule {
    const char*           name;     // @0
    const GluExportEntry* entries;  // @8
    size_t                count;    // @16
};

// --- Type-tag traits (template deduction = the "no second declaration" trick)

template <typename T> struct type_tag {
    // Unsupported type -> compile error with a clear message (Constraint 7).
    static_assert(sizeof(T) == 0,
        "glucore::export: unsupported type — expected double, int64_t, "
        "std::string, or std::vector<uint8_t>");
};

template <> struct type_tag<double> {
    static constexpr GluTypeTag value = GluTypeTag::Float;
    using cpp_t = double;
};
template <> struct type_tag<int64_t> {
    static constexpr GluTypeTag value = GluTypeTag::Int;
    using cpp_t = int64_t;
};
template <> struct type_tag<std::string> {
    static constexpr GluTypeTag value = GluTypeTag::String;
    using cpp_t = std::string;
};
template <> struct type_tag<std::vector<uint8_t>> {
    static constexpr GluTypeTag value = GluTypeTag::Buffer;
    using cpp_t = std::vector<uint8_t>;
};

// --- Registry -------------------------------------------------------------

struct GluExportRecord {
    const char*     name;
    GluWrapper      wrapper;
    GluSignatureFFI signature;
};

// One definition lives in engine.cpp. Static initializers push into it at
// dlopen time — the standard C++ plugin idiom, mirroring Rust's
// inventory::submit!.
std::vector<GluExportRecord>& registry();

// --- Per-argument unpacking / per-return packing --------------------------
//
// These mirror what the Rust macro generates. The wrappers below branch on the
// matched type tag, exactly as the Rust wrapper reads args[i].float vs .string.

inline double   unpack_float(const GluValue& v) { return v.float_v; }
inline int64_t  unpack_int(const GluValue& v)   { return v.int_v; }

inline std::string unpack_string(const GluValue& v) {
    return std::string(reinterpret_cast<const char*>(v.string.ptr), v.string.len);
}
inline std::vector<uint8_t> unpack_buffer(const GluValue& v) {
    return std::vector<uint8_t>(v.buffer.ptr, v.buffer.ptr + v.buffer.len);
}

// Pack a return value into a GluValue, handing ownership to the caller (Rust
// core). String/Buffer return ownership is handed out as a heap allocation
// that glucore_free_buffer can reclaim — same convention as the Rust macro's
// Task-6a packer, so a single free function serves both languages.
inline GluValue pack_float(double r) { GluValue v; v.float_v = r; return v; }
inline GluValue pack_int(int64_t r)  { GluValue v; v.int_v = r; return v; }

inline GluValue pack_string(std::string&& r) {
    size_t len = r.size();
    uint8_t* mem = new uint8_t[len ? len : 1];
    if (len) std::memcpy(mem, r.data(), len);
    GluValue v; v.string.ptr = mem; v.string.len = len; return v;
}
inline GluValue pack_buffer(std::vector<uint8_t>&& r) {
    size_t len = r.size();
    uint8_t* mem = new uint8_t[len ? len : 1];
    if (len) std::memcpy(mem, r.data(), len);
    GluValue v; v.buffer.ptr = mem; v.buffer.len = len; return v;
}

// --- The template register_export (the core of the no-second-declaration trick)
//
// BUG FIX (Part 0c / Task 9): the previous design used a function-local
// `static Ret(*s_fn)(Args...)` inside `make_wrapper`. Because `make_wrapper`
// was a member of `Registrar<Ret(*)(Args...)>` (parameterized ONLY by the
// function signature), the static was SHARED across all functions with the
// same signature. The last `make_wrapper` call would overwrite `s_fn`,
// silently making every same-signature export call the LAST registered
// function. This manifested as `cpp_engine_link_table_size`,
// `cpp_engine_shared_state_checksum`, `cpp_engine_links_address`, and
// `attempt_physics_call_from_cpp` all returning 4 (the GluStatus::LinkDenied
// code from `attempt_physics_call_from_cpp`, the last-registered int64_t()
// function). Fixed by adding a per-function TAG as a template parameter —
// each function gets its own `s_fn` static.

// Primary template: parameterized by a unique Tag AND the function signature.
// The Tag is a unique empty struct generated by the GLUCORE_EXPORT macro
// per function, ensuring each function gets its own template instantiation
// and therefore its own `s_fn` static.
template <typename Tag, typename Sig> struct Registrar;

template <typename Tag, typename Ret, typename... Args>
struct Registrar<Tag, Ret(*)(Args...)> {
    // Per-instantiation static: each (Tag, Ret, Args...) combination gets
    // its own s_fn. The Tag guarantees uniqueness even for same-signature
    // functions.
    static inline Ret(*s_fn)(Args...) = nullptr;

    static GluWrapper make_wrapper(Ret(*fn)(Args...)) {
        s_fn = fn;
        return [](const GluValue* args, size_t argc) -> GluResult {
            try {
                return invoke<Ret, Args...>(s_fn, args, argc,
                    std::index_sequence_for<Args...>{});
            } catch (const std::exception& e) {
                GluResult r; r.status = GluStatus::Runtime;
                r.message = dup_cstr(e.what());
                r.value.int_v = 0; return r;
            } catch (...) {
                GluResult r; r.status = GluStatus::Runtime;
                r.message = dup_cstr("unknown C++ exception");
                r.value.int_v = 0; return r;
            }
        };
    }

    // Dispatch on each argument's type via type_tag<Args>::value (compile-time).
    template <typename R, typename... A, size_t... I>
    static GluResult invoke(R(*fn)(A...), const GluValue* args, size_t argc,
                            std::index_sequence<I...>) {
        (void)argc; // argc is validated structurally by the signature tags
        // For each Args[i], pick the right unpacker based on its tag.
        if constexpr (std::is_same_v<R, void>) {
            fn(unpack_one<A, I>(args)...);
            GluResult r; r.status = GluStatus::Ok; r.message = nullptr;
            r.value.int_v = 0; return r;
        } else {
            R ret = fn(unpack_one<A, I>(args)...);
            GluResult r; r.status = GluStatus::Ok; r.message = nullptr;
            r.value = pack_one<R>(std::move(ret)); return r;
        }
    }

    // Unpack argument i according to its C++ type (compile-time branched).
    template <typename A, size_t I>
    static A unpack_one(const GluValue* args) {
        if constexpr (std::is_same_v<A, double>)                 return unpack_float(args[I]);
        else if constexpr (std::is_same_v<A, int64_t>)           return unpack_int(args[I]);
        else if constexpr (std::is_same_v<A, std::string>)       return unpack_string(args[I]);
        else if constexpr (std::is_same_v<A, std::vector<uint8_t>>) return unpack_buffer(args[I]);
        else static_assert(sizeof(A) == 0, "unsupported argument type");
    }

    // Pack the return value according to its C++ type.
    template <typename R>
    static GluValue pack_one(R&& ret) {
        using D = std::decay_t<R>;
        if constexpr (std::is_same_v<D, double>)                return pack_float(std::move(ret));
        else if constexpr (std::is_same_v<D, int64_t>)          return pack_int(std::move(ret));
        else if constexpr (std::is_same_v<D, std::string>)      return pack_string(std::move(ret));
        else if constexpr (std::is_same_v<D, std::vector<uint8_t>>) return pack_buffer(std::move(ret));
        else static_assert(sizeof(D) == 0, "unsupported return type");
    }

    // helper: duplicate a message into C-string storage (leaked; module lifetime).
    static const char* dup_cstr(const char* s) {
        size_t n = std::strlen(s);
        char* m = new char[n + 1];
        std::memcpy(m, s, n + 1);
        return m;
    }
};

// Public registration entry point.
// Tag is a unique empty struct (generated by the GLUCORE_EXPORT macro) that
// forces a distinct template instantiation per function — see the bug-fix
// comment on `Registrar` above.
template <typename Tag, typename Ret, typename... Args>
void register_export(const char* name, Ret(*fn)(Args...)) {
    static const GluTypeTag param_types[] = { type_tag<Args>::value... };
    GluSignatureFFI sig;
    sig.param_types = param_types;
    sig.param_count = sizeof...(Args);
    sig.return_type = []() -> GluTypeTag {
        if constexpr (std::is_same_v<Ret, void>) return GluTypeTag::Void;
        else return type_tag<Ret>::value;
    }();
    GluExportRecord rec;
    rec.name = name;
    rec.wrapper = Registrar<Tag, Ret(*)(Args...)>::make_wrapper(fn);
    rec.signature = sig;
    registry().push_back(rec);
}

} // namespace glucore

// --- The macro authors use to mark an export. Mirrors #[glucore::export]:
//     a static initializer registers the function at dlopen time.
//
// BUG FIX: the macro generates a UNIQUE empty struct (GlucoreTag_##fn) per
// function and passes it as a template parameter to register_export. This
// forces a distinct template instantiation per function, giving each its
// own `s_fn` static. Without the tag, same-signature functions would share
// `s_fn` and silently call the LAST registered one.
#define GLUCORE_EXPORT(fn)                                                 \
    namespace {                                                            \
        struct GlucoreTag_##fn {};                                         \
        struct GlucoreRegistrar_##fn {                                     \
            GlucoreRegistrar_##fn() { ::glucore::register_export<GlucoreTag_##fn>(#fn, &fn); } \
        } glucore_registrar_instance_##fn;                                 \
    }
