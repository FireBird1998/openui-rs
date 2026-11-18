import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { kind, wasm } from './support';

test('MCP extraction returns the reference values and tool errors as JSON', () => {
  const limits = new wasm.ParseLimits();
  try {
    const golden = readFileSync(new URL('../../../conformance/golden/mcp-results.jsonl', import.meta.url), 'utf8');
    for (const line of golden.trimEnd().split('\n')) {
      const { name, input, outcome } = JSON.parse(line);
      if ('thrown' in outcome) {
        const error = kind(() => wasm.RuntimeSession.extractMcpToolResult(input, limits), 'Evaluation');
        expect(error.message, name).toBe(outcome.reason);
      } else {
        expect(wasm.RuntimeSession.extractMcpToolResult(input, limits), name).toBe(JSON.stringify(outcome));
      }
    }
  } finally {
    limits.free();
  }
});

test('MCP extraction applies explicit limits and guards input conversion', () => {
  const limits = new wasm.ParseLimits();
  const response = '{"structuredContent":[]}';
  try {
    kind(() => wasm.RuntimeSession.extractMcpToolResult('{', limits), 'InvalidJson');
    limits.resultSize = 1n;
    kind(() => wasm.RuntimeSession.extractMcpToolResult(response, limits), 'LimitExceeded');
    limits.resultSize = undefined;
    limits.work = 1n;
    kind(() => wasm.RuntimeSession.extractMcpToolResult(response, limits), 'LimitExceeded');
    limits.work = undefined;
    limits.depth = 1n;
    kind(() => wasm.RuntimeSession.extractMcpToolResult(response, limits), 'LimitExceeded');
    kind(() => wasm.RuntimeSession.extractMcpToolResult({ toString() { throw new Error('coerced'); } }, limits), 'InvalidJson');
    kind(() => wasm.RuntimeSession.extractMcpToolResult(response, {}), 'InvalidHandle');
  } finally {
    limits.free();
  }
  kind(() => wasm.RuntimeSession.extractMcpToolResult(response, limits), 'Disposed');
});

test('MCP extraction can project deeply nested JSON under raised limits', () => {
  const limits = new wasm.ParseLimits();
  const levels = 12_000;
  const nested = '['.repeat(levels) + 'null' + ']'.repeat(levels);
  try {
    const response = '{"structuredContent":' + nested + '}';
    kind(() => wasm.RuntimeSession.extractMcpToolResult(response, limits), 'LimitExceeded');
    limits.depth = BigInt(levels + 1);
    const structured = wasm.RuntimeSession.extractMcpToolResult(response, limits);
    const textResponse = JSON.stringify({ content: [{ type: 'text', text: nested }] });
    expect(wasm.RuntimeSession.extractMcpToolResult(textResponse, limits)).toBe(structured);
    const coerced = '{"content":[{"type":"text","text":' + nested + '}]}';
    limits.depth = BigInt(levels + 3);
    expect(() => wasm.RuntimeSession.extractMcpToolResult(coerced, limits)).not.toThrow();
    kind(() => wasm.RuntimeSession.extractMcpToolResult('{"ignored":[' + nested + '!', limits), 'InvalidJson');
  } finally {
    limits.free();
  }
});
