package dev.orither.ui

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.keyframes
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import dev.orither.R
import dev.orither.link.Link
import kotlinx.coroutines.Job
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeout

private const val CODE_LENGTH = 8
private const val CODE_GROUP = 4
private const val SHAKE_PIXELS = 18f
private const val SHAKE_MILLIS = 360
/** A code that leads nowhere, or a computer that went away, must not leave the phone waiting for ever. */
private const val JOIN_WITHIN_MS = 45_000L

private sealed interface Stage {
    data object Choosing : Stage
    data object Scanning : Stage
    data object Joining : Stage
}

/** Linking a computer: a scan, or eight characters typed in. */
@Composable
internal fun Linking(deepLink: String?, onLinked: () -> Unit, onCancel: (() -> Unit)?) {
    val scope = rememberCoroutineScope()
    var stage by remember { mutableStateOf<Stage>(Stage.Choosing) }
    var failed by remember { mutableStateOf(false) }
    val shake = remember { Animatable(0f) }
    var joining by remember { mutableStateOf<Job?>(null) }

    val join: (String) -> Unit = { token ->
        if (stage != Stage.Joining) {
            stage = Stage.Joining
            failed = false
            joining = scope.launch {
                val linked = runCatching { withTimeout(JOIN_WITHIN_MS) { Link.join(token) } }
                // Cancelled from the Cancel button: back to choosing, without calling it a failure.
                if (!isActive) return@launch
                if (linked.isFailure) {
                    stage = Stage.Choosing
                    failed = true
                    shake.animateTo(
                        0f,
                        keyframes {
                            durationMillis = SHAKE_MILLIS
                            SHAKE_PIXELS at 60
                            -SHAKE_PIXELS at 140
                            SHAKE_PIXELS / 2 at 220
                            -SHAKE_PIXELS / 2 at 290
                        },
                    )
                } else {
                    onLinked()
                }
            }
        }
    }
    LaunchedEffect(deepLink) { deepLink?.let(join) }

    Column(
        Modifier
            .fillMaxSize()
            .background(Palette.Background)
            .statusBarsPadding()
            .navigationBarsPadding()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 20.dp, vertical = 12.dp),
        Arrangement.spacedBy(12.dp),
    ) {
        Spacer(Modifier.height(36.dp))
        Orbit(72.dp, alive = stage == Stage.Joining)
        Text(
            stringResource(R.string.welcome_title),
            color = Palette.Text,
            fontSize = 34.sp,
            lineHeight = 38.sp,
            letterSpacing = (-0.8).sp,
            fontWeight = FontWeight.SemiBold,
        )
        Text(stringResource(R.string.welcome_hint), color = Palette.Dim, fontSize = 15.sp, lineHeight = 21.sp)
        Spacer(Modifier.height(16.dp))

        AnimatedContent(
            targetState = stage,
            transitionSpec = { (fadeIn() + scaleIn(Springs.pop(), initialScale = 0.94f)) togetherWith fadeOut() },
            contentKey = { it },
            label = "stage",
        ) { current ->
            when (current) {
                Stage.Joining -> Joining {
                    joining?.cancel()
                    stage = Stage.Choosing
                }
                Stage.Scanning -> ScanTile(join) { stage = Stage.Choosing }
                Stage.Choosing -> Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    ScanPrompt { stage = Stage.Scanning }
                    CodeTile(failed, Modifier.graphicsLayer { translationX = shake.value }, join)
                }
            }
        }
        onCancel?.let {
            Quiet(stringResource(R.string.cancel), Modifier.align(Alignment.CenterHorizontally), onClick = it)
        }
    }
}

@Composable
private fun Joining(stop: () -> Unit) {
    Tile(Modifier.fillMaxWidth()) {
        Row(Modifier.padding(20.dp), Arrangement.spacedBy(14.dp), Alignment.CenterVertically) {
            Orbit(28.dp, alive = true)
            Text(stringResource(R.string.linking), color = Palette.Text, fontSize = 16.sp, modifier = Modifier.weight(1f))
            Quiet(stringResource(R.string.cancel), onClick = stop)
        }
    }
}

