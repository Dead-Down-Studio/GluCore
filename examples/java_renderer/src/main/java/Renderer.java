import glucore.GluAdapter;
import glucore.GluTypes;
import java.io.*;

/**
 * Renderer — Example Java PROCESS module for GluCore.
 *
 * Extends {@link glucore.GluAdapter} which handles the wire protocol,
 * socket lifecycle, and registration. This class only implements the
 * actual module logic (the exported functions) and the registration
 * table.
 *
 * Run: java -cp adapters/java/target/classes:examples/java_renderer/build Renderer <socket_path>
 */
public class Renderer extends GluAdapter {

    // --- Exported Java methods (callable via the wire protocol) ---

    public static double scale(double v, double factor) {
        return v * factor;
    }

    public static long glucoreByteReadCount() {
        return instance.bytesReadFromSocket;
    }

    public static double identity(double v) {
        return v;
    }

    // --- Task 10: Java-initiated outward calls ---

    public double scaleViaPhysics(double mass, double accel) throws Exception {
        ByteArrayOutputStream argBuf = new ByteArrayOutputStream();
        argBuf.write(GluTypes.FLOAT);
        writeU64LE(argBuf, Double.doubleToRawLongBits(mass));
        argBuf.write(GluTypes.FLOAT);
        writeU64LE(argBuf, Double.doubleToRawLongBits(accel));
        byte[] result = glucoreCall("physics", "calculate_force", argBuf.toByteArray());
        if (result[0] != 0) {
            int[] off = {1};
            byte[] msgBytes = readLenBytes(result, off);
            throw new RuntimeException("physics::calculate_force failed: " + new String(msgBytes, "UTF-8"));
        }
        return Double.longBitsToDouble(readU64LE(result, new int[]{2}));
    }

    public long attemptUndeclaredCall() throws Exception {
        ByteArrayOutputStream argBuf = new ByteArrayOutputStream();
        argBuf.write(GluTypes.FLOAT);
        writeU64LE(argBuf, Double.doubleToRawLongBits(5.0));
        argBuf.write(GluTypes.FLOAT);
        writeU64LE(argBuf, Double.doubleToRawLongBits(2.0));
        byte[] result = glucoreCall("cpp_engine", "accelerate", argBuf.toByteArray());
        return result[0] & 0xFFL;
    }

    public long attemptSelfCall() throws Exception {
        ByteArrayOutputStream argBuf = new ByteArrayOutputStream();
        argBuf.write(GluTypes.FLOAT);
        writeU64LE(argBuf, Double.doubleToRawLongBits(3.0));
        argBuf.write(GluTypes.FLOAT);
        writeU64LE(argBuf, Double.doubleToRawLongBits(2.0));
        byte[] result = glucoreCall("java_renderer", "scale", argBuf.toByteArray());
        return result[0] & 0xFFL;
    }

    // --- GluAdapter implementation ---

    private static Renderer instance;

    @Override
    protected String getModuleName() { return "java_renderer"; }

    @Override
    protected void sendRegistration(OutputStream out) throws IOException {
        ByteArrayOutputStream payload = new ByteArrayOutputStream();
        writeLenStr(payload, "java_renderer");

        String[][] exports = {
            {"scale",                    String.valueOf(GluTypes.FLOAT), fmtTags(GluTypes.FLOAT, GluTypes.FLOAT)},
            {"identity",                 String.valueOf(GluTypes.FLOAT), fmtTags(GluTypes.FLOAT)},
            {"glucore_byte_read_count",  String.valueOf(GluTypes.INT),   ""},
            {"scale_via_physics",        String.valueOf(GluTypes.FLOAT), fmtTags(GluTypes.FLOAT, GluTypes.FLOAT)},
            {"attempt_undeclared_call",  String.valueOf(GluTypes.INT),   ""},
            {"attempt_self_call",        String.valueOf(GluTypes.INT),   ""},
        };
        payload.write(exports.length);
        for (String[] e : exports) {
            writeLenStr(payload, e[0]);
            payload.write(Integer.parseInt(e[1]));
            String[] params = e[2].isEmpty() ? new String[0] : e[2].split(",");
            payload.write(params.length);
            for (String p : params) payload.write(Integer.parseInt(p.trim()));
        }

        byte[] payloadBytes = payload.toByteArray();
        writeU32LE(out, payloadBytes.length);
        out.write(payloadBytes);
        out.flush();
    }

    private static String fmtTags(int... tags) {
        if (tags.length == 0) return "";
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < tags.length; i++) {
            if (i > 0) sb.append(",");
            sb.append(tags[i]);
        }
        return sb.toString();
    }

    @Override
    protected byte[] dispatchCall(String function, byte[] argsPayload) throws Exception {
        int[] off = {0};
        int argc = argsPayload[off[0]] & 0xFF; off[0] += 1;

        if (function.equals("scale")) {
            int tag0 = argsPayload[off[0]] & 0xFF; off[0] += 1;
            double v = Double.longBitsToDouble(readU64LE(argsPayload, off));
            int tag1 = argsPayload[off[0]] & 0xFF; off[0] += 1;
            double f = Double.longBitsToDouble(readU64LE(argsPayload, off));
            return encodeFloatResult(scale(v, f));
        }
        if (function.equals("identity")) {
            int tag0 = argsPayload[off[0]] & 0xFF; off[0] += 1;
            double v = Double.longBitsToDouble(readU64LE(argsPayload, off));
            return encodeFloatResult(identity(v));
        }
        if (function.equals("glucore_byte_read_count")) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            out.write(0); out.write(GluTypes.INT);
            writeU64LE(out, glucoreByteReadCount());
            return out.toByteArray();
        }
        if (function.equals("scale_via_physics")) {
            int tag0 = argsPayload[off[0]] & 0xFF; off[0] += 1;
            double mass = Double.longBitsToDouble(readU64LE(argsPayload, off));
            int tag1 = argsPayload[off[0]] & 0xFF; off[0] += 1;
            double accel = Double.longBitsToDouble(readU64LE(argsPayload, off));
            return encodeFloatResult(scaleViaPhysics(mass, accel));
        }
        if (function.equals("attempt_undeclared_call")) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            out.write(0); out.write(GluTypes.INT);
            writeU64LE(out, attemptUndeclaredCall());
            return out.toByteArray();
        }
        if (function.equals("attempt_self_call")) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            out.write(0); out.write(GluTypes.INT);
            writeU64LE(out, attemptSelfCall());
            return out.toByteArray();
        }
        throw new RuntimeException("unknown function: " + function);
    }

    private byte[] encodeFloatResult(double r) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.write(0); out.write(GluTypes.FLOAT);
        writeU64LE(out, Double.doubleToRawLongBits(r));
        return out.toByteArray();
    }

    // --- Main ---

    public static void main(String[] args) throws Exception {
        if (args.length < 1) {
            System.err.println("usage: Renderer <socket_path>");
            System.exit(2);
        }
        instance = new Renderer();
        instance.run(args[0]);
    }
}
