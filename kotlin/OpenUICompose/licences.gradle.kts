// The licence check (applied by build.gradle.kts).
//
// The rule, for every library a build resolves:
//
// - Its licences are read from its published POM; when the POM names none,
//   from its parent POM, and so on up. None anywhere fails.
// - Each licence's name, compared whole after lower-casing and collapsing
//   spaces, must be in the table of approved names below (MIT, Apache-2.0,
//   Unicode, Unlicense-or-MIT, Apache-2.0 WITH LLVM-exception). No part of a
//   name counts on its own, so "Apache License 2.0 (modified ...)" fails.
// - The name governs. A URL never approves anything; a licence that gives
//   a URL must give one of the known URLs of the licence its name names,
//   or it fails.
// - A POM listing more than one licence fails, unless the library is on the
//   multi-licence allowlist, at exactly the version there, and one of the
//   POM's licences is the approved option the allowlist names.
// - What ships (the release runtime classpath) must pass on that alone.
//   What only the tests and debug builds use (the unit test, instrumented
//   test and debug runtime classpaths, less what ships) may also use the
//   test-only allowlist: a library at exactly that version, under exactly
//   that licence name. Those libraries must not ship: they must not be on
//   the release runtime classpath, and none of their classes may be in the
//   release .aar.
//
// `checkLicencesSelfTest` runs the rule on fake POMs, so a gap can't reopen.

import java.util.zip.ZipFile
import java.util.zip.ZipInputStream
import javax.xml.parsers.DocumentBuilderFactory
import org.gradle.api.artifacts.component.ModuleComponentIdentifier
import org.w3c.dom.Element as XmlElement

/** Approved licence names, normalised (lower case, single spaces), to the licence they name. */
val approvedNames = mapOf(
    "apache-2.0" to "Apache-2.0",
    "apache 2.0" to "Apache-2.0",
    "apache license, version 2.0" to "Apache-2.0",
    "the apache license, version 2.0" to "Apache-2.0",
    "the apache software license, version 2.0" to "Apache-2.0",
    "mit" to "MIT",
    "mit license" to "MIT",
    "the mit license" to "MIT",
    "unicode-3.0" to "Unicode",
    "unicode license v3" to "Unicode",
    "unlicense or mit" to "Unlicense-or-MIT",
    "mit or unlicense" to "Unlicense-or-MIT",
    "apache-2.0 with llvm-exception" to "Apache-2.0 WITH LLVM-exception",
)

/** The known URLs of each approved licence, normalised (no scheme, no www., no trailing slash). */
val approvedUrls = mapOf(
    "Apache-2.0" to setOf(
        "apache.org/licenses/license-2.0", "apache.org/licenses/license-2.0.txt",
        "apache.org/licenses/license-2.0.html", "opensource.org/licenses/apache-2.0",
    ),
    "MIT" to setOf("opensource.org/licenses/mit", "opensource.org/licenses/mit-license.php"),
    "Unicode" to setOf("unicode.org/license.txt", "spdx.org/licenses/unicode-3.0.html"),
    "Unlicense-or-MIT" to setOf<String>(),
    "Apache-2.0 WITH LLVM-exception" to setOf("spdx.org/licenses/llvm-exception.html"),
)

/** A library whose POM offers several licences, accepted by the maintainer under one of them. */
data class MultiLicenceRuling(val module: String, val version: String, val option: String, val date: String)

val multiLicenceAllowlist = listOf(
    // JNA offers LGPL-2.1-or-later or Apache-2.0; the generated binding needs it.
    MultiLicenceRuling("net.java.dev.jna:jna", "5.17.0", option = "Apache-2.0", date = "2026-11-28"),
)

/** A library outside the approved licences, accepted by the maintainer for tests only; it never ships. */
data class TestOnlyRuling(val module: String, val version: String, val licence: String, val reason: String, val date: String)

val testOnlyAllowlist = listOf(
    TestOnlyRuling("junit:junit", "4.13.2", licence = "Eclipse Public License 1.0",
        reason = "JUnit 4 runs the JVM and instrumented tests; a test-only tool that never ships", date = "2026-11-28"),
    // JUnit 4's runner loads Hamcrest's classes itself (org.hamcrest.SelfDescribing),
    // so the tests can't run without it. Not a general approval of BSD-3-Clause.
    TestOnlyRuling("org.hamcrest:hamcrest-core", "1.3", licence = "New BSD License",
        reason = "JUnit 4.13.2's dependency, needed by its runner; test only, never ships", date = "2026-11-28"),
    // The same Hamcrest 1.3 project's other two jars. Espresso, which the Compose test rule
    // uses, loads org.hamcrest.Matchers from them, so the instrumented tests can't run without them.
    TestOnlyRuling("org.hamcrest:hamcrest-library", "1.3", licence = "New BSD License",
        reason = "Espresso's dependency, under the Compose test rule; test only, never ships", date = "2026-11-28"),
    TestOnlyRuling("org.hamcrest:hamcrest-integration", "1.3", licence = "New BSD License",
        reason = "Espresso's dependency, under the Compose test rule; test only, never ships", date = "2026-11-28"),
)

