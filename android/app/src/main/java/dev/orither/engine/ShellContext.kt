package dev.orither.engine

import android.annotation.SuppressLint
import android.content.AttributionSource
import android.content.Context
import android.content.ContextWrapper
import android.os.Process

private const val SHELL_PACKAGE = "com.android.shell"

/**
 * The context of the shell user. A process started with app_process has none, so
 * this builds one the way scrcpy does, from a system context with the shell's name.
 */
@SuppressLint("PrivateApi", "DiscouragedPrivateApi")
internal class ShellContext(base: Context) : ContextWrapper(base) {
    override fun getPackageName() = SHELL_PACKAGE

    override fun getOpPackageName() = SHELL_PACKAGE

    override fun getAttributionSource(): AttributionSource =
        AttributionSource.Builder(Process.SHELL_UID).setPackageName(SHELL_PACKAGE).build()

    override fun getApplicationContext(): Context = this

    /** The clipboard identifies its caller through the context it was created with. */
    override fun getSystemService(name: String): Any? {
        val service = super.getSystemService(name) ?: return null
        if (name == CLIPBOARD_SERVICE) {
            service.javaClass.getDeclaredField("mContext").apply { isAccessible = true }.set(service, this)
        }
        return service
    }

    companion object {
        fun create(): ShellContext {
            val type = Class.forName("android.app.ActivityThread")
            val thread = type.getDeclaredConstructor().apply { isAccessible = true }.newInstance()
            type.getDeclaredField("sCurrentActivityThread").apply { isAccessible = true }.set(null, thread)
            type.getDeclaredField("mSystemThread").apply { isAccessible = true }.setBoolean(thread, true)
            val system = type.getDeclaredMethod("getSystemContext").apply { isAccessible = true }.invoke(thread)
            return ShellContext(system as Context)
        }
    }
}
