@file:OptIn(ExperimentalLayoutApi::class)

package dev.openui.demo

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

object Palette {
    val bg = Color(0xFFF4F4F5)
    val surface = Color.White
    val sunk = Color(0xFFEDEDF0)
    val border = Color(0xFFE4E4E7)
    val text = Color(0xFF18181B)
    val muted = Color(0xFF71717A)
    val accent = Color(0xFF4F46E5)
    val partial = Color(0xFFF59E0B)
    val series = listOf(
        Color(0xFF4F46E5), Color(0xFF0EA5E9), Color(0xFF10B981), Color(0xFFF59E0B),
        Color(0xFFEF4444), Color(0xFF8B5CF6), Color(0xFFEC4899), Color(0xFF64748B),
    )

    fun tone(variant: String) = when (variant) {
        "info" -> Color(0xFF2563EB)
        "success" -> Color(0xFF16A34A)
        "warning" -> Color(0xFFD97706)
        "error", "danger" -> Color(0xFFDC2626)
        else -> Color(0xFF52525B)
    }
}

private val card = RoundedCornerShape(12.dp)
private val field = RoundedCornerShape(8.dp)

@Composable
fun Txt(
    s: String,
    size: TextUnit = 15.sp,
    weight: FontWeight = FontWeight.Normal,
    color: Color = Palette.text,
    italic: Boolean = false,
    mono: Boolean = false,
    modifier: Modifier = Modifier,
) = BasicText(
    s,
    modifier,
    style = TextStyle(
        fontSize = size,
        fontWeight = weight,
        color = color,
        fontStyle = if (italic) FontStyle.Italic else FontStyle.Normal,
        fontFamily = if (mono) FontFamily.Monospace else FontFamily.Default,
    ),
)

/** Any value in a result: elements drawn natively, text as text. */
@Composable
fun Value(v: PV) {
    when (v) {
        is PV.El -> Element(v)
        is PV.Str -> Txt(v.s)
        is PV.Num -> Txt(fmt(v.n))
        is PV.Bool -> Txt(v.b.toString(), color = Palette.muted)
        is PV.Expr -> Txt(v.source, italic = true, color = Palette.muted)
        is PV.Lst -> Column(verticalArrangement = Arrangement.spacedBy(8.dp)) { v.items.forEach { Value(it) } }
        is PV.Rec, PV.Null -> {}
    }
}

/** An element; one still arriving gets an amber outline. The outline is only
 * drawn, just outside the element, so it takes no space: the layout doesn't
 * move when the element completes. */
@Composable
fun Element(el: PV.El) {
    val modifier = if (el.partial) {
        Modifier.drawWithContent {
            drawContent()
            val out = 2.dp.toPx()
            drawRoundRect(
                Palette.partial.copy(alpha = 0.8f),
                topLeft = Offset(-out, -out),
                size = Size(size.width + 2 * out, size.height + 2 * out),
                cornerRadius = CornerRadius(6.dp.toPx()),
                style = Stroke(1.dp.toPx()),
            )
        }
    } else {
        Modifier
    }
    Box(modifier) { Native(el) }
}

