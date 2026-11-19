// Each script snapshots the dispatches at a checkpoint. Work scheduled by a
// completion can only be visited by a later checkpoint, including self-reads.
import type {createQueryManager} from './lang-core/src/runtime/queryManager';
import {previousSelfCase} from './query-self-wait';
import {readerPolicies, rows} from './query-reader-policies';
import type {Dispatch, Outcome, Scenario, Step} from './query-scenarios';

type Script = (calls: readonly Dispatch[]) => Iterable<Step>;
export type Replay =
    (s: Scenario, script?: Script, factory?: typeof createQueryManager,
     beginStep?: (index: number) => void) => Promise<string[]>;

function graph(seed: number) {
  let state = Math.imul(seed, 2654435761);
  function pick(n: number) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) % n;
  }
  const n = 2 + pick(5), shared = seed % 3 === 0, self = seed % 7 === 0;
  const names = Array.from({length: n}, (_, i) => 'q' + i);
  const tools =
      names.map((id, i) => shared ? 't' + (i % 2) : (seed % 13 === 0 && i === 0 ? 'm' : 't' + id));
  const deps = names.map((_, i) => self && !i ? 0 : seed % 4 === 0 ? (i + 1) % n : pick(n + 1));
  const forms = names.map(() => pick(7));
  function program(phase: number) {
    const out = ['$n=0', '$f=0'];
    if (seed % 13 === 0) out.push('m=Mutation("save",{})');
    for (let i = 0; i < n; i++) {
      const id = phase === 3 && i === 0 && seed % 3 === 0 ? 'renamed' : names[i];
      if (phase === 3 && i === 0 && seed % 3 === 1) continue;
      if (phase === 3 && i === 0 && seed % 3 === 2) {
        out.push(id + '="plain"');
        continue;
      }
      const dep = names[deps[i]], alias = 'alias' + i;
      if (dep && forms[i] === 5) out.push(alias + '=' + dep);
      const expr = [
        dep, dep + '.v', dep + '[0]', '@Count(' + dep + ')', '@Each(' + dep + ', item, item)', alias, dep
      ][forms[i]];
      const reads = phase && dep;
      const input = reads && forms[i] !== 6 ? expr : phase === 0 && seed % 2 ? '0' : '$n';
      out.push(
          id + '=Query("' + tools[i] + '",{r:' + input + (seed % 13 === 0 && i === 0 ? ',status:m.status' : '') + '},' + (reads && forms[i] === 6 ? expr : '"d"') +
          ')');
    }
    return out.join('\n');
  }
  const scenario: Scenario = {name: 'broad-reader-' + seed, program: program(0), steps: []};
  const outcomes: Outcome[] = [
    {kind: 'result', value: 'd'}, {kind: 'result', value: 'd'}, {kind: 'result', value: null},
    {kind: 'result'}, {kind: 'result', value: {v: 'd'}}, {kind: 'result', value: ['d']},
    {kind: 'mcp-error', message: 'offline'}, {kind: 'result', hostNumber: 'NaN'}
  ];
  function* script(calls: readonly Dispatch[]): Generator<Step> {
    const used = new Set<number>();
    yield {kind: 'evaluate'};
    if (seed % 13 === 0) yield {kind: 'register'};
    for (let phase = 0; phase < 6; phase++) {
      if (phase === 1 || phase === 4) {
        if (seed % 4 === 0) {
          const id = names[(seed + phase) % n];
          yield {kind: 'invalidate', ids: seed % 8 ? [id, id] : []};
        }
        if (seed % 13 === 0) yield {kind: 'fire', name: 'm', args: {phase}};
      }
      const pending = Array.from({length: calls.length}, (_, i) => i).filter(i => !used.has(i));
      const take = phase === 0 && seed % 5 === 0 ? Math.max(1, pending.length - 1) : pending.length;
      for (let j = 0; j < Math.min(take, 8); j++) {
        const [i] = pending.splice(pick(pending.length), 1);
        used.add(i);
        const {step, toolName, args} = calls[i];
        const occurrence = calls.slice(0, i)
                               .filter(
                                   c => c.step === step && c.toolName === toolName &&
                                       JSON.stringify(c.args) === JSON.stringify(args))
                               .length;
        yield {
          kind: 'complete',
          dispatch: {step, toolName, args, occurrence},
          outcome: outcomes[pick(outcomes.length)]
        };
      }
      if (phase === 0 || phase === 2 || phase === 4) {
        yield {kind: 'replace', program: program(phase === 2 ? 3 : 1)};
      } else if (phase === 3 && seed % 2) {
        yield {kind: 'extra', toolName: tools[pick(n)], values: {x: pick(3)}};
      } else {
        yield {kind: 'set', name: '$n', value: pick(3)};
      }
      yield {kind: 'evaluate'};
    }
  }
  return {scenario, script};
}

