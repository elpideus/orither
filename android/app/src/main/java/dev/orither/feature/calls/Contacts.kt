package dev.orither.feature.calls

import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.database.Cursor
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.provider.CallLog
import android.provider.ContactsContract
import android.provider.ContactsContract.CommonDataKinds.Phone
import android.provider.ContactsContract.Data
import android.provider.ContactsContract.PhoneLookup
import androidx.core.graphics.scale
import kotlinx.serialization.Serializable

private const val ITEM = "vnd.android.cursor.item/"

/**
 * The call actions of apps that are known to add them to contacts, with their apps.
 * Others are found by their type, see [callMime]; these work even when Android hides
 * which app opens them.
 */
private val KNOWN_CALL_MIMES = mapOf(
    "${ITEM}vnd.com.whatsapp.voip.call" to "com.whatsapp",
    "${ITEM}vnd.com.whatsapp.video.call" to "com.whatsapp",
    "${ITEM}vnd.com.whatsapp.w4b.voip.call" to "com.whatsapp.w4b",
    "${ITEM}vnd.com.whatsapp.w4b.video.call" to "com.whatsapp.w4b",
    "${ITEM}vnd.org.telegram.messenger.android.call" to "org.telegram.messenger",
    "${ITEM}vnd.org.telegram.messenger.android.call.video" to "org.telegram.messenger",
    "${ITEM}vnd.org.thoughtcrime.securesms.call" to "org.thoughtcrime.securesms",
)

@Serializable
internal data class PhoneNumber(val number: String, val label: String)

/** An app's call to one contact, which `place` starts by its contacts data row. */
@Serializable
internal data class AppCall(val data: Long, val app: String, val appLabel: String, val video: Boolean)

@Serializable
internal data class Contact(
    val id: Long,
    val name: String,
    val picture: Boolean,
    val numbers: List<PhoneNumber>,
    val apps: List<AppCall>,
)

@Serializable
internal data class Recent(
    val number: String,
    val name: String?,
    /** incoming, outgoing, missed, rejected, blocked or voicemail. */
    val type: String,
    val date: Long,
    /** In seconds. */
    val duration: Long,
    val picture: Boolean,
    /** The contact, for `contact-picture`, when the number belongs to one. */
    val contact: Long?,
)

/** A contact found by a phone number. */
internal data class Found(val id: Long, val name: String, val picture: Boolean)

/**
 * Whether a contacts data row is an app's call action, and if so whether it is a
 * video call. Apps that offer calls from the phone's contacts add rows of a type of
 * their own, which names a call. Viber's package name has "voip" in it, so that
 * word alone would take its message rows for calls too.
 */
internal fun callMime(mime: String): Boolean? {
    val kind = mime.lowercase()
    if (!kind.startsWith(ITEM) || "call" !in kind.removePrefix(ITEM)) return null
    return "video" in kind
}

/** Whether a contact matches what the user typed, by name or by the digits of a number. */
internal fun matches(name: String, numbers: List<String>, query: String): Boolean {
    val wanted = query.trim()
    if (wanted.isEmpty()) return true
    if (fold(wanted) in fold(name)) return true
    val digits = wanted.filter(Char::isDigit)
    return digits.isNotEmpty() && digits.length == wanted.count { !it.isWhitespace() && it !in "+-()." } &&
        numbers.any { digits in it.filter(Char::isDigit) }
}

internal fun recentType(type: Int): String = when (type) {
    CallLog.Calls.OUTGOING_TYPE -> "outgoing"
    CallLog.Calls.MISSED_TYPE -> "missed"
    CallLog.Calls.VOICEMAIL_TYPE -> "voicemail"
    CallLog.Calls.REJECTED_TYPE -> "rejected"
    CallLog.Calls.BLOCKED_TYPE -> "blocked"
    else -> "incoming"
}

/** The phone's contacts and call history, as a dialer on the computer needs them. */
internal class Contacts(private val context: Context) {
    private val resolver get() = context.contentResolver
    private val packages get() = context.packageManager

    /** Contacts with a number or an app's call, sorted by name. */
    fun find(query: String?, limit: Int): List<Contact> {
        val columns = arrayOf(Data._ID, Data.CONTACT_ID, Data.DISPLAY_NAME, Data.MIMETYPE, Data.DATA1, Data.DATA2, Data.DATA3, Data.PHOTO_ID)
        val selection = "${Data.MIMETYPE} = ? OR ${Data.MIMETYPE} LIKE ?"
        val rows = resolver.query(
            Data.CONTENT_URI, columns, selection, arrayOf(Phone.CONTENT_ITEM_TYPE, "$ITEM%call%"),
            "${Data.DISPLAY_NAME} COLLATE LOCALIZED ASC",
        ) ?: return emptyList()
        val found = LinkedHashMap<Long, Contact>()
        val openers = HashMap<String, Pair<String, String>?>()
        rows.use {
            while (it.moveToNext()) {
                val id = it.getLong(1)
                val contact = found[id] ?: Contact(id, it.text(2), !it.isNull(7) && it.getLong(7) != 0L, emptyList(), emptyList())
                found[id] = if (it.text(3) == Phone.CONTENT_ITEM_TYPE) contact.withNumber(it) else contact.withApp(it, openers)
            }
        }
        val wanted = query.orEmpty()
        return found.values
            .filter { it.numbers.isNotEmpty() || it.apps.isNotEmpty() }
            .filter { matches(it.name, it.numbers.map(PhoneNumber::number), wanted) }
            .take(limit)
    }

