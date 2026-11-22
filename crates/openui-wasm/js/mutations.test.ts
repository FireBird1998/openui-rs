import { expect, test } from 'bun:test';
import { nextToolTurn } from '../../../reference/tool-provider-scenarios';
import { kind, loadWith, wasm } from './support';

test('a managed mutation resumes through the guarded drain and cannot cross sessions', () => {
  using lib = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"action":{}}}}}');
  using s = new wasm.RuntimeSession(lib);
  using other = new wasm.RuntimeSession(lib);
  s.attachToolProvider(); other.attachToolProvider();
  using p = wasm.parseProgram('m=Mutation("save",{})\nroot=Box(Action([@Run(m),@OpenUrl("done")]))',lib);
  using shown = s.replaceProgram(p,undefined,undefined,true);
  using draw = shown.evaluation;
  using evaluation = draw.evaluation;
  using button = s.actionAt(evaluation,['props','action']);
  using run = s.run(button,'click');
  expect(run.continuation).toBeUndefined();
  expect(run.events).toEqual([]);
  using wanted = s.takeQueryUpdate();
  using call = wanted.callAt(0n);
  expect(call.toolName).toBe('save');
  expect(wanted.actionCount).toBe(0n);
  kind(() => other.completeToolCall(call,'{"value":1}'),'InvalidToolCall');
  s.completeToolCall(call,'{"value":1}');
  kind(() => s.completeToolCall(call,'{"value":2}'),'InvalidToolCall');
  using effects = s.takeQueryUpdate();
  expect(effects.actionCount).toBe(1n);
  expect(effects.actionAt(1n)).toBeUndefined();
  using tail = effects.actionAt(0n);
  expect(tail.events.map(JSON.parse)[0].params.url).toBe('done');
  expect(tail.failure).toBeNull();
  using next = s.takeQueryUpdate();
  expect(next.actionCount).toBe(0n);
});

test('a driven mutation delivers the resumed action once and releases the borrowed output', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"action":{}}}}}');
  using session = new wasm.RuntimeSession(library);
  using p = wasm.parseProgram('m=Mutation("save",{v:3})\nroot=Box(Action([@Run(m),@OpenUrl("done")]))', library);
  const args = [], events = [];
  let borrowed;
  session.setToolProvider({ save(input) { args.push(input); return 42; } }, {
    onAction(output) {
      borrowed = output;
      events.push(...output.events.map(JSON.parse));
    },
  });
  using replaced = session.replaceProgram(p, undefined, undefined, true);
  using draw = replaced.evaluation;
  using evaluation = draw.evaluation;
  using button = session.actionAt(evaluation, ['props','action']);
  using run = session.run(button, 'click');
  expect(args).toEqual([{v:3}]);
  expect(events).toEqual([]);
  await nextToolTurn();
  expect(events.map(event => event.params.url)).toEqual(['done']);
  kind(() => borrowed.events, 'Disposed');
  using finished = session.evaluate();
  expect(JSON.parse(finished.querySnapshotJson).m).toEqual({status:'success', data:42});
});

test('a map mutation preserves missing data and reports application rejections with their messages', async () => {
  const { mutationTranscript } = await import('../../../reference/tool-provider-mutations');
  const upstream = await Bun.file(new URL('../../../conformance/golden/tool-provider-mutations.jsonl', import.meta.url)).text();
  const exceptions = await Bun.file(new URL('../../../conformance/divergences/tool-provider-rejections.jsonl', import.meta.url)).text();
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"action":{}}}}}');
  using p = wasm.parseProgram('m=Mutation("save",{})\nroot=Box(Action([@Run(m)]))', library);
  const bytes = await mutationTranscript(async provider => {
    using session = new wasm.RuntimeSession(library);
    session.setToolProvider(provider);
    using replaced = session.replaceProgram(p, undefined, undefined, true);
    using draw = replaced.evaluation;
    using evaluation = draw.evaluation;
    using button = session.actionAt(evaluation, ['props','action']);
    using run = session.run(button, 'click');
    await Bun.sleep(0);
    using done = session.evaluate();
    return JSON.parse(done.querySnapshotJson);
  });
  const lines = upstream.split('\n');
  expect(bytes).toBe(lines.slice(0, 2).join('\n') + '\n' + exceptions);
});

