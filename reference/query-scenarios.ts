// Run from reference/: bun --no-install query-scenarios.ts
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { RefreshClock } from './refresh-clock';
import type { EvaluationContext } from './lang-core/src/runtime/evaluator';

process.env.OPENUI_TELEMETRY_DISABLED = '1';
process.env.DO_NOT_TRACK = '1';
const { compileSchema, parse } = await import('./lang-core/src/parser/parser');
const { createQueryManager } = await import('./lang-core/src/runtime/queryManager');
const { createStore } = await import('./lang-core/src/runtime/store');
const { ToolNotFoundError } = await import('./lang-core/src/runtime/toolProvider');
const { McpToolError } = await import('./lang-core/src/runtime/mcp');
const { unwrapFieldValue } = await import('./renderer-actions');

export type Outcome =
  | { kind: 'result'; value?: unknown; hostNumber?: 'NaN' | '-0' }
  | { kind: 'tool-not-found'; availableTools: string[] }
  | { kind: 'mcp-error' | 'tool-error'; message: string };
export type Dispatch = { step: number; toolName: string; args: unknown; occurrence?: number };
export type Step =
  | { kind: 'register' }
  | { kind: 'fire'; name: string; args?: unknown }
  | { kind: 'advance'; now: number }
  | { kind: 'dispose' | 'activate' }
  | { kind: 'evaluate'; blocked?: boolean }
  | { kind: 'complete'; call: number; outcome: Outcome }
  | { kind: 'complete'; dispatch: Dispatch; outcome: Outcome }
  | { kind: 'set'; name: string; value?: unknown }
  | { kind: 'extra'; toolName: string; values?: Record<string, unknown> }
  | { kind: 'replace'; program: string; repeatLastReader?: boolean }
  | { kind: 'invalidate'; ids?: string[] };
export type Scenario = { name: string; program: string; library?: Parameters<typeof compileSchema>[0]; steps: Step[] };
type Call = Dispatch & { finish(outcome: Outcome): Promise<void> };

type Script = (calls: readonly Dispatch[]) => Iterable<Step>;

export async function replay(scenario: Scenario, script?: Script, factory = createQueryManager, beginStep?: (index:number) => void, clock?: RefreshClock): Promise<string[]> {
  const library = compileSchema(scenario.library ?? { $defs: {} });
  let program = parse(scenario.program, library);
  const store = createStore();
  store.initialize(program.stateDeclarations ?? {}, {});
  const calls: Call[] = [];
  // The embedder's extra cache-key values, looked up by the tool name as a
  // property key, as an object of values per tool would be.
  const extras = new Map<string, Record<string, unknown>>();
  let stepIndex = 0;
  const manager = factory({
    callTool(name, args) {
      // toQueryNode uses a TypeScript assertion, so the runtime provider can
      // get a non-string name. Observe the JS string used by the Rust seam.
      if (calls.length >= 512) throw Error("query replay dispatch bound");
      const toolName = String(name);
      const promise = new Promise((resolve, reject) => {
        calls.push({ step: stepIndex, toolName, args, async finish(outcome) {
          switch (outcome.kind) {
            // Upstream can append status ids to a result array. Give the
            // provider its own value on each replay of the same script.
            case 'result': resolve(outcome.hostNumber === 'NaN' ? NaN : outcome.hostNumber === '-0' ? -0 : structuredClone(outcome.value)); break;
            case 'tool-not-found': reject(new ToolNotFoundError(toolName, outcome.availableTools)); break;
            case 'mcp-error': reject(new McpToolError(outcome.message)); break;
            case 'tool-error': reject(new Error(outcome.message)); break;
          }
          // executeFetch queued its continuation on this exact promise before
          // the script could settle it. Awaiting it here puts the next
          // observation after upstream's catch/finally and synchronous readers.
          await promise.catch(() => {});
        } });
      });
      return promise;
    },
  });
  const context: EvaluationContext = {
    getState: name => unwrapFieldValue(store.get(name)),
    resolveRef: name => manager.getMutationResult(name) ?? manager.getResult(name),
  };
  const { registerMutations } = await import('./mutation-registration');
  try {
    let rejected: string | undefined;
    const observations = [];
    for (const step of script ? script(calls) : scenario.steps) {
      beginStep?.(stepIndex);
      if (stepIndex >= 256) throw Error("query replay step bound");
      if (script) scenario.steps.push(step);
      const first = calls.length;
      const before = JSON.stringify(manager.getSnapshot());
      // Evaluate passes the freshly parsed statements and live context.
      // Complete settles a provider promise by zero-based dispatch order.
      // Set, extra and replace change inputs only; evaluate applies them to the manager.
      switch (step.kind) {
        case 'register': registerMutations(false, program, context, manager); break;
        case 'fire': void manager.fireMutation(step.name, step.args).catch(error => { rejected = error.message; }); break;
        case 'advance':
          if (!clock) throw Error('missing scripted clock');
          clock.advance(step.now);
          break;
        case 'dispose': manager.dispose(); break;
        case 'activate': manager.activate(); break;
        case 'evaluate':
          // useOpenUIState.ts:218. The renderer skips evaluation while blocked.
          if (step.blocked) break;
          manager.evaluateStatements(program.queryStatements ?? [], context, { extraDeps: name => extras.get(String(name)) });
          break;
        case 'complete':
          if ('call' in step) await calls[step.call].finish(step.outcome);
          else {
            // A difference in dispatch must never redirect an outcome to the
            // next positional call. Only the side that dispatched here settles.
            const target = step.dispatch;
            const matches = calls.filter(c => c.step === target.step && c.toolName === target.toolName &&
              JSON.stringify(c.args) === JSON.stringify(target.args));
            if (matches.length > 1 && target.occurrence === undefined) throw new Error("ambiguous dispatch address");
            await matches[target.occurrence ?? 0]?.finish(step.outcome);
          }
          break;
        case 'set': store.set(step.name, step.value); break;
        case 'extra':
          if (step.values === undefined) extras.delete(step.toolName);
          else extras.set(step.toolName, step.values);
          break;
        case 'replace':
          program = parse(step.program, library);
          // Raw manager callers may supply repeated statement ids. The parser
          // itself coalesces repeated declarations for the renderer.
          if (step.repeatLastReader) {
            const statements = program.queryStatements ?? [];
            const last = statements.at(-1);
            if (!last?.waitFor?.length) throw Error('expected a reader to repeat');
            program.queryStatements = [...statements, last];
          }
          store.initialize(program.stateDeclarations ?? {}, {});
          break;
        // A run of the named queries, as the renderer's @Run and Retry call
        // it. Omitted ids pass undefined, which runs every query, as [] does.
        case 'invalidate': manager.invalidate(step.ids); break;
      }
      const snapshot = manager.getSnapshot();
      observations.push(JSON.stringify({
        snapshot, ...(rejected === undefined ? {} : {rejected}),
        calls: calls.slice(first).map(({ toolName, args }, offset) => ({ call: first + offset, toolName, args })),
        changed: before !== JSON.stringify(snapshot),
        ...(clock ? {nextDue: clock.nextDue} : {}),
      }));
      rejected = undefined;
      stepIndex++;
    }
    return observations;
  } finally {
    manager.dispose();
    store.dispose();
  }
}

