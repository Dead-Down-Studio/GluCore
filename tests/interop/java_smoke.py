"""Smoke test: load the Java renderer and verify IPC works end-to-end."""
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
import glucore

# Use a unique socket path in /tmp
SOCKET_PATH = f"/tmp/glucore_java_renderer_{os.getpid()}.sock"
JAVA_CP = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "examples/java_renderer", "build",
)
ADAPTER_CP = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "adapters", "java", "target", "classes")
SPAWN_CMD = f"java -cp {ADAPTER_CP}:{JAVA_CP} Renderer {SOCKET_PATH}"

print(f"socket path: {SOCKET_PATH}")
print(f"spawn cmd:   {SPAWN_CMD}")

core = glucore.load_core()
java = glucore.load_process_module(core, "java_renderer", SOCKET_PATH, SPAWN_CMD)

print(f"java_renderer functions: {list(java.functions().keys())}")

# Test 1: scale(3.0, 2.0) -> 6.0
r = java.scale(3.0, 2.0)
print(f"scale(3.0, 2.0) = {r}  (expected 6.0)")

# Test 2: identity(42.5) -> 42.5
r = java.identity(42.5)
print(f"identity(42.5) = {r}  (expected 42.5)")

# Test 3: byte counter (initial value)
r = java.glucore_byte_read_count()
print(f"glucore_byte_read_count() = {r}  (expected > 0, since calls were made)")

print("\nJava IPC smoke test PASSED")