@Composable
private fun Native(el: PV.El) {
    val p = Props(el)
    when (el.typeName) {
        "TextContent" -> TextContent(p)
        "Label" -> Txt(p.text("text") ?: "", 14.sp, FontWeight.Medium)
        "Stack" -> StackView(p, null)
        "Card" -> StackView(p, p.word("variant", "card"))
        "CardHeader" -> CardHeader(p)
        "Button" -> ButtonView(p)
        "Buttons" -> Flow(8.dp) { p.list("buttons").forEach { Value(it) } }
        "Tag" -> TagView(p)
        "TagBlock" -> Flow(6.dp) { p.list("tags").forEach { Value(it) } }
        "Callout", "TextCallout" -> CalloutView(p)
        "Separator" -> Box(Modifier.fillMaxWidth().height(1.dp).background(Palette.border))
        "Table" -> TableView(p)
        "Form" -> Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            p.list("fields").forEach { Value(it) }
            p.element("buttons")?.let { Element(it) }
        }
        "FormControl" -> Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Txt(p.text("label") ?: "", 14.sp, FontWeight.Medium)
            p.element("input")?.let { Element(it) }
            p.text("hint")?.let { Txt(it, 12.sp, color = Palette.muted) }
        }
        "Input" -> Field(p.text("placeholder") ?: p.text("name") ?: "", 40.dp)
        "TextArea" -> Field(p.text("placeholder") ?: p.text("name") ?: "", 84.dp)
        "Select" -> Field((p.text("placeholder") ?: p.text("name") ?: "") + "  ▾", 40.dp)
        "DatePicker" -> Field("Pick a date  ▾", 40.dp)
        "Slider" -> SliderView(p)
        "SwitchGroup" -> Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            p.elements("items", "SwitchItem").forEach { SwitchItem(Props(it)) }
        }
        "SwitchItem" -> SwitchItem(p)
        "RadioGroup" -> Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            val chosen = p.text("defaultValue")
            p.elements("items", "RadioItem").forEach {
                val item = Props(it)
                Choice(item.text("label") ?: "", item.text("description"), round = true, on = item.text("value") == chosen)
            }
        }
        "CheckBoxGroup" -> Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            p.elements("items", "CheckBoxItem").forEach {
                val item = Props(it)
                Choice(item.text("label") ?: "", item.text("description"), round = false, on = item.bool("defaultChecked"))
            }
        }
        "BarChart", "LineChart", "AreaChart", "HorizontalBarChart" -> SeriesChart(p, el.typeName)
        "PieChart", "RadialChart", "SingleStackedBarChart" -> PieChart(p, donut = p.word("variant", "pie") == "donut" || el.typeName == "RadialChart")
        "Tabs" -> TabsView(p)
        "Accordion" -> AccordionView(p)
        "Steps" -> StepsView(p)
        "MarkDownRenderer" -> Markdown(p.text("textMarkdown") ?: "")
        "CodeBlock" -> Box(Modifier.fillMaxWidth().clip(field).background(Color(0xFF18181B)).padding(12.dp)) {
            Txt(p.text("codeString") ?: "", 12.sp, color = Color(0xFFE4E4E7), mono = true)
        }
        "Image", "ImageBlock" -> ImagePlaceholder(p.text("alt") ?: "image", 160.dp)
        "ImageGallery" -> Flow(8.dp) {
            p.list("images").forEach { _ -> Box(Modifier.width(96.dp)) { ImagePlaceholder("", 72.dp) } }
        }
        else -> Fallback(el)
    }
}

@Composable
private fun Flow(gap: Dp, content: @Composable () -> Unit) =
    FlowRow(horizontalArrangement = Arrangement.spacedBy(gap), verticalArrangement = Arrangement.spacedBy(gap)) { content() }

private fun gapOf(word: String) = when (word) {
    "none" -> 0.dp
    "xs" -> 4.dp
    "s" -> 8.dp
    "l" -> 16.dp
    "xl" -> 24.dp
    "2xl" -> 32.dp
    else -> 12.dp
}

@Composable
private fun TextContent(p: Props) {
    val (size, weight) = when (p.word("size", "default")) {
        "small" -> 13.sp to FontWeight.Normal
        "large" -> 19.sp to FontWeight.Normal
        "small-heavy" -> 13.sp to FontWeight.SemiBold
        "large-heavy" -> 24.sp to FontWeight.Bold
        else -> 15.sp to FontWeight.Normal
    }
    Txt(p.text("text") ?: "", size, weight)
}

/** Stack, and Card, which is a stack on a surface. */
@Composable
private fun StackView(p: Props, variant: String?) {
    val row = p.word("direction", "column") == "row"
    val gap = gapOf(p.word("gap", "m"))
    val children = p.list("children")
    Surface(variant) {
        when {
            row && p.bool("wrap") -> FlowRow(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(gap),
                verticalArrangement = Arrangement.spacedBy(gap),
                maxItemsInEachRow = 2,
            ) { children.forEach { Box(Modifier.weight(1f)) { Value(it) } } }
            row -> Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(gap)) {
                children.forEach { Box(Modifier.weight(1f)) { Value(it) } }
            }
            else -> Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(gap)) {
                children.forEach { Value(it) }
            }
        }
    }
}

@Composable
private fun Surface(variant: String?, content: @Composable () -> Unit) {
    when (variant) {
        null -> content()
        "clear" -> Box(Modifier.padding(4.dp)) { content() }
        "sunk" -> Box(Modifier.fillMaxWidth().clip(card).background(Palette.sunk).padding(14.dp)) { content() }
        else -> Box(
            Modifier.fillMaxWidth().clip(card).background(Palette.surface).border(1.dp, Palette.border, card).padding(14.dp),
        ) { content() }
    }
}

@Composable
private fun CardHeader(p: Props) = Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
    p.text("title")?.let { Txt(it, 17.sp, FontWeight.SemiBold) }
    p.text("subtitle")?.let { Txt(it, 13.sp, color = Palette.muted) }
}

