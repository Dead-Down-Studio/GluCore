# glucore (Python adapter)

Python adapter for [GluCore](https://github.com/glucore/glucore) — a
language-agnostic runtime coordination layer.

## Install

```bash
pip3 install glucore
```

## Quick start

```python
import glucore

core = glucore.load_core()
physics = glucore.load_module(core, "physics")
result = physics.calculate_force(10.0, 9.8)
print(result)  # 98.0
```

## What this package provides

- `glucore.load_core()` — load libglucore_core.so, apply topology from glucore.toml
- `glucore.load_module(core, name)` — load an ARTIFACT module (.so)
- `glucore.load_process_module(core, name, socket, cmd)` — load a PROCESS module (IPC)
- `glucore.GluProxy` — module proxy with pre-call type validation
- `glucore.pack_arg` / `glucore.unpack_return` — type conversion helpers

## Requirements

- Python >= 3.11 (for `tomllib`)
- `libglucore_core.so` must be built and discoverable. Build it from the
  Rust core: `cargo build --release` in the GluCore project root, then
  set `GLUCORE_CORE_PATH` or place it on `LD_LIBRARY_PATH`.

## License

MIT
