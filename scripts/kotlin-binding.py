#!/usr/bin/env python3
"""Hide the generated Kotlin binding inside OpenUICompose.

    kotlin-binding.py GENERATED_KT OUT_KT   # write a copy with every top-level declaration internal
    kotlin-binding.py --check KT            # fail unless every top-level declaration is internal or private
    kotlin-binding.py --self-test

UniFFI writes its Kotlin with public declarations. OpenUICompose compiles the
file as part of its own module, so making each top-level declaration
`internal` keeps all of it out of the library's public API, and the Kotlin
compiler then refuses any public declaration of the library that exposes a
generated type.

Top level is found by reading the file as Kotlin (comments, strings, string
templates, character literals and backticked names skipped; brackets
counted), not by indentation: UniFFI indents its top-level functions. A
declaration is recognised where top-level code starts a line (the first
code on it, outside any bracket), after any annotations. Two things stop
the script, so a new UniFFI template can't leave something public
unnoticed: code starting a top-level line that is none of a declaration,
an annotation, an import, the package, a closing bracket or the rest of an
expression the line before left open; and a declaration keyword (class,
interface, object, fun, val, var, typealias) anywhere at top level outside
a declaration recognised that way, such as a second declaration after a
`;` or a `}` on the same line. It is not a whole Kotlin parser: the
module's explicit API mode (kotlin/OpenUICompose/build.gradle.kts) makes
the compiler refuse, as well, any declaration without an explicit
visibility.
"""
import pathlib
import re
import sys

DECLARATION = re.compile(
    r"(?:(public|internal|private|protected)\s+)?"
    r"(?:(?:sealed|open|abstract|data|inline|enum|annotation|value|const|fun)\s+)*"
    r"(?:class|interface|object|fun|val|var|typealias)\b"
)
ANNOTATIONS = re.compile(r"(?:@[\w.:]+(?:\([^()]*\))?\s*)*")
NOT_A_DECLARATION = re.compile(r"(?:import |package |[})\]]|\{\s*$|$)")
# What a line ending this way leaves open: the next line goes on with it.
CONTINUES = ("=", ",", ".", ":", "->", "(", "&&", "||", "+", "-", "?:")
KEYWORDS = {"class", "interface", "object", "fun", "val", "var", "typealias"}


class Unknown(Exception):
    pass


def top_level_starts(source):
    """Offsets where top-level code starts a line, each with the code ending the line before it,
    and the offsets of declaration keywords at top level."""
    starts, keywords = [], []
    depth = 0  # (, [ and { of code outside string templates
    stack = []  # inside strings: "string", "raw", or a template's brace count
    i, n = 0, len(source)
    line_open = True  # no code yet on this line
    last_code = ""  # the last code before this line, stripped
    current = []  # this line's code characters
    while i < n:
        c = source[i]
        if stack and stack[-1] in ("string", "raw"):
            if stack[-1] == "string" and c == "\\":
                i += 2
                continue
            if stack[-1] == "raw" and source.startswith('"""', i):
                stack.pop()
                i += 3
                continue
            if stack[-1] == "string" and c == '"':
                stack.pop()
            elif source.startswith("${", i):
                stack.append(0)
                i += 2
                continue
            elif c == "\n":
                current.append(" ")
            i += 1
            continue
        if source.startswith("//", i):
            while i < n and source[i] != "\n":
                i += 1
            continue
        if source.startswith("/*", i):
            nesting, i = 1, i + 2
            while i < n and nesting:
                if source.startswith("/*", i):
                    nesting, i = nesting + 1, i + 2
                elif source.startswith("*/", i):
                    nesting, i = nesting - 1, i + 2
                else:
                    i += 1
            continue
        if c == "\n":
            if current:
                last_code = "".join(current).strip() or last_code
            current, line_open = [], True
            i += 1
            continue
        if c in " \t\r":
            i += 1
            continue
        if line_open and depth == 0 and not stack:
            starts.append((i, last_code))
        line_open = False
        if not stack and depth == 0 and (c.isalpha() or c == "_") and not (
                i and (source[i - 1].isalnum() or source[i - 1] == "_")):
            word = re.match(r"\w+", source[i:i + 64])[0]
            if word in KEYWORDS:
                keywords.append(i)
            current.append(word)
            i += len(word)
            continue
        current.append(c)
        if source.startswith('"""', i):
            stack.append("raw")
            i += 3
            continue
        if c == '"':
            stack.append("string")
        elif c == "'":
            end = source.index("'", i + (3 if source[i + 1] == "\\" else 2))
            i = end
        elif c == "`":
            i = source.index("`", i + 1)
        elif stack:  # code inside a string template
            if c == "{":
                stack[-1] += 1
            elif c == "}":
                if stack[-1] == 0:
                    stack.pop()
                else:
                    stack[-1] -= 1
        elif c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        i += 1
    return starts, keywords


