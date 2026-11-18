package dev.orither.feature.screen

import android.content.Context
import android.widget.Toast
import dev.orither.R
import dev.orither.engine.EngineClient
import dev.orither.feature.Feature
import dev.orither.feature.number
import dev.orither.feature.text
import dev.orither.link.LinkStream
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/** Screen, audio and camera capture, carried from the engine to the computer. */
internal class Screen(private val context: Context, private val engine: EngineClient) : Feature {
    override val id = "screen"
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val sessions = ConcurrentHashMap<String, String>()

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement = when (method) {
        "start" -> {
            val options = body.jsonObject.getValue("options").jsonArray.map { it.jsonPrimitive.content }
            val channels = body.jsonObject.getValue("channels").jsonPrimitive.int
            val session = engine.capture(options, channels)
            sessions[session] = peer
            HandoffTile.used(context, peer)
            JsonPrimitive(session)
        }
        "stop" -> {
            val session = body.jsonObject.getValue("session").jsonPrimitive.content
            sessions.remove(session)
            engine.stop(session)
            JsonPrimitive(true)
        }
        "foreground" -> front()
        "touch" -> Json.parseToJsonElement(engine.touch().toString())
        "move" -> {
            val session = body.text("session")
            check(sessions[session] == peer) { "no capture $session for this computer" }
            engine.move(session, body.number("task"))
            JsonPrimitive(true)
        }
        else -> super.call(method, body, peer)
    }

    /**
     * The app the user has in front, for a computer to take over after the user
     * asked for that here. Null when there is none, which the phone says itself.
     */
    private suspend fun front(): JsonElement {
        runCatching { engine.collapseShade() }
        val app = engine.foreground()
        if (app == null) {
            withContext(Dispatchers.Main) { Toast.makeText(context, R.string.handoff_nothing, Toast.LENGTH_SHORT).show() }
            return JsonNull
        }
        val packages = context.packageManager
        val label = runCatching { packages.getApplicationLabel(packages.getApplicationInfo(app.packageName, 0)) }
        return buildJsonObject {
            put("task", app.task)
            put("package", app.packageName)
            put("label", label.getOrNull()?.toString() ?: app.packageName)
        }
    }

    override suspend fun stream(method: String, body: JsonElement, stream: LinkStream, peer: String) {
        val session = body.jsonObject.getValue("session").jsonPrimitive.content
        val channel = body.jsonObject.getValue("channel").jsonPrimitive.int
        val paced = body.jsonObject["paced"]?.jsonPrimitive?.booleanOrNull == true
        val socket = engine.attach(session, channel)
        if (paced) stream.attachVideo(socket) else stream.attach(socket)
    }

    /** A capture has no reason to outlive the computer that asked for it. */
    override fun disconnected(peer: String) {
        val ended = sessions.filterValues { it == peer }.keys
        sessions.keys.removeAll(ended)
        scope.launch { ended.forEach { runCatching { engine.stop(it) } } }
    }
}
