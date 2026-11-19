// A shared call is outstanding on the key that becomes the self-reader's
// previous key. Its result equals the value used to build the new node.
import {writeFileSync} from 'node:fs';
import type {Replay} from './query-reader-graphs';
import {readerPolicies, rows} from './query-reader-policies';
import type {Scenario, Step} from './query-scenarios';

export function previousSelfCase(seed: number): Scenario {
  let state = Math.imul(seed, 2654435761);
  function pick(n: number) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) % n;
  }
  const value = seed === 1 ? 1 : seed === 2 ? null : [null, seed + 7, 'v' + seed, '', false, true][pick(6)];
  const repeated = seed === 2 || (seed > 2 && pick(2) === 0);
  const start = seed <= 2 ? 5 : 1 + pick(97);
  const tool = seed <= 2 ? 'u' : 'tool' + pick(11);
  const source = 'q0=Query("' + tool + '",{x:$n})';
  const reader = 'q1=Query("' + tool + '",{x:q1})';
  const siblings = Array.from({length: seed <= 2 ? 0 : pick(5)}, (_, i) =>
    '\nh' + i + '=Query("' + tool + '",{x:$n})').join('');
  const first = '$n=0\n' + source + '\nq1=Query("' + tool + '",{x:$n})' + siblings;
  const second = '$n=0\n' + source + siblings + '\n' + reader + (repeated ? '\n' + reader : '');
  return {
    name: 'previous-self-' + seed, program: first,
    steps: [
      {kind:'set', name:'$n', value:start}, {kind:'evaluate'},
      {kind:'complete', call:0, outcome:{kind:'result',value}},
      {kind:'set', name:'$n', value:0}, {kind:'evaluate'},
      {kind:'replace', program:second, ...(repeated ? {repeatLastReader:true} : {})}, {kind:'evaluate'},
      {kind:'complete', call:1, outcome:{kind:'result',value}},
      {kind:'invalidate', ids:['q1']},
    ]
  };
}

// Start blocked so the renderer's first call uses the seeded Store value.
// Each subsequent render goes through the real renderer-effects transcription.
type RendererStep =
    {replace: string; streaming?: boolean} | {evaluate: true; streaming?: boolean} |
    {set: unknown} | {finish: number; value: unknown} | {invalidate: string[] | undefined};

export function rendererSelfCases() {
  return [1,2].map(seed => {
    const s = previousSelfCase(seed);
    const draw = (p: string) => p + '\nroot=Box([q0,q1])';
    const convert = (step: Step): RendererStep => {
      switch (step.kind) {
        case 'set': return {set:step.value};
        case 'evaluate': return {evaluate:true};
        case 'replace': return {replace:draw(step.program)};
        case 'invalidate': return {invalidate:step.ids};
        case 'complete': if ('call' in step && step.outcome.kind === 'result')
          return {finish:step.call,value:step.outcome.value};
      }
      throw Error('unsupported self-reader renderer step');
    };
    return {name:s.name, steps:[{replace:draw(s.program),streaming:true}, ...s.steps.map(convert)]};
  });
}

export async function selfWaitGoldens(replay: Replay) {
  const policy = await readerPolicies();
  const output = [];
  for (const seed of [1,2]) {
    const scenario = previousSelfCase(seed);
    const reference = await replay(scenario);
    const rust = await replay(scenario, undefined, policy(rows.length));
    output.push({scenario, reference, rust});
  }
  writeFileSync('../conformance/golden/query-self-wait.json', JSON.stringify(output) + '\n');
}
