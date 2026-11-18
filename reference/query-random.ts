// Seeded scripts for differential replay on the public query-manager seams.
import type { Outcome, Scenario, Step } from './query-scenarios';

const statusLists = ['__openui_loading', '__openui_refetching', '__openui_errors'] as const;

export function randomScenario(seed: number) {
  let state = seed;
  function pick(count: number) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) % count;
  }
  // Use array values for a list-named statement. Upstream pushes directly
  // into its result/default array when it builds the status lists.
  const reserved = seed % 8 === 0 ? statusLists[(seed / 8) % statusLists.length] : undefined;
  const ids = [reserved ?? 'a', 'b'];
  let slots = new Map([[ids[0], 0], [ids[1], 1]]);
  function source() {
    return ['$f = 0', ...Array.from(slots, ([id, slot]) =>
      `${id} = Query("t", ${slot === 3 ? '{f:$f}' : `{v:${slot}}`}, ${id === reserved ? '[]' : JSON.stringify(id + 'def')})`)].join('\n');
  }
  const scenario: Scenario = { name: `seed-${seed}`, program: source(), steps: [] };
  const keys: Record<string, string>[] = [];
  let field = 0;
  function* script(calls: readonly { args: unknown }[]): Generator<Step> {
    const pending = new Set<number>();
    function* evaluate(): Generator<Step> {
      keys.push(Object.fromEntries(Array.from(slots, ([id, slot]) =>
        [id, JSON.stringify(slot === 3 ? { f: field } : { v: slot })])));
      const first = calls.length;
      yield { kind: 'evaluate' };
      for (let call = first; call < calls.length; call++) pending.add(call);
    }
    function complete(): Step {
      const call = Array.from(pending)[pick(pending.size)];
      pending.delete(call);
      const outcomes: Outcome[] = [
        { kind: 'result', value: [seed, call] }, { kind: 'result', value: reserved ? [] : null },
        { kind: 'tool-not-found', availableTools: ['other'] },
        { kind: 'mcp-error', message: 'remote' }, { kind: 'tool-error', message: 'offline' },
      ];
      return { kind: 'complete', call, outcome: outcomes[pick(outcomes.length)] };
    }
    yield* evaluate();
    for (let turn = 0; turn < 18; turn++) {
      const action = pick(11);
      if (action < 3 && pending.size) {
        yield complete();
      } else if (action === 3) {
        field = pick(3);
        yield { kind: 'set', name: '$f', value: field };
        yield* evaluate();
      } else if (action === 4) {
        yield* evaluate();
      } else {
        const [id, other] = pick(2) === 0 ? ids : [ids[1], ids[0]];
        if (action === 5) slots.delete(id);
        else if (action === 6) slots.set(id, pick(4));
        else if (action === 10) {
          slots = new Map([[id, slots.get(other) ?? pick(4)]]);
        } else {
          // Replace both statements in one command. Share, exchange, or
          // independently choose keys, in either declaration order.
          const first = action === 8 ? slots.get(other) ?? pick(4) : pick(4);
          const second = action === 7 ? first : action === 8 ? slots.get(id) ?? pick(4) : pick(4);
          slots = new Map([[id, first], [other, second]]);
        }
        yield { kind: 'replace', program: source() };
        yield* evaluate();
      }
    }
    while (pending.size) yield complete();
    yield* evaluate();
  }
  return { scenario, script, keys };
}
