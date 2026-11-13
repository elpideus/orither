package dev.orither.engine

import android.annotation.SuppressLint
import android.app.ActivityManager
import android.app.TaskInfo
import android.content.Context
import android.view.Display
import org.json.JSONObject

private const val OWN_PACKAGE = "dev.orither"
/** Enough to get past anything shown on other displays. */
private const val LOOKED_AT = 16
// WindowConfiguration's activity types, which the SDK keeps hidden.
private const val TYPE_UNDEFINED = 0
private const val TYPE_STANDARD = 1

/** The tasks on the phone, all of them, as only the shell user may see. */
@SuppressLint("DiscouragedPrivateApi")
internal class Tasks(shell: Context) {
    private val activities = shell.getSystemService(ActivityManager::class.java)

    /**
     * The app the user has in front on the phone's own screen, as its task and package.
     * Null when that is the home screen, the recent apps, or Orither itself.
     */
    fun front(): JSONObject? {
        @Suppress("DEPRECATION")
        val tasks = activities.getRunningTasks(LOOKED_AT)
        val task = tasks.firstOrNull { displayOf(it) == Display.DEFAULT_DISPLAY } ?: return null
        val app = (task.baseActivity ?: task.topActivity)?.packageName ?: return null
        if (typeOf(task) !in setOf(TYPE_UNDEFINED, TYPE_STANDARD) || app == OWN_PACKAGE) return null
        return JSONObject().put("task", task.taskId).put("package", app)
    }

    // Both fields are public in the framework, but hidden from apps until recently.
    private fun displayOf(task: TaskInfo): Int = TaskInfo::class.java.getField("displayId").getInt(task)

    private fun typeOf(task: TaskInfo): Int =
        runCatching { TaskInfo::class.java.getField("topActivityType").getInt(task) }.getOrDefault(TYPE_UNDEFINED)
}
