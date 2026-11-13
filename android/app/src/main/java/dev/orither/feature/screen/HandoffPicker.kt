package dev.orither.feature.screen

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.slideInVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.orither.R
import dev.orither.link.Link
import dev.orither.link.LinkService
import dev.orither.link.Peer
import dev.orither.ui.Glyph
import dev.orither.ui.Heading
import dev.orither.ui.Labelled
import dev.orither.ui.Orbit
import dev.orither.ui.OritherTheme
import dev.orither.ui.Palette
import dev.orither.ui.Shapes
import dev.orither.ui.Springs
import dev.orither.ui.Tile
import dev.orither.ui.routeTitle

private const val OFFLINE_ALPHA = 0.5f
private val SHEET_ELEVATION = 16.dp

/**
 * Chooses the computer that takes the app in front, on a long press of
 * [HandoffTile]. It has a task of its own, so that once it closes the app under
 * it is the one in front again, and not Orither.
 */
class HandoffPicker : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        LinkService.start(this)
        setContent {
            val peers by Link.peers.collectAsState()
            val ready by Link.ready.collectAsState()
            val target = remember(peers) { HandoffTile.target(this)?.id }
            OritherTheme { Picker(peers.takeIf { ready }, target, close = ::finishAndRemoveTask, pick = ::send) }
        }
    }

    private fun send(peer: Peer) {
        // Gone first, since the computer asks right away which app is in front.
        finishAndRemoveTask()
        HandoffTile.send(this, peer)
    }
}

@Composable
private fun Picker(peers: List<Peer>?, target: String?, close: () -> Unit, pick: (Peer) -> Unit) {
    val shown = remember { MutableTransitionState(false) }.apply { targetState = true }
    Box(Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures { close() } }, Alignment.BottomCenter) {
        AnimatedVisibility(shown, enter = fadeIn() + slideInVertically(Springs.page()) { it / 2 }) {
            Column(
                Modifier
                    .navigationBarsPadding()
                    .padding(12.dp)
                    .fillMaxWidth()
                    // Lifted off the app instead of dimming it, which stays in view.
                    .shadow(SHEET_ELEVATION, Shapes.Tile)
                    .background(Palette.Background)
                    .border(1.dp, Palette.Border, Shapes.Tile)
                    // Taps between the rows stay in here, instead of closing it.
                    .pointerInput(Unit) { detectTapGestures {} }
                    .padding(12.dp),
                Arrangement.spacedBy(8.dp),
            ) {
                Heading(stringResource(R.string.handoff_pick), Modifier.padding(start = 8.dp, top = 4.dp, bottom = 4.dp))
                if (peers?.isEmpty() == true) {
                    Text(stringResource(R.string.handoff_none), Modifier.padding(8.dp), Palette.Dim, fontSize = 14.sp)
                }
                peers?.forEach { peer -> Choice(peer, peer.id == target) { pick(peer) } }
            }
        }
    }
}

/** A computer to send to, which only takes a tap while it is connected. */
@Composable
private fun Choice(peer: Peer, target: Boolean, pick: () -> Unit) {
    Tile(Modifier.fillMaxWidth().alpha(if (peer.online) 1f else OFFLINE_ALPHA), onClick = pick.takeIf { peer.online }) {
        Row(Modifier.padding(14.dp), Arrangement.spacedBy(14.dp), Alignment.CenterVertically) {
            Orbit(40.dp, alive = peer.online, color = if (peer.online) Palette.Accent else Palette.Dim)
            Labelled(peer.device.name, stringResource(routeTitle(peer.route.takeIf { peer.online })), Modifier.weight(1f))
            if (target) Glyph(R.drawable.ic_check, tint = Palette.Accent, size = 18.dp)
        }
    }
}
