package dev.orither.feature.calls

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class RingingTest {
    @Test
    fun buttonsAreReadInSeveralLanguages() {
        assertEquals(Act.ANSWER, actOf("Answer", 0))
        assertEquals(Act.ANSWER, actOf("RISPONDI", 0))
        assertEquals(Act.ANSWER, actOf("Répondre", 0))
        assertEquals(Act.ANSWER, actOf("接听", 0))
        assertEquals(Act.DECLINE, actOf("Decline", 0))
        assertEquals(Act.DECLINE, actOf("Rifiuta", 0))
        assertEquals(Act.DECLINE, actOf("Отклонить", 0))
        assertEquals(Act.HANG_UP, actOf("Hang up", 0))
        assertEquals(Act.HANG_UP, actOf("End call", 0))
        assertEquals(Act.HANG_UP, actOf("Beenden", 0))
    }

    @Test
    fun wordsInsideOtherWordsDoNotCount() {
        assertNull(actOf("Send message", 0))
        assertNull(actOf("Mute", 0))
        assertNull(actOf(null, 0))
    }

    @Test
    fun aCallButtonWithoutAKnownTitleAnswers() {
        assertEquals(Act.ANSWER, actOf("✓", 10))
        assertEquals(Act.DECLINE, actOf("Decline", 10))
    }

    @Test
    fun aCallStyleIsAlwaysACall() {
        assertTrue(isCall(null, "android.app.Notification\$CallStyle", 0, ongoing = false, urgent = false))
        assertTrue(isCall(null, null, 1, ongoing = false, urgent = false))
    }

    @Test
    fun aPlainCallNotificationMustStayOrRing() {
        assertTrue(isCall("call", null, 0, ongoing = true, urgent = false))
        assertTrue(isCall("call", null, 0, ongoing = false, urgent = true))
        // A missed call can be marked a call too, but it can be swiped away and never rang through the screen.
        assertFalse(isCall("call", null, 0, ongoing = false, urgent = false))
        assertFalse(isCall("msg", null, 0, ongoing = true, urgent = true))
    }

    @Test
    fun theStyleSaysWhetherItRings() {
        assertEquals(RINGING, callState(1, setOf(Act.DECLINE)))
        assertEquals(ACTIVE, callState(2, setOf(Act.ANSWER)))
        assertEquals(RINGING, callState(0, setOf(Act.ANSWER, Act.DECLINE)))
        assertEquals(ACTIVE, callState(0, setOf(Act.HANG_UP)))
    }

    @Test
    fun videoCallsSayVideo() {
        assertTrue(mentionsVideo("Incoming video call"))
        assertTrue(mentionsVideo("Videochiamata in arrivo"))
        assertTrue(mentionsVideo("Videollamada entrante"))
        assertFalse(mentionsVideo("Incoming voice call"))
    }
}
