package dev.orither.feature

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import androidx.annotation.DrawableRes
import androidx.annotation.StringRes
import androidx.core.content.ContextCompat

/** Something the user allows once so that a feature can work. */
sealed interface Access {
    @get:StringRes val title: Int

    @get:StringRes val about: Int

    @get:DrawableRes val icon: Int

    /** The shell commands that allow this without asking, when the engine runs. */
    val grants: List<List<String>>

    fun isGranted(context: Context): Boolean

    /** Android permissions, asked for with the system's own dialog. */
    class Permissions(
        override val title: Int,
        override val about: Int,
        override val icon: Int,
        val names: List<String>,
    ) : Access {
        override val grants = names.map { listOf("pm", "grant", PACKAGE, it) }

        override fun isGranted(context: Context) = names.all {
            ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED
        }
    }

    /** A switch that lives in Android's settings. [grants] are the commands that turn it on, in order. */
    class Special(
        override val title: Int,
        override val about: Int,
        override val icon: Int,
        private val isGranted: (Context) -> Boolean,
        val settings: (Context) -> Intent,
        override val grants: List<List<String>>,
    ) : Access {

        override fun isGranted(context: Context) = isGranted.invoke(context)
    }

    /** A permission Android never asks the user about, so only the engine can give it. */
    class Shell(
        override val title: Int,
        override val about: Int,
        override val icon: Int,
        private val name: String,
    ) : Access {
        override val grants = listOf(listOf("pm", "grant", PACKAGE, name))

        override fun isGranted(context: Context) =
            ContextCompat.checkSelfPermission(context, name) == PackageManager.PERMISSION_GRANTED
    }

    companion object {
        private const val PACKAGE = "dev.orither"

        /**
         * Turns on notification access for [listener]. Android 13 and later keep it from apps
         * installed from a file ("restricted settings") until this app-op allows it; on older
         * versions the first command fails harmlessly.
         */
        fun listenerGrants(listener: String): List<List<String>> = listOf(
            listOf("appops", "set", PACKAGE, "ACCESS_RESTRICTED_SETTINGS", "allow"),
            listOf("cmd", "notification", "allow_listener", listener),
        )
    }
}
