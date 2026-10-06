// Writes conformance/merge-generated.json: realistic edit-mode patches for
// every corpus program with a root statement. Deterministic; the expected
// output of each comes from the reference (reference/edits.ts).
//
//   bun reference/merge-cases.ts
//
// Per program: edit a string, add a child, delete a statement, drop the root's
// first child (garbage-collecting its subtree), a fenced patch with prose, a
// patch merged from a non-default root, and the program replayed as its own
// patch, on top of itself and onto nothing.
import { readFileSync, writeFileSync } from "fs";
import { join } from "path";

const CONFORMANCE = join(import.meta.dir, "..", "conformance");
type Case = { name: string; file: string };
const corpus: Case[] = ["manifest.json", "imported.json"].flatMap(
  (f) => JSON.parse(readFileSync(join(CONFORMANCE, f), "utf8")).cases,
);

const STATEMENT = /^(\$?[A-Za-z_][A-Za-z0-9_]*)[ \t]*=(?!=)/;
const STRING = /"(?:[^"\\\n]|\\.)*"/;
const CALL = /\b([A-Z][A-Za-z0-9_]*)\(/;

const cases: object[] = [];
for (const c of corpus) {
  const text = readFileSync(join(CONFORMANCE, c.file), "utf8");
  const lines = text.split("\n");
  const statements = lines
    .map((line) => ({ line, id: STATEMENT.exec(line)?.[1] }))
    .filter((s): s is { line: string; id: string } => !!s.id);
  const root = statements.find((s) => s.id === "root");
  if (!root) continue;
  const others = statements.filter((s) => s.id !== "root");
  const add = (kind: string, patch: string, extra: object = {}) =>
    cases.push({ name: `gen/${c.name}/${kind}`, existingCase: c.name, patch, ...extra, source: "generated" });

  const withString = others.find((s) => STRING.test(s.line));
  const edit = withString ? withString.line.replace(STRING, '"Edited"') : undefined;
  if (edit) add("edit", edit);

  const component = others.map((s) => CALL.exec(s.line)?.[1]).find(Boolean) ?? "Text";
  const added = `added = ${component}("Added")`;
  const rootWithAdded = root.line.includes("[") ? root.line.replace(/\[\s*(\]?)/, (_, close) => (close ? "[added]" : "[added, ")) : undefined;
  add("add", rootWithAdded ? `${rootWithAdded}\n${added}` : added);

  const victim = others[Math.floor(others.length / 2)];
  const remove = victim ? `${victim.id} = null` : undefined;
  if (remove) add("delete", remove);

  const dropped = root.line.replace(/\[\s*\$?[A-Za-z_][A-Za-z0-9_]*\s*(,\s*|(?=\]))/, "[");
  if (dropped !== root.line) add("drop-child", dropped);

  const body = [edit, remove].filter(Boolean).join("\n") || added;
  add("fenced", `Here's the update:\n\n\`\`\`openui-lang\n${body}\n\`\`\`\n\nLet me know if you'd like anything else.`);

  if (others.length && edit) add(`root-${others[0].id}`, edit, { rootId: others[0].id });

  add("replay", text);
  cases.push({ name: `gen/${c.name}/onto-nothing`, existing: "", patchCase: c.name, source: "generated" });
}

writeFileSync(join(CONFORMANCE, "merge-generated.json"), JSON.stringify({ cases }, null, 1) + "\n");
console.log(`Wrote ${cases.length} generated merge cases to conformance/merge-generated.json`);
