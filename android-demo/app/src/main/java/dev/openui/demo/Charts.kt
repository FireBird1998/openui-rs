@file:OptIn(ExperimentalLayoutApi::class)

package dev.openui.demo

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private fun color(i: Int) = Palette.series[i % Palette.series.size]

@Composable
private fun Legend(names: List<String>) = FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
    names.forEachIndexed { i, name ->
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
            Box(Modifier.size(9.dp).clip(RoundedCornerShape(2.dp)).background(color(i)))
            Txt(name, 12.sp, color = Palette.muted)
        }
    }
}

/** Bar, line and area charts: labels along the bottom, one series per colour. */
@Composable
fun SeriesChart(p: Props, kind: String) {
    val labels = p.list("labels").mapNotNull {
        when (it) {
            is PV.Str -> it.s
            is PV.Num -> fmt(it.n)
            else -> null
        }
    }
    val series = p.elements("series", "Series").map { s ->
        val sp = Props(s)
        (sp.text("category") ?: "") to sp.list("values").map { (it as? PV.Num)?.n ?: 0.0 }
    }
    val bars = kind == "BarChart" || kind == "HorizontalBarChart"
    val stacked = bars && p.word("variant", "grouped") == "stacked"
    val measurer = rememberTextMeasurer()
    val axis = TextStyle(fontSize = 9.sp, color = Palette.muted)
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (labels.isNotEmpty() && series.isNotEmpty()) {
            Canvas(Modifier.fillMaxWidth().height(180.dp)) {
                val bottom = size.height - 16.dp.toPx()
                val n = labels.size
                val slot = size.width / n
                val top = when {
                    stacked -> labels.indices.maxOf { i -> series.sumOf { it.second.getOrElse(i) { 0.0 } } }
                    else -> series.maxOf { it.second.maxOrNull() ?: 0.0 }
                }.coerceAtLeast(1e-9)
                fun y(v: Double) = (bottom - (v / top * bottom * 0.92)).toFloat()
                for (g in 0..3) {
                    val gy = bottom - bottom * 0.92f * g / 3
                    drawLine(Palette.border, Offset(0f, gy), Offset(size.width, gy), 1f)
                }
                if (bars) {
                    labels.indices.forEach { i ->
                        if (stacked) {
                            var base = 0.0
                            series.forEachIndexed { s, (_, values) ->
                                val v = values.getOrElse(i) { 0.0 }
                                drawRect(color(s), Offset(i * slot + slot * 0.15f, y(base + v)), Size(slot * 0.7f, y(base) - y(base + v)))
                                base += v
                            }
                        } else {
                            val w = slot * 0.8f / series.size
                            series.forEachIndexed { s, (_, values) ->
                                val v = values.getOrElse(i) { 0.0 }
                                drawRect(color(s), Offset(i * slot + slot * 0.1f + s * w, y(v)), Size(w * 0.9f, bottom - y(v)))
                            }
                        }
                    }
                } else {
                    series.forEachIndexed { s, (_, values) ->
                        val points = values.take(n).mapIndexed { i, v -> Offset(i * slot + slot / 2, y(v)) }
                        if (points.isEmpty()) return@forEachIndexed
                        val line = Path().apply {
                            moveTo(points[0].x, points[0].y)
                            points.drop(1).forEach { lineTo(it.x, it.y) }
                        }
                        if (kind == "AreaChart") {
                            val area = Path().apply {
                                addPath(line)
                                lineTo(points.last().x, bottom)
                                lineTo(points.first().x, bottom)
                                close()
                            }
                            drawPath(area, color(s).copy(alpha = 0.18f))
                        }
                        drawPath(line, color(s), style = Stroke(2.dp.toPx()))
                        points.forEach { drawCircle(color(s), 2.5.dp.toPx(), it) }
                    }
                }
                // Every label that fits.
                val every = ((n * 34.dp.toPx()) / size.width).toInt().coerceAtLeast(1)
                labels.forEachIndexed { i, label ->
                    if (i % every == 0) {
                        val text = measurer.measure(label.take(6), axis)
                        drawText(text, topLeft = Offset(i * slot + slot / 2 - text.size.width / 2, bottom + 3.dp.toPx()))
                    }
                }
            }
        }
        Legend(series.map { it.first })
    }
}

/** Pie and donut charts. */
@Composable
fun PieChart(p: Props, donut: Boolean) {
    val slices = p.elements("slices", "Slice").map { s ->
        val sp = Props(s)
        (sp.text("category") ?: "") to (sp.num("value") ?: 0.0)
    }
    val total = slices.sumOf { it.second }
    // The legend goes under the pie, so a narrow column still has room for it.
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Canvas(Modifier.size(140.dp)) {
            if (total <= 0) return@Canvas
            var start = -90f
            val stroke = 26.dp.toPx()
            slices.forEachIndexed { i, (_, v) ->
                val sweep = (v / total * 360).toFloat()
                if (donut) {
                    drawArc(color(i), start, sweep - 1f, false, Offset(stroke / 2, stroke / 2), Size(size.width - stroke, size.height - stroke), style = Stroke(stroke))
                } else {
                    drawArc(color(i), start, sweep - 0.5f, true)
                }
                start += sweep
            }
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            slices.forEachIndexed { i, (name, v) ->
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Box(Modifier.size(9.dp).clip(RoundedCornerShape(2.dp)).background(color(i)))
                    Txt("$name  ${fmt(v)}", 12.sp, color = Palette.muted)
                }
            }
        }
    }
}
