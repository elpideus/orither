package dev.orither.engine

import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.io.OutputStream
import org.json.JSONObject

/** How the app and the engine talk: one JSON line each way, then maybe raw bytes. */
internal object Protocol {
    const val PORT = 47183
    const val VERSION = 8
    const val SCRCPY_ASSET = "scrcpy-server.jar"

    fun write(output: OutputStream, message: JSONObject) {
        output.write(message.toString().toByteArray())
        output.write('\n'.code)
        output.flush()
    }

    /**
     * Reads byte by byte so that nothing past the line gets buffered away, and decodes the line
     * as UTF-8 at the end, since a character may take several bytes.
     */
    fun read(input: InputStream): JSONObject {
        val line = ByteArrayOutputStream()
        while (true) {
            val byte = input.read()
            check(byte >= 0) { "connection closed" }
            if (byte == '\n'.code) return JSONObject(line.toString(Charsets.UTF_8.name()))
            line.write(byte)
        }
    }
}
