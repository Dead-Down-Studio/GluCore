package glucore;

import java.io.*;
import java.net.*;
import java.nio.*;
import java.nio.channels.*;
import java.nio.file.*;
import java.util.*;

/**
 * GluAdapter — base class for Java PROCESS modules in GluCore.
 *
 * Handles the wire protocol, socket lifecycle, registration, and provides
 * a {@link #glucoreCall} helper for Java-initiated outward calls (Task 10).
 *
 * <h3>Usage</h3>
 * <pre>{@code
 * public class MyModule extends GluAdapter {
 *     public static void main(String[] args) throws Exception {
 *         new MyModule().run(args[0]);
 *     }
 *
 *     @Override
 *     protected void sendRegistration(OutputStream out) throws IOException {
 *         // Declare your exports: name, return tag, param tags
 *         // See GluTypes for tag constants.
 *     }
 *
 *     @Override
 *     protected byte[] dispatchCall(String function, byte[] argsPayload) throws Exception {
 *         // Dispatch on function name, decode args, call your Java methods,
 *         // encode the result. Use the read/write helpers from this class.
 *     }
 * }
 * }</pre>
 *
 * <h3>Java-initiated outward calls (Task 10)</h3>
 * While handling an inbound CALL, your dispatchCall can call
 * {@link #glucoreCall} to ask the Rust core to perform a glucore_call on
 * your behalf:
 * <pre>{@code
 * byte[] args = encodeArgs(10.0, 9.8);
 * byte[] result = glucoreCall("physics", "calculate_force", args);
 * // result[0] = 0 (OK) or 1 (error)
 * }</pre>
 */
public abstract class GluAdapter {

    /** Byte counter — incremented for every byte read from the socket. */
    public long bytesReadFromSocket = 0;

    /** The socket streams, stored so dispatchCall can call glucoreCall. */
    protected InputStream loopIn;
    protected OutputStream loopOut;

    // ---- Lifecycle -------------------------------------------------------

    /**
     * Bind a Unix domain socket, accept one connection from the Rust core,
     * send registration, then loop on CALL messages.
     */
    public void run(String socketPath) throws Exception {
        Path sockPath = Path.of(socketPath);
        try { Files.deleteIfExists(sockPath); }
        catch (IOException e) { System.err.println("warning: " + e); }

        ServerSocketChannel server = ServerSocketChannel.open(
            StandardProtocolFamily.UNIX);
        server.bind(UnixDomainSocketAddress.of(sockPath));
        SocketChannel ch = server.accept();

        loopIn = Channels.newInputStream(ch);
        loopOut = Channels.newOutputStream(ch);

        sendRegistration(loopOut);

        while (true) {
            byte[] lenBuf = new byte[4];
            int n = readFully(loopIn, lenBuf, 0, 4);
            if (n < 4) break;
            bytesReadFromSocket += 4;

            int msgLen = (int)(
                (lenBuf[0] & 0xFFL)
              | ((lenBuf[1] & 0xFFL) << 8)
              | ((lenBuf[2] & 0xFFL) << 16)
              | ((lenBuf[3] & 0xFFL) << 24)
            );
            if (msgLen < 0 || msgLen > 16 * 1024 * 1024) break;

            byte[] msg = new byte[msgLen];
            n = readFully(loopIn, msg, 0, msgLen);
            if (n < msgLen) break;
            bytesReadFromSocket += msgLen;

            if (msg[0] != GluTypes.MSG_CALL) break;

            // Parse payload: skip msg_type(1) + payload_len(4)
            int[] off = {1};
            readU32LE(msg, off); // discard payload_len
            byte[] payload = new byte[msgLen - off[0]];
            System.arraycopy(msg, off[0], payload, 0, payload.length);

            byte[] resultPayload;
            try {
                // Parse module/function from payload, then dispatch
                int[] poff = {0};
                String module = new String(readLenBytes(payload, poff), "UTF-8");
                String function = new String(readLenBytes(payload, poff), "UTF-8");
                readLenBytes(payload, poff); // caller id — ignored
                // Pass remaining bytes (starting with argc) to dispatchCall.
                // dispatchCall reads argc itself — don't consume it here.
                byte[] argsPayload = new byte[payload.length - poff[0]];
                System.arraycopy(payload, poff[0], argsPayload, 0, argsPayload.length);

                resultPayload = dispatchCall(function, argsPayload);
            } catch (Exception e) {
                ByteArrayOutputStream errOut = new ByteArrayOutputStream();
                errOut.write(1); // status error
                byte[] msgBytes = e.getMessage() == null
                    ? "(no message)".getBytes("UTF-8")
                    : e.getMessage().getBytes("UTF-8");
                writeLenBytes(errOut, msgBytes);
                resultPayload = errOut.toByteArray();
            }

            // Send RESULT: [u32 len][msg_type=0x01][u32 payload_len][payload]
            ByteArrayOutputStream fullOut = new ByteArrayOutputStream();
            fullOut.write(GluTypes.MSG_RESULT);
            writeU32LE(fullOut, resultPayload.length);
            fullOut.write(resultPayload);
            byte[] fullMsg = fullOut.toByteArray();
            writeU32LE(loopOut, fullMsg.length);
            loopOut.write(fullMsg);
            loopOut.flush();
        }

        ch.close();
        server.close();
    }

    // ---- Abstract methods ------------------------------------------------

