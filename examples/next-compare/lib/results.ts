import type { LongTasks } from "./longtasks.ts";
import type { PaneStats } from "./recorder.ts";

export interface RunResult {
  at: string;
  program: string;
  chunkSize: number | "network";
  /** Chunks per second; 0 is as fast as possible. */
  rate: number | "network";
  pacing: "every chunk" | "once per frame";
  order: "together" | "one at a time";
  js: PaneStats;
  rust: PaneStats;
  /** Together: one figure for the page. One at a time: one per engine's turn. */
  longTasks: { together?: LongTasks | null; js?: LongTasks | null; rust?: LongTasks | null };
  /** The two Renderers' last ParseResults, as JSON, were equal. */
  sameFinalResult: boolean;
}

const f = (n: number) => n.toFixed(2);
const lt = (l: LongTasks | null | undefined) => (l ? `${l.count} / ${f(l.totalMs)}` : "n/a");

export function toMarkdown(results: RunResult[]): string {
  const lines = [
    "| Program | Chunk | Rate | Pacing | Order | Engine | Parser calls | Parser mean / p95 / max / total (ms) | Commits | Commit mean / p95 / total (ms) | Wall (ms) | Long tasks (n / ms) | Same final result |",
    "|---|---:|---:|---|---|---|---:|---|---:|---|---:|---|---|",
  ];
  for (const r of results) {
    for (const engine of ["js", "rust"] as const) {
      const s = r[engine];
      const longTasks = r.order === "together" ? `${lt(r.longTasks.together)} (page)` : lt(r.longTasks[engine]);
      lines.push(
        `| ${r.program} | ${r.chunkSize} | ${r.rate === 0 ? "max" : r.rate} | ${r.pacing} | ${r.order} | ${engine === "js" ? "lang-core" : "openui-rs"} | ` +
          `${s.parse.count} | ${f(s.parse.mean)} / ${f(s.parse.p95)} / ${f(s.parse.max)} / ${f(s.parse.total)} | ` +
          `${s.commit.count} | ${f(s.commit.mean)} / ${f(s.commit.p95)} / ${f(s.commit.total)} | ${f(s.wallMs)} | ${longTasks} | ${r.sameFinalResult ? "yes" : "NO"} |`,
      );
    }
  }
  return lines.join("\n");
}