fun normalName(name: String) = name.trim().lowercase().replace(Regex("""\s+"""), " ")
fun normalUrl(url: String) =
    url.trim().lowercase().removePrefix("https://").removePrefix("http://").removePrefix("www.").removeSuffix("/")

data class PomLicence(val name: String, val url: String)
data class Pom(val licences: List<PomLicence>, val parent: String?)

fun readPom(text: String): Pom {
    val factory = DocumentBuilderFactory.newInstance()
    factory.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true)
    val root = factory.newDocumentBuilder().parse(text.byteInputStream()).documentElement
    fun XmlElement.children(tag: String) =
        (0 until childNodes.length).map { childNodes.item(it) }.filterIsInstance<XmlElement>().filter { it.tagName == tag }
    fun XmlElement.text(tag: String) = children(tag).firstOrNull()?.textContent?.trim().orEmpty()
    val licences = root.children("licenses").flatMap { it.children("license") }
        .map { PomLicence(it.text("name"), it.text("url")) }
    val parent = root.children("parent").firstOrNull()
        ?.let { "${it.text("groupId")}:${it.text("artifactId")}:${it.text("version")}" }
    return Pom(licences, parent)
}

/** The approved licence this POM entry is, or why it isn't one. */
fun approved(licence: PomLicence): Pair<String?, String> {
    val named = approvedNames[normalName(licence.name)]
        ?: return null to "\"${licence.name}\" is not an approved licence name"
    if (licence.url.isNotBlank() && normalUrl(licence.url) !in approvedUrls.getValue(named)) {
        return null to "\"${licence.name}\" with the URL ${licence.url}, which is not $named's"
    }
    return named to named
}

data class Verdict(val module: String, val approved: Boolean, val text: String)

/**
 * Judges one library, `group:name:version`, reading POMs through `pom`.
 * `testOnly` allows the test-only allowlist.
 */
fun judge(module: String, testOnly: Boolean, pom: (String) -> String): Verdict {
    val (group, name, version) = module.split(":")
    var read = readPom(pom(module))
    var from = "its POM"
    while (read.licences.isEmpty() && read.parent != null) {
        from = "parent POM ${read.parent}"
        read = readPom(pom(read.parent!!))
    }
    val licences = read.licences
    fun fail(why: String) = Verdict(module, false, "$why (from $from)")
    if (licences.isEmpty()) return Verdict(module, false, "no licence in its POM or any parent POM")
    if (licences.size > 1) {
        val offered = licences.joinToString(" / ") { it.name }
        val ruling = multiLicenceAllowlist.firstOrNull { it.module == "$group:$name" }
            ?: return fail("offers ${licences.size} licences ($offered) and is not on the multi-licence allowlist")
        if (ruling.version != version) {
            return fail("offers $offered; the multi-licence allowlist accepts only version ${ruling.version}")
        }
        if (licences.none { approved(it).first == ruling.option }) {
            return fail("offers $offered, without the allowlisted option ${ruling.option}")
        }
        return Verdict(module, true, "${ruling.option}, the option taken from $offered " +
            "(multi-licence allowlist, ruled ${ruling.date}; from $from)")
    }
    val (licence, why) = approved(licences.single())
    if (licence != null) return Verdict(module, true, "$licence (from $from)")
    val ruling = testOnlyAllowlist.firstOrNull { it.module == "$group:$name" }
    if (testOnly && ruling != null && ruling.version == version &&
        normalName(ruling.licence) == normalName(licences.single().name)
    ) {
        return Verdict(module, true, "${ruling.licence}, test only (test-only allowlist, ruled ${ruling.date}; from $from)")
    }
    return fail(why)
}

fun pomOf(coordinates: String): String =
    configurations.detachedConfiguration(dependencies.create("$coordinates@pom"))
        .apply { isTransitive = false }.singleFile.readText()

fun modules(configuration: Configuration): Set<String> =
    configuration.incoming.resolutionResult.allComponents
        .mapNotNull { it.id as? ModuleComponentIdentifier }
        .map { "${it.group}:${it.module}:${it.version}" }
        .toSortedSet()

