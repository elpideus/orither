package dev.orither.feature.input

import android.view.KeyEvent
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

class AccelerationTest {
    @Test
    fun slowMotionStaysPreciseAndFastMotionGoesFar() {
        assertEquals(1.2f, Acceleration.gain(0f), 0.0001f)
        assertEquals(5f, Acceleration.gain(10f), 0.0001f)
        val speeds = (0..40).map { it * 0.05f }
        speeds.zipWithNext().forEach { (slower, faster) -> assertTrue(Acceleration.gain(slower) <= Acceleration.gain(faster)) }
    }

    @Test
    fun keepsTheDirection() {
        val (dx, dy) = Acceleration().apply(3f, -4f, 8)
        assertEquals(-4f / 3f, dy / dx, 0.0001f)
    }

    @Test
    fun theGainGrowsGraduallyWithinAFlick() {
        val acceleration = Acceleration()
        val first = acceleration.apply(1f, 0f, 16).first
        val gains = (1..6).map { acceleration.apply(20f, 0f, 8).first / 20f }
        assertTrue(first < 2f)
        gains.zipWithNext().forEach { (before, after) -> assertTrue(after >= before) }
        assertTrue(gains.first() < gains.last())
    }
}

class CoalescedTest {
    @Test
    fun slowMotionAddsUpInsteadOfGettingLost() {
        val coalesced = Coalesced()
        coalesced.add(0.04f, 0f)
        assertNull(coalesced.take())
        coalesced.add(0.04f, 0f)
        assertEquals(0.1f to 0f, coalesced.take())
        repeat(25) { coalesced.add(0.04f, -0.02f) }
        val (dx, dy) = coalesced.take()!!
        assertEquals(1f, dx, 0.11f)
        assertEquals(-0.5f, dy, 0.11f)
    }
}

private class Recorded : TrackpadOutput {
    val events = mutableListOf<String>()
    var moved = 0f to 0f
    var scrolled = 0f to 0f

    override fun move(dx: Float, dy: Float) {
        moved = moved.first + dx to moved.second + dy
    }

    override fun scroll(dx: Float, dy: Float) {
        scrolled = scrolled.first + dx to scrolled.second + dy
    }

    override fun scrollEnded() {
        events += "scrolled"
    }

    override fun click(button: String) {
        events += "click $button"
    }

    override fun button(button: String, down: Boolean) {
        events += "$button ${if (down) "down" else "up"}"
    }
}

class TrackpadTest {
    private val out = Recorded()
    private val trackpad = Trackpad(out)

    @Test
    fun aTapClicksOnceNoDragCanFollow() {
        trackpad.down(1, 10f, 10f, 0)
        trackpad.up(1, 80)
        trackpad.expire(150)
        assertEquals(emptyList(), out.events)
        trackpad.expire(80 + Trackpad.DRAG_WINDOW_MS)
        assertEquals(listOf("click left"), out.events)
    }

    @Test
    fun twoFingersRightClickAndThreeMiddleClick() {
        trackpad.down(1, 10f, 10f, 0)
        trackpad.down(2, 40f, 10f, 20)
        trackpad.up(1, 100)
        trackpad.up(2, 110)
        trackpad.down(1, 10f, 10f, 1000)
        trackpad.down(2, 40f, 10f, 1010)
        trackpad.down(3, 70f, 10f, 1020)
        trackpad.up(3, 1100)
        trackpad.up(2, 1100)
        trackpad.up(1, 1110)
        assertEquals(listOf("click right", "click middle"), out.events)
    }

    @Test
    fun oneFingerMovesThePointerAndDoesNotClick() {
        trackpad.down(1, 0f, 0f, 0)
        (1..10).forEach { trackpad.move(1, it * 5f, 0f, it * 8L) }
        trackpad.up(1, 90)
        trackpad.expire(1000)
        assertTrue(out.moved.first > 50f)
        assertEquals(0f, out.moved.second)
        assertEquals(emptyList(), out.events)
    }

