package dev.orither.ui

import androidx.annotation.DrawableRes
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlin.math.cos
import kotlin.math.sin

private const val ORBIT_MILLIS = 9000
/** Where the first computer sits: up and to the right, as on the desktop. */
private const val FRONT_DEGREES = -40f
private val RINGS = listOf(0.42f, 0.64f, 0.86f)

/** How far out each way of reaching a computer puts it, as on the desktop's orrery. */
internal fun orbitOf(route: String?, online: Boolean): Float = when {
    !online -> RINGS[2]
    route == "local" -> RINGS[0]
    route == "nearby" -> RINGS[1]
    else -> RINGS[2]
}

/** A computer on the orrery. */
internal data class Satellite(val label: String, val orbit: Float, val online: Boolean)

/**
 * The phone at the centre and its computers orbiting it at the distance of their
 * routes: the same picture the desktop draws, seen from the other end.
 */
@Composable
internal fun Orrery(size: Dp, satellites: List<Satellite>, modifier: Modifier = Modifier, front: Int = 0, waiting: Boolean = false) {
    val colors = LocalColors.current
    val measurer = rememberTextMeasurer()
    // Only a phone still waiting for its first computer has anything going round.
    val turn = if (waiting) circling() else 0f
    val orbits = satellites.map { animateFloatAsState(it.orbit, Springs.pop(), label = "orbit").value }
    // Picking another computer turns the satellites round until it is at the front.
    val step = if (satellites.isEmpty()) 0f else 360f / satellites.size
    val rotation by animateFloatAsState(-step * front, Springs.page(), label = "front")
    Canvas(modifier.size(size)) {
        val middle = Offset(this.size.width / 2, this.size.height / 2)
        val radius = this.size.minDimension / 2
        val used = satellites.filter { it.online }.map { it.orbit }.toSet()
        RINGS.forEach { ring ->
            val color = if (ring in used) colors.text.copy(alpha = 0.18f) else colors.border
            drawCircle(color, ring * radius, middle, style = Stroke(1.dp.toPx()))
        }
        // The phone itself, upright, at the centre.
        val phone = Size(radius * 0.17f, radius * 0.28f)
        drawRoundRect(
            colors.dim.copy(alpha = 0.75f),
            Offset(middle.x - phone.width / 2, middle.y - phone.height / 2),
            phone,
            CornerRadius(radius * 0.05f),
        )
        val dot = radius * 0.21f
        if (waiting) {
            val angle = Math.toRadians((FRONT_DEGREES + turn).toDouble())
            val at = middle + Offset(cos(angle).toFloat(), sin(angle).toFloat()) * (RINGS[0] * radius)
            drawCircle(colors.accent.copy(alpha = 0.6f), dot, at, style = Stroke(2.dp.toPx()))
        }
        satellites.forEachIndexed { index, satellite ->
            val degrees = FRONT_DEGREES + step * index + rotation
            val angle = Math.toRadians(degrees.toDouble())
            val at = middle + Offset(cos(angle).toFloat(), sin(angle).toFloat()) * (orbits[index] * radius)
            val first = index == front && satellite.online
            drawCircle(colors.background, dot + 2.dp.toPx(), at)
            if (first) drawCircle(colors.accent, dot, at)
            drawCircle(if (satellite.online) colors.accent else colors.text.copy(alpha = 0.25f), dot, at, style = Stroke(2.dp.toPx()))
            drawLabel(measurer, satellite.label, at, dot, if (first) colors.onAccent else if (satellite.online) colors.accentInk else colors.dim)
        }
    }
}

@Composable
private fun circling(): Float {
    val turn by rememberInfiniteTransition(label = "waiting").animateFloat(
        initialValue = 0f,
        targetValue = 360f,
        animationSpec = infiniteRepeatable(tween(ORBIT_MILLIS, easing = LinearEasing), RepeatMode.Restart),
        label = "waiting",
    )
    return turn
}

private fun androidx.compose.ui.graphics.drawscope.DrawScope.drawLabel(measurer: TextMeasurer, label: String, at: Offset, dot: Float, color: Color) {
    val style = TextStyle(color = color, fontSize = (dot * 0.95f / density / fontScale).sp, fontWeight = FontWeight.SemiBold)
    val laid = measurer.measure(label, style)
    drawText(laid, topLeft = at - Offset(laid.size.width / 2f, laid.size.height / 2f))
}

