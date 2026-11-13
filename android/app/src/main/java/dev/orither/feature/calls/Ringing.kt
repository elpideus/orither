package dev.orither.feature.calls

import android.app.ActivityOptions
import android.app.Notification
import android.app.PendingIntent
import android.app.Person
import android.content.Context
import android.graphics.Bitmap
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.telecom.TelecomManager
import androidx.activity.ComponentActivity
import androidx.core.graphics.drawable.toBitmap
import androidx.core.os.BundleCompat
import java.text.Normalizer
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

internal const val EXTRA_CALL = "call"
internal const val EXTRA_ACT = "act"
private const val SOURCE = "app"
/** Where a call notification keeps its parts. `Notification.CallStyle` and its AndroidX twin use the same keys. */
private const val EXTRA_CALL_TYPE = "android.callType"
private const val EXTRA_CALL_PERSON = "android.callPerson"
private const val EXTRA_CALL_IS_VIDEO = "android.callIsVideo"
private const val EXTRA_ANSWER_INTENT = "android.answerIntent"
private const val EXTRA_DECLINE_INTENT = "android.declineIntent"
private const val EXTRA_HANG_UP_INTENT = "android.hangUpIntent"
private const val CALL_STYLE = "android.app.Notification\$CallStyle"
private const val CALL_TYPE_INCOMING = 1
private const val CALL_TYPE_ONGOING = 2
private const val CALL_TYPE_SCREENING = 3
private const val SEMANTIC_ACTION_CALL = 10

/**
 * The apps that show the mobile network's own calls. Their notification is the call
 * [Cellular] lists already, so it only lends that call its caller's name and picture.
 */
private val PHONE_APPS = setOf(
    "com.android.dialer",
    "com.android.incallui",
    "com.android.phone",
    "com.android.server.telecom",
    "com.google.android.dialer",
    "com.samsung.android.incallui",
    "com.samsung.android.dialer",
)

/**
 * What action titles say, in the languages most phones speak, for apps that post a
 * plain call notification with buttons rather than a `CallStyle` one. They are
 * compared without case or accents.
 */
private val SAYINGS = mapOf(
    Act.ANSWER to listOf(
        "answer", "accept", "pick up", "rispondi", "accetta", "responder", "contestar", "aceptar", "atender",
        "aceitar", "répondre", "décrocher", "accepter", "annehmen", "abnehmen", "antworten", "ответить",
        "принять", "接听", "接聽", "응답", "応答", "cevapla", "odbierz", "opnemen",
    ),
    Act.DECLINE to listOf(
        "decline", "reject", "ignore", "rifiuta", "ignora", "rechazar", "rejeitar", "recusar", "refuser",
        "ignorer", "ablehnen", "отклонить", "拒绝", "拒絕", "거절", "拒否", "reddet", "odrzuć", "weigeren",
    ),
    Act.HANG_UP to listOf(
        "hang up", "end call", "end", "leave", "riaggancia", "termina", "chiudi", "colgar", "finalizar",
        "encerrar", "desligar", "raccrocher", "terminer", "auflegen", "beenden", "завершить", "挂断", "掛斷",
        "结束", "종료", "終了", "kapat", "rozłącz", "zakończ", "ophangen", "beëindigen",
    ),
).mapValues { (_, words) -> words.map(::fold).map { Regex("(^|\\W)${Regex.escape(it)}($|\\W)") } }

private val VIDEO_WORDS = listOf("video", "видео", "视频", "視訊", "ビデオ", "영상").map(::fold)

/** Lower case without accents, so that "Rispondi" and "rispondi", "Répondre" and "repondre" compare equal. */
internal fun fold(text: CharSequence): String =
    Normalizer.normalize(text.toString().lowercase(), Normalizer.Form.NFD).replace(Regex("\\p{M}+"), "")

/** What a notification button does to a call, judged by its title, or by its meaning when the app gave one. */
internal fun actOf(title: CharSequence?, semantic: Int): Act? {
    val said = title?.let(::fold)
    if (said != null) SAYINGS.entries.firstOrNull { (_, ways) -> ways.any { it.containsMatchIn(said) } }?.let { return it.key }
    return if (semantic == SEMANTIC_ACTION_CALL) Act.ANSWER else null
}

