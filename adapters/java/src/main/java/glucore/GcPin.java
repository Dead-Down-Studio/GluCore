package glucore;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * GcPin — JVM GC pinning for cross-language object references.
 *
 * STUB: v2 does not implement GC pinning because v2 has no GluHandle type.
 * Every value is copied across the boundary (Constraint 4: "copy, don't
 * borrow, for now").
 *
 * When GluHandle support is added, this class will prevent the JVM GC from
 * collecting objects GluCore holds handles to. The pattern:
 * <pre>{@code
 * private static final Map<Long, Object> PINNED = new ConcurrentHashMap<>();
 *
 * public static void pin(long handle, Object obj) {
 *     PINNED.put(handle, obj);
 * }
 *
 * public static void unpin(long handle) {
 *     PINNED.remove(handle);
 * }
 * }</pre>
 */
public final class GcPin {
    private GcPin() {}

    private static final Map<Long, Object> PINNED = new ConcurrentHashMap<>();

    public static void pin(long handle, Object obj) {
        PINNED.put(handle, obj);
    }

    public static void unpin(long handle) {
        PINNED.remove(handle);
    }
}
