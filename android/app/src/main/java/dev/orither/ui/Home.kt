package dev.orither.ui

import android.app.StatusBarManager
import android.content.ActivityNotFoundException
import android.content.ComponentName
import android.content.Intent
import android.graphics.drawable.Icon
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.RequiresApi
import androidx.annotation.StringRes
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.net.toUri
import androidx.lifecycle.compose.LifecycleResumeEffect
import dev.orither.R
import dev.orither.feature.Access
import dev.orither.feature.Features
import dev.orither.feature.screen.HandoffTile
import dev.orither.link.Link
import dev.orither.link.Peer
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private const val ENGINE_CHECK_MS = 4_000L
private const val UNLINK_ARMED_MS = 4_000L
private const val XIAOMI = "Xiaomi"

/** The letter a computer goes by on its satellite, as on the desktop. */
private fun initialOf(peer: Peer, all: List<Peer>): String {
    val own = peer.device.name.filter { it.isLetterOrDigit() }.uppercase().ifEmpty { "?" }
    val clash = all.any { it.id != peer.id && it.device.name.firstOrNull { c -> c.isLetterOrDigit() }?.uppercaseChar() == own.first() }
    return if (clash) own.take(2) else own.take(1)
}

/** The phone's own page: its computers on the orrery, the trackpad for the one in front, and what they may reach. */
@Composable
internal fun Home(peers: List<Peer>, addComputer: () -> Unit, openRemote: (Peer) -> Unit) {
    // Grants happen in other screens, so everything is read again on the way back.
    var visits by remember { mutableIntStateOf(0) }
    LifecycleResumeEffect(Unit) {
        visits++
        onPauseOrDispose {}
    }
    val running = rememberEngineRunning(visits)
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    // What a grant left off, so a button that seems to do nothing says why.
    var stillOff by remember { mutableStateOf<List<Access>>(emptyList()) }
    val grant: (List<Access>) -> Unit = { accesses ->
        scope.launch {
            runCatching { Features.engine.grant(accesses.flatMap { it.grants }) }
            visits++
            Features.allowed()
            stillOff = accesses.filterNot { it.isGranted(context) }
        }
    }
    // The computer in front is the one picked, or else the first one the phone reaches now.
    var picked by rememberSaveable { mutableStateOf<String?>(null) }
    val front = peers.firstOrNull { it.id == picked } ?: peers.firstOrNull { it.online } ?: peers.firstOrNull()

    Column(
        Modifier
            .fillMaxSize()
            .background(Palette.Background)
            .statusBarsPadding()
            .navigationBarsPadding()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 16.dp, vertical = 12.dp),
        Arrangement.spacedBy(12.dp),
    ) {
        Header(peers, front)
        front?.let { computer ->
            Action(
                stringResource(R.string.remote_open, computer.device.name),
                Modifier.fillMaxWidth(),
                R.drawable.ic_pointer,
                enabled = computer.online,
            ) { openRemote(computer) }
        }
        Tile(Modifier.fillMaxWidth()) {
            Column(Modifier.padding(vertical = 6.dp)) {
                peers.forEach { peer -> Computer(peer, initialOf(peer, peers), inFront = peer.id == front?.id) { picked = peer.id } }
                Item(R.drawable.ic_plus, stringResource(R.string.add_computer), stringResource(R.string.add_computer_about), onClick = addComputer)
            }
        }
        Spacer(Modifier.height(4.dp))
        Heading(stringResource(R.string.section_access), Modifier.padding(horizontal = 4.dp))
        Tile(Modifier.fillMaxWidth()) {
            Column(Modifier.padding(vertical = 6.dp).animateContentSize(Springs.page())) {
                Engine(visits, running, stillOff, grant)
                AccessList(visits, running, grant)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) Handoff(visits)
                if (Build.MANUFACTURER.equals(XIAOMI, ignoreCase = true)) Autostart()
            }
        }
        Spacer(Modifier.height(16.dp))
    }
}

