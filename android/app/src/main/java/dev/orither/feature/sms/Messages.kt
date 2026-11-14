package dev.orither.feature.sms

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
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement

/** How often to look for the engine again while a computer is connected and it is not running. */
private const val RETRY_MS = 5_000L

private const val NEEDS_HELPER =
    "Messages are read by Orither's helper on your phone, which is not running. Connect your phone to your computer with a USB cable once."

/**
 * The phone's text messages, read-only. The app asks for no SMS access: Play Protect blocks any app
 * from outside Google Play that does, so the engine reads them with the shell user's rights.
 */
internal class Messages(private val engine: EngineClient) : Feature {
    override val id = "sms"
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val connected = mutableSetOf<String>()
    private var following: Job? = null

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement {
        if (method != "threads" && method != "messages") throw UnsupportedOperationException("sms has no method $method")
        val read = runCatching {
            if (method == "threads") engine.textThreads() else engine.texts(body.number("thread"))
        }
        val rows = read.getOrElse { if (engine.isRunning()) throw it else error(NEEDS_HELPER) }
        return Json.parseToJsonElement(rows.toString())
    }

    /** Tells every computer about each text as it arrives, while any is connected. */
    override fun connected(peer: Peer) {
        synchronized(connected) {
            connected += peer.id
            if (following?.isActive == true) return
            following = scope.launch {
                while (isActive) {
                    runCatching {
                        engine.followTexts { text ->
                            text.remove("id")
                            Link.broadcast(id, "received", Json.parseToJsonElement(text.toString()))
                        }
                    }
                    delay(RETRY_MS)
                }
            }
        }
    }

    override fun disconnected(peer: String) {
        synchronized(connected) {
            connected -= peer
            if (connected.isEmpty()) following?.cancel()
        }
    }
}