/** The mark on its own: one ring with a satellite, circling for as long as [alive]. */
@Composable
internal fun Orbit(size: Dp, alive: Boolean, modifier: Modifier = Modifier, color: Color = Palette.Accent) {
    val colors = LocalColors.current
    val turn by rememberInfiniteTransition(label = "orbit").animateFloat(
        initialValue = 0f,
        targetValue = 360f,
        animationSpec = infiniteRepeatable(tween(ORBIT_MILLIS, easing = LinearEasing), RepeatMode.Restart),
        label = "orbit",
    )
    Canvas(modifier.size(size)) {
        val middle = Offset(this.size.width / 2, this.size.height / 2)
        val radius = this.size.minDimension / 2
        drawCircle(colors.text.copy(alpha = 0.18f), radius * 0.72f, middle, style = Stroke(1.5.dp.toPx()))
        val phone = Size(radius * 0.26f, radius * 0.42f)
        drawRoundRect(colors.dim.copy(alpha = 0.75f), Offset(middle.x - phone.width / 2, middle.y - phone.height / 2), phone, CornerRadius(radius * 0.07f))
        val angle = Math.toRadians((FRONT_DEGREES + if (alive) turn else 0f).toDouble())
        val at = middle + Offset(cos(angle).toFloat(), sin(angle).toFloat()) * (radius * 0.72f)
        drawCircle(colors.background, radius * 0.26f, at)
        drawCircle(color, radius * 0.19f, at)
    }
}

/** A surface things sit on. */
@Composable
internal fun Tile(
    modifier: Modifier = Modifier,
    shape: Shape = Shapes.Tile,
    onClick: (() -> Unit)? = null,
    content: @Composable () -> Unit,
) {
    val tappable = if (onClick != null) Modifier.clickable(role = Role.Button, onClick = onClick) else Modifier
    Box(
        modifier
            .clip(shape)
            .background(Palette.Surface)
            .border(1.dp, Palette.Border, shape)
            .then(tappable),
    ) { content() }
}

@Composable
internal fun Glyph(@DrawableRes icon: Int, modifier: Modifier = Modifier, tint: Color = Palette.Text, size: Dp = 22.dp) {
    Icon(painterResource(icon), null, modifier.size(size), tint)
}

@Composable
internal fun Heading(text: String, modifier: Modifier = Modifier) {
    Text(text, modifier, color = Palette.Dim, style = MaterialTheme.typography.labelMedium)
}

/** The screen's one strong button. */
@Composable
internal fun Action(text: String, modifier: Modifier = Modifier, @DrawableRes icon: Int? = null, enabled: Boolean = true, onClick: () -> Unit) {
    Button(
        onClick,
        modifier.heightIn(min = 56.dp),
        enabled = enabled,
        shape = Shapes.Card,
        colors = ButtonDefaults.buttonColors(containerColor = Palette.Accent, contentColor = Palette.OnAccent, disabledContainerColor = Palette.Raised, disabledContentColor = Palette.Dim),
        contentPadding = PaddingValues(horizontal = 22.dp, vertical = 14.dp),
    ) {
        icon?.let { Glyph(it, tint = LocalContentColor.current, size = 20.dp) }
        if (icon != null) Spacer(Modifier.width(10.dp))
        Text(text, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** Every other button: a tonal one, which never competes with the [Action]. */
@Composable
internal fun Quiet(text: String, modifier: Modifier = Modifier, color: Color = Palette.Text, onClick: () -> Unit) {
    FilledTonalButton(
        onClick,
        modifier.heightIn(min = 48.dp),
        shape = Shapes.Small,
        colors = ButtonDefaults.filledTonalButtonColors(containerColor = Palette.Raised, contentColor = color),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 10.dp),
    ) { Text(text, style = MaterialTheme.typography.labelMedium) }
}

/** A row of a title with its explanation. */
@Composable
internal fun Labelled(title: String, about: String, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth(), Arrangement.spacedBy(2.dp)) {
        Text(title, color = Palette.Text, style = MaterialTheme.typography.titleSmall)
        Text(about, color = Palette.Dim, style = MaterialTheme.typography.bodySmall)
    }
}

/** One line of a list: an icon in its well, words, and whatever ends the row. */
@Composable
internal fun Item(
    @DrawableRes icon: Int,
    title: String,
    about: String,
    modifier: Modifier = Modifier,
    tint: Color = Palette.Dim,
    onClick: (() -> Unit)? = null,
    trailing: @Composable RowScope.() -> Unit = {},
) {
    val tappable = if (onClick != null) Modifier.clickable(role = Role.Button, onClick = onClick) else Modifier
    Row(
        modifier.fillMaxWidth().heightIn(min = 56.dp).clip(Shapes.Card).then(tappable).padding(horizontal = 14.dp, vertical = 12.dp),
        Arrangement.spacedBy(14.dp),
        Alignment.CenterVertically,
    ) {
        Box(Modifier.size(36.dp).background(Palette.Raised, Shapes.Small), Alignment.Center) { Glyph(icon, tint = tint, size = 19.dp) }
        Labelled(title, about, Modifier.weight(1f))
        trailing()
    }
}

/** The small light that says whether a computer can be reached. Always next to words saying the same. */
@Composable
internal fun StatusDot(online: Boolean, modifier: Modifier = Modifier) {
    Box(modifier.size(8.dp).background(if (online) Palette.Positive else Palette.Dim, CircleShape))
}
