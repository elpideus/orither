package dev.orither.feature.input

import android.os.SystemClock
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.changedToDownIgnoreConsumed
import androidx.compose.ui.input.pointer.changedToUpIgnoreConsumed
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import kotlin.math.hypot
import kotlin.math.max
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** What a trackpad does, in the computer's terms. Buttons are `left`, `right` and `middle`. */
interface TrackpadOutput {
    fun move(dx: Float, dy: Float)

    /** Pixels to scroll the view by, positive down and right. */
    fun scroll(dx: Float, dy: Float)

    fun scrollEnded()

    fun click(button: String)

    fun button(button: String, down: Boolean)
}

/**
 * Tells a laptop touchpad's gestures apart, from touches in dp:
 *
 * - one finger moves the pointer, with [Acceleration]
 * - a tap clicks, two fingers right-click, three middle-click
 * - two fingers scroll, the content following them
 * - a tap and then a touch drags with the left button held, until that finger lifts
 *
 * A tap's click waits [DRAG_WINDOW_MS] for that second touch, as a touchpad's does,
 * because a click followed by a press would be a double click.
 */
class Trackpad(private val out: TrackpadOutput, private val acceleration: Acceleration = Acceleration()) {
    private class Finger(val startX: Float, val startY: Float) {
        var x = startX
        var y = startY

        fun travelled() = hypot(x - startX, y - startY)
    }

    private val fingers = mutableMapOf<Long, Finger>()
    private var startedAt = 0L
    private var lastMoveAt = 0L

    /** The most fingers down at once since the gesture started. */
    private var most = 0
    private var travelled = false
    private var scrolling = false
    private var dragging = false

    /** When the last one-finger tap lifted, while its click still waits. */
    private var tappedAt: Long? = null

    fun down(id: Long, x: Float, y: Float, time: Long) {
        if (fingers.isEmpty()) start(time)
        fingers[id] = Finger(x, y)
        most = max(most, fingers.size)
        // A second finger before the drag went anywhere was a two-finger gesture, and
        // letting go of the button completes the tap's click.
        if (dragging && fingers.size > 1 && !travelled) {
            out.button(LEFT, false)
            dragging = false
        }
    }

    private fun start(time: Long) {
        startedAt = time
        lastMoveAt = time
        most = 0
        travelled = false
        scrolling = false
        acceleration.reset()
        val tapped = tappedAt ?: return
        tappedAt = null
        if (time - tapped <= DRAG_WINDOW_MS) {
            dragging = true
            out.button(LEFT, true)
        } else {
            out.click(LEFT)
        }
    }

    fun move(id: Long, x: Float, y: Float, time: Long) {
        val finger = fingers[id] ?: return
        val dx = x - finger.x
        val dy = y - finger.y
        finger.x = x
        finger.y = y
        if (!travelled && finger.travelled() > SLOP) travelled = true
        when {
            most == 1 -> {
                val (pointerX, pointerY) = acceleration.apply(dx, dy, time - lastMoveAt)
                out.move(pointerX, pointerY)
            }
            // Each finger moves the middle of the two by half its own motion.
            most == 2 && fingers.size == 2 && travelled -> {
                scrolling = true
                out.scroll(-dx / 2 * SCROLL_GAIN, -dy / 2 * SCROLL_GAIN)
            }
        }
        lastMoveAt = time
    }

    fun up(id: Long, time: Long) {
        fingers.remove(id) ?: return
        if (fingers.isNotEmpty()) return
        val tap = !travelled && time - startedAt <= TAP_MS
        when {
            dragging -> {
                out.button(LEFT, false)
                dragging = false
                // Two quick taps: the press and release just now, and one more, are a double click.
                if (tap && most == 1) out.click(LEFT)
            }
            scrolling -> out.scrollEnded()
            tap && most == 1 -> tappedAt = time
            tap && most == 2 -> out.click(RIGHT)
            tap && most >= 3 -> out.click(MIDDLE)
        }
    }

    /** Clicks a tap once no drag can follow it any more. Call it [DRAG_WINDOW_MS] after an [up]. */
    fun expire(time: Long) {
        val tapped = tappedAt ?: return
        if (time - tapped >= DRAG_WINDOW_MS) {
            tappedAt = null
            out.click(LEFT)
        }
    }

    /** The touches were taken away, or the trackpad is going: nothing may stay held. */
    fun cancel() {
        if (dragging) out.button(LEFT, false)
        if (scrolling) out.scrollEnded()
        fingers.clear()
        dragging = false
        scrolling = false
        tappedAt = null
    }

    companion object {
        const val LEFT = "left"
        const val RIGHT = "right"
        const val MIDDLE = "middle"
        const val TAP_MS = 250L
        const val DRAG_WINDOW_MS = 200L

        /** How far a finger may wander, in dp, and still tap. */
        const val SLOP = 8f

        /** Scrolled pixels per dp the fingers move. */
        const val SCROLL_GAIN = 1.5f
    }
}

/**
 * Feeds this element's touches to [trackpad], in dp. [onTouch] runs as each gesture
 * starts, before the trackpad hears of it.
 */
fun Modifier.trackpad(trackpad: Trackpad, onTouch: () -> Unit = {}): Modifier = pointerInput(trackpad) {
    coroutineScope {
        var expiry: Job? = null
        try {
            awaitPointerEventScope {
                while (true) {
                    val event = awaitPointerEvent()
                    for (change in event.changes) {
                        val id = change.id.value
                        val x = change.position.x.toDp().value
                        val y = change.position.y.toDp().value
                        when {
                            change.changedToDownIgnoreConsumed() -> {
                                if (event.changes.none { it.previousPressed }) onTouch()
                                trackpad.down(id, x, y, change.uptimeMillis)
                            }
                            change.changedToUpIgnoreConsumed() -> {
                                trackpad.up(id, change.uptimeMillis)
                                expiry?.cancel()
                                expiry = launch {
                                    delay(Trackpad.DRAG_WINDOW_MS)
                                    trackpad.expire(SystemClock.uptimeMillis())
                                }
                            }
                            change.pressed && change.positionChanged() -> trackpad.move(id, x, y, change.uptimeMillis)
                        }
                        change.consume()
                    }
                }
            }
        } finally {
            trackpad.cancel()
        }
    }
}
