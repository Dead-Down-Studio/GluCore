# CMakeLists.txt — cpp_engine, a C++ ARTIFACT module for GluCore (Task 7, 9).
#
# Lives at examples/cpp_engine/. The C++ adapter header (glucore_export.hpp)
# lives at adapters/cpp/include/ — shared across all C++ modules.
# The project root is two levels up from this directory.

cmake_minimum_required(VERSION 3.15)
project(cpp_engine CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

set(CMAKE_POSITION_INDEPENDENT_CODE ON)

add_library(cpp_engine SHARED
    src/engine.cpp
)

# The C++ adapter header is at <root>/adapters/cpp/include/.
# Also add <root>/core/include/ so engine.cpp can #include <glucore.h> if needed.
get_filename_component(GLUCORE_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../.." ABSOLUTE)
target_include_directories(cpp_engine PRIVATE
    "${GLUCORE_ROOT}/adapters/cpp/include"
    "${GLUCORE_ROOT}/core/include"
)

if(MSVC)
    target_compile_options(cpp_engine PRIVATE /EHsc)
else()
    target_compile_options(cpp_engine PRIVATE -fexceptions)
endif()

target_compile_options(cpp_engine PRIVATE
    $<$<PLATFORM_ID:Darwin>:-fvisibility=default>
)

# --- Link against libglucore_core ---
set(GLUCORE_CORE_LIB
    "${GLUCORE_ROOT}/target/release/${CMAKE_SHARED_LIBRARY_PREFIX}glucore_core${CMAKE_SHARED_LIBRARY_SUFFIX}")

if(NOT EXISTS "${GLUCORE_CORE_LIB}")
    message(FATAL_ERROR
        "libglucore_core not built yet. Run 'cargo build --release' from the "
        "project root before configuring cpp_engine. Expected: "
        "${GLUCORE_CORE_LIB}")
endif()

target_link_libraries(cpp_engine PRIVATE "${GLUCORE_CORE_LIB}")

target_compile_definitions(cpp_engine PRIVATE
    GLUCORE_CORE_PATH="${GLUCORE_CORE_LIB}")

# rpath
set_target_properties(cpp_engine PROPERTIES
    BUILD_RPATH "$ORIGIN"
    INSTALL_RPATH "$ORIGIN")
if(APPLE)
    set_target_properties(cpp_engine PROPERTIES
        BUILD_RPATH "@loader_path"
        INSTALL_RPATH "@loader_path")
endif()

# Symlink into target/release/ so glucore.py finds it.
add_custom_command(TARGET cpp_engine POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E create_symlink
        "$<TARGET_FILE:cpp_engine>"
        "${GLUCORE_ROOT}/target/release/${CMAKE_SHARED_LIBRARY_PREFIX}cpp_engine${CMAKE_SHARED_LIBRARY_SUFFIX}"
    COMMENT "Symlinking cpp_engine .so into target/release/")
