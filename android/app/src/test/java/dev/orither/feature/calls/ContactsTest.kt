package dev.orither.feature.calls

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class ContactsTest {
    @Test
    fun appCallActionsAreFoundByTheirType() {
        assertEquals(false, callMime("vnd.android.cursor.item/vnd.com.whatsapp.voip.call"))
        assertEquals(true, callMime("vnd.android.cursor.item/vnd.com.whatsapp.video.call"))
        assertEquals(false, callMime("vnd.android.cursor.item/vnd.org.telegram.messenger.android.call"))
        assertEquals(true, callMime("vnd.android.cursor.item/vnd.org.telegram.messenger.android.call.video"))
        assertEquals(false, callMime("vnd.android.cursor.item/vnd.org.thoughtcrime.securesms.call"))
        assertEquals(false, callMime("vnd.android.cursor.item/vnd.com.viber.voip.viber_number_call"))
    }

    @Test
    fun otherRowsAreNotCalls() {
        assertNull(callMime("vnd.android.cursor.item/phone_v2"))
        assertNull(callMime("vnd.android.cursor.item/vnd.com.whatsapp.profile"))
        assertNull(callMime("vnd.android.cursor.item/vnd.com.viber.voip.viber_number_message"))
        assertNull(callMime("vnd.android.cursor.dir/vnd.example.call"))
    }

    @Test
    fun contactsMatchByNameWithoutCaseOrAccents() {
        assertTrue(matches("José Álvarez", listOf("+34 600 000 000"), "jose"))
        assertTrue(matches("Ana", emptyList(), ""))
        assertFalse(matches("Ana", emptyList(), "marco"))
    }

    @Test
    fun contactsMatchByTheDigitsOfANumber() {
        assertTrue(matches("Ana", listOf("+39 333 123 4567"), "3331234"))
        assertTrue(matches("Ana", listOf("+39 333 123 4567"), "+39 333"))
        assertFalse(matches("Ana", listOf("+39 333 123 4567"), "999"))
        // Digits in a name search are not a number.
        assertFalse(matches("Ana", listOf("+39 333 123 4567"), "ana 3"))
    }

    @Test
    fun recentCallsTakeTheirKind() {
        assertEquals("incoming", recentType(1))
        assertEquals("outgoing", recentType(2))
        assertEquals("missed", recentType(3))
        assertEquals("voicemail", recentType(4))
        assertEquals("rejected", recentType(5))
        assertEquals("blocked", recentType(6))
        assertEquals("incoming", recentType(7))
    }
}