    /** The contact with the number in this row, which it has already if the digits are the same. */
    private fun Contact.withNumber(row: Cursor): Contact {
        val number = row.text(4).takeIf(String::isNotBlank) ?: return this
        val digits = number.filter(Char::isDigit)
        if (numbers.any { it.number.filter(Char::isDigit) == digits }) return this
        val label = Phone.getTypeLabel(context.resources, row.getInt(5), row.getString(6)).toString()
        return copy(numbers = numbers + PhoneNumber(number, label))
    }

    /** The contact with the app's call in this row, if an app on the phone opens it. [openers] remembers which does. */
    private fun Contact.withApp(row: Cursor, openers: MutableMap<String, Pair<String, String>?>): Contact {
        val mime = row.text(3)
        val video = callMime(mime) ?: return this
        val data = row.getLong(0)
        val opener = if (mime in openers) openers[mime] else app(mime, data).also { openers[mime] = it }
        val (app, label) = opener ?: return this
        return copy(apps = apps + AppCall(data, app, label, video))
    }

    /** The latest calls, newest first. */
    fun recents(limit: Int): List<Recent> {
        val columns = arrayOf(CallLog.Calls.NUMBER, CallLog.Calls.CACHED_NAME, CallLog.Calls.TYPE, CallLog.Calls.DATE, CallLog.Calls.DURATION)
        val uri = CallLog.Calls.CONTENT_URI.buildUpon().appendQueryParameter(CallLog.Calls.LIMIT_PARAM_KEY, "$limit").build()
        val rows = resolver.query(uri, columns, null, null, "${CallLog.Calls.DATE} DESC") ?: return emptyList()
        val people = HashMap<String, Found?>()
        return rows.use {
            buildList {
                while (it.moveToNext()) {
                    val number = it.text(0)
                    val contact = number.takeIf(String::isNotBlank)?.let { known -> people.getOrPut(known) { lookup(known) } }
                    val name = contact?.name ?: it.getString(1)?.takeIf(String::isNotBlank)
                    add(Recent(number, name, recentType(it.getInt(2)), it.getLong(3), it.getLong(4), contact?.picture == true, contact?.id))
                }
            }
        }
    }

    /** The contact a phone number belongs to. */
    fun lookup(number: String): Found? = runCatching {
        val uri = Uri.withAppendedPath(PhoneLookup.CONTENT_FILTER_URI, Uri.encode(number))
        resolver.query(uri, arrayOf(PhoneLookup._ID, PhoneLookup.DISPLAY_NAME, PhoneLookup.PHOTO_URI), null, null, null)?.use {
            if (it.moveToFirst()) Found(it.getLong(0), it.text(1), !it.isNull(2)) else null
        }
    }.getOrNull()

    /** What opens an app's call to a contact: its data row, and the row's type. */
    fun callAction(data: Long): Pair<Uri, String>? {
        val uri = ContentUris.withAppendedId(Data.CONTENT_URI, data)
        val mime = resolver.query(uri, arrayOf(Data.MIMETYPE), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        } ?: return null
        return (uri to mime).takeIf { callMime(mime) != null }
    }

    fun picture(contact: Long): Bitmap? {
        val uri = ContentUris.withAppendedId(ContactsContract.Contacts.CONTENT_URI, contact)
        val photo = ContactsContract.Contacts.openContactPhotoInputStream(resolver, uri, true)?.use(BitmapFactory::decodeStream)
            ?: return null
        val side = minOf(photo.width, photo.height, PICTURE_SIZE * 2)
        return if (photo.width == side && photo.height == side) photo else photo.scale(side, side)
    }

    /** The app that opens a call action, and its name. */
    private fun app(mime: String, data: Long): Pair<String, String>? {
        val view = Intent(Intent.ACTION_VIEW).setDataAndType(ContentUris.withAppendedId(Data.CONTENT_URI, data), mime)
        val opener = packages.queryIntentActivities(view, 0).firstOrNull()?.activityInfo?.packageName
            ?: KNOWN_CALL_MIMES[mime]?.takeIf(::installed)
            ?: return null
        return opener to label(opener)
    }

    private fun installed(app: String) = runCatching { packages.getApplicationInfo(app, 0) }.isSuccess

    private fun label(app: String): String =
        runCatching { packages.getApplicationLabel(packages.getApplicationInfo(app, 0)).toString() }.getOrDefault(app)
}

private fun Cursor.text(column: Int): String = getString(column).orEmpty()
