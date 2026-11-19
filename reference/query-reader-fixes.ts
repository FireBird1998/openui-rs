import {readFileSync, writeFileSync} from 'node:fs';

import type {Replay} from './query-reader-graphs';
import {readerPolicies, type RoundTrace, rows} from './query-reader-policies';
import type {Scenario} from './query-scenarios';

export async function fixGoldens(replay: Replay) {
  const cases: (Scenario&{situation?: string; traceRounds?: boolean})[] =
      JSON.parse(readFileSync('../conformance/query-reader-fixes.json', 'utf8'));
  const policies = await readerPolicies();
  const golden: Record < string, unknown >= {};
  for (const s of cases) {
    const reference = await replay(s);
    async function rounds(enabled: string[]|number, expectedBytes: string[]) {
      let step = 0;
      const traces: (RoundTrace|null)[] = Array(s.steps.length).fill(null);
      const factory = policies(enabled, () => traces[step] = {rounds: [], left: []});
      const bytes = await replay(s, undefined, factory, i => {
        step = i;
      });
      if (bytes.some((v, i) => v !== expectedBytes[i])) throw Error('round tracer changed ' + s.name);
      return traces;
    }
    if (!s.situation) {
      golden[s.name] = s.traceRounds ? {reference, upstreamRounds: await rounds(0, reference)} : {reference};
      continue;
    }
    const index = rows.indexOf(s.situation as typeof rows[number]);
    if (index < 0) throw Error('unknown pinned row ' + s.situation);
    // Isolate the named correction, without any other policy switch.
    const policy = await readerPolicies();
    const isolated = await replay(s, undefined, policy([s.situation]));
    const expected = await replay(s, undefined, policies(rows.length));
    if (isolated.some((v, i) => v !== expected[i])) throw Error('other rules affect ' + s.name);
    golden[s.name] = {
      reference,
      rust: expected,
      upstreamRounds: await rounds(0, reference),
      rustRounds: await rounds([s.situation], expected)
    };
  }
  writeFileSync('../conformance/golden/query-reader-fixes.json', JSON.stringify(golden) + '\n');
}
