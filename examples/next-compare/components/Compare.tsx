"use client";

import { openuiLibrary } from "@openuidev/react-ui/genui-lib";
import { useEffect, useRef, useState } from "react";
import { CHUNK_SIZES, chunk } from "../lib/chunks.ts";
import { loadWasm, parserCalls } from "../lib/engines.ts";
import { watchLongTasks } from "../lib/longtasks.ts";
import bundled from "../lib/programs.generated.json";
import { toMarkdown, type RunResult } from "../lib/results.ts";
import { Pane, type Pacing, type PaneHandle } from "./Pane.tsx";

const RATES = [10, 30, 60, 120, 0] as const; // chunks per second; 0 is as fast as possible

type Program = { name: string; text: string };

// Waits: until a time (Replay's rate), for the browser (as fast as possible), for frames (commits to land).
const until = (t: number) => new Promise((r) => setTimeout(r, Math.max(0, t - performance.now())));
const channel = typeof MessageChannel === "undefined" ? null : new MessageChannel();
const yieldToBrowser = () =>
  new Promise<void>((resolve) => {
    if (!channel) return void setTimeout(resolve, 0);
    channel.port1.onmessage = () => resolve();
    channel.port2.postMessage(null);
  });
const frames = (n: number) =>
  new Promise<void>((resolve) => {
    const step = (left: number) => (left ? requestAnimationFrame(() => step(left - 1)) : resolve());
    step(n);
  });

