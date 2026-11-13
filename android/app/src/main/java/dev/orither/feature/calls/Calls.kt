package dev.orither.feature.calls

import android.Manifest
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.os.PowerManager
import android.util.Log
import dev.orither.R
import dev.orither.engine.EngineClient
import dev.orither.feature.Access
import dev.orither.feature.Feature
import dev.orither.feature.body
import dev.orither.feature.text
import dev.orither.feature.textOrNull
import dev.orither.link.Link
import dev.orither.link.LinkStream
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long

private const val ID = "calls"
/** How long a call that ended stays listed, so that a computer can show it ending. */
private const val ENDED_SHOWN_MS = 3_000L
private const val CONTACTS_SHOWN = 500
private const val RECENTS_SHOWN = 100
/** The engine starts this to act on an app's call, see [CallActivity]. */
private const val ACTOR = "dev.orither/dev.orither.feature.calls.CallActivity"
internal const val PICTURE_SIZE = 128
internal const val RINGING = "ringing"
internal const val ACTIVE = "active"
internal const val ENDED = "ended"

/** What a computer may do about a call. */
private const val LOG_TAG = "Orither"

@Serializable
internal data class Can(val answer: Boolean = false, val decline: Boolean = false, val hangup: Boolean = false)

/** A call as a computer shows it, whether the mobile network's or an app's. */
@Serializable
internal data class Call(
    val id: String,
    /** `cellular` or `app`. */
    val source: String,
    val app: String,
    val appLabel: String,
    val name: String,
    val number: String?,
    val picture: Boolean,
    val video: Boolean,
    val state: String,
    /** When the call took its [state]. */
    val since: Long,
    val can: Can,
)

@Serializable
private data class Listed(val calls: List<Call>)

/** A SIM a call can go out on, by its phone account's id. */
@Serializable
internal data class Sim(val id: String, val label: String)

@Serializable
internal data class Sims(val sims: List<Sim>, val preferred: String?)

/** The three things a computer can do to a call. */
internal enum class Act { ANSWER, DECLINE, HANG_UP }

/**
 * Every call on the phone, from the mobile network and from apps, as computers see
 * them. Each source tells what it has now, and a call it no longer has stays a few
 * seconds as ended.
 */
internal object Board {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val live = LinkedHashMap<String, Call>()
    private val ended = LinkedHashMap<String, Call>()

    @Synchronized
    fun all(): List<Call> = (live.values + ended.values).sortedBy(Call::since)

    @Synchronized
    fun find(id: String): Call? = live[id]

    /** Takes what [source] has now, ending the calls it had before and has no more. */
    @Synchronized
    fun show(source: String, calls: List<Call>) {
        val now = System.currentTimeMillis()
        var changed = false
        val gone = live.values.filter { it.source == source && calls.none { call -> call.id == it.id } }
        for (call in gone) {
            live.remove(call.id)
            ended[call.id] = call.copy(state = ENDED, since = now, can = Can())
            scope.launch {
                delay(ENDED_SHOWN_MS)
                forget(call.id)
            }
            changed = true
        }
        for (call in calls) {
            if (live.put(call.id, call) != call) changed = true
        }
        if (changed) announce()
    }

    @Synchronized
    private fun forget(id: String) {
        if (ended.remove(id) != null) announce()
    }

    private fun announce() = Link.broadcast(ID, "changed", body(Listed(all())))
}

/**
 * Calls on the computer: those of the mobile network through Android's telephony,
 * and any app's through its call notification. Their sound reaches the computer over
 * the link ([carryAudio]), or, as a fallback, because the computer is the phone's
 * Bluetooth headset.
 */
internal class Calls(private val context: Context, private val engine: EngineClient) : Feature {
    override val id = ID
    override val access = Access.Permissions(
        R.string.access_calls,
        R.string.access_calls_about,
        R.drawable.ic_phone,
        listOf(
            Manifest.permission.READ_PHONE_STATE,
            Manifest.permission.READ_CALL_LOG,
            Manifest.permission.ANSWER_PHONE_CALLS,
            Manifest.permission.CALL_PHONE,
            Manifest.permission.READ_CONTACTS,
        ),
    )
    private val contacts = Contacts(context)
    private val cellular = Cellular(context, contacts)