def declarations(source):
    """Each top-level declaration as (offset of its first keyword, visibility or None)."""
    found, spans = [], []
    starts, keywords = top_level_starts(source)
    for start, before in starts:
        end = source.find("\n", start)
        line = source[start:end if end >= 0 else len(source)]
        at = ANNOTATIONS.match(line).end()
        match = DECLARATION.match(line, at)
        if match:
            found.append((start + at, match[1]))
            spans.append((start + at, start + match.end()))
        elif not NOT_A_DECLARATION.match(line, at) and not before.endswith(CONTINUES):
            number = source.count("\n", 0, start) + 1
            raise Unknown(f"line {number} isn't a declaration I know: {line.strip()!r}")
    for keyword in keywords:
        if not any(begin <= keyword < end for begin, end in spans):
            number = source.count("\n", 0, keyword) + 1
            line = source[source.rfind("\n", 0, keyword) + 1:source.find("\n", keyword)]
            raise Unknown(f"line {number} has a top-level declaration I can't place: {line.strip()!r}")
    return found


def hidden(source):
    """The source with every public top-level declaration made internal."""
    out, done = [], 0
    for at, visibility in declarations(source):
        if visibility == "protected":
            raise Unknown(f"a protected top-level declaration: {source[at:source.find(chr(10), at)]!r}")
        if visibility in ("internal", "private"):
            continue
        out.append(source[done:at])
        out.append("internal ")
        done = at + (len("public ") if visibility == "public" else 0)
        if visibility == "public":
            while source[done] in " \t":
                done += 1
    out.append(source[done:])
    return "".join(out)


def exposed(source):
    """Top-level declarations that are neither internal nor private, as (line number, text)."""
    return [(source.count("\n", 0, at) + 1, source[at:source.find("\n", at)].strip())
            for at, visibility in declarations(source) if visibility not in ("internal", "private")]


def self_test(argv):
    import unittest

    class Tests(unittest.TestCase):
        def test_hides_every_kind_of_declaration(self):
            source = "\n".join([
                "@file:Suppress(\"X\")", "package p", "import a.b", "@Structure.FieldOrder(\"a\", \"b\")",
                "open class A: B {", "    public fun f() { val s = \"}\" }", "}",
                "public interface I {", "}", "data class D (", "    val x: Int,", ") {", "}",
                "sealed class E: Exception() {", "}", "object O// a comment", "public object F: I {", "}",
                "inline fun <T> T.use(): Int =", "    try { 1 } finally { }",
                "    /**", "     * Indented, as UniFFI writes top-level functions.", "     */ fun g(): String {",
                "        return \"${'$'}{x} {\"", "    }", "    @Throws(E::class) fun h() = '{'",
                "internal const val C = 1", "private fun k() = 1", "val `odd name` = \"\"\"{ \"\"\"", "",
            ])
            result = hidden(source)
            self.assertEqual(exposed(result), [])
            for expected in ["internal open class A: B {\n    public fun f()", "internal interface I {",
                             "internal data class D (", "internal object O// a comment", "internal object F: I",
                             "internal inline fun <T> T.use(): Int =\n    try", "*/ internal fun g(): String",
                             "@Throws(E::class) internal fun h()", "internal const val C = 1\nprivate fun k()",
                             "internal val `odd name`", "@Structure.FieldOrder(\"a\", \"b\")\ninternal open class"]:
                self.assertIn(expected, result)
            self.assertEqual(len(exposed(source)), 10)

        def test_check_finds_a_public_declaration(self):
            self.assertEqual(exposed("internal class A\npublic class B\nclass C {\n  class Nested\n}\n    fun d() = 1\n"),
                             [(2, "public class B"), (3, "class C {"), (6, "fun d() = 1")])

        def test_unknown_top_level_code_stops(self):
            for source in ["package p\nexpect class A\n", "protected class A\n", "val a = 1\nb()\n",
                           # The review's examples (#258): declarations that don't start a line.
                           "class A {}; class B\n", "internal class A {\n} class Leak\n",
                           "internal fun a() = 1; fun b() = 2\n", "internal val a = 1 +\nsuspend fun leak() = 2\n"]:
                with self.assertRaises(Unknown, msg=source):
                    hidden(source)

    result = unittest.TextTestRunner(verbosity=0 if "-q" in argv else 2).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
    sys.exit(0 if result.wasSuccessful() and result.testsRun else 1)


def main(argv):
    try:
        if argv[1:2] == ["--self-test"]:
            self_test(argv)
        elif len(argv) == 3 and argv[1] == "--check":
            found = exposed(pathlib.Path(argv[2]).read_text())
            for number, line in found:
                print(f"{argv[2]}:{number}: not hidden: {line}", file=sys.stderr)
            if found:
                sys.exit(f"kotlin-binding.py: {len(found)} generated declarations would be public API")
            print("Generated binding: every top-level declaration is internal or private")
        elif len(argv) == 3:
            out = pathlib.Path(argv[2])
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(hidden(pathlib.Path(argv[1]).read_text()))
        else:
            sys.exit(__doc__.split("\n\n")[1])
    except Unknown as error:
        sys.exit(f"kotlin-binding.py: {argv[-1]}: {error}")


if __name__ == "__main__":
    main(sys.argv)
