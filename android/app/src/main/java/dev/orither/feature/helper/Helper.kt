package dev.orither.feature.helper

import android.Manifest
import android.content.Context
import android.database.ContentObserver
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.NetworkCapabilities
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.provider.Settings
import androidx.core.content.edit
import dev.orither.R
import dev.orither.engine.EngineClient
import dev.orither.feature.Access
import dev.orither.feature.Feature
import dev.orither.feature.Features
import dev.orither.feature.body
import dev.orither.feature.flag
import dev.orither.feature.number
import dev.orither.link.Link
import dev.orither.link.LinkStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.Socket
import java.security.MessageDigest
import kotlin.coroutines.resume
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.firstOrNull
import kotlinx.coroutines.flow.mapNotNull
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

private const val WIRELESS = "adb_wifi_enabled"
private const val SERVICE = "_adb-tls-connect._tcp"
private const val BEFORE = "before"
private const val FAMILIAR = "familiar"

/** Android turns wireless debugging straight back off on a network the user has not trusted. */
private const val SETTLE_MS = 1_500L
private const val ANSWER_WITHIN_MS = 60_000L
private const val DISCOVER_WITHIN_MS = 15_000L
private const val PROBE_MS = 1_000

/** A computer needs seconds. This is for one that went quiet before saying it was done. */
private const val OPEN_FOR_MS = 120_000L

/** Android's question can outlast the wait, and an answer then would turn it on for no one. */
private const val GUARD_MS = 300_000L

@Serializable
private data class Status(val running: Boolean, val launch: String, val model: String, val recovers: Boolean)

@Serializable
private data class Opened(val state: String, val port: Int? = null)

/** What a computer's request to turn everything on left off, by name. */
@Serializable
private data class Allowed(val missing: List<String>)

/**
 * Lets a computer start the engine without a cable, which is what a restarted phone
 * needs. Only adb can start the engine, so the computer brings its own: this turns
 * on Android's wireless debugging for a moment and carries the computer's adb to it
 * through the link, on whatever network the two are. Android still checks the
 * computer's key, and asks the user once for each network.
 */
internal class Helper(private val context: Context, private val engine: EngineClient) : Feature {
    override val id = "helper"
    override val access = Access.Shell(
        R.string.access_restarts,
        R.string.access_restarts_about,
        R.drawable.ic_restart,
        Manifest.permission.WRITE_SECURE_SETTINGS,
    )
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val resolver = context.contentResolver
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val memory = context.getSharedPreferences(id, Context.MODE_PRIVATE)
    private val lock = Mutex()
    private var owner: String? = null

    @Volatile private var port: Int? = null
    private var ending: Job? = null
    private var guarding: Job? = null

    /** Wireless debugging as it was before Orither turned it on, on disk in case Orither stops first. */
    private var before: Int?
        get() = memory.getInt(BEFORE, -1).takeIf { it >= 0 }
        set(value) = memory.edit { if (value == null) remove(BEFORE) else putInt(BEFORE, value) }

    init {
        scope.launch { lock.withLock { close() } }
    }

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement = when (method) {
        "status" -> body(Status(engine.isRunning(), engine.launchCommand, Build.MODEL, access.isGranted(context)))
        "open" -> body(lock.withLock { open(body.flag("quiet"), peer) })
        "allow" -> body(allowAll())
        else -> super.call(method, body, peer)
    }

    override suspend fun notice(method: String, body: JsonElement, peer: String) {
        if (method == "close") lock.withLock { close() }
    }

    /** Carries one adb connection, to the port opened for it and nowhere else. */
    override suspend fun stream(method: String, body: JsonElement, stream: LinkStream, peer: String) {
        val wanted = body.number("port")
        if (method != "connect" || wanted != port) return stream.close()
        val socket = withContext(Dispatchers.IO) { runCatching { Socket(InetAddress.getLoopbackAddress(), wanted) } }
            .getOrElse { return stream.close() }
        try {
            stream.attach(ParcelFileDescriptor.fromSocket(socket))
        } finally {
            socket.close()
        }
    }

    override fun disconnected(peer: String) {
        scope.launch { lock.withLock { if (peer == owner) close() } }
    }

    /**
     * Turns on everything the engine can, for a computer setting the phone up over USB, and says
     * what is still off afterwards. Reading each one again is what counts, not the commands' exit
     * codes, since some fail harmlessly on Android versions that lack them.
     */
    private suspend fun allowAll(): Allowed {
        check(engine.isRunning()) { "Orither's helper is not running on the phone" }
        val missing = Features.access.filterNot { it.isGranted(context) }
        if (missing.isNotEmpty()) runCatching { engine.grant(missing.flatMap { it.grants }) }
        Features.allowed()
        return Allowed(Features.access.filterNot { it.isGranted(context) }.map { context.getString(it.title) })
    }

    /** The user is looking at the phone, which is the moment for Android to ask about this network. */
    override fun allowed() {
        if (access.isGranted(context) && wifi() !in familiar()) Link.broadcast(id, "ready")
    }

    private suspend fun open(quiet: Boolean, peer: String): Opened {
        if (!access.isGranted(context)) return Opened("permission")
        val network = wifi() ?: return Opened("wifi")
        if (quiet && network !in familiar()) return Opened("unfamiliar")
        guarding?.cancelAndJoin()
        if (before == null) before = wireless()
        owner = peer
        if (!switchOn(quiet)) {
            // A quiet open that made Android ask was wrong about this network.
            if (quiet) remember(network, false)
            close()
            guarding = scope.launch { guard() }
            return Opened(if (quiet) "unfamiliar" else "denied")
        }
        val found = discover() ?: return Opened("unreachable").also { close() }
        port = found
        remember(network, true)
        ending?.cancel()
        ending = scope.launch {
            delay(OPEN_FOR_MS)
            lock.withLock { close() }
        }
        return Opened("ready", found)
    }