/**
 * Whether a notification is a call in progress or ringing. A `CallStyle` one says
 * so. Other apps only mark it a call, which a missed call can be too, so then it has
 * to stay on the shade or ring through the screen as a call does.
 */
internal fun isCall(category: String?, template: String?, callType: Int, ongoing: Boolean, urgent: Boolean): Boolean =
    template == CALL_STYLE || callType != 0 || (category == Notification.CATEGORY_CALL && (ongoing || urgent))

/** A call that can be answered is ringing, unless its style says otherwise. */
internal fun callState(callType: Int, acts: Set<Act>): String = when (callType) {
    CALL_TYPE_INCOMING, CALL_TYPE_SCREENING -> RINGING
    CALL_TYPE_ONGOING -> ACTIVE
    else -> if (Act.ANSWER in acts) RINGING else ACTIVE
}

internal fun mentionsVideo(text: String): Boolean = fold(text).let { said -> VIDEO_WORDS.any { it in said } }

/** The caller as the phone app's call notification shows them. */
internal class Caller(val name: String?, val number: String?, val icon: Icon?)

/**
 * The calls apps show in their notifications, such as WhatsApp's, Telegram's or
 * Signal's. Call notifications are ongoing, so the notifications feature leaves
 * them out, and the listener hands them here first.
 */
internal object Ringing {
    private class Kept(val call: Call, val intents: Map<Act, PendingIntent>, val icon: Icon?)

    private val kept = ConcurrentHashMap<String, Kept>()
    private val labels = ConcurrentHashMap<String, String>()
    private val caller = MutableStateFlow<Caller?>(null)
    @Volatile private var callerKey: String? = null

    /** The phone app's caller, while it shows a call. */
    val phoneCaller: StateFlow<Caller?> = caller

    fun connected(listener: NotificationListenerService, active: List<StatusBarNotification>) {
        kept.clear()
        caller.value = null
        active.forEach { take(listener, it, announce = false) }
        publish()
    }

    fun disconnected() {
        kept.clear()
        caller.value = null
        publish()
    }

    fun posted(listener: NotificationListenerService, posted: StatusBarNotification) = take(listener, posted, announce = true)

    fun removed(key: String) {
        if (key == callerKey) {
            callerKey = null
            caller.value = null
        }
        if (kept.remove(idOf(key)) != null) publish()
    }

    fun intent(id: String, act: Act): PendingIntent? = kept[id]?.intents?.get(act)

    fun picture(context: Context, id: String): Bitmap? {
        val icon = kept[id]?.icon ?: return null
        return runCatching { icon.loadDrawable(context)?.toBitmap(PICTURE_SIZE, PICTURE_SIZE) }.getOrNull()
    }

    private fun idOf(key: String) = "$SOURCE:$key"

    private fun take(listener: NotificationListenerService, posted: StatusBarNotification, announce: Boolean) {
        val id = idOf(posted.key)
        val notification = posted.notification
        val extras = notification.extras
        val callType = extras.getInt(EXTRA_CALL_TYPE, 0)
        val ongoing = notification.flags and Notification.FLAG_ONGOING_EVENT != 0
        val template = extras.getString(Notification.EXTRA_TEMPLATE)
        val call = posted.packageName != listener.packageName &&
            isCall(notification.category, template, callType, ongoing, notification.fullScreenIntent != null)
        when {
            // An update can end a call by turning its notification into something else.
            !call -> removed(posted.key)
            posted.packageName in phoneApps(listener) -> {
                callerKey = posted.key
                caller.value = person(extras).let { Caller(it?.name?.toString() ?: title(extras), number(it), it?.icon) }
            }
            else -> {
                kept[id] = describe(listener, posted, id, callType)
                if (announce) publish()
            }
        }
    }

