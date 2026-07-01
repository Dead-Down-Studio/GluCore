"""Part 0c re-verification: byte-level evidence for Constraint #8.

Constraint #8 (Phase 2 handoff) says: "the Rust core checks caller→callee
links BEFORE sending anything over the socket."

The original Phase 2 report supported this claim only with "the call failed
fast" — which doesn't actually distinguish "denied before sending" from
"sent, and something else made it fail quickly."

This script adds the missing evidence: a byte-counter on the Java side that
increments on every byte read from the socket. We trigger a denied-link
case (an undeclared caller attempting java_renderer) and read the counter
AFTER the denial. If the counter stayed at the value it had BEFORE the
denial attempt, that's byte-level proof the Rust core never sent anything
to Java for the denied call — the link check happened before the IPC
write, exactly as Constraint #8 requires.

Scenario:
  (0) Start Java renderer, get initial byte counter (after registration
      handshake — should be > 0 because the registration bytes ARE read).
  (1) python -> java_renderer.scale(3.0, 2.0) : SUCCEED (link declared)
      Counter should increase (CALL + RESULT bytes were exchanged).
  (2) physics -> java_renderer.scale(...) : DENY (link NOT declared)
      Counter MUST NOT increase — Rust should refuse before sending.
  (3) python -> java_renderer.scale(3.0, 2.0) : SUCCEED (still declared)
      Counter should increase again (proves the socket is still usable).
  (4) Final counter read for the record.

The literal byte counts are reported at every step. If step (2) shows the
same counter as step (1)'s post-call value, Constraint #8 is verified at
the byte level, not by inference.
"""
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
import glucore
from glucore import GluStatus

# Reuse the Java smoke test setup
SOCKET_PATH = f"/tmp/glucore_java_renderer_part0c_{os.getpid()}.sock"
JAVA_CP = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "examples/java_renderer", "build",
)
ADAPTER_CP = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "adapters", "java", "target", "classes")
SPAWN_CMD = f"java -cp {ADAPTER_CP}:{JAVA_CP} Renderer {SOCKET_PATH}"


