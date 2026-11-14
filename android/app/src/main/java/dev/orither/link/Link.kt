package dev.orither.link

import android.content.Context
import android.os.Build
import android.os.ParcelFileDescriptor
import android.provider.Settings
import dev.orither.feature.Features
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull

/** This phone's connection to its paired computers. */
object Link {
    private val json = Json { ignoreUnknownKeys = true }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val paired = MutableStateFlow<List<Peer>>(emptyList())
    private val started = MutableStateFlow(false)

    /** Paired computers, kept up to date. */
    val peers: StateFlow<List<Peer>> = paired

    /** Whether [peers] means anything yet. */
    val ready: StateFlow<Boolean> = started

    /** The computer features should talk to when nothing more specific is known. */
    val activePeer: Peer? get() = paired.value.firstOrNull { it.online }

    /** Brings the link online. Later calls return right away. */
    fun start(context: Context) {
        val app = context.applicationContext
        Native.start(app, app.filesDir.resolve("link").path, deviceName(app)) { event ->
            handle(json.decodeFromString<Update>(event))
        }
        refresh()
        started.value = true
    }

    fun invite(): Invite = json.decodeFromString(Native.invite())

    fun cancelInvite() = Native.cancelInvite()

    /**
     * Redeems a pairing code or link. A pairing link can start the app, and arrive before the
     * service has brought the link online, so it waits for that first.
     */
    suspend fun join(token: String): Peer {
        started.first { it }
        return json.decodeFromString(awaiting { Native.join(token, it) })
    }

    fun unpair(peer: String) = Native.unpair(peer)

    fun networkChanged() = Native.networkChanged()

    suspend fun call(peer: String, feature: String, method: String, body: JsonElement = JsonNull): JsonElement =
        json.parseToJsonElement(awaiting { Native.call(peer, feature, method, body.toString(), it) })

    fun notify(peer: String, feature: String, method: String, body: JsonElement = JsonNull) =
        Native.notify(peer, feature, method, body.toString())

    /** Tells every connected computer, skipping those that went away. */
    fun broadcast(feature: String, method: String, body: JsonElement = JsonNull) {
        paired.value.filter { it.online }.forEach { peer ->
            runCatching { notify(peer.id, feature, method, body) }
        }
    }

    suspend fun open(peer: String, feature: String, method: String, body: JsonElement = JsonNull): LinkStream =
        LinkStream(awaiting { Native.open(peer, feature, method, body.toString(), it) }.toLong())

    /** Offers a connected socket as a way to reach a computer. Takes ownership of [socket]. */
    suspend fun addNearby(socket: ParcelFileDescriptor): String =
        json.decodeFromString(awaiting { Native.addNearby(socket.detachFd(), it) })

    private fun handle(update: Update) {
        when (update) {
            is Update.Paired, is Update.Unpaired -> refresh()
            is Update.Connected -> {
                refresh()
                Features.connected(update.peer)
            }
            is Update.Disconnected -> {
                refresh()
                Features.disconnected(update.peer)
            }
            is Update.Route -> {
                refresh()
                Features.route(update.peer, update.route)
            }
            is Update.Notice -> scope.launch {
                Features.notice(update.feature, update.method, update.body, update.peer)
            }
            is Update.Call -> scope.launch {
                runCatching { Features.call(update.feature, update.method, update.body, update.peer) }
                    .onSuccess { Native.respond(update.handle, true, it.toString()) }
                    .onFailure { Native.respond(update.handle, false, it.message ?: "failed") }
            }
            is Update.Stream -> scope.launch {
                val stream = LinkStream(update.handle)
                runCatching {
                    Features.stream(update.feature, update.method, update.body, stream, update.peer)
                }.onFailure { stream.close() }
            }
        }
    }

    private fun refresh() {
        paired.value = json.decodeFromString(Native.peers())
    }

    private fun deviceName(context: Context): String =
        Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME) ?: Build.MODEL

    private suspend fun awaiting(start: (Native.Reply) -> Unit): String =
        suspendCancellableCoroutine { continuation ->
            start { result, error ->
                if (result != null) {
                    continuation.resume(result)
                } else {
                    continuation.resumeWithException(LinkException(error.orEmpty()))
                }
            }
        }
}
