// Clock-free mutation commands run against the vendored manager.
import { readFileSync, writeFileSync } from 'node:fs';
import { replay, withFakeClock, type Outcome, type Scenario, type Step } from './query-scenarios';

async function randomReplay(seed: number) {
  const name = seed % 16 === 0 ? '__openui_loading' : 'm';
  const program = `q=Query("read")\n${name}=Mutation("write")\nn=Mutation("other")`;
  const scenario: Scenario = { name: 'mutation-seed-' + seed, program, steps: [] };
  let state = seed;
  function pick(count: number) {
    state = (Math.imul(1664525, state) + 1013904223) >>> 0;
    return (state >>> 8) % count;
  }
  function* script(calls: readonly unknown[]): Generator<Step> {
    const pending = new Set<number>();
    yield { kind: 'evaluate' };
    pending.add(0);
    yield { kind: 'register' };
    for (let i = 0; i < 32; i++) {
      const command = pick(6);
      if (command === 0) {
        const mutation = pick(3) ? `${name}=Mutation("w${pick(3)}")\n` : '';
        yield { kind: 'replace', program: `q=Query("read")\n${mutation}n=Mutation("other")` };
        yield { kind: 'register' };
      } else if (pending.size && command <= 2) {
        const call = [...pending][pick(pending.size)];
        pending.delete(call);
        const outcomes: Outcome[] = [
          { kind: 'result', value: i }, { kind: 'mcp-error', message: '' },
          { kind: 'tool-error', message: 'offline' }, { kind: 'tool-not-found', availableTools: ['x'] },
        ];
        yield { kind: 'complete', call, outcome: outcomes[pick(outcomes.length)] };
      } else {
        const first = calls.length;
        yield { kind: 'fire', name: pick(2) ? name : 'n', args: { i } };
        if (calls.length > first) pending.add(first);
      }
    }
    for (const call of pending) yield { kind: 'complete', call, outcome: { kind: 'result', value: null } };
    if (seed % 3 === 0) {
      // Finish the original batch first. The new source has one remembered
      // run and the reader samples the live mutation when the source changes.
      const program = 'm=Mutation("save")\nsrc=Query("source")\nview=Query("view",{src:src,s:m.status})';
      yield {kind: 'replace', program};
      yield {kind: 'register'};
      const first = calls.length;
      yield {kind: 'evaluate'};
      yield {kind: 'invalidate', ids: ['view', 'src']};
      yield {kind: 'fire', name: 'm', args: {seed}};
      yield {kind: 'complete', call: first, outcome: {kind: 'result', value: seed}};
      yield {kind: 'complete', call: first + 1, outcome: {kind: 'result', value: seed}};
      yield {kind: 'complete', call: first + 2, outcome: {kind: 'result', value: seed + 1}};
      const end = calls.length;
      for (let call = first + 3; call < end; call++)
        yield {kind: 'complete', call, outcome: {kind: 'result', value: null}};
    }
  }
  const reference = await replay(scenario, script);
  return { scenario, reference };
}

// The session pause test pins both outcomes against these manager observations.
async function disposeRows() {
  const { createQueryManager } = await import('./lang-core/src/runtime/queryManager');
  const rows = [];
  for (const success of [true, false]) {
    const { promise, resolve, reject } = Promise.withResolvers<unknown>();
    const manager = createQueryManager({ callTool: () => promise });
    manager.registerMutations([{ statementId: 'm', toolName: 'save' }]);
    const plan = manager.fireMutation('m', {});
    manager.dispose();
    const before = manager.getSnapshot();
    if (success) resolve(3);
    else reject(new Error('late'));
    rows.push(JSON.stringify({
      success, before, resumed: await plan, after: manager.getSnapshot(),
      result: manager.getMutationResult('m'), fireAfterDispose: await manager.fireMutation('m', {}),
    }));
  }
  return rows;
}

await withFakeClock(async () => {
  if (process.argv[2] === '--random') {
    for (let seed = 1; seed <= 512; seed++) console.log(JSON.stringify(await randomReplay(seed)));
  } else {
    const scenarios: Scenario[] = JSON.parse(readFileSync('../conformance/mutation-scenarios.json', 'utf8'));
    const rows = [];
    for (const scenario of scenarios) rows.push(...await replay(scenario));
    writeFileSync('../conformance/golden/mutation-scenarios.jsonl', rows.join('\n') + '\n');
    writeFileSync('../conformance/golden/mutation-dispose.jsonl', (await disposeRows()).join('\n') + '\n');
  }
});
