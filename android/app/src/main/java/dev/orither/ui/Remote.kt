package dev.orither.ui

import android.view.HapticFeedbackConstants
import android.view.WindowManager
import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.annotation.DrawableRes
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.BorderStroke
import androidx.compose.material3.IconButton
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import dev.orither.R
import dev.orither.feature.input.KeyCatcher
import dev.orither.feature.input.RemoteInput
import dev.orither.feature.input.Trackpad
import dev.orither.feature.input.trackpad
import dev.orither.link.Peer
import kotlinx.coroutines.launch

private val ARROWS = listOf(
    Triple("ArrowLeft", R.drawable.ic_arrow_left, R.string.remote_key_left),
    Triple("ArrowUp", R.drawable.ic_arrow_up, R.string.remote_key_up),
    Triple("ArrowDown", R.drawable.ic_arrow_down, R.string.remote_key_down),
    Triple("ArrowRight", R.drawable.ic_arrow_right, R.string.remote_key_right),
)
private val STICKY = listOf(
    RemoteInput.CONTROL to R.string.remote_key_control,
    RemoteInput.ALT to R.string.remote_key_alt,
    RemoteInput.META to R.string.remote_key_super,
)

/**
 * The phone as [peer]'s trackpad and keyboard. The keyboard comes up at once and
 * stays up whatever is touched, with the trackpad filling the space above it.
 */
@Composable
internal fun Remote(peer: Peer, onClose: () -> Unit) {
    val view = LocalView.current
    val input = remember(peer.id) {
        RemoteInput(peer.id) { view.performHapticFeedback(HapticFeedbackConstants.CONTEXT_CLICK) }
    }
    val trackpad = remember(input) { Trackpad(input) }
    val catcher = remember(input) { KeyCatcher(view.context, input) }
    val scope = rememberCoroutineScope()
    ResizeForKeyboard()
    BackHandler(onBack = onClose)
    LaunchedEffect(input) { input.stayOpen() }
    DisposableEffect(catcher) { onDispose { catcher.dismiss() } }
    // Touching brings back a keyboard the user put away, and asks again after a no.
    val touched = {
        catcher.summon()
        if (input.problem != null) scope.launch { input.open(ask = true) }
    }
    val note = when {
        !peer.online -> stringResource(R.string.remote_offline)
        input.problem != null -> input.problem.orEmpty()
        input.ready -> stringResource(R.string.remote_hint)
        else -> stringResource(R.string.remote_waiting)
    }

    Column(
        Modifier
            .fillMaxSize()
            .background(Palette.Background)
            .safeDrawingPadding()
            .padding(horizontal = 12.dp, vertical = 8.dp),
        Arrangement.spacedBy(8.dp),
    ) {
        Bar(peer, ready = peer.online && input.ready, onClose)
        Tile(Modifier.weight(1f).fillMaxWidth().trackpad(trackpad, touched), shape = Shapes.Tile) {
            AndroidView({ catcher }, Modifier.size(1.dp))
            Box(Modifier.fillMaxSize().padding(24.dp), Alignment.Center) {
                val quiet = input.problem == null && peer.online
                Text(note, color = if (quiet) Palette.Dim else Palette.Text, style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center)
            }
        }
        Keys(input)
    }
}

@Composable
private fun Bar(peer: Peer, ready: Boolean, onClose: () -> Unit) {
    Row(Modifier.fillMaxWidth(), Arrangement.spacedBy(8.dp), Alignment.CenterVertically) {
        // Never takes focus, which would take the soft keyboard with it.
        IconButton(onClose, Modifier.focusProperties { canFocus = false }) {
            Glyph(R.drawable.ic_close, size = 22.dp)
        }
        Column(Modifier.weight(1f)) {
            Text(peer.device.name, color = Palette.Text, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                StatusDot(ready)
                Text(
                    stringResource(if (ready) R.string.remote_ready else R.string.remote_title),
                    color = Palette.Dim,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
    }
}

/** The keys a phone keyboard lacks. Ctrl, Alt and Super hold for the next key. */
@Composable
private fun Keys(input: RemoteInput) {
    // Two rows, so every key keeps a full-size target at any phone width and font size.
    Column(Modifier.fillMaxWidth(), Arrangement.spacedBy(8.dp)) {
        Row(Modifier.fillMaxWidth().height(48.dp), Arrangement.spacedBy(8.dp)) {
            Key(stringResource(R.string.remote_key_escape)) { input.press("Escape") }
            Key(stringResource(R.string.remote_key_tab)) { input.press("Tab") }
            STICKY.forEach { (name, label) -> Key(stringResource(label), armed = name in input.armed) { input.toggle(name) } }
        }
        Row(Modifier.fillMaxWidth().height(48.dp), Arrangement.spacedBy(8.dp)) {
            ARROWS.forEach { (name, icon, label) -> Key(stringResource(label), icon = icon) { input.press(name) } }
        }
    }
}

/** A key that never takes focus, which would take the soft keyboard with it. */
@Composable
private fun RowScope.Key(label: String, armed: Boolean? = null, @DrawableRes icon: Int? = null, onClick: () -> Unit) {
    val on = armed == true
    Surface(
        onClick = onClick,
        modifier = Modifier
            .weight(1f)
            .fillMaxHeight()
            .focusProperties { canFocus = false }
            .semantics {
                contentDescription = label
                // Ctrl, Alt and Super stay pressed for the next key, so they read as switches.
                role = if (armed != null) Role.Switch else Role.Button
                if (armed != null) stateDescription = if (on) "on" else "off"
            },
        shape = Shapes.Small,
        color = if (on) Palette.Accent else Palette.Surface,
        contentColor = if (on) Palette.OnAccent else Palette.Text,
        border = BorderStroke(1.dp, Palette.Border),
    ) {
        Box(contentAlignment = Alignment.Center) {
            if (icon != null) {
                Glyph(icon, size = 20.dp, tint = LocalContentColor.current)
            } else {
                Text(label, style = MaterialTheme.typography.labelMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}

/** Has the window make room for the keyboard while this shows, instead of sliding under it. */
@Composable
private fun ResizeForKeyboard() {
    val window = LocalActivity.current?.window ?: return
    DisposableEffect(window) {
        val before = window.attributes.softInputMode
        @Suppress("DEPRECATION")
        val resize = WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE
        window.setSoftInputMode((before and WindowManager.LayoutParams.SOFT_INPUT_MASK_ADJUST.inv()) or resize)
        onDispose { window.setSoftInputMode(before) }
    }
}
