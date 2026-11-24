import { expect, test } from 'bun:test';
import { schema } from '../conformance/session-mutations.json';
import { replayMutationScenario } from './session-mutations';

test('a removed mutation resumes its action on a large provider success', async () => {
  const steps = [
    { replace: '$done=false\nm=Mutation("save")\nroot=Box($done,Action([@Run(m),@Set($done,true),@OpenUrl("done")]))' },
    { click: true },
    { replace: '$done=false\nroot=Box($done)' },
    // One byte over Rust's 4096-byte result limit, including the JSON quotes.
    { finish: 0, value: 'x'.repeat(4095) },
  ];
  const rows = [];
  for await (const row of replayMutationScenario(schema, { steps })) {
    expect(row.tree.errors).toEqual([]);
    expect(row.changed).toBe(true);
    rows.push([row.calls, row.tree.root.props.value, row.snapshot, row.events]);
  }

  const empty = { __openui_loading: [], __openui_refetching: [], __openui_errors: [] };
  expect(rows).toEqual([
    [[], false, { ...empty, m: { status: 'idle' } }, []],
    [[{ toolName: 'save', args: {} }], false, { ...empty, m: { status: 'loading' } }, []],
    [[], false, empty, []],
    [[], true, empty, [{
      type: 'open_url', params: { url: 'done' }, humanFriendlyMessage: '', formState: { $done: true },
    }]],
  ]);
});
