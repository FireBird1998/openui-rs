// Which parser a Renderer gets, and what each one costs.
//
// react-lang's Renderer builds its parser with
//   createStreamingParser(library.toJSONSchema(), library.root)
// and that import is aliased to ./lang-core-shim.ts, which asks engineFor()
// here. A library wrapped by tagLibrary() registers every schema object its
// toJSONSchema() returns, with the engine and the sink its timings go to.
// Anything else (an untagged library) gets lang-core, counted as JS.
import type { Library, LibraryJSONSchema, StreamParser } from "@openuidev/lang-core";
import initWasm, { ComponentLibrary, StreamSession, initSync } from "../wasm/openui_wasm.js";
import { wasmStreamParser } from "./wasm-adapter.ts";

export type Engine = "js" | "rust";

/** Where one pane's parser timings go: milliseconds per set/push/getResult call. */
export interface ParseSink {
  parse(ms: number): void;
}

type Tag = { engine: Engine; sink?: ParseSink };
const tags = new WeakMap<object, Tag>();

/** Calls made into each engine since the page loaded: proof of which one ran. */
export const parserCalls: Record<Engine, number> = { js: 0, rust: 0 };

/**
 * `library` with a toJSONSchema() that registers what it returns. The schema
 * is a fresh copy each call, so a library that caches its schema can't make
 * one object belong to both engines.
 */
export function tagLibrary<L extends Library>(library: L, engine: Engine, sink?: ParseSink): L {
  return new Proxy(library, {
    get(target, key) {
      if (key === "toJSONSchema") {
        return () => {
          const schema = { ...target.toJSONSchema() };
          tags.set(schema, { engine, sink });
          return schema;
        };
      }
      return Reflect.get(target, key, target);
    },
  });
}

// ─── The WebAssembly core ───

let wasmReady: Promise<void> | null = null;
let wasmLoaded = false;

/** Fetches and instantiates the core once; every page waits on this before rendering. */
export function loadWasm(): Promise<void> {
  wasmReady ??= initWasm().then(() => {
    wasmLoaded = true;
  });
  return wasmReady;
}

/** For Node (npm run compare): the module's bytes, instantiated synchronously. */
export function loadWasmSync(bytes: BufferSource): void {
  initSync({ module: bytes });
  wasmLoaded = true;
}

// One ComponentLibrary per schema object: the Renderer memoizes its schema
// per library, so this is built once per pane.
const compiled = new WeakMap<object, ComponentLibrary>();

function rustParser(schema: LibraryJSONSchema, rootName?: string): StreamParser {
  if (!wasmLoaded) throw new Error("openui-rs: WebAssembly isn't loaded yet (await loadWasm())");
  let library = compiled.get(schema);
  if (!library) {
    library = new ComponentLibrary(JSON.stringify(schema));
    compiled.set(schema, library);
  }
  return wasmStreamParser(new StreamSession(library, rootName));
}

// ─── The seam ───

type Create = (schema: LibraryJSONSchema, rootName?: string) => StreamParser;

/** The parser for `schema`: the core for a schema a "rust" library returned, lang-core otherwise. */
export function chooseParser(jsCreate: Create, schema: LibraryJSONSchema, rootName?: string): StreamParser {
  const tag = tags.get(schema) ?? { engine: "js" as const };
  const parser = tag.engine === "rust" ? rustParser(schema, rootName) : jsCreate(schema, rootName);
  return timed(parser, tag.engine, tag.sink);
}

// Every call is timed and counted. For the core the time includes JSON.parse
// of the result, since that's what the Renderer needs.
function timed(parser: StreamParser, engine: Engine, sink?: ParseSink): StreamParser {
  function wrap<A extends unknown[], R>(call: (...args: A) => R) {
    return (...args: A): R => {
      const started = performance.now();
      const result = call(...args);
      const ms = performance.now() - started;
      parserCalls[engine]++;
      sink?.parse(ms);
      return result;
    };
  }
  return {
    push: wrap(parser.push),
    set: wrap(parser.set),
    getResult: wrap(parser.getResult),
  };
}
