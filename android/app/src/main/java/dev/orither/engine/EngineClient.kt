package dev.orither.engine

import android.content.Context
import android.os.ParcelFileDescriptor
import java.io.BufferedInputStream
import java.io.File
import java.net.InetAddress
import java.net.Socket
import kotlinx.coroutines.DisposableHandle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.InternalCoroutinesApi
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.job
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/** The app's connection to the engine process. */
class EngineClient(context: Context) {
    private val apk = context.packageCodePath
    private val secret = loadSecret(context.filesDir.resolve("engine.secret"))

    /**
     * The shell command that starts the engine, to run over adb. It first ends any
     * engine left by an older install. The class name is split by a variable so
     * that the shell running this very command does not match the pattern.
     */
    val launchCommand: String
        get() = "E=${Main::class.java.name.removeSuffix(".Main")}; pkill -f \"\$E[.]Main\"; " +
            "CLASSPATH=$apk setsid nohup app_process / \$E.Main ${secret.toHexString()} >/dev/null 2>&1 &"

    /** Whether the engine is running, and speaks this version of the protocol. */
    suspend fun isRunning(): Boolean = withContext(Dispatchers.IO) {
        runCatching { request(JSONObject().put("op", "hello")).getInt("version") == Protocol.VERSION }
            .getOrDefault(false)
    }

    /** Runs commands about this app with the engine's rights. Returns those that failed. */
    suspend fun grant(commands: List<List<String>>): JSONArray = withContext(Dispatchers.IO) {
        val list = JSONArray(commands.map(::JSONArray))
        request(JSONObject().put("op", "grant").put("commands", list)).getJSONArray("failed")
    }

    /**
     * Runs `am start` with [arguments], which works from the background too. With a
     * capture's [session], the activity starts on the display that capture made.
     */
    suspend fun startActivity(arguments: List<String>, session: String? = null): Boolean = withContext(Dispatchers.IO) {
        val request = JSONObject().put("op", "activity").put("arguments", JSONArray(arguments))
        if (session != null) request.put("session", session)
        request(request).getBoolean("started")
    }

    /** This device's Bluetooth address, which an app cannot read for itself. */
    suspend fun bluetoothAddress(): String = withContext(Dispatchers.IO) {
        request(JSONObject().put("op", "bluetooth")).getString("address")
    }

    /** Calls [changed] for each clipboard change, until the connection breaks. */
    suspend fun followClipboard(changed: (JSONObject) -> Unit) = withContext(Dispatchers.IO) {
        open().use { socket ->
            val cancel = launchClosing(socket)
            try {
                val input = BufferedInputStream(socket.getInputStream())
                Protocol.write(socket.getOutputStream(), JSONObject().put("op", "clipboard"))
                checked(Protocol.read(input))
                while (true) changed(Protocol.read(input))
            } finally {
                cancel.dispose()
            }
        }
    }

    /** The phone's text conversations, newest first. The app has no SMS access of its own. */
    suspend fun textThreads(): JSONArray = withContext(Dispatchers.IO) {
        request(JSONObject().put("op", "threads")).getJSONArray("threads")
    }

    /** The latest messages of one conversation, oldest first. */
    suspend fun texts(thread: Int): JSONArray = withContext(Dispatchers.IO) {
        request(JSONObject().put("op", "texts").put("thread", thread)).getJSONArray("texts")
    }

    /** The id of the newest text on the phone. */
    suspend fun newestText(): Long = withContext(Dispatchers.IO) {
        request(JSONObject().put("op", "newest-text")).getLong("id")
    }

    /** The texts received after the one with id [after], oldest first. */
    suspend fun textsSince(after: Long): JSONArray = withContext(Dispatchers.IO) {
        request(JSONObject().put("op", "texts-since").put("after", after)).getJSONArray("texts")
    }

    /** The bytes behind a content URI the app itself may not open. */
    suspend fun read(uri: String): ParcelFileDescriptor = withContext(Dispatchers.IO) {
        val socket = open()
        try {
            Protocol.write(socket.getOutputStream(), JSONObject().put("op", "read").put("uri", uri))
            checked(Protocol.read(socket.getInputStream()))
            ParcelFileDescriptor.fromSocket(socket)
        } finally {
            socket.close()
        }
    }

