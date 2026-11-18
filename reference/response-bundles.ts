// The renderer source keeps its upstream import. Resolve that import to the
// pinned lang-core message reader for both golden generation and fuzzing.
import { readFileSync, writeFileSync } from "node:fs";
import { parseResponseBundle } from "./responseBundle";

type Case = { name: string; raw?: string | null; units?: number[] };
export type ResponseInput = string | null | number[];

function responseText(input: ResponseInput | undefined) {
  // Case and fuzz inputs are small. Unit arrays preserve literal lone
  // surrogates alongside real Unicode scalars on the Rust/JSON wire.
  return Array.isArray(input) ? String.fromCharCode(...input) : input ?? null;
}

function read(response: string | null, streaming: boolean, revisions: Map<string, number>) {
  const { scripts, scriptRevision, ...result } = parseResponseBundle(response, streaming);
  let revision: number | undefined;
  if (scriptRevision !== undefined) {
    if (!revisions.has(scriptRevision)) revisions.set(scriptRevision, revisions.size);
    revision = revisions.get(scriptRevision);
  }
  const { program, metadata, complete, isBundle, error } = result;
  return JSON.stringify({ program, metadata, scripts: [...scripts], complete, isBundle, error, scriptRevision: revision });
}

function readBoth(raw: string | null, revisions: Map<string, number>) {
  return {
    complete: read(raw, false, revisions),
    cuts: Array.from({ length: (raw?.length ?? 0) + 1 }, (_, cut) =>
      read(raw?.slice(0, cut) ?? null, true, revisions)),
  };
}

export function answerResponseBundles(raw: ResponseInput, other?: ResponseInput) {
  const revisions = new Map<string, number>();
  const first = readBoth(responseText(raw), revisions);
  return other === undefined ? [first] : [first, readBoth(responseText(other), revisions)];
}

if (import.meta.main) {
  const base = new URL("../conformance/", import.meta.url);
  const corpus: {
    cases: Case[];
    revisionPairs: { left: string; right: string }[];
  } = JSON.parse(readFileSync(new URL("response-bundles.json", base), "utf8"));
  const cases = Object.fromEntries(corpus.cases.map(({ name, raw, units }) => [
    name, readBoth(responseText(units ?? raw), new Map()),
  ]));
  const byName = new Map(corpus.cases.map(({ name, raw, units }) => [name, responseText(units ?? raw)]));
  const revisionPairs = corpus.revisionPairs.map(({ left, right }) => {
    if (!byName.has(left) || !byName.has(right)) throw new Error("unknown revision pair case");
    return parseResponseBundle(byName.get(left) ?? null, false).scriptRevision
      === parseResponseBundle(byName.get(right) ?? null, false).scriptRevision;
  });
  writeFileSync(new URL("golden/response-bundles.json", base), JSON.stringify({ cases, revisionPairs }, null, 2) + "\n");
  console.log(`${corpus.cases.length} response bundle cases, ${revisionPairs.length} revision pairs`);
}
