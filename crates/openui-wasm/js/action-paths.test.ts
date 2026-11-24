import { test, expect } from 'bun:test';
import { kind, loadWith, PROGRAM, SCHEMA, wasm } from './support';

function measuredPackage() {
  const measured = { active: false, quotedBytes: 0, copyBytes: 0 };
  const stringify = JSON.stringify;
  let w: ReturnType<typeof loadWith>;
  try {
    // The package captures stringify during load. Restore the global before calling it.
    JSON.stringify = (value, replacer?: any, space?: string | number) => {
      const text = stringify(value, replacer, space);
      if (measured.active && typeof text === 'string') measured.quotedBytes += Buffer.byteLength(text);
      return text;
    };
    w = loadWith(`
      const pass = passStringToWasm0;
      passStringToWasm0 = (text, malloc, realloc) => {
        if (host.active) host.copyBytes += Buffer.byteLength(text);
        return pass(text, malloc, realloc);
      };
    `, measured);
  } finally { JSON.stringify = stringify; }
  const measure = (call: () => void) => {
    measured.quotedBytes = 0;
    measured.copyBytes = 0;
    measured.active = true;
    try { call(); } finally { measured.active = false; }
    return { quotedBytes: measured.quotedBytes, copyBytes: measured.copyBytes };
  };
  return { w, measure };
}

test('actionAt stops quoting and reading the path at the first overflow', () => {
  const { w, measure } = measuredPackage();
  using library = new w.ComponentLibrary(SCHEMA);
  using limits = new w.RuntimeLimits();
  limits.maxJsonBytes = 1024n;
  using session = w.RuntimeSession.withLimits(library, undefined, limits);
  using program = w.parseProgram(PROGRAM, library);
  session.setProgram(program);
  session.initialize();
  using out = session.evaluate();
  using evaluation = out.evaluation;
  const refuse = (path: (string | number)[]) => {
    expect(kind(() => session.actionAt(evaluation, path), 'LimitExceeded').message).toBe('action path byte limit');
  };

  const path = ['props', '\u0001'.repeat(64 * 1024), 'tail'];
  let tailReads = 0;
  Object.defineProperty(path, 2, { get() { tailReads++; return 'tail'; } });
  expect(measure(() => refuse(path))).toEqual({ quotedBytes: 7, copyBytes: 0 });
  expect(tailReads).toBe(0);
  expect(measure(() => refuse(['props', path[1], 0]))).toEqual({ quotedBytes: 7, copyBytes: 0 });
  expect(kind(() => session.actionAt(evaluation, [0, path[1]]), 'InvalidHandle').message)
    .toBe('expected string path segments');

  // This segment fits in UTF-16 units and overflows when measured in UTF-8.
  expect(measure(() => refuse(['props', '中'.repeat(500)]))).toEqual({ quotedBytes: 7, copyBytes: 0 });

  using action = session.actionAt(evaluation, ['props', 'a']);
  expect(action).toBeInstanceOf(w.ActionHandle);

  using foreign = w.RuntimeSession.withLimits(library, undefined, limits);
  expect(measure(() => {
    expect(kind(() => foreign.actionAt(evaluation, path), 'ForeignHandle').message).toBe('foreign capability');
  }).copyBytes).toBe(0);
  using next = session.evaluate();
  expect(measure(() => {
    expect(kind(() => session.actionAt(evaluation, path), 'StaleHandle').message).toBe('stale action capability');
  }).copyBytes).toBe(0);
  session.dispose();
  expect(measure(() => {
    expect(kind(() => session.actionAt(evaluation, path), 'Disposed').message).toBe('session disposed');
  }).copyBytes).toBe(0);
  expect(tailReads).toBe(0);
});

