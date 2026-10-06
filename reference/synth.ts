// A large program built from a sample: `copies` renamed copies of it, nested
// under one root in groups of four, so the tree gets deeper rather than the
// root wider, as a real program would. Every statement id `x` of copy k
// becomes `x_k`, so the program is `copies` times the size with the same
// shape per copy.
//
//   bun reference/synth.ts <sample.oui> <copies> <out.oui>
import { readFileSync, writeFileSync } from "fs";

process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const { tokenize } = await import("./lang-core/src/parser/lexer");
const { split } = await import("./lang-core/src/parser/statements");

const [sample, copiesArg, out] = process.argv.slice(2);
const copies = Number(copiesArg);
if (!sample || !out || !Number.isInteger(copies) || copies < 1) {
  console.error("usage: bun reference/synth.ts <sample.oui> <copies> <out.oui>");
  process.exit(1);
}
const text = readFileSync(sample, "utf8").trim();
// Longest ids first so `heroTitle` isn't rewritten as `hero_k` + `Title`.
const ids = [...new Set(split(tokenize(text)).map((s) => s.id.replace(/^\$/, "")))].sort(
  (a, b) => b.length - a.length,
);
const roots: string[] = [];
const parts: string[] = [];
for (let k = 1; k <= copies; k++) {
  let copy = text;
  for (const id of ids) copy = copy.replace(new RegExp(`\\b${id}\\b`, "g"), `${id}_${k}`);
  roots.push(`root_${k}`);
  parts.push(copy);
}
// Groups of four, top down: the root, then each level of groups.
const groups: string[] = [];
let level = roots;
let next = 1;
while (level.length > 4) {
  const parents: string[] = [];
  for (let i = 0; i < level.length; i += 4) {
    const id = `group_${next++}`;
    groups.push(`${id} = Stack([${level.slice(i, i + 4).join(", ")}], "column", "l")`);
    parents.push(id);
  }
  level = parents;
}
groups.unshift(`root = Stack([${level.join(", ")}], "column", "l")`);
writeFileSync(out, `${groups.join("\n")}\n\n${parts.join("\n\n")}\n`);
