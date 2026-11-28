package dev.openui.compose

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * Libraries defined in Kotlin, held to upstream's record: every recorded
 * definition the typed declarations can say exactly is said with them, and
 * the library the core builds from it must give upstream's JSON Schema and
 * prompt spec text byte for byte, or upstream's refusal. The recordings are
 * conformance/golden/library-definitions.json and
 * library-definition-roots.json, written by reference/library-definitions.ts
 * from upstream's createLibrary, toJSONSchema and toSpec.
 */
class LibraryDefinitionTest {
    private fun refused(kind: OpenUIException.Kind, message: String, call: () -> Unit) {
        try {
            call()
        } catch (error: OpenUIException) {
            assertEquals(kind, error.kind)
            assertEquals(message, error.message)
            return
        }
        fail("expected $kind: $message")
    }

    @Test
    fun everyRecordedTypedDefinitionGivesUpstreamsSchemaAndSpec() {
        val cases = Recorded.golden("library-definitions.json").jsonArray
        assertEquals("the recording's samples changed", 99, cases.size)
        val asked = mutableListOf<String>()
        var refusals = 0
        for (case in cases) {
            val name = case.name()
            val typed = try {
                typedDefinition(case.jsonObject.getValue("definition"))
            } catch (_: Untyped) {
                continue
            }
            asked += name
            val error = case.field("error")?.text
            if (error != null) {
                refused(OpenUIException.Kind.InvalidJson, error) { typed.create() }
                refusals++
                continue
            }
            val library = typed.create()
            assertEquals("$name: JSON Schema", Recorded.stringify(case.jsonObject.getValue("schema")), library.jsonSchema)
            assertEquals("$name: prompt spec", Recorded.stringify(case.jsonObject.getValue("spec")), library.promptSpec)
        }
        // Every recorded definition of components, groups and a root with
        // the prop types OpenUINative offers. The rest declare functions,
        // actions, an `id`, a default, `minItems` or an unknown kind, which
        // the typed declarations don't (and OpenUINative's don't either).
        assertEquals("cases asked for: $asked", 57, asked.size)
        assertEquals("refusals asked for", 6, refusals)
        for (name in listOf("scalars", "nested-kinds", "binding", "named-type-collisions", "action-slot",
            "reference-cycle", "strict-object", "enum-quotes", "root-missing", "root-description-no-proto-component")) {
            assertTrue("$name wasn't asked for", name in asked)
        }
    }

    @Test
    fun everyPropKindGivesTheCoresSignature() {
        // OpenUINative's ComponentDefinitionTests library, said in Kotlin:
        // the same declarations give the same signature from the core.
        val card = defineComponent("Card", "A card.", listOf(
            PropDefinition("title", PropType.string, binding = true),
            PropDefinition("n", PropType.number, optional = true),
            PropDefinition("flag", PropType.boolean),
            PropDefinition("payload", PropType.any),
            PropDefinition("size", PropType.words(listOf("sm", "lg"))),
            PropDefinition("items", PropType.array(PropType.component("Item"))),
            PropDefinition("choice", PropType.union(listOf(PropType.number, PropType.string))),
            PropDefinition("point", PropType.objectOf(listOf(
                FieldDefinition("y", PropType.number), FieldDefinition("x", PropType.number, optional = true)))),
            PropDefinition("labels", PropType.record(PropType.string)),
            PropDefinition("note", PropType.nullable(PropType.string)),
            PropDefinition("go", PropType.action()),
            PropDefinition("open", PropType.action("OpenUrl")),
            PropDefinition("tint", PropType.named("Color", PropType.string)),
        ))
        val item = defineComponent("Item", "", emptyList())
        val groups = listOf(ComponentGroup("Content", listOf("Item", "Card"), notes = listOf("In order.")))
        val spec = Json.parseToJsonElement(createLibrary(listOf(card, item), groups, root = "Card").promptSpec).jsonObject
        val signature = "Card(title: \$binding<string>, n?: number, flag: boolean, payload: any, " +
            "size: \"sm\" | \"lg\", items: Item[], choice: number | string, point: {y: number, x?: number}, " +
            "labels: Record<string, string>, note?: string, go: ActionExpression, open: @OpenUrl, tint: Color)"
        val components = spec.getValue("components").jsonObject
        assertEquals(JsonPrimitive(signature), components.getValue("Card").field("signature"))
        assertEquals(JsonPrimitive("Item()"), components.getValue("Item").field("signature"))
        assertEquals(JsonPrimitive("Card"), spec["root"])
        assertEquals(
            Json.parseToJsonElement("""[{"name":"Content","components":["Item","Card"],"notes":["In order."]}]"""),
            spec["componentGroups"],
        )
    }

