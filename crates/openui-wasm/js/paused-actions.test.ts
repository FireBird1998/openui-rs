// #117: an action dropped while queries are paused is reported once, as a
// QueriesPaused failure, through the session and through the driver.
import { expect, test } from 'bun:test';
import { wasm } from './support';
import { nextToolTurn } from '../../../reference/tool-provider-scenarios';

const SCHEMA = '{"$defs":{"Box":{"properties":{"value":{},"action":{}}}}}';
const PLAN = '$n=0\nm=Mutation("save")\nroot=Box($n,Action([@Set($n,1),@Run(m),@Set($n,2),@OpenUrl("after")]))';

function session() {
  const library = new wasm.ComponentLibrary(SCHEMA);
  const s = new wasm.RuntimeSession(library);
  const program = wasm.parseProgram(PLAN, library);
  library.free();
  return { s, program };
}

function click(s) {
  using draw = s.evaluate();
  using action = s.actionAt(draw.evaluation, ['props', 'action']);
  return s.run(action, 'click');
}

function paused(failure, text) {
  expect(failure.step).toBe(1);
  expect(failure.error.kind).toBe('QueriesPaused');
  expect(failure.error.message).toContain(text);
}

test('the session reports a click while paused and a call in flight at pause', () => {
  const { s, program } = session();
  try {
    s.attachToolProvider();
    s.replaceProgram(program, undefined, undefined, true).free();
    s.takeQueryUpdate().free();
    click(s).free();
    using wanted = s.takeQueryUpdate();
    using call = wanted.callAt(0n);
    expect(call.toolName).toBe('save');
    s.pauseQueries();
    using update = s.takeQueryUpdate();
    expect(update.callCount).toBe(0n);
    expect(update.actionCount).toBe(1n);
    using dropped = update.actionAt(0n);
    paused(dropped.failure, 'outcome');
    expect(dropped.events).toEqual([]);
    s.completeToolCall(call, '{"value":1}');
    using late = s.takeQueryUpdate();
    expect(late.actionCount).toBe(0n);
    using run = click(s);
    paused(run.failure, 'not sent');
    expect(run.continuation).toBeUndefined();
    s.resumeQueries();
    using resumed = s.takeQueryUpdate();
    expect(resumed.actionCount).toBe(0n);
    expect(resumed.callCount).toBe(0n);
    expect(s.get('$n')).toBe('1');
  } finally {
    program.free();
    s.free();
  }
});

function driven(provider) {
  const { s, program } = session();
  const errors = [];
  const actions = [];
  s.setToolProvider(provider, {
    onError: e => errors.push([e.kind, e.message]),
    onAction: output => actions.push(output.failure),
  });
  s.replaceProgram(program, undefined, undefined, true).free();
  program.free();
  return { s, errors, actions };
}

test('the driver reports a mutation in flight at pause once, and ignores its late outcome', async () => {
  for (const settle of ['resolve', 'reject', 'never']) {
    const pending = Promise.withResolvers();
    let calls = 0;
    const { s, errors, actions } = driven({ save: () => { calls++; return pending.promise; } });
    try {
      click(s).free();
      await nextToolTurn();
      expect(calls).toBe(1);
      s.pauseQueries();
      await nextToolTurn();
      expect(errors).toHaveLength(1);
      expect(errors[0][0]).toBe('QueriesPaused');
      expect(errors[0][1]).toContain('outcome');
      expect(actions).toHaveLength(1);
      paused(actions[0], 'outcome');
      if (settle === 'resolve') pending.resolve(1);
      if (settle === 'reject') pending.reject(Error('late'));
      await nextToolTurn();
      s.resumeQueries();
      await nextToolTurn();
      // Pausing twice reports nothing more.
      s.pauseQueries();
      s.pauseQueries();
      s.resumeQueries();
      await nextToolTurn();
      expect(errors).toHaveLength(1);
      expect(actions).toHaveLength(1);
      expect(calls).toBe(1);
      expect(s.get('$n')).toBe('1');
    } finally {
      s.free();
    }
  }
});

test('a click while the driver is paused fails in its own output and runs nothing after resume', async () => {
  let calls = 0;
  const { s, errors, actions } = driven({ save: () => { calls++; return 1; } });
  try {
    await nextToolTurn();
    s.pauseQueries();
    using run = click(s);
    paused(run.failure, 'not sent');
    s.resumeQueries();
    await nextToolTurn();
    expect(calls).toBe(0);
    expect(errors).toEqual([]);
    expect(actions).toEqual([]);
    expect(s.get('$n')).toBe('1');
  } finally {
    s.free();
  }
});

test('freeing or disposing right after a pause still delivers the failure once', async () => {
  for (const stop of ['free', 'dispose']) {
    const pending = Promise.withResolvers();
    const { s, errors, actions } = driven({ save: () => pending.promise });
    click(s).free();
    await nextToolTurn();
    // The pause's drain waits for a task while the call was pending.
    s.pauseQueries();
    s[stop]();
    await nextToolTurn();
    pending.resolve(1);
    await nextToolTurn();
    expect(errors.map(e => e[0])).toEqual(['QueriesPaused']);
    expect(actions).toHaveLength(1);
    if (stop === 'dispose') s.free();
  }
});

test('a mutation whose drain waits for a task is reported as not sent and its tool never runs', async () => {
  const slow = Promise.withResolvers();
  let saves = 0;
  const { s, errors, actions } = driven({});
  try {
    // A query call pending past its checkpoint makes the next drain wait for a task.
    s.setToolProvider({ save: () => { saves++; return 1; }, read: () => slow.promise });
    using library = new wasm.ComponentLibrary(SCHEMA);
    using parsed = wasm.parseProgram(PLAN + '\nq=Query("read")', library);
    s.replaceProgram(parsed, undefined, undefined, true).free();
    await nextToolTurn();
    click(s).free();
    s.pauseQueries();
    expect(errors).toHaveLength(0);
    await nextToolTurn();
    slow.resolve(1);
    await nextToolTurn();
    expect(saves).toBe(0);
    expect(errors).toHaveLength(1);
    expect(errors[0][0]).toBe('QueriesPaused');
    expect(errors[0][1]).toContain('not sent');
    paused(actions[0], 'not sent');
    s.resumeQueries();
    await nextToolTurn();
    expect(saves).toBe(0);
    expect(errors).toHaveLength(1);
  } finally {
    s.free();
  }
});
