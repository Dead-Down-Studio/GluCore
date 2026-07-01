#!/usr/bin/env bash
# build.sh — universal build script for GluCore v2.
#
# Builds every component in the project in dependency order and leaves the
# project in a run-ready state. After this script succeeds, you can run:
#
#     python3 main.py                       # basic smoke test
#     python3 scripts/bench_final.py        # perf benchmark
#     python3 tests/interop/task9_dod.py    # three-language mesh test
#
# What gets built:
#   1. Rust workspace (core + adapters/rust + examples/physics) → target/release/*.so
#   2. C++ module (examples/cpp_engine)       → examples/cpp_engine/build/libcpp_engine.so
#                                                + symlinked into target/release/
#   3. Java renderer module (examples/java_renderer) → examples/java_renderer/build/Renderer.class
#   4. Raw-ctypes engine (scripts/engine.so)  → for benchmark comparison
#
# Project structure (matches the C-based skeleton):
#   core/               Rust core crate (libglucore_core.so)
#   adapters/python/    Python adapter (ctypes bindings, GluProxy, etc.)
#   adapters/rust/      Rust proc-macro (#[glucore::export])
#   adapters/cpp/       C++ adapter header (glucore_export.hpp)
#   adapters/java/      (reserved — Java adapter is fused with the module)
#   examples/physics/   Example Rust ARTIFACT module
#   examples/cpp_engine/ Example C++ ARTIFACT module
#   examples/java_renderer/ Example Java PROCESS module
#   tests/unit/         Single-module tests
#   tests/integration/  Multi-module same-process tests
#   tests/interop/      Cross-language IPC tests
#   scripts/            Benchmarks
#   docs/               Wiki
#   parser/env/cli/schemas/  Stubs (reserved for future use)
set -euo pipefail

if [ -t 1 ]; then
    GREEN="\033[32m"; YELLOW="\033[33m"; RED="\033[31m"; BOLD="\033[1m"; RESET="\033[0m"
else
    GREEN=""; YELLOW=""; RED=""; BOLD=""; RESET=""
fi
section() { printf "\n${BOLD}=== %s ===${RESET}\n" "$1"; }
ok()      { printf "  ${GREEN}[OK]${RESET} %s\n" "$1"; }
warn()    { printf "  ${YELLOW}[WARN]${RESET} %s\n" "$1"; }
die()     { printf "  ${RED}[FAIL]${RESET} %s\n" "$1" >&2; exit 1; }

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$PROJECT_ROOT"

case "$(uname -s)" in
    Linux)  DLL_EXT="so";    DLL_PREFIX="lib"; PLATFORM="linux"   ;;
    Darwin) DLL_EXT="dylib"; DLL_PREFIX="lib"; PLATFORM="macos"   ;;
    *)
        die "Unrecognized OS: $(uname -s). Linux and macOS only."
        ;;
esac

# --- Toolchain checks ---
section "Checking toolchain"
command -v cargo   >/dev/null || die "cargo not found. Install via https://rustup.rs/"
command -v rustc   >/dev/null || die "rustc not found. Install via https://rustup.rs/"
command -v cmake   >/dev/null || die "cmake not found."
command -v g++     >/dev/null || CXX="$(command -v clang++)"
command -v g++     >/dev/null || CXX="${CXX:-}"
: "${CXX:=g++}"
command -v "$CXX"  >/dev/null || die "Neither g++ nor clang++ found."
command -v javac   >/dev/null || die "javac not found. Install JDK >= 16."
command -v java    >/dev/null || die "java runtime not found."
command -v python3 >/dev/null || die "python3 not found. Install Python >= 3.11."

PY_VER="$(python3 -c 'import sys; print(f"{sys.version_info[0]}.{sys.version_info[1]}")')"
PY_OK="$(python3 -c 'import sys; print(1 if sys.version_info >= (3,11) else 0)')"
if [ "$PY_OK" != "1" ]; then
    die "Python $PY_VER found, but >= 3.11 is required (for tomllib)."
fi

JAVA_VER="$(javac --version 2>&1 | awk '{print $2}')"
ok "cargo:    $(cargo --version)"
ok "cmake:    $(cmake --version | head -1)"
ok "C++:      $CXX ($($CXX --version | head -1))"
ok "javac:    $JAVA_VER"
ok "python:   $PY_VER"
ok "platform: $PLATFORM (DLL_EXT=.$DLL_EXT)"

# --- 1. Rust workspace ---
section "Building Rust workspace (core + adapters/rust + examples/physics)"
cargo build --release
CORE_LIB="target/release/${DLL_PREFIX}glucore_core.${DLL_EXT}"
PHYSICS_LIB="target/release/${DLL_PREFIX}physics.${DLL_EXT}"
[ -f "$CORE_LIB" ]   || die "$CORE_LIB not built"
[ -f "$PHYSICS_LIB" ] || die "$PHYSICS_LIB not built"
ok "$CORE_LIB"
ok "$PHYSICS_LIB"

# --- 2. C++ module ---
section "Building C++ module (examples/cpp_engine)"
cmake -S examples/cpp_engine -B examples/cpp_engine/build -DCMAKE_BUILD_TYPE=Release \
    > /dev/null 2>&1 || die "cmake configure failed for cpp_engine."
