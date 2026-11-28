package dev.openui.compose

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/**
 * Draws a live program's evaluated tree, and redraws when it changes;
 * nothing while it has no root. When the core cut what is shown at a limit
 * ([LiveProgram.sizeLimited], [LiveProgram.depthLimited]), a notice above
 * the tree says so, in the words the Viewer uses, so a cut drawn this way is
 * never silent. The overload that takes an [Element] draws only that element
 * and no notice.
 */
@Composable
public fun ElementView(live: LiveProgram, modifier: Modifier = Modifier) {
    val root = live.root
    if (!live.sizeLimited && !live.depthLimited) {
        root?.let { ElementView(it, modifier) }
        return
    }
    Column(modifier) {
        if (live.sizeLimited) LimitNotice("Cut at the size limit")
        if (live.depthLimited) LimitNotice("Cut at the depth limit")
        root?.let { ElementView(it) }
    }
}

/** A Material 3 notice that what is shown is not the whole program. */
@Composable
private fun LimitNotice(text: String) {
    Surface(
        Modifier.fillMaxWidth().padding(bottom = 8.dp),
        color = MaterialTheme.colorScheme.errorContainer,
        shape = MaterialTheme.shapes.small,
    ) {
        Text(
            text,
            Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
            style = MaterialTheme.typography.labelLarge,
        )
    }
}

/**
 * Draws an element: as one of this library's base components when it knows
 * the component and its props make sense, and otherwise as a labelled box.
 * The base components are Material 3 and follow the app's Material theme.
 *
 * This draws the element alone. It knows nothing of the live program the
 * element came from, so it draws no notice when that program was cut at a
 * limit; a caller drawing a live program's root this way reads
 * [LiveProgram.sizeLimited] and [LiveProgram.depthLimited] and shows the cut itself.
 */
@Composable
public fun ElementView(element: Element, modifier: Modifier = Modifier) {
    when (element.typeName) {
        "TextContent" -> TextContent(element, modifier)
        else -> UnknownElement(element, modifier)
    }
}

@Composable
private fun TextContent(element: Element, modifier: Modifier) {
    val text = (element.prop("text") as? PropValue.Scalar)?.json?.takeIf { it.isString }?.content
    if (text == null) {
        UnknownElement(element, modifier)
        return
    }
    Text(text, modifier.fillMaxWidth(), style = MaterialTheme.typography.bodyLarge)
}

/** The fallback: the component's name in an outlined box, so nothing is dropped silently. */
@Composable
private fun UnknownElement(element: Element, modifier: Modifier) {
    Text(
        element.typeName,
        modifier
            .border(1.dp, MaterialTheme.colorScheme.outline, MaterialTheme.shapes.small)
            .padding(horizontal = 8.dp, vertical = 4.dp),
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        style = MaterialTheme.typography.labelMedium,
    )
}
