package dev.orither.link

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.net.wifi.WifiManager
import android.os.IBinder
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import dev.orither.MainActivity
import dev.orither.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/** Keeps the link alive whenever the app is not on screen. */
class LinkService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var multicast: WifiManager.MulticastLock? = null

    private val network = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = Link.networkChanged()
        override fun onLost(network: Network) = Link.networkChanged()
    }

    override fun onCreate() {
        super.onCreate()
        createChannel()
        goForeground(getString(R.string.link_waiting))

        // Local discovery uses multicast, which Android filters out by default.
        multicast = getSystemService(WifiManager::class.java)
            .createMulticastLock("orither")
            .apply { setReferenceCounted(false); acquire() }

        scope.launch(Dispatchers.IO) {
            Link.start(this@LinkService)
            getSystemService(ConnectivityManager::class.java).registerDefaultNetworkCallback(network)
            Link.peers.collect { peers ->
                val online = peers.firstOrNull { it.online }
                goForeground(
                    online?.let { getString(R.string.link_connected, it.device.name) }
                        ?: getString(R.string.link_waiting),
                )
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int) = START_STICKY

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        multicast?.release()
        scope.cancel()
        runCatching { getSystemService(ConnectivityManager::class.java).unregisterNetworkCallback(network) }
        super.onDestroy()
    }

    private fun createChannel() {
        val channel = NotificationChannel(CHANNEL, getString(R.string.link_channel), NotificationManager.IMPORTANCE_MIN)
        channel.setShowBadge(false)
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun goForeground(status: String) {
        ServiceCompat.startForeground(this, NOTIFICATION, notification(status), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
    }

    private fun notification(status: String): Notification {
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
        )
        return Notification.Builder(this, CHANNEL)
            .setContentTitle(status)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentIntent(open)
            .setOngoing(true)
            .build()
    }

    companion object {
        private const val CHANNEL = "link"
        private const val NOTIFICATION = 1

        fun start(context: Context) {
            ContextCompat.startForegroundService(context, Intent(context, LinkService::class.java))
        }
    }
}