    /**
     * Send the registration message: module name + export count + per-export
     * (name, return tag, param count, param tags).
     */
    protected abstract void sendRegistration(OutputStream out) throws IOException;

    /**
     * Dispatch a CALL to the matching Java method. The `argsPayload` contains
     * the raw arg bytes (after module/function/caller have been stripped).
     * Return the RESULT payload (starting with status byte).
     */
    protected abstract byte[] dispatchCall(String function, byte[] argsPayload) throws Exception;

    // ---- Task 10: Java-initiated outward calls ---------------------------

    /**
     * Send a CALLBACK_CALL asking Rust to perform a glucore_call on our
     * behalf, and read the CALLBACK_RESULT response.
     *
     * @param module   target module name
     * @param function target function name
     * @param args     encoded args: per arg [1 tag][8 value] for Float/Int,
     *                 [1 tag][4 len][bytes] for String/Buffer
     * @return result payload: [1 status][if OK: 1 tag + value][if error: 4 len + msg]
     */
    protected byte[] glucoreCall(String module, String function, byte[] args) throws IOException {
        ByteArrayOutputStream payload = new ByteArrayOutputStream();
        writeLenStr(payload, module);
        writeLenStr(payload, function);
        writeLenStr(payload, getModuleName()); // caller id
        int argc = args.length == 0 ? 0 : args.length / 9; // rough: 1 tag + 8 value
        payload.write(argc);
        payload.write(args);

        ByteArrayOutputStream msgBuf = new ByteArrayOutputStream();
        msgBuf.write(GluTypes.MSG_CALLBACK_CALL);
        writeU32LE(msgBuf, payload.size());
        msgBuf.write(payload.toByteArray());
        byte[] msg = msgBuf.toByteArray();

        synchronized (this) {
            writeU32LE(loopOut, msg.length);
            loopOut.write(msg);
            loopOut.flush();

            byte[] lenBuf = new byte[4];
            int n = readFully(loopIn, lenBuf, 0, 4);
            if (n < 4) throw new IOException("short read on CALLBACK_RESULT length");
            bytesReadFromSocket += 4;
            int respLen = (int)(
                (lenBuf[0] & 0xFFL)
              | ((lenBuf[1] & 0xFFL) << 8)
              | ((lenBuf[2] & 0xFFL) << 16)
              | ((lenBuf[3] & 0xFFL) << 24)
            );
            if (respLen < 0 || respLen > 16 * 1024 * 1024)
                throw new IOException("implausible CALLBACK_RESULT length: " + respLen);
            byte[] resp = new byte[respLen];
            n = readFully(loopIn, resp, 0, respLen);
            if (n < respLen) throw new IOException("short read on CALLBACK_RESULT body");
            bytesReadFromSocket += respLen;

            if (resp.length == 0 || resp[0] != GluTypes.MSG_CALLBACK_RESULT)
                throw new IOException("expected CALLBACK_RESULT (0x03), got " +
                    (resp.length == 0 ? "empty" : resp[0]));

            int payloadLen = (int)(
                (resp[1] & 0xFFL)
              | ((resp[2] & 0xFFL) << 8)
              | ((resp[3] & 0xFFL) << 16)
              | ((resp[4] & 0xFFL) << 24)
            );
            byte[] resultPayload = new byte[payloadLen];
            System.arraycopy(resp, 5, resultPayload, 0, payloadLen);
            return resultPayload;
        }
    }

    /** Override to return this module's name (used as caller id in callbacks). */
    protected String getModuleName() { return "java_module"; }

    // ---- Wire protocol helpers (shared by all modules) -------------------

    protected static long readU32LE(byte[] buf, int[] off) {
        long v = ((buf[off[0]] & 0xFFL))
               | ((buf[off[0]+1] & 0xFFL) << 8)
               | ((buf[off[0]+2] & 0xFFL) << 16)
               | ((buf[off[0]+3] & 0xFFL) << 24);
        off[0] += 4;
        return v;
    }

    protected static long readU64LE(byte[] buf, int[] off) {
        long v = 0;
        for (int i = 0; i < 8; i++)
            v |= ((buf[off[0]+i] & 0xFFL) << (i * 8));
        off[0] += 8;
        return v;
    }

    protected static byte[] readLenBytes(byte[] buf, int[] off) {
        int len = (int) readU32LE(buf, off);
        byte[] out = new byte[len];
        System.arraycopy(buf, off[0], out, 0, len);
        off[0] += len;
        return out;
    }

    protected static void writeU32LE(OutputStream out, int v) throws IOException {
        out.write(v & 0xFF);
        out.write((v >> 8) & 0xFF);
        out.write((v >> 16) & 0xFF);
        out.write((v >> 24) & 0xFF);
    }

    protected static void writeU64LE(OutputStream out, long v) throws IOException {
        for (int i = 0; i < 8; i++)
            out.write((int) ((v >> (i * 8)) & 0xFF));
    }

    protected static void writeLenBytes(OutputStream out, byte[] data) throws IOException {
        writeU32LE(out, data.length);
        out.write(data);
    }

    protected static void writeLenStr(OutputStream out, String s) throws IOException {
        writeLenBytes(out, s.getBytes("UTF-8"));
    }

    protected static int readFully(InputStream in, byte[] buf, int off, int len) throws IOException {
        int total = 0;
        while (total < len) {
            int n = in.read(buf, off + total, len - total);
            if (n < 0) return total == 0 ? -1 : total;
            total += n;
        }
        return total;
    }
}
