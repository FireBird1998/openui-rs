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
  kind(() => wasm.buildMessage('{"content":"body","attributes":{"x":"\\ud800"}}'), 'InvalidAttribute');
  kind(() => wasm.parseMessage({ toString() { throw new Error('unexpected call'); } }, false), 'InvalidJson');
});

test('a message attribute URI encoding failure has its own kind', () => {
  // Upstream's URIError: a lone surrogate in an attribute's value or name.
  for (const attributes of [{ x: '\ud800' }, { '\udfff': 'x' }]) {
    const error = kind(() => wasm.buildMessage(JSON.stringify({ content: 'body', attributes })), 'InvalidAttribute');
    expect(error).toBeInstanceOf(Error);
    expect(error.message).toBe('cannot URI-encode message attribute containing a lone surrogate or reserved scalar');
  }
  // Malformed parts are still InvalidJson, and a valid pair still encodes.
  for (const parts of ['{"content":1}', '[', '{"content":"body","attributes":{"x":1}}']) {
    kind(() => wasm.buildMessage(parts), 'InvalidJson');
  }
  expect(() => wasm.buildMessage(JSON.stringify({ content: 'body', attributes: { x: '\ud83d\ude00' } }))).not.toThrow();
});