function addresses(observations: string[]): Dispatch[] {
  return observations.flatMap((bytes, step) => {
    const seen = new Map<string, number>();
    const calls: {toolName: string, args: unknown}[] = JSON.parse(bytes).calls;
    return calls.map(({toolName, args}) => {
      const key = JSON.stringify([toolName, args]);
      const occurrence = seen.get(key) ?? 0;
      seen.set(key, occurrence + 1);
      return {step, toolName, args, occurrence};
    });
  });
}

// Attribute to the transition out of the last policy prefix matching the
// genuine reference, as in query-lifetimes. Validate the final bytes in Rust.
export async function runGraphs(replay: Replay, previousSelf = false) {
  const policies = await readerPolicies();
  for (let seed = 1; seed <= (previousSelf ? 128 : 5000); seed++) {
    const {scenario, script} = previousSelf ? {scenario: previousSelfCase(seed), script: undefined} : graph(seed);
    let reference = await replay(scenario, script);
    let expected = await replay(scenario, undefined, policies(rows.length));
    const original = new Set(addresses(reference).map(a => JSON.stringify(a)));
    const extra = addresses(expected).filter(a => !original.has(JSON.stringify(a)));
    if (scenario.steps.length + extra.length > 256) throw Error('reader graph step budget');
    // One finite pass over previously observed calls, including released receipts.
    // Calls created by this pass belong to a later host checkpoint.
    for (const [i,dispatch] of extra.entries()) {
      const outcome: Outcome = (seed+i)%2 ? {kind:'result',value:{v:seed}} : {kind:'tool-error',message:'late failure'};
      scenario.steps.push({kind:'complete',dispatch,outcome});
    }
    if (extra.length) {
      reference = await replay(scenario);
      expected = await replay(scenario, undefined, policies(rows.length));
    }
    const baseline = await replay(scenario, undefined, policies(0));
    if (reference.some((v, i) => v !== baseline[i]))
      throw Error('instrumentation changed ' + scenario.name);
    const differing = reference.flatMap((s, i) => s === expected[i] ? [] : [i]);
    const reasons: Record < number, string >= {};
    if (differing.length) {
      let before = baseline;
      for (let j = 0; j < rows.length; j++) {
        const after = j + 1 === rows.length ? expected : await replay(scenario, undefined, policies(j + 1));
        for (const i of differing)
          if (before[i] === reference[i] && after[i] !== reference[i]) reasons[i] = rows[j];
        before = after;
      }
      // In a previous-key handoff, a loading discrepancy can predate a
      // different dispatch. Attribute that new call to the wake only when
      // removing that one policy restores upstream's dispatch for the command.
      if (previousSelf) {
        const wake = 'a-previous-key-releases-a-waiting-self-reader';
        const without = await replay(scenario, undefined, policies(rows.filter(row => row !== wake)));
        for (const i of differing) {
          const calls = (bytes: string) => JSON.stringify(JSON.parse(bytes).calls);
          if (calls(reference[i]) !== calls(expected[i]) && calls(reference[i]) === calls(without[i]))
            reasons[i] = wake;
        }
      }
      for (const i of differing) {
        if (!reasons[i]) throw Error('unattributed ' + scenario.name + ' step ' + i);
        const a = JSON.parse(reference[i]), b = JSON.parse(expected[i]);
        if (JSON.stringify(a.snapshot) === JSON.stringify(b.snapshot) &&
            JSON.stringify(a.calls) === JSON.stringify(b.calls))
          reasons[i] = 'change-flag-after-loading-converges';
      }
    }
    console.log(JSON.stringify({scenario, reference, expected, reasons, extraCalls: extra.length}));
  }
}
