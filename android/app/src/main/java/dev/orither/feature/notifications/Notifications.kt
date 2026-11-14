package dev.orither.feature.notifications

import android.app.ActivityOptions
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.graphics.Bitmap
import android.os.Build
import android.provider.Settings
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.view.Display
import androidx.activity.ComponentActivity
import androidx.core.app.NotificationCompat
import androidx.core.graphics.drawable.toBitmap
import androidx.core.os.BundleCompat
import dev.orither.R
import dev.orither.engine.EngineClient
import dev.orither.feature.Access
import dev.orither.feature.Feature
import dev.orither.feature.body
import dev.orither.feature.text
import dev.orither.feature.textOrNull
import dev.orither.feature.whatsapp.WhatsAppListener
import dev.orither.link.Link
import dev.orither.link.LinkStream
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive

/** The listener behind every feature that reads notifications, as the engine names it. */
internal const val LISTENER = "dev.orither/dev.orither.feature.whatsapp.WhatsAppListener"
private const val OPENER = "dev.orither/dev.orither.feature.notifications.OpenActivity"
private const val ID = "notifications"
private const val EXTRA_KEY = "key"
private const val EXTRA_APP = "app"
/** Where a notification keeps the app that posted it. */
private const val EXTRA_APP_INFO = "android.appInfo"
private const val PICTURE_SIZE = 128
private const val MAX_TEXT = 2000
/** The latest lines of a chat, as many as fit in a pop-up. */
private const val LINES_SHOWN = 3

/** Whether Android passes this app the phone's notifications. */
internal fun listening(context: Context): Boolean =
    context.getSystemService(NotificationManager::class.java)
        .isNotificationListenerAccessGranted(ComponentName(context, WhatsAppListener::class.java))

/** A notification as a computer shows it. */
@Serializable
internal data class Shown(
    val key: String,
    val app: String,
    val label: String,
    val title: String,
    val text: String,
    val time: Long,
    /** Whether to call attention to it. Some apps ask for that only once, not on each update. */
    val alert: Boolean,
    /** Listed without a sound or a pop-up, as on the phone. */
    val quiet: Boolean,
    /** Whether it has a picture of its own, such as the sender's. */
    val picture: Boolean,
    /** The chat it is about, by the app's own id for it, when it is a message. */
    val conversation: String?,
)

/** The phone's notifications on the computer, each opening there where it leads on the phone. */
internal class Notifications(private val context: Context, private val engine: EngineClient) : Feature {
    override val id = ID
    override val access = Access.Special(
        R.string.access_notifications,
        R.string.access_notifications_about,
        R.drawable.ic_bell,
        isGranted = ::listening,
        settings = { Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS) },
        grants = Access.listenerGrants(LISTENER),
    )

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement = when (method) {
        "list" -> body(Shade.all())
        // With a capture's session it opens on that capture's display, and on the phone's own otherwise.
        "open" -> {
            val opener = listOf("-n", OPENER, "--es", EXTRA_KEY, body.text("key"), "--es", EXTRA_APP, body.text("app"))
            check(engine.startActivity(opener, body.textOrNull("session"))) { "Couldn't open the notification on the phone" }
            JsonPrimitive(true)
        }
        "dismiss" -> {
            Shade.dismiss(body.text("key"))
            JsonPrimitive(true)
        }
        else -> super.call(method, body, peer)
    }

    override suspend fun stream(method: String, body: JsonElement, stream: LinkStream, peer: String) {
        val picture = withContext(Dispatchers.IO) {
            if (method == "picture") Shade.picture(context, body.text("key")) else null
        }
        if (picture == null) return stream.close()
        stream.send { picture.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }
}

/**
 * The notifications a computer may show, as the listener sees them: those the user
 * could swipe away. Ongoing ones, such as music or a download, belong to the phone,
 * and so do Orither's own.
 */
internal object Shade {
    private class Kept(val posted: StatusBarNotification, val shown: Shown)

    @Volatile private var listener: NotificationListenerService? = null
    private val kept = ConcurrentHashMap<String, Kept>()
    private val labels = ConcurrentHashMap<String, String>()

    /** What the phone shows now, none of it calling for attention again. */
    fun all(): List<Shown> = kept.values.map { it.shown.copy(alert = false) }.sortedByDescending(Shown::time)

    fun find(key: String): StatusBarNotification? = kept[key]?.posted

    /** Takes a notification away, as a swipe on the phone does. */
    fun dismiss(key: String) {
        checkNotNull(listener) { "Orither can't reach the phone's notifications" }.cancelNotification(key)
    }

    fun picture(context: Context, key: String): Bitmap? {
        val icon = kept[key]?.posted?.notification?.getLargeIcon() ?: return null
        return runCatching { icon.loadDrawable(context)?.toBitmap(PICTURE_SIZE, PICTURE_SIZE) }.getOrNull()
    }

    /** Takes in what is on the phone already, which computers have no need to hear about again. */
    fun connected(listener: NotificationListenerService, active: List<StatusBarNotification>) {
        this.listener = listener
        kept.clear()
        active.forEach { take(listener, it, announce = false) }
    }

    fun disconnected(listener: NotificationListenerService) {
        if (this.listener === listener) this.listener = null
    }

