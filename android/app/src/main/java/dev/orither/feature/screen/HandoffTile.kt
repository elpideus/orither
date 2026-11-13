package dev.orither.feature.screen

import android.content.Context
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.widget.Toast
import androidx.core.content.edit
import dev.orither.R
import dev.orither.link.Link
import dev.orither.link.Peer
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

private const val MEMORY = "handoff"
private const val ADDED = "added"
private const val LAST = "last"

/**
 * A Quick Settings button that sends the app in front to the computer. The app
 * keeps running as it is, and moves to a window there, see the "handoff" notice.
 * A tap sends it to the computer used last, and a long press opens [HandoffPicker].
 */
class HandoffTile : TileService() {
    private val scope = MainScope()
    private var listening: Job? = null

    override fun onTileAdded() = remember(this, true)

    override fun onTileRemoved() = remember(this, false)

    override fun onStartListening() {
        listening = scope.launch { Link.peers.collect { show(target(this@HandoffTile)) } }
    }

    override fun onStopListening() {
        listening?.cancel()
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    override fun onClick() {
        if (isLocked) unlockAndRun { send(this, target(this)) } else send(this, target(this))
    }

    private fun show(peer: Peer?) {
        val tile = qsTile ?: return
        tile.state = if (peer == null) Tile.STATE_UNAVAILABLE else Tile.STATE_INACTIVE
        tile.label = getString(R.string.handoff_tile)
        tile.subtitle = peer?.device?.name ?: getString(R.string.handoff_offline_short)
        tile.updateTile()
    }

    companion object {
        /** Whether the user has this button in Quick Settings, as far as the phone told us. */
        fun isAdded(context: Context): Boolean = memory(context).getBoolean(ADDED, false)

        fun remember(context: Context, added: Boolean) = memory(context).edit { putBoolean(ADDED, added) }

        /** The computer a tap sends to: the one used last if it is connected, or else the first that is. */
        fun target(context: Context): Peer? {
            val online = Link.peers.value.filter { it.online }
            val last = memory(context).getString(LAST, null)
            return online.firstOrNull { it.id == last } ?: online.firstOrNull()
        }

        /** Remembers [peer] as the computer in use, which is where the next tap sends to. */
        fun used(context: Context, peer: String) = memory(context).edit { putString(LAST, peer) }

        /** The computer does the rest, since it may first have to start the engine. */
        fun send(context: Context, peer: Peer?) {
            if (peer != null && runCatching { Link.notify(peer.id, "screen", "handoff") }.isSuccess) {
                used(context, peer.id)
            } else {
                Toast.makeText(context.applicationContext, R.string.handoff_offline, Toast.LENGTH_SHORT).show()
            }
        }

        private fun memory(context: Context) = context.getSharedPreferences(MEMORY, Context.MODE_PRIVATE)
    }
}