    @Test
    fun twoFingersScrollTheWayTheyMoveTheContent() {
        trackpad.down(1, 0f, 100f, 0)
        trackpad.down(2, 40f, 100f, 5)
        (1..10).forEach {
            trackpad.move(1, 0f, 100f - it * 5f, it * 8L)
            trackpad.move(2, 40f, 100f - it * 5f, it * 8L)
        }
        trackpad.up(1, 120)
        trackpad.up(2, 125)
        // The fingers went up 50 dp, so the view scrolls down, as the content follows them.
        assertEquals(0f, out.moved.first)
        assertTrue(out.scrolled.second > 0f)
        assertEquals(listOf("scrolled"), out.events)
    }

    @Test
    fun aTapAndATouchDragWithTheButtonHeld() {
        trackpad.down(1, 0f, 0f, 0)
        trackpad.up(1, 60)
        trackpad.down(1, 0f, 0f, 160)
        (1..10).forEach { trackpad.move(1, it * 4f, 0f, 160 + it * 8L) }
        trackpad.up(1, 600)
        trackpad.expire(2000)
        assertEquals(listOf("left down", "left up"), out.events)
        assertTrue(out.moved.first > 0f)
    }

    @Test
    fun twoQuickTapsAreADoubleClick() {
        trackpad.down(1, 0f, 0f, 0)
        trackpad.up(1, 60)
        trackpad.down(1, 0f, 0f, 160)
        trackpad.up(1, 220)
        trackpad.expire(2000)
        assertEquals(listOf("left down", "left up", "click left"), out.events)
    }

    @Test
    fun aTapThenATwoFingerTapIsAClickAndARightClick() {
        trackpad.down(1, 0f, 0f, 0)
        trackpad.up(1, 60)
        trackpad.down(1, 0f, 0f, 160)
        trackpad.down(2, 30f, 0f, 170)
        trackpad.up(1, 240)
        trackpad.up(2, 250)
        assertEquals(listOf("left down", "left up", "click right"), out.events)
    }

    @Test
    fun aLongPressIsNotATap() {
        trackpad.down(1, 0f, 0f, 0)
        trackpad.up(1, 800)
        trackpad.expire(5000)
        assertEquals(emptyList(), out.events)
    }

    @Test
    fun cancellingLetsGoOfADrag() {
        trackpad.down(1, 0f, 0f, 0)
        trackpad.up(1, 60)
        trackpad.down(1, 0f, 0f, 160)
        trackpad.cancel()
        assertEquals(listOf("left down", "left up"), out.events)
    }
}

class ComposerTest {
    @Test
    fun aRewrittenWordSendsOnlyWhatChanged() {
        val composer = Composer()
        assertEquals(Edit(0, "h"), composer.compose("h"))
        assertEquals(Edit(0, "el"), composer.compose("hel"))
        assertEquals(Edit(2, "y"), composer.compose("hy"))
        assertEquals(Edit(1, "ey "), composer.commit("hey "))
        assertEquals(Edit(0, "next"), composer.commit("next"))
    }

    @Test
    fun finishingKeepsWhatWasSent() {
        val composer = Composer()
        composer.compose("ok")
        composer.finish()
        assertEquals("", composer.sent)
        assertEquals(Edit(0, "a"), composer.compose("a"))
    }

    @Test
    fun anEmojiTakesOneBackspace() {
        assertEquals(Edit(1, "!"), edit("hi😀", "hi!"))
        assertEquals(Edit(3, ""), edit("abc", ""))
    }
}

class KeyNameTest {
    @Test
    fun namesTheKeysKeyboardsSendAsEvents() {
        assertEquals("Backspace", keyName(KeyEvent.KEYCODE_DEL))
        assertEquals("Enter", keyName(KeyEvent.KEYCODE_ENTER))
        assertEquals("ArrowLeft", keyName(KeyEvent.KEYCODE_DPAD_LEFT))
        assertEquals("F1", keyName(KeyEvent.KEYCODE_F1))
        assertEquals("F12", keyName(KeyEvent.KEYCODE_F12))
        assertNull(keyName(KeyEvent.KEYCODE_BACK))
        assertNull(keyName(KeyEvent.KEYCODE_A))
    }
}