/** Judges `modules`, writes `report`, and fails if any isn't approved. */
fun checkModules(label: String, modules: Set<String>, testOnly: Boolean, report: File) {
    val verdicts = modules.map { judge(it, testOnly) { coordinates -> pomOf(coordinates) } }
    val failures = verdicts.filterNot { it.approved }
    report.parentFile.mkdirs()
    report.writeText(verdicts.joinToString("\n", postfix = "\n") {
        (if (it.approved) "" else "NOT APPROVED ") + "${it.module}: ${it.text}"
    })
    logger.lifecycle("$label: ${modules.size} libraries, ${modules.size - failures.size} approved (report: $report)")
    verdicts.filter { it.approved && "allowlist" in it.text }.forEach { logger.lifecycle("  ${it.module}: ${it.text}") }
    if (failures.isNotEmpty()) {
        throw GradleException("$label: libraries outside the approved licences:\n" +
            failures.joinToString("\n") { "  ${it.module}: ${it.text}" })
    }
}

val reports = layout.buildDirectory.dir("reports/licences")

tasks.register("checkLicences") {
    group = "verification"
    description = "Checks the licence of every library that ships, and of every library only the tests use."
    doLast {
        val shipped = modules(configurations.getByName("releaseRuntimeClasspath"))
        checkModules("The release runtime classpath", shipped, testOnly = false, reports.get().file("release.txt").asFile)
        val testAndDebug = listOf("debugUnitTestRuntimeClasspath", "debugAndroidTestRuntimeClasspath", "debugRuntimeClasspath")
            .flatMap { modules(configurations.getByName(it)) }.toSortedSet() - shipped
        checkModules("The test and debug classpaths, less what ships", testAndDebug, testOnly = true,
            reports.get().file("test.txt").asFile)
        val leaked = shipped.filter { module -> testOnlyAllowlist.any { module.startsWith(it.module + ":") } }
        if (leaked.isNotEmpty()) throw GradleException("Test-only libraries on the release runtime classpath: $leaked")
    }
}

// The spec's starting libraries (#257), with Coil and Vico that this
// library doesn't use yet: resolved as the release runtime classpath would
// resolve them, and judged as shipped, so their licences and their
// dependencies' are known before a ticket adds them.
tasks.register("checkCandidateLicences") {
    group = "verification"
    description = "Checks the licences of the spec's starting libraries, including those not added yet."
    doLast {
        val release = configurations.getByName("releaseRuntimeClasspath")
        val candidates = configurations.detachedConfiguration(
            *listOf(
                "androidx.compose.material3:material3:1.4.0",
                "org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2",
                "org.jetbrains.kotlinx:kotlinx-serialization-json:1.9.0",
                "io.coil-kt.coil3:coil-compose:3.3.0",
                "com.patrykandpatrick.vico:compose-m3:2.2.1",
            ).map { dependencies.create(it) }.toTypedArray()
        )
        candidates.attributes {
            for (key in release.attributes.keySet()) {
                @Suppress("UNCHECKED_CAST")
                attribute(key as Attribute<Any>, release.attributes.getAttribute(key)!!)
            }
        }
        checkModules("The starting libraries, Coil and Vico included", modules(candidates), testOnly = false,
            reports.get().file("candidates.txt").asFile)
    }
}

// The test-only libraries' packages, which must not be in the release .aar.
val testOnlyPackages = mapOf("junit:junit" to listOf("org/junit/", "junit/"), "org.hamcrest:hamcrest-core" to listOf("org/hamcrest/"),
    "org.hamcrest:hamcrest-library" to listOf("org/hamcrest/"), "org.hamcrest:hamcrest-integration" to listOf("org/hamcrest/"))

tasks.register("checkTestOnlyNotShipped") {
    group = "verification"
    description = "Checks that no test-only library's classes are in the release .aar."
    dependsOn("bundleReleaseAar")
    doLast {
        val aar = layout.buildDirectory.file("outputs/aar/OpenUICompose-release.aar").get().asFile
        check(testOnlyAllowlist.all { it.module in testOnlyPackages }) { "every test-only library needs its packages listed" }
        val prefixes = testOnlyPackages.values.flatten()
        val found = mutableListOf<String>()
        var jars = 0
        ZipFile(aar).use { zip ->
            for (entry in zip.entries()) {
                if (!entry.name.endsWith(".jar")) continue
                jars++
                ZipInputStream(zip.getInputStream(entry)).use { jar ->
                    generateSequence { jar.nextEntry }.forEach { inner ->
                        if (prefixes.any { inner.name.startsWith(it) }) found += "${entry.name}!${inner.name}"
                    }
                }
            }
        }
        check(jars > 0) { "no jar in $aar" }
        if (found.isNotEmpty()) throw GradleException("Test-only classes in the release .aar:\n" + found.joinToString("\n"))
        logger.lifecycle("The release .aar: $jars jar(s), no classes from ${prefixes.joinToString(", ")}")
    }
}

