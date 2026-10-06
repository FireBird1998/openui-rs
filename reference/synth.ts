// A large program built from a sample: `copies` renamed copies of it, stacked
// under one root. Every statement id `x` of copy k becomes `x_k`, so the
// program is `copies` times the size with the same shape per copy.
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
writeFileSync(out, `root = Stack([${roots.join(", ")}], "column", "l")\n\n${parts.join("\n\n")}\n`);
