package dev.orither.feature.files

import android.content.Context
import android.content.Intent
import android.media.MediaScannerConnection
import android.os.Environment
import android.os.ParcelFileDescriptor
import android.os.StatFs
import android.os.storage.StorageManager
import android.provider.Settings
import android.webkit.MimeTypeMap
import androidx.core.net.toUri
import dev.orither.R
import dev.orither.feature.Access
import dev.orither.feature.Feature
import dev.orither.feature.body
import dev.orither.feature.text
import dev.orither.link.LinkStream
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive

@Serializable
private data class Root(val name: String, val path: String, val free: Long, val total: Long)

@Serializable
private data class Entry(
    val name: String,
    val path: String,
    val folder: Boolean,
    val size: Long,
    val modified: Long,
    val mime: String?,
)

/** The phone's shared storage. Nothing outside the storage volumes is reachable. */
internal class Files(private val context: Context) : Feature {
    override val id = "files"
    override val access = Access.Special(
        R.string.access_files,
        R.string.access_files_about,
        R.drawable.ic_folder,
        isGranted = { Environment.isExternalStorageManager() },
        settings = {
            Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, "package:${it.packageName}".toUri())
        },
        grants = listOf(listOf("appops", "set", "--uid", "dev.orither", "MANAGE_EXTERNAL_STORAGE", "allow")),
    )

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement = withContext(Dispatchers.IO) {
        when (method) {
            "roots" -> body(roots())
            "list" -> body(list(reachable(body.text("path"))))
            "delete" -> {
                delete(reachable(body.text("path")))
                JsonPrimitive(true)
            }
            else -> throw UnsupportedOperationException("files has no method $method")
        }
    }

    override suspend fun stream(method: String, body: JsonElement, stream: LinkStream, peer: String) {
        withContext(Dispatchers.IO) {
            when (method) {
                "read" -> stream.send(ParcelFileDescriptor.open(reachable(body.text("path")), ParcelFileDescriptor.MODE_READ_ONLY))
                "write" -> receive(stream, reachable(body.text("folder")), body.text("name"))
                else -> stream.close()
            }
        }
    }

    private suspend fun receive(stream: LinkStream, folder: File, name: String) {
        val target = folder.resolve(File(name).name)
        val mode = ParcelFileDescriptor.MODE_CREATE or ParcelFileDescriptor.MODE_TRUNCATE or ParcelFileDescriptor.MODE_WRITE_ONLY
        stream.receive(ParcelFileDescriptor.open(target, mode))
        MediaScannerConnection.scanFile(context, arrayOf(target.path), null, null)
    }

    private fun roots(): List<Root> =
        context.getSystemService(StorageManager::class.java).storageVolumes.mapNotNull { volume ->
            val folder = volume.directory ?: return@mapNotNull null
            val stats = StatFs(folder.path)
            Root(volume.getDescription(context), folder.path, stats.availableBytes, stats.totalBytes)
        }

    private fun list(folder: File): List<Entry> =
        folder.listFiles().orEmpty()
            .filterNot(File::isHidden)
            .sortedWith(compareBy({ !it.isDirectory }, { it.name.lowercase() }))
            .map {
                Entry(
                    name = it.name,
                    path = it.path,
                    folder = it.isDirectory,
                    size = if (it.isDirectory) 0 else it.length(),
                    modified = it.lastModified(),
                    mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(it.extension.lowercase()),
                )
            }

    /** Only files and empty folders: a click on the computer never wipes a whole tree. */
    private fun delete(target: File) {
        check(target.delete()) { "This folder still has files in it" }
        MediaScannerConnection.scanFile(context, arrayOf(target.path), null, null)
    }

    private fun reachable(path: String): File {
        val file = File(path).canonicalFile
        val allowed = roots().any { file.startsWith(File(it.path).canonicalFile) }
        check(allowed) { "That place is outside the phone's storage" }
        return file
    }
}
