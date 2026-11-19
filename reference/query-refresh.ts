// From reference/: bun --no-install query-refresh.ts [--random]
import { readFileSync, writeFileSync } from 'node:fs';
import { replay, withFakeClock, type Scenario, type Step } from './query-scenarios';
import { compileSchema, parse } from './lang-core/src/parser/parser';
import { evaluate } from './lang-core/src/runtime/evaluator';

const scenarios: Scenario[] = JSON.parse(readFileSync('../conformance/query-refresh.json', 'utf8'));

async function observe(scenario: Scenario, script?: Parameters<typeof replay>[1]) {
  let observations: string[] = [];
  await withFakeClock(async clock => { observations = await replay(scenario, script, clock); });
  return observations;
}

if (process.argv.includes('--random')) {
  const fixture = scenarios.find(s => s.name === 'javascript-interval-coercion');
  if (!fixture) throw Error('missing interval coercion fixture');
  const { queryStatements } = parse(fixture.program, compileSchema({$defs:{}}));
  const intervals = ['0', '1', '2', '3', '4', '1.25', '"1.5"'];
  for (const query of queryStatements ?? []) {
    if (!query.refreshAST) throw Error('coercion fixture needs a refresh expression');
    const value = evaluate(query.refreshAST, { getState: () => undefined, resolveRef: () => undefined });
    intervals.push(JSON.stringify(value));
  }
  for (let seed = 1; seed <= 512; seed++) {
    let state = seed, now = 0, finished = 0;
    const pick = (n: number) => {
      state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
      return Math.floor((state / 2 ** 32) * n);
    };
    const program = () => ['a','b','c'].map(id =>
      `${id}=Query("${id}",{},null,${intervals[pick(intervals.length)]})`).join('\n');
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
            yield {kind:'dispose'}; yield {kind:'advance',now:now += pick(5000)};
            // Retired successes rebuild upstream's snapshot without warming its cache.
            for (const end = calls.length; finished < end; finished++)
              yield {kind:'complete',call:finished,outcome:{kind:'result',value:'retired'}};
            yield {kind:'activate'}; yield {kind:'evaluate'}; break;
          case 3: if (finished < calls.length) yield complete(); break;
          case 4: yield {kind:'evaluate'}; break;
          default: yield {kind:'advance',now:now += pick(12000)};
        }
      }
    });
    // ADR 0013 publishes the cleared loading lists at dispose. Only those lists
    // and the resulting changed bit may differ before the next activation.
    const overrides: Record<number, string> = {};
    let paused = false, previousSnapshot = '';
    for (const [i, bytes] of reference.entries()) {
      const row = JSON.parse(bytes);
      if (scenario.steps[i].kind === 'activate') paused = false;
      if (scenario.steps[i].kind === 'dispose') paused = true;
      if (paused) {
        row.snapshot.__openui_loading = [];
        row.snapshot.__openui_refetching = [];
        row.changed = JSON.stringify(row.snapshot) !== previousSnapshot;
        const expected = JSON.stringify(row);
        if (expected !== bytes) overrides[i] = expected;
      }
      previousSnapshot = JSON.stringify(row.snapshot);
    }
    console.log(JSON.stringify({scenario, reference, overrides, reason:'dispose-publishes-loading'}));
  }
  // The floor and Infinity are #79 decisions. Test their scheduling against an
  // ordinary upstream program that explicitly requests the contract's interval.
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
    const effective = infinity ? 'q=Query("read",{})' : `q=Query("read",{},null,${minimumMs / 1000})`;
    const policy = await observe({...scenario, program:effective});
    console.log(JSON.stringify({scenario, reference, policy, reason}));
  }
} else {
  const rows = [];
  for (const scenario of scenarios) rows.push([scenario.name, await observe(scenario)]);
  writeFileSync('../conformance/golden/query-refresh.json', JSON.stringify(Object.fromEntries(rows)) + '\n');
}
