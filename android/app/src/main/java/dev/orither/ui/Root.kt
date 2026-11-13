package dev.orither.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import dev.orither.link.Link

private enum class Screen { Starting, Linking, Home, Remote }

/** Everything the app shows. A pairing link can arrive at any moment through [deepLink]. */
@Composable
internal fun Root(deepLink: String?, onDeepLinkUsed: () -> Unit) {
    val ready by Link.ready.collectAsState()
    val peers by Link.peers.collectAsState()
    var adding by remember { mutableStateOf(false) }
    // The computer the trackpad drives, by id, so it always shows that computer's live state.
    var remote by remember { mutableStateOf<String?>(null) }
    val remotePeer = peers.firstOrNull { it.id == remote }
    val leaveLinking = {
        adding = false
        onDeepLinkUsed()
    }
    val screen = when {
        !ready -> Screen.Starting
        peers.isEmpty() || adding || deepLink != null -> Screen.Linking
        remotePeer != null -> Screen.Remote
        else -> Screen.Home
    }
    OritherTheme {
        AnimatedContent(
            targetState = screen,
            modifier = Modifier.fillMaxSize().background(Palette.Background),
            transitionSpec = {
                (fadeIn() + scaleIn(Springs.page(), initialScale = 0.96f)) togetherWith
                    (fadeOut() + scaleOut(Springs.page(), targetScale = 1.02f))
            },
            label = "screen",
        ) { shown ->
            when (shown) {
                Screen.Starting -> Box(Modifier.fillMaxSize())
                Screen.Linking -> Linking(deepLink, onLinked = leaveLinking, onCancel = leaveLinking.takeIf { peers.isNotEmpty() })
                Screen.Home -> Home(peers, addComputer = { adding = true }, openRemote = { remote = it.id })
                Screen.Remote -> remotePeer?.let { Remote(it) { remote = null } } ?: Box(Modifier.fillMaxSize())
            }
        }
    }
}
