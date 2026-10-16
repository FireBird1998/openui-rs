// Turns the viewer's recordings (conformance/recordings/*.recording.json, the
// format documented on `Recording` in viewer/Sources/ViewerKit/Recording.swift)
// into corpus cases. Run by scripts/import-recordings.sh, which copies new
// recordings in first and regenerates the goldens after.
//
//   bun --no-install import-recordings.ts
//
// A fresh recording becomes a parse case, `recorded/<file>`: the program it
// received, in its library, with a recorded chunking cut where its deltas
// arrived (sizes in code points). A follow-up (one with `patchOf`) becomes a
// merge case, the program it patched with its text as the patch, and a parse
// case of the merged program, `recorded/<file>-merged`. The request, the
// models, the effort and when it was made stay with each case. Recordings
// never hold a key.
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { join } from "path";

process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const { mergeStatements } = await import("./lang-core/src/parser/merge");

const CONFORMANCE = join(import.meta.dir, "..", "conformance");
const RECORDINGS = join(CONFORMANCE, "recordings");
const CASES = join(CONFORMANCE, "cases", "recorded");
const SUFFIX = ".recording.json";

type Recording = {
  format: string;
  version: number;
  createdAt: string;
  request: string;
  model: string;
  servedBy?: string;
  effort: string;
  library: string;
  stopReason?: string;
  chunks: { t: number; text: string }[];
  patchOf?: string;
};

// The libraries a case may name: those of the corpus's manifests.
const libraries = new Set<string>();
for (const file of ["manifest.json", "imported.json", "fuzz.json", "synthetic.json"]) {
  const path = join(CONFORMANCE, file);
  if (existsSync(path)) Object.keys(JSON.parse(readFileSync(path, "utf8")).schemas).forEach((s) => libraries.add(s));
}

const files = existsSync(RECORDINGS) ? readdirSync(RECORDINGS).filter((f) => f.endsWith(SUFFIX)).sort() : [];
rmSync(CASES, { recursive: true, force: true });
mkdirSync(CASES, { recursive: true });
const cases: object[] = [];
const merges: object[] = [];
for (const file of files) {
  const recording: Recording = JSON.parse(readFileSync(join(RECORDINGS, file), "utf8"));
  if (recording.format !== "openui-recording") throw new Error(`${file}: not an OpenUI recording`);
  if (!libraries.has(recording.library)) throw new Error(`${file}: the corpus has no library ${recording.library}`);
  const slug = file.slice(0, -SUFFIX.length).replace(/[^A-Za-z0-9-]+/g, "-");
  const name = `recorded/${slug}`;
  const text = recording.chunks.map((c) => c.text).join("");
  const about = {
    origin: `conformance/recordings/${file}`,
    request: recording.request,
    model: recording.model,
    ...(recording.servedBy ? { servedBy: recording.servedBy } : {}),
    effort: recording.effort,
    createdAt: recording.createdAt,
    ...(recording.stopReason ? { stopReason: recording.stopReason } : {}),
  };
  if (recording.patchOf === undefined) {
    writeFileSync(join(CASES, `${slug}.oui`), text);
    cases.push({
      name,
      schema: recording.library,
      file: `cases/recorded/${slug}.oui`,
      source: "recorded",
      recorded: recording.chunks.map((c) => Array.from(c.text).length),
      ...about,
    });
  } else {
    merges.push({ name, existing: recording.patchOf, patch: text, source: "recorded", ...about });
    writeFileSync(join(CASES, `${slug}-merged.oui`), mergeStatements(recording.patchOf, text));
    cases.push({
      name: `${name}-merged`,
      schema: recording.library,
      file: `cases/recorded/${slug}-merged.oui`,
      source: "recorded",
      ...about,
    });
  }
}
writeFileSync(join(CONFORMANCE, "recorded.json"), JSON.stringify({ schemas: {}, cases }, null, 1) + "\n");
writeFileSync(join(CONFORMANCE, "merge-recorded.json"), JSON.stringify({ cases: merges }, null, 1) + "\n");
console.log(`Imported ${files.length} recordings: ${cases.length} parse cases and ${merges.length} merge cases`);
