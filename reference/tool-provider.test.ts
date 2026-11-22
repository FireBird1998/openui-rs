import {expect, test} from 'bun:test';
import {runInNewContext} from 'node:vm';
import {referenceHost} from './tool-provider';
import {nextToolTurn} from './tool-provider-scenarios';

test('query snapshots publish built-ins by identity across prototype changes and realms', async () => {
  const replaced = {};
  const values = [
    new Date(123), new Map(), new Uint8Array(0),
    Object.setPrototypeOf(new Date(123), replaced),
    Object.setPrototypeOf(new Number(5), replaced),
    Object.assign(Object.setPrototypeOf(new Number(5), null), {valueOf: () => NaN}),
    Object.setPrototypeOf(new Number(5), {valueOf: () => 7}),
    Object.setPrototypeOf(new Map([['k', 7]]), replaced),
    Object.setPrototypeOf(new Map(), null),
    Object.setPrototypeOf(new Set([7]), replaced),
    Object.setPrototypeOf(new WeakMap([[{}, 7]]), replaced),
    Object.setPrototypeOf(new WeakSet([{}]), replaced),
    Object.setPrototypeOf(new Date(123), null),
    Object.setPrototypeOf(new Uint8Array([7]), Object.create(Object.create(null))),
    Object.assign(Object.setPrototypeOf(new Number(5), Object.create(Object.create(null))), {valueOf: () => NaN}),
    Object.setPrototypeOf(new Uint8Array(0), null),
    Object.setPrototypeOf(new Uint8Array(0), replaced),
    ...runInNewContext("[new Map([['k', 7]]), new Uint8Array(0), new Uint8Array([3])]"),
  ];
  for (const value of values) {
    for (const provider of [{read: () => value}, {callTool: () => ({structuredContent: value})}]) {
      const host = referenceHost();
      try {
        host.provider(provider);
        host.draw(false);
        await nextToolTurn();
        const snapshot = host.snapshot();
        expect(snapshot.__openui_loading).toEqual([]);
        expect(snapshot.__openui_errors).toEqual([]);
        expect(snapshot.q).toBe(value);
      } finally { host.dispose(); }
    }
  }
});
