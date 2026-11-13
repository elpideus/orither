package dev.orither.feature.nearby

import android.Manifest
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothSocket
import android.content.Context
import android.os.ParcelFileDescriptor
import dev.orither.R
import dev.orither.engine.EngineClient
import dev.orither.feature.Access
import dev.orither.feature.Feature
import dev.orither.feature.body
import dev.orither.link.Link
import dev.orither.link.Peer
import java.io.InputStream
import java.io.OutputStream
import java.util.UUID
import kotlin.concurrent.thread
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

private val SERVICE = UUID.fromString("6f726974-6865-7200-8000-00805f9b34fb")
private const val SERVICE_NAME = "Orither"
private const val ADDRESS = "address"
private const val RECHECK_MS = 60_000L
private const val PACKET_BUFFER = 4096

/**
 * Answers Bluetooth calls from computers, for when no local network connects them.
 * A computer can call without any pairing at the Bluetooth level, and needs none:
 * the link on top is already encrypted and only ever talks to paired devices.
 */
internal class Nearby(private val context: Context, private val engine: EngineClient) : Feature {
    override val id = "nearby"
    override val access = Access.Permissions(
        R.string.access_nearby,
        R.string.access_nearby_about,
        R.drawable.ic_bluetooth,
        listOf(Manifest.permission.BLUETOOTH_CONNECT),
    )
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val memory = context.getSharedPreferences(id, Context.MODE_PRIVATE)
    private val address = MutableStateFlow(memory.getString(ADDRESS, null))

    init {
        scope.launch {
            while (true) {
                if (access.isGranted(context)) runCatching { answer() }
                delay(RECHECK_MS)
            }
        }
        scope.launch { learnAddress() }
    }

    override fun connected(peer: Peer) {
        scope.launch {
            val own = address.filterNotNull().first()
            runCatching { Link.notify(peer.id, id, ADDRESS, body(mapOf(ADDRESS to own))) }
        }
    }

    /** Apps may not see their device's address, but the engine can, and it never changes. */
    private suspend fun learnAddress() {
        while (address.value == null) {
            runCatching { engine.bluetoothAddress() }.onSuccess { found ->
                memory.edit().putString(ADDRESS, found).apply()
                address.value = found
            }
            delay(RECHECK_MS)
        }
    }

    /** Takes calls until Bluetooth goes away. */
    private fun answer() {
        val adapter = context.getSystemService(BluetoothManager::class.java).adapter
        adapter.listenUsingInsecureRfcommWithServiceRecord(SERVICE_NAME, SERVICE).use { server ->
            while (true) {
                val socket = server.accept()
                scope.launch { socket.use { runCatching { carry(it) } } }
            }
        }
    }

    /** Joins the Bluetooth socket to the link, which needs one that it can poll. */
    private suspend fun carry(bluetooth: BluetoothSocket) {
        val (local, link) = ParcelFileDescriptor.createSocketPair()
        val input = ParcelFileDescriptor.AutoCloseInputStream(local.dup())
        val output = ParcelFileDescriptor.AutoCloseOutputStream(local)
        val out = thread(isDaemon = true) { pump(bluetooth.inputStream, output) }
        val back = thread(isDaemon = true) { pump(input, bluetooth.outputStream) }
        runCatching { Link.addNearby(link) }
        back.join()
        out.join()
    }

    private fun pump(from: InputStream, to: OutputStream) {
        val buffer = ByteArray(PACKET_BUFFER)
        runCatching {
            while (true) {
                val count = from.read(buffer)
                if (count < 0) break
                to.write(buffer, 0, count)
                to.flush()
            }
        }
        runCatching { from.close() }
        runCatching { to.close() }
    }
}
