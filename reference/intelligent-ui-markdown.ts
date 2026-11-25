// Run with bun --no-install from reference/. The Bun loader supplies JSX
// and style stubs and exposes the private parser for structural recording.
import { writeFileSync } from "node:fs";

const stub = (path: string) => ({ path, namespace: "markdown-stub" });
const build = await Bun.build({
  entrypoints: [`${import.meta.dir}/intelligent-ui-markdown/src/components/Markdown/markdownBlocks.tsx`],
  target: "bun",
  plugins: [{
    name: "Intelligent UI Markdown recording",
    setup(build) {
      build.onResolve({ filter: /^(react(\/jsx(-dev)?-runtime)?|clsx)$/ }, ({ path }) => stub(path));
      build.onResolve({ filter: /(?:link|markdown)\.styles$/ }, ({ path }) => stub(path));
      build.onLoad({ filter: /.*/, namespace: "markdown-stub" }, ({ path }) => ({
        loader: "js",
        contents: path.endsWith(".styles")
          ? "export const names = {headingLevel:{}}; export const linkStyles = names; export const markdownStyles = {...names, gap:{}}"
          : `export const Fragment = Symbol.for("markdown-fragment");
             export const jsx = (type, props) => ({type, props});
             export { jsx as jsxs, jsx as jsxDEV };
             export default () => "";`,
      }));
      build.onLoad({ filter: /\/markdownBlocks\.tsx$/ }, async ({ path }) => ({
        loader: "tsx",
        contents: (await Bun.file(path).text()) + "\nexport { blocks };\n",
      }));
    },
  }],
});

// The loader adds the export without editing the vendored file.
type Block =
  | { kind: "p" | "quote" | "code"; text: string }
  | { kind: "h"; level: number; text: string }
  | { kind: "list"; ordered: boolean; start: number; items: string[] }
  | { kind: "hr" }
  | { kind: "table"; head: string[]; rows: string[][] };

type Node = string | Node[] | { type: string | symbol; props: { children: Node; href?: string } };
type Inline = { kind: string; text?: string; target?: string; children?: Inline[] };
const [entry] = build.outputs;
if (!build.success || !entry) throw new AggregateError(build.logs, "Could not bundle Markdown reference");
const { inline, blocks } = await import(`data:text/javascript;base64,${Buffer.from(await entry.text()).toString("base64")}`) as {
  inline: (text: string) => Node;
  blocks: (text: string) => Block[];
};

function structure(node: Node): Inline[] {
  if (typeof node === "string") return [{ kind: "text", text: node }];
  if (Array.isArray(node)) return node.flatMap(structure);
  const children = structure(node.props.children);
  if (node.type === Symbol.for("markdown-fragment")) return children;
  if (typeof node.type !== "string") throw Error("Unexpected JSX node");
  return [{ kind: node.type, ...(node.props.href && { target: node.props.href }), children }];
}

const spans = (text: string) => ({ text, inline: structure(inline(text)) });
const document = (source: string) => blocks(source).map((block) => {
  switch (block.kind) {
    case "h": case "p": case "quote":
      return { kind: block.kind, ...(block.kind === "h" && { level: block.level }), content: spans(block.text) };
    case "list":
      return { ...block, items: block.items.map(spans) };
    case "table":
      return { ...block, head: block.head.map(spans), rows: block.rows.map((row) => row.map(spans)) };
    default:
      return block;
  }
});

const cases: object[] = [];
function add(kind: "inline" | "document" | "both", label: string, source: string) {
  cases.push({
    label, source,
    ...(kind !== "document" && { inline: structure(inline(source)) }),
    ...(kind !== "inline" && { blocks: document(source) }),
  });
}

add("both", "header constructs", [
  "# Heading", "Paragraph **bold** *italic* ~~strike~~ `code` [text](https://example.org).",
  "- First", "- Second", "", "1. Ordered", "", "> Quoted", "> continuation", "",
  "```js", "const value = '**literal**';", "```", "", "---", "",
  "| **Name** | Value |", "| --- | ---: |", "| A | `42` |",
].join("\n"));
add("both", "snake case", "snake_case snake_case_name __init__ some__thing__here foo_bar_baz_qux");
add("both", "triple asterisks", "***bi***");
add("both", "literal punctuation", "<b>x</b> &amp; ![alt](https://example.org/image) \\*word*");

