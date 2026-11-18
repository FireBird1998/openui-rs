import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import corpus from '../../../conformance/response-bundles.json';
import golden from '../../../conformance/golden/response-bundles.json';
import { kind, packagePath, wasm } from './support';

function raw(name: string) {
  const item = corpus.cases.find(item => item.name === name);
  if (!item) throw new Error(`missing case ${name}`);
  return 'units' in item && item.units ? String.fromCharCode(...item.units) : item.raw;
}

function observe(text: string, revisions: Map<string, number>) {
  const output = JSON.parse(text);
  if (!Object.hasOwn(output, 'scriptRevision')) return text;
  const revision = output.scriptRevision;
  expect(typeof revision).toBe('string');
  if (!revisions.has(revision)) revisions.set(revision, revisions.size);
  const at = text.lastIndexOf(',"scriptRevision":');
  expect(at).toBeGreaterThan(0);
  return text.slice(0, at) + `,"scriptRevision":${revisions.get(revision)}}`;
}

test('the wasm package preserves response bundle fields, UTF-16 cuts and revision equality', async () => {
  const dir = join(dirname(packagePath), '../web');
  const web = await import(join(dir, 'openui_wasm.js'));
  web.initSync({ module: readFileSync(join(dir, 'openui_wasm_bg.wasm')) });
  for (const pkg of [wasm, web]) {
    for (const name of ['null', 'crlf', 'unfinished-section', 'reserved-and-lone-program',
      'reserved-and-lone-name', 'surrogates-in-script-strings', 'json-escape-pairs-with-raw-unit']) {
      const input = raw(name);
      const expected = golden.cases[name as keyof typeof golden.cases];
      const revisions = new Map<string, number>();
      expect(observe(pkg.parseResponseBundle(input, false), revisions), name).toBe(expected.complete);
      for (let cut = 0; cut < expected.cuts.length; cut++) {
        expect(observe(pkg.parseResponseBundle(input?.slice(0, cut) ?? null, true), revisions), `${name} cut ${cut}`).toBe(expected.cuts[cut]);
      }
    }
    const revision = (name: string) => {
      const value = JSON.parse(pkg.parseResponseBundle(raw(name), false)).scriptRevision;
      expect(typeof value).toBe('string');
      return value;
    };
    expect(revision('revision-original-order')).toBe(revision('revision-reordered'));
    expect(revision('revision-original-order')).not.toBe(revision('revision-changed-code'));
  }
});

test('the wasm package rejects an oversized or over-deep response bundle with a typed error', () => {
  kind(() => wasm.parseResponseBundle('x'.repeat(1024 * 1024 + 1), true), 'LimitExceeded');
  kind(() => wasm.parseResponseBundle('é'.repeat(512 * 1024 + 1), false), 'LimitExceeded');
  kind(() => wasm.parseResponseBundle(']]>openui:scripts {"count":0}\n' + '['.repeat(801), true), 'LimitExceeded');
  kind(() => wasm.parseResponseBundle({ toString() { throw new Error('unexpected call'); } }, false), 'InvalidJson');
});

test('the wasm bundle reader copies UTF-16 with captured typed-array intrinsics', () => {
  const prototype = Uint16Array.prototype;
  const length = Object.getOwnPropertyDescriptor(prototype, 'length');
  const set = Object.getOwnPropertyDescriptor(prototype, 'set');
  const trap = () => { throw new Error('host typed-array override ran'); };
  const input = raw('reserved-and-lone-program');
  let actual: string;
  try {
    Object.defineProperties(prototype, {
      length: { configurable: true, get: trap },
      set: { configurable: true, value: trap },
    });
    actual = wasm.parseResponseBundle(input, false);
  } finally {
    if (length) Object.defineProperty(prototype, 'length', length);
    else Reflect.deleteProperty(prototype, 'length');
    if (set) Object.defineProperty(prototype, 'set', set);
    else Reflect.deleteProperty(prototype, 'set');
  }
  expect(actual).toBe(golden.cases['reserved-and-lone-program'].complete);
});
