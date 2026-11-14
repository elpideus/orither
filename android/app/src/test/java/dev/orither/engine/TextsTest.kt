package dev.orither.engine

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

class TextsTest {
    private val columns = listOf("_id", "address", "body")

    @Test
    fun readsRowsWithCommasAndLineBreaksInTheLastColumn() {
        val said = "Row: 0 _id=7, address=+39 333 000, body=Hi, address=x, it's me\nsecond line\nRow: 1 _id=8, address=NULL, body=Il tuo codice è 482913\n"
        val rows = parseRows(said, columns)
        assertEquals(2, rows.size)
        assertEquals("7", rows[0]["_id"])
        assertEquals("+39 333 000", rows[0]["address"])
        assertEquals("Hi, address=x, it's me\nsecond line", rows[0]["body"])
        assertNull(rows[1]["address"])
        assertEquals("Il tuo codice è 482913", rows[1]["body"])
    }

    @Test
    fun readsNoRowsAsEmpty() {
        assertTrue(parseRows("No result found.\n", columns).isEmpty())
        assertTrue(parseRows("", columns).isEmpty())
    }

    @Test(expected = IllegalStateException::class)
    fun refusesAnError() {
        parseRows("Error while accessing provider:sms\njava.lang.SecurityException: nope\n", columns)
    }
}
