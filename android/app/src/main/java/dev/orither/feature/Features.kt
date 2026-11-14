package dev.orither.feature

import android.content.Context
import dev.orither.engine.EngineClient
import dev.orither.feature.apps.Apps
import dev.orither.feature.calls.Calls
import dev.orither.feature.clipboard.Clipboard
import dev.orither.feature.files.Files
import dev.orither.feature.gallery.Gallery
import dev.orither.feature.hardware.Speaker
import dev.orither.feature.helper.Helper
import dev.orither.feature.nearby.Nearby
import dev.orither.feature.notifications.Notifications
import dev.orither.feature.screen.Screen
import dev.orither.feature.sms.Messages
import dev.orither.feature.system.About
import dev.orither.feature.whatsapp.WhatsApp
import dev.orither.link.LinkStream
import dev.orither.link.Peer
import kotlinx.serialization.json.JsonElement

/** Every feature of the app, and the routing of messages to them. */
object Features {
    private var installed: Map<String, Feature> = emptyMap()

    lateinit var engine: EngineClient
        private set

    /** Everything the user may want to allow, in the order to show it. */
    val access: List<Access> get() = installed.values.mapNotNull { it.access }

    fun install(context: Context) {
        engine = EngineClient(context)
        installed = listOf(
            Echo(),
            Notifications(context, engine),
            Messages(engine),
            Calls(context, engine),
            WhatsApp(context, engine),
            Gallery(context),
            Files(context),
            Clipboard(context, engine),
            Nearby(context, engine),
            About(context),
            Apps(context, engine),
            Speaker(),
            Screen(context, engine),
            Helper(context, engine),
        ).associateBy { it.id }
    }

    suspend fun call(feature: String, method: String, body: JsonElement, peer: String): JsonElement =
        find(feature).call(method, body, peer)

    suspend fun notice(feature: String, method: String, body: JsonElement, peer: String) =
        find(feature).notice(method, body, peer)

    suspend fun stream(feature: String, method: String, body: JsonElement, stream: LinkStream, peer: String) =
        find(feature).stream(method, body, stream, peer)

    fun connected(peer: Peer) = installed.values.forEach { it.connected(peer) }

    fun disconnected(peer: String) = installed.values.forEach { it.disconnected(peer) }

    fun route(peer: String, route: String) = installed.values.forEach { it.route(peer, route) }

    fun allowed() = installed.values.forEach { it.allowed() }

    /** Whether a computer can start the engine again without a cable. */
    fun recovers(context: Context) = installed.values.filterIsInstance<Helper>().any { it.access.isGranted(context) }

    private fun find(feature: String): Feature =
        installed[feature] ?: throw UnsupportedOperationException("unknown feature $feature")
}
