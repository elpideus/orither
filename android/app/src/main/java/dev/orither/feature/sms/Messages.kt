package dev.orither.feature.sms

import android.Manifest
import android.content.ContentResolver
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.ContactsContract.PhoneLookup
import android.util.Log
import dev.orither.engine.EngineClient
import dev.orither.feature.Feature
import dev.orither.feature.number
import dev.orither.link.Link
import dev.orither.link.Peer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import org.json.JSONObject

private const val TAG = "Orither"

/** How often to look for new texts anyway, for a phone whose SMS app posts no notification Orither sees. */
private const val CHECK_EVERY_MS = 30_000L

/** The SMS app posts its notification as it stores the text, so the check waits a moment. */
private const val STORED_WITHIN_MS = 1_500L

private const val NEEDS_HELPER =
    "Messages are read by Orither's helper on your phone, which is not running. Connect your phone to your computer with a USB cable once."

/**
 * The phone's text messages, read-only. The app asks for no SMS access: Play Protect blocks any app
 * from outside Google Play that does, so the engine reads them with the shell user's rights. Android
 * tells the engine nothing when a text arrives, so the app asks it for new ones when the phone's SMS
 * app posts a notification, and every half minute besides.
 */
internal class Messages(private val context: Context, private val engine: EngineClient) : Feature {
    override val id = "sms"
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val connected = mutableSetOf<String>()
    private var checking: Job? = null
    private val lock = Mutex()

    /** The newest text already told about, once known. */
    private var last: Long? = null

    init {
        current = this
    }

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement {
        if (method != "threads" && method != "messages") throw UnsupportedOperationException("sms has no method $method")
        val read = runCatching {
            if (method == "threads") engine.textThreads() else engine.texts(body.number("thread"))
        }
        val rows = read.getOrElse { if (engine.isRunning()) throw it else error(NEEDS_HELPER) }
        if (method == "threads") {
            val names = HashMap<String, String?>()
            for (index in 0 until rows.length()) {
                val thread = rows.getJSONObject(index)
                val address = thread.getString("address")
                thread.put("name", names.getOrPut(address) { contactName(address) } ?: JSONObject.NULL)
            }
        }
        return Json.parseToJsonElement(rows.toString())
    }

    override fun connected(peer: Peer) {
        synchronized(connected) {
            connected += peer.id
            if (checking?.isActive == true) return
            checking = scope.launch {
                while (isActive) {
                    check()
                    delay(CHECK_EVERY_MS)
                }
            }
        }
    }

    override fun disconnected(peer: String) {
        synchronized(connected) {
            connected -= peer
            if (connected.isEmpty()) checking?.cancel()
        }
    }

    /** Tells every connected computer about the texts that came since the last check. */
    private suspend fun check() = lock.withLock {
        runCatching {
            if (!engine.isRunning()) return@runCatching
            val seen = last
            if (seen == null) {
                last = engine.newestText()
                return@runCatching
            }
            val texts = engine.textsSince(seen)
            for (index in 0 until texts.length()) {
                val text = texts.getJSONObject(index)
                last = maxOf(last ?: 0, text.getLong("id"))
                text.remove("id")
                text.put("name", contactName(text.getString("address")) ?: JSONObject.NULL)
                Link.broadcast(id, "received", Json.parseToJsonElement(text.toString()))
            }
        }.onFailure { Log.i(TAG, "checking for texts: ${it.message}") }
    }

    private fun soon() {
        if (synchronized(connected) { connected.isEmpty() }) return
        scope.launch {
            delay(STORED_WITHIN_MS)
            check()
        }
    }

    /** The contact's name for [address], when the app may read contacts and the phone has one. */
    private fun contactName(address: String): String? {
        if (context.checkSelfPermission(Manifest.permission.READ_CONTACTS) != PackageManager.PERMISSION_GRANTED) return null
        return lookUp(context.contentResolver, address)
    }

    companion object {
        @Volatile private var current: Messages? = null

        /** The phone's SMS app posted a notification, which is how a new text shows itself. */
        fun textMayHaveArrived() {
            current?.soon()
        }
    }
}

private fun lookUp(resolver: ContentResolver, address: String): String? = runCatching {
    val uri = Uri.withAppendedPath(PhoneLookup.CONTENT_FILTER_URI, Uri.encode(address))
    resolver.query(uri, arrayOf(PhoneLookup.DISPLAY_NAME), null, null, null)?.use { if (it.moveToFirst()) it.getString(0) else null }
}.getOrNull()