@Composable
private fun ButtonView(p: Props) {
    val destructive = p.word("type", "normal") == "destructive"
    val tint = if (destructive) Palette.tone("error") else Palette.accent
    val label = p.text("label") ?: ""
    when (p.word("variant", "primary")) {
        "secondary" -> Box(Modifier.clip(field).border(1.dp, Palette.border, field).background(Palette.surface).padding(horizontal = 14.dp, vertical = 8.dp)) {
            Txt(label, 14.sp, FontWeight.Medium, tint)
        }
        "tertiary" -> Box(Modifier.padding(horizontal = 6.dp, vertical = 8.dp)) { Txt(label, 14.sp, FontWeight.Medium, tint) }
        else -> Box(Modifier.clip(field).background(tint).padding(horizontal = 14.dp, vertical = 8.dp)) {
            Txt(label, 14.sp, FontWeight.Medium, Color.White)
        }
    }
}

@Composable
private fun TagView(p: Props) {
    val tone = Palette.tone(p.word("variant", "neutral"))
    Box(Modifier.clip(CircleShape).background(tone.copy(alpha = 0.12f)).padding(horizontal = 10.dp, vertical = 3.dp)) {
        Txt(p.text("text") ?: "", 12.sp, FontWeight.Medium, tone)
    }
}

@Composable
private fun CalloutView(p: Props) {
    val tone = Palette.tone(p.word("variant", "neutral"))
    Row(
        Modifier.fillMaxWidth().clip(field).background(tone.copy(alpha = 0.08f)).border(1.dp, tone.copy(alpha = 0.3f), field).padding(12.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Box(Modifier.padding(top = 4.dp).size(10.dp).clip(CircleShape).background(tone))
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            p.text("title")?.let { Txt(it, 14.sp, FontWeight.SemiBold) }
            p.text("description")?.let { Txt(it, 13.sp, color = Palette.muted) }
        }
    }
}

