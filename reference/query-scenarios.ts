// Run from reference/: bun --no-install query-scenarios.ts
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import type { EvaluationContext } from './lang-core/src/runtime/evaluator';

process.env.OPENUI_TELEMETRY_DISABLED = '1';
process.env.DO_NOT_TRACK = '1';
const { compileSchema, parse } = await import('./lang-core/src/parser/parser');
const { createQueryManager } = await import('./lang-core/src/runtime/queryManager');
const { createStore } = await import('./lang-core/src/runtime/store');
const { ToolNotFoundError } = await import('./lang-core/src/runtime/toolProvider');
const { McpToolError } = await import('./lang-core/src/runtime/mcp');
const { unwrapFieldValue } = await import('./renderer-actions');

type Outcome =
  | { kind: 'result'; value?: unknown }
  | { kind: 'tool-not-found'; availableTools: string[] }
  | { kind: 'mcp-error' | 'tool-error'; message: string };
type Step =
  | { kind: 'evaluate' }
  | { kind: 'complete'; call: number; outcome: Outcome }
  | { kind: 'set'; name: string; value?: unknown }
  | { kind: 'replace'; program: string };
type Scenario = { name: string; program: string; library?: Parameters<typeof compileSchema>[0]; steps: Step[] };
type Call = { toolName: string; args: unknown; finish(outcome: Outcome): Promise<void> };

function makeClock() {
  // Keep intervals behind a fake clock. This slice never advances it.
  const timers = new Map<number, { tick: () => void; interval: number; due: number }>();
  let next = 0;
  const now = 0;
  return {
    setInterval(tick: () => void, interval: number) {
      const id = ++next;
      timers.set(id, { tick, interval, due: now + interval });
      return id;
    },
    clearInterval(id: number) { timers.delete(id); },
  };
}

async function replay(scenario: Scenario) {
  const library = compileSchema(scenario.library ?? { $defs: {} });
  let program = parse(scenario.program, library);
  const store = createStore();
  store.initialize(program.stateDeclarations ?? {}, {});
  const calls: Call[] = [];
  const manager = createQueryManager({
    callTool(name, args) {
      // toQueryNode uses a TypeScript assertion, so the runtime provider can
      // get a non-string name. Observe the JS string used by the Rust seam.
      const toolName = String(name);
      const promise = new Promise((resolve, reject) => {
        calls.push({ toolName, args, async finish(outcome) {
          switch (outcome.kind) {
            case 'result': resolve(outcome.value); break;
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
    resolveRef: name => manager.getResult(name),
  };
  try {
    const observations = [];
    for (const step of scenario.steps) {
      const first = calls.length;
      const before = JSON.stringify(manager.getSnapshot());
      // Evaluate passes the freshly parsed statements and live context.
      // Complete settles a provider promise by zero-based dispatch order.
      // Set and replace change inputs only; evaluate applies them to the manager.
      switch (step.kind) {
        case 'evaluate': manager.evaluateStatements(program.queryStatements ?? [], context); break;
        case 'complete': await calls[step.call].finish(step.outcome); break;
        case 'set': store.set(step.name, step.value); break;
        case 'replace':
          program = parse(step.program, library);
          store.initialize(program.stateDeclarations ?? {}, {});
          break;
      }
      const snapshot = manager.getSnapshot();
      observations.push(JSON.stringify({
        snapshot,
        calls: calls.slice(first).map(({ toolName, args }, offset) => ({ call: first + offset, toolName, args })),
        changed: before !== JSON.stringify(snapshot),
      }));
    }
    return [scenario.name, observations];
  } finally {
    manager.dispose();
    store.dispose();
  }
}

const scenarios: Scenario[] = JSON.parse(readFileSync('../conformance/query-scenarios.json', 'utf8'));
const clock = makeClock();
const { setInterval, clearInterval } = globalThis;
const logError = console.error;
Object.assign(globalThis, clock);
console.error = () => {}; // Scripted rejections are observed through the snapshot.
try {
  const output = [];
  for (const scenario of scenarios) output.push(await replay(scenario));
  mkdirSync('../conformance/golden', { recursive: true });
  writeFileSync('../conformance/golden/query-scenarios.json', JSON.stringify(Object.fromEntries(output)) + '\n');
} finally {
  Object.assign(globalThis, { setInterval, clearInterval });
  console.error = logError;
}
