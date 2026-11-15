// What `@openuidev/lang-core` resolves to in this app (next.config.ts aliases
// it, and react-lang's own import of it with it). Everything is the real
// package, imported by file path so the alias doesn't catch it, except
// createStreamingParser, which picks the engine per call (lib/engines.ts).
import type { LibraryJSONSchema, StreamParser } from "../node_modules/@openuidev/lang-core/dist/index.mjs";
import { createStreamingParser as jsCreateStreamingParser } from "../node_modules/@openuidev/lang-core/dist/index.mjs";
import { chooseParser } from "./engines.ts";

export * from "../node_modules/@openuidev/lang-core/dist/index.mjs";

export function createStreamingParser(schema: LibraryJSONSchema, rootName?: string): StreamParser {
  return chooseParser(jsCreateStreamingParser, schema, rootName);
}

// Explicit export overrides export*: the Rust pane hands the renderer its
// evaluated root directly, without a second upstream evaluation pass.
import { evaluateRoot as jsEvaluateRoot } from "../node_modules/@openuidev/lang-core/dist/index.mjs";
import { rustEvaluation } from "./runtime-adapter.ts";
export function evaluateRoot(...args: Parameters<typeof jsEvaluateRoot>): ReturnType<typeof jsEvaluateRoot> {
  const rust = rustEvaluation(args[0], args[1]);
  return rust ? rust.root : jsEvaluateRoot(...args);
}