cmake --build examples/cpp_engine/build --config Release > /dev/null 2>&1 || die "cmake build failed for cpp_engine."

CPP_LIB="examples/cpp_engine/build/${DLL_PREFIX}cpp_engine.${DLL_EXT}"
[ -f "$CPP_LIB" ] || die "$CPP_LIB not built"

CPP_SYMLINK="target/release/${DLL_PREFIX}cpp_engine.${DLL_EXT}"
if [ -L "$CPP_SYMLINK" ] || [ -e "$CPP_SYMLINK" ]; then
    rm -f "$CPP_SYMLINK"
fi
ln -s "$PROJECT_ROOT/$CPP_LIB" "$CPP_SYMLINK"
ok "$CPP_LIB"
ok "symlinked into $CPP_SYMLINK"

if [ "$PLATFORM" = "linux" ]; then
    if command -v ldd >/dev/null; then
        LDD_OUT="$(ldd "$CPP_LIB" | grep glucore_core || true)"
        if [ -z "$LDD_OUT" ]; then
            warn "$CPP_LIB does not appear to link against libglucore_core."
        else
            ok "linked: $LDD_OUT"
        fi
    fi
fi

# --- 3. Java adapter + renderer module ---
section "Building Java adapter + renderer module"

# 3a. Build the Java adapter (glucore-java JAR)
mkdir -p adapters/java/target/classes
javac -d adapters/java/target/classes adapters/java/src/main/java/glucore/*.java
[ -f adapters/java/target/classes/glucore/GluAdapter.class ] || die "GluAdapter.class not built"
VERSION=$(cat VERSION)
jar cf "adapters/java/target/glucore-java-${VERSION}.jar" -C adapters/java/target/classes .
ok "adapters/java/target/glucore-java-${VERSION}.jar"

# 3b. Build the example renderer using the adapter
mkdir -p examples/java_renderer/build
javac -cp "adapters/java/target/classes" -d examples/java_renderer/build \
    examples/java_renderer/src/main/java/Renderer.java
[ -f examples/java_renderer/build/Renderer.class ] || die "Renderer.class not built"
ok "examples/java_renderer/build/Renderer.class (uses glucore adapter)"

# --- 4. Raw-ctypes engine ---
section "Building raw-ctypes engine (scripts/engine.${DLL_EXT})"
RAW_ENGINE="scripts/engine.${DLL_EXT}"
RAW_SRC="scripts/engine.cpp"
cat > "$RAW_SRC" <<'CPPEOF'
#include <cstdint>
extern "C" {
double add(double a, double b) { return a + b; }
void step_physics(float* h, float* v, int n, float dt, float g) {
    for (int i = 0; i < n; i++) {
        v[i] -= g * dt;
        h[i] += v[i] * dt;
        if (h[i] < 0.0f) { h[i] = 0.0f; v[i] = -v[i] * 0.8f; }
    }
}
const char* version() { return "raw_engine 0.1"; }
}
CPPEOF
"$CXX" -std=c++17 -O2 -fPIC -shared -o "$RAW_ENGINE" "$RAW_SRC"
[ -f "$RAW_ENGINE" ] || die "$RAW_ENGINE not built"
ok "$RAW_ENGINE"

# --- 5. Smoke test ---
section "Smoke test (python3 main.py)"
if python3 main.py > /tmp/glucore_smoke.log 2>&1; then
    ok "main.py ran successfully:"
    sed 's/^/    /' /tmp/glucore_smoke.log
else
    warn "main.py failed. Output:"
    sed 's/^/    /' /tmp/glucore_smoke.log
    die "Smoke test failed."
fi

# --- Final summary ---
section "Build complete — project is run-ready"
cat <<EOF

  Built artifacts:
    Rust core:     $CORE_LIB
    Rust physics:  $PHYSICS_LIB
    C++ module:    $CPP_LIB (symlinked into target/release/)
    Java renderer: examples/java_renderer/build/Renderer.class
    Raw engine:    $RAW_ENGINE

  Try these commands:
    python3 main.py                              # basic smoke test (just ran)
    python3 scripts/bench_final.py               # perf benchmark (4.7x slower than raw)
    python3 tests/unit/task1_dod.py              # Rust panic safety
    python3 tests/unit/task3_dod.py              # String/Buffer round-trips
    python3 tests/unit/task4_dod.py              # pre-call type validation
    python3 tests/unit/task6a_dod.py             # no RSS leak across 120k calls
    python3 tests/integration/task5_dod.py       # link-enforcement
    python3 tests/integration/task7_multicaller_dod.py  # multi-caller mesh
    python3 tests/integration/part0a_dod.py      # REAL C++ caller denied
    python3 tests/integration/part0b_dod.py      # caller-stack restore clean
    python3 tests/interop/part0c_dod.py          # 0 bytes for denied-link IPC
    python3 tests/interop/task9_dod.py           # three-language mesh
    python3 tests/interop/task10_dod.py          # Java IPC caller (CALLBACK_CALL)

  Notes:
    - Java tests need 'java' on PATH (already verified above).
    - See docs/ for the full wiki.
    - See KNOWN_FOOTGUNS.md for the 8 documented footguns.

EOF
