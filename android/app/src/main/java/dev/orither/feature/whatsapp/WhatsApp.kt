package dev.orither.feature.whatsapp

import android.content.Context
import android.content.Intent
import android.provider.Settings
import android.provider.Telephony
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import dev.orither.R
import dev.orither.engine.EngineClient
import dev.orither.feature.Access
import dev.orither.feature.Feature
import dev.orither.feature.body
import dev.orither.feature.calls.Ringing
import dev.orither.feature.notifications.LISTENER
import dev.orither.feature.notifications.Shade
import dev.orither.feature.notifications.listening
import dev.orither.feature.sms.Messages
import dev.orither.link.Link
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive

private val APPS = setOf("com.whatsapp", "com.whatsapp.w4b")

@Serializable
private data class Overview(val allowed: Boolean, val chats: List<Conversation>)

/**
 * WhatsApp's recent chats, from its notifications, for the computer's home board.
 * The computer runs WhatsApp Web for everything else.
 */
internal class WhatsApp(private val context: Context, private val engine: EngineClient) : Feature {
    override val id = "whatsapp"
    override val access = Access.Special(
        R.string.access_whatsapp,
        R.string.access_whatsapp_about,
        R.drawable.ic_whatsapp,
        isGranted = ::listening,
        settings = { Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS) },
        grants = Access.listenerGrants(LISTENER),
    )
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val conversations = Conversations.of(context)

    init {
        scope.launch { conversations.changes.collect { Link.broadcast(id, "changed") } }
    }

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement = withContext(Dispatchers.IO) {
        when (method) {
            "chats" -> body(Overview(access.isGranted(context), conversations.all()))
            "allow" -> {
                allow()
                JsonPrimitive(true)
            }
            else -> throw UnsupportedOperationException("whatsapp has no method $method")
        }
    }

    /** Sideloaded apps can't flip this switch in settings, so the engine does it when it can. */
    private suspend fun allow() {
        val done = runCatching { engine.grant(access.grants).length() == 0 }.getOrDefault(false)
        if (!done) context.startActivity(access.settings(context).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
}

private fun StatusBarNotification.isWhatsApp() = packageName in APPS

/**
 * Reads the phone's notifications as they come and go, for WhatsApp's feature, for
 * showing them all on a computer, and for the calls apps show in them. It keeps the
 * name it had when only WhatsApp's feature read them, because Android allows a
 * listener by its name, and a new name would take back what users allowed.
 */
class WhatsAppListener : NotificationListenerService() {
    private val conversations by lazy { Conversations.of(this) }

    override fun onListenerConnected() {
        val active = activeNotifications.orEmpty().toList()
        runCatching { conversations.showing(active.filter { it.isWhatsApp() }) }
        Shade.connected(this, active)
        Ringing.connected(this, active)
    }

    override fun onListenerDisconnected() {
        Shade.disconnected(this)
        Ringing.disconnected()
    }

    override fun onNotificationPosted(posted: StatusBarNotification) {
        if (posted.isWhatsApp()) runCatching { conversations.record(posted) }
        // The SMS app's notification is how a new text shows itself to an app without SMS access.
        if (posted.packageName == Telephony.Sms.getDefaultSmsPackage(this)) Messages.textMayHaveArrived()
        Shade.take(this, posted, announce = true)
        Ringing.posted(this, posted)
    }

    override fun onNotificationRemoved(removed: StatusBarNotification) {
        if (removed.isWhatsApp()) runCatching { conversations.seen(removed) }
        Shade.removed(removed.key)
        Ringing.removed(removed.key)
    }
}
