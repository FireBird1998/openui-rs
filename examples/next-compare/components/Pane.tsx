"use client";

import { Renderer, type Library } from "@openuidev/react-lang";
import { Profiler, memo, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState, type Ref } from "react";
import { flushSync } from "react-dom";
import { tagLibrary, type Engine } from "../lib/engines.ts";
import { PaneRecorder } from "../lib/recorder.ts";
import { makeTools } from "../lib/tools.ts";
import { Stats } from "./Stats.tsx";

export type Pacing = "chunk" | "frame";

/** What the stream source drives: the same calls, in the same order, for both panes. */
export interface PaneHandle {
  readonly engine: Engine;
  readonly recorder: PaneRecorder;
  /** A new run: a fresh Renderer (so a fresh parser and store) with no text. */
  begin(): void;
  /** The text so far. */
  feed(text: string): void;
  /** The stream has ended with `text`; renders it with isStreaming false. */
  end(text: string): void;
  /** JSON of the last ParseResult the Renderer reported. */
  lastResult(): string | null;
}

export function Pane(props: {
  engine: Engine;
  title: string;
  library: Library;
  pacing: Pacing;
  running: boolean;
  ref: Ref<PaneHandle>;
}) {
  const { engine, title, pacing, running } = props;
  const recorder = useMemo(() => new PaneRecorder(), []);
  // The pane's library: its toJSONSchema() tells the shim which engine to use
  // and where the timings go. Made once, so the Renderer's parser is made once per run.
  const library = useMemo(() => tagLibrary(props.library, engine, recorder), [props.library, engine, recorder]);
  const [run, setRun] = useState(0);
  const [response, setResponse] = useState<string | null>(null);
  const [streaming, setStreaming] = useState(false);
  const tools = useMemo(() => makeTools(), [run]); // eslint-disable-line react-hooks/exhaustive-deps
  const lastResult = useRef<unknown>(null);
  const pending = useRef<string | null>(null);
  const frame = useRef(0);
  const pacingRef = useRef(pacing);
  pacingRef.current = pacing;

  useImperativeHandle(
    props.ref,
    (): PaneHandle => ({
      engine,
      recorder,
      begin() {
        cancelAnimationFrame(frame.current);
        frame.current = 0;
        pending.current = null;
        lastResult.current = null;
        recorder.reset();
        flushSync(() => {
          setRun((n) => n + 1);
          setResponse(null);
          setStreaming(true);
        });
        // The remount's own commit isn't part of the run.
        recorder.reset();
      },
      feed(text) {
        if (!recorder.firstChunkAt) recorder.firstChunkAt = performance.now();
        if (pacingRef.current === "chunk") {
          flushSync(() => setResponse(text));
          return;
        }
        // Once per animation frame: keep the latest text, render it at the next frame.
        pending.current = text;
        if (!frame.current) {
          frame.current = requestAnimationFrame(() => {
            frame.current = 0;
            const latest = pending.current;
            pending.current = null;
            if (latest !== null) setResponse(latest);
          });
        }
      },
      end(text) {
        if (!recorder.firstChunkAt) recorder.firstChunkAt = performance.now();
        cancelAnimationFrame(frame.current);
        frame.current = 0;
        pending.current = null;
        flushSync(() => {
          setResponse(text);
          setStreaming(false);
        });
      },
      lastResult: () => (lastResult.current ? JSON.stringify(lastResult.current) : null),
    }),
    [engine, recorder],
  );

  useEffect(() => () => cancelAnimationFrame(frame.current), []);
  const onParseResult = useCallback((result: unknown) => {
    lastResult.current = result;
  }, []);

  return (
    <section className={`pane pane-${engine}`}>
      <h2>{title}</h2>
      <div className="pane-body">
        <Rendered
          engine={engine}
          run={run}
          response={response}
          streaming={streaming}
          library={library}
          tools={tools}
          recorder={recorder}
          onParseResult={onParseResult}
        />
      </div>
      <Stats recorder={recorder} live={running} />
    </section>
  );
}

// The Renderer under a Profiler, re-rendered only when what it shows changes,
// so the pane's own updates (its stats) never count as the Renderer's commits.
const Rendered = memo(function Rendered(props: {
  engine: Engine;
  run: number;
  response: string | null;
  streaming: boolean;
  library: Library;
  tools: ReturnType<typeof makeTools>;
  recorder: PaneRecorder;
  onParseResult: (result: unknown) => void;
}) {
  const { recorder } = props;
  return (
    <Profiler id={props.engine} onRender={(_id, _phase, actualDuration) => recorder.commit(actualDuration)}>
      <Renderer
        key={props.run}
        response={props.response}
        library={props.library}
        isStreaming={props.streaming}
        toolProvider={props.tools}
        onParseResult={props.onParseResult}
      />
    </Profiler>
  );
});
