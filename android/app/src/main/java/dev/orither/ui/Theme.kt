package dev.orither.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.spring
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch

/** One set of the app's colours. The dark one matches the desktop's fallback in `desktop/src/ui/app.css`. */
@Immutable
internal data class Colors(
    val background: Color,
    val surface: Color,
    val raised: Color,
    val text: Color,
    val dim: Color,
    val accent: Color,
    val onAccent: Color,
    val accentInk: Color,
    val border: Color,
    val danger: Color,
    /** Only for "connected": it must read as such whatever the accent is. */
    val positive: Color,
) {
    val accentSoft: Color get() = accent.copy(alpha = 0.16f)
}

internal val DarkColors = Colors(
    background = Color(0xFF171717),
    surface = Color(0xFF1C1C1C),
    raised = Color(0xFF252525),
    text = Color(0xFFEBEBEB),
    dim = Color(0xFFA0A0A0),
    accent = Color(0xFF3DDC84),
    onAccent = Color(0xFF05230F),
    accentInk = Color(0xFF5FE39A),
    border = Color.White.copy(alpha = 0.07f),
    danger = Color(0xFFE5484D),
    positive = Color(0xFF4CC38A),
)

internal val LightColors = Colors(
    background = Color(0xFFF2F2F3),
    surface = Color(0xFFFFFFFF),
    raised = Color(0xFFE9E9EB),
    text = Color(0xFF1B1B1C),
    dim = Color(0xFF5E6166),
    accent = Color(0xFF1FA35C),
    onAccent = Color(0xFFFFFFFF),
    accentInk = Color(0xFF13804A),
    border = Color.Black.copy(alpha = 0.08f),
    danger = Color(0xFFD13438),
    positive = Color(0xFF16A34A),
)

internal val LocalColors = staticCompositionLocalOf { DarkColors }

/** The colours in use, light or dark as the phone is. */
internal object Palette {
    val Background: Color @Composable @ReadOnlyComposable get() = LocalColors.current.background
    val Surface: Color @Composable @ReadOnlyComposable get() = LocalColors.current.surface
    val Raised: Color @Composable @ReadOnlyComposable get() = LocalColors.current.raised
    val Text: Color @Composable @ReadOnlyComposable get() = LocalColors.current.text
    val Dim: Color @Composable @ReadOnlyComposable get() = LocalColors.current.dim
    val Accent: Color @Composable @ReadOnlyComposable get() = LocalColors.current.accent
    val OnAccent: Color @Composable @ReadOnlyComposable get() = LocalColors.current.onAccent
    val AccentInk: Color @Composable @ReadOnlyComposable get() = LocalColors.current.accentInk
    val AccentSoft: Color @Composable @ReadOnlyComposable get() = LocalColors.current.accentSoft
    val Border: Color @Composable @ReadOnlyComposable get() = LocalColors.current.border
    val Danger: Color @Composable @ReadOnlyComposable get() = LocalColors.current.danger
    val Positive: Color @Composable @ReadOnlyComposable get() = LocalColors.current.positive
}

internal object Shapes {
    val Tile = RoundedCornerShape(18.dp)
    val Card = RoundedCornerShape(14.dp)
    val Small = RoundedCornerShape(10.dp)
    val Pill = RoundedCornerShape(50)
}

/** The desktop's motion from `desktop/src/core/motion.ts`, in Compose's terms. */
internal object Springs {
    /** Small things answering a press. */
    fun <T> snappy(): FiniteAnimationSpec<T> = spring(dampingRatio = 1f, stiffness = 700f)

    /** Anything that can be interrupted and turned around. */
    fun <T> page(): FiniteAnimationSpec<T> = spring(dampingRatio = 1f, stiffness = 260f)

    /** A physical object landing. */
    fun <T> pop(): FiniteAnimationSpec<T> = spring(dampingRatio = 0.8f, stiffness = 300f)
}

/** The type roles, the desktop's scale in Android's terms. Sizes in sp, so they follow the phone's font size. */
private val Type = Typography(
    headlineSmall = TextStyle(fontSize = 24.sp, lineHeight = 30.sp, fontWeight = FontWeight.Medium),
    titleMedium = TextStyle(fontSize = 16.sp, lineHeight = 22.sp, fontWeight = FontWeight.SemiBold),
    titleSmall = TextStyle(fontSize = 15.sp, lineHeight = 20.sp, fontWeight = FontWeight.Medium),
    bodyMedium = TextStyle(fontSize = 14.sp, lineHeight = 20.sp),
    bodySmall = TextStyle(fontSize = 13.sp, lineHeight = 18.sp),
    labelLarge = TextStyle(fontSize = 15.sp, lineHeight = 20.sp, fontWeight = FontWeight.SemiBold),
    labelMedium = TextStyle(fontSize = 13.sp, lineHeight = 18.sp, fontWeight = FontWeight.Medium),
)

/**
 * The desktop takes its accent from the desktop's theme; here the phone's own theme
 * gives it, through Material You where Android has it. The neutrals stay the
 * desktop's, so both ends look like one product.
 */
@Composable
internal fun OritherTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val base = if (dark) DarkColors else LightColors
    val dynamic = when {
        Build.VERSION.SDK_INT < Build.VERSION_CODES.S -> null
        dark -> dynamicDarkColorScheme(context)
        else -> dynamicLightColorScheme(context)
    }
    val colors = dynamic?.let { base.copy(accent = it.primary, onAccent = it.onPrimary, accentInk = it.primary) } ?: base
    val material = if (dark) {
        darkColorScheme(primary = colors.accent, onPrimary = colors.onAccent, background = colors.background, surface = colors.surface, onBackground = colors.text, onSurface = colors.text, secondaryContainer = colors.raised, onSecondaryContainer = colors.text)
    } else {
        lightColorScheme(primary = colors.accent, onPrimary = colors.onAccent, background = colors.background, surface = colors.surface, onBackground = colors.text, onSurface = colors.text, secondaryContainer = colors.raised, onSecondaryContainer = colors.text)
    }
    CompositionLocalProvider(LocalColors provides colors) {
        MaterialTheme(colorScheme = material, typography = Type, content = content)
    }
}

private const val PRESSED_SCALE = 0.96f

/** Sinks under the finger the moment it lands, and springs back when it lifts. */
internal fun Modifier.pressable(): Modifier = composed {
    val scale = remember { Animatable(1f) }
    val scope = rememberCoroutineScope()
    graphicsLayer {
        scaleX = scale.value
        scaleY = scale.value
    }.pointerInput(Unit) {
        awaitEachGesture {
            awaitFirstDown(requireUnconsumed = false)
            scope.launch { scale.animateTo(PRESSED_SCALE, Springs.snappy()) }
            waitForUpOrCancellation()
            scope.launch { scale.animateTo(1f, Springs.page()) }
        }
    }
}