export async function withFakeClock(run: (clock: RefreshClock) => Promise<void>) {
  const { setInterval, clearInterval } = globalThis;
  const logError = console.error;
  const clock = new RefreshClock();
  Object.assign(globalThis, { setInterval: clock.setInterval, clearInterval: clock.clearInterval });
  console.error = () => {}; // Scripted rejections are observed through the snapshot.
  try {
    await run(clock);
  } finally {
    Object.assign(globalThis, { setInterval, clearInterval });
    console.error = logError;
  }
}

if (import.meta.main) await withFakeClock(async () => {
  if (process.argv[2] === '--self-wait') {
    await (await import('./query-self-wait')).selfWaitGoldens(replay);
  } else if (process.argv[2] === '--previous-self-graphs') {
    await (await import('./query-reader-graphs')).runGraphs(replay, true);
  } else if (process.argv[2] === '--reader-fixes') {
    await (await import('./query-reader-fixes')).fixGoldens(replay);

  } else if (process.argv[2] === '--reader-rounds' || process.argv[2] === '--reader-rounds-golden') {
    const { runRoundOrders } = await import('./query-reader-rounds');
    await runRoundOrders(replay,process.argv[2].endsWith('-golden'));
  } else if (process.argv[2] === '--reader-graphs') {
    const { runGraphs } = await import('./query-reader-graphs');
    await runGraphs(replay);
  } else if (process.argv[2] === '--readers' || process.argv[2] === '--random-readers') {
    const { runReaders } = await import('./query-readers');
    await runReaders(replay, process.argv[2] === '--random-readers');
  } else if (process.argv[2] === '--random') {
    const { randomScenario } = await import('./query-random');
    const { addressCompletions, compareLifetimes } = await import('./query-lifetimes');
    let leavingFirst = 0, takingFirst = 0;
    for (let seed = 1; seed <= 6000; seed++) {
      const { scenario, script, keys, waitFor } = randomScenario(seed);
      let reference = await replay(scenario, script);
      // Require one-way handoffs in both declaration orders. A symmetric
      // swap alone cannot distinguish the outgoing and incoming statements.
      for (let i = 1; i < keys.length; i++) {
        const previous = keys[i - 1], next = keys[i];
        const [a, b] = Object.keys(next).filter(id => !waitFor[id]);
        if (b === undefined || previous[a] === next[a] || previous[b] === next[b]) continue;
        const leaves = previous[a] === next[b], takes = previous[b] === next[a];
        if (leaves && !takes) leavingFirst++;
        if (takes && !leaves) takingFirst++;
      }
      if (addressCompletions(scenario, reference, keys, seed, waitFor)) reference = await replay(scenario);
      console.log(JSON.stringify({ scenario, reference, ...compareLifetimes(scenario, reference, keys, waitFor) }));
    }
    if (!leavingFirst || !takingFirst) throw new Error('missing a batch handoff order');
    console.warn(`One-way batch handoffs: leaving statement first ${leavingFirst}, taking statement first ${takingFirst}`);
  } else {
    mkdirSync('../conformance/golden', { recursive: true });
    // Runs (#87) keep their own corpus and golden file.
    for (const corpus of ['query-scenarios', 'query-runs']) {
      const scenarios: Scenario[] = JSON.parse(readFileSync(`../conformance/${corpus}.json`, 'utf8'));
      const output = [];
      for (const scenario of scenarios) output.push([scenario.name, await replay(scenario)]);
      writeFileSync(`../conformance/golden/${corpus}.json`, JSON.stringify(Object.fromEntries(output)) + '\n');
    }
  }
});
