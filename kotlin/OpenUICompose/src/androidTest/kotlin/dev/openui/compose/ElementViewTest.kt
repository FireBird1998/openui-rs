package dev.openui.compose

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.sp
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonPrimitive
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The drawn screen seam, on a device: a live program's tree, evaluated by the
 * core built for Android and reached through the generated binding, drawn by
 * ElementView under the Compose test rule. Checked through the semantics tree,
 * not pixels. The theme is the test's own (light, then dark, with its own
 * body text size), and the test drives the frame clock itself, so no device
 * setting decides what runs.
 */
@RunWith(AndroidJUnit4::class)
class ElementViewTest {
    @get:Rule
    val compose = createComposeRule()

    private lateinit var live: LiveProgram
    private val text = "Drawn from the core"

    /** Body text at a size no default theme has, so the drawing must take it from the theme. */
    private val typography = Typography(bodyLarge = TextStyle(fontSize = 23.sp, lineHeight = 30.sp))

    @Before
    fun showProgram() {
        val library = createLibrary(listOf(
            defineComponent("TextContent", "A paragraph of text.", listOf(PropDefinition("text", PropType.string))),
        ))
        live = LiveProgram(library)
        runBlocking { live.show("words = \"Drawn from\"\nroot = TextContent(words + \" the core\")") }
        // The binding loaded and the core evaluated the tree: the text is
        // put together by the core's evaluator.
        assertEquals("TextContent", live.root?.typeName)
        assertEquals(PropValue.Scalar(JsonPrimitive(text)), live.root?.prop("text"))
    }

    @After
    fun dispose() {
        // Only if @Before got that far, so its own failure is what's reported.
        if (::live.isInitialized) live.dispose()
    }

    private fun draw(colors: ColorScheme = lightColorScheme()) {
        compose.mainClock.autoAdvance = false
        compose.setContent {
            MaterialTheme(colorScheme = colors, typography = typography) {
                Surface { ElementView(live) }
            }
        }
        compose.mainClock.advanceTimeByFrame()
    }

    @Test
    fun textContentIsMaterialTextInTheLightTheme() = drawsTextContent(lightColorScheme())

    @Test
    fun textContentIsMaterialTextInTheDarkTheme() = drawsTextContent(darkColorScheme())

    private fun drawsTextContent(colors: ColorScheme) {
        // The two themes must tell apart what this checks, and the test's
        // body size must not be the default one.
        assertNotEquals(lightColorScheme().onSurface, darkColorScheme().onSurface)
        assertNotEquals(Typography().bodyLarge.fontSize, typography.bodyLarge.fontSize)
        draw(colors)

        val node = compose.onNodeWithText(text)
        node.assertIsDisplayed()
        val layouts = mutableListOf<TextLayoutResult>()
        val layout = node.fetchSemanticsNode().config[SemanticsActions.GetTextLayoutResult].action
        assertEquals(true, layout?.invoke(layouts))
        val style = layouts.single().layoutInput.style
        // The theme's body text, in the theme's colour for text on a surface.
        assertEquals(colors.onSurface, style.color)
        assertEquals(typography.bodyLarge.fontSize, style.fontSize)
        compose.onNodeWithText("Cut at the size limit").assertDoesNotExist()
        compose.onNodeWithText("Cut at the depth limit").assertDoesNotExist()
    }

    @Test
    fun aNewProgramIsRedrawn() {
        draw()
        compose.onNodeWithText(text).assertIsDisplayed()

        runBlocking { live.show("root = TextContent(\"Shown next\")") }
        // Let the main thread take the state change, then draw one frame.
        compose.waitForIdle()
        compose.mainClock.advanceTimeByFrame()

        compose.onNodeWithText("Shown next").assertIsDisplayed()
        compose.onNodeWithText(text).assertDoesNotExist()
    }

    @Test
    fun aCutAtTheSizeLimitIsShownAboveTheTree() {
        // Over 1 MiB, the core's default program limit, cut where the
        // unused padding is, so what is drawn stays small.
        val program = "root = TextContent(\"Kept\")\npadding = \"" + "a".repeat(1 shl 20) + "\""
        runBlocking { live.show(program) }
        assertTrue("the core reported the cut (a program over 1 MiB)", live.sizeLimited)
        draw()

        compose.onNodeWithText("Cut at the size limit").assertIsDisplayed()
        compose.onNodeWithText("Kept").assertIsDisplayed()
        compose.onNodeWithText("Cut at the depth limit").assertDoesNotExist()
    }

    @Test
    fun aCutAtTheDepthLimitIsShownEvenWithNoTree() {
        runBlocking { live.show("root = TextContent(" + "(".repeat(5000)) }
        assertTrue("the core reported the cut (5000 levels, past 256)", live.depthLimited)
        draw()

        compose.onNodeWithText("Cut at the depth limit").assertIsDisplayed()
    }
}
