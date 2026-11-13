package dev.orither.feature.whatsapp

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class ChatNameTest {
    @Test
    fun aNamedChatKeepsItsName() {
        assertEquals("Family", chatName("Family", listOf("Ana", null)))
    }

    @Test
    fun theCountIsNotPartOfTheName() {
        assertEquals("Family", chatName("Family (3 messages)", listOf("Ana")))
    }

    @Test
    fun aChatWithoutANameIsTheOtherPerson() {
        assertEquals("Ana", chatName(null, listOf("Ana", null)))
    }

    @Test
    fun theUserAloneNamesNothing() {
        assertNull(chatName(null, listOf(null)))
    }

    @Test
    fun onlyMessagesSinceTheUserWroteWait() {
        assertEquals(2, waiting(listOf("Ana", null, "Ana", "Marco")))
        assertEquals(0, waiting(listOf("Ana", null)))
    }
}
