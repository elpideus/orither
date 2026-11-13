package dev.orither.feature.gallery

import android.Manifest
import android.content.ContentResolver
import android.content.ContentUris
import android.content.Context
import android.graphics.Bitmap
import android.net.Uri
import android.os.Bundle
import android.provider.MediaStore
import android.provider.MediaStore.Files.FileColumns
import android.provider.MediaStore.MediaColumns
import android.util.Size
import dev.orither.R
import dev.orither.feature.Access
import dev.orither.feature.Feature
import dev.orither.feature.body
import dev.orither.feature.number
import dev.orither.feature.text
import dev.orither.feature.textOrNull
import dev.orither.link.LinkStream
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

private const val THUMBNAIL_QUALITY = 80
private const val IMAGE = FileColumns.MEDIA_TYPE_IMAGE
private const val VIDEO = FileColumns.MEDIA_TYPE_VIDEO
private const val MEDIA_ONLY = "${FileColumns.MEDIA_TYPE} IN ($IMAGE, $VIDEO)"

@Serializable
private data class Album(val id: String, val name: String, val count: Int)

@Serializable
private data class Item(
    val id: String,
    val name: String,
    val date: Long,
    val video: Boolean,
    val width: Int,
    val height: Int,
)

/** The phone's photos and videos, newest first. */
internal class Gallery(private val context: Context) : Feature {
    override val id = "gallery"
    override val access = Access.Permissions(
        R.string.access_photos,
        R.string.access_photos_about,
        R.drawable.ic_image,
        listOf(Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VIDEO),
    )
    private val resolver: ContentResolver = context.contentResolver
    private val media = MediaStore.Files.getContentUri(MediaStore.VOLUME_EXTERNAL)

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement = withContext(Dispatchers.IO) {
        when (method) {
            "albums" -> body(albums())
            "items" -> body(items(body.textOrNull("album"), body.number("offset"), body.number("limit")))
            else -> throw UnsupportedOperationException("gallery has no method $method")
        }
    }

    override suspend fun stream(method: String, body: JsonElement, stream: LinkStream, peer: String) {
        val uri = uri(body.text("id"))
        withContext(Dispatchers.IO) {
            when (method) {
                "thumbnail" -> {
                    val size = body.number("size")
                    val image = resolver.loadThumbnail(uri, Size(size, size), null)
                    stream.send { image.compress(Bitmap.CompressFormat.JPEG, THUMBNAIL_QUALITY, it) }
                }
                "original" -> stream.send(checkNotNull(resolver.openFileDescriptor(uri, "r")))
                else -> stream.close()
            }
        }
    }

    private fun albums(): List<Album> {
        val columns = arrayOf(MediaColumns.BUCKET_ID, MediaColumns.BUCKET_DISPLAY_NAME)
        val counts = LinkedHashMap<String, Album>()
        resolver.query(media, columns, MEDIA_ONLY, null, "${MediaColumns.DATE_ADDED} DESC")?.use { rows ->
            while (rows.moveToNext()) {
                val id = rows.getString(0) ?: continue
                val album = counts[id] ?: Album(id, rows.getString(1).orEmpty(), 0)
                counts[id] = album.copy(count = album.count + 1)
            }
        }
        return counts.values.toList()
    }

    private fun items(album: String?, offset: Int, limit: Int): List<Item> {
        val columns = arrayOf(
            MediaColumns._ID, FileColumns.MEDIA_TYPE, MediaColumns.DISPLAY_NAME,
            MediaColumns.DATE_ADDED, MediaColumns.WIDTH, MediaColumns.HEIGHT,
        )
        val query = Bundle().apply {
            putString(ContentResolver.QUERY_ARG_SQL_SELECTION, MEDIA_ONLY + album?.let { " AND ${MediaColumns.BUCKET_ID} = ?" }.orEmpty())
            putStringArray(ContentResolver.QUERY_ARG_SQL_SELECTION_ARGS, album?.let { arrayOf(it) })
            putStringArray(ContentResolver.QUERY_ARG_SORT_COLUMNS, arrayOf(MediaColumns.DATE_ADDED))
            putInt(ContentResolver.QUERY_ARG_SORT_DIRECTION, ContentResolver.QUERY_SORT_DIRECTION_DESCENDING)
            putInt(ContentResolver.QUERY_ARG_OFFSET, offset)
            putInt(ContentResolver.QUERY_ARG_LIMIT, limit)
        }
        val rows = resolver.query(media, columns, query, null) ?: return emptyList()
        return rows.use {
            buildList {
                while (it.moveToNext()) {
                    val type = it.getInt(1)
                    add(
                        Item(
                            id = "$type:${it.getLong(0)}",
                            name = it.getString(2).orEmpty(),
                            date = it.getLong(3) * 1000,
                            video = type == VIDEO,
                            width = it.getInt(4),
                            height = it.getInt(5),
                        ),
                    )
                }
            }
        }
    }

    private fun uri(id: String): Uri {
        val (type, row) = id.split(':')
        val collection = if (type.toInt() == VIDEO) MediaStore.Video.Media.EXTERNAL_CONTENT_URI else MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        return ContentUris.withAppendedId(collection, row.toLong())
    }
}