    private fun describe(context: Context, posted: StatusBarNotification, id: String, callType: Int): Kept {
        val notification = posted.notification
        val extras = notification.extras
        val intents = intents(notification)
        val state = callState(callType, intents.keys)
        val person = person(extras)
        val name = person?.name?.toString() ?: title(extras) ?: label(context, posted.packageName)
        val text = listOfNotNull(title(extras), extras.getCharSequence(Notification.EXTRA_TEXT)?.toString()).joinToString(" ")
        val before = kept[id]?.call?.takeIf { it.state == state }
        val since = before?.since ?: if (state == ACTIVE && extras.getBoolean(Notification.EXTRA_SHOW_CHRONOMETER)) {
            notification.`when`
        } else {
            System.currentTimeMillis()
        }
        val icon = person?.icon ?: notification.getLargeIcon()
        val call = Call(
            id = id,
            source = SOURCE,
            app = posted.packageName,
            appLabel = label(context, posted.packageName),
            name = name,
            number = number(person),
            picture = icon != null,
            video = extras.getBoolean(EXTRA_CALL_IS_VIDEO) || mentionsVideo(text),
            state = state,
            since = since,
            can = Can(
                answer = state == RINGING && Act.ANSWER in intents,
                decline = state == RINGING && Act.DECLINE in intents,
                hangup = Act.HANG_UP in intents,
            ),
        )
        return Kept(call, intents, icon)
    }

    /** A `CallStyle` keeps its intents apart, and other apps leave them on their buttons. */
    private fun intents(notification: Notification): Map<Act, PendingIntent> = buildMap {
        val extras = notification.extras
        val styled = mapOf(Act.ANSWER to EXTRA_ANSWER_INTENT, Act.DECLINE to EXTRA_DECLINE_INTENT, Act.HANG_UP to EXTRA_HANG_UP_INTENT)
        styled.forEach { (act, key) -> BundleCompat.getParcelable(extras, key, PendingIntent::class.java)?.let { put(act, it) } }
        notification.actions.orEmpty().forEach { action ->
            val act = actOf(action.title, action.semanticAction) ?: return@forEach
            if (act !in this) action.actionIntent?.let { put(act, it) }
        }
    }

    private fun publish() = Board.show(SOURCE, kept.values.map(Kept::call))

    private fun person(extras: Bundle): Person? = BundleCompat.getParcelable(extras, EXTRA_CALL_PERSON, Person::class.java)

    private fun title(extras: Bundle): String? = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString()?.takeIf(String::isNotBlank)

    private fun number(person: Person?): String? =
        person?.uri?.let(Uri::parse)?.takeIf { it.scheme == "tel" }?.schemeSpecificPart?.takeIf(String::isNotBlank)

    private fun phoneApps(context: Context): Set<String> {
        val telecom = context.getSystemService(TelecomManager::class.java)
        return PHONE_APPS + listOfNotNull(telecom?.defaultDialerPackage, telecom?.systemDialerPackage)
    }

    private fun label(context: Context, app: String): String = labels.getOrPut(app) {
        val packages = context.packageManager
        runCatching { packages.getApplicationLabel(packages.getApplicationInfo(app, 0)).toString() }.getOrDefault(app)
    }
}

/**
 * Sends one of a call's intents, lending it this app's right to start activities,
 * which it has while one of its activities is on the screen or was just started.
 */
internal fun lend(target: PendingIntent, context: Context, options: ActivityOptions = ActivityOptions.makeBasic()): Boolean {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.BAKLAVA) {
        options.setPendingIntentBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOW_ALWAYS)
    } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
        @Suppress("DEPRECATION")
        options.setPendingIntentBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED)
    }
    return runCatching { target.send(context, 0, null, null, null, null, options.toBundle()) }.isSuccess
}

/**
 * Answers, declines or hangs up an app's call from the phone's screen, where the
 * engine starts it. Answering usually opens the app's call screen, which Android
 * lets only an app on the screen do. A ringing phone is often locked, so this shows
 * over the lock screen, though with nothing in it.
 */
class CallActivity : ComponentActivity() {
    private var done = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setShowWhenLocked(true)
    }

    override fun onResume() {
        super.onResume()
        if (done) return
        done = true
        val act = intent.getStringExtra(EXTRA_ACT)?.let { name -> Act.entries.firstOrNull { it.name == name } }
        val target = act?.let { Ringing.intent(intent.getStringExtra(EXTRA_CALL).orEmpty(), it) }
        if (target != null) lend(target, this)
        finish()
    }
}
