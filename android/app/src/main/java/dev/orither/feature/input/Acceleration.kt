package dev.orither.feature.input

import kotlin.math.hypot

/**
 * Pointer acceleration as on a laptop's touchpad. Slow motion moves the pointer
 * about as far as the finger, for precision, and fast motion several times farther,
 * so that the pointer crosses a large screen in a swipe or two. The gain follows a
 * smoothed speed, so it never jumps from one event to the next.
 */
class Acceleration {
    /** The finger's speed in dp per millisecond, smoothed. */
    private var speed = 0f

    /** Forgets the last gesture's speed. */
    fun reset() {
        speed = 0f
    }

    /** The pointer motion in pixels for a finger's motion in dp over [elapsedMs]. */
    fun apply(dx: Float, dy: Float, elapsedMs: Long): Pair<Float, Float> {
        // Events are about a frame apart. A longer gap is a finger that paused, not one that was slow.
        val elapsed = elapsedMs.coerceIn(1L, LONGEST_GAP_MS).toFloat()
        val now = hypot(dx, dy) / elapsed
        speed = if (speed == 0f) now else speed + SMOOTHING * (now - speed)
        val gain = gain(speed)
        return dx * gain to dy * gain
    }

    companion object {
        private const val LONGEST_GAP_MS = 50L
        private const val SMOOTHING = 0.4f
        private const val SLOW = 0.05f
        private const val FAST = 1.4f
        private const val LEAST = 1.2f
        private const val MOST = 5f

        /** Pixels per dp at [speed] dp/ms: flat while slow, an eased ramp, flat again while fast. */
        fun gain(speed: Float): Float {
            val ramp = ((speed - SLOW) / (FAST - SLOW)).coerceIn(0f, 1f)
            val eased = ramp * ramp * (3 - 2 * ramp)
            return LEAST + (MOST - LEAST) * eased
        }
    }
}