/** The orrery, with what it says in words beside it. */
@Composable
private fun Header(peers: List<Peer>, front: Peer?) {
    Row(Modifier.fillMaxWidth().padding(top = 8.dp, bottom = 4.dp), Arrangement.spacedBy(16.dp), Alignment.CenterVertically) {
        Orrery(
            108.dp,
            peers.map { Satellite(initialOf(it, peers), orbitOf(it.route, it.online), it.online) },
            front = peers.indexOf(front).coerceAtLeast(0),
        )
        Column(Modifier.weight(1f), Arrangement.spacedBy(4.dp)) {
            Text(stringResource(R.string.app_name), color = Palette.Text, style = MaterialTheme.typography.headlineSmall)
            if (front != null) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    StatusDot(front.online)
                    Text(
                        if (front.online) stringResource(R.string.linked_to, front.device.name) else stringResource(R.string.computers_away),
                        color = Palette.Dim,
                        style = MaterialTheme.typography.bodyMedium,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

@StringRes
internal fun routeTitle(route: String?): Int = when (route) {
    "local" -> R.string.route_local
    "nearby" -> R.string.route_nearby
    "internet" -> R.string.route_internet
    "relay" -> R.string.route_relay
    else -> R.string.computer_waiting
}

/** A computer in the list. A tap brings it to the front; another tap on the one in front offers to unlink it. */
@Composable
private fun Computer(peer: Peer, initial: String, inFront: Boolean, pick: () -> Unit) {
    var armed by remember { mutableStateOf(false) }
    // A second tap on the computer in front offers to unlink it; a tap on another brings that one forward.
    var open by remember { mutableStateOf(false) }
    if (!inFront) open = false
    LaunchedEffect(armed) {
        if (armed) {
            delay(UNLINK_ARMED_MS)
            armed = false
        }
    }
    val filled = inFront && peer.online
    Column(Modifier.animateContentSize(Springs.page())) {
        Row(
            Modifier
                .fillMaxWidth()
                .heightIn(min = 56.dp)
                .clickable(role = Role.Button, onClickLabel = peer.device.name) { if (inFront) open = !open else pick() }
                .padding(horizontal = 14.dp, vertical = 12.dp),
            Arrangement.spacedBy(14.dp),
            Alignment.CenterVertically,
        ) {
            Box(
                Modifier
                    .size(36.dp)
                    .clip(CircleShape)
                    .background(if (filled) Palette.Accent else Palette.Raised),
                Alignment.Center,
            ) {
                Text(initial, color = if (filled) Palette.OnAccent else if (peer.online) Palette.AccentInk else Palette.Dim, style = MaterialTheme.typography.labelLarge)
            }
            Labelled(peer.device.name, stringResource(routeTitle(peer.route.takeIf { peer.online })), Modifier.weight(1f))
        }
        AnimatedVisibility(inFront && open, enter = fadeIn() + expandVertically(Springs.page()), exit = fadeOut() + shrinkVertically(Springs.snappy())) {
            Row(Modifier.padding(start = 64.dp, end = 14.dp, bottom = 12.dp)) {
                Quiet(stringResource(if (armed) R.string.unlink_confirm else R.string.unlink), color = if (armed) Palette.Danger else Palette.Text) {
                    if (armed) Link.unpair(peer.id) else armed = true
                }
            }
        }
    }
}

/** Each thing to allow, with whether it is, so a feature's switch comes with the feature. */
@Composable
private fun AccessList(visits: Int, running: Boolean, grant: (List<Access>) -> Unit) {
    val context = LocalContext.current
    var answers by remember { mutableIntStateOf(0) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        answers++
        Features.allowed()
    }
    Features.access.forEach { access ->
        val granted = remember(access, visits, answers) { access.isGranted(context) }
        val allow: (() -> Unit)? = when (access) {
            is Access.Permissions -> ({ ask.launch(access.names.toTypedArray()) })
            is Access.Special -> ({ context.startActivity(access.settings(context)) })
            is Access.Shell -> ({ grant(listOf(access)) }).takeIf { running }
        }
        AccessRow(access, granted, allow)
    }
}

@Composable
private fun AccessRow(access: Access, granted: Boolean, allow: (() -> Unit)?) {
    val mark by animateColorAsState(if (granted) Palette.AccentInk else Palette.Dim, Springs.snappy(), label = "access")
    Item(access.icon, stringResource(access.title), stringResource(access.about), tint = mark) {
        when {
            granted -> Text(stringResource(R.string.allowed), color = Palette.AccentInk, style = MaterialTheme.typography.labelMedium)
            allow == null -> Text(stringResource(R.string.allow_with_usb), color = Palette.Dim, style = MaterialTheme.typography.labelMedium)
            else -> Quiet(stringResource(R.string.allow), onClick = allow)
        }
    }
}

/** Whether the engine runs. Only a computer can start it, so it is read again now and then. */
@Composable
private fun rememberEngineRunning(visits: Int): Boolean {
    var running by remember { mutableStateOf(false) }
    LaunchedEffect(visits) {
        while (true) {
            running = Features.engine.isRunning()
            delay(ENGINE_CHECK_MS)
        }
    }
    return running
}

/** Whether the part of Orither that only a computer can start is running. */
@Composable
private fun Engine(visits: Int, running: Boolean, stillOff: List<Access>, grant: (List<Access>) -> Unit) {
    val context = LocalContext.current
    val missing = remember(visits) { Features.access.filterNot { it.isGranted(context) } }
    val waiting = if (remember(visits) { Features.recovers(context) }) R.string.engine_waiting else R.string.engine_needed
    Item(
        if (running) R.drawable.ic_screen else R.drawable.ic_usb,
        stringResource(R.string.engine_title),
        stringResource(if (running) R.string.engine_ready else waiting),
        tint = if (running) Palette.AccentInk else Palette.Dim,
    )
    if (running && missing.isNotEmpty()) {
        Quiet(stringResource(R.string.allow_all), Modifier.fillMaxWidth().padding(start = 64.dp, end = 14.dp, bottom = 8.dp)) { grant(missing) }
        val off = stillOff.filter { it in missing }
        if (off.isNotEmpty()) {
            Text(
                stringResource(R.string.allow_all_partly, off.joinToString { context.getString(it.title) }),
                Modifier.padding(start = 64.dp, end = 14.dp, bottom = 10.dp),
                color = Palette.Dim,
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

/** Offers the Quick Settings button that moves an app to the computer, until it is there. */
@RequiresApi(Build.VERSION_CODES.TIRAMISU)
@Composable
private fun Handoff(visits: Int) {
    val context = LocalContext.current
    var added by remember(visits) { mutableStateOf(HandoffTile.isAdded(context)) }
    val label = stringResource(R.string.handoff_tile)
    if (added) return
    run {
        Item(
            R.drawable.ic_monitor_up,
            stringResource(R.string.handoff_title),
            stringResource(R.string.handoff_about),
            tint = Palette.AccentInk,
            onClick = {
                context.getSystemService(StatusBarManager::class.java).requestAddTileService(
                    ComponentName(context, HandoffTile::class.java),
                    label,
                    Icon.createWithResource(context, R.drawable.ic_monitor_up),
                    context.mainExecutor,
                ) { result ->
                    if (result == StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_ADDED ||
                        result == StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_ALREADY_ADDED
                    ) {
                        HandoffTile.remember(context, true)
                        added = true
                    }
                }
            },
        ) { Glyph(R.drawable.ic_plus, tint = Palette.Dim, size = 18.dp) }
    }
}

/** HyperOS and MIUI end background apps unless told otherwise, and only there can it be said. */
@Composable
private fun Autostart() {
    val context = LocalContext.current
    run {
        Item(
            R.drawable.ic_rocket,
            stringResource(R.string.xiaomi_title),
            stringResource(R.string.xiaomi_about),
            tint = Palette.AccentInk,
            onClick = {
                val autostart = Intent().setComponent(
                    ComponentName("com.miui.securitycenter", "com.miui.permcenter.autostart.AutoStartManagementActivity"),
                )
                val details = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, "package:${context.packageName}".toUri())
                try {
                    context.startActivity(autostart)
                } catch (_: ActivityNotFoundException) {
                    context.startActivity(details)
                } catch (_: SecurityException) {
                    context.startActivity(details)
                }
            },
        ) { Glyph(R.drawable.ic_chevron, tint = Palette.Dim, size = 18.dp) }
    }
}