    /**
     * Turns wireless debugging on. On a network the user has not trusted, Android turns
     * it straight back off and asks them, then turns it on itself if they allow.
     */
    private suspend fun switchOn(quiet: Boolean): Boolean {
        Settings.Global.putInt(resolver, WIRELESS, 1)
        delay(SETTLE_MS)
        if (wireless() == 1) return true
        if (quiet) return false
        owner?.let { Link.notify(it, id, "asking") }
        return withTimeoutOrNull(ANSWER_WITHIN_MS) { changes().first { wireless() == 1 } } != null
    }

    /** Puts wireless debugging back the way it was before Orither turned it on. */
    private fun close() {
        ending?.cancel()
        port = null
        owner = null
        val restore = before ?: return
        runCatching { Settings.Global.putInt(resolver, WIRELESS, restore) }
        before = null
    }

    private suspend fun guard() = withTimeoutOrNull(GUARD_MS) {
        changes().collect { if (wireless() == 1) runCatching { Settings.Global.putInt(resolver, WIRELESS, 0) } }
    }

    private fun wireless() = Settings.Global.getInt(resolver, WIRELESS, 0)

    /** Each change to wireless debugging, starting with the current state. */
    private fun changes(): Flow<Unit> = callbackFlow {
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) {
                trySend(Unit)
            }
        }
        resolver.registerContentObserver(Settings.Global.getUriFor(WIRELESS), false, observer)
        trySend(Unit)
        awaitClose { resolver.unregisterContentObserver(observer) }
    }

    /** adbd takes a new port each time, and announces it only to the network. */
    @Suppress("DEPRECATION") // NsdServiceInfo.host: its replacement needs Android 14.
    private suspend fun discover(): Int? = withTimeoutOrNull(DISCOVER_WITHIN_MS) {
        val own = ownAddresses()
        services()
            .mapNotNull { resolve(it) }
            .firstOrNull { it.host?.hostAddress?.substringBefore('%') in own && answers(it.port) }
            ?.port
    }

    private fun services(): Flow<NsdServiceInfo> = callbackFlow {
        val listener = object : NsdManager.DiscoveryListener {
            override fun onServiceFound(service: NsdServiceInfo) {
                trySend(service)
            }

            override fun onStartDiscoveryFailed(type: String, code: Int) {
                close()
            }

            override fun onDiscoveryStarted(type: String) {}

            override fun onDiscoveryStopped(type: String) {}

            override fun onServiceLost(service: NsdServiceInfo) {}

            override fun onStopDiscoveryFailed(type: String, code: Int) {}
        }
        nsd.discoverServices(SERVICE, NsdManager.PROTOCOL_DNS_SD, listener)
        awaitClose { runCatching { nsd.stopServiceDiscovery(listener) } }
    }

    // The replacement, registerServiceInfoCallback, needs Android 14.
    @Suppress("DEPRECATION")
    private suspend fun resolve(service: NsdServiceInfo): NsdServiceInfo? = suspendCancellableCoroutine { waiting ->
        nsd.resolveService(
            service,
            object : NsdManager.ResolveListener {
                override fun onServiceResolved(info: NsdServiceInfo) = waiting.resume(info)

                override fun onResolveFailed(info: NsdServiceInfo, code: Int) = waiting.resume(null)
            },
        )
    }

    /** An announcement can outlive the adbd that made it. */
    private suspend fun answers(port: Int) = withContext(Dispatchers.IO) {
        runCatching { Socket().use { it.connect(InetSocketAddress(InetAddress.getLoopbackAddress(), port), PROBE_MS) } }
            .isSuccess
    }

    private fun ownAddresses(): Set<String> = NetworkInterface.getNetworkInterfaces()?.toList().orEmpty()
        .flatMap { it.inetAddresses.toList() }
        .mapNotNull { it.hostAddress?.substringBefore('%') }
        .toSet()

    private fun familiar(): Set<String> = memory.getStringSet(FAMILIAR, null).orEmpty()

    private fun remember(network: String, known: Boolean) = memory.edit {
        putStringSet(FAMILIAR, if (known) familiar() + network else familiar() - network)
    }

    /**
     * The Wi-Fi network, which wireless debugging needs, told apart from others by what it
     * hands out. Android trusts networks by an address apps may not read, so this stands in.
     */
    @Suppress("DEPRECATION") // allNetworks is the only way to see Wi-Fi that is not the default network.
    private fun wifi(): String? {
        val connectivity = context.getSystemService(ConnectivityManager::class.java)
        val network = connectivity.allNetworks.firstOrNull {
            val capabilities = connectivity.getNetworkCapabilities(it)
            capabilities != null && capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) &&
                !capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
        } ?: return null
        val link = connectivity.getLinkProperties(network) ?: return null
        val traits = link.routes.filter { it.isDefaultRoute }.mapNotNull { it.gateway?.hostAddress } +
            listOfNotNull(link.interfaceName, link.domains, dhcpServer(link))
        val digest = MessageDigest.getInstance("SHA-256").digest(traits.sorted().joinToString("\n").toByteArray())
        return digest.take(8).joinToString("") { "%02x".format(it) }
    }

    private fun dhcpServer(link: LinkProperties): String? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) link.dhcpServerAddress?.hostAddress else null
}
