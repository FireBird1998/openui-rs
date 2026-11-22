// From reference/: bun --no-install mutation-after-state-write.ts
import { readFileSync, writeFileSync } from 'node:fs';
import { replayMutationScenario } from './session-mutations';

export async function replay() {
  const schema = { $defs: { Box: { properties: { value: {}, action: {} } } } };
  const program = readFileSync(new URL('../conformance/mutation-after-state-write.oui', import.meta.url), 'utf8');
  // The write doubles the query default to 2,400 bytes. Upstream accepts
  // it, while the port replays with a 2,048-byte result limit.
  const steps = [
    { replace: program, persisted: { $detail: 'x'.repeat(1200) } },
    { set: '$t', value: 'delete' },
    { click: true },
  ];
  const [initial, updated, click] = await Array.fromAsync(replayMutationScenario(schema, { steps }));
  return { before: initial.tree.root.props.value, after: updated.tree.root.props.value, clickCalls: click.calls };
}

if (import.meta.main) {
  const golden = new URL('../conformance/golden/mutation-after-state-write.json', import.meta.url);
  writeFileSync(golden, JSON.stringify(await replay(), null, 2) + '\n');
}
