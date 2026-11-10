import type { ParseSink } from "./engines.ts";
import { summarize, type Summary } from "./stats.ts";

/** One pane's measurements for the current run. */
export class PaneRecorder implements ParseSink {
  parses: number[] = [];
  commits: number[] = [];
  firstChunkAt = 0;
  lastCommitAt = 0;

  reset() {
    this.parses = [];
    this.commits = [];
    this.firstChunkAt = 0;
    this.lastCommitAt = 0;
  }

  /** Time inside the parser's set/push/getResult (lib/engines.ts calls this). */
  parse(ms: number) {
    this.parses.push(ms);
  }

  /** A React commit of this pane (its <Profiler>). */
  commit(actualDuration: number) {
    this.commits.push(actualDuration);
    this.lastCommitAt = performance.now();
  }

  snapshot(): PaneStats {
    return {
      parse: summarize(this.parses),
      commit: summarize(this.commits),
      wallMs: this.firstChunkAt && this.lastCommitAt > this.firstChunkAt ? this.lastCommitAt - this.firstChunkAt : 0,
    };
  }
}

export interface PaneStats {
  parse: Summary;
  commit: Summary;
  /** First chunk given to the pane to its last commit. */
  wallMs: number;
}
