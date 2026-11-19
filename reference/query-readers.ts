// Drive the real manager with acyclic graphs, independent roots, fan-in,
// arbitrary completion order, refetches and changes between evaluate and finish.
import { readFileSync, writeFileSync } from 'node:fs';
import type { createQueryManager } from './lang-core/src/runtime/queryManager';
import type { Outcome, Scenario, Step } from './query-scenarios';

type Script = (calls: readonly { toolName: string }[]) => Iterable<Step>;
type Replay = (scenario: Scenario, script?: Script, factory?: typeof createQueryManager) => Promise<string[]>;

export async function runReaders(replay: Replay, random: boolean) {
  if (!random) {
    const cases: Scenario[] = JSON.parse(readFileSync('../conformance/query-readers.json', 'utf8'));
    const rows = [];
    for (const scenario of cases) rows.push([scenario.name, await replay(scenario)]);
    writeFileSync('../conformance/golden/query-readers.json', JSON.stringify(Object.fromEntries(rows)) + '\n');
    return;
  }
  const { withEntryLoading } = await import('./query-reader-loading');
  const waiting = await withEntryLoading(false);
  const entryLoading = await withEntryLoading(true);
  for (let seed = 1; seed <= 512; seed++) {
    let state = seed;
    const pick = (n: number) => {
      state ^= state << 13; state ^= state >>> 17; state ^= state << 5;
      return (state >>> 0) % n;
    };
    const count = 3 + pick(6);
    const lines = [];
    for (let i = 0; i < count; i++) {
      const inputs = i === 0 ? ['n:$n'] : ['f:$f'];
      if (i && pick(4) !== 0) inputs.push('a:q' + pick(i));
      if (i > 1 && pick(3) === 0) inputs.push('b:q' + pick(i));
      lines.push('q' + i + '=Query("t' + i + '",{' + inputs.join(',') + '},"wait")');
    }
    if (pick(2)) lines.reverse();
    const scenario: Scenario = {name: 'reader-seed-' + seed, program: ['$n=0', '$f=0', ...lines].join('\n'), steps: []};
    function* script(calls: readonly {toolName: string}[]): Generator<Step> {
      yield {kind: 'evaluate'};
      let first = 0;
      const pending: number[] = [];
      for (let round = 0; round < 3; round++) {
        if (round) {
          yield {kind: 'set', name: '$n', value: round};
          yield {kind: 'evaluate'};
        }
        while (first < calls.length || pending.length) {
          while (first < calls.length) pending.push(first++);
          const [call] = pending.splice(pick(pending.length), 1);
          if (!pick(3)) yield {kind: 'set', name: '$f', value: first};
          const outcome: Outcome = !pick(8) ? {kind: 'mcp-error', message: 'failed'} :
            {kind: 'result', value: pick(4) ? [seed, call] : null};
          yield {kind: 'complete', call, outcome};
        }
      }
    }
    const reference = await replay(scenario, script);
    const byEntry = await replay(scenario, undefined, entryLoading);
    const byWaiting = reference.some((row, i) => row !== byEntry[i]) ? await replay(scenario, undefined, waiting) : reference;
    const reasons = Object.fromEntries(byEntry.flatMap((row, i) => row === reference[i] ? [] :
      [[i, row === byWaiting[i] ? 'a-waiting-reader-leaves-its-own-pending-key' : 'return-to-finished-key']]));
    console.log(JSON.stringify({scenario, reference, byEntry, reasons}));
  }
}
