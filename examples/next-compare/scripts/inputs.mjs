// Writes the app's generated inputs (git-ignored):
//
// lib/system-prompt.generated.json: react-ui's library prompt, which Live sends
// as the system prompt. Made here, in Node, because a route handler runs
// under React's server build, where react-ui's components can't load.
//
// lib/programs.generated.json: the programs Replay offers:
//   bench/*      the seven benchmark samples from the repo's conformance corpus.
//                They were written for conformance/imported/schemas/bench.json,
//                an earlier react-ui library: against today's openuiLibrary six
//                of them have validation errors (the parser drops the
//                offending components or arguments), so they render only in part.
//   react-ui/*   the examples react-ui ships in its own system prompt, which
//                render cleanly with openuiLibrary.
//   docs/*       two programs from upstream's docs with $state, Query and
//                Mutation, so the ParseResult fields beyond `root` get used
//                (lib/tools.ts answers their tools in memory).
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const { openuiExamples, openuiLibrary, openuiPromptOptions } = await import("@openuidev/react-ui/genui-lib");

const here = dirname(fileURLToPath(import.meta.url));
const cases = join(here, "../../../conformance/imported/cases");
const DOCS = ["openui-lang-patterns-02.oui", "openui-lang-queries-mutations-08.oui"];

const programs = [];
for (const file of readdirSync(join(cases, "bench")).filter((f) => f.endsWith(".oui")).sort()) {
  programs.push({ name: `bench/${file.replace(/\.oui$/, "")}`, text: readFileSync(join(cases, "bench", file), "utf8") });
}
// Each example is a title line ("Example 1 — Table (column-oriented):"), a blank line, then the program.
openuiExamples.forEach((example, i) => {
  const [title, ...rest] = example.split("\n");
  const label = title.replace(/^Example \d+ — /, "").replace(/:$/, "");
  programs.push({ name: `react-ui/${i + 1} ${label}`, text: rest.join("\n").trim() + "\n" });
});
for (const file of DOCS) {
  programs.push({ name: `docs/${file.replace(/\.oui$/, "")}`, text: readFileSync(join(cases, "docs", file), "utf8") });
}
writeFileSync(join(here, "../lib/programs.generated.json"), JSON.stringify(programs, null, 1) + "\n");
writeFileSync(
  join(here, "../lib/system-prompt.generated.json"),
  JSON.stringify({ prompt: openuiLibrary.prompt(openuiPromptOptions) }) + "\n",
);
console.log(`next-compare: wrote react-ui's system prompt and bundled ${programs.length} programs into lib/programs.generated.json`);