    /** Keeps a notification that was posted or changed, and tells the computers if [announce]. */
    fun take(listener: NotificationListenerService, posted: StatusBarNotification, announce: Boolean) {
        val shown = describe(listener, posted)
        if (shown == null) {
            // An update can take a notification out of what computers show, by making it ongoing.
            removed(posted.key)
            return
        }
        kept[posted.key] = Kept(posted, shown)
        if (announce) Link.broadcast(ID, "posted", body(shown))
    }

    fun removed(key: String) {
        if (kept.remove(key) != null) Link.broadcast(ID, "removed", body(mapOf("key" to key)))
    }

    private fun describe(listener: NotificationListenerService, posted: StatusBarNotification): Shown? {
        val notification = posted.notification
        if (posted.packageName == listener.packageName || !posted.isClearable) return null
        if (notification.flags and Notification.FLAG_GROUP_SUMMARY != 0) return null
        val (title, text) = words(notification)
        if (title.isBlank() && text.isBlank()) return null
        val ranking = NotificationListenerService.Ranking().takeIf { listener.currentRanking?.getRanking(posted.key, it) == true }
        val quiet = ranking != null &&
            (ranking.importance < NotificationManager.IMPORTANCE_DEFAULT || !ranking.matchesInterruptionFilter())
        val again = notification.flags and Notification.FLAG_ONLY_ALERT_ONCE == 0
        return Shown(
            key = posted.key,
            app = posted.packageName,
            label = label(listener, posted),
            title = title.take(MAX_TEXT),
            text = text.take(MAX_TEXT),
            time = posted.postTime,
            alert = again || !kept.containsKey(posted.key),
            quiet = quiet,
            picture = notification.getLargeIcon() != null,
            conversation = notification.shortcutId,
        )
    }

    /** The title and text the phone shows, with a chat's latest lines in place of a summary. */
    private fun words(notification: Notification): Pair<String, String> {
        val extras = notification.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE) ?: extras.getCharSequence(Notification.EXTRA_TITLE_BIG)
        val chat = NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(notification)
        if (chat != null && chat.messages.isNotEmpty()) {
            val lines = chat.messages.takeLast(LINES_SHOWN).mapNotNull { message ->
                val text = message.text ?: return@mapNotNull null
                val sender = message.person?.name?.takeIf { chat.isGroupConversation }
                if (sender != null) "$sender: $text" else text.toString()
            }
            return (chat.conversationTitle ?: title)?.toString().orEmpty() to lines.joinToString("\n")
        }
        val text = extras.getCharSequence(Notification.EXTRA_BIG_TEXT)
            ?: extras.getCharSequenceArray(Notification.EXTRA_TEXT_LINES)?.joinToString("\n")
            ?: extras.getCharSequence(Notification.EXTRA_TEXT)
        return title?.toString().orEmpty() to text?.toString().orEmpty()
    }

    private fun label(context: Context, posted: StatusBarNotification): String = labels.getOrPut(posted.packageName) {
        val packages = context.packageManager
        val info = BundleCompat.getParcelable(posted.notification.extras, EXTRA_APP_INFO, ApplicationInfo::class.java)
            ?: runCatching { packages.getApplicationInfo(posted.packageName, 0) }.getOrNull()
        info?.let { packages.getApplicationLabel(it).toString() } ?: posted.packageName
    }
}

/**
 * Follows a notification where it leads, on the display the engine started this on,
 * as a tap on the phone would. Only an app on the screen may start others, so this
 * has a window, though one with nothing in it.
 */
class OpenActivity : ComponentActivity() {
    private var opened = false

    override fun onResume() {
        super.onResume()
        if (opened) return
        opened = true
        open()
        finish()
    }

    private fun open() {
        val key = intent.getStringExtra(EXTRA_KEY).orEmpty()
        val posted = Shade.find(key)
        val led = posted?.notification?.contentIntent?.let(::follow) == true
        // A notification without a place of its own, or gone by now, opens its app.
        if (!led) startApp(intent.getStringExtra(EXTRA_APP))
        if (posted != null && posted.notification.flags and Notification.FLAG_AUTO_CANCEL != 0) {
            runCatching { Shade.dismiss(key) }
        }
    }

    /** Sends the notification's own intent, lending it this app's right to start activities. */
    private fun follow(target: PendingIntent): Boolean {
        // A broadcast or a service would start its app on the phone's own screen, if anywhere.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && !target.isActivity) return false
        val options = here()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.BAKLAVA) {
            options.setPendingIntentBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOW_ALWAYS)
        } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            @Suppress("DEPRECATION")
            options.setPendingIntentBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED)
        }
        return runCatching { target.send(this, 0, null, null, null, null, options.toBundle()) }.isSuccess
    }

    private fun startApp(app: String?) {
        val launch = app?.let(packageManager::getLaunchIntentForPackage) ?: return
        runCatching { startActivity(launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK), here().toBundle()) }
    }

    /** Options that start an activity on this one's display. */
    private fun here(): ActivityOptions {
        val display = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            display?.displayId
        } else {
            @Suppress("DEPRECATION")
            windowManager.defaultDisplay.displayId
        }
        return ActivityOptions.makeBasic().setLaunchDisplayId(display ?: Display.DEFAULT_DISPLAY)
    }
}
