package dev.orither.feature.calls

import android.Manifest
import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.telecom.TelecomManager
import android.telephony.PhoneStateListener
import android.telephony.TelephonyCallback
import android.telephony.TelephonyManager
import androidx.core.content.ContextCompat
import androidx.core.graphics.drawable.toBitmap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

private const val SOURCE = "cellular"
private const val PHONE_APP = "com.android.phone"
/** A call that starts this soon after the computer dialed is the one it dialed. */
private const val DIALED_WITHIN_MS = 15_000L

/** What Android's telephony says the phone is doing. */
internal enum class Phase { IDLE, RINGING, OFFHOOK }

/** One mobile call, as far as telephony lets an app follow it. */
internal data class Line(val id: String, val number: String?, val ringing: Boolean, val since: Long)

/**
 * The mobile calls on the phone. Telephony tells apps only whether the phone is
 * idle, ringing or in a call, so a second call that rings during one is [waiting],
 * and once the phone is back in a call it is unknown whether that one was taken or
 * turned down. It is dropped, and the first call stays.
 */
internal data class Lines(val current: Line? = null, val waiting: Line? = null) {
    fun all(): List<Line> = listOfNotNull(current, waiting)

    /**
     * The calls after telephony moved to [phase]. [number] is the caller's when it is
     * known, [dialed] the number the computer just called, and [id] names a new call.
     */
    fun after(phase: Phase, number: String?, dialed: String?, now: Long, id: String): Lines = when (phase) {
        Phase.IDLE -> Lines()
        Phase.RINGING -> when {
            current == null -> Lines(Line(id, number, ringing = true, since = now))
            current.ringing -> copy(current = current.numbered(number))
            else -> copy(waiting = waiting?.numbered(number) ?: Line(id, number, ringing = true, since = now))
        }
        Phase.OFFHOOK -> when {
            current == null -> Lines(Line(id, number ?: dialed, ringing = false, since = now))
            current.ringing -> Lines(current.copy(ringing = false, since = now).numbered(number))
            else -> Lines(current)
        }
    }

    /** The calls once the number of the one that rings is known. */
    fun numbered(number: String): Lines = when {
        waiting != null -> copy(waiting = waiting.numbered(number))
        current?.ringing == true -> copy(current = current.numbered(number))
        else -> this
    }

    private fun Line.numbered(number: String?) = if (this.number == null && number != null) copy(number = number) else this
}

/**
 * Follows the mobile network's calls through telephony, and answers, ends and
 * places them through Telecom, as a headset or a watch does.
 */
