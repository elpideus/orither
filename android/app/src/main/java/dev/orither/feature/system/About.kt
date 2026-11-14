package dev.orither.feature.system

import android.app.WallpaperManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.Bitmap
import android.graphics.drawable.BitmapDrawable
import android.os.BatteryManager
import android.os.Build
import android.os.Environment
import android.os.PowerManager
import android.os.StatFs
import android.provider.Settings
import androidx.core.graphics.drawable.toBitmap
import androidx.core.graphics.scale
import androidx.core.net.toUri
import dev.orither.R
import dev.orither.feature.Access
import dev.orither.feature.Feature
import dev.orither.feature.body
import dev.orither.link.Link
import dev.orither.link.LinkStream
import dev.orither.link.Peer
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

private const val WALLPAPER_WIDTH = 720
private const val WALLPAPER_QUALITY = 82

@Serializable
private data class Battery(val level: Int, val charging: Boolean)

@Serializable
private data class Storage(val free: Long, val total: Long)

@Serializable
private data class Details(val model: String, val android: String, val storage: Storage)

/** What the computer shows about the phone itself. */
internal class About(private val context: Context) : Feature {
    override val id = "system"
    override val access = Access.Special(
        R.string.access_battery,
        R.string.access_battery_about,
        R.drawable.ic_battery,
        isGranted = { it.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(it.packageName) },
        settings = { Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, "package:${it.packageName}".toUri()) },
        grants = listOf(listOf("cmd", "deviceidle", "whitelist", "+dev.orither")),
    )
    private var reported: Battery? = null

    init {
        val changes = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                val battery = battery(intent)
                if (battery != reported) {
                    reported = battery
                    Link.broadcast(id, "battery", body(battery))
                }
            }
        }
        context.registerReceiver(changes, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
    }

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement = when (method) {
        "battery" -> body(battery(context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))))
        "about" -> body(details())
        else -> super.call(method, body, peer)
    }

    override suspend fun stream(method: String, body: JsonElement, stream: LinkStream, peer: String) {
        when (method) {
            "wallpaper" -> {
                val wallpaper = withContext(Dispatchers.IO) { wallpaper() }
                stream.send { wallpaper.compress(Bitmap.CompressFormat.JPEG, WALLPAPER_QUALITY, it) }
            }
            else -> stream.close()
        }
    }

    override fun connected(peer: Peer) {
        reported?.let { Link.notify(peer.id, id, "battery", body(it)) }
    }

    private fun battery(intent: Intent?): Battery {
        val level = intent?.getIntExtra(BatteryManager.EXTRA_LEVEL, 0) ?: 0
        val scale = intent?.getIntExtra(BatteryManager.EXTRA_SCALE, 100) ?: 100
        val status = intent?.getIntExtra(BatteryManager.EXTRA_STATUS, 0)
        val charging = status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL
        return Battery((level * 100f / scale).roundToInt(), charging)
    }

    private fun details(): Details {
        val stats = StatFs(Environment.getDataDirectory().path)
        val name = Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME) ?: Build.MODEL
        return Details(name, Build.VERSION.RELEASE, Storage(stats.availableBytes, stats.totalBytes))
    }

    private fun wallpaper(): Bitmap {
        val drawable = checkNotNull(WallpaperManager.getInstance(context).drawable) { "no wallpaper" }
        val bitmap = (drawable as? BitmapDrawable)?.bitmap ?: drawable.toBitmap()
        return bitmap.scale(WALLPAPER_WIDTH, bitmap.height * WALLPAPER_WIDTH / bitmap.width)
    }
}
