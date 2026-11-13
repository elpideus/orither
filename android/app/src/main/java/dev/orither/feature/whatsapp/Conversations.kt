package dev.orither.feature.whatsapp

import android.app.Notification
import android.content.Context
import android.service.notification.StatusBarNotification
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

private val COUNT_SUFFIX = Regex("""\s*\(\d+[^)]*\)$""")

/** A chat, as WhatsApp's latest notification from it showed it. */
@Serializable
internal data class Conversation(
    val id: String,
    val title: String,
    /** Messages from others since the user last wrote, while the notification is up. */
    val unread: Int,
    val last: String,
    val time: Long,
)

/**
 * A chat's name. Android titles a chat that has no name of its own after whoever
 * wrote last, which is the user once they reply, so the name comes from the other
 * side. [senders] go in the order the messages came, with null for the user.
 */
internal fun chatName(title: String?, senders: List<String?>): String? =
    (title ?: senders.lastOrNull { it != null })?.replace(COUNT_SUFFIX, "")

/** How many messages wait for an answer: those from others since the user wrote. */
internal fun waiting(senders: List<String?>): Int = senders.takeLastWhile { it != null }.size

/** WhatsApp's own id for the chat a notification is about. */
internal fun chatOf(posted: StatusBarNotification): String? = posted.notification.shortcutId ?: posted.tag

/**
 * WhatsApp's recent chats, as its notifications last showed them, kept in the app's
 * private files. They start when Orither gets notification access.
 */
internal class Conversations private constructor(context: Context) {
    private val file = context.applicationContext.filesDir.resolve("whatsapp.json")
    private val json = Json { ignoreUnknownKeys = true }
    private val updates = MutableSharedFlow<Unit>(extraBufferCapacity = 1)
    private val chats: MutableMap<String, Conversation> = runCatching {
        json.decodeFromString<List<Conversation>>(file.readText()).associateBy { it.id }
    }.getOrDefault(emptyMap()).toMutableMap()

    /** Emits after every change. */
    val changes: SharedFlow<Unit> = updates

    @Synchronized
    fun all(): List<Conversation> = chats.values.sortedByDescending { it.time }

    /** Starts over from the notifications up now. A chat that has none has been read. */
    @Synchronized
    fun showing(active: List<StatusBarNotification>) {
        val up = active.mapNotNullTo(HashSet(), ::chatOf)
        chats.replaceAll { id, chat -> if (id in up) chat else chat.copy(unread = 0) }
        active.forEach(::record)
        changed()
    }

    /** Takes in whatever a WhatsApp notification shows. */
    @Synchronized
    fun record(posted: StatusBarNotification) {
        val notification = posted.notification
        if (notification.flags and Notification.FLAG_GROUP_SUMMARY != 0) return
        val style = NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(notification) ?: return
        val id = chatOf(posted) ?: return
        val messages = style.messages.filter { it.text != null }
        val last = messages.lastOrNull() ?: return
        val me = style.user.name?.toString()
        val senders = messages.map { message -> message.person?.name?.toString()?.takeUnless { it == me } }
        val title = chatName(style.conversationTitle?.toString(), senders)
            ?: chats[id]?.title
            ?: notification.extras.getCharSequence(Notification.EXTRA_TITLE)?.toString()
            ?: return
        val sender = senders.last().takeIf { style.isGroupConversation }
        chats[id] = Conversation(
            id = id,
            title = title,
            unread = waiting(senders),
            last = listOfNotNull(sender, last.text).joinToString(": "),
            time = last.timestamp,
        )
        changed()
    }

    /** Clears a chat's count when its notification goes, as it does once the chat is read anywhere. */
    @Synchronized
    fun seen(removed: StatusBarNotification) {
        val id = chatOf(removed) ?: return
        val chat = chats[id]?.takeIf { it.unread > 0 } ?: return
        chats[id] = chat.copy(unread = 0)
        changed()
    }

    private fun changed() {
        file.writeText(json.encodeToString(chats.values.toList()))
        updates.tryEmit(Unit)
    }

    companion object {
        @Volatile private var instance: Conversations? = null

        fun of(context: Context): Conversations = instance ?: synchronized(this) {
            instance ?: Conversations(context).also { instance = it }
        }
    }
}