test('throwing callbacks preserve both queued actions, redraws and the successful write', async () => {
  const pkg = loadWith(`
    const take = RuntimeSession.prototype.takeQueryUpdate;
    const count = Object.getOwnPropertyDescriptor(QueryEffects.prototype, 'actionCount').get;
    RuntimeSession.prototype.takeQueryUpdate = function() {
      const update = take.call(this);
      const actions = count.call(update);
      if (actions) (this.batches ??= []).push(actions);
      return update;
    };`);
  using library = new pkg.ComponentLibrary('{"$defs":{"Box":{"properties":{"first":{},"second":{}}}}}');
  using session = new pkg.RuntimeSession(library);
  using p = pkg.parseProgram('m=Mutation("save",{})\nn=Mutation("save",{})\nroot=Box(Action([@Run(m),@OpenUrl("done")]),Action([@Run(n),@OpenUrl("done")]))', library);
  const actions = [], errors = [];
  session.setToolProvider({save: () => 1}, {
    onAction(out) {
      actions.push(...out.events.map(JSON.parse));
      if (actions.length === 1) throw Error('first action');
      return Promise.reject('second action');
    },
    onChange() { throw Error('redraw'); },
    onError(error) {
      errors.push(error.message);
      if (errors.length % 2) throw Error('error handler');
      return Promise.reject(Error('async error handler'));
    },
  });
  using shown = session.replaceProgram(p, undefined, undefined, true);
  using draw = shown.evaluation;
  using evaluation = draw.evaluation;
  using button = session.actionAt(evaluation, ['props', 'first']);
  using other = session.actionAt(evaluation, ['props', 'second']);
  // Leave both resumed outputs in one drain.
  using first = session.run(button, 'click');
  using second = session.run(other, 'click');
  await nextToolTurn();
  expect(session.batches).toEqual([2n]);
  expect(actions.map(e => e.params.url)).toEqual(['done', 'done']);
  using run = session.run(button, 'click');
  await nextToolTurn();
  expect(actions.length).toBe(3);
  expect(errors).toContain('first action');
  expect(errors.filter(e => e === 'second action')).toHaveLength(2);
  expect(errors.filter(e => e === 'redraw').length).toBeGreaterThan(1);
});

test('replacing callbacks delivers an in-flight action, its error and its redraw to the current options', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"action":{}}}}}');
  using session = new wasm.RuntimeSession(library);
  using p = wasm.parseProgram('m=Mutation("save",{})\nroot=Box(Action([@Run(m),@OpenUrl("done")]))', library);
  const first = Promise.withResolvers<number>();
  const old = [], current = [];
  const provider = {save: () => first.promise};
  session.setToolProvider(provider, {
    onChange: () => old.push('redraw'), onAction: () => old.push('action'), onError: () => old.push('error'),
  });
  using out = session.replaceProgram(p, undefined, undefined, true);
  using draw = out.evaluation;
  using evaluation = draw.evaluation;
  using button = session.actionAt(evaluation, ['props', 'action']);
  using run = session.run(button, 'click');
  old.length = 0;
  session.setToolProvider(provider, {
    onChange: () => current.push('redraw'),
    onAction() { current.push('action'); throw Error('callback'); },
    onError: e => current.push(e.message),
  });
  first.resolve(1);
  await nextToolTurn();
  expect(old).toEqual([]);
  expect(current).toEqual(['action', 'callback', 'redraw']);
  session.setToolProvider(provider, {});
  using again = session.run(button, 'click');
  await nextToolTurn();
  expect(current).toEqual(['action', 'callback', 'redraw']);
});

test('an undefined mutation result resumes its action tail and a cache refusal stops the tail', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"action":{}}}}}');
  using p = wasm.parseProgram('m=Mutation("save",{})\nroot=Box(Action([@Run(m),@OpenUrl("done")]))', library);
  using limits = new wasm.RuntimeLimits();
  limits.maxCacheBytes = 16n;
  using session = wasm.RuntimeSession.withLimits(library, undefined, limits);
  const actions = [];
  session.setToolProvider({save() {}}, {onAction: out => actions.push(out.events.map(JSON.parse))});
  using shown = session.replaceProgram(p, undefined, undefined, true);
  using draw = shown.evaluation;
  using evaluation = draw.evaluation;
  using button = session.actionAt(evaluation, ['props','action']);
  using first = session.run(button, 'click');
  await nextToolTurn();
  expect(actions[0].map(event => event.params.url)).toEqual(['done']);
  session.setToolProvider({save: () => 'x'.repeat(32)});
  using second = session.run(button, 'click');
  await nextToolTurn();
  expect(actions[1]).toEqual([]);
  using completed = session.evaluate();
  expect(JSON.parse(completed.querySnapshotJson).__openui_errors[0].message).toContain('cache limit of 16 bytes');
});

test('the driver delivers an eviction segment once to onError and onAction', () => {
  // Inject the session's drained segment at the driver boundary. A second
  // public managed submission at capacity is refused before it can evict.
  const pkg = loadWith(`
    const take = RuntimeSession.prototype.takeQueryUpdate;
    RuntimeSession.prototype.takeQueryUpdate = function() {
      const update = take.call(this);
      if (this.eviction) {
        const failure = this.eviction;
        delete this.eviction;
        Object.defineProperties(update, {
          actionCount: { value: 1n },
          actionAt: { value: () => ({ failure, events: [], free() {} }) },
        });
      }
      return update;
    };`);
  using library = new pkg.ComponentLibrary('{}');
  using session = new pkg.RuntimeSession(library);
  const delivered = [];
  session.eviction = { step: 1, error: { kind: 'EvictedHandle', message: 'continuation evicted' } };
  session.setToolProvider({}, {
    onError: e => delivered.push([e.kind, e.message]),
    onAction: action => delivered.push([action.failure.step, action.failure.error.kind, action.events]),
  });
  session.evaluate().free();
  expect(delivered).toEqual([
    ['EvictedHandle', 'continuation evicted'],
    [1, 'EvictedHandle', []],
  ]);
});
