package dev.orither.feature.clipboard

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Environment
import android.os.ParcelFileDescriptor
import androidx.core.content.FileProvider
import dev.orither.engine.EngineClient
import dev.orither.feature.Feature
import dev.orither.feature.flag
import dev.orither.feature.text
import dev.orither.link.Link
import dev.orither.link.LinkStream
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import org.json.JSONObject

private const val AUTHORITY = "dev.orither.files"
private const val LABEL = "Orither"
private const val RETRY_MS = 5_000L

/** One clipboard across both devices, for as long as the computer asks for it. */
internal class Clipboard(private val context: Context, private val engine: EngineClient) : Feature {
    override val id = "clipboard"
    private val clipboard = context.getSystemService(ClipboardManager::class.java)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val watchers = ConcurrentHashMap<String, Job>()
    private val incoming = ConcurrentHashMap<String, MutableList<Uri>>()
    private val folder = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS).resolve(LABEL)

    /** The last text from the computer, which must not be sent straight back. */
    @Volatile private var received: String? = null

    /** The last files from the computer, for the same reason. */
    @Volatile private var placed: Set<String> = emptySet()

    /** Android announces some changes twice. */
    @Volatile private var reported: String? = null

    override suspend fun notice(method: String, body: JsonElement, peer: String) {
        when (method) {
            "follow" -> if (body.flag("on")) follow(peer) else watchers.remove(peer)?.cancel()
            "set" -> set(body.text("text"))
        }
    }

    override suspend fun stream(method: String, body: JsonElement, stream: LinkStream, peer: String) {
        withContext(Dispatchers.IO) {
            when (method) {
                "read" -> stream.send(engine.read(body.text("uri")))
                "put" -> put(stream, body.text("name"), body.text("batch"), body.flag("last"))
                else -> stream.close()
            }
        }
    }

    override fun disconnected(peer: String) {
        watchers.remove(peer)?.cancel()
    }

    private fun follow(peer: String) {
        watchers[peer]?.cancel()
        watchers[peer] = scope.launch {
            while (isActive) {
                runCatching { engine.followClipboard { report(peer, it) } }
                delay(RETRY_MS)
            }
        }
    }

    private fun report(peer: String, change: JSONObject) {
        val text = change.optString("text").takeIf { change.has("text") }
        val files = change.getJSONArray("files").let { list ->
            List(list.length()) { list.getJSONObject(it).getString("uri") }.toSet()
        }
        val message = change.toString()
        if ((text != null && text == received) || (files.isNotEmpty() && files == placed) || message == reported) return
        reported = message
        Link.notify(peer, id, "changed", Json.parseToJsonElement(message))
    }

    private fun set(text: String) {
        received = text
        clipboard.setPrimaryClip(ClipData.newPlainText(LABEL, text))
    }

    /** Files land in Downloads, where they stay useful after the clipboard moves on. */
    private suspend fun put(stream: LinkStream, name: String, batch: String, last: Boolean) {
        folder.mkdirs()
        val target = folder.resolve(File(name).name)
        val mode = ParcelFileDescriptor.MODE_CREATE or ParcelFileDescriptor.MODE_TRUNCATE or ParcelFileDescriptor.MODE_WRITE_ONLY
        stream.receive(ParcelFileDescriptor.open(target, mode))
        MediaScannerConnection.scanFile(context, arrayOf(target.path), null, null)

        val files = incoming.getOrPut(batch) { mutableListOf() }
        files += FileProvider.getUriForFile(context, AUTHORITY, target)
        if (last) {
            incoming.remove(batch)
            setFiles(files)
        }
    }

    private fun setFiles(files: List<Uri>) {
        placed = files.mapTo(HashSet(), Uri::toString)
        val clip = ClipData.newUri(context.contentResolver, LABEL, files.first())
        files.drop(1).forEach { clip.addItem(ClipData.Item(it)) }
        clipboard.setPrimaryClip(clip)
    }
}
