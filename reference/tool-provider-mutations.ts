// A map may return undefined or reject with lang-core's own error subclasses.
import { writeFileSync } from 'node:fs';
import { McpToolError } from './lang-core/src/runtime/mcp';
import { ToolNotFoundError } from './lang-core/src/runtime/toolProvider';
import { createQueryManager } from './lang-core/src/runtime/queryManager';

export async function mutationTranscript(run) {
  const rows: string[] = [];
  for (const [label, handler] of [
    ['undefined', () => undefined],
    ['null', () => null],
    ['application rejects with McpToolError', () => Promise.reject(new McpToolError('offline'))],
    ['application rejects with ToolNotFoundError', () => Promise.reject(new ToolNotFoundError('save', ['other']))],
  ] as const) {
    rows.push(JSON.stringify({label, snapshot: await run({ save: handler })}));
  }
  return rows.join('\n') + '\n';
}

if (import.meta.main) {
  const bytes = await mutationTranscript(async current => {
    // Renderer.tsx's function-map branch. Every fixture provides its handler.
    const manager = createQueryManager({ async callTool(name, args) {
      const fn = current[name];
      if (!fn) throw new ToolNotFoundError(name, Object.keys(current));
      return fn(args);
    } });
    try {
      manager.registerMutations([{statementId:'m', toolName:'save'}]);
      await manager.fireMutation('m', {});
      return manager.getSnapshot();
    } finally { manager.dispose(); }
  });
  writeFileSync('../conformance/golden/tool-provider-mutations.jsonl', bytes);
}
