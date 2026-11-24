// From the clone: bun --no-install reference/refused-refresh-resume.ts [fixture name]
import { readFileSync } from 'node:fs';
import { withFakeClock } from './query-scenarios';
import { unwrapFieldValue } from './renderer-actions';

const { compileSchema, parse } = await import('./lang-core/src/parser/parser');
const { createQueryManager } = await import('./lang-core/src/runtime/queryManager');
const { createStore } = await import('./lang-core/src/runtime/store');
const { evaluateRoot } = await import('./lang-core/src/runtime/evaluate-tree');
const schema = { $defs: { Box: { properties: { value: {}, action: {} } } } };
const name = process.argv[2] ?? 'refused-refresh-resume';
const fixture = JSON.parse(readFileSync(new URL(`../conformance/${name}.json`, import.meta.url), 'utf8'));
const program = parse(fixture.program, compileSchema(schema));

await withFakeClock(async clock => {
  const store = createStore();
  store.initialize(program.stateDeclarations ?? {}, {});
  let calls: { toolName: string; args: unknown }[] = [];
  let stepIndex = 0;
  const pending = new Map<string, (step: { value?: unknown; error?: string }) => Promise<void>>();
  const manager = createQueryManager({
    callTool(toolName, args) {
      calls.push({ toolName, args });
      const promise = new Promise((resolve, reject) => {
        pending.set(`${stepIndex}:${toolName}`, async step => {
          if (step.error !== undefined) reject(new Error(step.error));
          else resolve(step.value);
          await promise.catch(() => {});
        });
      });
      return promise;
    },
  });
  const ctx = { getState: name => unwrapFieldValue(store.get(name)), resolveRef: manager.getResult };
  const register = () => manager.evaluateStatements(program.queryStatements ?? [], ctx);

  try {
    for (const [index, step] of fixture.steps.entries()) {
      stepIndex = index;
      calls = [];
      const before = JSON.stringify(manager.getSnapshot());
      let registrationError = null;
      let clockRejected: boolean | undefined;
      try {
        switch (step.kind) {
          case 'register': register(); break;
          case 'pause': manager.dispose(); break;
          case 'set':
            store.set(step.name ?? '$detail', step.bytes === undefined ? step.value : 'x'.repeat(step.bytes));
            break;
          case 'advance': clock.advance(step.now); break;
          case 'resume':
            clock.advance(step.now);
            manager.activate();
            if (!step.streaming) register();
            break;
          case 'settle':
            clock.advance(step.now);
            register();
            break;
          case 'backwards':
            try {
              clock.advance(step.now);
              clockRejected = false;
            } catch (error) {
              if (!(error instanceof Error) || error.message !== 'invalid scripted time') throw error;
              clockRejected = true;
            }
            break;
          case 'finish': {
            const key = `${step.dispatch}:${step.tool ?? 'poll'}`;
            const finish = pending.get(key);
            if (finish) {
              pending.delete(key);
              await finish(step);
            }
            break;
          }
          default: throw Error('unknown replay step');
        }
      } catch (error) {
        if (step.kind !== 'resume' || !(error instanceof TypeError)) throw error;
        registrationError = String(error);
      }
      const snapshot = manager.getSnapshot();
      const errors = [];
      const root = evaluateRoot(program, { ctx, library: { components: {} } as any, store, errors });
      console.log(JSON.stringify({
        calls, nextDue: clock.nextDue, snapshot, registrationError,
        changed: before !== JSON.stringify(snapshot), value: root.props.value, errors, clockRejected,
      }));
    }
  } finally {
    manager.dispose();
    store.dispose();
  }
});
