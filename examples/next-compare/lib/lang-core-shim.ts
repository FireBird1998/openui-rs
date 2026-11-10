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