    @Test
    fun aRootThatIsntAComponentIsRefusedInUpstreamsWords() {
        val a = defineComponent("A", "", emptyList())
        refused(OpenUIException.Kind.InvalidJson,
            "[createLibrary] Root component \"B\" was not found in components. Available components: A") {
            createLibrary(listOf(a), root = "B")
        }
    }

    @Test
    fun aDefinitionOfExactlyTheCoresHostInputLimitIsAdmittedAndOneByteMoreIsRefused() {
        // The core's default host input limit (RuntimeLimits.max_json_bytes,
        // 8 MiB) applies to the definition text, counted in UTF-8 bytes;
        // Kotlin adds no limit of its own.
        val limit = 8 * 1024 * 1024
        val prefix = """{"components":[{"name":"A","description":""""
        val suffix = """","props":[]}]}"""
        // This start costs six bytes written as JSON: four for the emoji,
        // two for the escaped line break.
        val description = "😀\n" + "a".repeat(limit - prefix.length - suffix.length - 6)
        val library = createLibrary(listOf(defineComponent("A", description, emptyList())))
        assertTrue("the core kept the whole description", library.promptSpec.contains("a".repeat(1000) + "\""))
        refused(OpenUIException.Kind.LimitExceeded, "host input byte limit") {
            createLibrary(listOf(defineComponent("A", description + "a", emptyList())))
        }
    }

    /** The same element: name, flags, statement and props, all the way down. */
    private fun assertSameElement(where: String, want: Element?, got: Element?) {
        if (want == null || got == null) {
            assertEquals(where, want == null, got == null)
            return
        }
        assertEquals(where, want.toString(), got.toString())
        assertEquals(where, listOf(want.typeName, want.partial, want.hasDynamicProps, want.statementId),
            listOf(got.typeName, got.partial, got.hasDynamicProps, got.statementId))
        assertEquals(where, want.props.map { it.name }, got.props.map { it.name })
        want.props.zip(got.props).forEach { (w, g) -> assertSameValue("$where.${w.name}", w.value, g.value) }
    }

    private fun assertSameValue(where: String, want: PropValue, got: PropValue) {
        when (want) {
            is PropValue.ElementValue -> assertSameElement(where, want.element, (got as? PropValue.ElementValue)?.element)
            is PropValue.ListValue -> {
                val items = (got as? PropValue.ListValue)?.items
                assertEquals(where, want.items.size, items?.size)
                want.items.zip(items!!).forEach { (w, g) -> assertSameValue(where, w, g) }
            }
            else -> assertEquals(where, want, got)
        }
    }

    @Test
    fun aLiveProgramStartsFromTheLibrarysRootAsUpstreamsParserDoes() = runBlocking {
        // react-lang parses with the library's root: a program with no
        // `root` statement starts from the first statement calling it.
        val cases = Recorded.golden("library-definition-roots.json").jsonArray
        assertEquals("the recording's cases changed", 12, cases.size)
        val scalars = Recorded.golden("library-definitions.json").jsonArray.first { it.name() == "scalars" }
        val typed = typedDefinition(scalars.jsonObject.getValue("definition"))
        var drawn = 0
        for (case in cases) {
            assertEquals("scalars", case.name())
            val root = case.field("root")?.text
            val program = case.field("program")!!.text!!
            val recorded = case.field("result")!!.field("root")
            val want = recorded?.takeUnless { it == JsonNull }?.let { elementOrNull(it) ?: throw AssertionError("not an element: $it") }
            val live = LiveProgram(typed.create(root = root))
            try {
                live.show(program)
                assertSameElement("root $root: $program", want, live.root)
                if (want != null) drawn++
            } finally {
                live.dispose()
            }
        }
        // Upstream's parser drew a root in seven of the twelve; four of
        // those have no `root` statement and are drawn from the library's root.
        assertEquals("cases with a root drawn", 7, drawn)
        val none = LiveProgram(typed.create(root = null))
        try {
            none.show("h = Heading(\"Hi\")\np = Page(\"Title\", [h])")
            assertNull("no root statement and no library root draws nothing", none.root)
        } finally {
            none.dispose()
        }
        val page = LiveProgram(typed.create(root = "Page"))
        try {
            page.show("h = Heading(\"Hi\")\np = Page(\"Title\", [h])")
            assertNotNull("the library's root draws the Page statement", page.root)
            assertEquals("p", page.root?.statementId)
        } finally {
            page.dispose()
        }
    }
}
