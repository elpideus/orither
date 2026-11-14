package dev.orither.engine

import android.provider.Telephony.Sms
import org.json.JSONArray
import org.json.JSONObject

private const val ROW = "Row: "

/**
 * The phone's text messages, read with the shell user's rights. An app that asks for SMS access
 * is blocked by Play Protect when installed from outside Google Play, so the app itself has none
 * and reads its texts through the engine.
 *
 * The rows come from Android's own `content query` command, which runs as the shell: a content
 * resolver in this process names the system's package as the caller, which Android refuses from
 * the shell's uid. Android does not report changes to the engine either (it is no app process it
 * knows), so the app asks for new texts when the phone's SMS app posts a notification. The app adds
 * contact names, with its own contacts permission.
 */
internal class Texts {
    /** Every conversation, newest first. */
    fun threads(): JSONArray {
        val threads = LinkedHashMap<String, JSONObject>()
        for (row in query("content://sms", listOf("thread_id", "address", "date", "type", "read", "body"), sort = "date DESC")) {
            val id = row["thread_id"] ?: continue
            val unread = if (row["type"] == Sms.MESSAGE_TYPE_INBOX.toString() && row["read"] == "0") 1 else 0
            val known = threads[id]
            if (known != null) {
                known.put("unread", known.getInt("unread") + unread)
            } else {
                threads[id] = JSONObject()
                    .put("id", id.toInt())
                    .put("address", row["address"].orEmpty())
                    .put("snippet", row["body"].orEmpty())
                    .put("date", row["date"]?.toLongOrNull() ?: 0)
                    .put("unread", unread)
            }
        }
        return JSONArray(threads.values)
    }

    /** The latest [limit] messages of a conversation, oldest first. */
    fun messages(thread: Int, limit: Int): JSONArray {
        val rows = query("content://sms", listOf("_id", "date", "type", "body"), where = "thread_id=$thread", sort = "date DESC LIMIT $limit")
        return JSONArray(rows.reversed().map { row ->
            JSONObject()
                .put("id", row["_id"]?.toLongOrNull() ?: 0)
                .put("body", row["body"].orEmpty())
                .put("date", row["date"]?.toLongOrNull() ?: 0)
                .put("incoming", row["type"] == Sms.MESSAGE_TYPE_INBOX.toString())
        })
    }

    /** The id of the newest text, received or sent, or 0 when there is none. */
    fun newest(): Long =
        query("content://sms", listOf("_id"), sort = "_id DESC LIMIT 1").firstOrNull()?.get("_id")?.toLongOrNull() ?: 0

    /** The incoming texts stored after the one with id [after]. */
    fun since(after: Long): JSONArray = JSONArray(
        query("content://sms/inbox", listOf("_id", "address", "body"), where = "_id>$after", sort = "_id ASC").map { row ->
            JSONObject().put("id", row["_id"]?.toLongOrNull() ?: 0).put("address", row["address"].orEmpty()).put("body", row["body"].orEmpty())
        },
    )
}

/**
 * Runs `content query` and reads its rows. The last of [columns] takes the rest of each row, line
 * breaks too, so a free-text column such as a message body goes last.
 */
internal fun query(uri: String, columns: List<String>, where: String? = null, sort: String? = null): List<Map<String, String?>> {
    val command = mutableListOf("content", "query", "--uri", uri, "--projection", columns.joinToString(":"))
    if (where != null) command += listOf("--where", where)
    if (sort != null) command += listOf("--sort", sort)
    val process = ProcessBuilder(command).redirectErrorStream(true).start()
    val said = process.inputStream.bufferedReader().readText()
    process.waitFor()
    return parseRows(said, columns)
}

/** The rows `content query` printed. Anything else it printed is an error. */
internal fun parseRows(said: String, columns: List<String>): List<Map<String, String?>> {
    if (said.isBlank() || said.trim() == "No result found.") return emptyList()
    check(said.startsWith(ROW)) { said.lineSequence().first().take(200) }
    val records = Regex("""(?m)^Row: \d+ """).split(said).drop(1)
    return records.map { record ->
        val text = record.removeSuffix("\n")
        val row = HashMap<String, String?>()
        var at = 0
        for ((index, column) in columns.withIndex()) {
            val start = at + column.length + 1
            val end = if (index == columns.lastIndex) text.length else text.indexOf(", ${columns[index + 1]}=", start).takeIf { it >= 0 } ?: text.length
            val value = if (start <= end) text.substring(start, end) else ""
            row[column] = value.takeUnless { it == "NULL" }
            at = end + 2
        }
        row
    }
}