const delimiters = ["*", "**", "***", "_", "__", "~~", "`"];
for (const a of delimiters) for (const b of delimiters) {
  add("both", "two nested delimiters", `${a}a ${b}b${b} c${a}`);
  add("both", "two touching delimiters", `${a}${b}body${b}${a}`);
  add("both", "two adjacent spans", `${a}a${a}${b}b${b}`);
  for (const c of delimiters) {
    add("both", "three nested delimiters", `${a}a ${b}b ${c}c${c} d${b} e${a}`);
    add("both", "three touching delimiters", `${a}${b}${c}body${c}${b}${a}`);
    add("both", "three adjacent spans", `${a}a${a}${b}b${b}${c}c${c}`);
  }
}

for (const target of [
  "https://example.org", "http://example.org/a?x=1#p", "HTTP://EXAMPLE.ORG",
  "https:ok.com", "https:/ok.com", "https:\\ok.com\\one", "https://user:pass@ok.com",
  "https://[::1]:8080/a", "http://127.1/", "https://0x7f000001",
  "https://ok.com:65535", "https://ok.com:65536", "https://ok.com:999999999999999999999",
  "https://[abc]", "https://", "https://%2Fok.com", "http://1.2.3.999", "https://a.09",
  "https://bücher.example/雪%20茶", "\0https://ok.com", "\u0001https://ok.com", "HtTpS://Example.org/a", "https://ok.com/\0a", "https://ok.com/%zz",
  "https://ok.com:443/\u0301", "https://ok.com:443?\u0301", "https://ok.com:443#\u0301",
  "https://ok.com:443\\\u0301", "https://ok.com:\u0301", "http://0x", "http://0x.1", "http://0x..1",
  "https://ok.com/path(1)", "https://ok.com/[x]", "https://ok.com/a\\b?q=\\c",
  "/relative", "relative", "#anchor", "?query", "//ok.com/path", "", "https://ok.com/a b",
  " https://ok.com", "https://ok.com/a\tb", "https://ok.com/a\nb", "https://ok.com/a\u00a0b",
  "javascript:alert(1)", "JaVaScRiPt:x", "java%73cript:x", "&#106;avascript:x", "\0javascript:x",
  "file:///tmp/f", "data:text/html,x", "mailto:x@y.test", "tel:123", "ftp://ok.com", "custom://ok.com",
]) add("both", "target", `[**label**](${target})`);
for (const source of ["[](/x)", "[a\nb](https://ok.com)", "[a[**b**](https://ok.com)", "[a] [b](/relative)",
  "[``code``](https://ok.com)", "[a[b]x](https://ok.com)", "[**a**](broken) *tail*",
  "*[a](https://ok.com/)*", "[*a](https://ok.com/)*", "~~[a\rb](https://ok.com)~~",
]) add("both", "link boundaries", source);

for (let n = 0; n <= 6; n++) {
  for (const marker of ["-", "+", "*", "0.", "7)", "0007."]) {
    const line = " ".repeat(n) + marker + " **item**";
    add("document", "indented item", line);
    add("document", "indented sibling or continuation", "- first\n" + line + "\n- last");
    add("document", "indented continuation", "2. first\n" + " ".repeat(n) + "more\n1. last");
  }
}
for (let n = 1; n <= 6; n++) for (const prefix of ["", " ", "  ", "\t"]) {
  add("document", "heading", prefix + "#".repeat(n) + " *heading*");
}
for (const source of [
  "#", "# ", "####\t", "- ", "0. ", ">", "x\n# ", "x\n# y", "first\n\nsecond",
  " \t```js\n  body\n \t```closing\nnext", "   ```\nunclosed\n", "```", "x\n```\ny\n```",
  "A | B\n:-- | -:\n**row** | [link](/x)\nafter", "A | B\n  |-|-|  \n| only |\na | b | c",
  "| A || C |\n| - | - | - |\n|\n||\n|||", "A | B\n-\nX | Y", "A | B\n|-| |\nX | Y",
  "A | B\n--- |  | ---\nX | Y", "A | B\n|:-- :|\nX | Y", "||\n||", "p\nA | B\n|-|-|\na | b",
  "> first\n  >> second\n\t>\tthird\n>   fourth\nnext", "p\n---\ncontinued", "---\n\n* * *\n\n _ _ _",
  " - - - \n\n-*-\n\n**\n\n__\n\n-------", "x\r\n> a\r> b\r\n\r- z", "*a\rb*",
  "a\r\u0301", "a\r\n\u0301", "> quote\r\u0301", "```\ra\r\u0301\r```",
  "2147483648. Large start\n1. Next",
  "a\u2028b", "- first\u2028second", "# \u2028 ", "- \u2029 ", "# x\u000b", "A | B\n---|---\u0085",
]) add("both", "block boundaries", source);

