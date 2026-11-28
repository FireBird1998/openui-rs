package dev.openui.compose

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The drawn screen seam, on a device: a component an app defines with its
 * own composable is drawn by that composable when `ElementView` draws a
 * live program on the library, given the element the core evaluated; and a
 * defined composable draws its children with `ElementView(child)`, which
 * finds their composables in the same library.
 */
@RunWith(AndroidJUnit4::class)
class DefinedComponentTest {
    @get:Rule
    val compose = createComposeRule()

    private val label = PropDefinition("label", PropType.string)

    private val shown = mutableListOf<LiveProgram>()

    @After
    fun dispose() {
        shown.forEach(LiveProgram::dispose)
    }

    private fun draw(library: Library, program: String, check: (LiveProgram) -> Unit) {
        val live = LiveProgram(library).also(shown::add)
        runBlocking { live.show(program) }
        check(live)
        compose.mainClock.autoAdvance = false
        compose.setContent {
            MaterialTheme(colorScheme = lightColorScheme()) { Surface { ElementView(live) } }
        }
        compose.mainClock.advanceTimeByFrame()
    }

    @Test
    fun anAppComponentIsDrawnByItsOwnComposable() {
        val badge = defineComponent("Badge", "A short label.", listOf(label)) { element ->
            val text = (element.prop("label") as? PropValue.Scalar)?.json?.content
            Text("Badge says $text")
        }
        draw(createLibrary(listOf(badge), root = "Badge"), "words = \"Rea\" + \"dy\"\nb = Badge(words)") { live ->
            // Drawn from the library's root, with the label the core put together.
            assertEquals("Badge", live.root?.typeName)
        }
        compose.onNodeWithText("Badge says Ready").assertIsDisplayed()
        // Not the labelled box an unknown component gets.
        compose.onNodeWithText("Badge").assertDoesNotExist()
    }

    @Test
    fun withoutAComposableTheSameComponentIsTheLabelledBox() {
        // The precondition of the test above: what draws it there is the composable.
        draw(createLibrary(listOf(defineComponent("Badge", "A short label.", listOf(label)))),
            "root = Badge(\"Ready\")") { live -> assertEquals("Badge", live.root?.typeName) }
        compose.onNodeWithText("Badge").assertIsDisplayed()
        compose.onNodeWithText("Badge says Ready").assertDoesNotExist()
    }

    @Test
    fun aDefinedComposableDrawsItsChildrenWithTheirComposables() {
        val item = defineComponent("Item", "", listOf(label)) { element ->
            Text("Item ${(element.prop("label") as? PropValue.Scalar)?.json?.content}")
        }
        val panel = defineComponent("Panel", "", listOf(PropDefinition("items", PropType.array(PropType.component("Item"))))) { element ->
            (element.prop("items") as? PropValue.ListValue)?.items?.forEach { child ->
                if (child is PropValue.ElementValue) ElementView(child.element)
            }
        }
        draw(createLibrary(listOf(panel, item)), "root = Panel([Item(\"one\"), Item(\"two\")])") { live ->
            assertEquals("Panel", live.root?.typeName)
        }
        compose.onNodeWithText("Item one").assertIsDisplayed()
        compose.onNodeWithText("Item two").assertIsDisplayed()
    }
}
