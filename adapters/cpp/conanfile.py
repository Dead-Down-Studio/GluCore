# conanfile.py — Conan recipe for the GluCore C++ adapter.
#
# Build:   conan create . --version=0.2.0
# Install: conan install --requires=glucore-cpp/0.2.0 --output-folder=build
# Use:     #include <glucore_export.hpp>  (in your CMakeLists.txt: target_link_libraries(my_mod glucore-cpp::glucore-cpp))

from conan import ConanFile
from conan.tools.files import copy
from conan.tools.cmake import CMakeDeps
import os


class GlucoreCppConan(ConanFile):
    name = "glucore-cpp"
    version = "0.2.0"
    description = "C++ adapter for GluCore — language-agnostic runtime coordination"
    license = "MIT"
    url = "https://github.com/glucore/glucore"
    homepage = "https://github.com/glucore/glucore"
    topics = ("ffi", "ipc", "cross-language", "runtime", "adapter")
    settings = "os", "compiler", "build_type", "arch"

    # The C++ adapter is header-only — it's just glucore_export.hpp.
    # The core library (libglucore_core.so) is a separate dependency,
    # built from the Rust crate `glucore_core`.
    package_type = "header-library"
    no_copy_source = True

    def requirements(self):
        # The C++ adapter needs the core library at link time.
        # In a real setup this would be:
        #   self.requires(f"glucore-core/{self.version}")
        # For now, document that the user must build libglucore_core.so
        # from the Rust crate and make it discoverable.
        pass

    def package(self):
        # Copy the C++ adapter header
        copy(self, "glucore_export.hpp",
             src=os.path.join(self.source_folder, "include"),
             dst=os.path.join(self.package_folder, "include"))
        # Also copy the public C ABI contract
        copy(self, "glucore.h",
             src=os.path.join(self.source_folder, "..", "..", "core", "include"),
             dst=os.path.join(self.package_folder, "include"))

    def package_info(self):
        self.cpp_info.bindirs = []
        self.cpp_info.libdirs = []
        self.cpp_info.includedirs = ["include"]
        # C++17 required for the adapter (if constexpr, structured bindings)
        self.cpp_info.cxxflags = ["-std=c++17"]