internal class Cellular(private val context: Context, private val contacts: Contacts) {
    private val telephony = context.getSystemService(TelephonyManager::class.java)
    private val telecom = context.getSystemService(TelecomManager::class.java)
    /** Everything here happens one thing at a time, on this. */
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO.limitedParallelism(1))
    @Volatile private var lines = Lines()
    private var dialed: Pair<String, Long>? = null
    private var started = false
    private var count = 0
    private val known = HashMap<String, Found?>()

    /** Starts following calls, once Android allows reading the phone's state. */
    fun start() {
        scope.launch {
            if (started || !granted(Manifest.permission.READ_PHONE_STATE)) return@launch
            started = true
            listen()
            // The caller's number comes with this broadcast, to apps that may read the call log.
            val states = IntentFilter(TelephonyManager.ACTION_PHONE_STATE_CHANGED)
            ContextCompat.registerReceiver(context, numbers, states, ContextCompat.RECEIVER_NOT_EXPORTED)
            Ringing.phoneCaller.collect { publish() }
        }
    }

    @SuppressLint("MissingPermission")
    @Suppress("DEPRECATION")
    fun act(id: String, act: Act) {
        check(granted(Manifest.permission.ANSWER_PHONE_CALLS)) { "Allow Calls on the phone first" }
        check(lines.all().any { it.id == id }) { "That call is over" }
        when (act) {
            Act.ANSWER -> telecom.acceptRingingCall()
            Act.DECLINE, Act.HANG_UP -> check(telecom.endCall()) { "The phone didn't end the call" }
        }
    }

    /** The SIMs calls can go out on, with the one Android uses by itself, if any. */
    @SuppressLint("MissingPermission")
    fun sims(): Sims {
        if (!granted(Manifest.permission.READ_PHONE_STATE)) return Sims(emptyList(), null)
        val accounts = telecom.callCapablePhoneAccounts
        val sims = accounts.mapIndexed { index, handle ->
            val label = telecom.getPhoneAccount(handle)?.label?.toString()?.takeIf(String::isNotBlank) ?: "SIM ${index + 1}"
            Sim(handle.id, label)
        }
        return Sims(sims, telecom.getDefaultOutgoingPhoneAccount("tel")?.id)
    }

    /**
     * Calls [number] on [sim], or on the SIM Android would use. With two SIMs and none
     * chosen, Android would stop to ask on the phone's screen, out of sight of whoever
     * dialed on the computer, so the computer has to say which.
     */
    @SuppressLint("MissingPermission")
    fun dial(number: String, sim: String?) {
        check(granted(Manifest.permission.CALL_PHONE)) { "Allow Calls on the phone first" }
        val digits = number.trim()
        require(digits.isNotEmpty()) { "No number to call" }
        val accounts = if (granted(Manifest.permission.READ_PHONE_STATE)) telecom.callCapablePhoneAccounts else emptyList()
        val account = accounts.firstOrNull { it.id == sim }
            ?: telecom.getDefaultOutgoingPhoneAccount("tel")
            ?: accounts.singleOrNull()
        check(account != null || accounts.size < 2) { "Choose which SIM to call from" }
        val extras = Bundle()
        if (account != null) extras.putParcelable(TelecomManager.EXTRA_PHONE_ACCOUNT_HANDLE, account)
        scope.launch { dialed = digits to System.currentTimeMillis() }
        telecom.placeCall(Uri.fromParts("tel", digits, null), extras)
    }

    fun picture(id: String): Bitmap? {
        val number = Board.find(id)?.number
        val contact = number?.let(contacts::lookup)?.takeIf(Found::picture)
        contact?.let { return contacts.picture(it.id) }
        val icon = Ringing.phoneCaller.value?.icon ?: return null
        return runCatching { icon.loadDrawable(context)?.toBitmap(PICTURE_SIZE, PICTURE_SIZE) }.getOrNull()
    }

    private fun listen() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val callback = object : TelephonyCallback(), TelephonyCallback.CallStateListener {
                override fun onCallStateChanged(state: Int) {
                    scope.launch { changed(state, null) }
                }
            }
            telephony.registerTelephonyCallback(context.mainExecutor, callback)
        } else {
            @Suppress("DEPRECATION")
            val listener = object : PhoneStateListener(context.mainExecutor) {
                @Deprecated("Deprecated in Java")
                override fun onCallStateChanged(state: Int, phoneNumber: String?) {
                    scope.launch { changed(state, phoneNumber?.takeIf(String::isNotBlank)) }
                }
            }
            @Suppress("DEPRECATION")
            telephony.listen(listener, PhoneStateListener.LISTEN_CALL_STATE)
        }
    }

    private val numbers = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            @Suppress("DEPRECATION")
            val number = intent.getStringExtra(TelephonyManager.EXTRA_INCOMING_NUMBER)?.takeIf(String::isNotBlank) ?: return
            scope.launch {
                lines = lines.numbered(number)
                publish()
            }
        }
    }

    private fun changed(state: Int, number: String?) {
        val phase = when (state) {
            TelephonyManager.CALL_STATE_RINGING -> Phase.RINGING
            TelephonyManager.CALL_STATE_OFFHOOK -> Phase.OFFHOOK
            else -> Phase.IDLE
        }
        val now = System.currentTimeMillis()
        val outgoing = dialed?.takeIf { now - it.second < DIALED_WITHIN_MS }?.first
        lines = lines.after(phase, number, outgoing, now, "$SOURCE:${++count}")
        if (phase != Phase.RINGING) dialed = null
        if (phase == Phase.IDLE) known.clear()
        publish()
    }

    private fun publish() = Board.show(SOURCE, lines.all().map(::describe))

    private fun describe(line: Line): Call {
        val contact = line.number?.let { known.getOrPut(it) { contacts.lookup(it) } }
        val caller = Ringing.phoneCaller.value
        val answerable = granted(Manifest.permission.ANSWER_PHONE_CALLS)
        return Call(
            id = line.id,
            source = SOURCE,
            app = dialer(),
            appLabel = label(dialer()),
            name = contact?.name ?: caller?.name ?: line.number.orEmpty(),
            number = line.number ?: caller?.number,
            picture = contact?.picture == true || caller?.icon != null,
            video = false,
            state = if (line.ringing) RINGING else ACTIVE,
            since = line.since,
            can = Can(answer = answerable && line.ringing, decline = answerable && line.ringing, hangup = answerable && !line.ringing),
        )
    }

    /** The phone app the user calls with. */
    private fun dialer(): String = telecom.defaultDialerPackage ?: PHONE_APP

    private fun label(app: String): String {
        val packages = context.packageManager
        return runCatching { packages.getApplicationLabel(packages.getApplicationInfo(app, 0)).toString() }.getOrDefault(app)
    }

    private fun granted(permission: String) =
        ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

    companion object {
        fun owns(id: String) = id.startsWith("$SOURCE:")
    }
}
