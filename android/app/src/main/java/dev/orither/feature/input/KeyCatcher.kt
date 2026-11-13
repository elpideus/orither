package dev.orither.feature.input

import android.annotation.SuppressLint
import android.content.Context
import android.text.InputType
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

/** Where typing goes. Key names are those of the `input` feature's `key` notice. */
interface Typist {
    fun type(text: String)

    fun press(key: String)

    /** Takes back characters the keyboard has since rewritten. */
    fun erase(count: Int)
}

/**
 * An invisible view that holds the soft keyboard and hands what it types to
 * [typist]. It describes itself as a password field shown in clear, so that
 * keyboards type character by character instead of composing words, and copes
 * with those that compose all the same.
 */
@SuppressLint("ViewConstructor")
class KeyCatcher(context: Context, private val typist: Typist) : View(context) {
    private val keyboard = context.getSystemService(InputMethodManager::class.java)

    /** Whether the keyboard should come back when the window does. */
    private var wanted = true

    init {
        isFocusable = true
        isFocusableInTouchMode = true
    }

    override fun onCheckIsTextEditor() = true

    override fun onCreateInputConnection(attributes: EditorInfo): InputConnection {
        attributes.inputType = InputType.TYPE_CLASS_TEXT or
            InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD or
            InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        attributes.imeOptions = EditorInfo.IME_ACTION_NONE or
            EditorInfo.IME_FLAG_NO_ENTER_ACTION or
            EditorInfo.IME_FLAG_NO_EXTRACT_UI or
            EditorInfo.IME_FLAG_NO_FULLSCREEN
        attributes.initialSelStart = 0
        attributes.initialSelEnd = 0
        return Connection(this, typist)
    }

    /** Brings the keyboard up, unless it is up already. */
    fun summon() {
        wanted = true
        if (!hasFocus()) requestFocus()
        val shown = ViewCompat.getRootWindowInsets(this)?.isVisible(WindowInsetsCompat.Type.ime()) == true
        if (!shown) keyboard.showSoftInput(this, 0)
    }

    fun dismiss() {
        wanted = false
        keyboard.hideSoftInputFromWindow(windowToken, 0)
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        // The keyboard only comes for a view that is in a window and focused.
        post { if (wanted) summon() }
    }

    override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
        super.onWindowFocusChanged(hasWindowFocus)
        if (hasWindowFocus && wanted) post { summon() }
    }

    /** A hardware keyboard's keys. Back and anything without a name stay the system's. */
    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean = handled(event, typist) || super.onKeyDown(keyCode, event)
}

/** Keys that keyboards send as events rather than text, by the names the computer knows. */
internal fun keyName(keyCode: Int): String? = when (keyCode) {
    KeyEvent.KEYCODE_DEL -> "Backspace"
    KeyEvent.KEYCODE_FORWARD_DEL -> "Delete"
    KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> "Enter"
    KeyEvent.KEYCODE_TAB -> "Tab"
    KeyEvent.KEYCODE_ESCAPE -> "Escape"
    KeyEvent.KEYCODE_SPACE -> "Space"
    KeyEvent.KEYCODE_DPAD_LEFT -> "ArrowLeft"
    KeyEvent.KEYCODE_DPAD_RIGHT -> "ArrowRight"
    KeyEvent.KEYCODE_DPAD_UP -> "ArrowUp"
    KeyEvent.KEYCODE_DPAD_DOWN -> "ArrowDown"
    KeyEvent.KEYCODE_MOVE_HOME -> "Home"
    KeyEvent.KEYCODE_MOVE_END -> "End"
    KeyEvent.KEYCODE_PAGE_UP -> "PageUp"
    KeyEvent.KEYCODE_PAGE_DOWN -> "PageDown"
    in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F12 -> "F${keyCode - KeyEvent.KEYCODE_F1 + 1}"
    else -> null
}

/** Passes a key press on, by name or by the character it types. Returns whether it was one. */
private fun handled(event: KeyEvent, typist: Typist): Boolean {
    if (event.action != KeyEvent.ACTION_DOWN) return keyName(event.keyCode) != null
    keyName(event.keyCode)?.let {
        typist.press(it)
        return true
    }
    val character = event.getUnicodeChar(event.metaState)
    if (character <= 0) return false
    typist.type(String(Character.toChars(character)))
    return true
}

/** How to turn the text the computer was sent into the text the keyboard now has. */
data class Edit(val erase: Int, val type: String)

/** The Backspaces and characters that turn [from] into [to], keeping what they start with. */
internal fun edit(from: String, to: String): Edit {
    // By code points, since the computer erases an emoji with one Backspace, not two.
    val before = from.codePoints().toArray()
    val after = to.codePoints().toArray()
    var same = 0
    while (same < before.size && same < after.size && before[same] == after[same]) same++
    return Edit(before.size - same, String(after, same, after.size - same))
}

/**
 * The word a keyboard is still composing, as far as the computer has it. A keyboard
 * may rewrite that word at every step, so each step goes out as an [Edit].
 */
class Composer {
    var sent = ""
        private set

    fun compose(text: String): Edit = edit(sent, text).also { sent = text }

    fun commit(text: String): Edit = edit(sent, text).also { sent = "" }

    fun finish() {
        sent = ""
    }
}

/**
 * Takes the keyboard's edits. The field it edits stays empty apart from the word in
 * composition, so keyboards send Backspace as a key event and never try to correct
 * text the computer already has.
 */
private class Connection(view: View, private val typist: Typist) : BaseInputConnection(view, false) {
    private val composer = Composer()

    private fun send(edit: Edit) {
        if (edit.erase > 0) typist.erase(edit.erase)
        if (edit.type.isNotEmpty()) typist.type(edit.type)
    }

    override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
        send(composer.compose(text?.toString().orEmpty()))
        return true
    }

    override fun finishComposingText(): Boolean {
        composer.finish()
        return true
    }

    override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
        send(composer.commit(text?.toString().orEmpty()))
        return true
    }

    override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
        repeat(beforeLength) { typist.press("Backspace") }
        repeat(afterLength) { typist.press("Delete") }
        return true
    }

    override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int) =
        deleteSurroundingText(beforeLength, afterLength)

    override fun sendKeyEvent(event: KeyEvent): Boolean {
        handled(event, typist)
        return true
    }

    override fun performEditorAction(actionCode: Int): Boolean {
        typist.press("Enter")
        return true
    }

    override fun getTextBeforeCursor(length: Int, flags: Int): CharSequence = composer.sent.takeLast(length)

    override fun getTextAfterCursor(length: Int, flags: Int): CharSequence = ""

    override fun getSelectedText(flags: Int): CharSequence? = null

    override fun setComposingRegion(start: Int, end: Int) = false
}