    init {
        cellular.start()
    }

    override fun allowed() = cellular.start()

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement = withContext(Dispatchers.IO) {
        when (method) {
            "list" -> body(Listed(Board.all()))
            "answer" -> act(body.text("id"), Act.ANSWER)
            "decline" -> act(body.text("id"), Act.DECLINE)
            "hangup" -> act(body.text("id"), Act.HANG_UP)
            "sims" -> body(cellular.sims())
            "dial" -> {
                cellular.dial(body.text("number"), body.textOrNull("sim"))
                JsonPrimitive(true)
            }
            "contacts" -> body(contacts.find(body.textOrNull("query"), body.limit() ?: CONTACTS_SHOWN))
            "place" -> {
                place(body.jsonObject.getValue("data").jsonPrimitive.long)
                JsonPrimitive(true)
            }
            "recents" -> body(contacts.recents(body.limit() ?: RECENTS_SHOWN))
            else -> throw UnsupportedOperationException("calls has no method $method")
        }
    }

    override suspend fun stream(method: String, body: JsonElement, stream: LinkStream, peer: String) {
        if (method == "audio") return carryAudio(body.text("id"), stream)
        val picture = withContext(Dispatchers.IO) {
            when (method) {
                "picture" -> picture(body.text("id"))
                "contact-picture" -> contacts.picture(body.jsonObject.getValue("id").jsonPrimitive.long)
                else -> null
            }
        }
        if (picture == null) return stream.close()
        stream.send { picture.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }

    /**
     * Joins a computer's stream to the call's sound, which the engine takes from the
     * modem for a phone call and from the app for an app's call. The sound flows both
     * ways until either end closes.
     */
    private suspend fun carryAudio(id: String, stream: LinkStream) {
        val call = Board.all().firstOrNull { it.id == id }
        if (call == null) return stream.close()
        val uid = if (call.source == "cellular") {
            null
        } else {
            runCatching { context.packageManager.getApplicationInfo(call.app, 0).uid }.getOrNull() ?: return stream.close()
        }
        runCatching { engine.callAudio(uid) }
            .onSuccess {
                Log.i(LOG_TAG, "call sound for ${call.source} call started")
                stream.attach(it)
            }
            .onFailure {
                Log.w(LOG_TAG, "call sound for ${call.source} call: ${it.message}")
                stream.close()
            }
    }

    private fun picture(id: String): Bitmap? =
        if (Cellular.owns(id)) cellular.picture(id) else Ringing.picture(context, id)

    private suspend fun act(id: String, act: Act): JsonElement {
        if (Cellular.owns(id)) cellular.act(id, act) else actOnApp(id, act)
        return JsonPrimitive(true)
    }

    /**
     * Sends the app's own answer, decline or hang-up, as a tap on its notification
     * would. Answering often opens the app's call screen, which Android only lets an
     * app on the screen do, so while the screen is on the engine starts [CallActivity]
     * to send it from there. With the screen off nothing would show it, and the
     * intent goes straight out.
     */
    private suspend fun actOnApp(id: String, act: Act) {
        val target = checkNotNull(Ringing.intent(id, act)) { "This call can't do that now" }
        val awake = context.getSystemService(PowerManager::class.java).isInteractive
        val opener = listOf("-n", ACTOR, "--es", EXTRA_CALL, id, "--es", EXTRA_ACT, act.name)
        if (awake && runCatching { engine.startActivity(opener) }.getOrDefault(false)) return
        check(lend(target, context)) { "The app didn't take it" }
    }

    /** Starts an app's call to a contact, as tapping it in the contact would. */
    private suspend fun place(data: Long) {
        val (uri, mime) = checkNotNull(contacts.callAction(data)) { "That contact can't be called this way" }
        val view = listOf("-a", Intent.ACTION_VIEW, "-d", uri.toString(), "-t", mime)
        if (runCatching { engine.startActivity(view) }.getOrDefault(false)) return
        // Without the engine Android may well block this, as Orither is in the background.
        val intent = Intent(Intent.ACTION_VIEW).setDataAndType(uri, mime).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(intent)
    }
}

private fun JsonElement.limit(): Int? = jsonObject["limit"]?.jsonPrimitive?.intOrNull?.takeIf { it > 0 }
