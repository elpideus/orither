package dev.orither.feature.sms

import android.Manifest
import android.content.BroadcastReceiver
import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.database.Cursor
import android.net.Uri
import android.provider.ContactsContract.PhoneLookup
import android.provider.Telephony.Sms
import dev.orither.R
import dev.orither.feature.Access
import dev.orither.feature.Feature
import dev.orither.feature.body
import dev.orither.feature.number
import dev.orither.link.Link
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

private const val MESSAGES_SHOWN = 500

@Serializable
private data class Thread(
    val id: Int,
    val address: String,
    val name: String?,
    val snippet: String,
    val date: Long,
    val unread: Int,
)

@Serializable
private data class Message(val id: Long, val body: String, val date: Long, val incoming: Boolean)

@Serializable
private data class Received(val name: String?, val address: String, val body: String)

/** The phone's text messages, read-only. */
internal class Messages(private val context: Context) : Feature {
    override val id = "sms"
    override val access = Access.Permissions(
        R.string.access_messages,
        R.string.access_messages_about,
        R.drawable.ic_message,
        listOf(Manifest.permission.READ_SMS, Manifest.permission.RECEIVE_SMS, Manifest.permission.READ_CONTACTS),
    )

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement = withContext(Dispatchers.IO) {
        when (method) {
            "threads" -> body(threads())
            "messages" -> body(messages(body.number("thread")))
            else -> throw UnsupportedOperationException("sms has no method $method")
        }
    }

    private fun threads(): List<Thread> {
        val columns = arrayOf(Sms.THREAD_ID, Sms.ADDRESS, Sms.BODY, Sms.DATE, Sms.TYPE, Sms.READ)
        val threads = LinkedHashMap<Int, Thread>()
        context.contentResolver.query(Sms.CONTENT_URI, columns, null, null, "${Sms.DATE} DESC")?.use { rows ->
            while (rows.moveToNext()) {
                val id = rows.getInt(0)
                val unread = if (rows.getInt(4) == Sms.MESSAGE_TYPE_INBOX && rows.getInt(5) == 0) 1 else 0
                val known = threads[id]
                threads[id] = known?.copy(unread = known.unread + unread) ?: Thread(
                    id = id,
                    address = rows.text(1),
                    name = null,
                    snippet = rows.text(2),
                    date = rows.getLong(3),
                    unread = unread,
                )
            }
        }
        val names = HashMap<String, String?>()
        return threads.values.map {
            it.copy(name = names.getOrPut(it.address) { contactName(context.contentResolver, it.address) })
        }
    }

    private fun messages(thread: Int): List<Message> {
        val columns = arrayOf(Sms._ID, Sms.BODY, Sms.DATE, Sms.TYPE)
        val rows = context.contentResolver.query(
            Sms.CONTENT_URI, columns, "${Sms.THREAD_ID} = ?", arrayOf(thread.toString()),
            "${Sms.DATE} DESC LIMIT $MESSAGES_SHOWN",
        ) ?: return emptyList()
        return rows.use {
            buildList {
                while (it.moveToNext()) {
                    add(Message(it.getLong(0), it.text(1), it.getLong(2), it.getInt(3) == Sms.MESSAGE_TYPE_INBOX))
                }
            }.reversed()
        }
    }
}

private fun Cursor.text(column: Int): String = getString(column).orEmpty()

private fun contactName(resolver: ContentResolver, address: String): String? {
    val uri = Uri.withAppendedPath(PhoneLookup.CONTENT_FILTER_URI, Uri.encode(address))
    return runCatching {
        resolver.query(uri, arrayOf(PhoneLookup.DISPLAY_NAME), null, null, null)?.use { rows ->
            if (rows.moveToFirst()) rows.getString(0) else null
        }
    }.getOrNull()
}

/** Tells the computer about each text message as it arrives. */
class SmsReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val parts = Sms.Intents.getMessagesFromIntent(intent).orEmpty()
        val address = parts.firstOrNull()?.displayOriginatingAddress ?: return
        val text = parts.joinToString("") { it.displayMessageBody.orEmpty() }
        val name = contactName(context.contentResolver, address)
        Link.broadcast("sms", "received", body(Received(name, address, text)))
    }
}
