# lang-core vs openui-rs, side by side

A Next.js page that streams the same OpenUI Lang program into two copies of
upstream's React renderer and shows what each costs:

- left: upstream's JavaScript parser, `@openuidev/lang-core` 1.1.0;
- right: this repo's Rust core, compiled to WebAssembly (`crates/openui-wasm`).

Both panes are a real `<Renderer>` from `@openuidev/react-lang` 1.1.0 with
react-ui 0.18.1's `openuiLibrary` (the versions at the upstream commit
`reference/UPSTREAM` pins).

## Why it's a fair comparison

Only the parser differs. react-lang's Renderer builds its parser with
`createStreamingParser(library.toJSONSchema(), library.root)` and calls
`set(textSoFar, !isStreaming)` on every update. The app aliases
`@openuidev/lang-core` to `lib/lang-core-shim.ts`, which is the real package
except for `createStreamingParser`: that asks `lib/engines.ts`, which gives
the core (through `lib/wasm-adapter.ts`, lang-core's `push`/`set`/`getResult`
over the wasm `StreamSession`) when the schema came from the right pane's
library, and lang-core otherwise. Each pane's library is wrapped so its
`toJSONSchema()` registers the object it returns. The "parser calls: JS n /
Rust m" counter shows both engines really run.

Everything else is shared: the same renderer, components, CSS, evaluator,
store and query manager. One source feeds both panes the same text at the same
moment, pane after pane within one task. The adapter's time includes
`JSON.parse` of the core's result, since a renderer needs objects.

## Run it

From the repo root (needs the `wasm32-unknown-unknown` target and the
`wasm-bindgen` CLI at the version `Cargo.lock` names; the script says how to
install it), then in this folder (Node 22.18 or later):

```sh
scripts/wasm.sh                 # builds target/wasm-pkg/web
cd examples/next-compare
npm ci
npm run compare                 # headless: both engines agree, with timings
npm run build && npm start      # http://localhost:3000
```

`npm run dev` works too, but use the production build for numbers: dev
React is several times slower, and Strict Mode calls each parser twice per
update. `npm run build` builds with `--profile`, so React's `<Profiler>`
still reports commit times.

`predev`, `prebuild` and `precompare` copy the wasm build into `wasm/` (and
stop, telling you to run `scripts/wasm.sh`, if it isn't there). They also
write `lib/programs.generated.json` and `lib/system-prompt.generated.json`.
All of these are build output and git-ignored.

To keep the packages' telemetry off, set `NEXT_TELEMETRY_DISABLED=1` and
`OPENUI_TELEMETRY_DISABLED=1` in your shell. lang-core's runtime telemetry is
opt-in already; its install-time telemetry and Next's are not.

## Using the page

**Replay** plays a bundled program:

- `bench/*`: the seven benchmark samples from `conformance/imported/cases/bench/`;
- `react-ui/*`: the six examples react-ui puts in its own system prompt;
- `docs/*`: two upstream docs programs with `$state`, `Query` and `Mutation`.
  `lib/tools.ts` answers their tools in memory.

Choose the chunk size (1, 4, 16 or 64 characters) and the rate (chunks per
second, or as fast as possible), then press Play. Reset stops the run and
clears both panes.

**Update pacing** chooses how each pane passes text to its Renderer:

- *every chunk*: one synchronous render per chunk.
- *once per animation frame*: the latest text, at most once per frame. With
  fast streams the parser then sees fewer, bigger steps.

**Run panes one at a time** plays the same stream into the left pane, then
into the right. Use it for long-task figures that belong to one engine.

**Live** sends your prompt to a model and streams the answer into both panes
as it arrives from the network. When the answer is done, it is added to
Replay's list, so you can play it again one pane at a time or with other
chunk sizes.

## Reading the numbers

Under each pane, and per run in the table:

- **Parser**: time inside that engine's `set`/`push`/`getResult` per update.
  The table shows count, mean, p95, max and total.
- **Commits**: React `<Profiler>` `actualDuration` for that pane's Renderer:
  count, mean, p95 and total. This is rendering work, which is the same code
  in both panes. Differences here come from what the parser returned, or
  from noise.
- **Wall**: from the first chunk to that pane's last commit. At a fixed rate
  this is mostly the stream's own length.
- **Long tasks**: main-thread tasks over 50 ms, as count and total ms. Chrome
  only. They are page-wide, so a run with both panes gives one figure for
  both. Tick "one at a time" to get a figure per engine.
- **Same result**: whether the two Renderers' last ParseResults are
  JSON-equal.

The page is cross-origin isolated (COOP/COEP headers in `next.config.ts`).
This keeps `performance.now()` fine enough for parser calls that take
hundredths of a millisecond.

To approximate a phone, turn on CPU throttling (4× or 6×) in the Performance
panel of Chrome DevTools, then replay. "Copy results as JSON" and "Copy as
Markdown" put the table on the clipboard.

`npm run compare` runs without a browser. It drives both engines through the
same shim, the way the Renderer does, over every bundled program at every
chunk size. It checks that their results are JSON-equal at every update, lists
any key that only one engine's result objects have, and prints the parser
times (Node, so V8 without a page around it).

## Live setup

Copy `.env.local.example` to `.env.local` (git-ignored) and fill in an
OpenAI-compatible chat-completions endpoint:

```sh
LLM_BASE_URL=https://api.openai.com/v1   # the part before /chat/completions
LLM_API_KEY=...
LLM_MODEL=...
```

Restart the server. Only the server reads these values (`app/api/live/route.ts`).
The page is only told whether they are set. The system prompt is
`openuiLibrary.prompt(openuiPromptOptions)`, react-ui's own. When the values
are unset, the Live tab says so and Replay still works.

## Known limits

- Six of the seven bench samples were written for an earlier react-ui
  library (`conformance/imported/schemas/bench.json`). Against 0.18.1 they
  have validation errors: Input/Select `rules` as arrays, Form `buttons` as an
  array, PieChart slices, Tag icons, Table's second argument. The parser
  drops those components or arguments, so those programs render only in part.
  Both engines drop exactly the same things. `chart-with-data` and the
  `react-ui/*` programs render cleanly.
- Long tasks can't be split between panes when both run together, so use
  "one at a time". The panes run in one page, so garbage collection caused by
  one pane can land in the other pane's timings.
- "Once per animation frame" needs the tab to be visible. Browsers throttle
  `requestAnimationFrame` in hidden tabs and panes.
- The core cuts a stream that passes its size or time limits (see
  `ParseResult::size_limited` in the crate). lang-core has no such limits, so
  results differ only for programs that hit them.