test('actionAt admits escaped paths up to the decoded byte limit', () => {
  const { w, measure } = measuredPackage();
  const outerKey = 'p'.repeat(128);
  using library = new w.ComponentLibrary(JSON.stringify({ $defs: { Box: { properties: { [outerKey]: {} } } } }));
  using limits = new w.RuntimeLimits();
  limits.maxJsonBytes = 1024n;
  limits.maxActionPathBytes = 2048n;
  using session = w.RuntimeSession.withLimits(library, undefined, limits);
  using program = w.parseProgram('$data=null\nroot=Box($data)', library);
  session.setProgram(program);
  session.initialize();

  const sample = '\b\t\n\f\r"\\\u0001\u000b\u001f\u007fé中😀\ud800x\udc00';
  const charge = (path: string[]) => path.reduce((bytes, part) => bytes + Buffer.byteLength(part) + 1, 0);
  const baseCharge = charge(['props', outerKey, sample, '0']);
  for (const [serialized, retained] of [[1023, 992], [1024, 993], [1025, 994], [1054, 1023], [1055, 1024], [1056, 1025]]) {
    const key = sample + 'a'.repeat(retained - baseCharge);
    const path = ['props', outerKey, key, '0'];
    const normalized = path.map(part => part.toWellFormed());
    expect(charge(normalized)).toBe(retained);
    expect(Buffer.byteLength(JSON.stringify(normalized))).toBe(serialized);
    expect(session.set('$data', JSON.stringify({ [key.toWellFormed()]: [{ steps: [] }] }))).toBe(true);
    using out = session.evaluate();
    using evaluation = out.evaluation;
    const copied = measure(() => {
      if (retained > 1024) {
        expect(kind(() => session.actionAt(evaluation, path), 'LimitExceeded').message).toBe('action path byte limit');
      } else {
        using action = session.actionAt(evaluation, path);
        expect(action).toBeInstanceOf(w.ActionHandle);
      }
    }).copyBytes;
    expect(copied).toBe(retained > 1024 ? 0 : serialized);
  }
});

test('actionAt finds a path at the default 8 MiB boundary', () => {
  const boundary = 8 * 1024 * 1024;
  const outerKey = 'p'.repeat(128);
  const key = 'a'.repeat(boundary - outerKey.length - 'props'.length - '0'.length - 4);
  using library = new wasm.ComponentLibrary(JSON.stringify({ $defs: { Box: { properties: { [outerKey]: {} } } } }));
  using limits = new wasm.RuntimeLimits();
  using evaluationLimits = limits.evaluation;
  // Give the rendered tree room for its JSON syntax as well as the key.
  evaluationLimits.resultSize = BigInt(boundary * 2);
  limits.evaluation = evaluationLimits;
  using session = wasm.RuntimeSession.withLimits(library, undefined, limits);
  using program = wasm.parseProgram('$data=null\nroot=Box($data)', library);
  session.setProgram(program);
  session.initialize();
  expect(session.set('$data', JSON.stringify({ [key]: [{ steps: [] }] }))).toBe(true);
  using out = session.evaluate();
  using evaluation = out.evaluation;
  using action = session.actionAt(evaluation, ['props', outerKey, key, '0']);
  expect(action).toBeInstanceOf(wasm.ActionHandle);
});

test('actionAt uses the snapshotted smaller path limit', () => {
  const { w, measure } = measuredPackage();
  using library = new w.ComponentLibrary(SCHEMA);
  using limits = new w.RuntimeLimits();
  limits.maxJsonBytes = 1024n;
  limits.maxActionPathBytes = 8n;
  using exact = w.RuntimeSession.withLimits(library, undefined, limits);
  limits.maxActionPathBytes = 4n;
  using small = w.RuntimeSession.withLimits(library, undefined, limits);
  limits.maxJsonBytes = 1n;
  limits.maxActionPathBytes = undefined;

  using program = w.parseProgram(PROGRAM, library);
  exact.setProgram(program);
  exact.initialize();
  using exactOut = exact.evaluate();
  using exactEvaluation = exactOut.evaluation;
  using action = exact.actionAt(exactEvaluation, ['props', 'a']);
  expect(action).toBeInstanceOf(w.ActionHandle);
  expect(measure(() => {
    expect(kind(() => exact.actionAt(exactEvaluation, ['props', 'aa']), 'LimitExceeded').message)
      .toBe('action path byte limit');
  })).toEqual({ quotedBytes: 7, copyBytes: 0 });

  small.setProgram(program);
  small.initialize();
  using smallOut = small.evaluate();
  using smallEvaluation = smallOut.evaluation;
  expect(measure(() => {
    expect(kind(() => small.actionAt(smallEvaluation, ['props', 'a', '0']), 'LimitExceeded').message)
      .toBe('action path byte limit');
  })).toEqual({ quotedBytes: 0, copyBytes: 0 });
});
