package dev.orither.feature.calls

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class LinesTest {
    @Test
    fun anIncomingCallRingsThenIsTaken() {
        val ringing = Lines().after(Phase.RINGING, null, null, 10, "a")
        assertEquals(Line("a", null, ringing = true, since = 10), ringing.current)
        val numbered = ringing.numbered("+391234")
        assertEquals("+391234", numbered.current?.number)
        val taken = numbered.after(Phase.OFFHOOK, null, null, 20, "b")
        assertEquals(Line("a", "+391234", ringing = false, since = 20), taken.current)
        assertEquals(Lines(), taken.after(Phase.IDLE, null, null, 30, "c"))
    }

    @Test
    fun anOutgoingCallTakesTheNumberTheComputerDialed() {
        val calling = Lines().after(Phase.OFFHOOK, null, "555", 10, "a")
        assertEquals(Line("a", "555", ringing = false, since = 10), calling.current)
    }

    @Test
    fun aSecondCallWaitsBesideTheFirst() {
        val talking = Lines().after(Phase.OFFHOOK, null, null, 10, "a")
        val waiting = talking.after(Phase.RINGING, null, null, 20, "b").numbered("777")
        assertEquals("a", waiting.current?.id)
        assertEquals(Line("b", "777", ringing = true, since = 20), waiting.waiting)
        // Telephony cannot say whether it was taken or turned down, so the first call stays.
        val after = waiting.after(Phase.OFFHOOK, null, null, 30, "c")
        assertEquals("a", after.current?.id)
        assertNull(after.waiting)
    }

    @Test
    fun aNumberNeverReplacesOneAlreadyKnown() {
        val ringing = Lines().after(Phase.RINGING, "111", null, 10, "a")
        assertEquals("111", ringing.numbered("222").current?.number)
        assertEquals("111", ringing.after(Phase.RINGING, "222", null, 11, "b").current?.number)
    }
}