def main():
    core = glucore.load_core()
    physics = glucore.load_module(core, "physics")
    java = glucore.load_process_module(core, "java_renderer", SOCKET_PATH, SPAWN_CMD)
    core.set_caller_identity("python")

    print("=" * 78)
    print("PART 0c RE-VERIFICATION — byte-level evidence for Constraint #8")
    print("=" * 78)
    print()
    print("Constraint #8: the Rust core checks caller→callee links BEFORE")
    print("sending anything over the socket. Evidence: a byte-counter on the")
    print("Java side increments on every byte read. A denied-link case must")
    print("leave the counter unchanged.")
    print()

    # (0) Initial byte counter — registration bytes have already been read.
    # We also calibrate the "counter-query overhead": each call to
    # glucore_byte_read_count() ITSELF sends a CALL message over the socket
    # and reads the response — but only the CALL direction increments the
    # counter (the RESULT is written, not read). So each counter query adds
    # a fixed number of bytes (the size of its CALL message) to the counter.
    #
    # To measure this overhead, do two consecutive counter queries with
    # nothing in between. The difference is X = bytes per counter query.
    initial_count = java.glucore_byte_read_count()
    calibration_count = java.glucore_byte_read_count()
    counter_query_overhead = calibration_count - initial_count
    print(f"  (0) initial byte counter (after registration): {initial_count}")
    print(f"      counter-query overhead X = {counter_query_overhead} bytes")
    print(f"      (each call to glucore_byte_read_count() itself reads this many")
    print(f"       bytes off the socket — subtract from any measurement below)")
    print()

    # (1) python -> java_renderer.scale(3.0, 2.0) : SUCCEED (link declared)
    before_succeed = java.glucore_byte_read_count()
    r1 = java.scale(3.0, 2.0)
    after_succeed = java.glucore_byte_read_count()
    # Between `before_succeed` and `after_succeed`, two things happened:
    #   - scale() sent Y bytes (the CALL message Java reads)
    #   - the `after_succeed` counter query itself added X bytes
    # So: after_succeed - before_succeed = Y + X. Solve for Y:
    bytes_for_succeed = after_succeed - before_succeed - counter_query_overhead
    print(f"  (1) python->java_renderer.scale(3.0, 2.0) = {r1}  (expected 6.0)")
    print(f"      byte counter before: {before_succeed}")
    print(f"      byte counter after:  {after_succeed}")
    print(f"      bytes exchanged for this call (scale): {bytes_for_succeed}")
    print(f"      (proves the counter DOES increment when bytes are sent)")
    print()

    # (2) physics -> java_renderer : DENY. physics has NO link to
    # java_renderer in glucore.toml. The Rust core MUST refuse BEFORE
    # sending anything over the socket. The byte counter MUST NOT change
    # (other than the counter-query overhead).
    before_deny = java.glucore_byte_read_count()
    core.set_caller_identity("physics")
    denied_ok = False
    try:
        # Use the raw core.call to bypass the GluProxy's signature check,
        # so we observe the CORE's decision directly.
        v0 = glucore.GluValue()
        v0.float = 3.0
        v1 = glucore.GluValue()
        v1.float = 2.0
        packed = [v0, v1]
        r = core.call("java_renderer", "scale", packed)
        if r.status == GluStatus.LINK_DENIED:
            denied_ok = True
            detail = f"core returned LinkDenied(status={r.status})"
        else:
            detail = f"EXPECTED LinkDenied but got status={r.status} (BUG)"
    except Exception as e:
        detail = f"unexpected exception: {e!r}"
    core.set_caller_identity("python")  # restore
    after_deny = java.glucore_byte_read_count()
    # Same formula: between before_deny and after_deny, the only counter-query
    # is the after_deny call. The denied call should add 0 bytes.
    bytes_for_deny = after_deny - before_deny - counter_query_overhead
    print(f"  (2) physics->java_renderer.scale(3.0, 2.0) [impersonated caller]")
    print(f"      {detail}")
    print(f"      byte counter before: {before_deny}")
    print(f"      byte counter after:  {after_deny}")
    print(f"      bytes exchanged for this DENIED call: {bytes_for_deny}")
    if bytes_for_deny == 0:
        print(f"      [PASS] zero bytes exchanged — Constraint #8 verified at byte level")
    else:
        print(f"      [FAIL] {bytes_for_deny} bytes exchanged for a denied call — Constraint #8 VIOLATED")
    print()

    # (3) python -> java_renderer.scale(3.0, 2.0) : SUCCEED (still declared)
    # Proves the socket is still usable after the denial — the Rust core
    # didn't corrupt the IPC state by half-sending a message.
    before_recover = java.glucore_byte_read_count()
    r3 = java.scale(3.0, 2.0)
    after_recover = java.glucore_byte_read_count()
    bytes_for_recover = after_recover - before_recover - counter_query_overhead
    print(f"  (3) python->java_renderer.scale(3.0, 2.0) = {r3}  (expected 6.0)")
    print(f"      byte counter before: {before_recover}")
    print(f"      byte counter after:  {after_recover}")
    print(f"      bytes exchanged for this call (scale, recovery): {bytes_for_recover}")
    print(f"      (proves the socket is still usable after the denial)")
    print()

    print("=" * 78)
    all_ok = (denied_ok and bytes_for_deny == 0 and r1 == 6.0 and r3 == 6.0)
    if all_ok:
        print("CONSTRAINT #8 VERIFIED AT BYTE LEVEL:")
        print(f"  - Initial counter (post-registration): {initial_count}")
        print(f"  - After SUCCEED call:                  {after_succeed} (+{bytes_for_succeed} bytes)")
        print(f"  - After DENIED call:                   {after_deny} (+{bytes_for_deny} bytes)")
        print(f"  - After SUCCEED call (recovery):       {after_recover} (+{bytes_for_recover} bytes)")
        print()
        print(f"The denied-link case exchanged {bytes_for_deny} bytes — byte-level evidence")
        print("that the Rust core checks caller→callee links BEFORE sending anything")
        print("over the socket. The original Phase 2 report's claim is now backed")
        print("by a number, not inferred from call latency or failure speed.")
    else:
        print("SOME CHECKS FAILED.")
        sys.exit(1)


if __name__ == "__main__":
    main()
