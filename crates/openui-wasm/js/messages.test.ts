import { expect, test } from 'bun:test';
import cases from '../../../conformance/messages.json';
import golden from '../../../conformance/golden/messages.json';
import { kind, wasm } from './support';

test('message text and parts cross the guarded package without losing UTF-16 units', () => {
  for (const name of ['upstream-round-trip', 'writer-lone-surrogate-body']) {
    const input = cases.find(c => c.name === name)!.build!;
    const expected = golden[name];
    const written = wasm.buildMessage(JSON.stringify(input));
    expect(written).toBe(expected.text);
    expect(wasm.parseMessage(written, false)).toBe(expected.read.complete);
    expect(wasm.parseMessage(written.slice(0, -2), true)).toBe(expected.read.cuts.at(-3));
  }
  const raw = cases.find(c => c.name === 'astral-and-lone-surrogates')!.raw!;
  for (let cut = 0; cut <= raw.length; cut++) {
    expect(wasm.parseMessage(raw.slice(0, cut), true)).toBe(golden['astral-and-lone-surrogates'].cuts[cut]);
  }
  kind(() => wasm.parseMessage('x'.repeat(1024 * 1024 + 1), true), 'LimitExceeded');
  kind(() => wasm.buildMessage('{"content":"body","attributes":{"x":"\\ud800"}}'), 'InvalidJson');
  kind(() => wasm.parseMessage({ toString() { throw new Error('unexpected call'); } }, false), 'InvalidJson');
});

test('the package identifies a message attribute URI encoding failure', () => {
  const error = kind(() => wasm.buildMessage('{"content":"body","attributes":{"x":"\\ud800"}}'), 'InvalidJson');
  expect(error.message).toBe('cannot URI-encode message attribute containing a lone surrogate or reserved scalar');
});
