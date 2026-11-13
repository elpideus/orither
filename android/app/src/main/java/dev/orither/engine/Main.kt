package dev.orither.engine

import android.net.LocalSocket
import android.net.LocalSocketAddress
import android.os.Looper
import java.io.File
import java.io.FileOutputStream
import java.io.InputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.zip.ZipFile
import kotlin.concurrent.thread
import kotlin.random.Random
import org.json.JSONArray
import org.json.JSONObject

/**
 * The engine: a small process that runs as Android's `shell` user, which may do
 * what an app may not, such as capturing the screen and injecting input. It is
 * started once over adb and outlives the adb connection. It has no framework
 * context and does exactly what its operations below spell out, for callers that
 * passed the [Handshake].
 */
object Main {
    @JvmStatic
    fun main(args: Array<String>) {
        // Framework callbacks, such as clipboard changes, arrive on the main looper.
        Looper.prepareMainLooper()
        val engine = Engine(args.single().hexToByteArray())
        thread {
            runCatching { engine.serve() }
            Runtime.getRuntime().exit(1)
        }
        Looper.loop()
    }
}

private const val SCRCPY_VERSION = "5.0.1"
private const val SCRCPY_MAIN = "com.genymobile.scrcpy.Server"
private const val PACKAGE = "dev.orither"
private const val BUFFER_SIZE = 64 * 1024
private const val CONNECT_ATTEMPTS = 200
private const val CONNECT_DELAY_MS = 25L
/** How long scrcpy may take to make a display once its channels are connected. */
private const val DISPLAY_WAIT_S = 10L
private val NEW_DISPLAY = Regex("""New display: .* \(id=(\d+)\)""")

private val WORK_DIR = File("/data/local/tmp/orither")
private val LOG = File(WORK_DIR, "capture.log")

/**
 * A running scrcpy server and the sockets it accepted, waiting to be attached, and
 * the display it made for the capture, if it was asked to make one.
 */
private class Capture(
    val process: Process,
    val channels: MutableMap<Int, LocalSocket>,
    val display: CompletableFuture<Int>,
)

private class Engine(private val secret: ByteArray) {
    private val captures = ConcurrentHashMap<String, Capture>()

    // Built here because the framework objects behind these need the main looper.
    private val shell = runCatching(ShellContext::create)
    private val clipboard = shell.map(::ClipboardWatch)
    private val tasks = shell.map(::Tasks)
    private val calls = shell.map(::CallAudio)

    fun serve() {
        WORK_DIR.mkdirs()
        LOG.delete()
        ServerSocket(Protocol.PORT, 16, InetAddress.getLoopbackAddress()).use { server ->
            while (true) {
                val client = server.accept()
                thread(isDaemon = true) { runCatching { client.use(::handle) } }
            }
        }
    }

    private fun handle(client: Socket) {
        client.tcpNoDelay = true
        if (!Handshake.asEngine(client, secret)) return
        val request = Protocol.read(client.getInputStream())
        val reply = JSONObject()
        val output = client.getOutputStream()
        runCatching {
            when (val op = request.getString("op")) {
                "hello" -> reply.put("version", Protocol.VERSION)
                "exit" -> {
                    Protocol.write(output, reply)
                    Runtime.getRuntime().exit(0)
                }
                "grant" -> reply.put("failed", grant(request.getJSONArray("commands")))
                "activity" -> {
                    // A capture's session puts the activity on the display the capture made.
                    val on = request.optString("session").takeIf(String::isNotEmpty)
                        ?.let { listOf("--display", "${display(it)}") }.orEmpty()
                    reply.put("started", run(listOf("am", "start") + on + words(request.getJSONArray("arguments"))) == 0)
                }
                "bluetooth" -> reply.put("address", bluetoothAddress())
                "clipboard" -> {
                    val watch = clipboard.getOrThrow()
                    Protocol.write(output, reply)
                    watch.follow(client)
                    return
                }
                "read" -> {
                    val content = ProcessBuilder("content", "read", "--uri", request.getString("uri"))
                        .redirectError(File("/dev/null"))
                        .start()
                    Protocol.write(output, reply)
                    content.inputStream.use { copy(it, output) }
                    return
                }
                "capture" -> reply.put(
                    "session",
                    capture(request.getString("apk"), request.getJSONArray("options"), request.getInt("channels")),
                )
                "stop" -> captures[request.getString("session")]?.process?.destroy()
                "foreground" -> tasks.getOrThrow().front()?.let { reply.put("front", it) }
                "move" -> move(request.getString("session"), request.getInt("task"))
                "collapse" -> run(listOf("cmd", "statusbar", "collapse"))
                "callaudio" -> {
                    val uid = request.optInt("uid", -1)
                    val prepared = calls.getOrThrow().let { if (uid >= 0) it.app(uid) else it.cellular() }
                    Protocol.write(output, reply)
                    prepared.carry(client)
                    return
                }
                "attach" -> {
                    val socket = take(request.getString("session"), request.getInt("channel"))
                    Protocol.write(output, reply)
                    relay(client, socket)
                    return
                }
                else -> error("unknown operation $op")
            }
        }.onFailure { reply.put("error", it.message ?: it.toString()) }
        Protocol.write(output, reply)
    }