@Composable
private fun ScanPrompt(start: () -> Unit) {
    val context = LocalContext.current
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { if (it) start() }
    Tile(Modifier.fillMaxWidth(), onClick = {
        val allowed = ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
        if (allowed) start() else ask.launch(Manifest.permission.CAMERA)
    }) {
        Row(Modifier.padding(20.dp), Arrangement.spacedBy(14.dp), Alignment.CenterVertically) {
            Box(Modifier.size(44.dp).background(Palette.AccentSoft, Shapes.Small), Alignment.Center) {
                Glyph(R.drawable.ic_qr, tint = Palette.Accent)
            }
            Labelled(stringResource(R.string.scan_code), stringResource(R.string.camera_needed), Modifier.weight(1f))
            Glyph(R.drawable.ic_chevron, tint = Palette.Dim, size = 18.dp)
        }
    }
}

@Composable
private fun ScanTile(found: (String) -> Unit, close: () -> Unit) {
    Tile(Modifier.fillMaxWidth().animateContentSize(Springs.page())) {
        Box(Modifier.fillMaxWidth().aspectRatio(1f)) {
            Scanner(found, Modifier.fillMaxSize())
            Text(
                stringResource(R.string.scan_hint),
                Modifier
                    .align(Alignment.BottomCenter)
                    .padding(16.dp)
                    .clip(Shapes.Pill)
                    .background(Color.Black.copy(alpha = 0.55f))
                    .padding(horizontal = 14.dp, vertical = 8.dp),
                color = Color.White,
                fontSize = 13.sp,
            )
            Box(
                Modifier
                    .align(Alignment.TopEnd)
                    .padding(12.dp)
                    .size(36.dp)
                    .pressable()
                    .clip(Shapes.Pill)
                    .background(Color.Black.copy(alpha = 0.55f))
                    .clickable(remember { MutableInteractionSource() }, null, onClick = close),
                Alignment.Center,
            ) { Glyph(R.drawable.ic_close, tint = Color.White, size = 18.dp) }
        }
    }
}

/** Eight cells that fill as the user types, joining by themselves when complete. */
@Composable
private fun CodeTile(failed: Boolean, modifier: Modifier, join: (String) -> Unit) {
    var code by remember { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    Tile(modifier.fillMaxWidth().animateContentSize(Springs.snappy())) {
        Column(Modifier.padding(20.dp), Arrangement.spacedBy(14.dp)) {
            Heading(stringResource(R.string.type_code))
            BasicTextField(
                value = code,
                onValueChange = { typed ->
                    code = typed.filter(Char::isLetterOrDigit).uppercase().take(CODE_LENGTH)
                    if (code.length == CODE_LENGTH) join("${code.take(CODE_GROUP)}-${code.drop(CODE_GROUP)}")
                },
                modifier = Modifier.fillMaxWidth().focusRequester(focus),
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.Characters,
                    autoCorrectEnabled = false,
                    keyboardType = KeyboardType.Ascii,
                ),
                singleLine = true,
                decorationBox = { Cells(code) { focus.requestFocus() } },
            )
            if (failed) {
                Text(stringResource(R.string.link_failed), color = Palette.Dim, fontSize = 13.sp, lineHeight = 18.sp)
            }
        }
    }
}

@Composable
private fun Cells(code: String, tap: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(remember { MutableInteractionSource() }, null, onClick = tap),
        Arrangement.spacedBy(5.dp),
        Alignment.CenterVertically,
    ) {
        repeat(CODE_LENGTH) { index ->
            if (index == CODE_GROUP) Spacer(Modifier.width(6.dp))
            Cell(code.getOrNull(index), active = index == code.length, Modifier.weight(1f))
        }
    }
}

@Composable
private fun Cell(character: Char?, active: Boolean, modifier: Modifier) {
    val edge by animateColorAsState(if (active) Palette.Accent else Palette.Border, Springs.snappy(), label = "cell")
    Box(
        modifier
            .aspectRatio(0.78f)
            .clip(Shapes.Small)
            .background(Palette.Raised)
            .border(1.5.dp, edge, Shapes.Small),
        Alignment.Center,
    ) {
        AnimatedContent(
            character,
            transitionSpec = { scaleIn(Springs.pop(), initialScale = 0.5f) togetherWith fadeOut() },
            label = "character",
        ) {
            Text(
                it?.toString().orEmpty(),
                color = Palette.Text,
                fontSize = 19.sp,
                fontWeight = FontWeight.SemiBold,
                fontFamily = FontFamily.Monospace,
                textAlign = TextAlign.Center,
            )
        }
    }
}
