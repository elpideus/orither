package dev.orither.engine

import android.content.ClipboardManager
import android.content.Context
import java.net.Socket
import org.json.JSONArray
import org.json.JSONObject

private val NAME_AND_SIZE = Regex("_display_name=(.*), _size=(\\d+)")

/** Reports clipboard changes, which only the shell user may read in the background. */
internal class ClipboardWatch(shell: Context) {
    private val clipboard = shell.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    /** Writes one JSON line to [client] for each change, until it hangs up. */
    fun follow(client: Socket) {
        val output = client.getOutputStream()
        val listener = ClipboardManager.OnPrimaryClipChangedListener {
            runCatching { Protocol.write(output, describe()) }
        }
        clipboard.addPrimaryClipChangedListener(listener)
        try {
            client.getInputStream().readBytes()
        } finally {
            clipboard.removePrimaryClipChangedListener(listener)
        }
    }

    private fun describe(): JSONObject {
        val message = JSONObject()
        val clip = clipboard.primaryClip ?: return message
        val files = JSONArray()
        for (index in 0 until clip.itemCount) {
            val item = clip.getItemAt(index)
            val uri = item.uri
            when {
                uri != null -> files.put(file(uri.toString(), clip.description.getMimeType(0)))
                item.text != null -> message.put("text", item.text.toString())
            }
        }
        return message.put("files", files)
    }

    private fun file(uri: String, mime: String): JSONObject {
        val row = ProcessBuilder("content", "query", "--uri", uri, "--projection", "_display_name:_size")
            .redirectErrorStream(true)
            .start()
            .inputStream
            .bufferedReader()
            .readText()
        val match = NAME_AND_SIZE.find(row)
        return JSONObject()
            .put("uri", uri)
            .put("mime", mime)
            .put("name", match?.groupValues?.get(1) ?: uri.substringAfterLast('/'))
            .put("size", match?.groupValues?.get(2)?.toLong() ?: 0)
    }
}
