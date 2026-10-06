// Goldens for prompt generation. Every conformance/prompts/<name>.json is the
// argument to generateSystemPrompt; conformance/golden/prompts/<name>.json
// records what it returned ({"prompt": ...}) or threw ({"error": message}).
//
//   bun reference/prompts.ts golden        write every golden
//   bun reference/prompts.ts show <name>   print the reference prompt (or error)
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { join } from "path";

process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const { generateSystemPrompt } = await import("./lang-core/src/parser/prompt");

const CONFORMANCE = join(import.meta.dir, "..", "conformance");
const SPECS = join(CONFORMANCE, "prompts");
const GOLDEN = join(CONFORMANCE, "golden", "prompts");

function run(name: string): { prompt: string } | { error: string } {
  const spec = JSON.parse(readFileSync(join(SPECS, `${name}.json`), "utf8"));
  try {
    return { prompt: generateSystemPrompt(spec) };
  } catch (e) {
    return { error: e instanceof Error ? e.message : String(e) };
  }
}

const names = readdirSync(SPECS)
  .filter((f) => f.endsWith(".json"))
  .map((f) => f.slice(0, -".json".length))
  .sort();

const [command, arg] = process.argv.slice(2);
if (command === "golden") {
  rmSync(GOLDEN, { recursive: true, force: true });
  mkdirSync(GOLDEN, { recursive: true });
  for (const name of names) {
    writeFileSync(join(GOLDEN, `${name}.json`), JSON.stringify(run(name)) + "\n");
  }
  console.log(`Wrote prompt goldens for ${names.length} specs to conformance/golden/prompts/`);
} else if (command === "show" && arg) {
  const result = run(arg);
  console.log("prompt" in result ? result.prompt : `error: ${result.error}`);
} else {
  console.error("usage: bun reference/prompts.ts golden | show <name>");
  process.exit(1);
}