    /** Runs permission commands for this app and returns the ones that failed. */
    private fun grant(commands: JSONArray): JSONArray {
        val failed = JSONArray()
        for (index in 0 until commands.length()) {
            val command = words(commands.getJSONArray(index))
            check(command.any { PACKAGE in it }) { "not about this app" }
            if (run(command) != 0) failed.put(command.joinToString(" "))
        }
        return failed
    }

    private fun words(list: JSONArray): List<String> = List(list.length(), list::getString)

    /** Runs a command to its end and returns its exit status. */
    private fun run(command: List<String>): Int = output(command).first

    /** Runs a command to its end and returns its exit status and what it said. */
    private fun output(command: List<String>): Pair<Int, String> =
        ProcessBuilder(command).redirectErrorStream(true).start().run {
            val said = inputStream.bufferedReader().readText()
            waitFor() to said
        }

    private fun bluetoothAddress(): String {
        val settings = ProcessBuilder("settings", "get", "secure", "bluetooth_address")
            .redirectError(File("/dev/null"))
            .start()
        return settings.inputStream.bufferedReader().use { it.readText().trim() }
    }

    /** Starts scrcpy and connects to its [count] channels in the order it accepts them. */
    private fun capture(apk: String, options: JSONArray, count: Int): String {
        val id = "%08x".format(Random.nextInt(Int.MAX_VALUE))
        val jar = File(WORK_DIR, "$id.jar")
        ZipFile(apk).use { zip ->
            val entry = zip.getEntry("assets/${Protocol.SCRCPY_ASSET}")
            zip.getInputStream(entry).use { input -> jar.outputStream().use(input::copyTo) }
        }
        val command = listOf("app_process", "/", SCRCPY_MAIN, SCRCPY_VERSION, "scid=$id", "tunnel_forward=true", "send_dummy_byte=false") +
            List(options.length(), options::getString)
        val process = ProcessBuilder(command)
            .apply { environment()["CLASSPATH"] = jar.path }
            .redirectErrorStream(true)
            .start()
        val display = CompletableFuture<Int>()
        thread(isDaemon = true) {
            runCatching { follow(process.inputStream, display) }
            process.waitFor()
            display.completeExceptionally(IllegalStateException("the capture ended without a display"))
            captures.remove(id)?.channels?.values?.forEach { runCatching { it.close() } }
            jar.delete()
        }
        val channels = ConcurrentHashMap<Int, LocalSocket>()
        repeat(count) { index -> channels[index] = connect("scrcpy_$id") }
        captures[id] = Capture(process, channels, display)
        return id
    }

    /** Copies what scrcpy says to the log, picking out the id of the display it makes. */
    private fun follow(output: InputStream, display: CompletableFuture<Int>) {
        FileOutputStream(LOG, true).bufferedWriter().use { log ->
            output.bufferedReader().forEachLine { line ->
                NEW_DISPLAY.find(line)?.let { display.complete(it.groupValues[1].toInt()) }
                log.appendLine(line)
                log.flush()
            }
        }
    }

    /**
     * Puts a task that is already running onto the display of a capture, as it is.
     * The app keeps its process and its place, and only adapts to the new screen.
     */
    private fun move(session: String, task: Int) {
        val (status, said) = output(listOf("am", "display", "move-stack", "$task", "${display(session)}"))
        check(status == 0) {
            val lines = said.lineSequence().map(String::trim).filter(String::isNotEmpty)
            lines.firstOrNull { "Exception:" in it } ?: lines.firstOrNull() ?: "the app did not move"
        }
    }

    /** The display a capture made, once scrcpy has made it. */
    private fun display(session: String): Int {
        val capture = captures[session] ?: error("no capture $session")
        return runCatching { capture.display.get(DISPLAY_WAIT_S, TimeUnit.SECONDS) }
            .getOrElse { error("the capture made no display for the app") }
    }

    private fun connect(name: String): LocalSocket {
        repeat(CONNECT_ATTEMPTS) {
            val socket = LocalSocket()
            try {
                socket.connect(LocalSocketAddress(name, LocalSocketAddress.Namespace.ABSTRACT))
                return socket
            } catch (_: java.io.IOException) {
                socket.close()
                Thread.sleep(CONNECT_DELAY_MS)
            }
        }
        error("the capture did not start")
    }

    private fun take(session: String, channel: Int): LocalSocket =
        captures[session]?.channels?.remove(channel) ?: error("no channel $channel in $session")

    /** Carries bytes between the app and scrcpy until either side hangs up. */
    private fun relay(client: Socket, socket: LocalSocket) {
        socket.use {
            val incoming = thread(isDaemon = true) {
                runCatching { copy(client.getInputStream(), socket.outputStream) }
                runCatching { socket.shutdownOutput() }
            }
            runCatching { copy(socket.inputStream, client.getOutputStream()) }
            runCatching { client.shutdownOutput() }
            incoming.join()
        }
    }

    private fun copy(input: java.io.InputStream, output: java.io.OutputStream) {
        val buffer = ByteArray(BUFFER_SIZE)
        while (true) {
            val count = input.read(buffer)
            if (count < 0) return
            output.write(buffer, 0, count)
            output.flush()
        }
    }
}