    /** Closes [socket] when the calling coroutine is cancelled, to end blocking reads. */
    @OptIn(InternalCoroutinesApi::class)
    private suspend fun launchClosing(socket: Socket): DisposableHandle =
        currentCoroutineContext().job.invokeOnCompletion(onCancelling = true) { socket.close() }

    /** Starts a capture with scrcpy [options] and returns its session id. */
    suspend fun capture(options: List<String>, channels: Int): String = withContext(Dispatchers.IO) {
        request(
            JSONObject()
                .put("op", "capture")
                .put("apk", apk)
                .put("options", JSONArray(options))
                .put("channels", channels),
        ).getString("session")
    }

    suspend fun stop(session: String) = withContext(Dispatchers.IO) {
        request(JSONObject().put("op", "stop").put("session", session))
        Unit
    }

    /** Whether the phone takes a computer's touches, as the engine knows it: see its `touch`. */
    suspend fun touch(): JSONObject = withContext(Dispatchers.IO) {
        request(JSONObject().put("op", "touch"))
    }

    /** The app in front on the phone's own screen, or null for the home screen and the like. */
    suspend fun foreground(): RunningApp? = withContext(Dispatchers.IO) {
        val front = request(JSONObject().put("op", "foreground")).optJSONObject("front") ?: return@withContext null
        RunningApp(front.getInt("task"), front.getString("package"))
    }

    /** Moves a running app onto the display of a capture, without starting it again. */
    suspend fun move(session: String, task: Int) = withContext(Dispatchers.IO) {
        request(JSONObject().put("op", "move").put("session", session).put("task", task))
        Unit
    }

    /** Closes the notification shade, which an app may no longer do for itself. */
    suspend fun collapseShade() = withContext(Dispatchers.IO) {
        request(JSONObject().put("op", "collapse"))
        Unit
    }

    /**
     * A socket carrying a call's sound both ways: the other person's voice comes in,
     * and what is written goes into the call. An app's call is named by the app's
     * [uid]; without one, the phone call is meant.
     */
    suspend fun callAudio(uid: Int?): ParcelFileDescriptor = withContext(Dispatchers.IO) {
        val socket = open()
        try {
            val request = JSONObject().put("op", "callaudio")
            if (uid != null) request.put("uid", uid)
            Protocol.write(socket.getOutputStream(), request)
            checked(Protocol.read(socket.getInputStream()))
            ParcelFileDescriptor.fromSocket(socket)
        } finally {
            socket.close()
        }
    }

    /** A socket connected to one channel of a capture. */
    suspend fun attach(session: String, channel: Int): ParcelFileDescriptor = withContext(Dispatchers.IO) {
        val socket = open()
        try {
            val request = JSONObject().put("op", "attach").put("session", session).put("channel", channel)
            Protocol.write(socket.getOutputStream(), request)
            checked(Protocol.read(socket.getInputStream()))
            ParcelFileDescriptor.fromSocket(socket)
        } finally {
            socket.close()
        }
    }

    private fun request(message: JSONObject): JSONObject = open().use { socket ->
        Protocol.write(socket.getOutputStream(), message)
        checked(Protocol.read(socket.getInputStream()))
    }

    private fun open(): Socket {
        val socket = Socket(InetAddress.getLoopbackAddress(), Protocol.PORT)
        socket.tcpNoDelay = true
        if (!Handshake.asApp(socket, secret)) {
            socket.close()
            error("something else is using the engine's port")
        }
        return socket
    }

    private fun checked(reply: JSONObject): JSONObject {
        check(!reply.has("error")) { reply.getString("error") }
        return reply
    }

    private fun loadSecret(file: File): ByteArray {
        if (file.exists()) return file.readBytes()
        return Handshake.nonce().also(file::writeBytes)
    }
}

/** An app as it runs on the phone, in the task that holds its screens. */
data class RunningApp(val task: Int, val packageName: String)
