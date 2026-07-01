package glucore;

/**
 * GluTypes — Java representations of the GluCore type system.
 *
 * Tags: 0=Int, 1=Float, 2=String, 3=Buffer, 4=Handle, 5=Void.
 *
 * These are the wire-protocol constants. Every Java PROCESS module uses
 * these when encoding/decoding CALL and RESULT messages.
 */
public final class GluTypes {
    private GluTypes() {}  // no instances

    public static final int INT    = 0;
    public static final int FLOAT  = 1;
    public static final int STRING = 2;
    public static final int BUFFER = 3;
    public static final int HANDLE = 4;
    public static final int VOID   = 5;

    /** Wire-protocol message types. */
    public static final int MSG_CALL             = 0x00;
    public static final int MSG_RESULT           = 0x01;
    public static final int MSG_CALLBACK_CALL    = 0x02;
    public static final int MSG_CALLBACK_RESULT  = 0x03;

    /** Convert a tag to a human-readable name. */
    public static String tagName(int tag) {
        return switch (tag) {
            case INT -> "Int";
            case FLOAT -> "Float";
            case STRING -> "String";
            case BUFFER -> "Buffer";
            case HANDLE -> "Handle";
            case VOID -> "Void";
            default -> "<unknown:" + tag + ">";
        };
    }
}
