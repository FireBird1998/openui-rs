// The initial batch and the second batch each have n outstanding receipts.
// Enumerate every permutation of both batches, with one stale receipt after
// the cycle. All loops are over fixed arrays, never a list of newly issued calls.
import {createHash} from 'node:crypto';
import {readFileSync, writeFileSync} from 'node:fs';

import type {Replay} from './query-reader-graphs';
import {readerPolicies, type RoundTrace, rows} from './query-reader-policies';
import type {Step} from './query-scenarios';

function permutations(xs: number[]): number[][] {
  if (!xs.length) return [[]];
  return xs.flatMap(x => permutations(xs.filter(i => i !== x)).map(tail => [x, ...tail]));
}
export async function runRoundOrders(replay: Replay, golden: boolean) {
  const policies = await readerPolicies();
  const summaries = [];
  for (const n of [2, 3, 4])
    for (const reported of [true, false]) {
      const ids = Array.from({length: n}, (_, i) => i);
      const hash = createHash('sha256');
      let cases = 0, caps = 0, completionCaps = 0;
      for (const first of permutations(ids))
        for (const second of permutations(ids)) {
          const queries = (arg: (i: number) => string, name = 'q') =>
              ids.map(i => name + i + '=Query("t' + i + '",{r:' + arg(i) + '},"d")').join('\n');
          const anchors = queries(() => '0', 'hold0') + '\n' + queries(() => '1', 'hold1');
          const cycle = reported ?
              queries(i => 'q' + ((i + 1) % n)) :
              ids.map(i => 'q' + i + '=Query("t' + i + '",{r:0},q' + ((i + 1) % n) + ')').join('\n');
          const addr = (step: number, i: number, r: number) => ({step, toolName: 't' + i, args: {r}});
          const steps: Step[] = [
            {kind: 'evaluate'}, ...first.map(i => ({
                                               kind: 'complete' as const,
                                               dispatch: addr(0, i, 0),
                                               outcome: {kind: 'result' as const, value: i === 0 ? 1 : 0}
                                             }))
          ];
          steps.push(
              {
                kind: 'replace',
                program: queries(() => '1') + '\n' + queries(() => '0', 'hold0') + '\nold=Query("old",{})'
              },
              {kind: 'evaluate'});
          const start = steps.length - 1;
          steps.push({kind: 'replace', program: cycle + '\n' + anchors}, {kind: 'evaluate'});
          for (const i of second)
            steps.push({
              kind: 'complete',
              dispatch: addr(start, i, 1),
              outcome: {kind: 'result', value: i === 0 ? 0 : 1}
            });
          steps.push({
            kind: 'complete',
            dispatch: {step: start, toolName: 'old', args: {}},
            outcome: {kind: 'result', value: 'late'}
          });
          const scenario = {
            name: 'rounds-' + [n, reported, ...first, ...second].join('-'),
            program: queries(() => '0'),
            steps
          };
          const reference = await replay(scenario);
          async function observe(prefix: number) {
            const traces: (RoundTrace|null)[] = Array(steps.length).fill(null);
            let step = 0;
            const factory = policies(prefix, () => traces[step] = {rounds: [], left: []});
            const bytes = await replay(scenario, undefined, factory, i => {
              step = i;
            });
            return {traces, bytes};
          }
          const upstream = await observe(0);
          if (upstream.bytes.some((s, i) => s !== reference[i]))
            throw Error('round instrumentation affected output');
          const rules = await observe(rows.length);
          hash.update(JSON.stringify([reference, upstream.traces]) + '\n');
          for (const [i, t] of upstream.traces.entries())
            if (t?.left.length) {
              caps++;
              if (steps[i].kind === 'complete') completionCaps++;
            }
          if (!golden) {
            const differs = upstream.traces.flatMap(
                (t, i) => JSON.stringify(t) !== JSON.stringify(rules.traces[i]) ? [i] : []);
            const reasons: Record < number, string >= {};
            if (differs.length) {
              let previous = upstream;
              for (let j = 0; j < rows.length; j++) {
                const next = j + 1 === rows.length ? rules : await observe(j + 1);
                for (const i of differs)
                  if (JSON.stringify(previous.traces[i]) === JSON.stringify(upstream.traces[i]) &&
                      JSON.stringify(next.traces[i]) !== JSON.stringify(upstream.traces[i]))
                    reasons[i] = rows[j];
                previous = next;
              }
            }
            console.log(JSON.stringify({
              scenario,
              reference,
              traces: upstream.traces,
              expected: rules.bytes,
              byRule: rules.traces,
              reasons
            }));
          }
          cases++;
        }
      summaries.push({n, reported, cases, caps, completionCaps, sha256: hash.digest('hex')});
    }
  const path = '../conformance/golden/query-reader-rounds.json',
        bytes = JSON.stringify(summaries, null, 2) + '\n';
  if (!golden) {
    const cases = JSON.parse(readFileSync('../conformance/query-reader-fixes.json', 'utf8'));
    const fixes = JSON.parse(readFileSync('../conformance/golden/query-reader-fixes.json', 'utf8'));
    for (const scenario of cases.filter((s: {traceRounds?: boolean}) => s.traceRounds)) {
      const r = fixes[scenario.name];
      console.log(JSON.stringify({
        scenario,
        reference: r.reference,
        expected: r.rust ?? r.reference,
        traces: r.upstreamRounds,
        byRule: r.rustRounds ?? r.upstreamRounds,
        reasons:
            Object.fromEntries(r.upstreamRounds.map((_: unknown, i: number) => [i, scenario.situation]))
      }));
    }
  }
  if (golden)
    writeFileSync(path, bytes);
  else if (readFileSync(path, 'utf8') !== bytes)
    throw Error('round traces differ from the saved upstream golden');
}