export function Compare() {
  const [wasm, setWasm] = useState<"loading" | "ready" | string>("loading");
  const [mode, setMode] = useState<"replay" | "live">("replay");
  const [programs, setPrograms] = useState<Program[]>(bundled);
  const [programIndex, setProgramIndex] = useState(0);
  const [chunkSize, setChunkSize] = useState<number>(4);
  const [rate, setRate] = useState<number>(60);
  const [pacing, setPacing] = useState<Pacing>("chunk");
  const [oneAtATime, setOneAtATime] = useState(false);
  const [running, setRunning] = useState(false);
  const [results, setResults] = useState<RunResult[]>([]);
  const [live, setLive] = useState<{ configured: boolean } | null>(null);
  const [prompt, setPrompt] = useState("A dashboard for a small coffee shop: today's sales, a weekly bar chart and the top products.");
  const [note, setNote] = useState("");
  const js = useRef<PaneHandle>(null);
  const rust = useRef<PaneHandle>(null);
  const abort = useRef<AbortController | null>(null);

  useEffect(() => {
    loadWasm().then(
      () => setWasm("ready"),
      (e: unknown) => setWasm(e instanceof Error ? e.message : String(e)),
    );
    fetch("/api/live")
      .then((r) => r.json())
      .then(setLive, () => setLive({ configured: false }));
  }, []);

  const panes = () => [js.current!, rust.current!];

  // One stream into `targets`: identical calls, in the same order, at the same times.
  async function replay(targets: PaneHandle[], pieces: string[], signal: AbortSignal) {
    for (const t of targets) t.begin();
    const started = performance.now();
    let text = "";
    for (let i = 0; i < pieces.length; i++) {
      if (signal.aborted) return false;
      text += pieces[i];
      for (const t of targets) t.feed(text);
      await (rate > 0 ? until(started + ((i + 1) * 1000) / rate) : yieldToBrowser());
    }
    for (const t of targets) t.end(text);
    await frames(3);
    return !signal.aborted;
  }

  function record(program: string, extra: Pick<RunResult, "chunkSize" | "rate" | "order" | "longTasks">) {
    const [j, r] = panes();
    const result: RunResult = {
      at: new Date().toISOString(),
      program,
      pacing: pacing === "chunk" ? "every chunk" : "once per frame",
      js: j.recorder.snapshot(),
      rust: r.recorder.snapshot(),
      sameFinalResult: j.lastResult() === r.lastResult(),
      ...extra,
    };
    setResults((all) => [result, ...all]);
  }

  async function play() {
    const program = programs[programIndex];
    const pieces = chunk(program.text, chunkSize);
    const controller = new AbortController();
    abort.current = controller;
    setRunning(true);
    setNote("");
    try {
      const [j, r] = panes();
      if (!oneAtATime) {
        const watch = watchLongTasks();
        const done = await replay([j, r], pieces, controller.signal);
        const longTasks = watch.stop();
        if (done) record(program.name, { chunkSize, rate, order: "together", longTasks: { together: longTasks } });
      } else {
        // The same stream into one pane, then the other; the idle pane keeps its last run's UI.
        const watchJs = watchLongTasks();
        if (!(await replay([j], pieces, controller.signal))) return void watchJs.stop();
        const jsTasks = watchJs.stop();
        const watchRust = watchLongTasks();
        if (!(await replay([r], pieces, controller.signal))) return void watchRust.stop();
        const rustTasks = watchRust.stop();
        record(program.name, { chunkSize, rate, order: "one at a time", longTasks: { js: jsTasks, rust: rustTasks } });
      }
    } finally {
      setRunning(false);
    }
  }

  // Live: whatever the model sends, as the network delivers it, into both panes.
  async function ask() {
    const controller = new AbortController();
    abort.current = controller;
    setRunning(true);
    setNote("");
    const [j, r] = panes();
    try {
      const response = await fetch("/api/live", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ prompt }),
        signal: controller.signal,
      });
      if (!response.ok || !response.body) {
        setNote(`Live request failed: ${response.status} ${await response.text()}`);
        return;
      }
      const watch = watchLongTasks();
      for (const t of [j, r]) t.begin();
      const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();
      let text = "";
      let pieces = 0;
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        if (!value) continue;
        text += value;
        pieces++;
        for (const t of [j, r]) t.feed(text);
      }
      for (const t of [j, r]) t.end(text);
      await frames(3);
      const longTasks = watch.stop();
      const name = `live: ${prompt.slice(0, 40)}${prompt.length > 40 ? "…" : ""}`;
      record(name, { chunkSize: "network", rate: "network", order: "together", longTasks: { together: longTasks } });
      // Keep the response, so Replay can play it again (one pane at a time, other chunkings).
      setPrograms((all) => [...all, { name, text }]);
      setNote(`Received ${pieces} network chunks, ${text.length} characters. The response is now in Replay's list.`);
    } catch (e) {
      if (!controller.signal.aborted) setNote(`Live request failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setRunning(false);
    }
  }

  function reset() {
    abort.current?.abort();
    for (const t of panes()) t.begin();
    setRunning(false);
  }

  async function copy(kind: "json" | "markdown") {
    const text = kind === "json" ? JSON.stringify(results, null, 2) : toMarkdown(results);
    try {
      await navigator.clipboard.writeText(text);
      setNote(`Copied ${results.length} runs as ${kind === "json" ? "JSON" : "Markdown"}.`);
    } catch {
      setNote("The clipboard isn't available here.");
    }
  }

  if (wasm !== "ready") {
    return <p className="loading">{wasm === "loading" ? "Loading the WebAssembly core…" : `The WebAssembly core didn't load: ${wasm}`}</p>;
  }

  return (
    <>
      <div className="controls">
        <div className="tabs" role="tablist">
          <button role="tab" aria-selected={mode === "replay"} onClick={() => setMode("replay")} disabled={running}>
            Replay
          </button>
          <button role="tab" aria-selected={mode === "live"} onClick={() => setMode("live")} disabled={running}>
            Live
          </button>
        </div>

        {mode === "replay" ? (
          <div className="row">
            <label>
              Program
              <select value={programIndex} onChange={(e) => setProgramIndex(Number(e.target.value))} disabled={running}>
                {programs.map((p, i) => (
                  <option key={p.name + i} value={i}>
                    {p.name} ({p.text.length} chars)
                  </option>
                ))}
              </select>
            </label>
            <label>
              Chunk
              <select value={chunkSize} onChange={(e) => setChunkSize(Number(e.target.value))} disabled={running}>
                {CHUNK_SIZES.map((n) => (
                  <option key={n} value={n}>
                    {n} char{n > 1 ? "s" : ""}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Rate
              <select value={rate} onChange={(e) => setRate(Number(e.target.value))} disabled={running}>
                {RATES.map((n) => (
                  <option key={n} value={n}>
                    {n ? `${n} chunks/s` : "as fast as possible"}
                  </option>
                ))}
              </select>
            </label>
            <label className="check">
              <input type="checkbox" checked={oneAtATime} onChange={(e) => setOneAtATime(e.target.checked)} disabled={running} />
              Run panes one at a time
            </label>
            <button className="primary" onClick={play} disabled={running}>
              Play
            </button>
            <button onClick={reset}>Reset</button>
          </div>
        ) : (
          <div className="row">
            {live === null ? (
              <span>Checking the server's configuration…</span>
            ) : !live.configured ? (
              <span className="warn">
                Live is off: set LLM_BASE_URL, LLM_API_KEY and LLM_MODEL in .env.local and restart the server (see the README). Replay
                works without them.
              </span>
            ) : (
              <>
                <textarea value={prompt} onChange={(e) => setPrompt(e.target.value)} rows={2} disabled={running} />
                <button className="primary" onClick={ask} disabled={running || !prompt.trim()}>
                  Send
                </button>
                <button onClick={reset}>Stop</button>
              </>
            )}
          </div>
        )}

        <div className="row">
          <label>
            Update pacing
            <select value={pacing} onChange={(e) => setPacing(e.target.value as Pacing)} disabled={running}>
              <option value="chunk">every chunk</option>
              <option value="frame">once per animation frame</option>
            </select>
          </label>
          <ParserCalls running={running} />
          {note && <span className="note">{note}</span>}
        </div>
      </div>

      <div className="panes">
        <Pane ref={js} engine="js" title="lang-core (JavaScript)" library={openuiLibrary} pacing={pacing} running={running} />
        <Pane ref={rust} engine="rust" title="openui-rs (Rust, WebAssembly)" library={openuiLibrary} pacing={pacing} running={running} />
      </div>

      <section className="summary">
        <div className="row">
          <h2>Runs</h2>
          <button onClick={() => copy("json")} disabled={!results.length}>
            Copy results as JSON
          </button>
          <button onClick={() => copy("markdown")} disabled={!results.length}>
            Copy as Markdown
          </button>
          <button onClick={() => setResults([])} disabled={!results.length}>
            Clear
          </button>
        </div>
        <p className="hint">
          Times are milliseconds. Parser: time inside the engine per update (for openui-rs it includes JSON.parse of the result).
          Commits: React Profiler actualDuration for that pane. Long tasks are page-wide; for clean per-engine figures tick “Run panes one
          at a time”. To stand in for a phone, turn on CPU throttling (4× or 6×) in Chrome DevTools’ Performance panel.
        </p>
        <div className="table">
          <table>
            <thead>
              <tr>
                <th>Program</th>
                <th>Chunk</th>
                <th>Rate</th>
                <th>Pacing</th>
                <th>Order</th>
                <th>Engine</th>
                <th>Parser n</th>
                <th>mean</th>
                <th>p95</th>
                <th>max</th>
                <th>total</th>
                <th>Commits</th>
                <th>mean</th>
                <th>p95</th>
                <th>total</th>
                <th>Wall</th>
                <th>Long tasks n / ms</th>
                <th>Same result</th>
              </tr>
            </thead>
            <tbody>
              {results.flatMap((r, i) =>
                (["js", "rust"] as const).map((engine) => {
                  const s = r[engine];
                  const lt = r.order === "together" ? r.longTasks.together : r.longTasks[engine];
                  return (
                    <tr key={`${i}-${engine}`} className={engine === "rust" ? "second" : undefined}>
                      <td>{engine === "js" ? r.program : ""}</td>
                      <td>{engine === "js" ? r.chunkSize : ""}</td>
                      <td>{engine === "js" ? (r.rate === 0 ? "max" : r.rate) : ""}</td>
                      <td>{engine === "js" ? r.pacing : ""}</td>
                      <td>{engine === "js" ? r.order : ""}</td>
                      <td>{engine === "js" ? "lang-core" : "openui-rs"}</td>
                      <td>{s.parse.count}</td>
                      <td>{s.parse.mean.toFixed(3)}</td>
                      <td>{s.parse.p95.toFixed(3)}</td>
                      <td>{s.parse.max.toFixed(2)}</td>
                      <td>{s.parse.total.toFixed(1)}</td>
                      <td>{s.commit.count}</td>
                      <td>{s.commit.mean.toFixed(2)}</td>
                      <td>{s.commit.p95.toFixed(2)}</td>
                      <td>{s.commit.total.toFixed(1)}</td>
                      <td>{s.wallMs.toFixed(0)}</td>
                      <td>
                        {lt ? `${lt.count} / ${lt.totalMs.toFixed(0)}` : "n/a"}
                        {r.order === "together" && engine === "js" ? " (page)" : ""}
                      </td>
                      <td>{engine === "js" ? (r.sameFinalResult ? "yes" : "NO") : ""}</td>
                    </tr>
                  );
                }),
              )}
            </tbody>
          </table>
        </div>
      </section>
    </>
  );
}

/** Calls into each engine since the page loaded: shows the swap really happens. */
function ParserCalls({ running }: { running: boolean }) {
  const [, tick] = useState(0);
  useEffect(() => {
    tick((n) => n + 1);
    if (!running) return;
    const timer = setInterval(() => tick((n) => n + 1), 250);
    return () => clearInterval(timer);
  }, [running]);
  return (
    <span className="calls" title="Calls into each engine's set/push/getResult since the page loaded">
      parser calls: JS {parserCalls.js} / Rust {parserCalls.rust}
    </span>
  );
}
