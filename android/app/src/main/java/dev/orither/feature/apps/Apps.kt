package dev.orither.feature.apps

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import androidx.core.graphics.drawable.toBitmap
import dev.orither.engine.EngineClient
import dev.orither.feature.Feature
import dev.orither.feature.body
import dev.orither.feature.text
import dev.orither.link.LinkStream
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive

private const val ICON_SIZE = 192
private const val ICON_QUALITY = 100

@Serializable
private data class App(val `package`: String, val label: String)

/** The apps on the phone, and opening them from the computer. */
internal class Apps(private val context: Context, private val engine: EngineClient) : Feature {
    override val id = "apps"
    private val packages = context.packageManager

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement = withContext(Dispatchers.IO) {
        when (method) {
            "list" -> body(list())
            "open" -> {
                open(body.text("package"))
                JsonPrimitive(true)
            }
            else -> throw UnsupportedOperationException("apps has no method $method")
        }
    }

    override suspend fun stream(method: String, body: JsonElement, stream: LinkStream, peer: String) {
        check(method == "icon") { "apps has no stream $method" }
        val icon = withContext(Dispatchers.IO) {
            packages.getApplicationIcon(body.text("package")).toBitmap(ICON_SIZE, ICON_SIZE)
        }
        stream.send { icon.compress(Bitmap.CompressFormat.PNG, ICON_QUALITY, it) }
    }

    private fun launcher(): Intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)

    private fun list(): List<App> =
        packages.queryIntentActivities(launcher(), 0)
            .map { App(it.activityInfo.packageName, it.loadLabel(packages).toString()) }
            .filter { it.`package` != context.packageName }
            .distinctBy { it.`package` }
            .sortedBy { it.label.lowercase() }

    /** Android blocks apps from opening others in the background, but not the engine. */
    private suspend fun open(name: String) {
        val intent = checkNotNull(packages.getLaunchIntentForPackage(name)) { "This app can't be opened" }
        val component = checkNotNull(intent.component) { "This app can't be opened" }
        if (!runCatching { engine.startActivity(arguments(component)) }.getOrDefault(false)) {
            context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }

    private fun arguments(component: ComponentName) = listOf(
        "-a", Intent.ACTION_MAIN, "-c", Intent.CATEGORY_LAUNCHER, "-n", component.flattenToShortString(),
    )
}
