"""gc_pin.py — GC pinning for cross-language object references.

STUB: v2 does not implement GC pinning because v2 has no GluHandle type.
Every value is copied across the boundary (Constraint 4: "copy, don't
borrow, for now").

When GluHandle support is added (Phase 2/3 of the new project skeleton),
this module will register pin/unpin callbacks with the Rust core so that
Python's garbage collector doesn't collect objects GluCore holds handles
to. See doc/04-adapter-authoring.md "Step 5: Implement GC pinning" for
the design.

The planned API:

    _PINNED: dict[int, object] = {}  # handle_id → Python object

    def pin(handle_id: int, obj: object) -> None:
        _PINNED[handle_id] = obj

    def unpin(handle_id: int) -> None:
        _PINNED.pop(handle_id, None)
"""
