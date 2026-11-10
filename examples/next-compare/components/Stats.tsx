"use client";

import { useEffect, useState } from "react";
import type { PaneRecorder } from "../lib/recorder.ts";

const ms = (n: number) => n.toFixed(n < 10 ? 2 : 1);

/** A pane's numbers so far, refreshed four times a second while a run is going. */
export function Stats({ recorder, live }: { recorder: PaneRecorder; live: boolean }) {
  const [, tick] = useState(0);
  useEffect(() => {
    tick((n) => n + 1);
    if (!live) return;
    const timer = setInterval(() => tick((n) => n + 1), 250);
    return () => clearInterval(timer);
  }, [live]);
  const { parse, commit, wallMs } = recorder.snapshot();
  return (
    <dl className="stats">
      <dt>Parser</dt>
      <dd>
        {parse.count} calls · mean {ms(parse.mean)} · p95 {ms(parse.p95)} · max {ms(parse.max)} · total {ms(parse.total)} ms
      </dd>
      <dt>React commits</dt>
      <dd>
        {commit.count} · mean {ms(commit.mean)} · p95 {ms(commit.p95)} · total {ms(commit.total)} ms
      </dd>
      <dt>Wall</dt>
      <dd>{wallMs ? `${ms(wallMs)} ms, first chunk to last commit` : "–"}</dd>
    </dl>
  );
}
