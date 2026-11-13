package dev.orither.feature.input

import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.orither.link.Link
import kotlin.math.roundToInt
import kotlinx.coroutines.delay
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/**
 * Motion summed over a frame and cut to tenths of a pixel. What is cut off stays
 * for the next frame, so slow motion adds up instead of getting lost.
 */
class Coalesced {
    private var x = 0f
    private var y = 0f

    fun add(dx: Float, dy: Float) {
        x += dx
        y += dy
    }

    /** The whole tenths gathered so far, or null when there are none yet. */
    fun take(): Pair<Float, Float>? {
        val tenthsX = (x * TENTHS).roundToInt()
        val tenthsY = (y * TENTHS).roundToInt()
        if (tenthsX == 0 && tenthsY == 0) return null
        val takenX = tenthsX / TENTHS
        val takenY = tenthsY / TENTHS
        x -= takenX
        y -= takenY
        return takenX to takenY
    }

    private companion object {
        const val TENTHS = 10f
    }
}

/**
 * This phone as [peer]'s trackpad and keyboard, through the computer's `input`
 * feature. Everything goes out as a notice, in order, from the main thread. Motion
 * and scrolling go once a frame, and anything else first sends what motion is due.
 *
 * Ctrl, Alt and Meta are sticky, as Shift is on a phone keyboard: [toggle] arms one,
 * and the next key or text goes out with it held.
 */
class RemoteInput(private val peer: String, private val onClick: () -> Unit = {}) : TrackpadOutput, Typist {
    /** Whether the computer takes input. */
    var ready by mutableStateOf(false)
        private set

    /** What the user has to do before it does, in the computer's words. */
    var problem by mutableStateOf<String?>(null)
        private set

    var armed by mutableStateOf(emptySet<String>())
        private set

    private val main = Handler(Looper.getMainLooper())
    private val motion = Coalesced()
    private val scrolling = Coalesced()
    private var due = false
    private val sendDue = Runnable { sendMotion() }

    /** Asks the computer to take input, then keeps checking, for as long as it runs. */
    suspend fun stayOpen() {
        open(ask = true)
        while (true) {
            delay(CHECK_EVERY_MS)
            open(ask = false)
        }
    }

    /** With [ask] false the computer only says how things stand, and asks the user nothing. */
    suspend fun open(ask: Boolean) {
        val answer = runCatching { Link.call(peer, FEATURE, "open", buildJsonObject { put("ask", ask) }) }
        ready = answer.getOrNull()?.jsonObject?.get("ready")?.jsonPrimitive?.booleanOrNull == true
        problem = answer.exceptionOrNull()?.let { it.message ?: it.toString() }
    }

    fun toggle(modifier: String) {
        armed = if (modifier in armed) armed - modifier else armed + modifier
    }

    override fun move(dx: Float, dy: Float) {
        motion.add(dx, dy)
        soon()
    }

    override fun scroll(dx: Float, dy: Float) {
        scrolling.add(dx, dy)
        soon()
    }

    override fun scrollEnded() {
        sendMotion()
        send("scroll", buildJsonObject {
            put("dx", 0)
            put("dy", 0)
            put("end", true)
        })
    }

    override fun click(button: String) {
        sendMotion()
        onClick()
        send("click", buildJsonObject { put("button", button) })
    }

    override fun button(button: String, down: Boolean) {
        sendMotion()
        if (down) onClick()
        send("button", buildJsonObject {
            put("button", button)
            put("down", down)
        })
    }

    override fun type(text: String) = withArmed { send("text", buildJsonObject { put("text", text) }) }

    override fun press(key: String) = withArmed { key(key) }

    override fun erase(count: Int) = repeat(count) { key("Backspace") }

    private fun key(name: String, down: Boolean? = null) = send("key", buildJsonObject {
        put("key", name)
        down?.let { put("down", it) }
    })

    /** Holds the armed modifiers around [action], and disarms them. */
    private fun withArmed(action: () -> Unit) {
        val held = armed
        armed = emptySet()
        held.forEach { key(it, down = true) }
        action()
        held.forEach { key(it, down = false) }
    }

    private fun soon() {
        if (due) return
        due = true
        main.postDelayed(sendDue, FRAME_MS)
    }

    private fun sendMotion() {
        main.removeCallbacks(sendDue)
        due = false
        motion.take()?.let { (dx, dy) -> send("move", delta(dx, dy)) }
        scrolling.take()?.let { (dx, dy) -> send("scroll", delta(dx, dy)) }
    }

    private fun delta(dx: Float, dy: Float) = buildJsonObject {
        put("dx", dx)
        put("dy", dy)
    }

    /** A computer that went away misses this, which is all that can happen to it. */
    private fun send(method: String, body: JsonObject) {
        runCatching { Link.notify(peer, FEATURE, method, body) }
    }

    companion object {
        const val CONTROL = "Control"
        const val ALT = "Alt"
        const val META = "Meta"
        private const val FEATURE = "input"
        private const val FRAME_MS = 8L
        private const val CHECK_EVERY_MS = 2_000L
    }
}