@Composable
private fun TableView(p: Props) {
    val columns = p.elements("columns", "Col").map { Props(it).text("label") ?: "" }
    val rows = p.list("rows")
    val cell = 128.dp
    Column(Modifier.horizontalScroll(rememberScrollState())) {
        Row { columns.forEach { Box(Modifier.width(cell).padding(vertical = 6.dp)) { Txt(it, 12.sp, FontWeight.SemiBold, Palette.muted) } } }
        Box(Modifier.width(cell * columns.size.coerceAtLeast(1)).height(1.dp).background(Palette.border))
        rows.forEach { row ->
            Row {
                val cells = (row as? PV.Lst)?.items ?: listOf(row)
                columns.indices.forEach { c ->
                    Box(Modifier.width(cell).padding(vertical = 7.dp)) {
                        when (val v = cells.getOrNull(c)) {
                            is PV.Str -> Txt(v.s, 14.sp)
                            is PV.Num -> Txt(fmt(v.n), 14.sp)
                            null, PV.Null -> {}
                            else -> Value(v)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun Field(placeholder: String, height: Dp) = Box(
    Modifier.fillMaxWidth().height(height).clip(field).background(Palette.surface).border(1.dp, Palette.border, field).padding(horizontal = 12.dp, vertical = 10.dp),
) { Txt(placeholder, 14.sp, color = Palette.muted) }

@Composable
private fun SliderView(p: Props) {
    val min = p.num("min") ?: 0.0
    val max = p.num("max") ?: 100.0
    val value = (p.v("defaultValue") as? PV.Num)?.n ?: ((p.v("defaultValue") as? PV.Lst)?.items?.firstOrNull() as? PV.Num)?.n ?: min
    val f = if (max > min) ((value - min) / (max - min)).toFloat().coerceIn(0f, 1f) else 0f
    Row(Modifier.fillMaxWidth().height(20.dp), verticalAlignment = Alignment.CenterVertically) {
        if (f > 0f) Box(Modifier.weight(f).height(4.dp).clip(CircleShape).background(Palette.accent))
        Box(Modifier.size(16.dp).clip(CircleShape).background(Palette.accent))
        if (f < 1f) Box(Modifier.weight(1f - f).height(4.dp).clip(CircleShape).background(Palette.border))
    }
}

@Composable
private fun SwitchItem(p: Props) {
    val on = p.bool("checked") || p.bool("defaultChecked")
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Txt(p.text("label") ?: "", 14.sp, FontWeight.Medium)
            p.text("description")?.let { Txt(it, 12.sp, color = Palette.muted) }
        }
        Box(
            Modifier.width(40.dp).height(22.dp).clip(CircleShape).background(if (on) Palette.accent else Palette.border).padding(3.dp),
            contentAlignment = if (on) Alignment.CenterEnd else Alignment.CenterStart,
        ) { Box(Modifier.size(16.dp).clip(CircleShape).background(Color.White)) }
    }
}

@Composable
private fun Choice(label: String, description: String?, round: Boolean, on: Boolean) {
    val shape = if (round) CircleShape else RoundedCornerShape(4.dp)
    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        Box(
            Modifier.padding(top = 2.dp).size(18.dp).clip(shape).border(2.dp, if (on) Palette.accent else Palette.muted, shape).padding(4.dp),
        ) { if (on) Box(Modifier.size(10.dp).clip(shape).background(Palette.accent)) }
        Column {
            Txt(label, 14.sp)
            description?.let { Txt(it, 12.sp, color = Palette.muted) }
        }
    }
}

@Composable
private fun TabsView(p: Props) {
    val tabs = p.elements("items", "TabItem").map { Props(it) }
    var selected by remember { mutableIntStateOf(0) }
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(Modifier.clip(field).background(Palette.sunk).padding(3.dp), horizontalArrangement = Arrangement.spacedBy(3.dp)) {
            tabs.forEachIndexed { i, tab ->
                val on = i == selected
                Box(
                    Modifier.clip(RoundedCornerShape(6.dp)).background(if (on) Palette.surface else Color.Transparent)
                        .clickable { selected = i }.padding(horizontal = 12.dp, vertical = 6.dp),
                ) { Txt(tab.text("trigger") ?: "", 13.sp, if (on) FontWeight.SemiBold else FontWeight.Normal, if (on) Palette.text else Palette.muted) }
            }
        }
        tabs.getOrNull(selected)?.let { tab ->
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) { tab.list("content").forEach { Value(it) } }
        }
    }
}

@Composable
private fun AccordionView(p: Props) {
    var open by remember { mutableStateOf(0) }
    Column(Modifier.fillMaxWidth().clip(field).border(1.dp, Palette.border, field)) {
        p.elements("items", "AccordionItem").forEachIndexed { i, item ->
            val props = Props(item)
            if (i > 0) Box(Modifier.fillMaxWidth().height(1.dp).background(Palette.border))
            Row(Modifier.fillMaxWidth().clickable { open = if (open == i) -1 else i }.padding(12.dp)) {
                Txt(props.text("trigger") ?: "", 14.sp, FontWeight.Medium, modifier = Modifier.weight(1f))
                Txt(if (open == i) "▾" else "▸", 14.sp, color = Palette.muted)
            }
            if (open == i) {
                Column(Modifier.padding(start = 12.dp, end = 12.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    props.list("content").forEach { Value(it) }
                }
            }
        }
    }
}

@Composable
private fun StepsView(p: Props) = Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
    p.elements("items", "StepsItem").forEachIndexed { i, item ->
        val props = Props(item)
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Box(Modifier.size(22.dp).clip(CircleShape).background(Palette.accent), contentAlignment = Alignment.Center) {
                Txt("${i + 1}", 12.sp, FontWeight.Bold, Color.White)
            }
            Column {
                Txt(props.text("title") ?: "", 14.sp, FontWeight.Medium)
                props.text("details")?.let { Txt(it, 12.sp, color = Palette.muted) }
            }
        }
    }
}

/** Enough Markdown for the samples: headings, bullets, bold markers dropped. */
@Composable
private fun Markdown(text: String) = Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
    text.lines().filter { it.isNotBlank() }.forEach { raw ->
        val line = raw.replace("**", "").replace("`", "")
        when {
            line.startsWith("### ") -> Txt(line.drop(4), 15.sp, FontWeight.SemiBold)
            line.startsWith("## ") -> Txt(line.drop(3), 17.sp, FontWeight.SemiBold)
            line.startsWith("# ") -> Txt(line.drop(2), 20.sp, FontWeight.Bold)
            line.trimStart().startsWith("- ") || line.trimStart().startsWith("* ") ->
                Txt("•  " + line.trimStart().drop(2), 14.sp)
            else -> Txt(line, 14.sp)
        }
    }
}

@Composable
private fun ImagePlaceholder(alt: String, height: Dp) = Box(
    Modifier.fillMaxWidth().height(height).clip(field).background(Palette.sunk),
    contentAlignment = Alignment.Center,
) { if (alt.isNotEmpty()) Txt("🖼  $alt", 12.sp, color = Palette.muted) }

/** A component without a native view: its name, then whatever it holds. */
@Composable
private fun Fallback(el: PV.El) = Column(
    Modifier.fillMaxWidth().clip(field).border(1.dp, Palette.border, field).padding(10.dp),
    verticalArrangement = Arrangement.spacedBy(6.dp),
) {
    Txt(el.typeName, 11.sp, FontWeight.SemiBold, Palette.muted)
    el.props.values.forEach { v ->
        if (v is PV.El || v is PV.Lst || v is PV.Str) Value(v)
    }
    Spacer(Modifier.height(0.dp))
}
