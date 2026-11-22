import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { replay } from './mutation-after-state-write';

test('the renderer click after a state write matches the recorded tool call', async () => {
  const golden = JSON.parse(readFileSync(new URL('../conformance/golden/mutation-after-state-write.json', import.meta.url), 'utf8'));
  expect(await replay()).toEqual(golden);
});