tasks.register("checkLicencesSelfTest") {
    group = "verification"
    description = "Runs the licence rule on fake POMs."
    doLast {
        fun pom(licences: String, parent: String = "") =
            """<?xml version="1.0"?><project><modelVersion>4.0.0</modelVersion>$parent<licenses>$licences</licenses></project>"""
        fun licence(name: String, url: String = "") = "<license><name>$name</name><url>$url</url></license>"
        val apacheUrl = "https://www.apache.org/licenses/LICENSE-2.0.txt"
        val mitUrl = "https://opensource.org/licenses/MIT"
        val poms = mapOf(
            "t:apache:1" to pom(licence("The Apache Software License, Version 2.0", apacheUrl)),
            "t:mit:1" to pom(licence("MIT", mitUrl)),
            "t:child:1" to pom("", "<parent><groupId>t</groupId><artifactId>parent</artifactId><version>1</version></parent>"),
            "t:parent:1" to pom(licence("Apache-2.0")),
            "t:transitive:1" to pom(licence("The MIT License")),
            "t:unlicense-or-mit:1" to pom(licence("Unlicense OR MIT")),
            "net.java.dev.jna:jna:5.17.0" to pom(licence("LGPL-2.1-or-later", "https://www.gnu.org/licenses/old-licenses/lgpl-2.1") +
                licence("Apache-2.0", "https://www.apache.org/licenses/LICENSE-2.0.txt")),
            "net.java.dev.jna:jna:5.18.0" to pom(licence("LGPL-2.1-or-later") + licence("Apache-2.0")),
            "junit:junit:4.13.2" to pom(licence("Eclipse Public License 1.0", "http://www.eclipse.org/legal/epl-v10.html")),
            "junit:junit:4.13.3" to pom(licence("Eclipse Public License 1.0")),
            "org.hamcrest:hamcrest-core:1.3" to pom("", "<parent><groupId>org.hamcrest</groupId><artifactId>hamcrest-parent</artifactId><version>1.3</version></parent>"),
            "org.hamcrest:hamcrest-parent:1.3" to pom(licence("New BSD License", "http://www.opensource.org/licenses/bsd-license.php")),
            "t:epl-tool:1" to pom(licence("Eclipse Public License 1.0")),
            // The review's counterexamples (#258).
            "t:modified:1" to pom(licence("Apache License 2.0 (modified: no commercial use)", apacheUrl)),
            "t:apache-1.1:1" to pom(licence("Apache License 1.1 with LLVM notes")),
            "t:gpl-apache-url:1" to pom(licence("GPL-3.0", apacheUrl)),
            "t:proprietary-mit-url:1" to pom(licence("Proprietary", mitUrl)),
            "t:unicode-noncommercial:1" to pom(licence("Unicode License (non-commercial use only)")),
            "t:gpl-or-apache:1" to pom(licence("GPL-3.0-only") + licence("Apache-2.0", apacheUrl)),
            "t:apache-mit-url:1" to pom(licence("Apache-2.0", mitUrl)),
            "t:none:1" to pom(""),
        )
        val expected = listOf(
            Triple("t:apache:1", false, true), Triple("t:mit:1", false, true), Triple("t:child:1", false, true),
            Triple("t:transitive:1", false, true), Triple("t:unlicense-or-mit:1", false, true),
            Triple("net.java.dev.jna:jna:5.17.0", false, true), Triple("net.java.dev.jna:jna:5.18.0", false, false),
            Triple("junit:junit:4.13.2", true, true), Triple("junit:junit:4.13.2", false, false),
            Triple("junit:junit:4.13.3", true, false), Triple("org.hamcrest:hamcrest-core:1.3", true, true),
            Triple("org.hamcrest:hamcrest-core:1.3", false, false), Triple("t:epl-tool:1", true, false),
            Triple("t:modified:1", false, false), Triple("t:apache-1.1:1", false, false),
            Triple("t:gpl-apache-url:1", false, false), Triple("t:proprietary-mit-url:1", false, false),
            Triple("t:unicode-noncommercial:1", false, false), Triple("t:gpl-or-apache:1", false, false),
            Triple("t:gpl-or-apache:1", true, false), Triple("t:apache-mit-url:1", false, false),
            Triple("t:none:1", true, false),
        )
        val wrong = expected.mapNotNull { (module, testOnly, pass) ->
            val verdict = judge(module, testOnly) { poms.getValue(it) }
            if (verdict.approved == pass) null
            else "$module (${if (testOnly) "test only" else "shipped"}): expected ${if (pass) "approved" else "refused"}, got ${verdict.text}"
        }
        if (wrong.isNotEmpty()) throw GradleException("Licence rule self-test failed:\n" + wrong.joinToString("\n") { "  $it" })
        logger.lifecycle("Licence rule self-test: ${expected.size} cases, " +
            "${expected.count { it.third }} approved and ${expected.count { !it.third }} refused as expected")
    }
}
