package dev.orither.engine

import android.content.Context
import android.database.ContentObserver
import android.database.Cursor
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.ContactsContract.PhoneLookup
import android.provider.Telephony.Sms
import java.net.Socket
import org.json.JSONArray
import org.json.JSONObject

/**
 * The phone's text messages, read with the shell user's rights. An app that asks for SMS access
 * is blocked by Play Protect when installed from outside Google Play, so the app itself has none
 * and reads its texts through the engine.
 */
internal class Texts(shell: Context) {
    private val resolver = shell.contentResolver

    /** Every conversation, newest first, with the contact's name where the phone knows it. */
    fun threads(): JSONArray {
        val columns = arrayOf(Sms.THREAD_ID, Sms.ADDRESS, Sms.BODY, Sms.DATE, Sms.TYPE, Sms.READ)
        val threads = LinkedHashMap<Int, JSONObject>()
        resolver.query(Sms.CONTENT_URI, columns, null, null, "${Sms.DATE} DESC")?.use { rows ->
            while (rows.moveToNext()) {
                val id = rows.getInt(0)
                val unread = if (rows.getInt(4) == Sms.MESSAGE_TYPE_INBOX && rows.getInt(5) == 0) 1 else 0
                val known = threads[id]
                if (known != null) {
                    known.put("unread", known.getInt("unread") + unread)
                } else {
                    threads[id] = JSONObject()
                        .put("id", id)
                        .put("address", rows.text(1))
                        .put("snippet", rows.text(2))
                        .put("date", rows.getLong(3))
                        .put("unread", unread)
                }
            }
        }
        val names = HashMap<String, String?>()
        return JSONArray(threads.values.map { thread ->
            val address = thread.getString("address")
            thread.put("name", names.getOrPut(address) { name(address) } ?: JSONObject.NULL)
        })
    }

    /** The latest [limit] messages of a conversation, oldest first. */
    fun messages(thread: Int, limit: Int): JSONArray {
        val columns = arrayOf(Sms._ID, Sms.BODY, Sms.DATE, Sms.TYPE)
        val rows = resolver.query(Sms.CONTENT_URI, columns, "${Sms.THREAD_ID} = ?", arrayOf(thread.toString()), "${Sms.DATE} DESC LIMIT $limit")
            ?: return JSONArray()
        val list = rows.use {
            buildList {
                while (it.moveToNext()) {
                    add(JSONObject().put("id", it.getLong(0)).put("body", it.text(1)).put("date", it.getLong(2)).put("incoming", it.getInt(3) == Sms.MESSAGE_TYPE_INBOX))
                }
            }
        }
        return JSONArray(list.reversed())
    }

    /** Writes one JSON line to [client] for each text that arrives, until it hangs up. */
    fun follow(client: Socket) {
        val output = client.getOutputStream()
        var last = newest()
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) {
                runCatching {
                    for (text in since(last)) {
                        last = maxOf(last, text.getLong("id"))
                        Protocol.write(output, text)
                    }
                }
            }
        }
        resolver.registerContentObserver(Sms.CONTENT_URI, true, observer)
        try {
            client.getInputStream().readBytes()
        } finally {
            resolver.unregisterContentObserver(observer)
        }
    }

    private fun newest(): Long =
        resolver.query(Sms.CONTENT_URI, arrayOf(Sms._ID), null, null, "${Sms._ID} DESC LIMIT 1")?.use { if (it.moveToFirst()) it.getLong(0) else 0 } ?: 0

    /** The incoming texts stored after the one with id [after]. */
    private fun since(after: Long): List<JSONObject> {
        val columns = arrayOf(Sms._ID, Sms.ADDRESS, Sms.BODY)
        val where = "${Sms._ID} > ? AND ${Sms.TYPE} = ${Sms.MESSAGE_TYPE_INBOX}"
        val rows = resolver.query(Sms.CONTENT_URI, columns, where, arrayOf(after.toString()), "${Sms._ID} ASC") ?: return emptyList()
        return rows.use {
            buildList {
                while (it.moveToNext()) {
                    val address = it.text(1)
                    add(JSONObject().put("id", it.getLong(0)).put("address", address).put("body", it.text(2)).put("name", name(address) ?: JSONObject.NULL))
                }
            }
        }
    }

    private fun name(address: String): String? = runCatching {
        val uri = Uri.withAppendedPath(PhoneLookup.CONTENT_FILTER_URI, Uri.encode(address))
        resolver.query(uri, arrayOf(PhoneLookup.DISPLAY_NAME), null, null, null)?.use { if (it.moveToFirst()) it.getString(0) else null }
    }.getOrNull()
}

private fun Cursor.text(column: Int): String = getString(column).orEmpty()
