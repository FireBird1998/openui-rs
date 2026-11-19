// From reference/: bun --no-install query-refresh.ts [--random]
import { readFileSync, writeFileSync } from 'node:fs';
import { replay, withFakeClock, type Scenario, type Step } from './query-scenarios';

async function observe(scenario: Scenario, script?: Parameters<typeof replay>[1], minimumMs?: number) {
  let observations: string[] = [];
  await withFakeClock(async clock => { observations = await replay(scenario, script, clock); }, minimumMs);
  return observations;
}

if (process.argv.includes('--random')) {
  // Distinct keys, intervals at/above the default floor. Settle outstanding
  // calls before disposal: every observation must agree with upstream.
  for (let seed = 1; seed <= 512; seed++) {
    let state = seed, now = 0, finished = 0;
    const pick = (n: number) => {
      state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
      return Math.floor((state / 2 ** 32) * n);
    };
    const program = () => ['a','b','c'].map(id =>
      `${id}=Query("${id}",{},null,${pick(5)})`).join('\n');
    const scenario: Scenario = {name: `random-refresh-${seed}`, program: program(), steps: []};
    const reference = await observe(scenario, function*(calls): Generator<Step> {
      const complete = (): Step => ({kind:'complete', call: finished++,
        outcome: pick(4) ? {kind:'result',value:pick(10)} : {kind:'tool-error',message:'offline'}});
      yield {kind:'evaluate'};
      for (let i = 0; i < 40; i++) {
        switch (pick(8)) {
          case 0: yield {kind:'replace',program:program()}; yield {kind:'evaluate'}; break;
          case 1: yield {kind:'invalidate',ids:[['a','b','c'][pick(3)]]}; break;
          case 2:
            while (finished < calls.length) yield complete();
            yield {kind:'dispose'}; yield {kind:'advance',now:now += pick(5000)};
            yield {kind:'activate'}; yield {kind:'evaluate'}; break;
          case 3: if (finished < calls.length) yield complete(); break;
          case 4: yield {kind:'evaluate'}; break;
          default: yield {kind:'advance',now:now += pick(12000)};
        }
      }
    });
    console.log(JSON.stringify({scenario, reference}));
  }
  // Replay each of these scripts twice. One documented change to the
  // platform timer accounts for every differing byte, including dispatches.
  for (let seed = 1; seed <= 128; seed++) {
    const infinity = seed % 2 === 0;
    const minimumMs = 25 + seed * 3;
    const reason = infinity ? 'infinite-refresh-is-disabled' : 'configured-refresh-floor';
    const seconds = infinity ? '"Infinity"' : (seed % 15 + 1) / 1000;
    const scenario = {name: `random-refresh-policy-${seed}`, minimumMs,
      program: `q=Query("read",{},null,${seconds})`,
      steps: [{kind:'evaluate'}, {kind:'complete',call:0,outcome:{kind:'result',value:seed}}] as Step[]};
    let now = 0;
    for (let i = 0; i < 12; i++) {
      now += (seed * (i+1)) % 31 + 1;
      scenario.steps.push({kind:'advance',now});
      if (i % 3 === 0) scenario.steps.push({kind:'evaluate'});
      if (i === 5) scenario.steps.push({kind:'invalidate'});
    }
    const reference = await observe(scenario);
    const policy = await observe(scenario, undefined, minimumMs);
    console.log(JSON.stringify({scenario, reference, policy, reason}));
  }
} else {
  const scenarios: Scenario[] = JSON.parse(readFileSync('../conformance/query-refresh.json', 'utf8'));
  const rows = [];
  for (const scenario of scenarios) rows.push([scenario.name, await observe(scenario)]);
  writeFileSync('../conformance/golden/query-refresh.json', JSON.stringify(Object.fromEntries(rows)) + '\n');
}