// The review's stress shapes at sizes small enough for upstream's regexes.
for (const n of [4, 12, 32]) {
  const repeats = [
    "a", "a\n", "> ", ">\n", "- ", "- one\n", "*", "*a ", "*a **b ", "_",
    "_a __b ", "~", "~~a ~b ", "`", "[", "[a ", "[a](", "[a](https://ok.com)",
    "(", "**_x_**", "*_x_*", "[[a](https://ok.com)", ">", "#", "# a\n",
    "|", "-", "---\n", "```\na\n```\n", "<img x> ", "a\r\n", "*\u0301a* ", "é 😀 ",
  ];
  for (const unit of repeats) add("both", "small hostile repeat", unit.repeat(n));
  for (const source of [
    "[".repeat(n) + "x" + "]".repeat(n) + "(https://ok.com)",
    "- first\n" + Array.from({ length: n }, (_, i) => " ".repeat(i) + "- item").join("\n"),
    Array.from({ length: n }, (_, i) => "`".repeat(i + 1) + "a").join(" "),
    "A | B\n- | -\n" + "A | B\n".repeat(n),
    "| A ".repeat(n) + "|\n" + "| - ".repeat(n) + "|\n" + "| X ".repeat(n) + "|",
    "```unclosed\n" + "x\n".repeat(n),
  ]) add("both", "small hostile shape", source);
}
const streamed = "# *Heading*\n- [item](https://ok.com)\n  continued\n\n> ~~quote~~\n\n```js\ncode\n```\n\nA | B\n-| -\nx | y";
for (let end = 0; end <= streamed.length; end++) add("both", "stream prefix", streamed.slice(0, end));

// Xorshift32, with a fixed seed. Include whole Markdown delimiters and
// punctuation as well as letters, line boundaries and URL substrings.
let state = 0x170169;
function random(n: number) {
  state ^= state << 13;
  state ^= state >>> 17;
  state ^= state << 5;
  return (state >>> 0) % n;
}
const alphabet = [...delimiters, "`", "[", "]", "(", ")", "https://ok.com", "/x",
  "a", "b", "word", "2", ".", "-", "+", ">", "#", "|", ":", " ", "  ", "\t", "\n", "\r", "\u00a0", "\u0301", "😀"];
const generated = new Set<string>();
while (generated.size < 4096) {
  const count = 1 + random(48);
  let source = "";
  for (let i = 0; i < count; i++) source += alphabet[random(alphabet.length)];
  generated.add(source);
}
for (const source of generated) add("both", "seeded", source);

// These #169 props pass each string to inline() independently. Label,
// Badge, Link, and ListItem.marker use literal labels upstream.
const textComponents = [
  { component: "Title", prop: "text", parts: ["Release ***ready*** and __*checked*__"] },
  { component: "Text", prop: "text", parts: ["[**site**](https://example.org) and ~~old~~.\nUse `x_y`."] },
  { component: "Text", prop: "text", parts: ["[:](https://example.org:443/\u0301)"] },
  { component: "Text", prop: "text", parts: ["First **no", " cross-part**; ", "", "_last_"] },
  { component: "Text", prop: "text", parts: ["Before ***bi*** ", { component: "Badge", text: "New" }, " then __*done*__"] },
  { component: "Caption", prop: "text", parts: ["A snake_case_name and `a*b*`"] },
  { component: "Quote", prop: "text", parts: ["A ***quote*** and _source_."] },
  { component: "Quote", prop: "cite", parts: ["[**Author**](https://example.org/author)"] },
  { component: "List", prop: "items", parts: ["***first***", "[second](/relative)"] },
  { component: "ListItem", prop: "title", parts: ["**First** _item_"] },
  { component: "ListItem", prop: "description", parts: ["~~Old~~ and ***new***"] },
].map(({ parts, ...item }) => ({
  ...item, parts: parts.map((part) => typeof part === "string" ? spans(part) : part),
}));

writeFileSync("../conformance/golden/intelligent-ui-markdown.json", [
  '{"cases":[',
  cases.map((entry) => JSON.stringify(entry)).join(",\n"),
  '],"textComponents":[',
  textComponents.map((entry) => JSON.stringify(entry)).join(",\n"),
  "]}",
].join("\n") + "\n");
