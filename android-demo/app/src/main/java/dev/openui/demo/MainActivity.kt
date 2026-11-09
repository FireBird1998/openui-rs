package dev.openui.demo

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import uniffi.openui_ffi.ComponentLibrary
import uniffi.openui_ffi.StreamSession

/** Streams OpenUI Lang programs through the Rust core (via UniFFI) a few
 * characters at a time, as a model would send them, and draws each result
 * natively as it arrives. Start with `--ez autoplay true` to play every
 * program in turn. */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val schema = assets.open("bench.json").bufferedReader().readText()
        val programs = assets.list("programs")!!.sorted().map {
            Program(it.removeSuffix(".oui"), assets.open("programs/$it").bufferedReader().readText())
        }
        val library = ComponentLibrary(schema)
        val autoplay = intent.getBooleanExtra("autoplay", false)
        setContent { DemoScreen(library, programs, autoplay) }
    }
}

data class Program(val name: String, val text: String) {
    val title get() = name.split('-').joinToString(" ") { it.replaceFirstChar(Char::uppercase) }
}

/** What the screen shows after a push. */
class Frame(
    val root: PV?,
    val pushes: Int,
    val of: Int,
    val chars: Int,
    val coreMicros: Long,
    val totalCoreMicros: Long,
    val elements: Int,
    val cut: Boolean,
    val done: Boolean,
)

/** The text in chunks of `size` code points, as a model streams tokens. */
fun chunksOf(text: String, size: Int): List<String> {
    val out = mutableListOf<String>()
    var i = 0
    while (i < text.length) {
        val end = text.offsetByCodePoints(i, minOf(size, text.codePointCount(i, text.length)))
        out += text.substring(i, end)
        i = end
    }
    return out
}

private const val CHUNK = 6
private const val PUSH_EVERY_MS = 16L

@Composable
fun DemoScreen(library: ComponentLibrary, programs: List<Program>, autoplay: Boolean) {
    var selected by remember { mutableIntStateOf(0) }
    var run by remember { mutableIntStateOf(0) }
    var frame by remember { mutableStateOf<Frame?>(null) }
    val scroll = rememberScrollState()

    LaunchedEffect(selected, run) {
        val program = programs[selected]
        val chunks = chunksOf(program.text, CHUNK)
        frame = null
        scroll.scrollTo(0)
        val session = StreamSession(library, null)
        try {
            var chars = 0
            var total = 0L
            for ((i, chunk) in chunks.withIndex()) {
                chars += chunk.length
                frame = withContext(Dispatchers.Default) {
                    val out = session.push(chunk)
                    val root = rootOf(out.json)
                    val micros = (out.parseNanos.toLong() + out.jsonNanos.toLong()) / 1000
                    total += micros
                    Frame(root, i + 1, chunks.size, chars, micros, total, countElements(root), out.sizeLimited || out.depthLimited, false)
                }
                scroll.scrollTo(scroll.maxValue)
                delay(PUSH_EVERY_MS)
            }
            frame = withContext(Dispatchers.Default) {
                val out = session.finish()
                val root = rootOf(out.json)
                val micros = (out.parseNanos.toLong() + out.jsonNanos.toLong()) / 1000
                total += micros
                Frame(root, chunks.size, chunks.size, chars, micros, total, countElements(root), out.sizeLimited || out.depthLimited, true)
            }
        } finally {
            session.close()
        }
        delay(600)
        scroll.animateScrollTo(0, tween(1500))
        if (autoplay && selected + 1 < programs.size) {
            delay(1500)
            selected += 1
        }
    }

    Column(Modifier.fillMaxSize().background(Palette.bg).windowInsetsPadding(WindowInsets.safeDrawing)) {
        Column(Modifier.fillMaxWidth().background(Color(0xFF111827)).padding(horizontal = 16.dp, vertical = 12.dp)) {
            Txt("OpenUI Lang · Rust core on Android", 17.sp, FontWeight.Bold, Color.White)
            Txt("Streamed through UniFFI, $CHUNK characters a push, drawn natively", 12.sp, color = Color(0xFF9CA3AF))
        }
        Row(
            Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            programs.forEachIndexed { i, program ->
                val on = i == selected
                Box(
                    Modifier.clip(CircleShape).background(if (on) Palette.accent else Palette.surface)
                        .clickable { selected = i; run += 1 }.padding(horizontal = 12.dp, vertical = 6.dp),
                ) { Txt(program.title, 13.sp, FontWeight.Medium, if (on) Color.White else Palette.text) }
            }
        }
        Stats(frame)
        Column(
            Modifier.weight(1f).fillMaxWidth().verticalScroll(scroll).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            frame?.root?.let { Value(it) } ?: Txt("Waiting for the first statement…", 14.sp, color = Palette.muted)
        }
    }
}

@Composable
private fun Stats(frame: Frame?) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 6.dp)) {
        val f = frame
        val line = when {
            f == null -> "Starting…"
            f.done -> "Done · ${f.of} pushes · ${f.chars} chars · ${f.elements} elements · core ${f.totalCoreMicros / f.of.coerceAtLeast(1)} µs a push on average"
            else -> "Push ${f.pushes}/${f.of} · ${f.chars} chars · ${f.elements} elements · core ${f.coreMicros} µs"
        }
        Txt(line + if (f?.cut == true) " · cut" else "", 12.sp, color = Palette.muted)
        Box(Modifier.padding(top = 6.dp).fillMaxWidth().height(3.dp).clip(CircleShape).background(Palette.border)) {
            val fraction = if (f == null) 0f else f.pushes.toFloat() / f.of.coerceAtLeast(1)
            if (fraction > 0f) {
                Box(Modifier.fillMaxWidth(fraction).height(3.dp).background(if (f?.done == true) Palette.tone("success") else Palette.accent))
            }
        }
    }
}
