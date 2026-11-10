// lang-core's StreamParser interface over the core's WebAssembly StreamSession.
//
// lang-core's `set(fullText)` diffs against the text it already has and
// pushes only the new part, starting over when the text was replaced; this
// does the same. `complete` is lang-core's "the stream has ended" flag, which
// only changes how a `$name` at the very end of the text is read: the core's
// `finish()` is lang-core's `getResult(true)`. Results cross as JSON text and
// are read with JSON.parse, which is part of what the timings measure.
import type { ParseResult, StreamParser } from "@openuidev/lang-core";

/** The parts of the wasm-bindgen StreamSession this uses. */
export interface WasmSession {
  push(chunk: string): string;
  feed(chunk: string): void;
  set(text: string): string;
  result(): string;
  finish(): string;
}

export function wasmStreamParser(session: WasmSession): StreamParser {
  let given = "";
  const read = (json: string) => JSON.parse(json) as ParseResult;

  // The new text, then the result: when the stream has ended, the new text
  // is fed without writing a result, and finish() writes the only one.
  function advance(delta: string, complete?: boolean): ParseResult {
    if (complete) {
      if (delta) session.feed(delta);
      return read(session.finish());
    }
    return read(delta ? session.push(delta) : session.result());
  }

  return {
    push(chunk, complete) {
      given += chunk;
      return advance(chunk, complete);
    },
    set(fullText, complete) {
      if (fullText.startsWith(given)) {
        const delta = fullText.slice(given.length);
        given = fullText;
        return advance(delta, complete);
      }
      // Replaced, not appended: start over, as lang-core does.
      given = fullText;
      if (!complete) return read(session.set(fullText));
      session.set("");
      return advance(fullText, true);
    },
    getResult(complete) {
      return read(complete ? session.finish() : session.result());
    },
  };
}
