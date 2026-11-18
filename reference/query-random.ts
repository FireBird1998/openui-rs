// Seeded scripts for differential replay on the public query-manager seams.
import type { Outcome, Scenario, Step } from './query-scenarios';

export function randomScenario(seed: number) {
  let state = seed;
  function pick(count: number) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) % count;
  }
  // Both statements use the same tool and pool of keys, including settled
  // results delivered to the other statement. Field names distinguish a
  // literal from an argument that has a state dependency.
  function keyFor(slot: number, field: number) {
    return JSON.stringify(slot === 3 ? { f: field } : { v: slot });
  }
  let slots = new Map([['a', 0], ['b', 1]]);
  function source() {
    return ['$f = 0', ...Array.from(slots, ([id, slot]) =>
      `${id} = Query("t", ${slot === 3 ? '{f:$f}' : `{v:${slot}}`}, "${id}def")`)].join('\n');
  }
  const scenario: Scenario = { name: `seed-${seed}`, program: source(), steps: [] };
  const keys: Record<string, string>[] = [];
  let field = 0;
  function* script(calls: readonly { args: unknown }[]): Generator<Step> {
    const pending = new Map<number, { id: string; key: string }>();
    // Exercise unshared calls. Transferring an outstanding call between
    // statements has its own ADR 0013 pin, including a difference in data.
    function unshared(candidate: Map<string, number>, nextField: number) {
      const occupied = new Set<string>();
      for (const [id, slot] of candidate) {
        const key = keyFor(slot, nextField);
        if (occupied.has(key)) return false;
        for (const call of pending.values()) if (call.key === key && call.id !== id) return false;
        occupied.add(key);
      }
      return true;
    }
    function* evaluate(): Generator<Step> {
      const active = Object.fromEntries(Array.from(slots, ([id, slot]) => [id, keyFor(slot, field)]));
      keys.push(active);
      const first = calls.length;
      yield { kind: 'evaluate' };
      for (let call = first; call < calls.length; call++) {
        const key = JSON.stringify(calls[call].args);
        const id = Object.keys(active).find(id => active[id] === key);
        if (!id) throw new Error('unexpected dispatch');
        pending.set(call, { id, key });
      }
    }
    function complete(): Step {
      const call = Array.from(pending.keys())[pick(pending.size)];
      pending.delete(call);
      const outcomes: Outcome[] = [
        { kind: 'result', value: [seed, call] }, { kind: 'result', value: null },
        { kind: 'tool-not-found', availableTools: ['other'] },
        { kind: 'mcp-error', message: 'remote' }, { kind: 'tool-error', message: 'offline' },
      ];
      return { kind: 'complete', call, outcome: outcomes[pick(outcomes.length)] };
    }
    yield* evaluate();
    for (let turn = 0; turn < 18; turn++) {
      const action = pick(8);
      if (action < 3 && pending.size) {
        yield complete();
      } else if (action === 3) {
        const choices = [0, 1, 2].filter(next => unshared(slots, next));
        field = choices[pick(choices.length)];
        yield { kind: 'set', name: '$f', value: field };
        yield* evaluate();
      } else if (action === 4) {
        yield* evaluate();
      } else {
        const id = pick(2) ? 'a' : 'b';
        if (action === 5 && slots.has(id)) {
          slots.delete(id);
        } else {
          const candidates = [0, 1, 2, 3].map(slot => new Map(slots).set(id, slot));
          const choices = candidates.filter(next => unshared(next, field));
          if (choices.length) slots = choices[pick(choices.length)];
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
